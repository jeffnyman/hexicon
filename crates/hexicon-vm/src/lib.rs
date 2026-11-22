//! The Hexicon virtual machine.
//!
//! This crate runs T3 program images: the `.t3` files the TADS 3 compiler
//! produces. It follows the T3 VM specification and, where the
//! specification is silent, the behavior of the reference interpreter.
//!
//! The VM does no input or output of its own. Everything it needs from
//! the outside world comes through a [`hexicon_host::Host`].

mod bif;
pub(crate) use hexicon_bignum as bignum;
mod bignum_math;
mod bignum_obj;
mod bytearray;
mod call;
mod casefold;
mod charset;
mod chartype;
mod console;
mod date;
mod dict;
mod dynfunc;
mod entities;
mod error;
mod file;
mod gc;
mod gram;
mod hash;
mod heap;
pub mod identity;
mod interp;
mod io;
mod list;
mod load;
mod lookup;
mod meta;
mod methods;
mod object;
mod ops;
mod pack;
mod props;
mod rebuild;
mod reflect;
mod regex;
mod rng;
mod save;
mod specials;
mod sprintf;
mod strbuf;
mod strcomp;
mod string;
mod tads;
mod trace;
mod tz;
mod value;
mod vector;
mod vm;

pub use charset::decode_text;
pub use error::{ErrParam, RuntimeError, VmError, VmResult, code};
pub use rebuild::RebuiltImage;
pub use value::{ObjId, PropId, Value};
pub use vm::{RunState, Vm};

#[cfg(test)]
mod tests;
