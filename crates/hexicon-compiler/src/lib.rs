//! The Hexicon compiler.
//!
//! So far this is the preprocessor: source files in, preprocessed text
//! out. The compiler has no access to files or to a console of its own;
//! whoever uses it supplies the files through [`pp::SourceFiles`] and
//! decides what to do with what comes out.

mod lex;
pub mod pp;
mod ppexpr;

pub use lex::Tok;

#[cfg(test)]
mod tests;
