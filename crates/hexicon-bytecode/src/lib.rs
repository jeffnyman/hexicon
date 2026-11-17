//! T3 VM byte code: the instruction set, a decoder, and a disassembler.
//!
//! - [`Opcode`] defines every instruction and its operand layout.
//! - [`decode`] reads one instruction.
//! - [`disassemble`] reads a whole function from an image.
//! - [`discover`] finds the functions in an image.
//! - [`listing`] renders functions as text.

mod decode;
mod discover;
mod function;
pub mod listing;
mod opcode;

pub use decode::{DecodeError, Instruction, Operand, Payload, decode};
pub use discover::{Functions, discover, discover_without_debug_info};
pub use function::{
    DisassembleError, ExceptionHandler, Extent, Function, MethodHeader, StrayBranch, disassemble,
};
pub use opcode::{Layout, Opcode, OperandKind};

#[cfg(test)]
mod tests;
