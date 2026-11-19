//! Writing byte code: the code stream, and the writer that every
//! instruction goes through.
//!
//! The instruction writer does the reference compiler's peephole
//! optimizations, in the reference's order, because they decide the
//! exact bytes that come out: a comparison followed by a conditional jump
//! becomes a compare-and-jump, pushing a local and then getting a
//! property becomes one instruction that does both, and so on. Each rule
//! looks only at the last instruction or two written.

use hexicon_bytecode::Opcode as Op;

/// A place in the code that jumps go to. It may not be known yet where
/// it is when a jump to it is written.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Label(usize);

#[derive(Debug, Default)]
struct LabelInfo {
    /// Where the label is, once it has been placed.
    pos: Option<usize>,
    /// The jump operands waiting to be filled in when it is.
    fixups: Vec<usize>,
}

/// The code being generated, and what is known about the last
/// instructions written to it.
#[derive(Debug)]
pub(crate) struct CodeGen {
    /// The byte code.
    pub code: Vec<u8>,
    last_op: u8,
    second_last_op: u8,
    labels: Vec<LabelInfo>,
    /// How many values are on the stack at this point, as far as the
    /// compiler can tell, and the most there have been in this method.
    sp_depth: i32,
    max_sp_depth: i32,
}

const NOP: u8 = Op::Nop as u8;

impl CodeGen {
    pub(crate) fn new() -> Self {
        Self {
            code: Vec::new(),
            last_op: NOP,
            second_last_op: NOP,
            labels: Vec::new(),
            sp_depth: 0,
            max_sp_depth: 0,
        }
    }

    /// Where the next byte goes.
    pub(crate) fn ofs(&self) -> usize {
        self.code.len()
    }

    pub(crate) fn write(&mut self, byte: u8) {
        self.code.push(byte);
    }

    pub(crate) fn write2(&mut self, value: u16) {
        self.code.extend_from_slice(&value.to_le_bytes());
    }

    pub(crate) fn write4(&mut self, value: u32) {
        self.code.extend_from_slice(&value.to_le_bytes());
    }

    pub(crate) fn write2_at(&mut self, at: usize, value: u16) {
        self.code[at..at + 2].copy_from_slice(&value.to_le_bytes());
    }

    pub(crate) fn write4_at(&mut self, at: usize, value: u32) {
        self.code[at..at + 4].copy_from_slice(&value.to_le_bytes());
    }

    fn byte_at(&self, at: usize) -> u8 {
        self.code[at]
    }

    fn dec_ofs(&mut self, by: usize) {
        self.code.truncate(self.code.len() - by);
    }

    // ---- the stack ----

    pub(crate) fn note_push(&mut self) {
        self.note_pushes(1);
    }

    pub(crate) fn note_pushes(&mut self, count: i32) {
        self.sp_depth += count;
        self.max_sp_depth = self.max_sp_depth.max(self.sp_depth);
    }

    pub(crate) fn note_pop(&mut self) {
        self.sp_depth -= 1;
    }

    pub(crate) fn note_pops(&mut self, count: i32) {
        self.sp_depth -= count;
    }

    pub(crate) fn sp_depth(&self) -> i32 {
        self.sp_depth
    }

    pub(crate) fn max_sp_depth(&self) -> i32 {
        self.max_sp_depth
    }

    /// Start a new method: nothing on the stack, and no instruction
    /// before the first.
    pub(crate) fn start_method(&mut self) {
        self.sp_depth = 0;
        self.max_sp_depth = 0;
        self.clear_peephole();
    }

    /// Forget the last instructions, so that the next one is not
    /// combined with them. This is needed wherever a jump can arrive.
    pub(crate) fn clear_peephole(&mut self) {
        self.last_op = NOP;
        self.second_last_op = NOP;
    }

    /// Whether the next instruction can never be reached, because the
    /// last one always goes somewhere else.
    pub(crate) fn can_skip_op(&self) -> bool {
        [
            Op::Ret,
            Op::RetVal,
            Op::RetTrue,
            Op::RetNil,
            Op::Throw,
            Op::Jmp,
            Op::LRet,
        ]
        .iter()
        .any(|&op| op as u8 == self.last_op)
    }

    // ---- labels ----

    pub(crate) fn new_label(&mut self) -> Label {
        self.labels.push(LabelInfo::default());
        Label(self.labels.len() - 1)
    }

    /// A label at the current position.
    pub(crate) fn new_label_here(&mut self) -> Label {
        let label = self.new_label();
        self.def_label(label);
        label
    }

    /// Put a label at the current position. Jumps can arrive here, so
    /// what came before is no guide to what the next instruction follows.
    pub(crate) fn def_label(&mut self, label: Label) {
        // A jump to the very next instruction does nothing: if the last
        // instruction is a jump to this label, take it back out.
        if self.last_op == Op::Jmp as u8 && self.ofs() >= 3 {
            let operand = self.ofs() - 2;
            let fixups = &mut self.labels[label.0].fixups;
            if let Some(index) = fixups.iter().position(|&at| at == operand) {
                fixups.remove(index);
                self.dec_ofs(3);
            }
        }
        let pos = self.ofs();
        let info = &mut self.labels[label.0];
        info.pos = Some(pos);
        for at in std::mem::take(&mut info.fixups) {
            let offset = pos as i64 - at as i64;
            self.code[at..at + 2].copy_from_slice(&(offset as i16).to_le_bytes());
        }
        self.clear_peephole();
    }

    /// Write a jump's operand: the distance from the operand to the
    /// label.
    pub(crate) fn write_label_ofs(&mut self, label: Label) {
        let at = self.ofs();
        match self.labels[label.0].pos {
            Some(pos) => {
                let offset = pos as i64 - at as i64;
                self.write2(offset as i16 as u16);
            }
            None => {
                self.labels[label.0].fixups.push(at);
                self.write2(0);
            }
        }
    }

    /// Write a jump instruction to a label.
    pub(crate) fn write_jump(&mut self, op: Op, label: Label) {
        self.write_op(op);
        self.write_label_ofs(label);
    }

    // ---- instructions ----

    /// Write an instruction's opcode, combining it with the instruction
    /// before where the two together have a shorter form.
    pub(crate) fn write_op(&mut self, op: Op) {
        let mut opc = op as u8;
        self.write(opc);
        let mut op_len = 1usize;
        let mut prv_len = 1usize;
        let is = |byte: u8, op: Op| byte == op as u8;
        let lcln = |byte: u8| (Op::GetLclN0 as u8..=Op::GetLclN5 as u8).contains(&byte);

        loop {
            let last = self.last_op;
            let combined: Option<u8> = match Op::from_byte(opc) {
                Some(Op::Dup) if is(last, Op::GetR0) => Some(Op::DupR0 as u8),
                Some(Op::Disc) if is(last, Op::Disc) => {
                    // Two discards are one that discards two.
                    op_len = 0;
                    prv_len = 2;
                    let at = self.ofs() - 1;
                    self.code[at] = 2;
                    Some(Op::Disc1 as u8)
                }
                Some(Op::Disc1) if is(last, Op::Disc) && self.byte_at(self.ofs() - 2) != 0xff => {
                    let at = self.ofs() - 2;
                    self.code[at] += 1;
                    prv_len = 2;
                    Some(Op::Disc1 as u8)
                }
                Some(Op::Jf) => match Op::from_byte(last) {
                    Some(Op::Not) => Some(Op::Jt as u8),
                    Some(Op::Eq) => Some(Op::Jne as u8),
                    Some(Op::Ne) => Some(Op::Je as u8),
                    Some(Op::Lt) => Some(Op::Jge as u8),
                    Some(Op::Le) => Some(Op::Jgt as u8),
                    Some(Op::Gt) => Some(Op::Jle as u8),
                    Some(Op::Ge) => Some(Op::Jlt as u8),
                    Some(Op::GetR0) => Some(Op::JR0F as u8),
                    _ => None,
                },
                Some(Op::Je) if is(last, Op::PushNil) => Some(Op::JNil as u8),
                Some(Op::Jne) if is(last, Op::PushNil) => Some(Op::JNotNil as u8),
                Some(Op::Jt) => match Op::from_byte(last) {
                    Some(Op::Not) => Some(Op::Jf as u8),
                    Some(Op::Eq) => Some(Op::Je as u8),
                    Some(Op::Ne) => Some(Op::Jne as u8),
                    Some(Op::Lt) => Some(Op::Jlt as u8),
                    Some(Op::Le) => Some(Op::Jle as u8),
                    Some(Op::Gt) => Some(Op::Jgt as u8),
                    Some(Op::Ge) => Some(Op::Jge as u8),
                    Some(Op::GetR0) => Some(Op::JR0T as u8),
                    _ => None,
                },
                Some(Op::Not) => match Op::from_byte(last) {
                    Some(Op::Eq) => Some(Op::Ne as u8),
                    Some(Op::Ne) => Some(Op::Eq as u8),
                    Some(Op::Gt) => Some(Op::Le as u8),
                    Some(Op::Ge) => Some(Op::Lt as u8),
                    Some(Op::Lt) => Some(Op::Ge as u8),
                    Some(Op::Le) => Some(Op::Gt as u8),
                    Some(Op::Boolize) => Some(Op::Not as u8),
                    Some(Op::Not) => Some(Op::Boolize as u8),
                    _ => None,
                },
                // A return right after a return is left as it is and not
                // looked at again. The reference stops here with the
                // opcode already written, so the second return stays in
                // the code; it is never reached.
                Some(Op::Ret)
                    if is(last, Op::Ret)
                        || is(last, Op::RetVal)
                        || is(last, Op::RetNil)
                        || is(last, Op::RetTrue) =>
                {
                    return;
                }
                Some(Op::RetNil) if is(last, Op::RetNil) => return,
                Some(Op::RetTrue) if is(last, Op::RetTrue) => return,
                Some(Op::RetVal) => match Op::from_byte(last) {
                    Some(Op::GetR0) => Some(Op::Ret as u8),
                    Some(Op::PushNil) => Some(Op::RetNil as u8),
                    Some(Op::PushTrue) => Some(Op::RetTrue as u8),
                    _ => None,
                },
                Some(Op::SetLcl1) => match Op::from_byte(last) {
                    Some(Op::GetR0) => Some(Op::SetLcl1R0 as u8),
                    Some(Op::Dup) => Some(Op::GetSetLcl1 as u8),
                    Some(Op::DupR0) => Some(Op::GetSetLcl1R0 as u8),
                    _ => None,
                },
                Some(Op::GetProp) => {
                    if is(last, Op::GetLcl1) {
                        prv_len = 2;
                        Some(Op::GetPropLcl1 as u8)
                    } else if lcln(last) {
                        // The short form of the push has no operand, so
                        // make room for the local's number.
                        self.write(0);
                        let at = self.ofs() - 2;
                        self.code[at] = last - Op::GetLclN0 as u8;
                        prv_len = 2;
                        Some(Op::GetPropLcl1 as u8)
                    } else if is(last, Op::GetR0) {
                        Some(Op::GetPropR0 as u8)
                    } else {
                        None
                    }
                }
                Some(Op::CallProp) if is(last, Op::GetR0) => Some(Op::CallPropR0 as u8),
                Some(Op::Index) => {
                    if is(last, Op::Push0) || is(last, Op::Push1) {
                        let at = self.ofs() - 1;
                        self.code[at] = u8::from(is(last, Op::Push1));
                        self.write(0);
                        prv_len = 2;
                        Some(Op::IdxInt8 as u8)
                    } else if is(last, Op::PushInt8) {
                        prv_len = 2;
                        Some(Op::IdxInt8 as u8)
                    } else {
                        None
                    }
                }
                Some(Op::IdxInt8) => {
                    if is(last, Op::GetLcl1) {
                        let index = self.byte_at(self.ofs() - op_len + 1);
                        let at = self.ofs() - op_len;
                        self.code[at] = index;
                        op_len -= 1;
                        prv_len = 3;
                        Some(Op::IdxLcl1Int8 as u8)
                    } else if lcln(last) {
                        let at = self.ofs() - op_len;
                        self.code[at] = last - Op::GetLclN0 as u8;
                        if op_len == 1 {
                            self.write(0);
                        }
                        op_len = 0;
                        prv_len = 3;
                        Some(Op::IdxLcl1Int8 as u8)
                    } else {
                        None
                    }
                }
                Some(Op::SetInd) => self.combine_setind(&mut op_len, &mut prv_len),
                _ => None,
            };

            match combined {
                Some(new_opc) => {
                    // Take the new instruction back out, and put the
                    // combined one where the one before it was.
                    opc = new_opc;
                    self.dec_ofs(op_len);
                    let at = self.ofs() - prv_len;
                    self.code[at] = opc;
                    self.last_op = self.second_last_op;
                    self.second_last_op = NOP;
                    op_len = prv_len;
                    prv_len = 1;
                }
                None => break,
            }
        }
        self.second_last_op = self.last_op;
        self.last_op = opc;
    }

    /// Assigning to an element of a local, where the index is a small
    /// constant: `lcl[i] = value` as one instruction.
    fn combine_setind(&mut self, op_len: &mut usize, prv_len: &mut usize) -> Option<u8> {
        let second = self.second_last_op;
        let last = self.last_op;
        let short_local = (Op::GetLclN0 as u8..=Op::GetLclN5 as u8).contains(&second);
        if second != Op::GetLcl1 as u8 && !short_local {
            return None;
        }
        let getlcl_len = if short_local { 1 } else { 2 };
        let ofs = self.ofs();
        let (index, push_len): (i64, usize) = match Op::from_byte(last) {
            Some(Op::PushInt8) => {
                let index = i64::from(self.byte_at(ofs - 2));
                if index >= 127 {
                    return None;
                }
                (index, 2)
            }
            Some(Op::PushInt) => {
                let bytes = [
                    self.byte_at(ofs - 5),
                    self.byte_at(ofs - 4),
                    self.byte_at(ofs - 3),
                    self.byte_at(ofs - 2),
                ];
                let index = i64::from(i32::from_le_bytes(bytes));
                if !(0..=255).contains(&index) {
                    return None;
                }
                (index, 5)
            }
            Some(Op::Push0) => (0, 1),
            Some(Op::Push1) => (1, 1),
            _ => return None,
        };
        let local = if short_local {
            second - Op::GetLclN0 as u8
        } else {
            self.byte_at(ofs - push_len - 2)
        };
        let at = ofs - getlcl_len - push_len;
        self.code[at] = local;
        self.code[at + 1] = index as u8;
        self.second_last_op = NOP;
        *op_len = getlcl_len + push_len - 2;
        *prv_len = 3;
        Some(Op::SetIndLcl1I8 as u8)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn written(build: impl FnOnce(&mut CodeGen)) -> Vec<u8> {
        let mut cg = CodeGen::new();
        build(&mut cg);
        cg.code
    }

    #[test]
    fn a_comparison_and_a_jump_become_one_instruction() {
        let code = written(|cg| {
            let label = cg.new_label();
            cg.write_op(Op::Gt);
            cg.write_jump(Op::Jf, label);
            cg.def_label(label);
        });
        assert_eq!(code, [Op::Jle as u8, 2, 0]);
    }

    #[test]
    fn negations_cancel() {
        let code = written(|cg| {
            cg.write_op(Op::Eq);
            cg.write_op(Op::Not);
            cg.write_op(Op::Not);
        });
        assert_eq!(code, [Op::Eq as u8]);
    }

    #[test]
    fn returns() {
        // Returning nil is one instruction, and a second return after it
        // is written but changes nothing.
        assert_eq!(
            written(|cg| {
                cg.write_op(Op::PushNil);
                cg.write_op(Op::RetVal);
                cg.write_op(Op::RetNil);
            }),
            [Op::RetNil as u8, Op::RetNil as u8]
        );
        assert_eq!(
            written(|cg| {
                cg.write_op(Op::GetR0);
                cg.write_op(Op::RetVal);
            }),
            [Op::Ret as u8]
        );
    }

    #[test]
    fn two_discards_become_one() {
        // Only in pairs: a third is not added to the two before it.
        let code = written(|cg| {
            cg.write_op(Op::Disc);
            cg.write_op(Op::Disc);
            cg.write_op(Op::Disc);
        });
        assert_eq!(code, [Op::Disc1 as u8, 2, Op::Disc as u8]);
    }

    #[test]
    fn a_local_and_what_is_done_with_it_become_one_instruction() {
        // local.prop
        let code = written(|cg| {
            cg.write_op(Op::GetLclN2);
            cg.write_op(Op::GetProp);
            cg.write2(0x1234);
        });
        assert_eq!(code, [Op::GetPropLcl1 as u8, 2, 0x34, 0x12]);

        // local[3]
        let code = written(|cg| {
            cg.write_op(Op::GetLclN1);
            cg.write_op(Op::PushInt8);
            cg.write(3);
            cg.write_op(Op::Index);
        });
        assert_eq!(code, [Op::IdxLcl1Int8 as u8, 1, 3]);

        // local[1] = value
        let code = written(|cg| {
            cg.write_op(Op::GetLcl1);
            cg.write(9);
            cg.write_op(Op::Push1);
            cg.write_op(Op::SetInd);
        });
        assert_eq!(code, [Op::SetIndLcl1I8 as u8, 9, 1]);
    }
}
