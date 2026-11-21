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

/// Where the system's headers and library are: as for the reference,
/// in `include` and `lib` beside the program, unless `-FI` and `-FL`
/// say otherwise; and the folders of the `TADSLIB` environment
/// variable for more libraries.
fn default_paths() -> ProjectPaths {
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(std::path::Path::to_path_buf))
        .unwrap_or_default();
    let beside = |name: &str| exe_dir.join(name).to_string_lossy().into_owned();
    let separator = if cfg!(windows) { ';' } else { ':' };
    let user_libraries = std::env::var("TADSLIB")
        .map(|value| {
            value
                .split(separator)
                .filter(|dir| !dir.is_empty())
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default();
    ProjectPaths {
        system_include: beside("include"),
        system_library: beside("lib"),
        user_libraries,
    }
}

pub(crate) fn run(args: &[&str], usage: &str) -> ExitCode {
    // What `t3make` takes is for the project; `--unspecified` is
    // Hexicon's own.
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
    if project.preinit {
        eprintln!(
            "hexicon: the program is not run before the image is written: preinitialization is not done yet (-nopre says not to)"
        );
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
    for problem in &compiled.diagnostics {
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
        if problem.file.is_empty() {
            eprintln!("{kind}: {}", problem.message);
        } else {
            eprintln!(
                "{}({}): {kind}: {}",
                problem.file, problem.line, problem.message
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
    if let Err(error) = std::fs::write(&output, image) {
        eprintln!("hexicon: {output}: {error}");
        return ExitCode::FAILURE;
    }
    if let Some(path) = unspecified {
        let list: String = compiled
            .unspecified
            .iter()
            .map(|(offset, len)| {
                format!(
                    "{offset} {len}
"
                )
            })
            .collect();
        if let Err(error) = std::fs::write(path, list) {
            eprintln!("hexicon: {path}: {error}");
            return ExitCode::FAILURE;
        }
    }
    ExitCode::SUCCESS
}
