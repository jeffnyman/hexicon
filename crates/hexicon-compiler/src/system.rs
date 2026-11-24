//! Hexicon's system library, built into the compiler: the program's
//! entry point and the classes every program is built on, and the headers
//! that declare the virtual machine's built-in functions and classes. The
//! files are in `lib/system`, and are read from there when the compiler is
//! built.
//!
//! A build looks for the system library and its headers where it is told
//! to, and then here, in the folder that messages and the image call
//! [`FOLDER`]. So a game builds with nothing else installed, and a build
//! told to use another system library (`-FL`, `-FI`) still uses it.

use crate::pp::SourceFiles;

/// Where the built-in files are said to be.
pub const FOLDER: &str = "<hexicon>/system";

/// The built-in files, by name.
pub(crate) const FILES: &[(&str, &[u8])] = &[
    ("_main.t", include_bytes!("../../../lib/system/_main.t")),
    ("array.h", include_bytes!("../../../lib/system/array.h")),
    ("bignum.h", include_bytes!("../../../lib/system/bignum.h")),
    ("bytearr.h", include_bytes!("../../../lib/system/bytearr.h")),
    ("charset.h", include_bytes!("../../../lib/system/charset.h")),
    ("date.h", include_bytes!("../../../lib/system/date.h")),
    ("dict.h", include_bytes!("../../../lib/system/dict.h")),
    ("dynfunc.h", include_bytes!("../../../lib/system/dynfunc.h")),
    ("dynfunc.t", include_bytes!("../../../lib/system/dynfunc.t")),
    ("file.h", include_bytes!("../../../lib/system/file.h")),
    ("file.t", include_bytes!("../../../lib/system/file.t")),
    (
        "filename.h",
        include_bytes!("../../../lib/system/filename.h"),
    ),
    (
        "gramprod.h",
        include_bytes!("../../../lib/system/gramprod.h"),
    ),
    (
        "gramprod.t",
        include_bytes!("../../../lib/system/gramprod.t"),
    ),
    ("hexicon.h", include_bytes!("../../../lib/system/hexicon.h")),
    ("httpreq.h", include_bytes!("../../../lib/system/httpreq.h")),
    ("httpsrv.h", include_bytes!("../../../lib/system/httpsrv.h")),
    ("lookup.h", include_bytes!("../../../lib/system/lookup.h")),
    (
        "multmeth.t",
        include_bytes!("../../../lib/system/multmeth.t"),
    ),
    ("reflect.h", include_bytes!("../../../lib/system/reflect.h")),
    ("strbuf.h", include_bytes!("../../../lib/system/strbuf.h")),
    ("strcomp.h", include_bytes!("../../../lib/system/strcomp.h")),
    (
        "system.toml",
        include_bytes!("../../../lib/system/system.toml"),
    ),
    ("systype.h", include_bytes!("../../../lib/system/systype.h")),
    ("t3.h", include_bytes!("../../../lib/system/t3.h")),
    ("t3test.h", include_bytes!("../../../lib/system/t3test.h")),
    ("tads.h", include_bytes!("../../../lib/system/tads.h")),
    ("tadsgen.h", include_bytes!("../../../lib/system/tadsgen.h")),
    ("tadsio.h", include_bytes!("../../../lib/system/tadsio.h")),
    ("tadsiox.h", include_bytes!("../../../lib/system/tadsiox.h")),
    ("tadsnet.h", include_bytes!("../../../lib/system/tadsnet.h")),
    ("tok.h", include_bytes!("../../../lib/system/tok.h")),
    ("tok.t", include_bytes!("../../../lib/system/tok.t")),
    ("vector.h", include_bytes!("../../../lib/system/vector.h")),
];

/// A built-in file: by its name (`tads.h`), or by its path in
/// [`FOLDER`] (`<hexicon>/system/tads.h`, with either kind of slash).
pub(crate) fn file(name: &str) -> Option<&'static [u8]> {
    let name = name.replace('\\', "/");
    let name = name
        .strip_prefix(FOLDER)
        .and_then(|rest| rest.strip_prefix('/'))
        .unwrap_or(&name);
    FILES
        .iter()
        .find(|(have, _)| have.eq_ignore_ascii_case(name))
        .map(|(_, data)| *data)
}

/// Whether a path is in [`FOLDER`].
pub(crate) fn in_folder(path: &str) -> bool {
    path.replace('\\', "/").starts_with(FOLDER)
}

/// Files as a build sees them: the built-in ones in [`FOLDER`], and the
/// rest as `files` has them.
pub(crate) struct WithSystem<'f> {
    files: &'f mut dyn SourceFiles,
}

impl<'f> WithSystem<'f> {
    pub(crate) fn new(files: &'f mut dyn SourceFiles) -> Self {
        Self { files }
    }
}

impl SourceFiles for WithSystem<'_> {
    fn read(&mut self, name: &str) -> Option<Vec<u8>> {
        if in_folder(name) {
            return file(name).map(<[u8]>::to_vec);
        }
        self.files.read(name)
    }

    fn exists(&mut self, name: &str) -> bool {
        if in_folder(name) {
            return file(name).is_some();
        }
        self.files.exists(name)
    }

    fn decode(&mut self, charset: &str, bytes: &[u8]) -> Option<Vec<u8>> {
        self.files.decode(charset, bytes)
    }
}
