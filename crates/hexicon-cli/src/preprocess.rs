//! `hexicon preprocess` and `hexicon tokens`: what the first stages of
//! the compiler make of a source file.
//!
//! The output of each is in a form the reference compiler can be made to
//! write too, so that the two can be compared. Preprocessed text is as
//! the reference's preprocess-only mode writes it: a `#line` directive
//! wherever the text stops following on from the line before, and
//! otherwise one line of output for each line of source. Tokens are
//! listed one to a line, as `tools/reference/tok_dump.cpp` lists them.

use std::io::Write as _;
use std::process::ExitCode;

use hexicon_compiler::Tok;
use hexicon_compiler::pp::{Preprocessor, Severity, SourceFiles};

/// Source files on disk.
struct DiskFiles;

impl SourceFiles for DiskFiles {
    fn read(&mut self, name: &str) -> Option<Vec<u8>> {
        std::fs::read(name).ok()
    }

    fn exists(&mut self, name: &str) -> bool {
        std::path::Path::new(name).is_file()
    }
}

/// A file name as a string in double quotes.
fn quoted(name: &str) -> String {
    let mut out = String::from('"');
    for c in name.chars() {
        if c == '\\' || c == '"' {
            out.push('\\');
        }
        out.push(c);
    }
    out.push('"');
    out
}

/// What to write.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Output {
    /// The preprocessed text.
    Text,
    /// The tokens.
    Tokens,
}

/// Write the problems found since the last time, in with the output,
/// where they were found, as the reference's test programs do. Returns
/// whether any was an error.
fn report(pp: &mut Preprocessor<'_>, out: &mut impl std::io::Write) -> bool {
    let mut failed = false;
    for problem in pp.take_diagnostics() {
        let kind = match problem.severity {
            Severity::Error => {
                failed = true;
                "error"
            }
            Severity::Warning => "warning",
            Severity::Message => {
                let _ = writeln!(out, "{}", problem.message);
                continue;
            }
        };
        let _ = writeln!(
            out,
            "{}({}): {kind}: {}",
            problem.file, problem.line, problem.message
        );
    }
    failed
}

pub(crate) fn run(args: &[&str], usage: &str, output: Output) -> ExitCode {
    let mut files = DiskFiles;
    let mut pp = Preprocessor::new(&mut files);
    let mut rest = args;
    let source = loop {
        match rest {
            ["-I", dir, tail @ ..] => {
                pp.add_include_path(dir);
                rest = tail;
            }
            ["-D", definition, tail @ ..] => {
                match definition.split_once('=') {
                    Some((name, text)) => pp.define(name, text),
                    None => pp.define(definition, "1"),
                }
                rest = tail;
            }
            ["-U", name, tail @ ..] => {
                pp.undefine(name);
                rest = tail;
            }
            ["--test", tail @ ..] => {
                pp.set_test_report_mode(true);
                rest = tail;
            }
            [source] if !source.starts_with('-') => break *source,
            _ => {
                eprint!("{usage}");
                return ExitCode::from(2);
            }
        }
    };
    if !pp.set_source(source) {
        eprintln!("hexicon: {source}: unable to open source file");
        return ExitCode::FAILURE;
    }

    let mut out = std::io::BufWriter::new(std::io::stdout().lock());
    let mut failed = false;
    match output {
        Output::Text => {
            let mut last: Option<(usize, u32)> = None;
            loop {
                let line = pp.next_line();
                failed |= report(&mut pp, &mut out);
                let Some(line) = line else {
                    break;
                };
                let follows =
                    last.is_some_and(|(file, number)| file == line.file && number + 1 == line.line);
                if !follows {
                    let _ = writeln!(
                        out,
                        "#line {} {}",
                        line.line,
                        quoted(pp.file_name(line.file))
                    );
                }
                last = Some((line.file, line.line));
                let _ = out.write_all(&line.text);
                let _ = out.write_all(b"\n");
            }
        }
        Output::Tokens => {
            pp.set_compile_mode();
            loop {
                let typ = pp.next_token();
                failed |= report(&mut pp, &mut out);
                if typ == Tok::Eof {
                    break;
                }
                // The kind's number, a tab, and the text, with anything
                // that is not printable ASCII given as its code.
                let _ = write!(out, "{}\t", typ as u32);
                for &c in &pp.cur_token().text {
                    let _ = match c {
                        b'\\' => out.write_all(b"\\\\"),
                        32..=126 => out.write_all(&[c]),
                        _ => write!(out, "\\x{c:02x}"),
                    };
                }
                let _ = out.write_all(b"\n");
            }
        }
    }
    let _ = out.flush();
    if failed {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}
