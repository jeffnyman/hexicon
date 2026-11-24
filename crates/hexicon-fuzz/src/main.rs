//! Feeds damaged program images and saved games to Hexicon's readers.
//!
//! ```text
//! hexicon-fuzz images <out-dir> <cases> <seed> <image>...
//! hexicon-fuzz saves  <out-dir> <cases> <seed> <probe.hxg>
//! ```
//!
//! Each case is a copy of one of the files given with a few mutations:
//! bits flipped, bytes set to boundary values, numbers in block headers
//! changed, stretches cut out or repeated, the end cut off. For a saved
//! game the mutations are mostly in its body, with its length and
//! checksum made to match, so that the case gets past them to the
//! reader proper. A case is made from the seed and its number alone, so
//! any case can be made again.
//!
//! An image is parsed, loaded, and run with no input. A saved game is
//! restored into the probe program, which then walks what it finds; the
//! probe is built from `probe.hxn`, beside this crate:
//!
//! ```text
//! hexicon build -nopre -o probe.hxg crates/hexicon-fuzz/probe.hxn
//! ```
//!
//! Damaged input should give an error, never a panic, a crash, or an
//! allocation that runs away. The cases run in child processes, so that
//! one that crashes or hangs costs only its own process: the parent
//! notes the case, keeps its input in the output folder, and carries on
//! from the next one. A case that runs too long is noted as a hang,
//! with what it was doing; a damaged program can loop for ever without
//! anything being wrong, so a hang in the run is worth checking against
//! the reference before it is called a bug.
//!
//! To look into one case, run a child for it alone:
//!
//! ```text
//! hexicon-fuzz child images|saves <seed> <case> <case + 1> <file>...
//! ```
//!
//! with `HEXICON_FUZZ_VERBOSE=1 RUST_BACKTRACE=1` for the usual panic
//! report, and `HEXICON_FUZZ_STACK_MB=512` to run it on a larger stack,
//! which tells deep recursion from endless recursion. Built with
//! `--features profile`, the VM can also say what it is doing as it
//! goes, with `HEXICON_PROF_LIVE=1`.

use std::io::{BufRead, BufReader, Write};
use std::process::{Command, ExitCode, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use hexicon_host::BufferHost;
use hexicon_image::Image;
use hexicon_vm::Vm;

/// How long a case may take before it counts as a hang.
const CASE_TIMEOUT: Duration = Duration::from_secs(10);

/// A small random number generator: SplitMix64.
struct Rng(u64);

impl Rng {
    fn new(seed: u64, case: u64) -> Self {
        Self(seed ^ case.wrapping_mul(0x9E37_79B9_7F4A_7C15))
    }

    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// A number below `n`, which must not be 0.
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

/// Where the blocks of an image or saved game seem to start: four
/// letters of a block name. Mutations aimed at these hit the lengths
/// and counts that readers trust.
fn block_starts(data: &[u8]) -> Vec<usize> {
    (0..data.len().saturating_sub(10))
        .filter(|&at| {
            data[at..at + 4]
                .iter()
                .all(|b| b.is_ascii_uppercase() || *b == b' ')
        })
        .filter(|&at| data[at..at + 4].iter().any(u8::is_ascii_uppercase))
        .collect()
}

/// The input for one case.
fn mutate(original: &[u8], blocks: &[usize], rng: &mut Rng) -> Vec<u8> {
    let mut data = original.to_vec();
    let count = 1 + rng.below(4);
    for _ in 0..count {
        if data.is_empty() {
            break;
        }
        match rng.below(7) {
            // Flip a bit.
            0 => {
                let at = rng.below(data.len());
                data[at] ^= 1 << rng.below(8);
            }
            // A boundary value in one byte.
            1 => {
                let at = rng.below(data.len());
                data[at] = [0x00, 0x01, 0x7f, 0x80, 0xff][rng.below(5)];
            }
            // A boundary value in a 16- or 32-bit number near the start
            // of a block, where its lengths and counts are.
            2 | 3 if !blocks.is_empty() => {
                let start = blocks[rng.below(blocks.len())];
                let at = start + 4 + rng.below(28);
                let values: [u32; 7] = [0, 1, 0x7f, 0xffff, 0x7fff_ffff, 0xffff_ffff, 0x10000];
                let value = values[rng.below(values.len())];
                if rng.below(2) == 0 {
                    if at + 2 <= data.len() {
                        data[at..at + 2].copy_from_slice(&(value as u16).to_le_bytes());
                    }
                } else if at + 4 <= data.len() {
                    data[at..at + 4].copy_from_slice(&value.to_le_bytes());
                }
            }
            // Cut out a stretch.
            4 => {
                let at = rng.below(data.len());
                let len = 1 + rng.below(64.min(data.len() - at));
                data.drain(at..at + len);
            }
            // Repeat a stretch.
            5 => {
                let at = rng.below(data.len());
                let len = 1 + rng.below(64.min(data.len() - at));
                let copy = data[at..at + len].to_vec();
                data.splice(at..at, copy);
            }
            // Cut off the end.
            _ => {
                let at = rng.below(data.len());
                data.truncate(at);
            }
        }
    }
    data
}

/// The length of a saved game's signature, and of the header with the
/// body's length and checksum after it.
const SAVE_SIGNATURE: usize = 17;
const SAVE_HEADER: usize = SAVE_SIGNATURE + 8;

/// The checksum of a saved game's body, as the VM computes it.
fn checksum(bytes: &[u8]) -> u32 {
    let mut acc: u32 = 0;
    for &byte in bytes {
        let mut entry = (acc ^ u32::from(byte)) & 0xff;
        for _ in 0..8 {
            entry = if entry & 1 != 0 {
                (entry >> 1) ^ 0xedb8_8320
            } else {
                entry >> 1
            };
        }
        acc = entry ^ (acc >> 8);
    }
    acc
}

/// A damaged saved game. Mostly the body is damaged and its length and
/// checksum made to match, so that the case gets past them to the
/// reader proper; now and then the header is left as damaged.
fn mutate_save(original: &[u8], blocks: &[usize], rng: &mut Rng) -> Vec<u8> {
    if original.len() <= SAVE_HEADER || rng.below(8) == 0 {
        return mutate(original, blocks, rng);
    }
    let body_blocks: Vec<usize> = blocks
        .iter()
        .filter(|&&at| at >= SAVE_HEADER)
        .map(|&at| at - SAVE_HEADER)
        .collect();
    let body = mutate(&original[SAVE_HEADER..], &body_blocks, rng);
    let mut data = original[..SAVE_SIGNATURE].to_vec();
    data.extend_from_slice(&(body.len() as u32).to_le_bytes());
    data.extend_from_slice(&checksum(&body).to_le_bytes());
    data.extend_from_slice(&body);
    data
}

/// What happened to a case that did not crash.
fn outcome_of(result: Result<String, String>) -> String {
    match result {
        Ok(stage) => stage,
        Err(message) => format!("panic: {message}"),
    }
}

/// Say what a case has got to, so that a hang can be placed.
fn stage(name: &str) {
    let mut out = std::io::stdout();
    let _ = writeln!(out, "stage {name}");
    let _ = out.flush();
}

/// Parse, load, and run an image.
fn try_image(data: &[u8]) -> String {
    stage("parse");
    let Ok(image) = Image::parse(data) else {
        return "rejected by the image reader".to_owned();
    };
    let host = BufferHost::new();
    stage("load");
    let Ok(mut vm) = Vm::new(&image, Box::new(host)) else {
        return "rejected by the loader".to_owned();
    };
    stage("run");
    match vm.run(&["fuzz.t3".to_owned()]) {
        Ok(()) => "ran".to_owned(),
        Err(_) => "ran to an error".to_owned(),
    }
}

/// Restore a saved game into the probe program.
fn try_save(probe: &Image, data: &[u8]) -> String {
    let host = BufferHost::new();
    host.set_file("fuzz.sav", data);
    let Ok(mut vm) = Vm::new(probe, Box::new(host.clone())) else {
        return "the probe does not load".to_owned();
    };
    stage("restore");
    let result = vm.run(&["probe.hxg".to_owned(), "restore".to_owned()]);
    vm.flush_output();
    let output = host.output();
    match result {
        Err(_) => "ran to an error".to_owned(),
        Ok(()) if output.contains("error:") => format!(
            "rejected {}",
            output
                .lines()
                .find(|l| l.contains("error:"))
                .unwrap_or("")
                .chars()
                .take(70)
                .collect::<String>()
        ),
        Ok(()) => "restored".to_owned(),
    }
}

/// A saved game of the probe program, to damage.
fn probe_save(probe: &Image) -> Result<Vec<u8>, String> {
    let host = BufferHost::new();
    let mut vm = Vm::new(probe, Box::new(host.clone())).map_err(|e| e.to_string())?;
    vm.run(&["probe.hxg".to_owned(), "save".to_owned()])
        .map_err(|e| e.to_string())?;
    host.file("fuzz.sav")
        .ok_or_else(|| "the probe saved nothing".to_owned())
}

/// The child's side: run cases `first..last`, saying when each starts
/// and how it ended.
fn child(mode: &str, seed: u64, first: u64, last: u64, files: &[String]) -> ExitCode {
    let originals: Vec<Vec<u8>> = match files.iter().map(std::fs::read).collect() {
        Ok(data) => data,
        Err(error) => {
            eprintln!("hexicon-fuzz: {error}");
            return ExitCode::FAILURE;
        }
    };
    let probe = if mode == "saves" {
        match Image::parse(&originals[0]) {
            Ok(image) => Some(image),
            Err(error) => {
                eprintln!("hexicon-fuzz: the probe: {error}");
                return ExitCode::FAILURE;
            }
        }
    } else {
        None
    };
    let originals = match &probe {
        Some(probe) => match probe_save(probe) {
            Ok(save) => vec![save],
            Err(error) => {
                eprintln!("hexicon-fuzz: the probe: {error}");
                return ExitCode::FAILURE;
            }
        },
        None => originals,
    };
    let blocks: Vec<Vec<usize>> = originals.iter().map(|data| block_starts(data)).collect();
    // A panic is reported as the case's outcome, not on standard error,
    // unless HEXICON_FUZZ_VERBOSE asks for the usual report as well.
    if std::env::var_os("HEXICON_FUZZ_VERBOSE").is_none() {
        std::panic::set_hook(Box::new(|_| {}));
    }
    let mut out = std::io::stdout();
    for case in first..last {
        let mut rng = Rng::new(seed, case);
        let which = rng.below(originals.len());
        let data = if probe.is_some() {
            mutate_save(&originals[which], &blocks[which], &mut rng)
        } else {
            mutate(&originals[which], &blocks[which], &mut rng)
        };
        let _ = writeln!(out, "start {case}");
        let _ = out.flush();
        let run = || match &probe {
            Some(probe) => try_save(probe, &data),
            None => try_image(&data),
        };
        // HEXICON_FUZZ_STACK_MB runs each case on a thread with a stack
        // that size, to tell deep recursion from endless recursion.
        let stack_mb: Option<usize> = std::env::var("HEXICON_FUZZ_STACK_MB")
            .ok()
            .and_then(|mb| mb.parse().ok());
        let result = std::panic::catch_unwind(|| match stack_mb {
            Some(mb) => std::thread::scope(|scope| {
                std::thread::Builder::new()
                    .stack_size(mb << 20)
                    .spawn_scoped(scope, run)
                    .expect("a thread")
                    .join()
                    .unwrap_or_else(|_| "panic: (on its thread)".to_owned())
            }),
            None => run(),
        })
        .map_err(|payload| {
            payload
                .downcast_ref::<String>()
                .cloned()
                .or_else(|| payload.downcast_ref::<&str>().map(|s| (*s).to_owned()))
                .unwrap_or_else(|| "(no message)".to_owned())
        });
        let _ = writeln!(out, "done {case} {}", outcome_of(result));
        let _ = out.flush();
    }
    ExitCode::SUCCESS
}

/// The input of a case, made again, for keeping.
fn case_input(mode: &str, seed: u64, case: u64, files: &[String]) -> Option<Vec<u8>> {
    let originals: Vec<Vec<u8>> = files.iter().filter_map(|f| std::fs::read(f).ok()).collect();
    let originals = if mode == "saves" {
        vec![probe_save(&Image::parse(originals.first()?).ok()?).ok()?]
    } else {
        originals
    };
    let mut rng = Rng::new(seed, case);
    let which = rng.below(originals.len());
    let blocks = block_starts(&originals[which]);
    Some(if mode == "saves" {
        mutate_save(&originals[which], &blocks, &mut rng)
    } else {
        mutate(&originals[which], &blocks, &mut rng)
    })
}

/// The parent's side: run the cases in children, a batch at a time, and
/// note every case that panics, crashes, or hangs.
fn parent(mode: &str, out_dir: &str, cases: u64, seed: u64, files: &[String]) -> ExitCode {
    let exe = std::env::current_exe().expect("the fuzzer's own path");
    let _ = std::fs::create_dir_all(out_dir);
    let mut tally: std::collections::BTreeMap<String, u64> = Default::default();
    let mut findings = Vec::new();
    let mut next = 0;
    let started = Instant::now();
    while next < cases {
        let mut child = Command::new(&exe)
            .arg("child")
            .arg(mode)
            .arg(seed.to_string())
            .arg(next.to_string())
            .arg(cases.to_string())
            .args(files)
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("a child process");
        let stdout = child.stdout.take().expect("the child's output");
        let (send, receive) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { break };
                if send.send(line).is_err() {
                    break;
                }
            }
        });
        let mut current: Option<u64> = None;
        let mut current_stage = String::new();
        let mut failure: Option<&str> = None;
        loop {
            match receive.recv_timeout(CASE_TIMEOUT) {
                Ok(line) => {
                    if let Some(case) = line.strip_prefix("start ") {
                        current = case.parse().ok();
                        current_stage.clear();
                    } else if let Some(name) = line.strip_prefix("stage ") {
                        current_stage = name.to_owned();
                    } else if let Some(rest) = line.strip_prefix("done ") {
                        let (case, outcome) = rest.split_once(' ').unwrap_or((rest, ""));
                        let case: u64 = case.parse().unwrap_or(0);
                        let kind = if outcome.starts_with("panic") {
                            findings.push((case, outcome.to_owned()));
                            "panic"
                        } else {
                            outcome
                        };
                        *tally.entry(kind.to_owned()).or_default() += 1;
                        current = None;
                        next = case + 1;
                    }
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    let _ = child.kill();
                    failure = Some("hang");
                    break;
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    if current.is_some() {
                        failure = Some("crash");
                    }
                    break;
                }
            }
        }
        let _ = child.wait();
        if let (Some(kind), Some(case)) = (failure, current) {
            *tally.entry(kind.to_owned()).or_default() += 1;
            findings.push((case, format!("{kind} in {current_stage}")));
            next = case + 1;
        } else if failure.is_none() && current.is_none() && next < cases {
            // The child ended between cases without finishing; go on.
            if next == 0 {
                eprintln!("hexicon-fuzz: the child could not start");
                return ExitCode::FAILURE;
            }
        }
    }
    for (case, what) in &findings {
        let ext = if mode == "saves" { "sav" } else { "t3" };
        let path = format!("{out_dir}/case-{case}.{ext}");
        if let Some(data) = case_input(mode, seed, *case, files) {
            let _ = std::fs::write(&path, data);
        }
        println!("case {case}: {what} ({path})");
    }
    for (kind, count) in &tally {
        println!("{count:7}  {kind}");
    }
    println!(
        "{cases} cases in {:.1} s, {} findings",
        started.elapsed().as_secs_f64(),
        findings.len()
    );
    if findings.iter().any(|(_, what)| !what.starts_with("hang")) {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let usage = "usage: hexicon-fuzz images|saves <out-dir> <cases> <seed> <file>...";
    match args.first().map(String::as_str) {
        Some("child") if args.len() >= 6 => {
            let (Ok(seed), Ok(first), Ok(last)) =
                (args[2].parse(), args[3].parse(), args[4].parse())
            else {
                return ExitCode::FAILURE;
            };
            child(&args[1], seed, first, last, &args[5..])
        }
        Some(mode @ ("images" | "saves")) if args.len() >= 5 => {
            let (Ok(cases), Ok(seed)) = (args[2].parse(), args[3].parse()) else {
                eprintln!("{usage}");
                return ExitCode::FAILURE;
            };
            parent(mode, &args[1], cases, seed, &args[4..])
        }
        _ => {
            eprintln!("{usage}");
            ExitCode::FAILURE
        }
    }
}
