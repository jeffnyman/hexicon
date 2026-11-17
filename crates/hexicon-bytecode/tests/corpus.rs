//! Disassembles every image file in a directory of real, compiler-produced
//! images and checks that the result accounts for the whole code pool.
//!
//! The images are not part of this repository. Set `HEXICON_T3_CORPUS` to
//! one or more directories (separated like `PATH`) to run this test; it is
//! skipped otherwise. `tools/reference/run-suite.sh` leaves a suitable
//! directory at `build/reference-suite/test/out`.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use hexicon_bytecode::{Extent, discover, discover_without_debug_info};
use hexicon_image::Image;

fn collect(dir: &Path, found: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        panic!("cannot read corpus directory {}", dir.display());
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect(&path, found);
        } else if path.extension().is_some_and(|e| e == "t3") {
            found.push(path);
        }
    }
}

/// Check one image, returning a description of anything wrong.
fn check(image: &Image) -> Result<(usize, usize), String> {
    let found = discover(image);
    if let Some((address, error)) = found.errors().next() {
        return Err(format!("function {address:#x}: {error}"));
    }

    // Functions must not overlap, and together they must be the whole code
    // pool: every byte is a header, an instruction, or a table.
    let mut covered = 0usize;
    let mut previous_end = 0u32;
    let mut instructions = 0usize;
    for function in found.functions() {
        if function.address < previous_end {
            return Err(format!(
                "function {:#x} overlaps the one before",
                function.address
            ));
        }
        if let Extent::Gap { len, .. } = function.extent {
            return Err(format!(
                "{len} bytes after function {:#x} were not decoded",
                function.address
            ));
        }
        previous_end = function.tail_address();
        covered += function.size as usize;
        instructions += function.instructions.len();
    }
    let total = image.code_pool.data_len();
    if covered != total {
        return Err(format!("functions cover {covered} of {total} code bytes"));
    }

    // A debug build lists its functions. Everything on the list must have
    // been found, with or without the list's help.
    let listed: BTreeSet<u32> = image.debug.method_headers.iter().copied().collect();
    if !listed.is_empty() {
        let unaided = discover_without_debug_info(image);
        let missed = listed
            .iter()
            .filter(|address| !unaided.by_address.contains_key(address))
            .count();
        // Functions nothing refers to cannot be found by following
        // references, and in a debug build they are not adjacent to bytes
        // that give them away. Allow a few.
        if missed * 20 > listed.len() {
            return Err(format!(
                "{missed} of {} listed functions were not found without the list",
                listed.len()
            ));
        }
    }
    Ok((found.by_address.len(), instructions))
}

#[test]
fn disassembles_every_image_in_the_corpus() {
    let Some(dirs) = std::env::var_os("HEXICON_T3_CORPUS") else {
        eprintln!("HEXICON_T3_CORPUS is not set; skipping");
        return;
    };
    let mut files = Vec::new();
    for dir in std::env::split_paths(&dirs) {
        collect(&dir, &mut files);
    }
    assert!(!files.is_empty(), "no .t3 files found in HEXICON_T3_CORPUS");

    let mut failures = Vec::new();
    let mut functions = 0usize;
    let mut instructions = 0usize;
    for path in &files {
        let data = std::fs::read(path).unwrap();
        let image = match Image::parse(&data) {
            Ok(image) => image,
            Err(error) => {
                failures.push(format!("{}: {error}", path.display()));
                continue;
            }
        };
        match check(&image) {
            Ok((f, i)) => {
                functions += f;
                instructions += i;
            }
            Err(problem) => failures.push(format!("{}: {problem}", path.display())),
        }
    }
    eprintln!(
        "disassembled {} images: {functions} functions, {instructions} instructions, {} failed",
        files.len(),
        failures.len()
    );
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
