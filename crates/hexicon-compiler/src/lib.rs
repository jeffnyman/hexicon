//! The Hexicon compiler.
//!
//! [`compile`] turns a source file into a program image. So far it
//! compiles programs made of functions; objects and the rest of the
//! language are to come. The preprocessor ([`pp`]) can also be used on
//! its own.
//!
//! The compiler has no access to files, to a clock, or to a console of
//! its own; whoever uses it supplies the files through
//! [`pp::SourceFiles`] and decides what to do with what comes out.

mod ast;
mod codegen;
mod compile;
mod emit;
mod flow;
mod fold;
mod image;
mod lex;
mod parse;
pub mod pp;
mod ppexpr;

pub use compile::{CompileOptions, Compiled, compile};
pub use lex::Tok;

#[cfg(test)]
mod tests;
