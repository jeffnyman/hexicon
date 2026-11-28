//! `hexicon test`: build a game for debugging, play its scenarios and
//! walkthroughs, and report how they went; or print a walkthrough.
//!
//! The game is built into a file of its own in the game's folder, beside
//! the image its manifest names, which is left alone, and played by
//! `hexicon run` with the same random choices and clock on every run, as
//! game-check plays games. The game is told, by its arguments, what to
//! play (`-scenario=<name>` for each named, `-walkthroughs` for every
//! walkthrough) and where to write what happened (`-hexicon-test=<file>`)
//! and the walkthroughs that pass, printed (`-hexicon-print=<file>`). The
//! scenario runner (Hexad's scenarios, as tried in the playground) does so
//! at the first prompt, a line for each scenario or walkthrough, its notes
//! and its failure, and a last line, and quits:
//!
//! ```text
//! scenario <tab> name <tab> passed|failed|not run <tab> steps <tab> after
//! walkthrough <tab> name <tab> passed|failed <tab> steps <tab>
//! note <tab> name <tab> what is noted
//! failure <tab> name <tab> step <tab> command <tab> why <tab> what failed
//!         <tab> where <tab> what the game said
//! done <tab> passed <tab> failed <tab> not run
//! ```
//!
//! The exit status is 0 if everything played passed, 1 if anything failed
//! or was not run, and 2 if the game could not be built or did not play
//! them.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};

/// The image the game is built into, the file its results go to, and the
/// file a printed walkthrough goes to, in the game's folder; all are
/// deleted afterwards.
const IMAGE: &str = ".hexicon-test.hxg";
const RESULTS: &str = ".hexicon-test-results.txt";
const PRINTED: &str = ".hexicon-test-printed.md";

/// The clock a game is played with, as game-check plays it.
const CLOCK: &str = "2000-01-01T09:15:00";

/// What to play, and how.
#[derive(Default)]
struct Asked<'a> {
    names: Vec<&'a str>,
    walkthroughs: bool,
    print: bool,
}

pub(crate) fn run(args: &[&str], usage: &str) -> ExitCode {
    let mut manifest: Option<&str> = None;
    let mut transcript: Option<&str> = None;
    let mut asked = Asked::default();
    let mut rest = args;
    while let Some((first, tail)) = rest.split_first() {
        match (*first, tail) {
            ("-f", [path, tail @ ..]) => {
                manifest = Some(path);
                rest = tail;
            }
            ("--transcript", [path, tail @ ..]) => {
                transcript = Some(path);
                rest = tail;
            }
            ("--walkthroughs", _) => {
                asked.walkthroughs = true;
                rest = tail;
            }
            ("--print", [name, tail @ ..]) => {
                asked.print = true;
                asked.names = vec![name];
                rest = tail;
            }
            (option, _) if option.starts_with('-') => {
                eprint!("{usage}");
                return ExitCode::from(2);
            }
            (name, _) => {
                asked.names.push(name);
                rest = tail;
            }
        }
    }
    if asked.print && asked.names.len() != 1 {
        eprint!("{usage}");
        return ExitCode::from(2);
    }

    // The game's folder: the manifest's, or this one.
    let folder = match manifest {
        Some(path) if Path::new(path).is_dir() => PathBuf::from(path),
        Some(path) => Path::new(path)
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_default(),
        None => PathBuf::from("."),
    };
    let folder = if folder.as_os_str().is_empty() {
        PathBuf::from(".")
    } else {
        folder
    };
    let image = folder.join(IMAGE);
    let results = folder.join(RESULTS);
    let printed = folder.join(PRINTED);
    let _ = std::fs::remove_file(&results);
    let _ = std::fs::remove_file(&printed);

    let mut project_args = vec!["-d".to_owned()];
    if let Some(path) = manifest {
        project_args.push("-f".to_owned());
        project_args.push(path.to_owned());
    }
    let built =
        crate::build::build_project(&project_args, usage, None, Some(&image.to_string_lossy()));
    if built != ExitCode::SUCCESS {
        eprintln!("hexicon test: the game could not be built");
        return ExitCode::from(2);
    }

    let played = play(&folder, &asked);
    let _ = std::fs::remove_file(&image);
    let said = match played {
        Ok(said) => said,
        Err(error) => {
            eprintln!("hexicon test: the game could not be played: {error}");
            return ExitCode::from(2);
        }
    };
    if let Some(path) = transcript
        && let Err(error) = std::fs::write(path, &said)
    {
        eprintln!("hexicon test: {path}: {error}");
    }
    let lines = std::fs::read_to_string(&results).unwrap_or_default();
    let document = std::fs::read_to_string(&printed).unwrap_or_default();
    let _ = std::fs::remove_file(&results);
    let _ = std::fs::remove_file(&printed);

    let Some((text, status)) = report(&lines, transcript.is_some()) else {
        eprintln!(
            "hexicon test: the game did not play its scenarios: are Hexad's \
             scenarios among its sources? What it printed ends:"
        );
        let tail: Vec<&str> = said.lines().rev().take(12).collect();
        for line in tail.iter().rev() {
            eprintln!("    {line}");
        }
        return ExitCode::from(2);
    };
    if asked.print {
        // The walkthrough is the output; how it went is said aside.
        eprint!("{text}");
        if status == ExitCode::SUCCESS {
            print!("{document}");
        }
    } else {
        print!("{text}");
    }
    status
}

/// Play the game built for testing, telling it what to play: what it
/// printed.
fn play(folder: &Path, asked: &Asked<'_>) -> std::io::Result<String> {
    let mut command = Command::new(std::env::current_exe()?);
    command
        .args([
            "run", "--norand", "--clock", CLOCK, "--plain", "--nomore", IMAGE,
        ])
        .arg(format!("-hexicon-test={RESULTS}"))
        .args(asked.names.iter().map(|name| format!("-scenario={name}")))
        .current_dir(folder)
        .env("TZ", "Etc/UTC")
        .stdin(Stdio::null());
    if asked.walkthroughs {
        command.arg("-walkthroughs");
    }
    if asked.print {
        command.arg(format!("-hexicon-print={PRINTED}"));
    }
    let done = command.output()?;
    let mut said = String::from_utf8_lossy(&done.stdout).replace('\r', "");
    said.push_str(&String::from_utf8_lossy(&done.stderr).replace('\r', ""));
    Ok(said)
}

/// One scenario or walkthrough, as the game reported it.
struct Outcome {
    walkthrough: bool,
    name: String,
    outcome: String,
    steps: String,
    after: String,
    notes: Vec<String>,
    failure: Option<Vec<String>>,
}

/// The report of what the game reported, and the exit status; or None if
/// it reported nothing.
fn report(lines: &str, kept: bool) -> Option<(String, ExitCode)> {
    let mut outcomes: Vec<Outcome> = Vec::new();
    let mut done: Option<(usize, usize, usize)> = None;
    for line in lines.lines() {
        let fields: Vec<&str> = line.split('\t').collect();
        match fields.as_slice() {
            [
                kind @ ("scenario" | "walkthrough"),
                name,
                outcome,
                steps,
                after,
            ] => {
                outcomes.push(Outcome {
                    walkthrough: *kind == "walkthrough",
                    name: (*name).to_owned(),
                    outcome: (*outcome).to_owned(),
                    steps: (*steps).to_owned(),
                    after: (*after).to_owned(),
                    notes: Vec::new(),
                    failure: None,
                });
            }
            ["note", name, note] => {
                if let Some(found) = outcomes.iter_mut().rfind(|o| o.name == *name) {
                    found.notes.push((*note).to_owned());
                }
            }
            ["failure", name, rest @ ..] if rest.len() == 6 => {
                if let Some(found) = outcomes.iter_mut().rfind(|o| o.name == *name) {
                    found.failure = Some(rest.iter().map(|s| (*s).to_owned()).collect());
                }
            }
            ["done", passed, failed, not_run] => {
                done = match (passed.parse(), failed.parse(), not_run.parse()) {
                    (Ok(p), Ok(f), Ok(n)) => Some((p, f, n)),
                    _ => None,
                };
            }
            _ => {}
        }
    }

    let (passed, failed, not_run) = done?;
    let mut out = String::new();
    if outcomes.is_empty() {
        out.push_str("There are no scenarios.\n");
        return Some((out, ExitCode::SUCCESS));
    }

    for outcome in &outcomes {
        let name = if outcome.walkthrough {
            format!("walkthrough: {}", outcome.name)
        } else {
            outcome.name.clone()
        };
        match outcome.outcome.as_str() {
            "passed" => {
                let s = if outcome.steps == "1" { "" } else { "s" };
                let _ = writeln!(out, "passed   {name} ({} step{s})", outcome.steps);
            }
            "not run" => {
                let _ = writeln!(
                    out,
                    "not run  {name}, since \"{}\" did not pass",
                    outcome.after
                );
            }
            _ => {
                let _ = writeln!(out, "FAILED   {name}");
                if let Some([step, command, why, what, place, game_said]) =
                    outcome.failure.as_deref()
                {
                    if step == "0" {
                        // Not a step: the scenario itself, which is not
                        // there.
                    } else if command.is_empty() {
                        let _ = writeln!(out, "           at step {step}");
                    } else {
                        let _ = writeln!(out, "           at step {step}: {command}");
                    }
                    if !why.is_empty() {
                        let _ = writeln!(out, "           why: {why}");
                    }
                    if place.is_empty() {
                        let _ = writeln!(out, "           failed: {what}");
                    } else {
                        let _ = writeln!(out, "           failed: {what}, at {place}");
                    }
                    if !game_said.is_empty() {
                        let _ = writeln!(out, "           the game said: {game_said}");
                    }
                }
            }
        }
        for note in &outcome.notes {
            let _ = writeln!(out, "           note: {note}");
        }
    }
    let _ = writeln!(out, "\n{passed} passed, {failed} failed, {not_run} not run");
    if failed + not_run == 0 {
        return Some((out, ExitCode::SUCCESS));
    }
    if !kept {
        out.push_str("(--transcript <file> keeps the whole of what the game said)\n");
    }
    Some((out, ExitCode::FAILURE))
}
