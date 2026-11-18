//! The Hexicon virtual machine.
//!
//! This crate runs T3 program images: the `.t3` files the TADS 3 compiler
//! produces. It follows the T3 VM specification and, where the
//! specification is silent, the behavior of the reference interpreter.
//!
//! The VM does no input or output of its own. Everything it needs from
//! the outside world comes through a [`hexicon_host::Host`].

mod bif;
mod call;
mod console;
mod entities;
mod error;
mod interp;
mod load;
mod meta;
mod methods;
mod object;
mod ops;
mod props;
mod tads;
mod trace;
mod value;
mod vm;

pub use error::{ErrParam, RuntimeError, VmError, VmResult, code};
pub use value::{ObjId, PropId, Value};
pub use vm::Vm;

#[cfg(test)]
mod tests;
