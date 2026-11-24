//! `hexicon convert`: a TADS 3 game or library, a folder of it, made into
//! a Hexicon one in a new folder.
//!
//! Source files (`.t`) become Hexicon sources (`.hxn`) and headers stay
//! headers, each with Hexicon's names for the built-in functions, and
//! with Hexicon's declarations for adv3Lite's actions, handlers, and
//! verb rules.
//! Hexicon sources (`.hxn`) get the same, which brings a Hexicon
//! program written before a declaration was added up to date. A
//! project file (`.t3m`) becomes a game's manifest: `hexicon.toml` if it
//! is the only one in its folder, and `<name>.toml` otherwise. A library
//! (`.tl`) becomes `<name>.toml` beside where it was, which is where a
//! library of its name is looked for. A manifest already there keeps
//! its text, but a TADS 3 source it names that becomes a `.hxn` file is
//! named without its `.t`. Everything else is copied as it is.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use hexicon_compiler::convert;

/// The files under `dir`, as paths relative to it, in a steady order.
fn files_under(dir: &Path) -> std::io::Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    let mut stack = vec![PathBuf::new()];
    while let Some(rel) = stack.pop() {
        let mut entries: Vec<_> = std::fs::read_dir(dir.join(&rel))?.collect::<Result<_, _>>()?;
        entries.sort_by_key(std::fs::DirEntry::file_name);
        for entry in entries {
            let path = rel.join(entry.file_name());
            if entry.file_type()?.is_dir() {
                stack.push(path);
            } else {
                out.push(path);
            }
        }
    }
    out.sort();
    Ok(out)
}

fn ext_of(path: &Path) -> String {
    path.extension()
        .map(|ext| ext.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default()
}

pub(crate) fn run(args: &[&str], usage: &str) -> ExitCode {
    let [from, to] = args else {
        eprint!("{usage}");
        return ExitCode::from(2);
    };
    match convert_folder(Path::new(from), Path::new(to)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("hexicon: {message}");
            ExitCode::FAILURE
        }
    }
}

fn convert_folder(from: &Path, to: &Path) -> Result<(), String> {
    if !from.is_dir() {
        return Err(format!("{}: not a folder", from.display()));
    }
    if to.exists()
        && std::fs::read_dir(to)
            .map(|mut entries| entries.next().is_some())
            .unwrap_or(true)
    {
        return Err(format!(
            "{}: already exists and is not empty; convert into a new folder",
            to.display()
        ));
    }
    let files = files_under(from).map_err(|e| format!("{}: {e}", from.display()))?;
    let read = |rel: &Path| {
        std::fs::read(from.join(rel)).map_err(|e| format!("{}: {e}", from.join(rel).display()))
    };

    // The names the program's code already uses decide which renames
    // can be made.
    let mut used = HashSet::new();
    for rel in &files {
        if matches!(ext_of(rel).as_str(), "t" | "h" | "hxn") {
            used.extend(convert::code_names(&read(rel)?));
        }
    }
    let (renames, skipped) = convert::usable_renames(&used);
    for note in &skipped {
        eprintln!("note: {note}");
    }

    // How many project files each folder has.
    let mut projects_in: std::collections::HashMap<PathBuf, usize> = Default::default();
    for rel in &files {
        if ext_of(rel) == "t3m" {
            *projects_in.entry(parent_of(rel)).or_default() += 1;
        }
    }
    let sources: HashSet<String> = files
        .iter()
        .filter(|rel| ext_of(rel) == "t")
        .map(|rel| key_of(rel))
        .collect();

    let (mut converted, mut renamed, mut declared, mut copied) = (0, 0, 0, 0);
    for rel in &files {
        let ext = ext_of(rel);
        let text = read(rel)?;
        let folder = parent_of(rel);
        // Whether a source a project or library in `folder` names is
        // one of the files being converted.
        let mut is_local = |name: &str| {
            let path = folder.join(name.replace('\\', "/"));
            sources.contains(&key_of(&normalize(&path)))
        };
        let (out_rel, data) = match ext.as_str() {
            "t" | "h" | "hxn" => {
                let result = convert::convert_source(&text, &renames);
                renamed += result.renamed;
                declared += result.declarations;
                converted += 1;
                let out_rel = if ext == "t" {
                    rel.with_extension("hxn")
                } else {
                    rel.clone()
                };
                (out_rel, result.text)
            }
            "t3m" => {
                let stem = file_stem(rel);
                let result = convert::convert_project(&text, &stem, &mut is_local);
                for note in &result.notes {
                    eprintln!("note: {}: {note}", rel.display());
                }
                converted += 1;
                let out_rel = if projects_in.get(&folder) == Some(&1) {
                    folder.join(hexicon_compiler::MANIFEST)
                } else {
                    rel.with_extension("toml")
                };
                (out_rel, result.text.into_bytes())
            }
            "toml" => {
                // A manifest already: the sources it names that become
                // .hxn files are named without their .t.
                let text = String::from_utf8_lossy(&text);
                let (text, changed) = convert::convert_manifest(&text, &mut is_local);
                if changed > 0 {
                    converted += 1;
                } else {
                    copied += 1;
                }
                (rel.clone(), text.into_bytes())
            }
            "tl" => {
                let result = convert::convert_library(&text, &mut is_local);
                for note in &result.notes {
                    eprintln!("note: {}: {note}", rel.display());
                }
                converted += 1;
                (rel.with_extension("toml"), result.text.into_bytes())
            }
            _ => {
                copied += 1;
                (rel.clone(), text)
            }
        };
        let dest = to.join(&out_rel);
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
        }
        if dest.exists() {
            return Err(format!(
                "{}: two files would be written here",
                dest.display()
            ));
        }
        std::fs::write(&dest, data).map_err(|e| format!("{}: {e}", dest.display()))?;
    }
    eprintln!(
        "converted {converted} files ({renamed} names renamed, {declared} actions, handlers, \
         and verb rules declared), copied {copied}, into {}",
        to.display()
    );
    Ok(())
}

fn parent_of(rel: &Path) -> PathBuf {
    rel.parent().map(Path::to_path_buf).unwrap_or_default()
}

fn file_stem(rel: &Path) -> String {
    rel.file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// A relative path with `.` and `..` worked out.
fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for part in path.components() {
        match part {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                out.pop();
            }
            other => out.push(other),
        }
    }
    out
}

/// A relative path as a key that ignores case and the kind of slash, as
/// the file systems the games come from do.
fn key_of(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/").to_lowercase()
}
