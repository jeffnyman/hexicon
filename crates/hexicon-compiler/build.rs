//! Make the table of the libraries built into the compiler: the files of
//! `lib/system` and `lib/hexad`, when the `built-in-libraries` feature is
//! on, and none otherwise.

use std::fmt::Write as _;
use std::path::Path;

/// The libraries built in, by their folders in `lib`.
const LIBRARIES: [&str; 2] = ["system", "hexad"];

/// The kinds of file a build reads.
const KINDS: [&str; 4] = ["h", "t", "hxn", "toml"];

fn collect(dir: &Path, prefix: &str, out: &mut Vec<(String, String)>) {
    let mut entries: Vec<_> = std::fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("{}: {e}", dir.display()))
        .map(|entry| entry.unwrap().path())
        .collect();
    entries.sort();
    for path in entries {
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        if path.is_dir() {
            collect(&path, &format!("{prefix}/{name}"), out);
        } else if path
            .extension()
            .is_some_and(|ext| KINDS.contains(&ext.to_string_lossy().as_ref()))
        {
            let full = path.canonicalize().unwrap();
            out.push((
                format!("{prefix}/{name}"),
                full.to_string_lossy().into_owned(),
            ));
        }
    }
}

fn main() {
    let manifest = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let lib = Path::new(&manifest).join("../../lib");
    let mut files = Vec::new();
    if std::env::var_os("CARGO_FEATURE_BUILT_IN_LIBRARIES").is_some() {
        for library in LIBRARIES {
            let dir = lib.join(library);
            println!("cargo:rerun-if-changed={}", dir.display());
            collect(&dir, library, &mut files);
        }
    }
    let mut code = String::from("&[\n");
    for (name, path) in files {
        let _ = writeln!(code, "    ({name:?}, include_bytes!({path:?})),");
    }
    code.push_str("]\n");
    let out = Path::new(&std::env::var("OUT_DIR").unwrap()).join("built_in_files.rs");
    std::fs::write(out, code).unwrap();
    println!("cargo:rerun-if-changed=build.rs");
}
