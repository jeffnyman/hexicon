//! The Hexicon compiler.
//!
//! [`compile_program`] turns the source files of a program into a
//! program image, and [`compile`] does the same for a program that is
//! one file. The preprocessor ([`pp`]) can also be used on its own.
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
mod link;
mod parse;
pub mod pp;
mod ppexpr;

pub use compile::{CompileOptions, Compiled, compile, compile_program};
pub use lex::Tok;

#[cfg(test)]
mod tests;
