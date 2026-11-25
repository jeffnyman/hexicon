//! `hexicon build`: compile source files into a program image.

use std::process::ExitCode;
use std::time::{SystemTime, UNIX_EPOCH};

use hexicon_compiler::pp::Severity;
use hexicon_compiler::{CompileOptions, ProjectError, ProjectPaths, compile_program, read_project};

use crate::preprocess::DiskFiles;

/// The time as an image records it: 24 characters, like
/// `Thu Nov 19 09:24:12 2026`. It is in UTC.
fn timestamp(now: SystemTime) -> [u8; 24] {
    const DAYS: [&str; 7] = ["Thu", "Fri", "Sat", "Sun", "Mon", "Tue", "Wed"];
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    let secs = now
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_secs());
    let days = secs / 86_400;
    let time = secs % 86_400;

    // The date from the count of days, by the usual arithmetic on
    // 400-year cycles, with years taken to start in March.
    let shifted = days + 719_468;
    let era = shifted / 146_097;
    let day_of_era = shifted % 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let shifted_month = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * shifted_month + 2) / 5 + 1;
    let month = if shifted_month < 10 {
        shifted_month + 3
    } else {
        shifted_month - 9
    };
    let year = year_of_era + era * 400 + u64::from(month <= 2);

    let text = format!(
        "{} {} {:2} {:02}:{:02}:{:02} {:04}",
        DAYS[(days % 7) as usize],
        MONTHS[(month - 1) as usize],
        day,
        time / 3600,
        time / 60 % 60,
        time % 60,
        year % 10_000,
    );
    let mut stamp = [b' '; 24];
    for (to, from) in stamp.iter_mut().zip(text.bytes()) {
        *to = from;
    }
    stamp
}

/// A problem in a Hexicon source, as Hexicon shows one: what it is,
/// where, the lines of the source, and a note on it.
///
/// ```text
/// error: the object above ends without a ';'
///   --> start.hxn:41
///    |
/// 41 |     "A battered tin box, wedged between two rocks. "
/// 42 | ++ brassKey: Key 'small brass key'
///    |
///    = each object ends with ';', and the next starts here
/// ```
fn show_hexicon(problem: &hexicon_compiler::pp::Diagnostic, kind: &str) -> String {
    let mut out = format!(
        "{kind}: {}\n  --> {}:{}\n",
        problem.message, problem.file, problem.line
    );
    let (first, last) = problem.lines.unwrap_or((problem.line, problem.line));
    let text = hexicon_compiler::builtin::file(&problem.file)
        .map(<[u8]>::to_vec)
        .or_else(|| std::fs::read(&problem.file).ok())
        .map(|bytes| String::from_utf8_lossy(&bytes).into_owned());
    let lines: Vec<(u32, &str)> = match &text {
        Some(text) => text
            .lines()
            .zip(1u32..)
            .map(|(line, n)| (n, line))
            .filter(|(n, _)| (first..=last).contains(n))
            .collect(),
        None => Vec::new(),
    };
    let width = last.to_string().len();
    let gutter = " ".repeat(width);
    if !lines.is_empty() {
        out += &format!("{gutter} |\n");
        for (n, line) in &lines {
            let shown = format!("{n:>width$} | {line}");
            out += shown.trim_end();
            out.push('\n');
        }
        out += &format!("{gutter} |\n");
    }
    if let Some(note) = &problem.note {
        out += &format!("{gutter} = {note}\n");
    }
    out.push('\n');
    out
}

/// The folders of the `HEXICON_LIB` environment variable, for more
/// libraries. The system library and Hexad are built in.
fn default_paths() -> ProjectPaths {
    let separator = if cfg!(windows) { ';' } else { ':' };
    let user_libraries = std::env::var("HEXICON_LIB")
        .map(|value| {
            value
                .split(separator)
                .filter(|dir| !dir.is_empty())
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default();
    ProjectPaths { user_libraries }
}

pub(crate) fn run(args: &[&str], usage: &str) -> ExitCode {
    // The options are the project's, but for `--unspecified`.
    let mut unspecified: Option<&str> = None;
    let mut project_args: Vec<String> = Vec::new();
    let mut rest = args;
    while let Some((first, tail)) = rest.split_first() {
        if *first == "--unspecified" {
            let Some((path, tail)) = tail.split_first() else {
                eprint!("{usage}");
                return ExitCode::from(2);
            };
            unspecified = Some(path);
            rest = tail;
            continue;
        }
        project_args.push((*first).to_owned());
        rest = tail;
    }

    let project = match read_project(&mut DiskFiles, &project_args, &default_paths()) {
        Ok(project) => project,
        Err(ProjectError::Usage(problem)) => {
            if !problem.is_empty() {
                eprintln!("hexicon: {problem}");
            }
            eprint!("{usage}");
            return ExitCode::from(2);
        }
        Err(ProjectError::Failed(lines)) => {
            for line in lines {
                eprintln!("{line}");
            }
            return ExitCode::FAILURE;
        }
    };
    for line in &project.messages {
        eprintln!("{line}");
    }
    let Some(output) = project.image.clone() else {
        eprint!("{usage}");
        return ExitCode::from(2);
    };
    let options = CompileOptions {
        timestamp: timestamp(SystemTime::now()),
        ..project.options
    };
    let sources: Vec<&str> = project.sources.iter().map(String::as_str).collect();
    // What is built, as the reference shows its steps: each on a line
    // of its own, after a tab.
    if project.verbose {
        eprintln!("	Files to build: {}", sources.len());
        for source in &sources {
            eprintln!("	compile {source}");
        }
        eprintln!("	link -> {output}");
    }

    let compiled = compile_program(&mut DiskFiles, &sources, &options);
    let mut warned = false;
    // In a Hexicon source, a message is shown once, and what follows a
    // string with no end in the same file is only counted: it follows
    // from that.
    let mut shown: Vec<(String, u32, String)> = Vec::new();
    let mut swallowed: Vec<(String, usize)> = Vec::new();
    for problem in &compiled.diagnostics {
        if problem.hexicon {
            if let Some((_, count)) = swallowed.iter_mut().find(|(file, _)| *file == problem.file) {
                *count += 1;
                continue;
            }
            let key = (problem.file.clone(), problem.line, problem.message.clone());
            if shown.contains(&key) {
                continue;
            }
            shown.push(key);
            if problem.message.starts_with("this string has no end") {
                swallowed.push((problem.file.clone(), 0));
            }
        }
        let kind = match problem.severity {
            Severity::Error => "error",
            Severity::Fatal => "fatal error",
            Severity::Internal => "internal error",
            Severity::Warning if !project.warnings => continue,
            Severity::Warning => {
                warned = true;
                "warning"
            }
            Severity::Message => {
                eprintln!("{}", problem.message);
                continue;
            }
        };
        if problem.hexicon && !problem.file.is_empty() {
            eprint!("{}", show_hexicon(problem, kind));
        } else if problem.file.is_empty() {
            eprintln!("{kind}: {}", problem.message);
        } else {
            eprintln!(
                "{}({}): {kind}: {}",
                problem.file, problem.line, problem.message
            );
        }
    }
    for (file, count) in swallowed {
        if count > 0 {
            eprintln!(
                "({count} more problems in {file} follow from the string with no end, \
                 and are not shown)\n"
            );
        }
    }
    let Some(image) = compiled.image else {
        return ExitCode::FAILURE;
    };
    // With `-we`, a warning is as bad as an error.
    if warned && project.warnings_as_errors {
        return ExitCode::FAILURE;
    }
    // The program is run once before the image is written, unless told
    // not to, and the image is what that run leaves: preinitialization.
    // It is given the program's symbols and macros, and no arguments,
    // and goes by the name of the image it makes.
    let (image, image_unspecified) = if project.preinit {
        if project.verbose {
            eprintln!("	preinit -> {output}");
        }
        let name = output.clone();
        let symbols = (
            compiled.symbols.iter().map(global_symbol).collect(),
            compiled.macros.iter().map(runtime_macro).collect(),
        );
        match crate::preinit_image(&name, &image, &compiled.unspecified, &[], Some(symbols)) {
            Ok(rebuilt) => (rebuilt.bytes, rebuilt.unspecified),
            // No image is written: one would only run its failed
            // preinitialization again when the game starts.
            Err(message) => {
                eprintln!("error: preinitialization failed: {message}");
                return ExitCode::FAILURE;
            }
        }
    } else {
        (image, compiled.unspecified.clone())
    };
    // The resources go on the end, after preinitialization: the files
    // themselves, or in a build for debugging links to them.
    let mut image = image;
    if !project.resources.is_empty() {
        let link = if project.debug { " link" } else { "" };
        if project.verbose {
            eprintln!("	add_resource{link}s -> {output}");
        }
        let resources = crate::resources::list(&project.resources);
        let mut step = |message: &str| {
            if project.verbose {
                eprintln!("	{message}");
            }
        };
        if let Err(message) =
            crate::resources::bundle(&mut image, &output, &resources, project.debug, &mut step)
        {
            eprintln!("{message}");
            return ExitCode::FAILURE;
        }
    }
    if let Err(error) = std::fs::write(&output, image) {
        eprintln!("hexicon: {output}: {error}");
        return ExitCode::FAILURE;
    }
    // A build from a manifest says what it made.
    if project.manifest {
        println!("Built {output}.");
    }
    if let Some(path) = unspecified {
        let list: String = image_unspecified
            .iter()
            .map(|(offset, len)| format!("{offset} {len}\n"))
            .collect();
        if let Err(error) = std::fs::write(path, list) {
            eprintln!("hexicon: {path}: {error}");
            return ExitCode::FAILURE;
        }
    }
    ExitCode::SUCCESS
}

/// A global symbol as the VM takes it.
fn global_symbol(symbol: &hexicon_compiler::RuntimeSymbol) -> hexicon_image::GlobalSymbol {
    use hexicon_compiler::RuntimeValue;
    use hexicon_image::GlobalSymbolKind;
    let kind = match symbol.value {
        RuntimeValue::Function(code_offset) => GlobalSymbolKind::Function {
            code_offset,
            argc: 0,
            varargs: false,
            has_return: true,
            optional_argc: 0,
        },
        RuntimeValue::Object(id) => GlobalSymbolKind::Object { id, modifier: 0 },
        RuntimeValue::Property(id) => GlobalSymbolKind::Property {
            id,
            dictionary: false,
        },
        RuntimeValue::Enum(id) => GlobalSymbolKind::Enumerator { id, token: false },
        RuntimeValue::BuiltIn { set, index } => GlobalSymbolKind::BuiltinFunction {
            index,
            set,
            has_return: true,
            min_argc: 0,
            max_argc: 0,
            varargs: false,
        },
        RuntimeValue::Class(class_object) => GlobalSymbolKind::BuiltinClass {
            metaclass_index: 0,
            class_object,
        },
    };
    hexicon_image::GlobalSymbol {
        name: String::from_utf8_lossy(&symbol.name).into_owned(),
        kind,
    }
}

/// A macro as the VM takes it.
fn runtime_macro(mac: &hexicon_compiler::pp::LinkMacro) -> hexicon_image::Macro {
    let text = |bytes: &[u8]| String::from_utf8_lossy(bytes).into_owned();
    hexicon_image::Macro {
        name: text(&mac.name),
        function_like: mac.has_args,
        varargs: mac.has_varargs,
        parameters: mac.params.iter().map(|param| text(param)).collect(),
        expansion: text(&mac.expansion),
    }
}
