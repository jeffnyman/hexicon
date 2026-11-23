//! The Hexicon compiler.
//!
//! [`compile_program`] turns the source files of a program into a
//! program image, and [`compile`] does the same for a program that is
//! one file. The preprocessor ([`pp`]) can also be used on its own.
//! [`read_project`] reads what to build from a command line in the
//! form the reference's `t3make` takes, with its project files and
//! libraries.
//!
//! The compiler has no access to files, to a clock, or to a console of
//! its own; whoever uses it supplies the files through
//! [`pp::SourceFiles`] and decides what to do with what comes out.

mod ast;
mod codegen;
mod compile;
pub mod dynamic;
mod emit;
mod flow;
mod fold;
mod image;
mod lex;
mod link;
mod parse;
pub mod pp;
mod project;

pub use compile::{
    CompileOptions, Compiled, RuntimeSymbol, RuntimeValue, compile, compile_program,
};
pub use lex::Tok;
pub use project::{
    DirEntry, Project, ProjectError, ProjectPaths, Resource, ResourceSpec, expand_resources,
    read_project,
};

#[cfg(test)]
mod tests;
