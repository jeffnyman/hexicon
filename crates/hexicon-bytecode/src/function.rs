//! Functions: method headers, exception tables, and whole-function disassembly.

use std::collections::BTreeMap;
use std::fmt;

use hexicon_image::Image;

use crate::decode::{DecodeError, Instruction, Operand, Payload, decode};
use crate::opcode::Opcode;

/// The fixed-size record at the start of every function.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MethodHeader {
    /// Number of required parameters.
    pub param_count: u8,
    /// Whether the function accepts any number of further arguments.
    pub varargs: bool,
    /// Number of optional parameters after the required ones.
    pub optional_param_count: u8,
    /// Number of local variables.
    pub local_count: u16,
    /// Stack slots the function needs for locals and intermediate values.
    pub stack_depth: u16,
    /// Offset of the exception table from the start of the header, or 0.
    pub exception_table_offset: u16,
    /// Offset of the debug records from the start of the header, or 0.
    pub debug_records_offset: u16,
}

impl MethodHeader {
    /// The smallest header this crate reads. Images state their own header
    /// size, which may be larger; later fields are ignored.
    pub const MIN_SIZE: usize = 10;

    /// Read a header from the start of `bytes`.
    pub fn parse(bytes: &[u8]) -> Option<Self> {
        let b = bytes.get(..Self::MIN_SIZE)?;
        let u16_at = |at: usize| u16::from_le_bytes([b[at], b[at + 1]]);
        Some(Self {
            param_count: b[0] & 0x7f,
            varargs: b[0] & 0x80 != 0,
            optional_param_count: b[1],
            local_count: u16_at(2),
            stack_depth: u16_at(4),
            exception_table_offset: u16_at(6),
            debug_records_offset: u16_at(8),
        })
    }
}

/// One entry of a function's exception table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExceptionHandler {
    /// Offset of the first byte of protected code.
    pub start: u16,
    /// Offset of the last byte of protected code.
    pub end: u16,
    /// The exception class caught, or 0 to catch everything.
    pub exception_class: u32,
    /// Offset of the handler's code.
    pub handler: u16,
}

/// A disassembled function.
#[derive(Debug, Clone)]
pub struct Function {
    /// Code pool address of the function's method header.
    pub address: u32,
    /// The method header.
    pub header: MethodHeader,
    /// The instructions, in ascending order of offset.
    pub instructions: Vec<Instruction>,
    /// The exception table.
    pub handlers: Vec<ExceptionHandler>,
    /// Offset just past the last decoded instruction.
    pub code_end: u32,
    /// How the extent of the code was established.
    pub extent: Extent,
    /// Total size in bytes: header, code, exception table, and debug
    /// records, plus any zero padding that follows the code.
    pub size: u32,
    /// Branches that do not land on the start of an instruction.
    pub stray_branches: Vec<StrayBranch>,
}

/// A branch that does not land on the start of an instruction.
///
/// The reference compiler produces a few of these: the `switch` it
/// generates for some expressions has a default branch that points into
/// the middle of an instruction. The default is never taken, so the
/// program runs correctly, and a disassembler has to tolerate it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StrayBranch {
    /// Offset of the branching instruction within the function, or of the
    /// exception table for a stray handler.
    pub offset: u32,
    /// The target offset.
    pub target: i64,
}

/// How the end of a function's code was established.
///
/// A method header gives no code length. The code ends where the exception
/// table or debug records begin, if the function has either. Otherwise the
/// only evidence is the code itself and where the next function starts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Extent {
    /// The header locates a table that follows the code.
    Table,
    /// The code runs exactly to the start of the next known function, or to
    /// the end of its page.
    Limit,
    /// The code was followed along its branches; nothing more is known
    /// about what lies beyond the last reachable instruction.
    Reachable,
    /// Reachable code ends before the next known function, and the bytes
    /// between are not a whole number of instructions. They may be an
    /// undiscovered function.
    Gap {
        /// Number of bytes between the reachable code and the limit.
        len: u32,
        /// Offset within the function where the bytes after the reachable
        /// code stop decoding as instructions. If unreachable code follows
        /// the function, the next function starts here.
        decodable_end: u32,
    },
}

impl Function {
    /// Code pool address just past the function's code.
    pub fn end_address(&self) -> u32 {
        self.address + self.code_end
    }

    /// Code pool address just past everything that belongs to the function:
    /// its code, exception table, and debug records.
    pub fn tail_address(&self) -> u32 {
        self.address + self.size
    }

    /// Code pool addresses of the functions this function refers to.
    pub fn function_references(&self) -> impl Iterator<Item = u32> + '_ {
        self.instructions
            .iter()
            .flat_map(|i| &i.operands)
            .filter_map(|operand| match operand {
                Operand::Function(address) => Some(*address),
                _ => None,
            })
    }
}

/// Why a function could not be disassembled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisassembleError {
    /// The address is not in the image's code pool, or too close to the end
    /// of its page to hold a method header.
    BadAddress,
    /// The image's method header size is smaller than this crate can read.
    HeaderTooSmall,
    /// The header places the exception table or debug records before the
    /// first instruction or beyond the page.
    BadTableOffset,
    /// The exception table runs past the end of the page.
    BadExceptionTable,
    /// The debug records are malformed or run past the end of the page.
    BadDebugRecords,
    /// An instruction could not be decoded.
    Decode(DecodeError),
    /// An instruction runs past the end of the code.
    Overrun {
        /// Offset of the instruction within the function.
        offset: u32,
    },
    /// A branch leaves the function's code, so the code cannot be followed.
    BadBranch {
        /// Offset of the branching instruction within the function.
        offset: u32,
        /// The target offset.
        target: i64,
    },
}

impl fmt::Display for DisassembleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BadAddress => write!(f, "address is not a function in the code pool"),
            Self::HeaderTooSmall => write!(f, "method header size is too small"),
            Self::BadTableOffset => write!(f, "method header has an impossible table offset"),
            Self::BadExceptionTable => write!(f, "exception table runs past the end of its page"),
            Self::BadDebugRecords => write!(f, "debug records are malformed"),
            Self::Decode(error) => write!(f, "{error}"),
            Self::Overrun { offset } => {
                write!(
                    f,
                    "instruction at offset {offset:#x} runs past the end of the code"
                )
            }
            Self::BadBranch { offset, target } => {
                write!(
                    f,
                    "branch at offset {offset:#x} targets {target:#x}, outside the function"
                )
            }
        }
    }
}

impl std::error::Error for DisassembleError {}

impl From<DecodeError> for DisassembleError {
    fn from(error: DecodeError) -> Self {
        Self::Decode(error)
    }
}

/// Whether control can continue to the instruction after this one.
fn falls_through(opcode: Opcode) -> bool {
    !matches!(
        opcode,
        Opcode::RetVal
            | Opcode::RetNil
            | Opcode::RetTrue
            | Opcode::Ret
            | Opcode::Jmp
            | Opcode::Throw
            | Opcode::Switch
            | Opcode::LRet
    )
}

/// Disassemble the function at a code pool address.
///
/// `limit` is the code pool address of the next function, if known. It is
/// used only for functions whose headers do not locate the end of the code.
pub fn disassemble(
    image: &Image,
    address: u32,
    limit: Option<u32>,
) -> Result<Function, DisassembleError> {
    let header_size = image.entry_point.method_header_size as usize;
    if header_size < MethodHeader::MIN_SIZE {
        return Err(DisassembleError::HeaderTooSmall);
    }
    // A function lies within one page, so the page bounds the code.
    let page = image
        .code_pool
        .at(address)
        .ok_or(DisassembleError::BadAddress)?;
    let header = MethodHeader::parse(page).ok_or(DisassembleError::BadAddress)?;
    if page.len() < header_size {
        return Err(DisassembleError::BadAddress);
    }
    let first = header_size as u32;

    let table_offset = match (header.exception_table_offset, header.debug_records_offset) {
        (0, 0) => None,
        (0, debug) => Some(debug),
        (exceptions, _) => Some(exceptions),
    };
    if let Some(offset) = table_offset {
        if (offset as usize) < header_size || offset as usize > page.len() {
            return Err(DisassembleError::BadTableOffset);
        }
    }

    let handlers = exception_table(image, page, &header)?;

    let mut padding = 0u32;
    let (instructions, code_end, extent) = match table_offset {
        Some(end) => {
            let code = &page[..end as usize];
            let instructions =
                linear(code, first)?.ok_or(DisassembleError::Overrun { offset: end.into() })?;
            (instructions, u32::from(end), Extent::Table)
        }
        None => {
            // No handlers exist without an exception table, so the first
            // instruction is the only entry.
            let mut found = reachable(page, &[first])?;
            let mut end = found
                .values()
                .map(|i| i.offset + i.len)
                .max()
                .unwrap_or(first);
            let bound = match limit {
                Some(limit) if limit > address && ((limit - address) as usize) <= page.len() => {
                    limit - address
                }
                _ => page.len() as u32,
            };
            let mut extent = Extent::Reachable;
            if bound >= end && (limit.is_some() || bound == end) {
                // Code after the last reachable instruction is unreachable
                // but still part of the function, if it decodes cleanly up
                // to the bound.
                match linear(&page[..bound as usize], end) {
                    Ok(Some(dead)) => {
                        for instruction in dead {
                            found.insert(instruction.offset, instruction);
                        }
                        end = bound;
                        extent = Extent::Limit;
                    }
                    _ => {
                        // Decode what there is; if only zero bytes remain,
                        // they are padding, not code.
                        let (dead, stop) = linear_prefix(&page[..bound as usize], end);
                        if page[stop as usize..bound as usize].iter().all(|&b| b == 0) {
                            for instruction in dead {
                                found.insert(instruction.offset, instruction);
                            }
                            end = stop;
                            padding = bound - stop;
                            extent = Extent::Limit;
                        } else {
                            extent = Extent::Gap {
                                len: bound - end,
                                decodable_end: stop,
                            };
                        }
                    }
                }
            }
            (found.into_values().collect(), end, extent)
        }
    };

    let mut size = code_end + padding;
    if header.exception_table_offset != 0 {
        let table = 2 + handlers.len() as u32 * u32::from(image.entry_point.exception_entry_size);
        size = u32::from(header.exception_table_offset) + table;
    }
    if header.debug_records_offset != 0 {
        size = debug_records_end(image, page, &header).ok_or(DisassembleError::BadDebugRecords)?;
    }

    let mut function = Function {
        address,
        header,
        instructions,
        handlers,
        code_end,
        extent,
        size,
        stray_branches: Vec::new(),
    };
    function.stray_branches = stray_branches(&function);
    Ok(function)
}

/// Decode instructions one after another from `start` to the end of `code`.
/// Returns `None` if the last instruction does not end exactly there.
fn linear(code: &[u8], start: u32) -> Result<Option<Vec<Instruction>>, DecodeError> {
    let mut instructions = Vec::new();
    let mut offset = start;
    while (offset as usize) < code.len() {
        let instruction = decode(code, offset)?;
        offset += instruction.len;
        instructions.push(instruction);
    }
    Ok((offset as usize == code.len()).then_some(instructions))
}

/// Decode instructions one after another from `start` for as long as they
/// decode and fit, returning them and the offset where decoding stopped.
fn linear_prefix(code: &[u8], start: u32) -> (Vec<Instruction>, u32) {
    let mut instructions = Vec::new();
    let mut offset = start;
    while (offset as usize) < code.len() {
        let Ok(instruction) = decode(code, offset) else {
            break;
        };
        if (offset + instruction.len) as usize > code.len() {
            break;
        }
        offset += instruction.len;
        instructions.push(instruction);
    }
    (instructions, offset)
}

/// Decode every instruction that can be reached from the entry offsets.
fn reachable(code: &[u8], entries: &[u32]) -> Result<BTreeMap<u32, Instruction>, DisassembleError> {
    let mut found: BTreeMap<u32, Instruction> = BTreeMap::new();
    let mut pending: Vec<u32> = entries.to_vec();
    // Switch defaults are followed last, and only if they do not point into
    // an instruction already decoded; see `StrayBranch`.
    let mut defaults: Vec<u32> = Vec::new();
    loop {
        while let Some(offset) = pending.pop() {
            if found.contains_key(&offset) {
                continue;
            }
            let instruction = decode(code, offset)?;
            let in_code = |target: i64| {
                u32::try_from(target)
                    .ok()
                    .filter(|t| (*t as usize) < code.len())
                    .ok_or(DisassembleError::BadBranch { offset, target })
            };
            for operand in &instruction.operands {
                match *operand {
                    Operand::Branch(target) => pending.push(in_code(target)?),
                    // A named argument table is data in the code stream,
                    // laid out as an instruction so it can be skipped.
                    Operand::Table(table) => pending.push(table),
                    _ => {}
                }
            }
            if let Payload::Switch { cases, default } = &instruction.payload {
                for (_, target) in cases {
                    pending.push(in_code(*target)?);
                }
                if let Ok(default) = in_code(*default) {
                    defaults.push(default);
                }
            }
            if falls_through(instruction.opcode) {
                pending.push(offset + instruction.len);
            }
            found.insert(offset, instruction);
        }
        let Some(default) = defaults.pop() else {
            break;
        };
        let inside_another = found
            .range(..default)
            .next_back()
            .is_some_and(|(start, i)| start + i.len > default);
        if !inside_another {
            pending.push(default);
        }
    }
    Ok(found)
}

fn exception_table(
    image: &Image,
    page: &[u8],
    header: &MethodHeader,
) -> Result<Vec<ExceptionHandler>, DisassembleError> {
    if header.exception_table_offset == 0 {
        return Ok(Vec::new());
    }
    let entry_size = image.entry_point.exception_entry_size as usize;
    let table = page
        .get(header.exception_table_offset as usize..)
        .ok_or(DisassembleError::BadExceptionTable)?;
    let count = table
        .get(..2)
        .map(|b| u16::from_le_bytes([b[0], b[1]]) as usize)
        .ok_or(DisassembleError::BadExceptionTable)?;
    if entry_size < 10 {
        return Err(DisassembleError::BadExceptionTable);
    }
    let entries = table
        .get(2..2 + count * entry_size)
        .ok_or(DisassembleError::BadExceptionTable)?;
    Ok(entries
        .chunks_exact(entry_size)
        .map(|e| ExceptionHandler {
            start: u16::from_le_bytes([e[0], e[1]]),
            end: u16::from_le_bytes([e[2], e[3]]),
            exception_class: u32::from_le_bytes([e[4], e[5], e[6], e[7]]),
            handler: u16::from_le_bytes([e[8], e[9]]),
        })
        .collect())
}

/// Find the offset just past a function's debug records.
///
/// The records are a header, a counted array of line records, a frame table
/// that states its own length, and a reserved 32-bit zero. Release builds
/// have them too, for functions with local variables: the frame table is
/// what lets a running program look up its locals by name.
fn debug_records_end(image: &Image, page: &[u8], header: &MethodHeader) -> Option<u32> {
    let entry = &image.entry_point;
    let u16_at =
        |at: usize| Some(u16::from_le_bytes([*page.get(at)?, *page.get(at + 1)?]) as usize);
    let mut pos = header.debug_records_offset as usize + entry.debug_table_header_size as usize;
    let lines = u16_at(pos)?;
    pos += 2 + lines * entry.debug_line_entry_size as usize;
    // The frame table starts with the offset, from that field, to the
    // first byte after the table.
    pos += u16_at(pos)?;
    pos += 4;
    (pos <= page.len()).then_some(pos as u32)
}

/// Find the branches and handlers that do not land on the start of an
/// instruction.
fn stray_branches(function: &Function) -> Vec<StrayBranch> {
    let starts: std::collections::BTreeSet<i64> = function
        .instructions
        .iter()
        .map(|i| i64::from(i.offset))
        .collect();
    let mut stray = Vec::new();
    for instruction in &function.instructions {
        for target in instruction.branch_targets() {
            if !starts.contains(&target) {
                stray.push(StrayBranch {
                    offset: instruction.offset,
                    target,
                });
            }
        }
    }
    for handler in &function.handlers {
        if !starts.contains(&i64::from(handler.handler)) {
            stray.push(StrayBranch {
                offset: u32::from(function.header.exception_table_offset),
                target: handler.handler.into(),
            });
        }
    }
    stray
}
