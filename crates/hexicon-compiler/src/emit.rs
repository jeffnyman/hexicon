//! The code generator: a parsed program in, byte code out.
//!
//! Each kind of node in the syntax tree generates its code the way the
//! reference compiler's does, since the instruction writer's peephole
//! rules, the count of stack slots, and the tables after each function
//! all depend on exactly what is written and in what order.

use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use hexicon_bytecode::{Layout, Opcode as Op};

use crate::ast::{
    Arg, AsiOp, BinOp, Catch, CodeBody, ConstVal, Expr, Finally, LabelSym, LocalId, Stmt, StmtKind,
    TabId, UnOp, Value, flow,
};
use crate::codegen::{CodeGen, Label};
use crate::flow::Flow;
use crate::fold::{self, Folded};
use crate::parse::{BifSym, Global, Program, enum_order};
use crate::pp::{MAX_ERRORS, Severity, TOO_MANY_ERRORS, show_name};

/// The start of one item in a stream: a function in the code, or a
/// string in the constants. An item is never split across pages.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Anchor {
    pub ofs: usize,
}

/// What an address in the code refers to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Target {
    /// A function: an index into the program's functions.
    Func(usize),
    /// An item in the constants: an index into the constants' anchors.
    Const(usize),
}

/// Four bytes in the code that are to hold an address once it is known
/// where everything goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Fixup {
    pub ofs: usize,
    pub target: Target,
}

/// The kinds of value in a constant list, as the virtual machine
/// numbers them.
mod datatype {
    pub(super) const NIL: u8 = 1;
    pub(super) const TRUE: u8 = 2;
    pub(super) const INT: u8 = 7;
    pub(super) const SSTRING: u8 = 8;
    pub(super) const LIST: u8 = 10;
    pub(super) const FUNCPTR: u8 = 12;
}

/// The number of the list metaclass among those every image has.
const METACLASS_LIST: u8 = 1;

/// A problem found while generating, and where.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct GenDiagnostic {
    pub file: usize,
    pub line: u32,
    pub severity: Severity,
    pub message: String,
}

/// What the generator produces.
#[derive(Debug, Default)]
pub(crate) struct Generated {
    /// The code of all the functions, one after another.
    pub code: Vec<u8>,
    pub code_anchors: Vec<Anchor>,
    /// Where each function's code is: an index into `code_anchors`.
    pub func_anchor: HashMap<usize, usize>,
    /// The constants: strings, each with its length before it.
    pub consts: Vec<u8>,
    pub const_anchors: Vec<Anchor>,
    pub fixups: Vec<Fixup>,
    /// Addresses to fill in within the constants, where a list has a
    /// string, a list, or a function in it.
    pub const_fixups: Vec<Fixup>,
    /// The length of the longest function, tables included.
    pub max_bytecode_len: usize,
    /// The length of the longest string.
    pub max_str_len: usize,
    /// The number of elements in the longest list.
    pub max_list_cnt: usize,
    /// Places in the code where two bytes are written that mean
    /// nothing.
    pub unspecified: Vec<usize>,
    /// Places in the constants where four bytes are written that mean
    /// nothing.
    pub const_unspecified: Vec<usize>,
    pub diagnostics: Vec<GenDiagnostic>,
}

/// How many chains the reference's table of a function's labels has,
/// and its table of global names.
const LABEL_HASH_SIZE: usize = 512;

/// The functions that the code refers to but the program never defines,
/// in the order the reference reports them: the order of its table of
/// global names when it links.
pub(crate) fn unresolved_externs(prog: &Program, generated: &Generated) -> Vec<Vec<u8>> {
    let mut funcs: Vec<usize> = generated
        .fixups
        .iter()
        .chain(&generated.const_fixups)
        .filter_map(|fixup| match fixup.target {
            Target::Func(func) if prog.funcs[func].body.is_none() => Some(func),
            _ => None,
        })
        .collect();
    funcs.sort_unstable();
    funcs.dedup();
    funcs.sort_by_key(|&func| {
        prog.funcs[func]
            .name
            .iter()
            .map(|&byte| usize::from(byte))
            .sum::<usize>()
            % LABEL_HASH_SIZE
    });
    funcs
        .into_iter()
        .map(|func| prog.funcs[func].name.clone())
        .collect()
}

/// Strings shorter than this are stored once however often they occur.
const INTERN_THRESHOLD: usize = 40;

/// A statement around the one being generated that a jump out of it
/// has to know about.
#[derive(Clone, Copy)]
enum Enclosing {
    /// A loop: where `break` and `continue` go.
    Loop {
        id: usize,
        break_lbl: Label,
        cont_lbl: Label,
    },
    /// A `switch`: where `break` goes, if anything after the switch can
    /// be reached.
    Switch { id: usize, break_lbl: Option<Label> },
    /// A labeled statement.
    Label {
        id: usize,
        /// The label, unless its name was already taken.
        label: Option<usize>,
        /// Where a `break` that names the label goes, once there has
        /// been one.
        break_lbl: Option<Label>,
        /// The statement labeled is a loop, so `continue` can name the
        /// label.
        labels_loop: bool,
    },
    /// The protected part of a `try`, or one of its `catch` blocks. The
    /// way out is through the `finally` block, if there is one.
    Try {
        id: usize,
        finally_lbl: Option<Label>,
    },
    /// A `finally` block.
    Finally { id: usize },
}

impl Enclosing {
    fn id(&self) -> usize {
        match self {
            Enclosing::Loop { id, .. }
            | Enclosing::Switch { id, .. }
            | Enclosing::Label { id, .. }
            | Enclosing::Try { id, .. }
            | Enclosing::Finally { id } => *id,
        }
    }
}

/// The table of a `switch` being generated: where the next `case` and
/// the `default` write where they are.
struct SwitchSlots {
    next_case: usize,
    default_slot: usize,
}

/// One entry of a function's table of exception handlers. The offsets
/// are from the start of the function.
struct ExcEntry {
    start_ofs: usize,
    end_ofs: usize,
    /// The class of exception caught; zero for all of them.
    class: u32,
    handler_ofs: usize,
}

struct Gen<'p> {
    prog: &'p mut Program,
    cg: CodeGen,
    out: Generated,
    /// Where the function being generated starts.
    method_ofs: usize,
    /// The scope whose code is being generated, and the function's
    /// scopes in the order they were first entered.
    cur_frame: Option<TabId>,
    frames: Vec<TabId>,
    enclosing: Vec<Enclosing>,
    switches: Vec<SwitchSlots>,
    /// The labels of the function being generated, and where each one
    /// is, once something has asked.
    labels: Rc<Vec<LabelSym>>,
    /// The labels of the last function in the source.
    last_labels: Rc<Vec<LabelSym>>,
    goto_labels: Vec<Option<Label>>,
    exc_table: Vec<ExcEntry>,
    /// How many locals the function has. Generating its code can add to
    /// what the parser counted.
    local_cnt: usize,
    /// The local that holds a value being returned while `finally`
    /// blocks run.
    fin_ret_lcl: Option<usize>,
    /// Where the statement being generated is, for messages.
    cur_file: usize,
    cur_line: u32,
    interned: HashMap<Vec<u8>, usize>,
    /// Places in the code that refer to a local's name, in the order
    /// they were written. The names go into the constants after
    /// everything else.
    local_names: Vec<(usize, Vec<u8>)>,
    errors: usize,
    /// The undefined names reported so far.
    undefined_names: HashSet<Vec<u8>>,
    /// How many problems had been reported when the generator gave up
    /// for too many errors, if it has.
    gave_up: Option<usize>,
}

/// Generate the code for a program.
pub(crate) fn generate(prog: &mut Program) -> Generated {
    let mut bodies = std::mem::take(&mut prog.bodies);
    let mut generator = Gen {
        prog,
        cg: CodeGen::new(),
        out: Generated::default(),
        method_ofs: 0,
        cur_frame: None,
        frames: Vec::new(),
        enclosing: Vec::new(),
        switches: Vec::new(),
        labels: Rc::default(),
        last_labels: Rc::default(),
        goto_labels: Vec::new(),
        exc_table: Vec::new(),
        local_cnt: 0,
        fin_ret_lcl: None,
        cur_file: 0,
        cur_line: 0,
        interned: HashMap::new(),
        local_names: Vec::new(),
        errors: 0,
        undefined_names: HashSet::new(),
        gave_up: None,
    };
    generator.out.max_list_cnt = generator.prog.max_list_cnt;
    if let Some((body, _)) = bodies.last() {
        generator.last_labels = Rc::clone(&body.labels);
    }
    // Names are looked up, and what has become constant is worked out,
    // for the whole program before any code is generated.
    for (body, _) in &mut bodies {
        generator.fold_stmt(&mut body.stm);
    }
    // The reference stops here if that found anything wrong.
    if generator.errors != 0 {
        let out = generator.out;
        prog.bodies = bodies;
        return out;
    }
    let mut scopes = Vec::new();
    for (body, func) in &bodies {
        if generator.gave_up.is_some() {
            break;
        }
        generator.gen_code_body(body, *func);
        scopes.push(std::mem::take(&mut generator.frames));
    }
    // Locals are checked for use once everything has been generated.
    for frames in scopes {
        for frame in frames {
            generator.check_unreferenced_locals(frame);
        }
    }
    // Giving up for too many errors is the last thing said.
    if let Some(count) = generator.gave_up {
        generator.out.diagnostics.truncate(count);
    }
    generator.finish_local_names();
    let mut out = generator.out;
    out.code = generator.cg.code;
    prog.bodies = bodies;
    out
}

impl Gen<'_> {
    // ---- messages ----

    fn log(&mut self, severity: Severity, message: impl Into<String>) {
        if self.gave_up.is_some() {
            return;
        }
        self.out.diagnostics.push(GenDiagnostic {
            file: self.cur_file,
            line: self.cur_line,
            severity,
            message: message.into(),
        });
        if severity == Severity::Error {
            self.errors += 1;
            if self.errors > MAX_ERRORS {
                self.out.diagnostics.push(GenDiagnostic {
                    file: self.cur_file,
                    line: self.cur_line,
                    severity: Severity::Fatal,
                    message: TOO_MANY_ERRORS.to_owned(),
                });
                self.gave_up = Some(self.out.diagnostics.len());
            }
        }
    }

    fn error(&mut self, message: impl Into<String>) {
        self.log(Severity::Error, message);
    }

    fn set_line(&mut self, stmt: &Stmt) {
        self.cur_file = stmt.file;
        self.cur_line = stmt.line;
    }

    /// Warn about the locals of a scope that are never used or never
    /// given a value.
    fn check_unreferenced_locals(&mut self, tab: TabId) {
        for id in enum_order(&self.prog.tabs[tab], &self.prog.locals) {
            let local = &self.prog.locals[id];
            let param = local.is_param || local.is_list_param;
            let name = show_name(&local.name);
            let message = if !local.val_used && !local.val_assigned && !param {
                format!("local variable \"{name}\" is never used")
            } else if !local.val_used {
                if param {
                    // An unused parameter is only worth a mention to
                    // someone who asks for every last warning.
                    continue;
                }
                format!(
                    "value is assigned to local variable \"{name}\" but the value is never used"
                )
            } else if !local.val_assigned && !param {
                format!("no value is assigned to local variable \"{name}\"")
            } else {
                continue;
            };
            self.out.diagnostics.push(GenDiagnostic {
                file: local.file,
                line: local.line,
                severity: Severity::Warning,
                message,
            });
        }
    }

    // ---- constant folding, once names are known ----

    fn fold_stmt(&mut self, stmt: &mut Stmt) {
        self.set_line(stmt);
        match &mut stmt.kind {
            StmtKind::Null | StmtKind::Break(_) | StmtKind::Continue(_) | StmtKind::Goto(_) => {}
            StmtKind::Switch { expr, body, .. } => {
                self.fold_in_place(expr);
                self.fold_stmt(body);
            }
            StmtKind::Case { expr, stm } => {
                self.fold_in_place(expr);
                if !expr.is_const() {
                    self.error("\"case\" expression has a non-constant value");
                }
                if let Some(stm) = stm {
                    self.fold_stmt(stm);
                }
            }
            StmtKind::Default(stm) | StmtKind::Label { stm, .. } => {
                if let Some(stm) = stm {
                    self.fold_stmt(stm);
                }
            }
            StmtKind::Try {
                body,
                catches,
                finally,
                ..
            } => {
                if let Some(body) = body {
                    self.fold_stmt(body);
                }
                for catch in catches {
                    if let Some(body) = &mut catch.body {
                        self.fold_stmt(body);
                    }
                }
                if let Some(body) = finally.as_mut().and_then(|finally| finally.body.as_mut()) {
                    self.fold_stmt(body);
                }
            }
            StmtKind::Throw(expr) => self.fold_in_place(expr),
            StmtKind::Comp { stmts, .. } => {
                for stmt in stmts {
                    self.fold_stmt(stmt);
                }
            }
            StmtKind::Expr(expr) => self.fold_in_place(expr),
            StmtKind::If {
                cond,
                then_part,
                else_part,
            } => {
                self.fold_in_place(cond);
                if let Some(part) = then_part {
                    self.fold_stmt(part);
                }
                if let Some(part) = else_part {
                    self.fold_stmt(part);
                }
            }
            StmtKind::Return(expr) => {
                if let Some(expr) = expr {
                    self.fold_in_place(expr);
                }
            }
            StmtKind::For {
                init,
                cond,
                reinit,
                body,
                ..
            } => {
                for expr in [init, cond, reinit].into_iter().flatten() {
                    self.fold_in_place(expr);
                }
                if let Some(body) = body {
                    self.fold_stmt(body);
                }
            }
            StmtKind::While { cond, body, .. } | StmtKind::DoWhile { body, cond, .. } => {
                self.fold_in_place(cond);
                if let Some(body) = body {
                    self.fold_stmt(body);
                }
            }
        }
    }

    fn fold_in_place(&mut self, expr: &mut Expr) {
        let taken = std::mem::replace(expr, Expr::constant(Value::Nil));
        *expr = self.fold_expr(taken);
    }

    fn fold_expr(&mut self, expr: Expr) -> Expr {
        match expr {
            Expr::Const(_) | Expr::Local(_) => expr,
            Expr::Sym(name) => match self.prog.globals.get(&name) {
                Some(Global::Func(func)) => Expr::constant(Value::FuncPtr(*func)),
                Some(Global::Bif(_)) | None => Expr::Sym(name),
            },
            Expr::DStr(_) => expr,
            Expr::DStrEmbed(sub) => Expr::DStrEmbed(Box::new(self.fold_expr(*sub))),
            Expr::OneOf(dbl, elements) => {
                let elements = elements
                    .into_iter()
                    .map(|element| self.fold_expr(element))
                    .collect();
                Expr::OneOf(dbl, elements)
            }
            Expr::List(elements) => {
                let elements = elements
                    .into_iter()
                    .map(|element| self.fold_expr(element))
                    .collect();
                Expr::list(elements)
            }
            Expr::Index(left, index) => {
                let left = self.fold_expr(*left);
                let index = self.fold_expr(*index);
                let mut errors = Vec::new();
                let folded = fold::subscript(&left, &index, &mut errors);
                for message in errors {
                    self.error(message);
                }
                folded.unwrap_or_else(|| Expr::Index(Box::new(left), Box::new(index)))
            }
            Expr::Unary(op, sub) => {
                let sub = self.fold_expr(*sub);
                if op == UnOp::Not {
                    match fold::not(sub) {
                        Ok(folded) => folded,
                        Err(sub) => Expr::Unary(op, Box::new(sub)),
                    }
                } else {
                    Expr::Unary(op, Box::new(sub))
                }
            }
            Expr::Binary(op, left, right) => {
                let left = self.fold_expr(*left);
                let right = self.fold_expr(*right);
                // Only these operators are tried again; the rest were
                // folded as far as they can be when they were parsed.
                let again = matches!(
                    op,
                    BinOp::Comma
                        | BinOp::Add
                        | BinOp::Sub
                        | BinOp::Eq
                        | BinOp::Ne
                        | BinOp::And
                        | BinOp::Or
                );
                if !again {
                    return Expr::Binary(op, Box::new(left), Box::new(right));
                }
                let mut errors = Vec::new();
                let result = fold::binary(op, left, right, &mut errors);
                for message in errors {
                    self.error(message);
                }
                match result {
                    Folded::Yes(expr) => {
                        if let Some(len) = expr.const_list_len() {
                            self.out.max_list_cnt = self.out.max_list_cnt.max(len);
                        }
                        expr
                    }
                    Folded::No(left, right) => Expr::Binary(op, Box::new(left), Box::new(right)),
                }
            }
            Expr::Assign(op, left, right) => {
                let left = Box::new(self.fold_expr(*left));
                Expr::Assign(op, left, Box::new(self.fold_expr(*right)))
            }
            Expr::IncDec(op, sub) => Expr::IncDec(op, Box::new(self.fold_expr(*sub))),
            Expr::If(first, second, third) => {
                let first = self.fold_expr(*first);
                let second = self.fold_expr(*second);
                let third = self.fold_expr(*third);
                match first.as_const().map(ConstVal::is_true) {
                    Some(true) => second,
                    Some(false) => third,
                    None => Expr::If(Box::new(first), Box::new(second), Box::new(third)),
                }
            }
            Expr::IfNil(first, second) => {
                let first = self.fold_expr(*first);
                let second = self.fold_expr(*second);
                match first.as_const() {
                    Some(value) if value.value == Value::Nil => second,
                    Some(_) => first,
                    None => Expr::IfNil(Box::new(first), Box::new(second)),
                }
            }
            Expr::Call(func, args) => {
                let func = Box::new(self.fold_expr(*func));
                let args = args
                    .into_iter()
                    .map(|arg| Arg {
                        expr: self.fold_expr(arg.expr),
                        varargs: arg.varargs,
                    })
                    .collect();
                Expr::Call(func, args)
            }
        }
    }

    // ---- scopes ----

    fn rel_ofs(&self) -> usize {
        self.cg.ofs() - self.method_ofs
    }

    fn add_to_range(&mut self, tab: TabId, ofs: usize) {
        let tab = &mut self.prog.tabs[tab];
        if tab.start_ofs == 0 || ofs < tab.start_ofs {
            tab.start_ofs = ofs;
        }
        if ofs > tab.end_ofs {
            tab.end_ofs = ofs;
        }
    }

    /// Make a scope the one whose code is being generated. The scope
    /// before it is returned, to be put back afterward.
    fn set_local_frame(&mut self, symtab: Option<TabId>) -> Option<TabId> {
        let old_frame = self.cur_frame;
        let ofs = self.rel_ofs();
        if let Some(old) = old_frame {
            self.add_to_range(old, ofs);
        }
        self.cur_frame = symtab;
        if let Some(tab) = symtab {
            if self.prog.tabs[tab].list_index == 0 {
                self.frames.push(tab);
                self.prog.tabs[tab].list_index = self.frames.len();
            }
            self.add_to_range(tab, ofs);
        }
        old_frame
    }

    // ---- functions ----

    fn gen_code_body(&mut self, body: &CodeBody, func: usize) {
        if body.replaced {
            return;
        }
        // The header. The counts of locals and stack slots and the
        // offsets of the tables are filled in at the end.
        self.cg.start_method();
        self.frames.clear();
        self.enclosing.clear();
        self.switches.clear();
        self.labels = Rc::clone(&body.labels);
        self.goto_labels = vec![None; body.labels.len()];
        self.exc_table.clear();
        self.local_cnt = body.local_cnt;
        self.fin_ret_lcl = None;
        self.method_ofs = self.cg.ofs();
        let method_ofs = self.method_ofs;
        self.out
            .func_anchor
            .insert(func, self.out.code_anchors.len());
        self.out.code_anchors.push(Anchor { ofs: method_ofs });
        self.cg
            .write(body.argc as u8 | if body.varargs { 0x80 } else { 0 });
        self.cg.write(body.opt_argc as u8);
        for _ in 0..4 {
            self.cg.write2(0);
        }
        let code_start = self.cg.ofs();

        // The scopes around the function's own come first in its list.
        if let Some(lcltab) = body.lcltab {
            let mut tab = self.prog.tabs[lcltab].parent;
            while let Some(id) = tab {
                self.set_local_frame(Some(id));
                tab = self.prog.tabs[id].parent;
            }
        }
        self.set_local_frame(body.lcltab);

        if let Some(local) = body.varargs_list_local {
            self.cg.write_op(Op::PushParLst);
            self.cg.write(body.argc as u8);
            self.cg.note_push();
            self.gen_setlcl_stk(self.prog.locals[local].var_num, false);
        }

        self.gen_stmt(&body.stm);

        // Falling off the end returns nil.
        if !self.cg.can_skip_op() {
            self.cg.write_op(Op::RetNil);
        }
        if self.errors == 0 {
            self.remove_jumps_to_jumps(code_start);
        }
        self.cg.write2_at(method_ofs + 2, self.local_cnt as u16);
        let stack = self.cg.max_sp_depth() as usize + self.local_cnt;
        self.cg.write2_at(method_ofs + 4, stack as u16);
        let end = self.rel_ofs();
        let mut tab = body.lcltab;
        while let Some(id) = tab {
            self.add_to_range(id, end);
            tab = self.prog.tabs[id].parent;
        }

        // The table of exception handlers, if the function has any.
        if !self.exc_table.is_empty() {
            self.cg.write2_at(method_ofs + 6, end as u16);
            self.cg.write2(self.exc_table.len() as u16);
            for entry in std::mem::take(&mut self.exc_table) {
                self.cg.write2(entry.start_ofs as u16);
                self.cg.write2(entry.end_ofs as u16);
                self.cg.write4(entry.class);
                self.cg.write2(entry.handler_ofs as u16);
            }
        }

        self.build_debug_table(method_ofs);

        // Labels nothing jumps to. The reference goes through them in
        // the order of its table, as it does with locals; but the table
        // of labels is made the size of the table of global names.
        let bucket = |label: &&LabelSym| {
            label
                .name
                .iter()
                .map(|&byte| usize::from(byte))
                .sum::<usize>()
                % LABEL_HASH_SIZE
        };
        let mut labels: Vec<&LabelSym> = body.labels.iter().rev().collect();
        labels.sort_by_key(bucket);
        for label in labels {
            if label.flags.get() & (flow::GOTO | flow::BREAK | flow::CONT) == 0 {
                self.out.diagnostics.push(GenDiagnostic {
                    file: label.file,
                    line: label.line,
                    severity: Severity::Warning,
                    message: format!("label \"{}\" is not referenced", show_name(&label.name)),
                });
            }
        }
        let len = self.cg.ofs() - method_ofs;
        self.out.max_bytecode_len = self.out.max_bytecode_len.max(len);
    }

    /// Write the table of local names that follows a function's code.
    /// It is there even in a program not compiled for debugging, so that
    /// a running program can ask about its own locals.
    fn build_debug_table(&mut self, method_ofs: usize) {
        // Scopes without locals are left out, and the rest numbered.
        let frames = self.frames.clone();
        let mut frame_cnt = 0;
        for &frame in &frames {
            let tab = &mut self.prog.tabs[frame];
            if tab.entries.is_empty() {
                tab.list_index = 0;
            } else {
                frame_cnt += 1;
                tab.list_index = frame_cnt;
            }
        }
        if frame_cnt == 0 {
            return;
        }
        let table_ofs = self.cg.ofs() - method_ofs;
        self.cg.write2_at(method_ofs + 8, table_ofs as u16);
        // No line records.
        self.cg.write2(0);
        let post_ptr_ofs = self.cg.ofs();
        self.cg.write2(0);
        self.cg.write2(frame_cnt as u16);
        let mut index_ofs = self.cg.ofs();
        for _ in 0..frame_cnt {
            self.cg.write2(0);
        }
        for &frame in &frames {
            if self.prog.tabs[frame].list_index == 0 {
                continue;
            }
            let here = self.cg.ofs();
            self.cg.write2_at(index_ofs, (here - index_ofs) as u16);
            index_ofs += 2;

            // The nearest scope around this one that is in the table.
            let mut parent = self.prog.tabs[frame].parent;
            while let Some(id) = parent {
                if self.prog.tabs[id].list_index != 0 {
                    break;
                }
                parent = self.prog.tabs[id].parent;
            }
            let parent_index = parent.map_or(0, |id| self.prog.tabs[id].list_index);
            self.cg.write2(parent_index as u16);
            let order = enum_order(&self.prog.tabs[frame], &self.prog.locals);
            self.cg.write2(order.len() as u16);
            self.cg.write2(self.prog.tabs[frame].start_ofs as u16);
            self.cg.write2(self.prog.tabs[frame].end_ofs as u16);
            for id in order {
                let local = &self.prog.locals[id];
                let name = local.name.clone();
                let inline = name.len() <= 2;
                let mut flags = 0u16;
                if local.is_param {
                    flags |= 0x0001;
                }
                if !inline {
                    flags |= 0x0004;
                }
                self.cg.write2(local.var_num as u16);
                self.cg.write2(flags);
                // The index within a shared context, for a local that is
                // shared with nested functions. For any other local the
                // reference writes whatever its memory holds.
                self.out.unspecified.push(self.cg.ofs());
                self.cg.write2(0);
                if inline {
                    self.cg.write2(name.len() as u16);
                    for byte in name {
                        self.cg.write(byte);
                    }
                } else {
                    self.local_names.push((self.cg.ofs(), name));
                    self.cg.write4(0);
                }
            }
        }
        let here = self.cg.ofs();
        self.cg
            .write2_at(post_ptr_ofs, (here - post_ptr_ofs) as u16);
        self.cg.write4(0);
    }

    /// Put the names of locals into the constants, each once, and point
    /// the tables at them.
    ///
    /// The reference first collects the names in a stream of their own,
    /// every use of every name, and then moves them to the constants. A
    /// name longer than a symbol can be (the tokenizer warns about those
    /// but passes them on whole) is not moved, and the table is left
    /// pointing at where the name was in that stream. Hexicon writes
    /// the same number there.
    fn finish_local_names(&mut self) {
        const SYM_MAX_LEN: usize = 80;
        const NAME_PAGE: usize = 65536;
        let mut known: HashMap<Vec<u8>, usize> = HashMap::new();
        // Where the next name would go in the reference's stream, which
        // it lays out in pages like a pool.
        let mut stream_ofs = 0;
        let mut page_rem = NAME_PAGE;
        for (ofs, name) in std::mem::take(&mut self.local_names) {
            let len = 2 + name.len();
            if len > page_rem {
                stream_ofs += page_rem;
                page_rem = NAME_PAGE;
            }
            let stream_addr = stream_ofs;
            stream_ofs += len;
            page_rem -= len;
            if name.len() > SYM_MAX_LEN {
                self.cg.write4_at(ofs, stream_addr as u32);
                continue;
            }
            let anchor = match known.get(&name) {
                Some(anchor) => *anchor,
                None => {
                    let anchor = self.out.const_anchors.len();
                    self.out.const_anchors.push(Anchor {
                        ofs: self.out.consts.len(),
                    });
                    self.out
                        .consts
                        .extend_from_slice(&(name.len() as u16).to_le_bytes());
                    self.out.consts.extend_from_slice(&name);
                    known.insert(name, anchor);
                    anchor
                }
            };
            self.out.fixups.push(Fixup {
                ofs,
                target: Target::Const(anchor),
            });
        }
    }

    /// Shorten jumps that land on jumps, and turn jumps that land on a
    /// return into the return. This goes over the finished code of one
    /// function.
    fn remove_jumps_to_jumps(&mut self, start_ofs: usize) {
        let end_ofs = self.cg.ofs();
        let code = &mut self.cg.code;
        let read2 =
            |code: &[u8], at: usize| i64::from(i16::from_le_bytes([code[at], code[at + 1]]));
        let readu2 =
            |code: &[u8], at: usize| usize::from(u16::from_le_bytes([code[at], code[at + 1]]));
        let jump_target = |code: &[u8], at: usize| (at as i64 + 1 + read2(code, at + 1)) as usize;
        let is = |byte: u8, op: Op| byte == op as u8;
        let mut prv_op = Op::Nop as u8;
        let mut ofs = start_ofs;
        while ofs < end_ofs {
            let mut op = code[ofs];
            match Op::from_byte(op) {
                Some(Op::RetVal) => {
                    if is(prv_op, Op::PushTrue) {
                        code[ofs - 1] = Op::RetTrue as u8;
                    } else if is(prv_op, Op::PushNil) {
                        code[ofs - 1] = Op::RetNil as u8;
                    } else if is(prv_op, Op::GetR0) {
                        code[ofs - 1] = Op::Ret as u8;
                    }
                    ofs += 1;
                }
                Some(Op::PushStrI) => ofs += 3 + readu2(code, ofs + 1),
                Some(Op::Switch) => ofs += 1 + 2 + 7 * readu2(code, ofs + 1) + 2,
                Some(Op::NamedArgTab) => ofs += 1 + 2 + readu2(code, ofs + 1),
                Some(Op::Jmp) => {
                    let orig_target = jump_target(code, ofs);
                    let mut target = orig_target;
                    let mut chain_len = 0;
                    while chain_len < 20 {
                        chain_len += 1;
                        let target_op = code[target];
                        match Op::from_byte(target_op) {
                            Some(Op::RetVal) if is(prv_op, Op::GetR0) => {
                                code[ofs - 1] = Op::Ret as u8;
                                break;
                            }
                            Some(Op::RetVal | Op::RetNil | Op::RetTrue | Op::Ret | Op::Throw) => {
                                code[ofs] = target_op;
                                code[ofs + 1] = Op::Nop as u8;
                                code[ofs + 2] = Op::Nop as u8;
                                break;
                            }
                            Some(Op::Jmp) => {
                                target = jump_target(code, target);
                                if target == ofs {
                                    break;
                                }
                            }
                            _ => {
                                if target != orig_target {
                                    let offset = target as i64 - (ofs as i64 + 1);
                                    code[ofs + 1..ofs + 3]
                                        .copy_from_slice(&(offset as i16).to_le_bytes());
                                }
                                break;
                            }
                        }
                    }
                    ofs += 3;
                }
                Some(
                    Op::Jt
                    | Op::Jf
                    | Op::Je
                    | Op::Jne
                    | Op::Jgt
                    | Op::Jge
                    | Op::Jlt
                    | Op::Jle
                    | Op::Jst
                    | Op::Jsf
                    | Op::JNil
                    | Op::JNotNil
                    | Op::JR0T
                    | Op::JR0F,
                ) => {
                    let orig_target = jump_target(code, ofs);
                    let mut target = orig_target;
                    let mut chain_len = 0;
                    while chain_len < 20 {
                        chain_len += 1;
                        let target_op = Op::from_byte(code[target]);
                        if target_op == Some(Op::Jmp) {
                            target = jump_target(code, target);
                            continue;
                        }
                        // A jump that keeps its value when taken, landing
                        // on another test of the same value, already
                        // knows how that test comes out.
                        let (same, opposite, plain) = if is(op, Op::Jsf) {
                            (Op::Jsf, Op::Jst, Op::Jf)
                        } else if is(op, Op::Jst) {
                            (Op::Jst, Op::Jsf, Op::Jt)
                        } else {
                            break;
                        };
                        let plain_opposite = if plain == Op::Jf { Op::Jt } else { Op::Jf };
                        match target_op {
                            Some(found) if found == same => target = jump_target(code, target),
                            Some(found) if found == opposite || found == plain_opposite => {
                                code[ofs] = plain as u8;
                                op = plain as u8;
                                target += 3;
                            }
                            Some(found) if found == plain => {
                                code[ofs] = plain as u8;
                                op = plain as u8;
                                target = jump_target(code, target);
                            }
                            _ => break,
                        }
                    }
                    if target != orig_target {
                        let offset = target as i64 - (ofs as i64 + 1);
                        code[ofs + 1..ofs + 3].copy_from_slice(&(offset as i16).to_le_bytes());
                    }
                    ofs += 3;
                }
                Some(other) => {
                    ofs += match other.layout() {
                        Layout::Fixed(operands) => {
                            1 + operands.iter().map(|kind| kind.size()).sum::<usize>()
                        }
                        _ => 1,
                    };
                }
                None => ofs += 1,
            }
            prv_op = op;
        }
    }

    // ---- labels ----

    /// Write a jump to a place further on, unless the jump could never
    /// be reached.
    fn gen_jump_ahead(&mut self, op: Op) -> Option<Label> {
        if self.cg.can_skip_op() {
            return None;
        }
        let label = self.cg.new_label();
        self.cg.write_jump(op, label);
        Some(label)
    }

    fn def_label_pos(&mut self, label: Option<Label>) {
        if let Some(label) = label {
            self.cg.def_label(label);
        }
    }

    // ---- statements ----

    fn gen_substm(&mut self, sub: &Stmt, outer: &Stmt) {
        self.gen_substm_at(sub, outer.file, outer.line);
    }

    /// Generate a statement that is part of another, which is at the
    /// given place in the source.
    fn gen_substm_at(&mut self, sub: &Stmt, file: usize, line: u32) {
        self.set_line(sub);
        self.gen_stmt(sub);
        self.cur_file = file;
        self.cur_line = line;
    }

    /// How control can leave a statement, as far as can be told now
    /// that constants are known.
    fn flow_of(&self, stmt: &Stmt) -> u32 {
        Flow {
            warn: false,
            syntax_only: false,
            labels: &self.labels,
            mark: &self.last_labels,
            warnings: Vec::new(),
        }
        .control_flow(stmt)
    }

    fn gen_stmt(&mut self, stmt: &Stmt) {
        match &stmt.kind {
            StmtKind::Null => {}
            StmtKind::Comp { stmts, symtab } => {
                let old_frame = self.set_local_frame(*symtab);
                for sub in stmts {
                    self.set_line(sub);
                    self.gen_stmt(sub);
                }
                self.set_local_frame(old_frame);
            }
            StmtKind::Expr(expr) => {
                self.set_line(stmt);
                self.gen_expr(expr, true, true);
            }
            StmtKind::If {
                cond,
                then_part,
                else_part,
            } => self.gen_if(stmt, cond, then_part.as_deref(), else_part.as_deref()),
            StmtKind::Return(expr) => self.gen_return(expr.as_ref()),
            StmtKind::For {
                init,
                cond,
                reinit,
                body,
                symtab,
                id,
            } => {
                let old_frame = self.set_local_frame(*symtab);
                if let Some(init) = init {
                    self.gen_expr(init, true, true);
                }
                let top_lbl = self.cg.new_label_here();
                let cont_lbl = self.cg.new_label();
                let end_lbl = self.cg.new_label();
                self.enclosing.push(Enclosing::Loop {
                    id: *id,
                    break_lbl: end_lbl,
                    cont_lbl,
                });
                if let Some(cond) = cond {
                    self.gen_cond(cond, None, Some(end_lbl));
                }
                if let Some(body) = body {
                    self.gen_substm(body, stmt);
                }
                self.cg.def_label(cont_lbl);
                if let Some(reinit) = reinit {
                    self.gen_expr(reinit, true, true);
                }
                self.cg.write_jump(Op::Jmp, top_lbl);
                self.cg.def_label(end_lbl);
                self.enclosing.pop();
                self.set_local_frame(old_frame);
            }
            StmtKind::While { cond, body, id } => {
                let top_lbl = self.cg.new_label_here();
                let end_lbl = self.cg.new_label();
                self.enclosing.push(Enclosing::Loop {
                    id: *id,
                    break_lbl: end_lbl,
                    cont_lbl: top_lbl,
                });
                self.gen_cond(cond, None, Some(end_lbl));
                if let Some(body) = body {
                    self.gen_substm(body, stmt);
                }
                self.cg.write_jump(Op::Jmp, top_lbl);
                self.cg.def_label(end_lbl);
                self.enclosing.pop();
            }
            StmtKind::DoWhile { body, cond, id } => {
                let top_lbl = self.cg.new_label_here();
                let end_lbl = self.cg.new_label();
                let cont_lbl = self.cg.new_label();
                self.enclosing.push(Enclosing::Loop {
                    id: *id,
                    break_lbl: end_lbl,
                    cont_lbl,
                });
                if let Some(body) = body {
                    self.gen_substm(body, stmt);
                }
                self.cg.def_label(cont_lbl);
                self.gen_cond(cond, Some(top_lbl), None);
                self.cg.def_label(end_lbl);
                self.enclosing.pop();
            }
            StmtKind::Break(label) => self.gen_break_or_continue(label.as_deref(), true),
            StmtKind::Continue(label) => self.gen_break_or_continue(label.as_deref(), false),
            StmtKind::Switch {
                expr,
                body,
                case_cnt,
                has_default,
                id,
            } => self.gen_switch(stmt, expr, body, *case_cnt, *has_default, *id),
            StmtKind::Case { expr, stm } => {
                let Some(slots) = self.switches.last_mut() else {
                    self.error("case/default without switch or non-constant case expression");
                    return;
                };
                let slot = slots.next_case;
                slots.next_case += 7;
                if let Some(value) = expr.as_const() {
                    self.write_const_at(slot, value);
                }
                // The jump is measured from where it is written.
                let jump = self.cg.ofs() - (slot + 5);
                self.cg.write2_at(slot + 5, jump as u16);
                self.cg.clear_peephole();
                if let Some(stm) = stm {
                    self.gen_substm(stm, stmt);
                }
            }
            StmtKind::Default(stm) => {
                let Some(slots) = self.switches.last() else {
                    self.error("case/default without switch or non-constant case expression");
                    return;
                };
                let slot = slots.default_slot;
                let jump = self.cg.ofs() - slot;
                self.cg.write2_at(slot, jump as u16);
                self.cg.clear_peephole();
                if let Some(stm) = stm {
                    self.gen_substm(stm, stmt);
                }
            }
            StmtKind::Label { label, stm, id } => {
                // `continue` can name the label if what it is on is a
                // loop, possibly under more labels.
                let mut inner = stm.as_deref();
                while let Some(Stmt {
                    kind: StmtKind::Label { stm, .. },
                    ..
                }) = inner
                {
                    inner = stm.as_deref();
                }
                let labels_loop = matches!(
                    inner.map(|stmt| &stmt.kind),
                    Some(StmtKind::For { .. } | StmtKind::While { .. } | StmtKind::DoWhile { .. })
                );
                self.enclosing.push(Enclosing::Label {
                    id: *id,
                    label: *label,
                    break_lbl: None,
                    labels_loop,
                });
                // A label whose name was taken still gets a place, which
                // nothing can jump to.
                let goto_lbl = match label {
                    Some(label) => self.goto_label(*label),
                    None => self.cg.new_label(),
                };
                self.cg.def_label(goto_lbl);
                if let Some(stm) = stm {
                    self.gen_substm(stm, stmt);
                }
                if let Some(Enclosing::Label {
                    break_lbl: Some(break_lbl),
                    ..
                }) = self.enclosing.pop()
                {
                    self.cg.def_label(break_lbl);
                }
            }
            StmtKind::Goto(name) => self.gen_goto(name),
            StmtKind::Try {
                id,
                body,
                catches,
                finally,
            } => self.gen_try(stmt, *id, body.as_deref(), catches, finally.as_deref()),
            StmtKind::Throw(expr) => {
                self.gen_expr(expr, false, false);
                self.cg.write_op(Op::Throw);
                self.cg.note_pop();
            }
        }
    }

    fn find_label(&self, name: &[u8]) -> Option<usize> {
        self.labels.iter().position(|label| label.name == name)
    }

    /// Where a label is, for jumping to.
    fn goto_label(&mut self, label: usize) -> Label {
        if let Some(found) = self.goto_labels[label] {
            return found;
        }
        let made = self.cg.new_label();
        self.goto_labels[label] = Some(made);
        made
    }

    /// Call a `finally` block, as a jump to it that it comes back from.
    fn gen_jsr_finally(&mut self, finally_lbl: Option<Label>) {
        if let Some(label) = finally_lbl {
            self.cg.write_jump(Op::LJsr, label);
            // The jump pushes where to come back to, and the block pops
            // it.
            self.cg.note_push();
            self.cg.note_pop();
        }
    }

    fn gen_break_or_continue(&mut self, label: Option<&[u8]>, is_break: bool) {
        if self.gen_jump_out(label, is_break) {
            return;
        }
        let message = match (label, is_break) {
            (None, true) => "invalid \"break\" - not in a for/while/do/switch".to_owned(),
            (None, false) => "invalid \"continue\" - not in a for/while/do".to_owned(),
            (Some(label), true) => format!(
                "invalid \"break\" - no label \"{}\" on any enclosing statement",
                show_name(label)
            ),
            (Some(label), false) => format!(
                "invalid \"continue\" - no label \"{}\" on any enclosing loop",
                show_name(label)
            ),
        };
        self.error(message);
    }

    /// Generate a `break` or a `continue`: each statement around it,
    /// from the nearest outward, either does the jump or does what
    /// leaving it takes and passes the job on. Nothing may take it.
    fn gen_jump_out(&mut self, label: Option<&[u8]>, is_break: bool) -> bool {
        // The label named, and whether the function has it.
        let named = label.map(|name| self.find_label(name));
        for index in (0..self.enclosing.len()).rev() {
            match self.enclosing[index] {
                Enclosing::Loop {
                    break_lbl,
                    cont_lbl,
                    ..
                } => {
                    if label.is_none() {
                        let target = if is_break { break_lbl } else { cont_lbl };
                        self.cg.write_jump(Op::Jmp, target);
                        return true;
                    }
                }
                Enclosing::Switch { break_lbl, .. } => {
                    if is_break && label.is_none() {
                        // With nowhere to go, the `break` cannot be
                        // reached, and there is nothing to do.
                        if let Some(target) = break_lbl {
                            self.cg.write_jump(Op::Jmp, target);
                        }
                        return true;
                    }
                }
                Enclosing::Label {
                    id,
                    label: own,
                    break_lbl,
                    labels_loop,
                } => {
                    // A statement whose label name was taken answers to
                    // any name the function does not have.
                    if named != Some(own) || self.labels.is_empty() {
                        continue;
                    }
                    if is_break {
                        let target = break_lbl.unwrap_or_else(|| self.cg.new_label());
                        self.enclosing[index] = Enclosing::Label {
                            id,
                            label: own,
                            break_lbl: Some(target),
                            labels_loop,
                        };
                        self.cg.write_jump(Op::Jmp, target);
                        return true;
                    }
                    // `continue` goes to the loop the label is on.
                    if !labels_loop {
                        return false;
                    }
                    let cont_lbl = self.enclosing[index..].iter().find_map(|enclosing| {
                        if let Enclosing::Loop { cont_lbl, .. } = enclosing {
                            Some(*cont_lbl)
                        } else {
                            None
                        }
                    });
                    let Some(target) = cont_lbl else {
                        return false;
                    };
                    self.cg.write_jump(Op::Jmp, target);
                    return true;
                }
                Enclosing::Try { finally_lbl, .. } => self.gen_jsr_finally(finally_lbl),
                Enclosing::Finally { .. } => {}
            }
        }
        false
    }

    fn gen_switch(
        &mut self,
        stmt: &Stmt,
        expr: &Expr,
        body: &Stmt,
        case_cnt: usize,
        has_default: bool,
        id: usize,
    ) {
        self.gen_expr(expr, false, false);
        // If nothing after the switch can be reached, there is no end
        // to jump to.
        let end_lbl = (self.flow_of(stmt) & flow::NEXT != 0).then(|| self.cg.new_label());
        self.enclosing.push(Enclosing::Switch {
            id,
            break_lbl: end_lbl,
        });
        self.cg.write_op(Op::Switch);
        self.cg.note_pop();
        // The table: for each case a value and a jump, filled in as the
        // cases are come to, and then the jump for no match.
        self.cg.write2(case_cnt as u16);
        let next_case = self.cg.ofs();
        for _ in 0..case_cnt * 7 {
            self.cg.write(0);
        }
        let default_slot = self.cg.ofs();
        match end_lbl {
            Some(end_lbl) if !has_default => self.cg.write_label_ofs(end_lbl),
            _ => self.cg.write2(0),
        }
        self.switches.push(SwitchSlots {
            next_case,
            default_slot,
        });
        self.gen_substm(body, stmt);
        if let Some(end_lbl) = end_lbl {
            self.cg.def_label(end_lbl);
        }
        self.switches.pop();
        self.enclosing.pop();
    }

    /// Write a constant into the code as a kind and four bytes, at a
    /// place already there.
    fn write_const_at(&mut self, ofs: usize, value: &ConstVal) {
        let (kind, bytes) = match &value.value {
            Value::Nil => (datatype::NIL, 0),
            Value::True => (datatype::TRUE, 0),
            Value::Int(int) => (datatype::INT, *int as u32),
            Value::SStr(text) => {
                self.add_const_str(text, false, ofs + 1);
                (datatype::SSTRING, 0)
            }
            Value::List(elements) => {
                self.add_const_list(elements, false, ofs + 1);
                (datatype::LIST, 0)
            }
            Value::FuncPtr(func) => {
                self.add_fixup(false, ofs + 1, Target::Func(*func));
                (datatype::FUNCPTR, 0)
            }
        };
        if matches!(value.value, Value::Nil | Value::True) {
            // The reference writes the kind and leaves the rest as its
            // memory had it.
            self.out.unspecified.push(ofs + 1);
            self.out.unspecified.push(ofs + 3);
        }
        self.cg.code[ofs] = kind;
        self.cg.write4_at(ofs + 1, bytes);
    }

    fn gen_goto(&mut self, name: &[u8]) {
        let Some(label) = self.find_label(name) else {
            self.error(format!(
                "invalid \"goto\" - label \"{}\" is not defined in this function or method",
                show_name(name)
            ));
            return;
        };
        // Leave each statement around the `goto`, from the nearest
        // outward, until one that the label is inside as well.
        let within = self.labels[label].within.clone();
        let mut shared = None;
        for index in (0..self.enclosing.len()).rev() {
            let id = self.enclosing[index].id();
            if let Some(found) = within.iter().position(|enclosure| enclosure.id == id) {
                shared = Some(found);
                break;
            }
            if let Enclosing::Try { finally_lbl, .. } = self.enclosing[index] {
                self.gen_jsr_finally(finally_lbl);
            }
        }
        // The jump goes into everything around the label from there
        // inward, and a `finally` block has no way to be left after
        // being entered like that.
        let entered = match shared {
            Some(found) => &within[found + 1..],
            None => &within[..],
        };
        if entered.iter().any(|enclosure| enclosure.is_finally) {
            self.error("'goto' cannot transfer control into a 'finally' block");
        }
        let target = self.goto_label(label);
        self.cg.write_jump(Op::Jmp, target);
    }

    fn gen_try(
        &mut self,
        stmt: &Stmt,
        id: usize,
        body: Option<&Stmt>,
        catches: &[Catch],
        finally: Option<&Finally>,
    ) {
        let finally_lbl = finally.map(|_| self.cg.new_label());
        self.enclosing.push(Enclosing::Try { id, finally_lbl });
        let start_ofs = self.rel_ofs();
        if let Some(body) = body {
            self.gen_substm(body, stmt);
        }
        // If the `finally` block never finishes, nothing that calls it
        // gets control back.
        let finally_never_returns = finally.is_some_and(|finally| {
            finally
                .body
                .as_deref()
                .is_some_and(|body| self.flow_of(body) & flow::NEXT == 0)
        });
        self.gen_jsr_finally(finally_lbl);
        let mut end_lbl = None;
        if !finally_never_returns {
            end_lbl = self.gen_jump_ahead(Op::Jmp);
        }
        // The protected code is everything up to the byte before here.
        let end_ofs = self.rel_ofs() - 1;
        for (index, catch) in catches.iter().enumerate() {
            self.gen_catch(stmt, catch, start_ofs, end_ofs);
            self.gen_jsr_finally(finally_lbl);
            if !finally_never_returns && (finally.is_some() || index + 1 < catches.len()) {
                match end_lbl {
                    None => end_lbl = self.gen_jump_ahead(Op::Jmp),
                    Some(end_lbl) => self.cg.write_jump(Op::Jmp, end_lbl),
                }
            }
        }
        // The `finally` block is not inside the `try` it belongs to: it
        // is not called again as it is left.
        self.enclosing.pop();
        if let (Some(finally), Some(finally_lbl)) = (finally, finally_lbl) {
            let end_ofs = self.rel_ofs() - 1;
            self.enclosing.push(Enclosing::Finally { id: finally.id });
            // First, what happens when an exception arrives: keep it,
            // run the block, and throw it again.
            self.exc_table.push(ExcEntry {
                start_ofs,
                end_ofs,
                class: 0,
                handler_ofs: self.rel_ofs(),
            });
            self.cg.clear_peephole();
            self.cg.note_push();
            self.gen_setlcl_stk(finally.exc_local, false);
            self.gen_jsr_finally(Some(finally_lbl));
            self.gen_getlcl(finally.exc_local, false);
            self.cg.write_op(Op::Throw);
            self.cg.note_pop();
            // Then the block itself. It is entered with where to go
            // back to on the stack.
            self.cg.def_label(finally_lbl);
            self.cg.note_push();
            self.gen_setlcl_stk(finally.jsr_local, false);
            if let Some(body) = &finally.body {
                self.gen_substm_at(body, finally.file, finally.line);
            }
            self.cg.write_op(Op::LRet);
            self.cg.write2(finally.jsr_local as u16);
            self.enclosing.pop();
        }
        self.def_label_pos(end_lbl);
    }

    fn gen_catch(&mut self, stmt: &Stmt, catch: &Catch, start_ofs: usize, end_ofs: usize) {
        let old_frame = self.set_local_frame(catch.symtab);
        // The class of exception caught has to be an object, and there
        // are no objects yet.
        if self.prog.globals.contains_key(&catch.class) {
            self.out.diagnostics.push(GenDiagnostic {
                file: catch.file,
                line: catch.line,
                severity: Severity::Error,
                message: format!(
                    "exception class symbol \"{}\" in \"catch\" clause is not an object",
                    show_name(&catch.class)
                ),
            });
            self.errors += 1;
        } else {
            self.undefined(&catch.class);
        }
        self.exc_table.push(ExcEntry {
            start_ofs,
            end_ofs,
            class: 0,
            handler_ofs: self.rel_ofs(),
        });
        self.cg.clear_peephole();
        // The exception is on the stack on arrival.
        self.cg.note_push();
        self.gen_setlcl_stk(self.prog.locals[catch.var].var_num, false);
        if let Some(body) = &catch.body {
            self.gen_substm(body, stmt);
        }
        self.set_local_frame(old_frame);
    }

    fn gen_if(
        &mut self,
        stmt: &Stmt,
        cond: &Expr,
        then_part: Option<&Stmt>,
        else_part: Option<&Stmt>,
    ) {
        // With a constant condition, only one part is generated.
        if let Some(value) = cond.as_const() {
            if value.is_true() {
                if else_part.is_some() {
                    self.log(
                        Severity::Warning,
                        "\"if\" condition is always true - \"else\" part is unreachable",
                    );
                }
                if let Some(part) = then_part {
                    self.gen_substm(part, stmt);
                }
            } else {
                if !value.ctc {
                    self.log(Severity::Warning, "\"if\" condition is always false");
                }
                if let Some(part) = else_part {
                    self.gen_substm(part, stmt);
                }
            }
            return;
        }
        match (then_part, else_part) {
            // Nothing depends on the condition; it is evaluated for
            // whatever else it does.
            (None, None) => self.gen_expr(cond, true, true),
            (Some(then_part), else_part) => {
                let lbl_else = self.cg.new_label();
                self.gen_cond(cond, None, Some(lbl_else));
                self.gen_substm(then_part, stmt);
                if let Some(else_part) = else_part {
                    let lbl_end = self.gen_jump_ahead(Op::Jmp);
                    self.cg.def_label(lbl_else);
                    self.gen_substm(else_part, stmt);
                    self.def_label_pos(lbl_end);
                } else {
                    self.cg.def_label(lbl_else);
                }
            }
            (None, Some(else_part)) => {
                let lbl_end = self.cg.new_label();
                self.gen_cond(cond, Some(lbl_end), None);
                self.gen_substm(else_part, stmt);
                self.cg.def_label(lbl_end);
            }
        }
    }

    fn gen_return(&mut self, expr: Option<&Expr>) {
        // Returning nil or true has an instruction of its own; any
        // other value is worked out first.
        let constant = expr.and_then(|expr| expr.as_const().map(|value| &value.value));
        let need_gen = expr.is_some() && !matches!(constant, Some(Value::Nil | Value::True));
        let mut val_on_stack = false;
        if let (true, Some(expr)) = (need_gen, expr) {
            let depth = self.cg.sp_depth();
            self.gen_expr(expr, false, false);
            val_on_stack = self.cg.sp_depth() > depth;
        }

        // Then the `finally` blocks around the statement run, with the
        // value put aside in a local meanwhile.
        if !self.enclosing.is_empty() {
            let finally_lbls: Vec<Label> = self
                .enclosing
                .iter()
                .rev()
                .filter_map(|enclosing| match enclosing {
                    Enclosing::Try { finally_lbl, .. } => *finally_lbl,
                    _ => None,
                })
                .collect();
            let saved = (val_on_stack && !finally_lbls.is_empty()).then(|| {
                *self.fin_ret_lcl.get_or_insert_with(|| {
                    self.local_cnt += 1;
                    self.local_cnt - 1
                })
            });
            if let Some(local) = saved {
                self.gen_setlcl_stk(local, false);
            }
            for label in finally_lbls {
                self.gen_jsr_finally(Some(label));
            }
            if let Some(local) = saved {
                self.gen_getlcl(local, false);
            }
        }

        match constant {
            _ if expr.is_none() => self.cg.write_op(Op::RetNil),
            Some(Value::Nil) => self.cg.write_op(Op::RetNil),
            Some(Value::True) => self.cg.write_op(Op::RetTrue),
            _ if val_on_stack => {
                self.cg.write_op(Op::RetVal);
                self.cg.note_pop();
            }
            // The expression left nothing to return.
            _ => self.cg.write_op(Op::RetNil),
        }
    }

    // ---- expressions ----

    /// Generate a test of a condition: a jump to `then_label` if it is
    /// true, or else a jump to `else_label` if it is false. Exactly one
    /// of the two is given; the other case falls through.
    fn gen_cond(&mut self, expr: &Expr, then_label: Option<Label>, else_label: Option<Label>) {
        match expr {
            Expr::Binary(BinOp::Or, left, right) => {
                // If the left side is true, the whole is.
                let internal_then = then_label.unwrap_or_else(|| self.cg.new_label());
                self.gen_cond(left, Some(internal_then), None);
                self.gen_cond(right, then_label, else_label);
                if then_label.is_none() {
                    self.cg.def_label(internal_then);
                }
            }
            Expr::Binary(BinOp::And, left, right) => {
                let internal_else = else_label.unwrap_or_else(|| self.cg.new_label());
                self.gen_cond(left, None, Some(internal_else));
                self.gen_cond(right, then_label, else_label);
                if else_label.is_none() {
                    self.cg.def_label(internal_else);
                }
            }
            _ => {
                self.gen_expr(expr, false, true);
                match (then_label, else_label) {
                    (Some(label), _) => self.cg.write_jump(Op::Jt, label),
                    (None, Some(label)) => self.cg.write_jump(Op::Jf, label),
                    (None, None) => unreachable!("a condition needs somewhere to go"),
                }
                self.cg.note_pop();
            }
        }
    }

    fn gen_int(&mut self, value: i32) {
        if value == 0 {
            self.cg.write_op(Op::Push0);
        } else if value == 1 {
            self.cg.write_op(Op::Push1);
        } else if (-128..127).contains(&value) {
            self.cg.write_op(Op::PushInt8);
            self.cg.write(value as u8);
        } else {
            self.cg.write_op(Op::PushInt);
            self.cg.write4(value as u32);
        }
        self.cg.note_push();
    }

    /// Record that four bytes are to hold an address: in the code, or
    /// in the constants.
    fn add_fixup(&mut self, in_consts: bool, ofs: usize, target: Target) {
        let fixup = Fixup { ofs, target };
        if in_consts {
            self.out.const_fixups.push(fixup);
        } else {
            self.out.fixups.push(fixup);
        }
    }

    /// Put a string in the constants, unless it is a short one that is
    /// there already, and note where its address is wanted.
    fn add_const_str(&mut self, text: &[u8], in_consts: bool, ofs: usize) {
        let known = (text.len() < INTERN_THRESHOLD)
            .then(|| self.interned.get(text).copied())
            .flatten();
        let anchor = known.unwrap_or_else(|| {
            let anchor = self.out.const_anchors.len();
            self.out.const_anchors.push(Anchor {
                ofs: self.out.consts.len(),
            });
            if text.len() < INTERN_THRESHOLD {
                self.interned.insert(text.to_vec(), anchor);
            }
            self.out
                .consts
                .extend_from_slice(&(text.len() as u16).to_le_bytes());
            self.out.consts.extend_from_slice(text);
            self.out.max_str_len = self.out.max_str_len.max(text.len());
            anchor
        });
        self.add_fixup(in_consts, ofs, Target::Const(anchor));
    }

    /// Put a list in the constants, and note where its address is
    /// wanted. The list is a count and then five bytes for each
    /// element; the strings and lists among the elements follow it.
    fn add_const_list(&mut self, elements: &[ConstVal], in_consts: bool, ofs: usize) {
        let anchor = self.out.const_anchors.len();
        let start = self.out.consts.len();
        self.out.const_anchors.push(Anchor { ofs: start });
        self.add_fixup(in_consts, ofs, Target::Const(anchor));
        self.out
            .consts
            .extend_from_slice(&(elements.len() as u16).to_le_bytes());
        self.out.consts.resize(start + 2 + 5 * elements.len(), 0);
        for (index, element) in elements.iter().enumerate() {
            let at = start + 2 + 5 * index;
            let (kind, value) = match &element.value {
                Value::Nil => (datatype::NIL, 0),
                Value::True => (datatype::TRUE, 0),
                Value::Int(int) => (datatype::INT, *int as u32),
                Value::SStr(text) => {
                    self.add_const_str(text, true, at + 1);
                    (datatype::SSTRING, 0)
                }
                Value::List(inner) => {
                    self.add_const_list(inner, true, at + 1);
                    (datatype::LIST, 0)
                }
                Value::FuncPtr(func) => {
                    self.add_fixup(true, at + 1, Target::Func(*func));
                    (datatype::FUNCPTR, 0)
                }
            };
            if matches!(element.value, Value::Nil | Value::True) {
                // The reference writes the kind and leaves the rest as
                // it found it.
                self.out.const_unspecified.push(at + 1);
            }
            self.out.consts[at] = kind;
            self.out.consts[at + 1..at + 5].copy_from_slice(&value.to_le_bytes());
        }
        self.out.max_list_cnt = self.out.max_list_cnt.max(elements.len());
    }

    fn gen_str(&mut self, text: &[u8]) {
        self.cg.write_op(Op::PushStr);
        self.cg.note_push();
        self.add_const_str(text, false, self.cg.ofs());
        self.cg.write4(0);
    }

    fn gen_func_ref(&mut self, func: usize) {
        self.out.fixups.push(Fixup {
            ofs: self.cg.ofs(),
            target: Target::Func(func),
        });
        self.cg.write4(0);
    }

    fn gen_expr(&mut self, expr: &Expr, discard: bool, for_condition: bool) {
        match expr {
            Expr::Const(value) => {
                if discard {
                    return;
                }
                if let Some(elements) = &value.list_node {
                    // A list whose value was overwritten is still
                    // generated as the list.
                    self.cg.write_op(Op::PushLst);
                    self.add_const_list(elements, false, self.cg.ofs());
                    self.cg.write4(0);
                    self.cg.note_push();
                    return;
                }
                match &value.value {
                    Value::Nil => self.cg.write_op(Op::PushNil),
                    Value::True => self.cg.write_op(Op::PushTrue),
                    Value::Int(int) => {
                        self.gen_int(*int);
                        self.cg.note_pop();
                    }
                    Value::SStr(text) => {
                        self.gen_str(text);
                        self.cg.note_pop();
                    }
                    Value::FuncPtr(func) => {
                        self.cg.write_op(Op::PushFnPtr);
                        self.gen_func_ref(*func);
                    }
                    Value::List(elements) => {
                        self.cg.write_op(Op::PushLst);
                        self.add_const_list(elements, false, self.cg.ofs());
                        self.cg.write4(0);
                    }
                }
                self.cg.note_push();
            }
            // A built-in function named without parentheses is called
            // with no arguments.
            Expr::Sym(name) => match self.prog.globals.get(name) {
                Some(Global::Bif(bif)) => self.gen_bif_call(*bif, discard, 0, false),
                _ => self.undefined(name),
            },
            Expr::DStr(text) => {
                if !discard {
                    self.error(format!(
                        "a double-quoted string (\"{}\") is not valid within an expression",
                        show_name(text)
                    ));
                }
                self.cg.write_op(Op::Say);
                self.add_const_str(text, false, self.cg.ofs());
                self.cg.write4(0);
            }
            Expr::DStrEmbed(sub) => {
                // Whatever value the expression leaves is displayed. It
                // need not leave one: a call to a function that returns
                // nothing is there for what it does.
                let depth = self.cg.sp_depth();
                self.gen_expr(sub, !self.has_return_value(sub), false);
                if self.cg.sp_depth() > depth {
                    self.cg.write_op(Op::SayVal);
                    self.cg.note_pop();
                }
            }
            Expr::OneOf(..) => {
                // The parser has reported that this needs a class the
                // program does not have, so nothing is generated.
                self.error("<<one of>> requires class OneOfIndexGen to be defined in the program");
            }
            Expr::List(elements) => {
                // The elements, last first, and then a list made of
                // them.
                for element in elements.iter().rev() {
                    self.gen_expr(element, false, false);
                }
                if elements.len() <= 255 {
                    self.cg.write_op(Op::New1);
                    self.cg.write(elements.len() as u8);
                    self.cg.write(METACLASS_LIST);
                } else {
                    self.cg.write_op(Op::New2);
                    self.cg.write2(elements.len() as u16);
                    self.cg.write2(u16::from(METACLASS_LIST));
                }
                self.cg.note_pops(elements.len() as i32);
                if !discard {
                    self.cg.write_op(Op::GetR0);
                    self.cg.note_push();
                }
            }
            Expr::Index(left, index) => self.gen_binary(Op::Index, left, index, discard),
            Expr::Local(id) => self.gen_local(*id, discard),
            Expr::Unary(op, sub) => match op {
                UnOp::Not => self.gen_unary(Op::Not, sub, discard, true),
                UnOp::Boolize => {
                    if for_condition {
                        // Whatever tests the value does the same
                        // conversion.
                        self.gen_expr(sub, discard, for_condition);
                    } else if !sub.is_bool() {
                        self.gen_unary(Op::Boolize, sub, discard, true);
                    }
                }
                UnOp::BNot => self.gen_unary(Op::BNot, sub, discard, false),
                UnOp::Pos => self.gen_expr(sub, discard, false),
                UnOp::Neg => self.gen_unary(Op::Neg, sub, discard, false),
            },
            Expr::Binary(op, left, right) => {
                self.gen_binary_expr(*op, left, right, discard, for_condition)
            }
            Expr::Assign(AsiOp::Simple, left, right) => {
                self.gen_asi(left, discard, 1, AsiOp::Simple, Some(right), false, true);
            }
            Expr::Assign(op, left, right) => {
                if !self.gen_asi(left, discard, 1, *op, Some(right), false, true) {
                    // Nothing special for this target: work out the new
                    // value, then store it.
                    let opc = match op {
                        AsiOp::Add => Op::Add,
                        AsiOp::Sub => Op::Sub,
                        AsiOp::Mul => Op::Mul,
                        AsiOp::Div => Op::Div,
                        AsiOp::Mod => Op::Mod,
                        AsiOp::BAnd => Op::BAnd,
                        AsiOp::BOr => Op::BOr,
                        AsiOp::BXor => Op::Xor,
                        AsiOp::Shl => Op::Shl,
                        AsiOp::AShr => Op::AShr,
                        _ => Op::LShr,
                    };
                    self.gen_expr(right, false, false);
                    self.cg.write_op(opc);
                    self.cg.note_pop();
                    self.gen_asi(left, discard, 2, *op, None, false, true);
                }
            }
            Expr::IncDec(op, sub) => {
                let (opc, post) = match op {
                    AsiOp::PreInc => (Op::Inc, false),
                    AsiOp::PreDec => (Op::Dec, false),
                    AsiOp::PostInc => (Op::Inc, true),
                    _ => (Op::Dec, true),
                };
                if !self.gen_asi(sub, discard, 1, *op, None, false, true) {
                    if post {
                        // The result is the value from before.
                        if !discard {
                            self.cg.write_op(Op::Dup);
                            self.cg.note_push();
                        }
                        self.cg.write_op(opc);
                        self.gen_asi(sub, true, 2, *op, None, false, true);
                    } else {
                        self.cg.write_op(opc);
                        self.gen_asi(sub, discard, 2, *op, None, false, true);
                    }
                }
            }
            Expr::If(first, second, third) => {
                self.gen_expr(first, false, true);
                let lbl_else = self.gen_jump_ahead(Op::Jf);
                self.cg.note_pop();
                // A branch that has no value still has to leave one if
                // the other does.
                let second_val = self.has_return_value(second);
                let third_val = self.has_return_value(third);
                self.gen_expr(second, discard || !second_val, for_condition);
                if !discard && !second_val {
                    self.cg.write_op(Op::PushNil);
                    self.cg.note_push();
                }
                let lbl_end = self.gen_jump_ahead(Op::Jmp);
                self.def_label_pos(lbl_else);
                self.gen_expr(third, discard || !third_val, for_condition);
                if !discard && !third_val {
                    self.cg.write_op(Op::PushNil);
                    self.cg.note_push();
                }
                // Only one branch runs, so only one value is left.
                if second_val || third_val {
                    self.cg.note_pop();
                }
                self.def_label_pos(lbl_end);
            }
            Expr::IfNil(first, second) => {
                self.gen_expr(first, false, false);
                if !discard {
                    self.cg.write_op(Op::Dup);
                    self.cg.note_push();
                }
                let lbl_end = self.gen_jump_ahead(Op::JNotNil);
                self.cg.note_pop();
                if !discard {
                    self.cg.write_op(Op::Disc);
                    self.cg.note_pop();
                }
                self.gen_expr(second, discard, for_condition);
                self.def_label_pos(lbl_end);
            }
            Expr::Call(func, args) => {
                let varargs = self.gen_arglist(args);
                self.gen_call(func, discard, args.len(), varargs);
            }
        }
    }

    /// Whether evaluating an expression leaves a value. A call to a
    /// function that returns nothing does not; but by now the name of a
    /// function has become a constant, and the reference takes a call
    /// through a constant to leave a value whatever the function does.
    /// So only a call by a name that was never defined is looked up.
    fn has_return_value(&self, expr: &Expr) -> bool {
        match expr {
            Expr::Call(func, _) => match &**func {
                Expr::Sym(name) => match self.prog.globals.get(name) {
                    Some(Global::Func(func)) => self.prog.funcs[*func].has_retval,
                    Some(Global::Bif(_)) | None => true,
                },
                _ => true,
            },
            Expr::Binary(BinOp::Comma, _, right) => self.has_return_value(right),
            Expr::If(_, second, third) => {
                self.has_return_value(second) || self.has_return_value(third)
            }
            Expr::IfNil(first, second) => {
                self.has_return_value(first) || self.has_return_value(second)
            }
            Expr::DStr(_) | Expr::DStrEmbed(_) => false,
            Expr::OneOf(dbl, _) => !dbl,
            _ => true,
        }
    }

    /// Report a name that nothing defines. It is reported once: the
    /// reference then notes the name as one that is known to be
    /// undefined, and says nothing about it again.
    fn undefined(&mut self, name: &[u8]) {
        if self.undefined_names.insert(name.to_vec()) {
            self.error(format!("undefined symbol \"{}\"", show_name(name)));
        }
    }

    fn gen_unary(&mut self, opc: Op, sub: &Expr, discard: bool, for_condition: bool) {
        self.gen_expr(sub, discard, for_condition);
        if !discard {
            self.cg.write_op(opc);
        }
    }

    fn gen_binary(&mut self, opc: Op, left: &Expr, right: &Expr, discard: bool) {
        self.gen_expr(left, discard, false);
        self.gen_expr(right, discard, false);
        if !discard {
            self.cg.write_op(opc);
            self.cg.note_pop();
        }
    }

    /// Evaluate one side for whatever it does, and leave zero.
    fn gen_zero_after(&mut self, side: &Expr) {
        self.gen_expr(side, true, true);
        self.cg.write_op(Op::Push0);
        self.cg.note_push();
    }

    fn gen_binary_expr(
        &mut self,
        op: BinOp,
        left: &Expr,
        right: &Expr,
        discard: bool,
        for_condition: bool,
    ) {
        match op {
            BinOp::Comma => {
                self.gen_expr(left, true, true);
                self.gen_expr(right, discard, for_condition);
            }
            BinOp::Or | BinOp::And => {
                // The right side is only evaluated if the left does not
                // settle it. When the value is wanted, the jump keeps
                // the left side's value if it is taken.
                self.gen_expr(left, false, true);
                let opc = match (op, discard) {
                    (BinOp::Or, true) => Op::Jt,
                    (BinOp::Or, false) => Op::Jst,
                    (_, true) => Op::Jf,
                    (_, false) => Op::Jsf,
                };
                let lbl = self.gen_jump_ahead(opc);
                self.gen_expr(right, discard, true);
                self.cg.note_pop();
                self.def_label_pos(lbl);
                if !for_condition && !(left.is_bool() && right.is_bool()) {
                    self.cg.write_op(Op::Boolize);
                }
            }
            BinOp::BOr => self.gen_binary(Op::BOr, left, right, discard),
            BinOp::BAnd => self.gen_binary(Op::BAnd, left, right, discard),
            BinOp::BXor => self.gen_binary(Op::Xor, left, right, discard),
            BinOp::Eq => self.gen_binary(Op::Eq, left, right, discard),
            BinOp::Ne => self.gen_binary(Op::Ne, left, right, discard),
            BinOp::Gt => self.gen_binary(Op::Gt, left, right, discard),
            BinOp::Ge => self.gen_binary(Op::Ge, left, right, discard),
            BinOp::Lt => self.gen_binary(Op::Lt, left, right, discard),
            BinOp::Le => self.gen_binary(Op::Le, left, right, discard),
            BinOp::Shl => self.gen_binary(Op::Shl, left, right, discard),
            BinOp::AShr => self.gen_binary(Op::AShr, left, right, discard),
            BinOp::LShr => self.gen_binary(Op::LShr, left, right, discard),
            BinOp::Mul => {
                // Multiplying by zero or one needs no multiplication.
                // The zero is pushed even when the value is not wanted,
                // as the reference does.
                if left.is_const_int(0) {
                    self.gen_zero_after(right);
                } else if right.is_const_int(0) {
                    self.gen_zero_after(left);
                } else if left.is_const_int(1) {
                    self.gen_expr(right, discard, false);
                } else if right.is_const_int(1) {
                    self.gen_expr(left, discard, false);
                } else {
                    self.gen_binary(Op::Mul, left, right, discard);
                }
            }
            BinOp::Div => {
                if right.is_const_int(1) {
                    self.gen_expr(left, discard, for_condition);
                } else if left.is_const_int(0) {
                    self.gen_zero_after(right);
                } else {
                    self.gen_binary(Op::Div, left, right, discard);
                }
            }
            BinOp::Mod => {
                if right.is_const_int(1) {
                    self.gen_expr(left, discard, for_condition);
                    self.cg.write_op(Op::Push0);
                    self.cg.note_push();
                } else if left.is_const_int(0) {
                    self.gen_zero_after(right);
                } else {
                    self.gen_binary(Op::Mod, left, right, discard);
                }
            }
            BinOp::Sub | BinOp::Add => {
                let (plain, by_one) = if op == BinOp::Add {
                    (Op::Add, Op::Inc)
                } else {
                    (Op::Sub, Op::Dec)
                };
                if right.is_const_int(1) {
                    self.gen_expr(left, discard, false);
                    if !discard {
                        self.cg.write_op(by_one);
                    }
                } else {
                    self.gen_binary(plain, left, right, discard);
                }
            }
        }
    }

    // ---- calls ----

    /// Push a call's arguments, last first. True if any of them is a
    /// list to be spread into arguments.
    fn gen_arglist(&mut self, args: &[Arg]) -> bool {
        let varargs = args.iter().any(|arg| arg.varargs);
        let fixed_cnt = args.iter().filter(|arg| !arg.varargs).count();
        let mut pushed_varargs_counter = false;
        for (index, arg) in args.iter().enumerate().rev() {
            let depth = self.cg.sp_depth();
            if arg.varargs && !pushed_varargs_counter {
                // The count starts at the number of plain arguments; the
                // lists add their lengths as the program runs.
                self.gen_int(fixed_cnt as i32);
                pushed_varargs_counter = true;
                self.cg.note_pop();
            }
            self.gen_expr(&arg.expr, false, false);
            if arg.varargs {
                self.cg.write_op(Op::MakeLstPar);
                self.cg.note_push();
                self.cg.note_pop();
            }
            if pushed_varargs_counter && !arg.varargs {
                self.cg.write_op(Op::Swap);
            }
            if self.cg.sp_depth() <= depth {
                self.error(format!("argument expression {} yields no value", index + 1));
            }
        }
        varargs
    }

    fn gen_call(&mut self, func: &Expr, discard: bool, argc: usize, varargs: bool) {
        match func {
            Expr::Const(ConstVal {
                value: Value::FuncPtr(id),
                ..
            }) => {
                let id = *id;
                if !self.prog.funcs[id].has_proto {
                    let name = show_name(&self.prog.funcs[id].name);
                    self.error(format!(
                        "cannot call extern function {name} without an argument list definition"
                    ));
                    return;
                }
                if varargs {
                    self.cg.write_op(Op::VarArgc);
                }
                self.cg.write_op(Op::Call);
                self.cg.write(argc as u8);
                self.gen_func_ref(id);
                self.cg.note_pops(argc as i32);
                let func = &self.prog.funcs[id];
                if !func.argc_ok(argc) {
                    let message = format!(
                        "wrong number of arguments to function \"{}\": {} required, {} actual",
                        show_name(&func.name),
                        func.argc_desc(),
                        argc
                    );
                    self.error(message);
                }
                if !discard {
                    self.cg.write_op(Op::GetR0);
                    self.cg.note_push();
                }
            }
            Expr::Const(_) => {
                self.error("this constant expression cannot be called as a function");
            }
            Expr::Sym(name) => match self.prog.globals.get(name) {
                Some(Global::Bif(bif)) => self.gen_bif_call(*bif, discard, argc, varargs),
                _ => self.undefined(name),
            },
            _ => {
                // Anything else is taken to be a pointer to a function.
                self.gen_expr(func, false, false);
                if varargs {
                    self.cg.write_op(Op::Swap);
                    self.cg.write_op(Op::VarArgc);
                }
                self.cg.write_op(Op::PtrCall);
                self.cg.write(argc as u8);
                self.cg.note_pops(argc as i32 + 1);
                if !discard {
                    self.cg.write_op(Op::GetR0);
                    self.cg.note_push();
                }
            }
        }
    }

    /// Call a function built into the virtual machine.
    fn gen_bif_call(&mut self, bif: usize, discard: bool, argc: usize, varargs: bool) {
        let BifSym {
            name,
            set,
            index,
            min_argc,
            max_argc,
            varargs: takes_more,
        } = self.prog.bifs[bif].clone();
        if argc < min_argc {
            self.error(format!(
                "too few arguments to function \"{}\"",
                show_name(&name)
            ));
        } else if !takes_more && argc > max_argc {
            self.error(format!(
                "too many arguments to function \"{}\"",
                show_name(&name)
            ));
        }
        if varargs {
            self.cg.write_op(Op::VarArgc);
        }
        // The first four sets have short instructions of their own.
        if set < 4 && index < 256 {
            let opc = [Op::BuiltinA, Op::BuiltinB, Op::BuiltinC, Op::BuiltinD][set];
            self.cg.write_op(opc);
            self.cg.write(argc as u8);
            self.cg.write(index as u8);
        } else {
            if index < 256 {
                self.cg.write_op(Op::Builtin1);
                self.cg.write(argc as u8);
                self.cg.write(index as u8);
            } else {
                self.cg.write_op(Op::Builtin2);
                self.cg.write(argc as u8);
                self.cg.write2(index as u16);
            }
            self.cg.write(set as u8);
        }
        self.cg.note_pops(argc as i32);
        if !discard {
            self.cg.write_op(Op::GetR0);
            self.cg.note_push();
        }
    }

    // ---- locals ----

    fn gen_local(&mut self, id: LocalId, discard: bool) {
        if !discard {
            let local = &self.prog.locals[id];
            self.gen_getlcl(local.var_num, local.is_param);
        }
        // The variable has been used, whether or not its value was
        // wanted here.
        self.prog.locals[id].val_used = true;
    }

    fn gen_getlcl(&mut self, var_num: usize, is_param: bool) {
        if is_param && var_num < 4 {
            let opc = [Op::GetArgN0, Op::GetArgN1, Op::GetArgN2, Op::GetArgN3][var_num];
            self.cg.write_op(opc);
        } else if !is_param && var_num < 6 {
            let opc = [
                Op::GetLclN0,
                Op::GetLclN1,
                Op::GetLclN2,
                Op::GetLclN3,
                Op::GetLclN4,
                Op::GetLclN5,
            ][var_num];
            self.cg.write_op(opc);
        } else if var_num <= 255 {
            self.cg
                .write_op(if is_param { Op::GetArg1 } else { Op::GetLcl1 });
            self.cg.write(var_num as u8);
        } else {
            self.cg
                .write_op(if is_param { Op::GetArg2 } else { Op::GetLcl2 });
            self.cg.write2(var_num as u16);
        }
        self.cg.note_push();
    }

    fn gen_setlcl_stk(&mut self, var_num: usize, is_param: bool) {
        if var_num <= 255 {
            self.cg
                .write_op(if is_param { Op::SetArg1 } else { Op::SetLcl1 });
            self.cg.write(var_num as u8);
        } else {
            self.cg
                .write_op(if is_param { Op::SetArg2 } else { Op::SetLcl2 });
            self.cg.write2(var_num as u16);
        }
        self.cg.note_pop();
    }

    /// Generate an assignment to an expression. In phase 1, the target
    /// may do the whole job with instructions of its own, and says so
    /// by returning true; otherwise it leaves its present value on the
    /// stack, the caller computes the new value, and phase 2 stores it.
    #[allow(clippy::too_many_arguments)]
    fn gen_asi(
        &mut self,
        target: &Expr,
        discard: bool,
        phase: u8,
        typ: AsiOp,
        rhs: Option<&Expr>,
        ignore_error: bool,
        explicit: bool,
    ) -> bool {
        match target {
            Expr::Local(id) => self.gen_local_asi(*id, discard, phase, typ, rhs, explicit),
            Expr::Index(left, index) => {
                self.gen_index_asi(left, index, discard, phase, typ, rhs, explicit)
            }
            Expr::Sym(name) => {
                if ignore_error || phase > 1 {
                    return false;
                }
                if self.prog.globals.contains_key(name) {
                    self.error(format!("cannot assign to \"{}\"", show_name(name)));
                    return true;
                }
                self.undefined(name);
                false
            }
            _ => {
                if !ignore_error && phase == 1 {
                    let message = if matches!(
                        typ,
                        AsiOp::PreInc | AsiOp::PreDec | AsiOp::PostInc | AsiOp::PostDec
                    ) {
                        format!(
                            "invalid lvalue - cannot apply \"{}\" operator to expression",
                            typ.text()
                        )
                    } else {
                        format!(
                            "invalid lvalue - cannot assign to expression on left of \"{}\"",
                            typ.text()
                        )
                    };
                    self.error(message);
                }
                false
            }
        }
    }

    /// Assign to an element: `left[index] = value`. Storing into an
    /// element gives a new container, which is then stored back into
    /// `left` if `left` can be assigned to.
    #[allow(clippy::too_many_arguments)]
    fn gen_index_asi(
        &mut self,
        left: &Expr,
        index: &Expr,
        discard: bool,
        phase: u8,
        typ: AsiOp,
        rhs: Option<&Expr>,
        explicit: bool,
    ) -> bool {
        if typ == AsiOp::Simple {
            if let Some(rhs) = rhs {
                self.gen_expr(rhs, false, false);
            }
            if !discard {
                self.cg.write_op(Op::Dup);
                self.cg.note_push();
            }
            // Have the container left on the stack. What is being
            // indexed may itself be an element of something, and then
            // there is more on the stack than the container.
            let rhs_depth = self.cg.sp_depth();
            self.gen_asi(left, false, 1, AsiOp::Idx, None, true, explicit);
            let mut extra_disc = 0;
            if self.cg.sp_depth() == rhs_depth {
                self.gen_expr(left, false, false);
            } else if self.cg.sp_depth() > rhs_depth + 1 {
                let dist = self.cg.sp_depth() - rhs_depth;
                if dist > 255 {
                    self.error("expression too complex");
                }
                self.cg.write_op(Op::GetSpN);
                self.cg.write(dist as u8);
                self.cg.note_push();
                self.cg.write_op(Op::Swap);
                extra_disc += 1;
            }
            self.gen_expr(index, false, false);
            self.cg.write_op(Op::SetInd);
            self.cg.note_pops(2);
            if self.cg.last_op() == Op::SetIndLcl1I8 as u8 {
                // The instruction writer made it one instruction that
                // stores back into the local too.
                self.cg.note_pop();
            } else if !self.gen_asi(left, true, 2, AsiOp::Idx, None, true, explicit) {
                self.cg.write_op(Op::Disc);
                self.cg.note_pop();
            }
            for _ in 0..extra_disc {
                self.cg.write_op(Op::Disc);
                self.cg.note_pop();
            }
            true
        } else if phase == 1 {
            // Leave the container, the index, and the element's present
            // value for the caller to work on.
            self.gen_asi(left, false, 1, AsiOp::Idx, None, true, explicit);
            self.gen_expr(index, false, false);
            self.cg.write_op(Op::Dup2);
            self.cg.note_pushes(2);
            self.cg.write_op(Op::Index);
            self.cg.note_pop();
            false
        } else if phase == 2 {
            if !discard {
                self.cg.write_op(Op::Dup);
                self.cg.note_push();
            }
            // Bring the container and the index back above the value.
            if discard {
                self.cg.write_op(Op::SwapN);
                self.cg.write(0);
                self.cg.write(2);
                self.cg.write_op(Op::Swap);
            } else {
                self.cg.write_op(Op::Swap2);
            }
            self.cg.write_op(Op::SetInd);
            self.cg.note_pops(2);
            if !self.gen_asi(left, true, 2, AsiOp::Idx, None, true, explicit) {
                self.cg.write_op(Op::Disc);
                self.cg.note_pop();
            }
            true
        } else {
            false
        }
    }

    fn gen_local_asi(
        &mut self,
        id: LocalId,
        discard: bool,
        phase: u8,
        typ: AsiOp,
        rhs: Option<&Expr>,
        explicit: bool,
    ) -> bool {
        if explicit {
            self.prog.locals[id].val_assigned = true;
        }
        if typ != AsiOp::Simple {
            self.prog.locals[id].val_used = true;
        }
        let var_num = self.prog.locals[id].var_num;
        let is_param = self.prog.locals[id].is_param;
        // The second phase of any assignment is a plain store.
        let typ = if phase == 2 { AsiOp::Simple } else { typ };
        match typ {
            AsiOp::Simple => {
                // Setting a local to nil, zero or one has instructions
                // of its own.
                let short = if is_param {
                    None
                } else {
                    match rhs.and_then(Expr::as_const).map(|value| &value.value) {
                        Some(Value::Nil) => Some((Op::NilLcl1, Op::NilLcl2, Op::PushNil)),
                        Some(Value::Int(0)) => Some((Op::ZeroLcl1, Op::ZeroLcl2, Op::Push0)),
                        Some(Value::Int(1)) => Some((Op::OneLcl1, Op::OneLcl2, Op::Push1)),
                        _ => None,
                    }
                };
                if let Some((op1, op2, push)) = short {
                    if var_num <= 255 {
                        self.cg.write_op(op1);
                        self.cg.write(var_num as u8);
                    } else {
                        self.cg.write_op(op2);
                        self.cg.write2(var_num as u16);
                    }
                    if !discard {
                        self.cg.write_op(push);
                        self.cg.note_push();
                    }
                    return true;
                }
                if let Some(rhs) = rhs {
                    self.gen_expr(rhs, false, false);
                }
                if !discard {
                    self.cg.write_op(Op::Dup);
                    self.cg.note_push();
                }
                self.gen_setlcl_stk(var_num, is_param);
                true
            }
            AsiOp::Add | AsiOp::Sub => {
                if is_param {
                    if phase == 1 {
                        self.gen_local(id, false);
                    }
                    return false;
                }
                let adding = typ == AsiOp::Add;
                let const_int = rhs
                    .and_then(Expr::as_const)
                    .and_then(|value| match value.value {
                        Value::Int(int) => Some(int),
                        _ => None,
                    })
                    .filter(|_| adding);
                if let Some(int) = const_int {
                    if int == 1 {
                        self.cg.write_op(Op::IncLcl);
                        self.cg.write2(var_num as u16);
                    } else if int == -1 {
                        self.cg.write_op(Op::DecLcl);
                        self.cg.write2(var_num as u16);
                    } else if (-128..=127).contains(&int) && var_num <= 255 {
                        self.cg.write_op(Op::AddILcl1);
                        self.cg.write(var_num as u8);
                        self.cg.write(int as u8);
                    } else {
                        self.cg.write_op(Op::AddILcl4);
                        self.cg.write2(var_num as u16);
                        self.cg.write4(int as u32);
                    }
                } else {
                    if let Some(rhs) = rhs {
                        self.gen_expr(rhs, false, false);
                    }
                    self.cg
                        .write_op(if adding { Op::AddToLcl } else { Op::SubFromLcl });
                    self.cg.write2(var_num as u16);
                    self.cg.note_pop();
                }
                if !discard {
                    self.gen_local(id, false);
                }
                true
            }
            AsiOp::PreInc | AsiOp::PreDec | AsiOp::PostInc | AsiOp::PostDec => {
                if is_param {
                    if phase == 1 {
                        self.gen_local(id, false);
                    }
                    return false;
                }
                let post = matches!(typ, AsiOp::PostInc | AsiOp::PostDec);
                let opc = if matches!(typ, AsiOp::PreInc | AsiOp::PostInc) {
                    Op::IncLcl
                } else {
                    Op::DecLcl
                };
                if post && !discard {
                    self.gen_local(id, false);
                }
                self.cg.write_op(opc);
                self.cg.write2(var_num as u16);
                if !post && !discard {
                    self.gen_local(id, false);
                }
                true
            }
            _ => {
                // The other kinds have nothing special: leave the value
                // for the caller to work on.
                self.gen_local(id, false);
                false
            }
        }
    }
}
