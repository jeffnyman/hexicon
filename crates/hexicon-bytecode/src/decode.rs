//! Decoding single instructions.

use std::fmt;

use hexicon_image::Value;

use crate::opcode::{Layout, Opcode, OperandKind};

/// A decoded operand.
///
/// Offsets within a function are measured from the start of the function's
/// method header, which is how the VM's own tables express them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Operand {
    /// An unsigned number.
    Unsigned(u32),
    /// A signed number.
    Signed(i32),
    /// An argument count.
    Argc(u16),
    /// A local variable number.
    Local(u16),
    /// A parameter number.
    Arg(u16),
    /// A constant pool offset of a string.
    String(u32),
    /// A constant pool offset of a list.
    List(u32),
    /// An object ID.
    Object(u32),
    /// A property ID.
    Property(u16),
    /// A code pool offset of a function.
    Function(u32),
    /// An enumerator ID.
    Enum(u32),
    /// A branch target, as an offset within the function. A target before
    /// the start of the function, which no valid code has, is negative.
    Branch(i64),
    /// The offset within the function of a named argument table.
    Table(u32),
    /// An index into the image's metaclass dependency list.
    Metaclass(u16),
    /// An index into the image's function set dependency list.
    FunctionSet(u16),
    /// The index of a function within its function set.
    FunctionIndex(u16),
}

/// The variable-length part of the instructions that have one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Payload {
    /// The instruction has only fixed operands.
    None,
    /// The text of a `pushstri`.
    InlineString(Vec<u8>),
    /// The case table of a `switch`.
    Switch {
        /// Each case value and its branch target, as an offset within the function.
        cases: Vec<(Value, i64)>,
        /// The branch target when no case matches.
        default: i64,
    },
    /// The argument names of a `namedargtab`.
    NamedArgs(Vec<String>),
}

/// A decoded instruction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Instruction {
    /// Offset of the instruction within its function.
    pub offset: u32,
    /// The instruction.
    pub opcode: Opcode,
    /// Total encoded length in bytes, including the opcode byte.
    pub len: u32,
    /// The fixed operands, in encoding order.
    pub operands: Vec<Operand>,
    /// The variable-length part, if any.
    pub payload: Payload,
}

impl Instruction {
    /// Offsets within the function that this instruction can branch to.
    pub fn branch_targets(&self) -> Vec<i64> {
        let mut targets: Vec<i64> = self
            .operands
            .iter()
            .filter_map(|operand| match operand {
                Operand::Branch(target) => Some(*target),
                _ => None,
            })
            .collect();
        if let Payload::Switch { cases, default } = &self.payload {
            targets.extend(cases.iter().map(|(_, target)| *target));
            targets.push(*default);
        }
        targets
    }
}

/// Why an instruction could not be decoded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecodeError {
    /// The byte at this offset is not a defined opcode.
    InvalidOpcode {
        /// Offset within the function.
        offset: u32,
        /// The byte found.
        byte: u8,
    },
    /// The instruction at this offset runs past the end of the code.
    Truncated {
        /// Offset within the function.
        offset: u32,
        /// The instruction being decoded.
        opcode: Opcode,
    },
    /// A named argument table's index is inconsistent.
    BadNamedArgTable {
        /// Offset within the function.
        offset: u32,
    },
}

impl fmt::Display for DecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidOpcode { offset, byte } => {
                write!(f, "invalid opcode {byte:#04x} at offset {offset:#x}")
            }
            Self::Truncated { offset, opcode } => {
                write!(
                    f,
                    "{} at offset {offset:#x} runs past the end of the code",
                    opcode.name()
                )
            }
            Self::BadNamedArgTable { offset } => {
                write!(f, "malformed named argument table at offset {offset:#x}")
            }
        }
    }
}

impl std::error::Error for DecodeError {}

/// Decode the instruction at `offset` in `code`.
///
/// `code` starts at the function's method header, so `offset` and all
/// branch targets are relative to the header.
pub fn decode(code: &[u8], offset: u32) -> Result<Instruction, DecodeError> {
    let start = offset as usize;
    let Some(&byte) = code.get(start) else {
        // There is no opcode to name; report the end of the code.
        return Err(DecodeError::InvalidOpcode { offset, byte: 0 });
    };
    let opcode = Opcode::from_byte(byte).ok_or(DecodeError::InvalidOpcode { offset, byte })?;
    let truncated = DecodeError::Truncated { offset, opcode };

    let mut cursor = Cursor {
        code,
        pos: start + 1,
    };
    let mut operands = Vec::new();
    let mut payload = Payload::None;

    match opcode.layout() {
        Layout::Fixed(kinds) => {
            operands.reserve(kinds.len());
            for &kind in kinds {
                operands.push(cursor.operand(kind).ok_or(truncated)?);
            }
        }
        Layout::InlineString => {
            let len = cursor.u16().ok_or(truncated)? as usize;
            payload = Payload::InlineString(cursor.bytes(len).ok_or(truncated)?.to_vec());
        }
        Layout::Switch => {
            let count = cursor.u16().ok_or(truncated)? as usize;
            let mut cases = Vec::with_capacity(count.min(1024));
            for _ in 0..count {
                let holder = cursor.bytes(Value::ENCODED_SIZE).ok_or(truncated)?;
                let value = Value::decode([holder[0], holder[1], holder[2], holder[3], holder[4]]);
                cases.push((value, cursor.branch().ok_or(truncated)?));
            }
            let default = cursor.branch().ok_or(truncated)?;
            payload = Payload::Switch { cases, default };
        }
        Layout::NamedArgTable => {
            let table_len = cursor.u16().ok_or(truncated)? as usize;
            let table = cursor.bytes(table_len).ok_or(truncated)?;
            payload = Payload::NamedArgs(
                named_args(table).ok_or(DecodeError::BadNamedArgTable { offset })?,
            );
        }
    }

    Ok(Instruction {
        offset,
        opcode,
        len: (cursor.pos - start) as u32,
        operands,
        payload,
    })
}

/// Read the names from a named argument table: a count, an index of
/// `count + 1` offsets measured from the start of the index, and the names
/// themselves, each running from its offset to the next.
fn named_args(table: &[u8]) -> Option<Vec<String>> {
    let u16_at = |at: usize| Some(u16::from_le_bytes([*table.get(at)?, *table.get(at + 1)?]));
    let count = u16_at(0)? as usize;
    let index = table.get(2..)?;
    let mut names = Vec::with_capacity(count.min(256));
    for i in 0..count {
        let start = u16_at(2 + i * 2)? as usize;
        let end = u16_at(2 + (i + 1) * 2)? as usize;
        names.push(String::from_utf8_lossy(index.get(start..end)?).into_owned());
    }
    Some(names)
}

struct Cursor<'a> {
    code: &'a [u8],
    pos: usize,
}

impl<'a> Cursor<'a> {
    fn bytes(&mut self, len: usize) -> Option<&'a [u8]> {
        let slice = self.code.get(self.pos..self.pos.checked_add(len)?)?;
        self.pos += len;
        Some(slice)
    }

    fn u8(&mut self) -> Option<u8> {
        Some(self.bytes(1)?[0])
    }

    fn u16(&mut self) -> Option<u16> {
        let b = self.bytes(2)?;
        Some(u16::from_le_bytes([b[0], b[1]]))
    }

    fn u32(&mut self) -> Option<u32> {
        let b = self.bytes(4)?;
        Some(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    /// A branch offset, resolved against its own position.
    fn branch(&mut self) -> Option<i64> {
        let at = self.pos as i64;
        Some(at + i64::from(self.u16()? as i16))
    }

    fn operand(&mut self, kind: OperandKind) -> Option<Operand> {
        Some(match kind {
            OperandKind::U8 => Operand::Unsigned(self.u8()?.into()),
            OperandKind::S8 => Operand::Signed((self.u8()? as i8).into()),
            OperandKind::U16 => Operand::Unsigned(self.u16()?.into()),
            OperandKind::S32 => Operand::Signed(self.u32()? as i32),
            OperandKind::Argc8 => Operand::Argc(self.u8()?.into()),
            OperandKind::Argc16 => Operand::Argc(self.u16()?),
            OperandKind::Local8 => Operand::Local(self.u8()?.into()),
            OperandKind::Local16 => Operand::Local(self.u16()?),
            OperandKind::Arg8 => Operand::Arg(self.u8()?.into()),
            OperandKind::Arg16 => Operand::Arg(self.u16()?),
            OperandKind::String => Operand::String(self.u32()?),
            OperandKind::List => Operand::List(self.u32()?),
            OperandKind::Object => Operand::Object(self.u32()?),
            OperandKind::Property => Operand::Property(self.u16()?),
            OperandKind::Function => Operand::Function(self.u32()?),
            OperandKind::Enum => Operand::Enum(self.u32()?),
            OperandKind::Branch => Operand::Branch(self.branch()?),
            OperandKind::TableOffset => {
                let at = self.pos as u32;
                Operand::Table(at + u32::from(self.u16()?))
            }
            OperandKind::Metaclass8 => Operand::Metaclass(self.u8()?.into()),
            OperandKind::Metaclass16 => Operand::Metaclass(self.u16()?),
            OperandKind::FunctionSet8 => Operand::FunctionSet(self.u8()?.into()),
            OperandKind::FunctionSet16 => Operand::FunctionSet(self.u16()?),
            OperandKind::FunctionIndex8 => Operand::FunctionIndex(self.u8()?.into()),
            OperandKind::FunctionIndex16 => Operand::FunctionIndex(self.u16()?),
        })
    }
}
