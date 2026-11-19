//! `hexicon build`: compile a source file into a program image.

use std::process::ExitCode;
use std::time::{SystemTime, UNIX_EPOCH};

use hexicon_compiler::pp::Severity;
use hexicon_compiler::{CompileOptions, compile};

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

pub(crate) fn run(args: &[&str], usage: &str) -> ExitCode {
    let mut options = CompileOptions {
        timestamp: timestamp(SystemTime::now()),
        ..CompileOptions::default()
    };
    let mut output: Option<&str> = None;
    let mut unspecified: Option<&str> = None;
    let mut rest = args;
    let source = loop {
        match rest {
            ["-I", dir, tail @ ..] => {
                options.include_paths.push((*dir).to_owned());
                rest = tail;
            }
            ["-D", definition, tail @ ..] => {
                let (name, text) = definition.split_once('=').unwrap_or((definition, "1"));
                options.defines.push((name.to_owned(), text.to_owned()));
                rest = tail;
            }
            ["-U", name, tail @ ..] => {
                options.undefines.push((*name).to_owned());
                rest = tail;
            }
            ["-cs", name, tail @ ..] => {
                options.default_charset = Some((*name).to_owned());
                rest = tail;
            }
            ["-o", path, tail @ ..] => {
                output = Some(path);
                rest = tail;
            }
            ["--unspecified", path, tail @ ..] => {
                unspecified = Some(path);
                rest = tail;
            }
            [source] if !source.starts_with('-') => break *source,
            _ => {
                eprint!("{usage}");
                return ExitCode::from(2);
            }
        }
    };
    // Without a name for the image, it goes beside the source.
    let default_output = std::path::Path::new(source)
        .with_extension("t3")
        .to_string_lossy()
        .into_owned();
    let output = output.unwrap_or(&default_output);

    let compiled = compile(&mut DiskFiles, source, &options);
    for problem in &compiled.diagnostics {
        let kind = match problem.severity {
            Severity::Error => "error",
            Severity::Fatal => "fatal error",
            Severity::Warning => "warning",
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
    if let Err(error) = std::fs::write(output, image) {
        eprintln!("hexicon: {output}: {error}");
        return ExitCode::FAILURE;
    }
    if let Some(path) = unspecified {
        let list: String = compiled
            .unspecified
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
