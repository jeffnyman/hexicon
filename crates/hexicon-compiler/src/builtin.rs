//! The libraries built into the compiler: Hexicon's system library (the
//! program's entry point, the classes every program is built on, and the
//! headers that declare the virtual machine's built-in functions and
//! classes) and Hexad. Their files are in `lib/system` and `lib/hexad`,
//! and are read from there when the compiler is built, with the
//! `built-in-libraries` feature, which is on unless turned off. (The
//! virtual machine, which compiles code while a program runs but builds
//! no games, turns it off.)
//!
//! A build looks for libraries and headers where it is told to, and then
//! here, in the folder that messages and the image call [`ROOT`]. So a
//! game builds with nothing else installed, and a build told to use other
//! files (`-FL`, `-FI`, `-Fs`, a manifest's `library-paths`) uses those.

use crate::pp::SourceFiles;

/// Where the built-in files are said to be: each library in a folder of
/// its own name there.
pub const ROOT: &str = "<hexicon>";

/// Where the built-in system library and its headers are said to be.
pub const SYSTEM: &str = "<hexicon>/system";

/// The built-in files, by their paths in [`ROOT`] (`system/tads.h`,
/// `hexad/english/english.hxn`).
pub(crate) const FILES: &[(&str, &[u8])] = include!(concat!(env!("OUT_DIR"), "/built_in_files.rs"));

/// A path in [`ROOT`] as the table has it: after the root, with `/`.
fn in_root(path: &str) -> Option<String> {
    let path = path.replace('\\', "/");
    path.strip_prefix(ROOT)?
        .strip_prefix('/')
        .map(str::to_owned)
}

/// A built-in file, by its path (`<hexicon>/system/tads.h`, with either
/// kind of slash).
pub(crate) fn file(path: &str) -> Option<&'static [u8]> {
    let name = in_root(path)?;
    FILES
        .iter()
        .find(|(have, _)| have.eq_ignore_ascii_case(&name))
        .map(|(_, data)| *data)
}

/// A system header built in, by its name (`tads.h`).
pub(crate) fn system_header(name: &str) -> Option<&'static [u8]> {
    file(&format!("{SYSTEM}/{name}"))
}

/// Whether a path is in [`ROOT`].
pub(crate) fn is_built_in(path: &str) -> bool {
    path.replace('\\', "/").starts_with(ROOT)
}

/// Whether a built-in path is a folder: one that a built-in file is in.
fn is_folder(path: &str) -> bool {
    match in_root(path) {
        Some(name) => {
            let name = name.trim_end_matches('/');
            FILES.iter().any(|(have, _)| {
                have.len() > name.len()
                    && have[..name.len()].eq_ignore_ascii_case(name)
                    && have.as_bytes()[name.len()] == b'/'
            })
        }
        None => path.replace('\\', "/") == ROOT && !FILES.is_empty(),
    }
}

/// Files as a build sees them: the built-in ones in [`ROOT`], and the
/// rest as `files` has them.
pub(crate) struct WithBuiltIns<'f> {
    files: &'f mut dyn SourceFiles,
}

impl<'f> WithBuiltIns<'f> {
    pub(crate) fn new(files: &'f mut dyn SourceFiles) -> Self {
        Self { files }
    }
}

impl SourceFiles for WithBuiltIns<'_> {
    fn read(&mut self, name: &str) -> Option<Vec<u8>> {
        if is_built_in(name) {
            return file(name).map(<[u8]>::to_vec);
        }
        self.files.read(name)
    }

    fn exists(&mut self, name: &str) -> bool {
        if is_built_in(name) {
            return file(name).is_some() || is_folder(name);
        }
        self.files.exists(name)
    }

    fn decode(&mut self, charset: &str, bytes: &[u8]) -> Option<Vec<u8>> {
        self.files.decode(charset, bytes)
    }
}
