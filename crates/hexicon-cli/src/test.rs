//! `hexicon test`: build a game for debugging, play its scenarios, and
//! report how they went.
//!
//! The game is built into a file of its own in the game's folder, beside
//! the image its manifest names, which is left alone, and played by
//! `hexicon run` with the same random choices and clock on every run, as
//! game-check plays games. The game is told, by its arguments, to play
//! its scenarios and write what happened to a file; the scenario runner
//! (Hexad's scenarios, as tried in the playground) does so at the first
//! prompt, one line for each scenario, each failure, and a last line, and
//! quits:
//!
//! ```text
//! scenario <tab> name <tab> passed|failed|not run <tab> steps <tab> after
//! failure <tab> name <tab> step <tab> command <tab> why <tab> what failed
//!         <tab> where <tab> what the game said
//! done <tab> passed <tab> failed <tab> not run
//! ```
//!
//! The exit status is 0 if every scenario passed, 1 if any failed or was
//! not run, and 2 if the game could not be built or did not play them.

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};

/// The image the game is built into, and the file its results go to, in
/// the game's folder; both are deleted afterwards.
const IMAGE: &str = ".hexicon-test.hxg";
const RESULTS: &str = ".hexicon-test-results.txt";

/// The clock a game is played with, as game-check plays it.
const CLOCK: &str = "2000-01-01T09:15:00";

pub(crate) fn run(args: &[&str], usage: &str) -> ExitCode {
    let mut manifest: Option<&str> = None;
    let mut transcript: Option<&str> = None;
    let mut names: Vec<&str> = Vec::new();
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
            (option, _) if option.starts_with('-') => {
                eprint!("{usage}");
                return ExitCode::from(2);
            }
            (name, _) => {
                names.push(name);
                rest = tail;
            }
        }
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
    let _ = std::fs::remove_file(&results);

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

    let played = play(&folder, &names);
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
    let _ = std::fs::remove_file(&results);
    report(&lines, &said, transcript.is_some())
}

/// Play the game built for testing, telling it to play its scenarios:
/// what it printed.
fn play(folder: &Path, names: &[&str]) -> std::io::Result<String> {
    let mut command = Command::new(std::env::current_exe()?);
    command
        .args([
            "run", "--norand", "--clock", CLOCK, "--plain", "--nomore", IMAGE,
        ])
        .arg(format!("-hexicon-test={RESULTS}"))
        .args(names.iter().map(|name| format!("-scenario={name}")))
        .current_dir(folder)
        .env("TZ", "Etc/UTC")
        .stdin(Stdio::null());
    let done = command.output()?;
    let mut said = String::from_utf8_lossy(&done.stdout).replace('\r', "");
    said.push_str(&String::from_utf8_lossy(&done.stderr).replace('\r', ""));
    Ok(said)
}

/// One scenario, as the game reported it.
struct Outcome {
    name: String,
    outcome: String,
    steps: String,
    after: String,
    failure: Option<Vec<String>>,
}

/// Print what the game reported, and say how it went.
fn report(lines: &str, said: &str, kept: bool) -> ExitCode {
    let mut outcomes: Vec<Outcome> = Vec::new();
    let mut done: Option<(usize, usize, usize)> = None;
    for line in lines.lines() {
        let fields: Vec<&str> = line.split('\t').collect();
        match fields.as_slice() {
            ["scenario", name, outcome, steps, after] => outcomes.push(Outcome {
                name: (*name).to_owned(),
                outcome: (*outcome).to_owned(),
                steps: (*steps).to_owned(),
                after: (*after).to_owned(),
                failure: None,
            }),
            ["failure", name, rest @ ..] if rest.len() == 6 => {
                if let Some(found) = outcomes.iter_mut().find(|o| o.name == *name) {
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

    let Some((passed, failed, not_run)) = done else {
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
    if outcomes.is_empty() {
        println!("There are no scenarios.");
        return ExitCode::SUCCESS;
    }

    for outcome in &outcomes {
        match outcome.outcome.as_str() {
            "passed" => {
                let s = if outcome.steps == "1" { "" } else { "s" };
                println!("passed   {} ({} step{s})", outcome.name, outcome.steps);
            }
            "not run" => println!(
                "not run  {}, since \"{}\" did not pass",
                outcome.name, outcome.after
            ),
            _ => {
                println!("FAILED   {}", outcome.name);
                if let Some(failure) = &outcome.failure {
                    let [step, command, why, what, place, game_said] = failure.as_slice() else {
                        continue;
                    };
                    if step == "0" {
                        // Not a step: the scenario itself, which is not
                        // there.
                    } else if command.is_empty() {
                        println!("           at step {step}");
                    } else {
                        println!("           at step {step}: {command}");
                    }
                    if !why.is_empty() {
                        println!("           why: {why}");
                    }
                    if place.is_empty() {
                        println!("           failed: {what}");
                    } else {
                        println!("           failed: {what}, at {place}");
                    }
                    if !game_said.is_empty() {
                        println!("           the game said: {game_said}");
                    }
                }
            }
        }
    }
    println!();
    println!("{passed} passed, {failed} failed, {not_run} not run");
    if failed + not_run == 0 {
        return ExitCode::SUCCESS;
    }
    if !kept {
        println!("(--transcript <file> keeps the whole of what the game said)");
    }
    ExitCode::FAILURE
}
