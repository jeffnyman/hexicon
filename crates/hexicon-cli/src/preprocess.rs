//! `hexicon preprocess`: write a source file's preprocessed text.
//!
//! The output is in the form the reference compiler's preprocess-only
//! mode writes, so that the two can be compared: a `#line` directive
//! wherever the text stops following on from the line before, and
//! otherwise one line of output for each line of source.

use std::io::Write as _;
use std::process::ExitCode;

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

pub(crate) fn run(args: &[&str], usage: &str) -> ExitCode {
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

    // Problems are written in with the text, where they were found, as
    // the reference's test program writes them.
    let mut out = std::io::stdout().lock();
    let mut failed = false;
    let mut last: Option<(usize, u32)> = None;
    let mut report = |pp: &mut Preprocessor<'_>, out: &mut std::io::StdoutLock<'_>| {
        for problem in pp.take_diagnostics() {
            let kind = match problem.severity {
                Severity::Error => {
                    failed = true;
                    "error"
                }
                Severity::Warning => "warning",
            };
            let _ = writeln!(
                out,
                "{}({}): {kind}: {}",
                problem.file, problem.line, problem.message
            );
        }
    };
    loop {
        let line = pp.next_line();
        report(&mut pp, &mut out);
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
    if failed {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}
