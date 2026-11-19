//! How control leaves a statement.
//!
//! The parser works this out for each function body once it has read
//! it, to warn about code that cannot be reached and about functions
//! that return a value on some paths and not on others. The code
//! generator asks again for a `switch` and a `finally`, to leave out
//! jumps that could never be taken.

use crate::ast::{LabelSym, Stmt, StmtKind, flow};

/// What working out the control flow needs to know, and what it finds.
pub(crate) struct Flow<'a> {
    /// Whether to collect warnings.
    pub warn: bool,
    /// The source is being read only to learn what it defines.
    pub syntax_only: bool,
    /// The labels of the function the statement is in.
    pub labels: &'a [LabelSym],
    /// The labels that a `goto`, `break`, or `continue` naming a label
    /// is recorded as a use of. While a function is being read, they
    /// are its own. Afterward the reference still has the labels of the
    /// last function it read, and it is those that get marked, whatever
    /// function the statement is in.
    pub mark: &'a [LabelSym],
    /// The warnings: where, and what.
    pub warnings: Vec<(usize, u32, &'static str)>,
}

impl Flow<'_> {
    fn mark(&self, name: &[u8], flag: u32) {
        if let Some(label) = self.mark.iter().find(|label| label.name == name) {
            label.flags.set(label.flags.get() | flag);
        }
    }

    fn part(&mut self, part: &Option<Box<Stmt>>) -> u32 {
        match part {
            Some(part) => self.control_flow(part),
            None => flow::NEXT,
        }
    }

    /// The ways a statement can be left.
    pub(crate) fn control_flow(&mut self, stmt: &Stmt) -> u32 {
        match &stmt.kind {
            StmtKind::Comp { stmts, .. } => {
                let mut flags = flow::NEXT;
                let mut index = 0;
                while index < stmts.len() {
                    let flags1 = self.control_flow(&stmts[index]);
                    flags |= flags1
                        & (flow::THROW
                            | flow::RET_VOID
                            | flow::RET_VAL
                            | flow::GOTO
                            | flow::BREAK
                            | flow::CONT);
                    if flags1 & flow::NEXT == 0 {
                        // Nothing after this can be reached, unless
                        // something further on has a label.
                        flags &= !flow::NEXT;
                        if let Some(next) = stmts.get(index + 1) {
                            if self.warn && !next.has_code_label() && !self.syntax_only {
                                self.warnings
                                    .push((next.file, next.line, "unreachable statement"));
                            }
                            let mut found = index + 1;
                            while found < stmts.len() && !stmts[found].has_code_label() {
                                found += 1;
                            }
                            if found < stmts.len() {
                                flags |= flow::NEXT;
                            }
                            index = found;
                            continue;
                        }
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
                    return self.part(part);
                }
                let flags1 = self.part(then_part);
                let flags2 = self.part(else_part);
                flags1 | flags2
            }
            StmtKind::Return(expr) => {
                if expr.is_some() {
                    flow::RET_VAL
                } else {
                    flow::RET_VOID
                }
            }
            StmtKind::For { cond, body, .. } => {
                let cond_const = cond.as_ref().and_then(|cond| cond.as_const());
                if let (Some(value), Some(body)) = (cond_const, body) {
                    if !value.is_true() && !body.has_code_label() && !self.syntax_only {
                        if self.warn && !value.ctc {
                            self.warnings.push((
                                stmt.file,
                                stmt.line,
                                "\"for\" condition is always false - body and reinitializer are unreachable",
                            ));
                        }
                        return flow::NEXT;
                    }
                }
                let mut flags = match body {
                    Some(body) => self.control_flow(body),
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
            StmtKind::While { cond, body, .. } => {
                let cond_const = cond.as_const();
                if let (Some(value), Some(body)) = (cond_const, body) {
                    // The reference only takes this way out when it is
                    // reading for symbols, so that is when it warns.
                    if !value.is_true() && !body.has_code_label() && self.syntax_only {
                        if self.warn && !value.ctc {
                            self.warnings.push((
                                stmt.file,
                                stmt.line,
                                "\"while\" condition is always false - loop body is unreachable",
                            ));
                        }
                        return flow::NEXT;
                    }
                }
                let mut flags = match body {
                    Some(body) => self.control_flow(body),
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
            StmtKind::DoWhile { body, cond, .. } => {
                let mut flags = match body {
                    Some(body) => self.control_flow(body),
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
            StmtKind::Break(label) => match label {
                None => flow::BREAK,
                Some(name) => {
                    self.mark(name, flow::BREAK);
                    flow::GOTO
                }
            },
            StmtKind::Continue(label) => match label {
                None => flow::CONT,
                Some(name) => {
                    self.mark(name, flow::CONT);
                    flow::GOTO
                }
            },
            StmtKind::Switch {
                body, has_default, ..
            } => {
                let mut flags = self.control_flow(body);
                if flags & flow::BREAK != 0 {
                    flags |= flow::NEXT;
                    flags &= !flow::BREAK;
                }
                // Without a `default`, some value probably matches no
                // case.
                if !has_default {
                    flags |= flow::NEXT;
                }
                flags
            }
            StmtKind::Case { stm, .. } | StmtKind::Default(stm) => self.part(stm),
            StmtKind::Label { label, stm, .. } => {
                let mut flags = self.part(stm);
                // A `break` that names the label goes to what follows.
                let used = label.map_or(0, |label| self.labels[label].flags.get());
                if used & flow::BREAK != 0 {
                    flags |= flow::NEXT;
                }
                flags
            }
            StmtKind::Goto(name) => {
                self.mark(name, flow::GOTO);
                flow::GOTO
            }
            StmtKind::Try {
                body,
                catches,
                finally,
                ..
            } => {
                let mut flags = self.part(body);
                // Any exception could arrive in the protected code, so
                // every `catch` can be entered.
                for catch in catches {
                    flags |= self.part(&catch.body);
                }
                let mut fin_flags = match finally {
                    Some(finally) => self.part(&finally.body),
                    None => flow::NEXT,
                };
                // Everything leaves through the `finally`; if that
                // never finishes, nothing else gets to do what it was
                // going to.
                if fin_flags & flow::NEXT == 0 {
                    flags &= !(flow::NEXT
                        | flow::BREAK
                        | flow::CONT
                        | flow::RET_VAL
                        | flow::RET_VOID
                        | flow::GOTO);
                }
                fin_flags &= !flow::NEXT;
                flags | fin_flags
            }
            StmtKind::Throw(_) => flow::THROW,
            StmtKind::Null | StmtKind::Expr(_) => flow::NEXT,
        }
    }
}
