//! Names the compiler checks: a property that a game's source gives a
//! value, that nothing in the program reads, and whose name is a slip
//! away from one that something does read.
//!
//! The language makes a property of any name the first time a source uses it
//! as one, so a misspelled property is silently a different property.
//! A misspelled read is already reported, as an undefined symbol,
//! unless something defines the misspelling too. A misspelled
//! definition is not: an object that says `isTakable = nil` where the
//! library reads `isTakeable` has a property that nothing reads, and the
//! one the library reads keeps its inherited value. `dobj Takee { }` is
//! the same mistake for a whole handler: it defines `verifyDobjTakee`
//! and the rest, and nothing calls them.
//!
//! Once every module has been read, the compiler knows every property
//! the program reads (by name, or by taking its address), declares, or
//! gives a value. A value given in a game's own Hexicon source to a
//! property that nothing reads, that no library defines, and that no
//! source declares (`property name;`), is reported when a property
//! that the program reads is spelled within one slip of it. The check
//! changes nothing in the image.

use std::collections::HashSet;

use crate::ast::{Expr, InlineValue, Stmt, StmtKind, Value};
use crate::parse::{Global, ObjDef, Program, PropValue, SPECIAL_PROPS};
use crate::pp::{Diagnostic, Severity};

/// A property named somewhere: the module's own number for the file,
/// and the line.
#[derive(Debug, Clone)]
pub(crate) struct Place {
    pub name: Vec<u8>,
    pub file: usize,
    pub line: u32,
}

/// What one module does with property names.
#[derive(Debug, Default)]
pub(crate) struct NameUses {
    /// The properties it declares, or that are made properties by
    /// something other than a value: `property name;`, `export`, a
    /// built-in class, a grammar rule's `-> name`, a dictionary
    /// property, and the properties every program has.
    pub declared: HashSet<Vec<u8>>,
    /// The properties it reads, by name or by address.
    pub read: HashSet<Vec<u8>>,
    /// The values it gives properties, in object definitions and by
    /// assignment, and where.
    pub definitions: Vec<Place>,
}

/// Whether a name is a property's in the program.
fn is_prop(prog: &Program, name: &[u8]) -> bool {
    matches!(prog.globals.peek(name), Some(Global::Prop(_)))
}

/// The properties a module declares, reads, and gives values.
pub(crate) fn collect(prog: &Program) -> NameUses {
    let mut walk = Walk {
        prog,
        uses: NameUses::default(),
        file: 0,
        line: 0,
    };
    for name in SPECIAL_PROPS {
        walk.declare(name.as_bytes());
    }
    for &prop in &prog.declared_props {
        walk.declare_prop(prop);
    }
    for (name, _) in &prog.exports {
        walk.declare(name);
    }
    for meta in &prog.metas {
        for &prop in &meta.props {
            walk.declare_prop(prop);
        }
    }
    // What a grammar rule matches is kept in the properties its tokens
    // name (`-> prop`). A rule with a tag keeps its ways with its object.
    let alts = prog.gramprods.iter().flat_map(|prod| &prod.alts).chain(
        prog.objs
            .iter()
            .filter_map(|obj| obj.gram.as_ref())
            .flat_map(|(_, alts)| alts),
    );
    for alt in alts {
        for tok in &alt.toks {
            if let Some(prop) = tok.prop {
                walk.declare_prop(prop);
            }
        }
    }
    for &prop in &prog.dict_props {
        walk.declare_prop(prop);
    }
    for def in &prog.objdefs {
        walk.objdef(def);
    }
    for (body, _) in &prog.bodies {
        if !body.replaced {
            walk.stmt(&body.stm);
        }
    }
    walk.uses
}

/// The check, made as the modules are read.
#[derive(Debug, Default)]
pub(crate) struct Checker {
    /// The properties nothing is reported for: those read, declared, or
    /// given values by a library.
    known: HashSet<Vec<u8>>,
    read: HashSet<Vec<u8>>,
    /// The values given in the sources that are checked: the name, the
    /// file's name, and the line.
    given: Vec<(Vec<u8>, String, u32)>,
}

impl Checker {
    /// What a module does with property names. `checked` is for a
    /// game's own Hexicon source; `library` for any source a library
    /// brings in.
    pub(crate) fn add(
        &mut self,
        uses: NameUses,
        checked: bool,
        library: bool,
        file_name: &dyn Fn(usize) -> String,
    ) {
        self.known.extend(uses.declared);
        self.known.extend(uses.read.iter().cloned());
        self.read.extend(uses.read);
        for place in uses.definitions {
            if library {
                self.known.insert(place.name);
            } else if checked {
                self.given
                    .push((place.name, file_name(place.file), place.line));
            }
        }
    }

    /// A warning for each value given to a property that nothing reads,
    /// with the property it is likely meant to be.
    pub(crate) fn warnings(&self) -> Vec<Diagnostic> {
        let mut read: Vec<&Vec<u8>> = self.read.iter().collect();
        read.sort();
        let mut out: Vec<Diagnostic> = Vec::new();
        for (name, file, line) in &self.given {
            if self.known.contains(name) {
                continue;
            }
            let Some(meant) = read.iter().find(|read| is_slip(name, read)) else {
                continue;
            };
            // The handlers of one action, given in one place, are one
            // mistake: `dobj ClimpUp { ... }` defines
            // `verifyDobjClimpUp` and the rest.
            let (message, note) = match (handler(name), handler(meant)) {
                (Some((role, action)), Some((meant_role, meant_action))) if role == meant_role => (
                    format!(
                        "the handler \"{role} {}\" is never called",
                        String::from_utf8_lossy(action)
                    ),
                    format!(
                        "did you mean \"{role} {}\"?",
                        String::from_utf8_lossy(meant_action)
                    ),
                ),
                _ => (
                    format!(
                        "\"{}\" is given a value here, but nothing reads it",
                        String::from_utf8_lossy(name)
                    ),
                    format!("did you mean \"{}\"?", String::from_utf8_lossy(meant)),
                ),
            };
            let duplicate = out
                .iter()
                .any(|d| d.file == *file && d.line == *line && d.message == message);
            if !duplicate {
                out.push(Diagnostic {
                    file: file.clone(),
                    line: *line,
                    severity: Severity::Warning,
                    message,
                    note: Some(note),
                    hexicon: true,
                    ..Default::default()
                });
            }
        }
        out
    }
}

/// The parts of the name of an action's handler for one of its
/// objects: the role, as a handler declares it
/// (`dobj`), and the action (`verifyDobjTake` is `dobj` and `Take`).
fn handler(name: &[u8]) -> Option<(&'static str, &[u8])> {
    const STAGES: [&[u8]; 6] = [
        b"verify", b"check", b"action", b"report", b"preCond", b"remap",
    ];
    const ROLES: [(&[u8], &str); 3] = [(b"Dobj", "dobj"), (b"Iobj", "iobj"), (b"Aobj", "aobj")];
    let rest = STAGES.iter().find_map(|stage| name.strip_prefix(*stage))?;
    ROLES.iter().find_map(|(role, macro_name)| {
        let action = rest.strip_prefix(*role)?;
        (!action.is_empty()).then_some((*macro_name, action))
    })
}

/// Whether one name is a slip for another: one letter left out, added,
/// changed, or two swapped, or only a difference of case. Names that
/// differ only in a final `_`, or only in a final digit, are not: those
/// are usually a family of names (`num` and `num_`, `room1` and
/// `room2`).
pub(crate) fn is_slip(name: &[u8], other: &[u8]) -> bool {
    if name == other || name.len().abs_diff(other.len()) > 1 {
        return false;
    }
    let strip = |s: &[u8]| {
        s.strip_suffix(b"_")
            .map(<[u8]>::to_vec)
            .unwrap_or(s.to_vec())
    };
    if strip(name) == strip(other) {
        return false;
    }
    if let (Some((a, a_rest)), Some((b, b_rest))) = (name.split_last(), other.split_last())
        && a.is_ascii_digit()
        && b.is_ascii_digit()
        && a_rest == b_rest
    {
        return false;
    }
    distance(name, other) <= 1
}

/// The number of letters to leave out, add, or change, or pairs to
/// swap, to make one name the other, ignoring case.
fn distance(a: &[u8], b: &[u8]) -> usize {
    let same = |x: u8, y: u8| x.eq_ignore_ascii_case(&y);
    let mut before: Vec<usize> = Vec::new();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for i in 1..=a.len() {
        let mut cur = vec![i; b.len() + 1];
        for j in 1..=b.len() {
            let cost = usize::from(!same(a[i - 1], b[j - 1]));
            cur[j] = (prev[j] + 1).min(cur[j - 1] + 1).min(prev[j - 1] + cost);
            if i > 1 && j > 1 && same(a[i - 1], b[j - 2]) && same(a[i - 2], b[j - 1]) {
                cur[j] = cur[j].min(before[j - 2] + 1);
            }
        }
        before = std::mem::replace(&mut prev, cur);
    }
    prev[b.len()]
}

struct Walk<'a> {
    prog: &'a Program,
    uses: NameUses,
    file: usize,
    line: u32,
}

impl Walk<'_> {
    fn declare(&mut self, name: &[u8]) {
        self.uses.declared.insert(name.to_vec());
    }

    fn declare_prop(&mut self, prop: usize) {
        let name = self.prog.props[prop].name.clone();
        self.declare(&name);
    }

    /// A value given to a property here.
    fn define_here(&mut self, name: &[u8]) {
        self.uses.definitions.push(Place {
            name: name.to_vec(),
            file: self.file,
            line: self.line,
        });
    }

    fn define_prop_here(&mut self, prop: usize) {
        let name = self.prog.props[prop].name.clone();
        self.define_here(&name);
    }

    fn read(&mut self, name: &[u8]) {
        self.uses.read.insert(name.to_vec());
    }

    fn objdef(&mut self, def: &ObjDef) {
        if def.replaced {
            return;
        }
        for prop in &def.props {
            (self.file, self.line) = (prop.file, prop.line);
            if prop.overwritable {
                self.declare_prop(prop.prop);
            } else {
                self.define_prop_here(prop.prop);
            }
            if let PropValue::Expr(expr) = &prop.value {
                self.expr(expr);
            }
        }
        for (_, prop) in &def.vocab {
            self.declare_prop(*prop);
        }
    }

    fn stmt(&mut self, stmt: &Stmt) {
        (self.file, self.line) = (stmt.file, stmt.line);
        let sub = |walk: &mut Self, stmt: &Option<Box<Stmt>>| {
            if let Some(stmt) = stmt {
                walk.stmt(stmt);
            }
        };
        match &stmt.kind {
            StmtKind::Null
            | StmtKind::ImplicitCtor(_)
            | StmtKind::MmStub(_)
            | StmtKind::MmInit
            | StmtKind::Break(_)
            | StmtKind::Continue(_)
            | StmtKind::Goto(_) => {}
            StmtKind::StaticPropInit(expr, _) => self.expr(expr),
            StmtKind::Comp { stmts, .. } => {
                for stmt in stmts {
                    self.stmt(stmt);
                }
            }
            StmtKind::Expr(expr) | StmtKind::Throw(expr) => self.expr(expr),
            StmtKind::Return(expr) => {
                if let Some(expr) = expr {
                    self.expr(expr);
                }
            }
            StmtKind::If {
                cond,
                then_part,
                else_part,
            } => {
                self.expr(cond);
                sub(self, then_part);
                sub(self, else_part);
            }
            StmtKind::For {
                init,
                cond,
                reinit,
                body,
                ..
            } => {
                for expr in [init, cond, reinit].into_iter().flatten() {
                    self.expr(expr);
                }
                sub(self, body);
            }
            StmtKind::While { cond, body, .. } | StmtKind::DoWhile { body, cond, .. } => {
                self.expr(cond);
                sub(self, body);
            }
            StmtKind::Foreach {
                iter, coll, body, ..
            } => {
                self.assigned(iter);
                self.expr(coll);
                sub(self, body);
            }
            StmtKind::Switch { expr, body, .. } => {
                self.expr(expr);
                self.stmt(body);
            }
            StmtKind::Case { expr, stm } => {
                self.expr(expr);
                sub(self, stm);
            }
            StmtKind::Default(stm) | StmtKind::Label { stm, .. } => sub(self, stm),
            StmtKind::Try {
                body,
                catches,
                finally,
                ..
            } => {
                sub(self, body);
                for catch in catches {
                    sub(self, &catch.body);
                }
                if let Some(finally) = finally {
                    sub(self, &finally.body);
                }
            }
        }
    }

    /// A property named in an expression, if that is what it is.
    fn prop_name<'e>(&self, expr: &'e Expr) -> Option<std::borrow::Cow<'e, [u8]>> {
        match expr {
            Expr::Sym(name) if is_prop(self.prog, name) => Some(name.as_slice().into()),
            Expr::Const(c) => match c.value {
                Value::Prop(id) => self
                    .prog
                    .props
                    .iter()
                    .find(|p| p.id == id)
                    .map(|p| p.name.clone().into()),
                _ => None,
            },
            _ => None,
        }
    }

    /// What an assignment's left side is: a property it sets is defined.
    fn assigned(&mut self, lhs: &Expr) {
        match lhs {
            Expr::Sym(name) if is_prop(self.prog, name) => self.define_here(name),
            Expr::Member {
                obj,
                prop,
                prop_is_expr: false,
                args: None,
            } => {
                self.expr(obj);
                if let Some(name) = self.prop_name(prop) {
                    let name = name.into_owned();
                    self.define_here(&name);
                }
            }
            other => self.expr(other),
        }
    }

    fn args(&mut self, args: &[crate::ast::Arg]) {
        for arg in args {
            self.expr(&arg.expr);
        }
    }

    fn expr(&mut self, expr: &Expr) {
        match expr {
            Expr::Const(_)
            | Expr::Local(_)
            | Expr::Replaced
            | Expr::ArgCount
            | Expr::InhTypes(..)
            | Expr::DStr(_)
            | Expr::SelfRef
            | Expr::Inh
            | Expr::InhClass(_)
            | Expr::CtxEle(_)
            | Expr::AnonFunc { .. } => {}
            Expr::Sym(name) => {
                if is_prop(self.prog, name) {
                    self.read(name);
                }
            }
            Expr::Unary(_, e)
            | Expr::Delegated(e)
            | Expr::Lookup(e)
            | Expr::DStrEmbed(e)
            | Expr::New(e, _) => self.expr(e),
            Expr::Binary(_, a, b) | Expr::IfNil(a, b) | Expr::Index(a, b) => {
                self.expr(a);
                self.expr(b);
            }
            Expr::Assign(op, lhs, rhs) => {
                if *op != crate::ast::AsiOp::Simple {
                    self.expr(lhs);
                }
                self.assigned(lhs);
                self.expr(rhs);
            }
            Expr::IncDec(_, lhs) => {
                self.expr(lhs);
                self.assigned(lhs);
            }
            Expr::If(a, b, c) => {
                self.expr(a);
                self.expr(b);
                self.expr(c);
            }
            Expr::Call(f, args) => {
                self.expr(f);
                self.args(args);
            }
            Expr::InlineObj { props, .. } => {
                for prop in props {
                    (self.file, self.line) = (prop.file, prop.line);
                    self.define_prop_here(prop.prop);
                    match &prop.value {
                        InlineValue::Method(e) | InlineValue::Eval(e) => self.expr(e),
                        InlineValue::Wrapped(a, b) => {
                            self.expr(a);
                            self.expr(b);
                        }
                    }
                }
            }
            Expr::List(items) | Expr::OneOf(_, items, _) => {
                for item in items {
                    self.expr(item);
                }
            }
            Expr::IsIn { left, args, .. } => {
                self.expr(left);
                for arg in args {
                    self.expr(arg);
                }
            }
            Expr::ForIn(for_in) => match &**for_in {
                crate::ast::ForIn::Coll { lval, expr, .. } => {
                    self.assigned(lval);
                    self.expr(expr);
                }
                crate::ast::ForIn::Range {
                    lval,
                    from,
                    to,
                    step,
                    ..
                } => {
                    self.assigned(lval);
                    self.expr(from);
                    self.expr(to);
                    if let Some(step) = step {
                        self.expr(step);
                    }
                }
            },
            Expr::Addr(e) => match self.prop_name(e) {
                Some(name) => {
                    let name = name.into_owned();
                    self.read(&name);
                }
                None => self.expr(e),
            },
            Expr::Member {
                obj,
                prop,
                prop_is_expr,
                args,
            } => {
                self.expr(obj);
                if *prop_is_expr {
                    self.expr(prop);
                } else if let Some(name) = self.prop_name(prop) {
                    let name = name.into_owned();
                    self.read(&name);
                }
                if let Some(args) = args {
                    self.args(args);
                }
            }
        }
    }
}
