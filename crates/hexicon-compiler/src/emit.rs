//! The code generator: a parsed program in, byte code out.
//!
//! Each kind of node in the syntax tree generates its code the way the
//! reference compiler's does, since the instruction writer's peephole
//! rules, the count of stack slots, and the tables after each function
//! all depend on exactly what is written and in what order.

use std::collections::HashMap;

use hexicon_bytecode::{Layout, Opcode as Op};

use crate::ast::{
    Arg, AsiOp, BinOp, CodeBody, ConstVal, Expr, LocalId, Stmt, StmtKind, TabId, UnOp, Value,
};
use crate::codegen::{CodeGen, Label};
use crate::fold::{self, Folded};
use crate::parse::{Global, Program, enum_order};
use crate::pp::{Severity, show_name};

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
    /// The length of the longest function, tables included.
    pub max_bytecode_len: usize,
    /// The length of the longest string.
    pub max_str_len: usize,
    /// Places in the code where two bytes are written that mean
    /// nothing.
    pub unspecified: Vec<usize>,
    pub diagnostics: Vec<GenDiagnostic>,
}

/// Strings shorter than this are stored once however often they occur.
const INTERN_THRESHOLD: usize = 40;

/// A loop that `break` and `continue` can apply to.
struct Enclosing {
    break_lbl: Label,
    cont_lbl: Label,
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
    /// Where the statement being generated is, for messages.
    cur_file: usize,
    cur_line: u32,
    interned: HashMap<Vec<u8>, usize>,
    /// Places in the code that refer to a local's name, in the order
    /// they were written. The names go into the constants after
    /// everything else.
    local_names: Vec<(usize, Vec<u8>)>,
    errors: usize,
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
        cur_file: 0,
        cur_line: 0,
        interned: HashMap::new(),
        local_names: Vec::new(),
        errors: 0,
    };
    // Names are looked up, and what has become constant is worked out,
    // for the whole program before any code is generated.
    for (body, _) in &mut bodies {
        generator.fold_stmt(&mut body.stm);
    }
    let mut scopes = Vec::new();
    for (body, func) in &bodies {
        generator.gen_code_body(body, *func);
        scopes.push(std::mem::take(&mut generator.frames));
    }
    // Locals are checked for use once everything has been generated.
    for frames in scopes {
        for frame in frames {
            generator.check_unreferenced_locals(frame);
        }
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
        if severity == Severity::Error {
            self.errors += 1;
        }
        self.out.diagnostics.push(GenDiagnostic {
            file: self.cur_file,
            line: self.cur_line,
            severity,
            message: message.into(),
        });
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
            StmtKind::Null | StmtKind::Break(_) | StmtKind::Continue(_) => {}
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
            StmtKind::While { cond, body } | StmtKind::DoWhile { body, cond } => {
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
                None => Expr::Sym(name),
            },
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
                    Folded::Yes(expr) => expr,
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
        self.cg.write2_at(method_ofs + 2, body.local_cnt as u16);
        let stack = self.cg.max_sp_depth() as usize + body.local_cnt;
        self.cg.write2_at(method_ofs + 4, stack as u16);
        let end = self.rel_ofs();
        let mut tab = body.lcltab;
        while let Some(id) = tab {
            self.add_to_range(id, end);
            tab = self.prog.tabs[id].parent;
        }

        self.build_debug_table(method_ofs);
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
        self.set_line(sub);
        self.gen_stmt(sub);
        self.set_line(outer);
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
            } => {
                let old_frame = self.set_local_frame(*symtab);
                if let Some(init) = init {
                    self.gen_expr(init, true, true);
                }
                let top_lbl = self.cg.new_label_here();
                let cont_lbl = self.cg.new_label();
                let end_lbl = self.cg.new_label();
                self.enclosing.push(Enclosing {
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
            StmtKind::While { cond, body } => {
                let top_lbl = self.cg.new_label_here();
                let end_lbl = self.cg.new_label();
                self.enclosing.push(Enclosing {
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
            StmtKind::DoWhile { body, cond } => {
                let top_lbl = self.cg.new_label_here();
                let end_lbl = self.cg.new_label();
                let cont_lbl = self.cg.new_label();
                self.enclosing.push(Enclosing {
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
        }
    }

    fn gen_break_or_continue(&mut self, label: Option<&[u8]>, is_break: bool) {
        // With a label, the jump is to a labeled statement around this
        // one, and there are none yet.
        let target = match (label, self.enclosing.last()) {
            (None, Some(enclosing)) => Some(if is_break {
                enclosing.break_lbl
            } else {
                enclosing.cont_lbl
            }),
            _ => None,
        };
        if let Some(target) = target {
            self.cg.write_jump(Op::Jmp, target);
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
        let Some(expr) = expr else {
            self.cg.write_op(Op::RetNil);
            return;
        };
        // Returning nil or true has an instruction of its own.
        match expr.as_const().map(|value| &value.value) {
            Some(Value::Nil) => self.cg.write_op(Op::RetNil),
            Some(Value::True) => self.cg.write_op(Op::RetTrue),
            _ => {
                let depth = self.cg.sp_depth();
                self.gen_expr(expr, false, false);
                if self.cg.sp_depth() > depth {
                    self.cg.write_op(Op::RetVal);
                    self.cg.note_pop();
                } else {
                    // The expression left nothing to return.
                    self.cg.write_op(Op::RetNil);
                }
            }
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

    fn gen_str(&mut self, text: &[u8]) {
        self.cg.write_op(Op::PushStr);
        self.cg.note_push();
        let ofs = self.cg.ofs();
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
        self.out.fixups.push(Fixup {
            ofs,
            target: Target::Const(anchor),
        });
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
                }
                self.cg.note_push();
            }
            Expr::Sym(name) => self.undefined(name),
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
                self.gen_asi(left, discard, 1, AsiOp::Simple, Some(right), true);
            }
            Expr::Assign(op, left, right) => {
                if !self.gen_asi(left, discard, 1, *op, Some(right), true) {
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
                    self.gen_asi(left, discard, 2, *op, None, true);
                }
            }
            Expr::IncDec(op, sub) => {
                let (opc, post) = match op {
                    AsiOp::PreInc => (Op::Inc, false),
                    AsiOp::PreDec => (Op::Dec, false),
                    AsiOp::PostInc => (Op::Inc, true),
                    _ => (Op::Dec, true),
                };
                if !self.gen_asi(sub, discard, 1, *op, None, true) {
                    if post {
                        // The result is the value from before.
                        if !discard {
                            self.cg.write_op(Op::Dup);
                            self.cg.note_push();
                        }
                        self.cg.write_op(opc);
                        self.gen_asi(sub, true, 2, *op, None, true);
                    } else {
                        self.cg.write_op(opc);
                        self.gen_asi(sub, discard, 2, *op, None, true);
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
                    None => true,
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
            _ => true,
        }
    }

    fn undefined(&mut self, name: &[u8]) {
        self.error(format!("undefined symbol \"{}\"", show_name(name)));
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
            Expr::Sym(name) => self.undefined(name),
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
    fn gen_asi(
        &mut self,
        target: &Expr,
        discard: bool,
        phase: u8,
        typ: AsiOp,
        rhs: Option<&Expr>,
        explicit: bool,
    ) -> bool {
        match target {
            Expr::Local(id) => self.gen_local_asi(*id, discard, phase, typ, rhs, explicit),
            Expr::Sym(name) => {
                if phase == 1 {
                    self.undefined(name);
                }
                false
            }
            _ => {
                if phase == 1 {
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
