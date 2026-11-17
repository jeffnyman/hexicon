//! Parses every image file in a directory of real, compiler-produced images.
//!
//! The images are not part of this repository. Set `HEXICON_T3_CORPUS` to
//! one or more directories (separated like `PATH`) to run this test; it is
//! skipped otherwise. `tools/reference/run-suite.sh` leaves a suitable
//! directory at `build/reference-suite/test/out`.

use std::path::{Path, PathBuf};

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

#[test]
fn parses_every_image_in_the_corpus() {
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
    for path in &files {
        let data = std::fs::read(path).unwrap();
        match Image::parse(&data) {
            Ok(image) => {
                // The entry point must lie in code the image supplies.
                if image.code_pool.at(image.entry_point.code_offset).is_none() {
                    failures.push(format!(
                        "{}: entry point is outside the code pool",
                        path.display()
                    ));
                }
            }
            Err(error) => failures.push(format!("{}: {error}", path.display())),
        }
    }
    eprintln!("parsed {} images, {} failed", files.len(), failures.len());
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
