//! The parser: tokens in, a syntax tree and the program's symbols out.
//!
//! It follows the reference compiler's parser routine by routine,
//! including how it carries on after an error, so that the same source
//! gives the same tree. So far it reads functions: their parameters,
//! local variables, the statements that need no objects, and
//! expressions over numbers, strings, locals and function calls.

use std::collections::HashMap;

use crate::ast::{
    Arg, AsiOp, BinOp, CodeBody, Expr, LocalId, Stmt, StmtKind, TabId, UnOp, Value, flow,
};
use crate::fold::{self, Folded};
use crate::lex::Tok;
use crate::pp::{Preprocessor, Severity, show_name};

/// A local variable or a parameter.
#[derive(Debug, Clone)]
pub(crate) struct Local {
    pub name: Vec<u8>,
    pub is_param: bool,
    /// Its slot: a parameter number or a local number.
    pub var_num: usize,
    /// It is the parameter that collects variable arguments in a list.
    pub is_list_param: bool,
    pub val_used: bool,
    pub val_assigned: bool,
    /// Where it is declared.
    pub file: usize,
    pub line: u32,
}

/// The locals of one scope.
#[derive(Debug, Clone, Default)]
pub(crate) struct LocalTab {
    pub parent: Option<TabId>,
    /// The scope's locals, in the order they were declared.
    pub entries: Vec<LocalId>,
    /// The part of the function's code the scope covers, as offsets from
    /// the start of the function. Filled in as code is generated.
    pub start_ofs: usize,
    pub end_ofs: usize,
    /// The scope's number in the function's list of scopes, from one,
    /// once it is in the list.
    pub list_index: usize,
}

/// How many chains the reference's table of a scope's locals has. The
/// order locals are written to the image in comes from it.
const LOCAL_HASH_SIZE: usize = 32;

/// A scope's locals in the order the reference goes through them: by
/// the sum of the name's bytes modulo the table size, and within one
/// sum, the latest declared first.
pub(crate) fn enum_order(tab: &LocalTab, locals: &[Local]) -> Vec<LocalId> {
    let bucket = |id: &LocalId| {
        locals[*id]
            .name
            .iter()
            .map(|&byte| usize::from(byte))
            .sum::<usize>()
            % LOCAL_HASH_SIZE
    };
    let mut order: Vec<LocalId> = tab.entries.iter().rev().copied().collect();
    order.sort_by_key(bucket);
    order
}

/// A function.
#[derive(Debug, Clone)]
pub(crate) struct FuncSym {
    pub name: Vec<u8>,
    pub argc: usize,
    pub opt_argc: usize,
    pub varargs: bool,
    pub has_retval: bool,
    /// It is declared but not defined yet in what has been read.
    pub is_extern: bool,
    pub has_proto: bool,
    /// Its definition: an index into [`Program::bodies`].
    pub body: Option<usize>,
}

impl FuncSym {
    /// Whether a call with this many arguments is right.
    pub(crate) fn argc_ok(&self, argc: usize) -> bool {
        argc >= self.argc && (self.varargs || argc <= self.argc + self.opt_argc)
    }

    /// The number of arguments wanted, for a message.
    pub(crate) fn argc_desc(&self) -> String {
        if self.varargs {
            format!("{}+", self.argc)
        } else if self.opt_argc != 0 {
            format!("{}-{}", self.argc, self.argc + self.opt_argc)
        } else {
            self.argc.to_string()
        }
    }
}

/// A function built into the virtual machine.
#[derive(Debug, Clone)]
pub(crate) struct BifSym {
    pub name: Vec<u8>,
    /// Which set of functions it is in: an index into
    /// [`Program::fnsets`].
    pub set: usize,
    /// Its place in the set.
    pub index: usize,
    pub min_argc: usize,
    pub max_argc: usize,
    pub varargs: bool,
}

/// What a name means at the outermost level of the program.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Global {
    /// An index into [`Program::funcs`].
    Func(usize),
    /// An index into [`Program::bifs`].
    Bif(usize),
}

/// A program as the parser leaves it.
#[derive(Debug, Default)]
pub(crate) struct Program {
    pub locals: Vec<Local>,
    pub tabs: Vec<LocalTab>,
    pub funcs: Vec<FuncSym>,
    pub bifs: Vec<BifSym>,
    /// The sets of built-in functions the program uses, by name, in the
    /// order they were first declared.
    pub fnsets: Vec<Vec<u8>>,
    /// The number of elements in the longest list in the source.
    pub max_list_cnt: usize,
    pub globals: HashMap<Vec<u8>, Global>,
    /// The function bodies, in the order the source has them, each with
    /// the function it defines.
    pub bodies: Vec<(CodeBody, usize)>,
}

impl Program {
    /// The functions as another reading of the same source should see
    /// them before it starts: known, but not yet defined.
    pub(crate) fn exported_funcs(&self) -> Vec<FuncSym> {
        self.funcs
            .iter()
            .map(|func| FuncSym {
                is_extern: true,
                body: None,
                ..func.clone()
            })
            .collect()
    }
}

/// What to put back when a scope is left.
struct Scope {
    local_symtab: Option<TabId>,
    enclosing_symtab: Option<TabId>,
}

pub(crate) struct Parser<'p, 'a> {
    tok: &'p mut Preprocessor<'a>,
    /// Reading only to learn what the program defines: nothing is
    /// generated from this pass, and some warnings are left for the pass
    /// that does generate.
    syntax_only: bool,
    prog: Program,

    // The function being read.
    local_symtab: Option<TabId>,
    enclosing_local_symtab: Option<TabId>,
    local_cnt: usize,
    max_local_cnt: usize,
    /// Where the statement being read starts.
    cur_file: usize,
    cur_line: u32,
}

type Parsed<T> = Result<T, Stop>;

/// The parser cannot go on: the source has ended in the middle of
/// something.
#[derive(Debug)]
pub(crate) struct Stop;

impl<'p, 'a> Parser<'p, 'a> {
    pub(crate) fn new(
        tok: &'p mut Preprocessor<'a>,
        syntax_only: bool,
        known_funcs: Vec<FuncSym>,
    ) -> Self {
        let mut prog = Program::default();
        for func in known_funcs {
            prog.globals
                .insert(func.name.clone(), Global::Func(prog.funcs.len()));
            prog.funcs.push(func);
        }
        Self {
            tok,
            syntax_only,
            prog,
            local_symtab: None,
            enclosing_local_symtab: None,
            local_cnt: 0,
            max_local_cnt: 0,
            cur_file: 0,
            cur_line: 0,
        }
    }

    // ---- tokens and messages ----

    fn cur(&self) -> Tok {
        self.tok.cur_token().typ
    }

    fn next(&mut self) -> Tok {
        self.tok.next_token()
    }

    fn cur_text(&self) -> Vec<u8> {
        self.tok.cur_token().text.clone()
    }

    fn cur_is_sym(&self, text: &[u8]) -> bool {
        self.cur() == Tok::Sym && self.tok.cur_token().text == text
    }

    fn error(&mut self, message: impl Into<String>) {
        self.tok.log_error(message);
    }

    /// Report an error that names the current token: `%` in the message
    /// is where the token goes.
    fn error_curtok(&mut self, message: &str) {
        let shown = self.tok.cur_token_shown();
        self.tok.log_error(message.replacen('%', &shown, 1));
    }

    /// Something the compiler does not do yet.
    fn unsupported(&mut self, what: &str) {
        self.tok
            .log_error(format!("{what} is not supported by this compiler yet"));
    }

    /// A name as a message shows it.
    fn name(text: &[u8]) -> String {
        show_name(text)
    }

    fn stmt(&self, kind: StmtKind) -> Stmt {
        Stmt {
            kind,
            file: self.cur_file,
            line: self.cur_line,
        }
    }

    // ---- scopes and locals ----

    fn enter_scope(&mut self) -> Scope {
        let scope = Scope {
            local_symtab: self.local_symtab,
            enclosing_symtab: self.enclosing_local_symtab,
        };
        self.enclosing_local_symtab = self.local_symtab;
        scope
    }

    fn leave_scope(&mut self, scope: Scope) {
        self.local_symtab = scope.local_symtab;
        self.enclosing_local_symtab = scope.enclosing_symtab;
    }

    /// Give the current scope a table of its own, if it is still using
    /// the one of the scope around it.
    fn create_scope_local_symtab(&mut self) {
        if self.local_symtab == self.enclosing_local_symtab {
            self.prog.tabs.push(LocalTab {
                parent: self.local_symtab,
                ..LocalTab::default()
            });
            self.local_symtab = Some(self.prog.tabs.len() - 1);
        }
    }

    fn alloc_local(&mut self) -> usize {
        self.max_local_cnt = self.max_local_cnt.max(self.local_cnt + 1);
        self.local_cnt += 1;
        self.local_cnt - 1
    }

    fn find_direct(&self, tab: TabId, name: &[u8]) -> Option<LocalId> {
        self.prog.tabs[tab]
            .entries
            .iter()
            .copied()
            .find(|&id| self.prog.locals[id].name == name)
    }

    /// Look a name up in the scopes of the function being read.
    fn find_local(&self, name: &[u8]) -> Option<LocalId> {
        let mut tab = self.local_symtab;
        while let Some(id) = tab {
            if let Some(found) = self.find_direct(id, name) {
                return Some(found);
            }
            tab = self.prog.tabs[id].parent;
        }
        None
    }

    fn add_local_entry(&mut self, name: &[u8], is_param: bool, var_num: usize) -> Option<LocalId> {
        let tab = self.local_symtab.expect("a scope's own table");
        if self.find_direct(tab, name).is_some() {
            let what = if is_param {
                "formal parameter"
            } else {
                "local variable"
            };
            self.error(format!(
                "{what} \"{}\" defined more than once",
                Self::name(name)
            ));
            return None;
        }
        let (file, line) = self.tok.last_pos();
        self.prog.locals.push(Local {
            name: name.to_vec(),
            is_param,
            var_num,
            is_list_param: false,
            val_used: false,
            val_assigned: false,
            file,
            line,
        });
        let id = self.prog.locals.len() - 1;
        self.prog.tabs[tab].entries.push(id);
        Some(id)
    }

    /// Declare a local variable. The reference takes the next slot
    /// whether or not the name turns out to be usable.
    fn add_local(&mut self, name: &[u8]) -> Option<LocalId> {
        let var_num = self.alloc_local();
        self.add_local_entry(name, false, var_num)
    }

    // ---- the top level ----

    /// Read the whole source.
    pub(crate) fn parse_top(mut self) -> Program {
        self.next();
        let mut suppress_error = false;
        loop {
            let mut suppress_next_error = false;
            let result = match self.cur() {
                Tok::Eof => break,
                Tok::Function | Tok::Method => self.parse_function(true),
                Tok::Sym => self.parse_object_or_func(suppress_error, &mut suppress_next_error),
                Tok::Sem => {
                    self.next();
                    Ok(())
                }
                Tok::Intrinsic => self.parse_intrinsic(),
                Tok::Extern
                | Tok::Transient
                | Tok::Object
                | Tok::Plus
                | Tok::Inc
                | Tok::Class
                | Tok::Replace
                | Tok::Modify
                | Tok::Property
                | Tok::Export
                | Tok::Dictionary
                | Tok::Grammar
                | Tok::Enum => {
                    if !suppress_error {
                        let shown = self.tok.cur_token_shown();
                        self.unsupported(&format!("a definition starting with \"{shown}\""));
                    }
                    suppress_next_error = true;
                    self.next();
                    Ok(())
                }
                _ => {
                    if !suppress_error {
                        self.error_curtok(
                            "expected function or object definition, but found \"%\"",
                        );
                    }
                    suppress_next_error = true;
                    self.next();
                    Ok(())
                }
            };
            if result.is_err() {
                break;
            }
            suppress_error = suppress_next_error;
        }
        self.prog
    }

    fn parse_object_or_func(
        &mut self,
        suppress_error: bool,
        suppress_next_error: &mut bool,
    ) -> Parsed<()> {
        match self.next() {
            Tok::LPar => {
                self.tok.unget_token();
                self.parse_function(false)
            }
            Tok::Colon => {
                self.unsupported("an object definition");
                *suppress_next_error = true;
                Ok(())
            }
            _ => {
                if !suppress_error {
                    self.error_curtok("expected function or object definition, but found \"%\"");
                }
                *suppress_next_error = true;
                Ok(())
            }
        }
    }

    fn parse_function(&mut self, func_kw_present: bool) -> Parsed<()> {
        if func_kw_present {
            self.next();
        }
        if self.cur() != Tok::Sym {
            self.error_curtok("expected symbol after 'function', but found \"%\"");
            return Ok(());
        }
        let name = self.cur_text();
        let (sym_file, sym_line) = self.tok.last_pos();
        match self.next() {
            Tok::Sem => {
                // A declaration ahead of the definition says nothing new.
                self.next();
                return Ok(());
            }
            Tok::LPar | Tok::LBrace => {}
            _ => {
                self.error_curtok("expected ';', '(', or '{', but found \"%\"");
                return Ok(());
            }
        }
        let Some((body, has_retval)) = self.parse_code_body()? else {
            return Ok(());
        };
        let (argc, opt_argc, varargs) = (body.argc, body.opt_argc, body.varargs);

        let existing = self.prog.globals.get(&name).copied();
        let func_id = match existing {
            Some(Global::Bif(_)) => {
                self.tok.log_at(
                    sym_file,
                    sym_line,
                    Severity::Error,
                    format!(
                        "symbol \"{}\" is already defined - can't redefine as function",
                        Self::name(&name)
                    ),
                );
                return Ok(());
            }
            Some(Global::Func(id)) => {
                let func = &self.prog.funcs[id];
                let mut redef = !func.is_extern;
                if func.has_proto {
                    redef |= func.argc != argc || func.varargs != varargs;
                }
                if redef {
                    let message = if func.is_extern {
                        "function \"%\" has an incompatible previous declaration"
                    } else {
                        "function \"%\" is already defined"
                    };
                    self.tok.log_at(
                        sym_file,
                        sym_line,
                        Severity::Error,
                        message.replacen('%', &Self::name(&name), 1),
                    );
                    return Ok(());
                }
                self.prog.funcs[id].is_extern = false;
                id
            }
            None => {
                self.prog.funcs.push(FuncSym {
                    name: name.clone(),
                    argc,
                    opt_argc,
                    varargs,
                    has_retval,
                    is_extern: false,
                    has_proto: true,
                    body: None,
                });
                let id = self.prog.funcs.len() - 1;
                self.prog.globals.insert(name, Global::Func(id));
                id
            }
        };
        self.prog.funcs[func_id].body = Some(self.prog.bodies.len());
        self.prog.bodies.push((body, func_id));
        Ok(())
    }

    /// Note that the program uses a set of built-in functions, and give
    /// the set's number. A set's name is a name proper, a slash, and a
    /// version; of two declarations of a set, the later version is
    /// kept.
    fn add_fnset(&mut self, name: &[u8]) -> usize {
        // No more of a version is kept than the slash and six characters.
        const VERSION_LEN: usize = 7;
        let split = |text: &[u8]| text.iter().position(|&c| c == b'/').unwrap_or(text.len());
        let slash = split(name);
        let version = &name[slash..];
        for (index, known) in self.prog.fnsets.iter_mut().enumerate() {
            let known_slash = split(known);
            if known[..known_slash] != name[..slash] {
                continue;
            }
            if known_slash < known.len() && !version.is_empty() {
                // Both have versions. The reference compares as many
                // bytes as the new one has, and if the new is greater,
                // writes those bytes over the start of the old:
                // `/123456789` and then `/2` leave `/223456789`.
                let version = &version[..version.len().min(VERSION_LEN)];
                let old: Vec<u8> = (0..version.len())
                    .map(|i| known.get(known_slash + i).copied().unwrap_or(0))
                    .collect();
                if version > old.as_slice() {
                    let end = known_slash + version.len();
                    if known.len() < end {
                        known.resize(end, 0);
                    }
                    known[known_slash..end].copy_from_slice(version);
                }
            } else if known_slash == known.len() {
                known.extend_from_slice(&version[..version.len().min(VERSION_LEN)]);
            }
            return index;
        }
        self.prog.fnsets.push(name.to_vec());
        self.prog.fnsets.len() - 1
    }

    /// Read `intrinsic 'set-name' { name(parameters); ... }`: the
    /// functions of a set built into the virtual machine, in the order
    /// the set has them.
    fn parse_intrinsic(&mut self) -> Parsed<()> {
        let set = match self.next() {
            Tok::SStr => {
                let name = self.cur_text();
                self.next();
                self.add_fnset(&name)
            }
            Tok::Class => {
                self.unsupported("an intrinsic class");
                self.next();
                return Ok(());
            }
            _ => {
                self.error_curtok(
                    "expected function set name string after \"intrinsic\", but found \"%\"",
                );
                0
            }
        };
        if self.cur() == Tok::LBrace {
            self.next();
        } else {
            self.error_curtok("expected '{' after intrinsic name, but found \"%\"");
        }
        let mut index = 0;
        loop {
            match self.cur() {
                Tok::RBrace => {
                    self.next();
                    return Ok(());
                }
                Tok::Eof => {
                    self.error("end of file in \"intrinsic\" list - '}' is probably missing");
                    return Err(Stop);
                }
                Tok::Sem => {
                    self.next();
                }
                Tok::Sym => {
                    let name = self.cur_text();
                    if self.next() == Tok::LPar {
                        self.next();
                    } else {
                        self.error_curtok(
                            "expected '(' after function name in intrinsic list, but found \"%\"",
                        );
                    }
                    // Only the number of parameters matters.
                    let mut formals = Formals {
                        count_only: true,
                        ..Formals::default()
                    };
                    self.parse_formal_list(&mut formals)?;
                    self.parse_req_sem()?;
                    self.prog
                        .globals
                        .insert(name.clone(), Global::Bif(self.prog.bifs.len()));
                    self.prog.bifs.push(BifSym {
                        name,
                        set,
                        index,
                        min_argc: formals.argc,
                        max_argc: formals.argc + formals.opt_argc,
                        varargs: formals.varargs,
                    });
                    index += 1;
                }
                _ => {
                    self.error_curtok("expected function name in intrinsic list, but found \"%\"");
                    self.next();
                }
            }
        }
    }

    /// Read a function's parameters and body. Nothing comes back if the
    /// body could not be made sense of.
    fn parse_code_body(&mut self) -> Parsed<Option<(CodeBody, bool)>> {
        self.local_symtab = None;
        self.enclosing_local_symtab = None;
        self.local_cnt = 0;
        self.max_local_cnt = 0;

        let mut formals = Formals::default();
        if self.cur() == Tok::LPar {
            self.next();
            self.parse_formal_list(&mut formals)?;
        }
        if self.cur() != Tok::LBrace {
            self.error_curtok("expected '{' at start of method code body, but found \"%\"");
            if matches!(self.cur(), Tok::Sem | Tok::RBrace) {
                // It looks like the end of the definition: take the body
                // to be empty.
                let stm = self.stmt(StmtKind::Comp {
                    stmts: Vec::new(),
                    symtab: None,
                });
                return Ok(Some((self.finish_code_body(formals, stm), false)));
            }
        }
        let stm = self.parse_compound(true, true, true)?;

        let flags = self.control_flow(&stm, true);
        if !self.syntax_only {
            let warning = if flags & flow::RET_VAL != 0 && flags & flow::RET_VOID != 0 {
                Some("code has \"return\" statements both with and without values")
            } else if flags & flow::RET_VAL != 0 && flags & flow::NEXT != 0 {
                Some("code has \"return\" with value but also falls off end")
            } else {
                None
            };
            if let Some(message) = warning {
                self.tok
                    .log_at(stm.file, stm.line, Severity::Warning, message.to_owned());
            }
        }
        let has_retval = flags & flow::RET_VAL != 0;
        Ok(Some((self.finish_code_body(formals, stm), has_retval)))
    }

    fn finish_code_body(&mut self, formals: Formals, stm: Stmt) -> CodeBody {
        let (end_file, end_line) = self.tok.last_pos();
        CodeBody {
            lcltab: self.local_symtab,
            stm,
            argc: formals.argc,
            opt_argc: 0,
            varargs: formals.varargs,
            varargs_list_local: formals.varargs_list_local,
            local_cnt: self.max_local_cnt,
            replaced: false,
            end_file,
            end_line,
        }
    }

    /// Read a parameter list. The opening parenthesis has been read.
    fn parse_formal_list(&mut self, formals: &mut Formals) -> Parsed<()> {
        const MISSING_RPAR: &str =
            "missing right parenthesis ')' at end of formal parameter list - found \"%\"";
        let mut done = false;
        if self.cur() == Tok::RPar {
            done = true;
            self.next();
        }
        while !done {
            match self.cur() {
                Tok::Ellipsis => {
                    formals.varargs = true;
                    done = self.after_ellipsis(MISSING_RPAR);
                }
                Tok::LBrack => {
                    formals.varargs = true;
                    match self.next() {
                        Tok::RBrack => done = self.after_ellipsis(MISSING_RPAR),
                        Tok::Sym => {
                            if !formals.count_only {
                                self.create_scope_local_symtab();
                                let name = self.cur_text();
                                let id = self.add_local(&name);
                                if let Some(id) = id {
                                    let local = &mut self.prog.locals[id];
                                    local.val_assigned = true;
                                    local.is_list_param = true;
                                }
                                formals.varargs_list_local = id;
                            }
                            match self.next() {
                                Tok::RBrack => {
                                    if self.next() == Tok::RPar {
                                        self.next();
                                        done = true;
                                    } else {
                                        match self.cur() {
                                            Tok::Comma => self.error(
                                                "variable-argument list parameter must be the last parameter",
                                            ),
                                            Tok::LBrace | Tok::RBrace | Tok::LPar | Tok::Eof => {
                                                self.error_curtok(MISSING_RPAR);
                                                done = true;
                                            }
                                            _ => {
                                                self.error_curtok(MISSING_RPAR);
                                                if self.next() == Tok::RPar {
                                                    self.next();
                                                }
                                                done = true;
                                            }
                                        }
                                    }
                                }
                                Tok::LBrace | Tok::RBrace | Tok::Eof => {
                                    self.error_curtok(
                                        "expected ']' after variable-argument list parameter, but found \"%\"",
                                    );
                                    done = true;
                                }
                                _ => {
                                    self.error_curtok(
                                        "expected ']' after variable-argument list parameter, but found \"%\"",
                                    );
                                    self.next();
                                    done = true;
                                }
                            }
                        }
                        _ => {
                            self.error_curtok(
                                "variable-argument list parameter name expected, but found \"%\"",
                            );
                            match self.cur() {
                                Tok::RPar => {
                                    self.next();
                                    done = true;
                                }
                                Tok::RBrace | Tok::LBrace | Tok::Eof => done = true,
                                _ => {
                                    self.next();
                                }
                            }
                        }
                    }
                }
                Tok::Sym => {
                    let name = self.cur_text();
                    self.next();
                    let mut named = false;
                    if formals.count_only {
                        // A parameter of a built-in function: it can be
                        // named, and it can be optional.
                        if self.cur() == Tok::Colon {
                            named = true;
                            self.next();
                        }
                        let suffix = self.cur();
                        let optional = matches!(suffix, Tok::Question | Tok::Eq);
                        if optional {
                            if !named {
                                formals.opt_argc += 1;
                            }
                            self.next();
                            if suffix == Tok::Eq {
                                self.parse_asi_expr();
                            }
                        }
                        if formals.opt_argc != 0 && !optional && !named {
                            self.error(format!(
                                "parameter '{}' must be optional because it follows an optional parameter",
                                String::from_utf8_lossy(&name)
                            ));
                        }
                    } else {
                        match self.cur() {
                            Tok::Colon => self.unsupported("a named parameter"),
                            Tok::Question | Tok::Eq => self.unsupported("an optional parameter"),
                            Tok::Sym if !self.cur_is_sym(b"multimethod") => {
                                self.unsupported("a parameter with a type");
                            }
                            _ => {}
                        }
                        self.create_scope_local_symtab();
                        self.add_local_entry(&name, true, formals.argc);
                    }
                    if !named {
                        formals.argc += 1;
                    }
                    match self.cur() {
                        Tok::RPar => {
                            self.next();
                            done = true;
                        }
                        Tok::Comma => {
                            self.next();
                        }
                        Tok::Sem | Tok::RBrace | Tok::LBrace | Tok::Eq | Tok::Eof => {}
                        Tok::Sym => self.error_curtok(
                            "expected a comma in formal parameter list, but found \"%\"",
                        ),
                        _ => {
                            self.error_curtok(
                                "expected a comma in formal parameter list, but found \"%\"",
                            );
                            self.next();
                        }
                    }
                }
                Tok::Sem | Tok::RBrace | Tok::LBrace | Tok::Eof => {
                    self.error_curtok(MISSING_RPAR);
                    done = true;
                }
                Tok::RPar => {
                    self.error("missing parameter name at end of formal parameter list");
                    self.next();
                    done = true;
                }
                _ => {
                    self.error_curtok(
                        "expected a symbol in formal parameter list, but found \"%\"",
                    );
                    self.next();
                }
            }
        }
        if self.cur_is_sym(b"multimethod") {
            self.unsupported("a multi-method");
            self.next();
        }
        // The count is of the parameters that have to be there.
        formals.argc -= formals.opt_argc;
        Ok(())
    }

    /// What follows `...` in a parameter list. True if the list is over.
    fn after_ellipsis(&mut self, missing_rpar: &str) -> bool {
        if self.next() == Tok::RPar {
            self.next();
            return true;
        }
        if self.cur() == Tok::Comma {
            self.next();
            self.error("'...' cannot be followed by additional formal parameters");
            false
        } else {
            self.error_curtok(missing_rpar);
            true
        }
    }

    // ---- statements ----

    fn parse_compound(
        &mut self,
        skip_lbrace: bool,
        need_rbrace: bool,
        use_enclosing_scope: bool,
    ) -> Parsed<Stmt> {
        let (file, line) = self.tok.last_pos();
        if skip_lbrace && self.cur() == Tok::LBrace {
            self.next();
        }
        let scope = (!use_enclosing_scope).then(|| self.enter_scope());
        let mut stmts = Vec::new();
        let mut skip_rbrace = false;
        loop {
            match self.cur() {
                Tok::RBrace => {
                    skip_rbrace = true;
                    break;
                }
                Tok::Eof => {
                    if need_rbrace {
                        self.error("unexpected end of file in code block - '}' missing");
                    }
                    break;
                }
                _ => {
                    if let Some(stmt) = self.parse_stm(false)? {
                        stmts.push(stmt);
                    }
                }
            }
        }
        if stmts.is_empty() {
            stmts.push(self.stmt(StmtKind::Null));
        }
        let comp = Stmt {
            kind: StmtKind::Comp {
                stmts,
                symtab: self.local_symtab,
            },
            file,
            line,
        };
        if skip_rbrace {
            self.next();
        }
        if let Some(scope) = scope {
            self.leave_scope(scope);
        }
        Ok(comp)
    }

    /// Read a statement. Some statements leave nothing behind: an empty
    /// one, or a `local` with no initial values.
    fn parse_stm(&mut self, compound_use_enclosing_scope: bool) -> Parsed<Option<Stmt>> {
        (self.cur_file, self.cur_line) = self.tok.last_pos();
        loop {
            return match self.cur() {
                Tok::Eof => {
                    self.error("unexpected end of file in code block - '}' missing");
                    Err(Stop)
                }
                Tok::DStrMid | Tok::DStrEnd | Tok::RBrace => {
                    self.error_curtok("expected start of statement, but found \"%\"");
                    self.next();
                    Ok(None)
                }
                Tok::Sem => {
                    self.next();
                    Ok(None)
                }
                Tok::Local => {
                    self.create_scope_local_symtab();
                    self.parse_local()
                }
                Tok::LBrace => self
                    .parse_compound(true, true, compound_use_enclosing_scope)
                    .map(Some),
                Tok::If => self.parse_if(),
                Tok::Return => self.parse_return(),
                Tok::For => self.parse_for(),
                Tok::While => self.parse_while(),
                Tok::Do => self.parse_do_while(),
                Tok::Break => self.parse_break_or_continue(true),
                Tok::Continue => self.parse_break_or_continue(false),
                Tok::Else => {
                    self.error("misplaced \"else\" - no corresponding \"if\" statement");
                    self.next();
                    Ok(None)
                }
                Tok::Foreach
                | Tok::Switch
                | Tok::Goto
                | Tok::Try
                | Tok::Throw
                | Tok::Catch
                | Tok::Finally
                | Tok::Case
                | Tok::Default => {
                    let shown = self.tok.cur_token_shown();
                    self.unsupported(&format!("a \"{shown}\" statement"));
                    self.next();
                    Ok(None)
                }
                Tok::Sym => {
                    if self.next() == Tok::Colon {
                        self.unsupported("a statement label");
                        self.next();
                        continue;
                    }
                    self.tok.unget_token();
                    self.parse_expr_stm()
                }
                Tok::RPar => {
                    self.error("unexpected ')' found - ignored");
                    self.next();
                    continue;
                }
                _ => self.parse_expr_stm(),
            };
        }
    }

    fn parse_expr_stm(&mut self) -> Parsed<Option<Stmt>> {
        let expr = self.parse_expr();
        self.parse_req_sem()?;
        Ok(expr.map(|expr| self.stmt(StmtKind::Expr(expr))))
    }

    /// Read the semicolon that ends a statement.
    fn parse_req_sem(&mut self) -> Parsed<()> {
        match self.cur() {
            Tok::Sem => {
                self.next();
                Ok(())
            }
            Tok::RPar => {
                self.error("unexpected ')' found - ignored");
                self.skip_to_sem()
            }
            Tok::RBrack => {
                self.error("unexpected ']' found - ignored");
                self.skip_to_sem()
            }
            Tok::Eof => {
                self.error("expected ';', but found \"<end of file>\"");
                Err(Stop)
            }
            _ => {
                self.error_curtok("expected ';', but found \"%\"");
                Ok(())
            }
        }
    }

    /// Skip what is left of a statement that has gone wrong.
    fn skip_to_sem(&mut self) -> Parsed<()> {
        loop {
            match self.cur() {
                Tok::Eof => return Err(Stop),
                Tok::Sem => {
                    self.next();
                    return Ok(());
                }
                Tok::LBrace | Tok::RBrace => return Ok(()),
                _ => {
                    self.next();
                }
            }
        }
    }

    fn parse_local(&mut self) -> Parsed<Option<Stmt>> {
        let mut stmts = Vec::new();
        self.next();
        loop {
            if self.cur() == Tok::Sym {
                let name = self.cur_text();
                let local = self.add_local(&name);
                if matches!(self.next(), Tok::Eq | Tok::Asi) {
                    let Some(expr) = self.parse_local_initializer(local) else {
                        break;
                    };
                    let (file, line) = self.tok.last_pos();
                    stmts.push(Stmt {
                        kind: StmtKind::Expr(expr),
                        file,
                        line,
                    });
                }
                match self.cur() {
                    Tok::Comma => {
                        self.next();
                    }
                    Tok::Sem => {
                        self.next();
                        break;
                    }
                    Tok::Sym => self
                        .error_curtok("expected ',' or ';' after local variable, but found \"%\""),
                    _ => {
                        self.error_curtok(
                            "expected ',' or ';' after local variable, but found \"%\"",
                        );
                        break;
                    }
                }
            } else {
                self.error_curtok(
                    "expected symbol name in local variable declaration, but found \"%\"",
                );
                match self.cur() {
                    Tok::Sem => {
                        self.next();
                        break;
                    }
                    Tok::Eof => return Err(Stop),
                    _ => {
                        self.next();
                    }
                }
            }
        }
        Ok(match stmts.len() {
            0 => None,
            1 => stmts.pop(),
            _ => Some(self.stmt(StmtKind::Comp {
                stmts,
                symtab: self.local_symtab,
            })),
        })
    }

    /// Read `= value` after a local's name. The current token is the
    /// assignment operator.
    fn parse_local_initializer(&mut self, local: Option<LocalId>) -> Option<Expr> {
        self.next();
        let expr = self.parse_asi_expr()?;
        Some(match local {
            Some(id) => Expr::Assign(AsiOp::Simple, Box::new(Expr::Local(id)), Box::new(expr)),
            None => expr,
        })
    }

    fn parse_if(&mut self) -> Parsed<Option<Stmt>> {
        let (file, line) = self.tok.last_pos();
        if self.next() == Tok::LPar {
            self.next();
        } else {
            self.error_curtok("expected '(' after \"if\", but found \"%\"");
        }
        let Some(cond) = self.parse_cond_expr() else {
            return Err(Stop);
        };
        if self.cur() == Tok::RPar {
            self.next();
        } else {
            self.error_curtok("expected ')' after the \"if\" condition, but found \"%\"");
        }
        let then_part = self.parse_stm(false)?.map(Box::new);
        let else_part = if self.cur() == Tok::Else {
            self.next();
            self.parse_stm(false)?.map(Box::new)
        } else {
            None
        };
        Ok(Some(Stmt {
            kind: StmtKind::If {
                cond,
                then_part,
                else_part,
            },
            file,
            line,
        }))
    }

    fn parse_return(&mut self) -> Parsed<Option<Stmt>> {
        match self.next() {
            Tok::Sem => {
                self.next();
                Ok(Some(self.stmt(StmtKind::Return(None))))
            }
            Tok::LBrace | Tok::RBrace => {
                self.error_curtok("expected ';' or expression after \"return\", but found \"%\"");
                Ok(None)
            }
            _ => {
                // The reference makes the statement even when the
                // expression is missing; a missing expression here means
                // the source has ended, which stops everything below.
                let expr = self.parse_expr();
                self.parse_req_sem()?;
                Ok(Some(self.stmt(StmtKind::Return(expr))))
            }
        }
    }

    fn parse_for(&mut self) -> Parsed<Option<Stmt>> {
        const MISSING_PART: &str =
            "missing expression in \"for\" statement - expected ';', found '%'";
        let (file, line) = self.tok.last_pos();
        let scope = self.enter_scope();
        if self.next() == Tok::LPar {
            self.next();
        } else {
            self.error_curtok("expected '(' after \"for\", but found \"%\"");
        }
        let mut init: Option<Expr> = None;
        let mut cond = None;
        let mut reinit = None;
        loop {
            let mut done = false;
            let mut expr = None;
            match self.cur() {
                Tok::Local => {
                    self.create_scope_local_symtab();
                    if self.next() == Tok::Sym {
                        let name = self.cur_text();
                        let local = self.add_local(&name);
                        match self.next() {
                            Tok::Asi | Tok::Eq => expr = self.parse_local_initializer(local),
                            Tok::Sym if self.cur_is_sym(b"in") => {
                                self.unsupported("a \"for\" over a collection");
                                self.next();
                            }
                            _ => self.error_curtok(
                                "initializer expected after local variable name in \"for\", but found \"%\"",
                            ),
                        }
                    } else {
                        self.error_curtok(
                            "expected symbol name in local variable declaration, but found \"%\"",
                        );
                    }
                }
                Tok::Sem => {
                    done = true;
                    if init.is_some() {
                        self.error("missing expression after comma in \"for\" initializer");
                    }
                }
                Tok::RPar | Tok::LBrace | Tok::RBrace => {
                    self.error_curtok(MISSING_PART);
                    done = true;
                }
                _ => {
                    expr = self.parse_asi_expr();
                    if expr.is_none() {
                        done = true;
                    }
                    if self.cur_is_sym(b"in") {
                        self.unsupported("a \"for\" over a collection");
                        self.next();
                    }
                }
            }
            if let Some(expr) = expr {
                init = Some(match init.take() {
                    Some(so_far) => Expr::Binary(BinOp::Comma, Box::new(so_far), Box::new(expr)),
                    None => expr,
                });
            }
            if done {
                break;
            }
            match self.cur() {
                Tok::Sem => break,
                Tok::Comma => {
                    self.next();
                }
                Tok::RPar | Tok::LBrace | Tok::RBrace => {
                    self.error_curtok(MISSING_PART);
                    break;
                }
                _ => {
                    self.error_curtok(
                        "expected ',' or ';' in \"for\" initializer, but found \"%\"",
                    );
                    self.next();
                }
            }
        }
        if self.cur() == Tok::Sem {
            let mut cont_to_reinit = true;
            self.next();
            if self.cur() != Tok::Sem {
                cond = self.parse_cond_expr();
            }
            match self.cur() {
                Tok::Sem => {
                    self.next();
                }
                Tok::RPar | Tok::LBrace | Tok::RBrace => {
                    self.error_curtok(MISSING_PART);
                    cont_to_reinit = false;
                }
                _ => self.error_curtok("expected ';' after \"for\" condition, but found \"%\""),
            }
            if cont_to_reinit && self.cur() != Tok::RPar {
                reinit = self.parse_expr();
            }
            if self.cur() == Tok::RPar {
                self.next();
            } else {
                self.error_curtok("missing ')' at end of \"for\" expression list - found \"%\"");
            }
        } else if self.cur() == Tok::RPar {
            self.next();
        }
        let symtab = self.local_symtab;
        let body = self.parse_stm(false)?.map(Box::new);
        self.leave_scope(scope);
        Ok(Some(Stmt {
            kind: StmtKind::For {
                init,
                cond,
                reinit,
                body,
                symtab,
            },
            file,
            line,
        }))
    }

    fn parse_while(&mut self) -> Parsed<Option<Stmt>> {
        if self.next() == Tok::LPar {
            self.next();
        } else {
            self.error_curtok("missing '(' after \"while\" - found \"%\"");
        }
        let Some(cond) = self.parse_cond_expr() else {
            return Err(Stop);
        };
        // The statement takes its position from where the parser is
        // when it is made, which is here.
        let (file, line) = (self.cur_file, self.cur_line);
        if self.cur() == Tok::RPar {
            self.next();
        } else {
            self.error_curtok("missing ')' after \"while\" expression - found \"%\"");
        }
        let body = self.parse_stm(false)?.map(Box::new);
        Ok(Some(Stmt {
            kind: StmtKind::While { cond, body },
            file,
            line,
        }))
    }

    fn parse_do_while(&mut self) -> Parsed<Option<Stmt>> {
        let (file, line) = (self.cur_file, self.cur_line);
        self.next();
        let body = self.parse_stm(false)?.map(Box::new);
        if self.cur() == Tok::While {
            self.next();
        } else {
            self.error_curtok("expected \"while\" in \"do\" statement, but found \"%\"");
        }
        if self.cur() == Tok::LPar {
            self.next();
        } else {
            self.error_curtok("missing '(' after \"while\" - found \"%\"");
        }
        let Some(cond) = self.parse_cond_expr() else {
            return Err(Stop);
        };
        if self.cur() == Tok::RPar {
            self.next();
        } else {
            self.error_curtok("missing ')' after \"while\" expression - found \"%\"");
        }
        self.parse_req_sem()?;
        Ok(Some(Stmt {
            kind: StmtKind::DoWhile { body, cond },
            file,
            line,
        }))
    }

    fn parse_break_or_continue(&mut self, is_break: bool) -> Parsed<Option<Stmt>> {
        let (file, line) = (self.cur_file, self.cur_line);
        let mut label = None;
        match self.next() {
            Tok::Sym => {
                label = Some(self.cur_text());
                self.next();
            }
            Tok::Sem | Tok::LBrace | Tok::RBrace => {}
            _ => {
                let word = if is_break { "break" } else { "continue" };
                self.error_curtok(&format!(
                    "label name expected after \"{word}\", but found \"%\""
                ));
            }
        }
        self.parse_req_sem()?;
        let kind = if is_break {
            StmtKind::Break(label)
        } else {
            StmtKind::Continue(label)
        };
        Ok(Some(Stmt { kind, file, line }))
    }

    // ---- how statements can be left ----

    /// The ways a statement can be left, warning about code that cannot
    /// be reached.
    fn control_flow(&mut self, stmt: &Stmt, warn: bool) -> u32 {
        match &stmt.kind {
            StmtKind::Comp { stmts, .. } => {
                let mut flags = flow::NEXT;
                let mut index = 0;
                while index < stmts.len() {
                    let flags1 = self.control_flow(&stmts[index], warn);
                    flags |= flags1
                        & (flow::THROW
                            | flow::RET_VOID
                            | flow::RET_VAL
                            | flow::GOTO
                            | flow::BREAK
                            | flow::CONT);
                    if flags1 & flow::NEXT == 0 {
                        // Nothing after this can be reached. (A label
                        // could make it reachable again, when there are
                        // labels.)
                        flags &= !flow::NEXT;
                        if let Some(next) = stmts.get(index + 1) {
                            if warn && !self.syntax_only {
                                self.tok.log_at(
                                    next.file,
                                    next.line,
                                    Severity::Warning,
                                    "unreachable statement".to_owned(),
                                );
                            }
                        }
                        break;
                    }
                    index += 1;
                }
                flags
            }
            StmtKind::If {
                cond,
                then_part,
                else_part,
            } => {
                if let Some(value) = cond.as_const() {
                    let part = if value.is_true() {
                        then_part
                    } else {
                        else_part
                    };
                    return match part {
                        Some(part) => self.control_flow(part, warn),
                        None => flow::NEXT,
                    };
                }
                let flags1 = match then_part {
                    Some(part) => self.control_flow(part, warn),
                    None => flow::NEXT,
                };
                let flags2 = match else_part {
                    Some(part) => self.control_flow(part, warn),
                    None => flow::NEXT,
                };
                flags1 | flags2
            }
            StmtKind::Return(expr) => {
                if expr.is_some() {
                    flow::RET_VAL
                } else {
                    flow::RET_VOID
                }
            }
            StmtKind::For {
                cond, reinit, body, ..
            } => {
                let cond_const = cond.as_ref().and_then(Expr::as_const);
                if let Some(value) = cond_const {
                    if !value.is_true() && body.is_some() && !self.syntax_only {
                        let _ = reinit;
                        if warn && !value.ctc {
                            self.tok.log_at(
                                stmt.file,
                                stmt.line,
                                Severity::Warning,
                                "\"for\" condition is always false - body and reinitializer are unreachable"
                                    .to_owned(),
                            );
                        }
                        return flow::NEXT;
                    }
                }
                let mut flags = match body {
                    Some(body) => self.control_flow(body, warn),
                    None => 0,
                };
                flags &= !flow::NEXT;
                if flags & flow::BREAK != 0 {
                    flags |= flow::NEXT;
                }
                if cond.is_some() && cond_const.is_none_or(|value| !value.is_true()) {
                    flags |= flow::NEXT;
                }
                flags & !(flow::BREAK | flow::CONT)
            }
            StmtKind::While { cond, body } => {
                let cond_const = cond.as_const();
                if let Some(value) = cond_const {
                    // The reference only takes this way out when it is
                    // reading for symbols, so that is when it warns.
                    if !value.is_true() && body.is_some() && self.syntax_only {
                        if warn && !value.ctc {
                            self.tok.log_at(
                                stmt.file,
                                stmt.line,
                                Severity::Warning,
                                "\"while\" condition is always false - loop body is unreachable"
                                    .to_owned(),
                            );
                        }
                        return flow::NEXT;
                    }
                }
                let mut flags = match body {
                    Some(body) => self.control_flow(body, warn),
                    None => 0,
                };
                flags &= !flow::NEXT;
                if flags & flow::BREAK != 0 {
                    flags |= flow::NEXT;
                }
                if cond_const.is_none_or(|value| !value.is_true()) {
                    flags |= flow::NEXT;
                }
                flags & !(flow::BREAK | flow::CONT)
            }
            StmtKind::DoWhile { body, cond } => {
                let mut flags = match body {
                    Some(body) => self.control_flow(body, warn),
                    None => 0,
                };
                if flags & flow::NEXT != 0 {
                    flags &= !flow::NEXT;
                    if cond.as_const().is_none_or(|value| !value.is_true()) {
                        flags |= flow::NEXT;
                    }
                }
                if flags & flow::BREAK != 0 {
                    flags |= flow::NEXT;
                }
                flags & !(flow::BREAK | flow::CONT)
            }
            StmtKind::Break(label) => {
                if label.is_none() {
                    flow::BREAK
                } else {
                    flow::GOTO
                }
            }
            StmtKind::Continue(label) => {
                if label.is_none() {
                    flow::CONT
                } else {
                    flow::GOTO
                }
            }
            StmtKind::Null | StmtKind::Expr(_) => flow::NEXT,
        }
    }

    // ---- expressions ----

    /// Read an expression, commas and all.
    fn parse_expr(&mut self) -> Option<Expr> {
        self.parse_binary_level(0)
    }

    /// Read a condition. An assignment there is probably a slip for a
    /// comparison.
    fn parse_cond_expr(&mut self) -> Option<Expr> {
        let cond = self.parse_expr()?;
        if matches!(cond, Expr::Assign(AsiOp::Simple, ..)) && !self.syntax_only {
            self.tok.log_warning(
                "assignment in condition (possible use of '=' where '==' was intended)",
            );
        }
        Some(cond)
    }

    /// Read an expression in which a comma ends it: an argument, or an
    /// initial value.
    fn parse_asi_expr(&mut self) -> Option<Expr> {
        let lhs = self.parse_if_expr()?;
        let op = match self.cur() {
            Tok::PlusEq => AsiOp::Add,
            Tok::MinEq => AsiOp::Sub,
            Tok::TimesEq => AsiOp::Mul,
            Tok::DivEq => AsiOp::Div,
            Tok::ModEq => AsiOp::Mod,
            Tok::AndEq => AsiOp::BAnd,
            Tok::OrEq => AsiOp::BOr,
            Tok::XorEq => AsiOp::BXor,
            Tok::ShlEq => AsiOp::Shl,
            Tok::AShrEq => AsiOp::AShr,
            Tok::LShrEq => AsiOp::LShr,
            Tok::Eq => AsiOp::Simple,
            _ => return Some(lhs),
        };
        if !lhs.check_lvalue() {
            self.error(format!(
                "invalid lvalue - cannot assign to expression on left of \"{}\"",
                op.text()
            ));
        }
        self.next();
        // Assignments group from the right.
        let rhs = self.parse_asi_expr()?;
        Some(Expr::Assign(op, Box::new(lhs), Box::new(rhs)))
    }

    fn parse_if_expr(&mut self) -> Option<Expr> {
        let first = self.parse_ifnil_expr()?;
        if self.cur() != Tok::Question {
            return Some(first);
        }
        self.next();
        // The middle can be anything, a comma expression included.
        let second = self.parse_expr()?;
        if self.cur() != Tok::Colon {
            self.error("missing ':' in '?' conditional expression");
            if self.cur() == Tok::Eof {
                return None;
            }
        }
        self.next();
        let third = self.parse_asi_expr()?;
        Some(match first.as_const() {
            Some(value) => {
                if value.is_true() {
                    second
                } else {
                    third
                }
            }
            None => Expr::If(Box::new(first), Box::new(second), Box::new(third)),
        })
    }

    fn parse_ifnil_expr(&mut self) -> Option<Expr> {
        let first = self.parse_binary_level(1)?;
        if self.cur() != Tok::QQ {
            return Some(first);
        }
        self.next();
        let second = self.parse_asi_expr()?;
        Some(match first.as_const() {
            Some(value) => {
                if value.value == Value::Nil {
                    second
                } else {
                    first
                }
            }
            None => Expr::IfNil(Box::new(first), Box::new(second)),
        })
    }

    /// The operators of one level of precedence, from the loosest (the
    /// comma) to the tightest (multiplication).
    fn level_ops(level: usize) -> &'static [(Tok, BinOp)] {
        match level {
            0 => &[(Tok::Comma, BinOp::Comma)],
            1 => &[(Tok::OrOr, BinOp::Or)],
            2 => &[(Tok::AndAnd, BinOp::And)],
            3 => &[(Tok::Or, BinOp::BOr)],
            4 => &[(Tok::Xor, BinOp::BXor)],
            5 => &[(Tok::And, BinOp::BAnd)],
            6 => &[(Tok::EqEq, BinOp::Eq), (Tok::Ne, BinOp::Ne)],
            7 => &[
                (Tok::Gt, BinOp::Gt),
                (Tok::Ge, BinOp::Ge),
                (Tok::Lt, BinOp::Lt),
                (Tok::Le, BinOp::Le),
            ],
            8 => &[
                (Tok::Shl, BinOp::Shl),
                (Tok::AShr, BinOp::AShr),
                (Tok::LShr, BinOp::LShr),
            ],
            9 => &[(Tok::Plus, BinOp::Add), (Tok::Minus, BinOp::Sub)],
            _ => &[
                (Tok::Times, BinOp::Mul),
                (Tok::Div, BinOp::Div),
                (Tok::Mod, BinOp::Mod),
            ],
        }
    }

    const LAST_LEVEL: usize = 10;

    fn parse_operand_of(&mut self, level: usize) -> Option<Expr> {
        match level {
            // Under the comma is assignment, and under `||` there is
            // nothing left but the operators with two operands.
            0 => self.parse_asi_expr(),
            Self::LAST_LEVEL => self.parse_unary(),
            _ => self.parse_binary_level(level + 1),
        }
    }

    fn parse_binary_level(&mut self, level: usize) -> Option<Expr> {
        let mut lhs = self.parse_operand_of(level)?;
        loop {
            let cur = self.cur();
            let found = Self::level_ops(level)
                .iter()
                .find(|(tok, _)| *tok == cur)
                .map(|(_, op)| *op);
            let Some(op) = found else {
                // `is in` and `not in` are tests of equality too. Neither
                // word is a keyword.
                if level == 6 && (self.cur_is_sym(b"is") || self.cur_is_sym(b"not")) {
                    self.next();
                    if self.cur_is_sym(b"in") {
                        self.unsupported("the \"is in\" operator");
                    } else {
                        self.tok.unget_token();
                    }
                }
                return Some(lhs);
            };
            self.next();
            let rhs = self.parse_operand_of(level)?;
            lhs = self.fold_or_build(op, lhs, rhs);
        }
    }

    fn fold_or_build(&mut self, op: BinOp, lhs: Expr, rhs: Expr) -> Expr {
        let mut errors = Vec::new();
        let result = fold::binary(op, lhs, rhs, &mut errors);
        for message in errors {
            self.error(message);
        }
        match result {
            Folded::Yes(expr) => {
                if let Some(len) = expr.const_list_len() {
                    self.prog.max_list_cnt = self.prog.max_list_cnt.max(len);
                }
                expr
            }
            Folded::No(lhs, rhs) => Expr::Binary(op, Box::new(lhs), Box::new(rhs)),
        }
    }

    fn parse_unary(&mut self) -> Option<Expr> {
        let op = self.cur();
        match op {
            Tok::And => {
                self.unsupported("the address operator");
                self.next();
                self.parse_unary()
            }
            Tok::Delete => {
                self.unsupported("\"delete\"");
                self.next();
                self.parse_unary()
            }
            Tok::Not | Tok::BNot | Tok::Plus | Tok::Minus | Tok::Inc | Tok::Dec => {
                self.next();
                let sub = self.parse_unary()?;
                Some(match op {
                    Tok::Not => match fold::not(sub) {
                        Ok(folded) => folded,
                        Err(sub) => Expr::Unary(UnOp::Not, Box::new(sub)),
                    },
                    Tok::BNot => self.parse_bnot(sub),
                    Tok::Plus => self.parse_pos(sub),
                    Tok::Minus => self.parse_neg(sub),
                    Tok::Inc => self.parse_inc_dec(AsiOp::PreInc, sub),
                    _ => self.parse_inc_dec(AsiOp::PreDec, sub),
                })
            }
            _ => self.parse_postfix(),
        }
    }

    fn parse_bnot(&mut self, sub: Expr) -> Expr {
        match sub {
            Expr::Const(mut value) => {
                if let Value::Int(int) = value.value {
                    value.value = Value::Int(!int);
                } else {
                    self.error("unary '~' operator requires numeric value in constant expression");
                }
                Expr::Const(value)
            }
            other => Expr::Unary(UnOp::BNot, Box::new(other)),
        }
    }

    fn parse_pos(&mut self, sub: Expr) -> Expr {
        match sub {
            Expr::Const(value) => {
                if !matches!(value.value, Value::Int(_)) {
                    self.error("unary '+' operator requires numeric value in constant expression");
                }
                Expr::Const(value)
            }
            other => Expr::Unary(UnOp::Pos, Box::new(other)),
        }
    }

    fn parse_neg(&mut self, sub: Expr) -> Expr {
        match sub {
            Expr::Const(mut value) => {
                match value.value {
                    Value::Int(i32::MIN) => {
                        self.unsupported("a number too large for an integer");
                    }
                    Value::Int(int) => value.value = Value::Int(-int),
                    _ => self
                        .error("unary '-' operator requires numeric value in constant expression"),
                }
                Expr::Const(value)
            }
            other => Expr::Unary(UnOp::Neg, Box::new(other)),
        }
    }

    fn parse_inc_dec(&mut self, op: AsiOp, sub: Expr) -> Expr {
        if !sub.check_lvalue() {
            self.error(format!(
                "invalid lvalue - cannot apply \"{}\" operator to expression",
                op.text()
            ));
        }
        Expr::IncDec(op, Box::new(sub))
    }

    fn parse_postfix(&mut self) -> Option<Expr> {
        let mut sub = self.parse_primary()?;
        loop {
            match self.cur() {
                Tok::LPar => sub = self.parse_call(sub)?,
                Tok::LBrack => sub = self.parse_subscript(sub)?,
                Tok::Dot => {
                    self.unsupported("a property or method reference");
                    self.next();
                    return Some(sub);
                }
                Tok::Inc => {
                    self.next();
                    sub = self.parse_inc_dec(AsiOp::PostInc, sub);
                }
                Tok::Dec => {
                    self.next();
                    sub = self.parse_inc_dec(AsiOp::PostDec, sub);
                }
                _ => return Some(sub),
            }
        }
    }

    /// Read `[index]` after an expression.
    fn parse_subscript(&mut self, lhs: Expr) -> Option<Expr> {
        self.next();
        let index = self.parse_expr()?;
        if self.cur() == Tok::RBrack {
            self.next();
        } else {
            self.error_curtok("expected ']' at end of subscript, but found \"%\"");
        }
        let mut errors = Vec::new();
        let folded = fold::subscript(&lhs, &index, &mut errors);
        for message in errors {
            self.error(message);
        }
        Some(folded.unwrap_or_else(|| Expr::Index(Box::new(lhs), Box::new(index))))
    }

    /// Read a list. The current token is the opening bracket.
    fn parse_list(&mut self) -> Option<Expr> {
        const MISSING_RBRACK: &str = "expected ']' or a list element, but found \"%\"";
        self.next();
        let mut elements = Vec::new();
        'list: loop {
            match self.cur() {
                Tok::RBrack => {
                    self.next();
                    break;
                }
                Tok::Eof | Tok::RBrace | Tok::Sem | Tok::DStrMid | Tok::DStrEnd => {
                    self.error_curtok(MISSING_RBRACK);
                    break;
                }
                Tok::RPar => {
                    self.error("found extraneous ')' in list; ignored");
                    self.next();
                }
                _ => {}
            }
            elements.push(self.parse_asi_expr()?);
            match self.cur() {
                Tok::Comma => {
                    self.next();
                    if self.cur() == Tok::RBrack {
                        self.error(
                            "extra list element expected after comma, but found end of list",
                        );
                        break 'list;
                    }
                }
                Tok::Arrow => {
                    self.unsupported("a lookup table list");
                    self.next();
                }
                Tok::RBrack => {
                    self.next();
                    break;
                }
                Tok::Eof | Tok::LBrace | Tok::RBrace | Tok::Sem | Tok::DStrMid | Tok::DStrEnd => {
                    self.error_curtok(MISSING_RBRACK);
                    break;
                }
                _ => {
                    self.error_curtok("expected ',' separating list elements, but found \"%\"");
                }
            }
        }
        self.prog.max_list_cnt = self.prog.max_list_cnt.max(elements.len());
        Some(Expr::list(elements))
    }

    /// Read a call's arguments. The current token is the opening
    /// parenthesis.
    fn parse_arg_list(&mut self) -> Option<Vec<Arg>> {
        self.next();
        let mut args = Vec::new();
        loop {
            if self.cur() == Tok::RPar {
                break;
            }
            if self.cur() == Tok::Sym {
                if self.next() == Tok::Colon {
                    self.unsupported("a named argument");
                    self.next();
                } else {
                    self.tok.unget_token();
                }
            }
            let expr = self.parse_asi_expr()?;
            let mut arg = Arg {
                expr,
                varargs: false,
            };
            if self.cur() == Tok::Ellipsis {
                self.next();
                arg.varargs = true;
            }
            args.push(arg);
            match self.cur() {
                Tok::RPar => break,
                Tok::Comma => {
                    self.next();
                }
                Tok::Sem | Tok::LBrace | Tok::RBrace | Tok::DStrMid | Tok::DStrEnd => {
                    self.error_curtok("expected ')' at end of argument list, but found \"%\"");
                    return Some(args);
                }
                _ => {
                    self.error_curtok("expected ',' or ')' in argument list, but found \"%\"");
                    if self.cur() == Tok::Eof {
                        return Some(args);
                    }
                }
            }
        }
        self.next();
        Some(args)
    }

    fn parse_call(&mut self, lhs: Expr) -> Option<Expr> {
        let args = self.parse_arg_list()?;
        if let Expr::Sym(name) = &lhs {
            if name == b"defined" || name == b"__objref" {
                self.unsupported(&format!("\"{}\"", Self::name(name)));
            }
        }
        if args.len() > 127 {
            self.error("function call argument list exceeds machine limit (maximum 127)");
        }
        Some(Expr::Call(Box::new(lhs), args))
    }

    fn parse_primary(&mut self) -> Option<Expr> {
        loop {
            let value = match self.cur() {
                Tok::LPar => {
                    self.next();
                    let sub = self.parse_expr()?;
                    if self.cur() == Tok::RPar {
                        self.next();
                    } else {
                        self.error_curtok("expected ')' but found \"%\"");
                    }
                    return Some(sub);
                }
                Tok::Nil => Value::Nil,
                Tok::True => Value::True,
                Tok::Int => Value::Int(self.tok.cur_token().int_val as i32),
                Tok::SStr => Value::SStr(self.cur_text()),
                Tok::DStr => {
                    let text = self.cur_text();
                    self.next();
                    return Some(Expr::DStr(text));
                }
                Tok::LBrack => return self.parse_list(),
                Tok::Sym => {
                    let name = self.cur_text();
                    self.next();
                    return Some(match self.find_local(&name) {
                        Some(id) => Expr::Local(id),
                        None => Expr::Sym(name),
                    });
                }
                Tok::RPar => {
                    self.error("unexpected ')' found - ignored");
                    self.next();
                    continue;
                }
                Tok::RBrack => {
                    self.error("unexpected ']' found - ignored");
                    self.next();
                    continue;
                }
                Tok::DStrMid | Tok::DStrEnd | Tok::Sem | Tok::RBrace => {
                    // The statement seems to be over where an operand
                    // should be. Stay here, so that whoever asked can
                    // carry on from this token.
                    self.error_curtok("expected an operand, but found \"%\"");
                    return Some(Expr::constant(Value::Int(
                        self.tok.cur_token().int_val as i32,
                    )));
                }
                Tok::Float
                | Tok::BigInt
                | Tok::DStrStart
                | Tok::SStrStart
                | Tok::ReStr
                | Tok::LBrace
                | Tok::Method
                | Tok::Function
                | Tok::Object
                | Tok::New
                | Tok::SelfKw
                | Tok::Replaced
                | Tok::TargetProp
                | Tok::TargetObj
                | Tok::DefiningObj
                | Tok::Invokee
                | Tok::ArgCount
                | Tok::Inherited
                | Tok::Delegated => {
                    let shown = self.tok.cur_token_shown();
                    self.unsupported(&format!("\"{shown}\" in an expression"));
                    self.next();
                    return Some(Expr::constant(Value::Nil));
                }
                _ => {
                    self.error_curtok(
                        "expected integer, string, symbol, '[', or '(', but found \"%\"",
                    );
                    self.next();
                    Value::Int(self.tok.cur_token().int_val as i32)
                }
            };
            self.next();
            return Some(Expr::constant(value));
        }
    }
}

/// What a parameter list says about a function.
#[derive(Default)]
struct Formals {
    /// The list is only being read for how many parameters there are;
    /// they are not declared.
    count_only: bool,
    argc: usize,
    opt_argc: usize,
    varargs: bool,
    varargs_list_local: Option<LocalId>,
}
