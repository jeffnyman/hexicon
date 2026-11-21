//! Compiling a program: source files in, an image out.

use crate::emit;
use crate::image;
use crate::link::{self, Linked, Module};
use crate::parse::{Exports, Global, Parser, merge_exports};
use crate::pp::{Diagnostic, Preprocessor, Severity, SourceFiles};

/// What to compile, and how.
#[derive(Debug, Clone)]
pub struct CompileOptions {
    /// Directories to look in for included files.
    pub include_paths: Vec<String>,
    /// Macros to define before the source is read: name and expansion.
    pub defines: Vec<(String, String)>,
    /// Macros to remove before the source is read.
    pub undefines: Vec<String>,
    /// The character set of source files that do not say what they are
    /// in. Without one, they are taken to be UTF-8.
    pub default_charset: Option<String>,
    /// When the program is compiled, as the 24 characters the image
    /// records (`Thu Nov 19 09:24:12 2026`). The compiler has no clock
    /// of its own.
    pub timestamp: [u8; 24],
    /// Objects say which source file they are in from the start of each
    /// file (`-Gstg`), as `#pragma sourceTextGroup(on)` makes them.
    pub source_text_group: bool,
    /// For each source, the name its source text group gives it and
    /// its place among the modules of the build, from 1. Without them,
    /// a source is named as it is given, and numbered by its place in
    /// the list.
    pub module_names: Vec<(String, usize)>,
}

impl Default for CompileOptions {
    fn default() -> Self {
        Self {
            include_paths: Vec::new(),
            defines: Vec::new(),
            undefines: Vec::new(),
            default_charset: None,
            timestamp: *b"Thu Jan  1 00:00:00 1970",
            source_text_group: false,
            module_names: Vec::new(),
        }
    }
}

/// The name of the system programs are compiled on, as `__TADS_SYSTEM_NAME`
/// gives it. Hexicon is the same system wherever it runs.
const SYSTEM_NAME: &str = "HEXICON";

/// What came of compiling.
#[derive(Debug, Clone, Default)]
pub struct Compiled {
    /// The image, unless there were errors.
    pub image: Option<Vec<u8>>,
    /// Stretches of the image, as offset and length, whose contents mean
    /// nothing to a program: the time it was compiled, bytes that are
    /// the compiler's own, and fields that are written but never read.
    /// Two images of the same program can differ there.
    pub unspecified: Vec<(usize, usize)>,
    /// The problems found.
    pub diagnostics: Vec<Diagnostic>,
}

/// Four bytes in the image's header that are the compiler's to use.
const TOOL_DATA: [u8; 4] = *b"hxcn";

fn has_errors(diagnostics: &[Diagnostic]) -> bool {
    diagnostics.iter().any(|diagnostic| {
        matches!(
            diagnostic.severity,
            Severity::Error | Severity::Fatal | Severity::Internal
        )
    })
}

fn start<'a>(
    files: &'a mut dyn SourceFiles,
    source: &str,
    options: &CompileOptions,
) -> Result<Preprocessor<'a>, Diagnostic> {
    let mut pp = Preprocessor::new(files);
    if let Some(charset) = &options.default_charset {
        pp.set_default_charset(charset);
    }

    // The macros the compiler defines before any source is read: what
    // is compiling, and when. The date is `Nov 19 2026` and the time
    // `09:24:12`, as strings.
    let stamp = String::from_utf8_lossy(&options.timestamp);
    let part = |range: std::ops::Range<usize>| stamp.get(range).unwrap_or("");
    pp.define(
        "__DATE__",
        &format!("'{} {} {}'", part(4..7), part(8..10), part(20..24)),
    );
    pp.define("__TIME__", &format!("'{}'", part(11..19)));
    pp.define(&format!("__TADS_SYS_{SYSTEM_NAME}"), "1");
    pp.define("__TADS_SYSTEM_NAME", &format!("'{SYSTEM_NAME}'"));
    pp.define("__TADS_VERSION_MAJOR", "3");
    pp.define("__TADS_VERSION_MINOR", "1");
    pp.define("__TADS_MACRO_FORMAT_VERSION", "1");
    pp.define("__TADS3", "1");

    for path in &options.include_paths {
        pp.add_include_path(path);
    }
    for (name, expansion) in &options.defines {
        pp.define(name, expansion);
    }
    for name in &options.undefines {
        pp.undefine(name);
    }
    if !pp.set_source(source) {
        return Err(Diagnostic {
            // A message about no place in particular.
            file: String::new(),
            line: 0,
            severity: Severity::Error,
            message: format!("unable to open source file \"{source}\""),
        });
    }
    pp.set_compile_mode();
    Ok(pp)
}

/// Compile a program that is a single source file.
pub fn compile(files: &mut dyn SourceFiles, source: &str, options: &CompileOptions) -> Compiled {
    compile_program(files, &[source], options)
}

/// Compile a program from its source files, which are put together in
/// the order given.
///
/// Each source is read twice, as the reference compiler reads it: once
/// to learn what it defines, and once more, knowing what all of them
/// define, to generate its code. Then what was generated for each is
/// made into one program.
pub fn compile_program(
    files: &mut dyn SourceFiles,
    sources: &[&str],
    options: &CompileOptions,
) -> Compiled {
    let mut compiled = Compiled::default();

    let mut exports = Vec::new();
    for source in sources {
        let mut pp = match start(files, source, options) {
            Ok(pp) => pp,
            Err(diagnostic) => {
                compiled.diagnostics.push(diagnostic);
                return compiled;
            }
        };
        let prog = Parser::new(&mut pp, true, Exports::default()).parse_top();
        compiled.diagnostics.append(&mut pp.take_diagnostics());
        if has_errors(&compiled.diagnostics) {
            return compiled;
        }
        exports.push(prog.exports());
    }
    let known = merge_exports(exports);

    let mut modules = Vec::new();
    for (index, source) in sources.iter().enumerate() {
        let module = options
            .module_names
            .get(index)
            .cloned()
            .unwrap_or_else(|| ((*source).to_owned(), index + 1));
        let mut pp = match start(files, source, options) {
            Ok(pp) => pp,
            Err(diagnostic) => {
                compiled.diagnostics.push(diagnostic);
                return compiled;
            }
        };
        let mut parser = Parser::new(&mut pp, false, known.clone());
        parser.set_module(module, options.source_text_group);
        let mut prog = parser.parse_top();
        compiled.diagnostics.append(&mut pp.take_diagnostics());
        if has_errors(&compiled.diagnostics) {
            return compiled;
        }

        let mut generated = emit::generate(&mut prog);
        for diagnostic in std::mem::take(&mut generated.diagnostics) {
            // A problem that is nowhere in particular has no file. Nor
            // has one found before any statement was come to.
            let file = if diagnostic.file == usize::MAX || diagnostic.line == 0 {
                String::new()
            } else {
                pp.file_name(diagnostic.file).to_owned()
            };
            compiled.diagnostics.push(Diagnostic {
                file,
                line: diagnostic.line,
                severity: diagnostic.severity,
                message: diagnostic.message,
            });
        }
        if has_errors(&compiled.diagnostics) {
            return compiled;
        }
        // The reference writes an object file for each source, and
        // names it when it finds something wrong on reading it back.
        // Hexicon writes no such file; it gives the name the file
        // would have.
        let object_file = {
            let name = source.rsplit(['/', '\\']).next().unwrap_or(source);
            let stem = name.rsplit_once('.').map_or(name, |(stem, _)| stem);
            format!("{stem}.t3o")
        };
        modules.push(Module {
            prog,
            out: generated,
            object_file,
        });
    }

    // Putting the program together: everything it refers to has to be
    // in it.
    let note = |compiled: &mut Compiled, severity: Severity, message: String| {
        compiled.diagnostics.push(Diagnostic {
            file: String::new(),
            line: 0,
            severity,
            message,
        });
    };
    let Linked {
        mut prog,
        out,
        enum_ids,
        errors,
    } = link::link(modules);
    // The reference reads every object file even when one is wrong,
    // and goes no further if any was.
    if !errors.is_empty() {
        for message in errors {
            note(&mut compiled, Severity::Error, message);
        }
        return compiled;
    }
    let mut generated = emit::link_generate(&mut prog, out, enum_ids);
    for name in emit::unresolved_externs(&prog, &generated) {
        let message = format!(
            "unresolved external reference \"{}\"",
            crate::pp::show_name(&name)
        );
        note(&mut compiled, Severity::Error, message);
    }
    // A program has to have somewhere to start.
    let has_main = matches!(
        prog.globals.peek(b"_main".as_slice()),
        Some(Global::Func(func)) if generated.func_anchor.contains_key(func)
    );
    if !has_main && prog.globals.peek(b"_main".as_slice()).is_none() {
        note(
            &mut compiled,
            Severity::Error,
            "function _main() is not defined".to_owned(),
        );
        return compiled;
    }
    for (severity, message) in emit::resolve_exports(&prog, &mut generated) {
        note(&mut compiled, severity, message);
    }
    if has_errors(&compiled.diagnostics) {
        return compiled;
    }
    // What is found wrong as the image is put together, which the
    // reference does not get to if anything above was.
    for message in std::mem::take(&mut generated.link_errors) {
        note(&mut compiled, Severity::Error, message);
    }
    if has_errors(&compiled.diagnostics) {
        return compiled;
    }

    match image::write_image(&prog, generated, &options.timestamp, &TOOL_DATA) {
        Ok(image) => {
            compiled.image = Some(image.bytes);
            compiled.unspecified = image.unspecified;
        }
        Err(message) => note(&mut compiled, Severity::Error, message),
    }
    compiled
}
