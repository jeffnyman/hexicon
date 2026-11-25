//! Compiling a program: source files in, an image out.

use crate::emit;
use crate::image;
use crate::link::{self, Linked, Module};
use crate::parse::{Exports, Global, Parser, merge_exports};
use crate::pp::{Diagnostic, LinkMacro, Preprocessor, Severity, SourceFiles};

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
    /// For each source, whether a library brought it in. A game's own
    /// Hexicon sources are checked for properties given values that
    /// nothing reads (see [`crate::names`]); a library's are not.
    /// Without them, no source is a library's.
    pub from_library: Vec<bool>,
    /// The libraries a Hexicon source can import (`import hexad;`): each
    /// by name, with its header, or none if it names none.
    pub imports: Vec<(String, Option<String>)>,
    /// Compile for debugging (`-d`): `__DEBUG` is defined, and the image
    /// has what a debugger needs: the global symbols, the macros, and
    /// the address of each line's code.
    pub debug: bool,
    /// The image says where in the source each statement's code came
    /// from, so that a run-time error can say where it happened. On by
    /// default, and always when compiling for debugging; off only where
    /// the code alone matters, as in tests that two sources compile to
    /// the same image.
    pub line_records: bool,
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
            from_library: Vec::new(),
            imports: Vec::new(),
            debug: false,
            line_records: true,
        }
    }
}

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
    /// The program's global symbols, each with its value in the image,
    /// in the order of the reference's table of them when the program
    /// is put together: what `globalSymbols()` gives a program run before
    /// its image is written.
    pub symbols: Vec<RuntimeSymbol>,
    /// The program's macros, likewise: what
    /// `globalSymbols(MacroTable)` gives it.
    pub macros: Vec<LinkMacro>,
}

/// A global symbol, and its value in the image.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeSymbol {
    /// The name.
    pub name: Vec<u8>,
    /// What the name means.
    pub value: RuntimeValue,
}

/// What a global symbol means.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeValue {
    /// A function: its address in the code.
    Function(u32),
    /// An object.
    Object(u32),
    /// A property.
    Property(u16),
    /// A value of an enumeration.
    Enum(u32),
    /// A built-in function: its set, and its place in the set.
    BuiltIn {
        /// The set's number.
        set: u16,
        /// The function's place in it.
        index: u16,
    },
    /// A built-in class: the object that stands for it.
    Class(u32),
}

/// The macros the compiler defines before any source is read: what is
/// compiling, and when. The date is `Nov 19 2026` and the time
/// `09:24:12`, as strings. A source is told it is Hexicon's
/// (`__HEXICON`, and Hexicon's version). Then those the options define.
fn predefined_macros(options: &CompileOptions) -> Vec<(String, String)> {
    let stamp = String::from_utf8_lossy(&options.timestamp);
    let part = |range: std::ops::Range<usize>| stamp.get(range).unwrap_or("").to_owned();
    let mut macros = vec![
        (
            "__DATE__".to_owned(),
            format!("'{} {} {}'", part(4..7), part(8..10), part(20..24)),
        ),
        ("__TIME__".to_owned(), format!("'{}'", part(11..19))),
    ];
    if options.debug {
        macros.push(("__DEBUG".to_owned(), "1".to_owned()));
    }
    macros.extend([
        ("__HEXICON".to_owned(), "1".to_owned()),
        (
            "__HEXICON_VERSION_MAJOR".to_owned(),
            env!("CARGO_PKG_VERSION_MAJOR").to_owned(),
        ),
        (
            "__HEXICON_VERSION_MINOR".to_owned(),
            env!("CARGO_PKG_VERSION_MINOR").to_owned(),
        ),
    ]);
    macros.extend(options.defines.iter().cloned());
    macros
}

/// The table of macros the reference has when it puts the program
/// together, and the names it undefined on the way.
struct MacroTable {
    /// The macros, each with whether it is one the preprocessor works
    /// out as it goes, which are `__LINE__` and `__FILE__`.
    macros: Vec<(LinkMacro, bool)>,
    undefined: std::collections::HashSet<Vec<u8>>,
}

/// The macros as the `MACR` block has them, for the debugger: those
/// that mean the same throughout, which are those never undefined, but
/// for `__LINE__` and `__FILE__`. Each is its name, its flags, its
/// parameters, and its expansion. Also where in it the date and time of
/// the compilation are, which mean nothing to a comparison of images.
fn debug_macros(table: &MacroTable) -> (Vec<u8>, Vec<(usize, usize)>) {
    let mut count = 0u32;
    let mut out = Vec::new();
    let mut clock = Vec::new();
    for (mac, pseudo) in &table.macros {
        if *pseudo || table.undefined.contains(&mac.name) {
            continue;
        }
        count += 1;
        out.extend_from_slice(&(mac.name.len() as u16).to_le_bytes());
        out.extend_from_slice(&mac.name);
        let flags = u16::from(mac.has_args) | u16::from(mac.has_varargs) << 1;
        out.extend_from_slice(&flags.to_le_bytes());
        out.extend_from_slice(&(mac.params.len() as u16).to_le_bytes());
        for param in &mac.params {
            out.extend_from_slice(&(param.len() as u16).to_le_bytes());
            out.extend_from_slice(param);
        }
        out.extend_from_slice(&(mac.expansion.len() as u32).to_le_bytes());
        if matches!(mac.name.as_slice(), b"__DATE__" | b"__TIME__") {
            clock.push((4 + out.len(), mac.expansion.len()));
        }
        out.extend_from_slice(&mac.expansion);
    }
    let mut block = count.to_le_bytes().to_vec();
    block.extend_from_slice(&out);
    (block, clock)
}

/// The table of macros as the reference has it when it puts the program
/// together: the ones it defines itself, then each module's in turn. A
/// macro two modules define differently is in neither, unless a later
/// module defines it again. `__LINE__` and `__FILE__` are in the table,
/// as `0` and nothing, as they are when no source is being read.
fn link_macro_table(options: &CompileOptions, modules: Vec<Vec<LinkMacro>>) -> MacroTable {
    struct Entry {
        mac: LinkMacro,
        pseudo: bool,
        seq: usize,
    }
    let mut table: Vec<Entry> = Vec::new();
    let mut seq = 0;
    let mut add = |table: &mut Vec<Entry>, mac: LinkMacro, pseudo: bool| {
        table.push(Entry { mac, pseudo, seq });
        seq += 1;
    };
    let plain = |name: &str, expansion: &str| LinkMacro {
        name: name.as_bytes().to_vec(),
        has_args: false,
        has_varargs: false,
        params: Vec::new(),
        expansion: expansion.as_bytes().to_vec(),
    };
    add(&mut table, plain("__LINE__", "0"), true);
    add(&mut table, plain("__FILE__", ""), true);
    for (name, expansion) in predefined_macros(options) {
        add(&mut table, plain(&name, &expansion), false);
    }
    let mut undefined = std::collections::HashSet::new();
    for name in &options.undefines {
        table.retain(|entry| entry.pseudo || entry.mac.name != name.as_bytes());
        undefined.insert(name.as_bytes().to_vec());
    }
    for module in modules {
        for mac in module {
            match table.iter().position(|entry| entry.mac.name == mac.name) {
                Some(at) => {
                    let entry = &table[at];
                    if !entry.pseudo && entry.mac != mac {
                        undefined.insert(mac.name.clone());
                        table.remove(at);
                    }
                }
                None => add(&mut table, mac, false),
            }
        }
    }
    table.sort_by_key(|entry| {
        (
            crate::pp::macro_chain(&entry.mac.name),
            std::cmp::Reverse(entry.seq),
        )
    });
    MacroTable {
        macros: table
            .into_iter()
            .map(|entry| (entry.mac, entry.pseudo))
            .collect(),
        undefined,
    }
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

    for (name, expansion) in predefined_macros(options) {
        pp.define(&name, &expansion);
    }
    for path in &options.include_paths {
        pp.add_include_path(path);
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
            ..Default::default()
        });
    }
    pp.set_compile_mode();
    pp.set_imports(options.imports.clone());
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
    // The built-in libraries' files are part of the compiler.
    let mut files = crate::builtin::WithBuiltIns::new(files);
    let files: &mut dyn SourceFiles = &mut files;
    let mut compiled = Compiled::default();

    let mut exports = Vec::new();
    // The warnings of a Hexicon source's first reading, which its second
    // reading gives again: the reference reports both, Hexicon each once.
    let mut first_warnings: Vec<Vec<Diagnostic>> = Vec::new();
    for source in sources {
        let mut pp = match start(files, source, options) {
            Ok(pp) => pp,
            Err(diagnostic) => {
                compiled.diagnostics.push(diagnostic);
                return compiled;
            }
        };
        let prog = Parser::new(&mut pp, true, Exports::default()).parse_top();
        let mut found = pp.take_diagnostics();
        found.iter_mut().for_each(crate::wording::for_hexicon);
        // A warning is given once, by the second reading, unless only
        // the first gave it.
        let (warnings, mut found): (Vec<_>, Vec<_>) = found
            .into_iter()
            .partition(|d| d.severity == Severity::Warning);
        first_warnings.push(warnings);
        compiled.diagnostics.append(&mut found);
        if has_errors(&compiled.diagnostics) {
            return compiled;
        }
        exports.push(prog.exports());
    }
    let known = merge_exports(exports);

    let mut modules = Vec::new();
    let mut module_macros = Vec::new();
    // The source files of all the modules, in order: each module's
    // files are numbered after those of the modules before it, a file
    // that two modules read once for each.
    let mut source_files: Vec<String> = Vec::new();
    // Names are checked when the game has a source of its own.
    let from_library = |index: usize| options.from_library.get(index) == Some(&true);
    let checks_names = (0..sources.len()).any(|index| !from_library(index));
    let mut names = crate::names::Checker::default();
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
        let mut found = pp.take_diagnostics();
        for diagnostic in &mut found {
            crate::wording::for_hexicon(diagnostic);
            crate::wording::suggest_name(diagnostic, &prog.globals);
        }
        // A warning only the first reading gave is still reported.
        for warning in std::mem::take(&mut first_warnings[index]) {
            if !found.contains(&warning) {
                compiled.diagnostics.push(warning);
            }
        }
        compiled.diagnostics.extend(found);
        module_macros.push(pp.link_macros());
        if has_errors(&compiled.diagnostics) {
            return compiled;
        }

        if checks_names {
            names.add(
                crate::names::collect(&prog),
                !from_library(index),
                from_library(index),
                &|file| pp.file_name(file).to_owned(),
            );
        }
        let mut generated = emit::generate(
            &mut prog,
            options.debug,
            options.line_records || options.debug,
            source_files.len(),
        );
        source_files.extend(pp.file_names());
        for diagnostic in std::mem::take(&mut generated.diagnostics) {
            // A problem that is nowhere in particular has no file. Nor
            // has one found before any statement was come to.
            let file = if diagnostic.file == usize::MAX || diagnostic.line == 0 {
                String::new()
            } else {
                pp.file_name(diagnostic.file).to_owned()
            };
            let mut diagnostic = Diagnostic {
                file,
                line: diagnostic.line,
                severity: diagnostic.severity,
                message: diagnostic.message,
                ..Default::default()
            };
            crate::wording::for_hexicon(&mut diagnostic);
            crate::wording::suggest_name(&mut diagnostic, &prog.globals);
            compiled.diagnostics.push(diagnostic);
        }
        if has_errors(&compiled.diagnostics) {
            return compiled;
        }
        // What is wrong in putting the sources together names the
        // source it was found in.
        let object_file = source
            .rsplit(['/', '\\'])
            .next()
            .unwrap_or(source)
            .to_owned();
        modules.push(Module {
            prog,
            out: generated,
            object_file,
        });
    }

    compiled.diagnostics.extend(names.warnings());
    // Putting the program together: everything it refers to has to be
    // in it.
    let note = |compiled: &mut Compiled, severity: Severity, message: String| {
        compiled.diagnostics.push(Diagnostic {
            file: String::new(),
            line: 0,
            severity,
            message,
            ..Default::default()
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
    let mut generated = emit::link_generate(&mut prog, out, enum_ids.clone());
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
    let exports = emit::resolve_exports(&prog, &mut generated);
    for (severity, message) in exports.problems {
        note(&mut compiled, severity, message);
    }
    for (name, id) in exports.assumed_props {
        prog.add_assumed_prop(name, id);
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

    let table = link_macro_table(options, module_macros);
    let debug = options.debug.then(|| {
        let (macros, macros_unspecified) = debug_macros(&table);
        image::DebugRecords {
            enum_ids: enum_ids.clone(),
            macros,
            macros_unspecified,
        }
    });
    match image::write_image(
        &prog,
        generated,
        &options.timestamp,
        &TOOL_DATA,
        if options.line_records || options.debug {
            &source_files
        } else {
            &[]
        },
        debug.as_ref(),
    ) {
        Ok(image) => {
            compiled.symbols = runtime_symbols(&prog, &enum_ids, &image.func_addrs);
            compiled.macros = table.macros.into_iter().map(|(mac, _)| mac).collect();
            compiled.image = Some(image.bytes);
            compiled.unspecified = image.unspecified;
        }
        Err(message) => note(&mut compiled, Severity::Error, message),
    }
    compiled
}

/// The global symbols, each with its value in the image, in the order of
/// the reference's table of global names when the program is put
/// together.
fn runtime_symbols(
    prog: &crate::parse::Program,
    enum_ids: &[u32],
    func_addrs: &std::collections::HashMap<usize, u32>,
) -> Vec<RuntimeSymbol> {
    prog.globals
        .in_table_order()
        .into_iter()
        .filter_map(|(name, global)| {
            let value = match global {
                Global::Func(func) => RuntimeValue::Function(*func_addrs.get(&func)?),
                Global::Obj(obj) => RuntimeValue::Object(prog.objs[obj].id),
                Global::Prop(prop) => RuntimeValue::Property(prog.props[prop].id),
                Global::Enum(sym) => RuntimeValue::Enum(*enum_ids.get(sym)?),
                Global::Bif(bif) => RuntimeValue::BuiltIn {
                    set: prog.bifs[bif].set as u16,
                    index: prog.bifs[bif].index as u16,
                },
                Global::Meta(meta) => RuntimeValue::Class(prog.metas[meta].obj_id),
                // Only code compiled while the program runs has these.
                Global::DynLocal(_) => return None,
            };
            Some(RuntimeSymbol {
                name: name.to_vec(),
                value,
            })
        })
        .collect()
}
