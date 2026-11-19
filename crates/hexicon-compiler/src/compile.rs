//! Compiling a program: source files in, an image out.

use crate::emit;
use crate::image;
use crate::parse::Parser;
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
    /// in. Without one, they are taken to be ASCII.
    pub default_charset: Option<String>,
    /// When the program is compiled, as the 24 characters the image
    /// records (`Thu Nov 19 09:24:12 2026`). The compiler has no clock
    /// of its own.
    pub timestamp: [u8; 24],
}

impl Default for CompileOptions {
    fn default() -> Self {
        Self {
            include_paths: Vec::new(),
            defines: Vec::new(),
            undefines: Vec::new(),
            default_charset: None,
            timestamp: *b"Thu Jan  1 00:00:00 1970",
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
    diagnostics
        .iter()
        .any(|diagnostic| diagnostic.severity == Severity::Error)
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
            file: source.to_owned(),
            line: 0,
            severity: Severity::Error,
            message: format!("unable to open source file \"{source}\""),
        });
    }
    pp.set_compile_mode();
    Ok(pp)
}

/// Compile a program that is a single source file.
///
/// The source is read twice, as the reference compiler reads it: once
/// to learn what the program defines, and once more, knowing that, to
/// generate the code.
pub fn compile(files: &mut dyn SourceFiles, source: &str, options: &CompileOptions) -> Compiled {
    let mut compiled = Compiled::default();

    let known_funcs = {
        let mut pp = match start(files, source, options) {
            Ok(pp) => pp,
            Err(diagnostic) => {
                compiled.diagnostics.push(diagnostic);
                return compiled;
            }
        };
        let prog = Parser::new(&mut pp, true, Vec::new()).parse_top();
        compiled.diagnostics = pp.take_diagnostics();
        if has_errors(&compiled.diagnostics) {
            return compiled;
        }
        prog.exported_funcs()
    };

    let mut pp = match start(files, source, options) {
        Ok(pp) => pp,
        Err(diagnostic) => {
            compiled.diagnostics.push(diagnostic);
            return compiled;
        }
    };
    let mut prog = Parser::new(&mut pp, false, known_funcs).parse_top();
    compiled.diagnostics.append(&mut pp.take_diagnostics());
    if has_errors(&compiled.diagnostics) {
        return compiled;
    }

    let mut generated = emit::generate(&mut prog);
    for diagnostic in std::mem::take(&mut generated.diagnostics) {
        compiled.diagnostics.push(Diagnostic {
            file: pp.file_name(diagnostic.file).to_owned(),
            line: diagnostic.line,
            severity: diagnostic.severity,
            message: diagnostic.message,
        });
    }
    if has_errors(&compiled.diagnostics) {
        return compiled;
    }

    match image::write_image(&prog, generated, &options.timestamp, &TOOL_DATA) {
        Ok(image) => {
            compiled.image = Some(image.bytes);
            compiled.unspecified = image.unspecified;
        }
        Err(message) => compiled.diagnostics.push(Diagnostic {
            file: String::new(),
            line: 0,
            severity: Severity::Error,
            message,
        }),
    }
    compiled
}
