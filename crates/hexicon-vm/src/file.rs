//! The File, FileName, and TemporaryFile intrinsic classes.
//!
//! A file name is text in one syntax on every host: parts separated by
//! `/`, with `.` for a folder itself and `..` for the one above it. The
//! reference VM calls this the universal syntax and keeps names in the
//! local system's; here the two are the same, and the host translates when
//! it touches a real file system.
//!
//! An open file is held in memory whole. Opening one reads it from the
//! host, and closing one writes it back if it changed. That keeps the
//! host's part small enough for a browser to supply.

use std::rc::Rc;

use hexicon_host::{FileError, Sandbox, file_type};

/// How the system ends a line in a text file.
const NEWLINE: &str = if cfg!(windows) { "\r\n" } else { "\n" };

use crate::error::{ErrParam, VmError, VmResult, code};
use crate::hash;
use crate::object::Object;
use crate::pack::Stream;
use crate::value::{INVALID_PROP, NativeDesc, ObjId, Value};
use crate::vm::{NativeCall, Vm};

// How a file is read and written.
pub(crate) const MODE_TEXT: u8 = 1;
pub(crate) const MODE_DATA: u8 = 2;
pub(crate) const MODE_RAW: u8 = 3;

/// What a program wants to do with a file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Access {
    Read,
    Write,
    ReadWriteKeep,
    ReadWriteTruncate,
    Delete,
    GetInfo,
    MakeDir,
    RemoveDir,
    ReadDir,
    RenameFrom,
    RenameTo,
}

impl Access {
    /// The access a program asks for by number when it opens a file.
    fn from_code(code: i32) -> Option<Self> {
        Some(match code {
            1 => Self::Read,
            2 => Self::Write,
            3 => Self::ReadWriteKeep,
            4 => Self::ReadWriteTruncate,
            _ => return None,
        })
    }

    fn code(self) -> u8 {
        match self {
            Self::Read => 1,
            Self::Write => 2,
            Self::ReadWriteKeep => 3,
            _ => 4,
        }
    }
}

// The values a data file marks its contents with.
const TAG_INT: u8 = 0x01;
const TAG_STRING: u8 = 0x03;
const TAG_TRUE: u8 = 0x08;
const TAG_ENUM: u8 = 0x20;
const TAG_BIGNUM: u8 = 0x21;
const TAG_BYTEARRAY: u8 = 0x22;

/// A file name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FileNameObj {
    /// The name. For a special file, where the host keeps it, if it said.
    pub path: String,
    /// The number of a special file, or zero.
    pub special: i32,
    /// How the player chose the name in a file dialog: 1 to open a file,
    /// 2 to save one, or zero if the name came from the program. A file
    /// the player chose can be used wherever it is.
    pub from_ui: u8,
}

/// The contents of an open file.
#[derive(Debug, Clone)]
pub(crate) struct OpenFile {
    /// The name to write the file back under. A resource has none.
    pub name: Option<String>,
    pub data: Vec<u8>,
    pub pos: usize,
    /// The contents differ from what the host has.
    pub dirty: bool,
}

impl OpenFile {
    /// The bytes from the position on, at most `len` of them, moving
    /// past them.
    fn read(&mut self, len: usize) -> &[u8] {
        let start = self.pos.min(self.data.len());
        let end = start.saturating_add(len).min(self.data.len());
        if end > start {
            self.pos = end;
        }
        &self.data[start..end]
    }

    /// Exactly `len` bytes, or nothing if there are not that many.
    fn read_exact(&mut self, len: usize) -> Option<Vec<u8>> {
        let start = self.pos.min(self.data.len());
        let bytes = self.data.get(start..start.checked_add(len)?)?.to_vec();
        self.pos = start + len;
        Some(bytes)
    }

    fn write(&mut self, bytes: &[u8]) {
        // Writing past the end leaves zeros in between.
        if self.pos > self.data.len() {
            self.data.resize(self.pos, 0);
        }
        let end = self.pos + bytes.len();
        if end > self.data.len() {
            self.data.resize(end, 0);
        }
        self.data[self.pos..end].copy_from_slice(bytes);
        self.pos = end;
        self.dirty = true;
    }
}

/// A file object.
#[derive(Debug, Clone)]
pub(crate) struct FileObj {
    /// The character set of a text file, or the invalid ID.
    pub charset: ObjId,
    pub mode: u8,
    pub access: u8,
    /// The object came from an image or a saved state, so it has no file.
    pub out_of_sync: bool,
    /// What the program gave to say which file to open.
    pub spec: Value,
    /// The contents, while the file is open.
    pub open: Option<OpenFile>,
}

/// Which file a program means, once its file specification is worked out.
#[derive(Debug, Clone)]
pub(crate) struct FileTarget {
    /// The name to give the host. `None` for a special file the host has
    /// no place for.
    pub name: Option<String>,
    special: bool,
    temporary: bool,
    from_ui: u8,
    spec: Value,
}

/// The exceptions the file classes throw. A program that does not define
/// the class gets a run-time error with the message instead.
#[derive(Debug, Clone, Copy)]
enum FileExc {
    NotFound,
    Creation,
    Open,
    OpenDir,
    Io,
    Sync,
    Closed,
    Mode,
    Safety,
}

// ---- names ----

/// Whether a name starts at a root: with `/`, or with a drive letter on a
/// system that has them.
pub(crate) fn is_absolute(path: &str) -> bool {
    let bytes = path.as_bytes();
    path.starts_with('/')
        || (bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':')
}

/// The last part of a name.
pub(crate) fn root_name(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

/// A name without its last part, or nothing if it has only one.
fn path_name(path: &str) -> &str {
    match path.rfind('/') {
        None => "",
        Some(at) => {
            let before = path[..at].trim_end_matches('/');
            if before.is_empty() { "/" } else { before }
        }
    }
}

/// Take `.` and empty parts out of a name, and each `..` with the part
/// before it.
fn canonicalize(path: &str) -> String {
    let mut parts: Vec<&str> = Vec::new();
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." if parts.last().is_some_and(|&last| last != "..") => {
                parts.pop();
            }
            _ => parts.push(part),
        }
    }
    let mut out = String::new();
    if path.starts_with('/') {
        out.push('/');
    }
    out.push_str(&parts.join("/"));
    if out.is_empty() {
        out.push('.');
    }
    out
}

/// A name in a folder. A name that starts at a root stays as it is. A
/// literal combination keeps `.` and `..` parts as they are.
pub(crate) fn combine(dir: &str, name: &str, literal: bool) -> String {
    let joined = if is_absolute(name) || dir.is_empty() {
        name.to_owned()
    } else if dir.ends_with('/') {
        format!("{dir}{name}")
    } else {
        format!("{dir}/{name}")
    };
    if literal {
        joined
    } else {
        canonicalize(&joined)
    }
}

/// The type flags that say a name is a folder's entry for itself or for
/// its parent.
fn special_link_flags(path: &str) -> i32 {
    match root_name(path) {
        "." => 0x80,
        ".." => 0x100,
        _ => 0,
    }
}

const FILE_METHODS: [NativeDesc; 25] = [
    NativeDesc::fixed(0),
    NativeDesc::optional(2, 1), // openTextFile
    NativeDesc::fixed(2),       // openDataFile
    NativeDesc::fixed(2),       // openRawFile
    NativeDesc::fixed(0),       // getCharacterSet
    NativeDesc::fixed(1),       // setCharacterSet
    NativeDesc::fixed(0),       // closeFile
    NativeDesc::fixed(0),       // readFile
    NativeDesc::fixed(1),       // writeFile
    NativeDesc::optional(1, 2), // readBytes
    NativeDesc::optional(1, 2), // writeBytes
    NativeDesc::fixed(0),       // getPos
    NativeDesc::fixed(1),       // setPos
    NativeDesc::fixed(0),       // setPosEnd
    NativeDesc::optional(1, 1), // openTextResource
    NativeDesc::fixed(1),       // openRawResource
    NativeDesc::fixed(0),       // getFileSize
    NativeDesc::fixed(0),       // getFileMode
    NativeDesc::fixed(1),       // getRootName
    NativeDesc::fixed(1),       // deleteFile
    NativeDesc::optional(1, 1), // setFileMode
    NativeDesc::variable(1),    // packBytes
    NativeDesc::fixed(1),       // unpackBytes
    NativeDesc::optional(0, 1), // sha256
    NativeDesc::optional(0, 1), // digestMD5
];

const FILE_NAME_METHODS: [NativeDesc; 18] = [
    NativeDesc::fixed(0),
    NativeDesc::fixed(0),       // getName
    NativeDesc::fixed(0),       // getBaseName
    NativeDesc::fixed(0),       // getPath
    NativeDesc::fixed(1),       // fromUniversal
    NativeDesc::fixed(0),       // toUniversal
    NativeDesc::fixed(1),       // addToPath
    NativeDesc::fixed(0),       // isAbsolute
    NativeDesc::fixed(0),       // getAbsolutePath
    NativeDesc::fixed(0),       // getRootDirs
    NativeDesc::fixed(0),       // getFileType
    NativeDesc::fixed(0),       // getFileInfo
    NativeDesc::fixed(0),       // deleteFile
    NativeDesc::fixed(1),       // renameFile
    NativeDesc::fixed(0),       // listDir
    NativeDesc::optional(1, 1), // forEachFile
    NativeDesc::optional(0, 1), // createDirectory
    NativeDesc::optional(0, 1), // removeDirectory
];

impl Vm {
    // ---- exceptions ----

    /// The error for throwing a new instance of an exception class, or a
    /// run-time error with the message if the program has no such class.
    pub(crate) fn throw_class(&mut self, class: ObjId, message: &str) -> VmError {
        if class.is_valid() {
            return match self.create_instance(class, 0) {
                Err(error) => error,
                Ok(()) => match self.r0 {
                    Value::Obj(exception) => VmError::Thrown(exception),
                    _ => VmError::new(code::EXCEPTION_OBJ_REQD),
                },
            };
        }
        VmError::with(
            code::INTCLS_GENERAL_ERROR,
            vec![ErrParam::Str(message.to_owned())],
        )
    }

    /// A resource that is not in the image is looked for as a file beside
    /// it, which is how a game under development gets at resources that
    /// have not been bundled yet, and how some games read back files they
    /// wrote. Only a file in the image's folder or below it will do,
    /// whatever the safety level, unless that allows no reading at all.
    pub(crate) fn resource_file(&mut self, name: Option<&str>) -> Option<Vec<u8>> {
        let name = name?;
        if self.io.safety_read > 3
            || name.is_empty()
            || is_absolute(name)
            || name.split('/').any(|part| part == "..")
        {
            return None;
        }
        self.host.read_file(name).ok()
    }

    fn file_exc(&mut self, which: FileExc) -> VmError {
        let predef = &self.predef;
        let (class, message) = match which {
            FileExc::NotFound => (predef.file_not_found_exc, "file not found"),
            FileExc::Creation => (predef.file_creation_exc, "error creating file"),
            FileExc::Open => (predef.file_open_exc, "error opening file"),
            FileExc::OpenDir => (predef.file_open_exc, "error reading directory"),
            FileExc::Io => (predef.file_io_exc, "file I/O error"),
            FileExc::Sync => (predef.file_sync_exc, "file out of sync"),
            FileExc::Closed => (predef.file_closed_exc, "operation attempted on closed file"),
            FileExc::Mode => (predef.file_mode_exc, "wrong file mode"),
            FileExc::Safety => (predef.file_safety_exc, "prohibited file access"),
        };
        self.throw_class(class, message)
    }

    // ---- which file ----

    /// A FileName object for a name.
    pub(crate) fn new_file_name(&mut self, path: &str) -> ObjId {
        self.alloc_obj(Object::FileName(Box::new(FileNameObj {
            path: path.to_owned(),
            special: 0,
            from_ui: 0,
        })))
    }

    /// Note that the player chose a file name in a dialog.
    pub(crate) fn set_file_name_from_ui(&mut self, id: ObjId, dialog: u8) {
        if let Some(Object::FileName(name)) = self.objects.get_mut(id) {
            name.from_ui = dialog;
        }
    }

    fn file_name(&self, id: ObjId) -> Option<&FileNameObj> {
        match self.objects.get(id) {
            Some(Object::FileName(name)) => Some(name),
            _ => None,
        }
    }

    /// The name a value gives: a FileName's, or the text of a string.
    pub(crate) fn path_of(&self, value: Value) -> Option<String> {
        if let Value::Obj(id) = value {
            if let Some(name) = self.file_name(id) {
                return Some(name.path.clone());
            }
        }
        self.as_str(value)
            .map(|text| String::from_utf8_lossy(&text).into_owned())
    }

    /// Where the host keeps a special file. Only two numbers mean
    /// anything.
    fn special_file_path(&mut self, special: i32) -> VmResult<Option<String>> {
        match special {
            1 | 2 => Ok(self.host.special_file(special)),
            _ => Err(VmError::new(code::BAD_VAL_BIF)),
        }
    }

    /// Work out which file a value refers to: a name, a FileName, a
    /// TemporaryFile, the number of a special file, or an object of the
    /// program's own that supplies one of those.
    pub(crate) fn file_target(&mut self, value: Value) -> VmResult<FileTarget> {
        let mut spec = value;
        if let Value::Obj(id) = value {
            let get_filename = self.predef.filespec_get_filename;
            if get_filename != INVALID_PROP && matches!(self.objects.get(id), Some(Object::Tads(_)))
            {
                let native = NativeCall::named("FileSpec.getFilename");
                spec = self.call_prop(value, get_filename, 0, Some(native))?;
            }
        }
        let keep = match value {
            Value::Obj(_) => value,
            _ => Value::Nil,
        };
        let mut target = FileTarget {
            name: None,
            special: false,
            temporary: false,
            from_ui: 0,
            spec: keep,
        };
        match spec {
            Value::Obj(id) => match self.objects.get(id) {
                Some(Object::TempFile(name)) => {
                    if name.is_empty() {
                        return Err(VmError::new(code::CREATE_FILE));
                    }
                    target.name = Some(name.clone());
                    target.temporary = true;
                    return Ok(target);
                }
                Some(Object::FileName(name)) => {
                    if name.special != 0 {
                        let special = name.special;
                        target.name = self.special_file_path(special)?;
                        target.special = true;
                    } else {
                        target.name = Some(name.path.clone());
                        target.from_ui = name.from_ui;
                    }
                    return Ok(target);
                }
                Some(Object::BigNumber(_)) => {
                    let special = self.cast_to_int(spec)?;
                    target.name = self.special_file_path(special)?;
                    target.special = true;
                    return Ok(target);
                }
                _ => {}
            },
            Value::Int(special) => {
                target.name = self.special_file_path(special)?;
                target.special = true;
                return Ok(target);
            }
            _ => {}
        }
        match self.as_str(spec) {
            Some(text) => {
                target.name = Some(String::from_utf8_lossy(&text).into_owned());
                Ok(target)
            }
            None => Err(VmError::new(code::BAD_TYPE_BIF)),
        }
    }

    /// Whether the safety levels let the program do something with a
    /// file. Level 0 allows anything, 1 reading anything but writing only
    /// in the sandbox folder, 2 reading and writing only there, 3 only
    /// reading there, and 4 nothing.
    fn safety_allows(&mut self, name: &str, access: Access) -> bool {
        let reading = matches!(access, Access::Read | Access::GetInfo | Access::ReadDir);
        if reading {
            if self.io.safety_read <= 1 {
                return true;
            }
            if self.io.safety_read >= 4 {
                return false;
            }
        } else {
            if self.io.safety_write == 0 {
                return true;
            }
            if self.io.safety_write >= 3 {
                return false;
            }
        }
        match self.host.sandbox(name) {
            Sandbox::Inside => true,
            // The folder itself can be looked at and listed, and so much
            // can be learned about the folders it is in.
            Sandbox::Itself => matches!(access, Access::GetInfo | Access::ReadDir),
            Sandbox::Contains => access == Access::GetInfo,
            Sandbox::Outside => false,
        }
    }

    /// Stop the program from doing what the safety levels do not allow.
    pub(crate) fn check_safety(&mut self, target: &FileTarget, access: Access) -> VmResult<()> {
        let opening = matches!(
            access,
            Access::Read | Access::Write | Access::ReadWriteKeep | Access::ReadWriteTruncate
        );
        let looking = matches!(access, Access::GetInfo | Access::ReadDir);
        // The special files and temporary files are the program's to use.
        if target.special && (opening || looking) {
            return Ok(());
        }
        if target.temporary && (opening || looking || access == Access::Delete) {
            return Ok(());
        }
        // So is a file the player picked, for what it was picked for.
        if (target.from_ui == 1 && access == Access::Read)
            || (target.from_ui == 2 && access == Access::Write)
        {
            return Ok(());
        }
        let allowed = match &target.name {
            Some(name) => {
                let name = name.clone();
                self.safety_allows(&name, access)
            }
            None => true,
        };
        if allowed {
            Ok(())
        } else {
            Err(self.file_exc(FileExc::Safety))
        }
    }

    /// The name of the file a value refers to, for a function that reads
    /// or writes a whole file, checked against the safety levels.
    pub(crate) fn file_name_arg(&mut self, value: Value, access: Access) -> VmResult<String> {
        let target = self.file_target(value)?;
        self.check_safety(&target, access)?;
        target
            .name
            .ok_or_else(|| VmError::new(code::FILE_NOT_FOUND))
    }

    /// Set how much of the file system the program may read and write.
    pub fn set_file_safety(&mut self, read: u8, write: u8) {
        self.io.safety_read = read;
        self.io.safety_write = write;
    }

    /// Supply a resource for the program to open by name.
    pub fn add_resource(&mut self, name: &str, data: &[u8]) {
        self.io.resources.insert(name.to_owned(), data.into());
    }

    pub(crate) fn resource_exists(&self, name: &str) -> bool {
        self.io.resources.contains_key(name)
    }

    // ---- open files ----

    /// Write every open file that has changed back to the host, so that
    /// whatever looks at the files next sees what the program wrote.
    pub(crate) fn sync_files(&mut self) {
        let ids = std::mem::take(&mut self.io.open_files);
        let mut still_open = Vec::with_capacity(ids.len());
        for id in ids {
            if still_open.contains(&id) {
                continue;
            }
            let Some(Object::File(file)) = self.objects.get_mut(id) else {
                continue;
            };
            let Some(open) = &mut file.open else {
                continue;
            };
            still_open.push(id);
            if open.dirty {
                if let Some(name) = &open.name {
                    if self.host.write_file(name, &open.data).is_ok() {
                        open.dirty = false;
                    }
                }
            }
        }
        self.io.open_files = still_open;
    }

    /// Finish with the files of objects the collector freed: write out
    /// what open files had not written, and delete temporary files.
    pub(crate) fn release_files(&mut self) {
        for released in self.objects.take_released() {
            match released {
                Object::File(file) => {
                    if let Some(OpenFile {
                        name: Some(name),
                        data,
                        dirty: true,
                        ..
                    }) = file.open
                    {
                        let _ = self.host.write_file(&name, &data);
                    }
                }
                Object::TempFile(name) if !name.is_empty() => {
                    let _ = self.host.delete_file(&name);
                }
                _ => {}
            }
        }
    }

    /// Finish with every file at the end of the run.
    pub fn close_files(&mut self) {
        self.sync_files();
        self.objects.release_all_files();
        self.release_files();
    }

    fn file(&self, id: ObjId) -> Option<&FileObj> {
        match self.objects.get(id) {
            Some(Object::File(file)) => Some(file),
            _ => None,
        }
    }

    /// An open file's contents, to read or change. A file object is
    /// never saved or undone, so changing one records nothing.
    fn open_file(&mut self, id: ObjId) -> VmResult<&mut OpenFile> {
        match self.objects.get_mut(id) {
            Some(Object::File(file)) => match &mut file.open {
                Some(open) => Ok(open),
                None => Err(VmError::new(code::BAD_TYPE_BIF)),
            },
            _ => Err(VmError::new(code::BAD_TYPE_BIF)),
        }
    }

    /// The file's access and mode, once it is known to be usable.
    fn check_valid_file(&mut self, id: ObjId) -> VmResult<(u8, u8)> {
        let Some(file) = self.file(id) else {
            return Err(VmError::new(code::BAD_TYPE_BIF));
        };
        let (out_of_sync, open, access, mode) = (
            file.out_of_sync,
            file.open.is_some(),
            file.access,
            file.mode,
        );
        if out_of_sync {
            return Err(self.file_exc(FileExc::Sync));
        }
        if !open {
            return Err(self.file_exc(FileExc::Closed));
        }
        Ok((access, mode))
    }

    fn check_raw(&mut self, id: ObjId, writing: bool) -> VmResult<()> {
        let (access, mode) = self.check_valid_file(id)?;
        let wrong_access = if writing {
            access == Access::Read.code()
        } else {
            access == Access::Write.code()
        };
        if wrong_access || mode != MODE_RAW {
            return Err(self.file_exc(FileExc::Mode));
        }
        Ok(())
    }

    /// The character set object an argument names, or the default one.
    fn file_charset_arg(&mut self, index: usize, argc: u32) -> VmResult<ObjId> {
        if (index as u32) < argc {
            let arg = self.arg(index);
            match arg {
                Value::Nil => {}
                Value::Obj(id) if matches!(self.objects.get(id), Some(Object::CharSet { .. })) => {
                    return Ok(id);
                }
                _ => match self.as_str(arg) {
                    Some(name) => {
                        let name = name.to_vec();
                        return Ok(self.new_charset(&name));
                    }
                    None => return Err(VmError::new(code::BAD_TYPE_BIF)),
                },
            }
        }
        // Text outside the VM is UTF-8 unless the program says otherwise.
        Ok(self.new_charset(b"utf8"))
    }

    /// Open a file: `File.openTextFile` and the rest.
    fn file_open(&mut self, mode: u8, resource: bool, argc: u32) -> VmResult<Value> {
        let access = if resource {
            Access::Read
        } else {
            let code = self.cast_to_int(self.arg(1))?;
            Access::from_code(code).ok_or_else(|| VmError::new(code::BAD_VAL_BIF))?
        };
        let target = if resource {
            let name = self.require_str(self.arg(0))?;
            FileTarget {
                name: Some(String::from_utf8_lossy(&name).into_owned()),
                special: false,
                temporary: false,
                from_ui: 0,
                spec: Value::Nil,
            }
        } else {
            let target = self.file_target(self.arg(0))?;
            self.check_safety(&target, access)?;
            target
        };
        let charset = if mode == MODE_TEXT {
            self.file_charset_arg(if resource { 1 } else { 2 }, argc)?
        } else {
            ObjId::INVALID
        };

        self.sync_files();
        let name = target.name.clone();
        let data = if resource {
            let found = name
                .as_ref()
                .and_then(|name| self.io.resources.get(name))
                .cloned();
            match found {
                Some(data) => data.to_vec(),
                None => match self.resource_file(name.as_deref()) {
                    Some(data) => data,
                    None => return Err(self.file_exc(FileExc::NotFound)),
                },
            }
        } else {
            let Some(name) = &name else {
                // A special file the host has nowhere to keep.
                return Err(self.file_exc(match access {
                    Access::Read => FileExc::NotFound,
                    Access::Write => FileExc::Creation,
                    _ => FileExc::Open,
                }));
            };
            match access {
                Access::Read => match self.host.read_file(name) {
                    Ok(data) => data,
                    Err(_) => return Err(self.file_exc(FileExc::NotFound)),
                },
                Access::Write => match self.host.write_file(name, b"") {
                    Ok(()) => Vec::new(),
                    Err(_) => return Err(self.file_exc(FileExc::Creation)),
                },
                Access::ReadWriteKeep => match self.host.read_file(name) {
                    Ok(data) => data,
                    Err(FileError::NotFound) if self.host.write_file(name, b"").is_ok() => {
                        Vec::new()
                    }
                    Err(_) => return Err(self.file_exc(FileExc::Open)),
                },
                _ => match self.host.write_file(name, b"") {
                    Ok(()) => Vec::new(),
                    Err(_) => return Err(self.file_exc(FileExc::Open)),
                },
            }
        };
        let file = FileObj {
            charset,
            mode,
            access: access.code(),
            out_of_sync: false,
            spec: target.spec,
            open: Some(OpenFile {
                name: if resource { None } else { name },
                data,
                pos: 0,
                dirty: false,
            }),
        };
        let id = self.alloc_obj(Object::File(Box::new(file)));
        self.objects.set_transient(id);
        self.io.open_files.push(id);
        Ok(Value::Obj(id))
    }

    /// Read a line of a text file, with its line ending as a newline, or
    /// nil at the end of the file.
    fn file_read_line(&mut self, id: ObjId) -> VmResult<Value> {
        let charset = self.file(id).map_or(ObjId::INVALID, |file| file.charset);
        let map = self.charset_map(charset)?;
        let wide = matches!(*map, crate::charset::CharMap::Utf16 { .. });
        let big_endian = matches!(*map, crate::charset::CharMap::Utf16 { big_endian: true });
        let open = self.open_file(id)?;
        let data = &open.data;
        let start = open.pos.min(data.len());
        let step = if wide { 2 } else { 1 };
        if data.len() - start < step {
            return Ok(Value::Nil);
        }
        // The character at a position, as far as line endings go.
        let unit = |at: usize| -> Option<(u32, usize)> {
            if wide {
                let pair = data.get(at..at + 2)?;
                let unit = if big_endian {
                    u16::from_be_bytes([pair[0], pair[1]])
                } else {
                    u16::from_le_bytes([pair[0], pair[1]])
                };
                Some((unit.into(), 2))
            } else if data.get(at..at + 3) == Some(&[0xe2, 0x80, 0xa8]) && !wide {
                Some((0x2028, 3))
            } else {
                data.get(at).map(|&byte| (byte.into(), 1))
            }
        };
        let line_separator_possible = matches!(*map, crate::charset::CharMap::Utf8) || wide;
        let mut at = start;
        let mut end = None;
        while let Some((c, len)) = unit(at) {
            let is_end =
                c == 0x0a || c == 0x0d || (c == 0x2028 && (line_separator_possible || wide));
            if is_end {
                let mut next = at + len;
                // A line feed and carriage return together, in either
                // order, are one line ending.
                let partner = match c {
                    0x0a => Some(0x0d),
                    0x0d => Some(0x0a),
                    _ => None,
                };
                if let (Some(partner), Some((following, following_len))) = (partner, unit(next)) {
                    if following == partner {
                        next += following_len;
                    }
                }
                end = Some((at, next));
                break;
            }
            at += len;
        }
        let (line_end, next) = end.unwrap_or((at, at));
        let mut text = map.decode(&data[start..line_end]);
        if end.is_some() {
            text.push('\n');
        }
        open.pos = next;
        Ok(self.new_str(&text))
    }

    /// Read a value from a data file, or nil at the end of the file.
    fn file_read_data(&mut self, id: ObjId) -> VmResult<Value> {
        let open = self.open_file(id)?;
        let Some(tag) = open.read_exact(1) else {
            return Ok(Value::Nil);
        };
        let int = |bytes: Vec<u8>| u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
        let value = match tag[0] {
            TAG_INT => open
                .read_exact(4)
                .map(|bytes| Value::Int(int(bytes) as i32)),
            TAG_ENUM => open.read_exact(4).map(|bytes| Value::Enum(int(bytes))),
            TAG_TRUE => Some(Value::True),
            TAG_STRING => {
                let text = open
                    .read_exact(2)
                    .map(|len| usize::from(u16::from_le_bytes([len[0], len[1]])))
                    .and_then(|len| len.checked_sub(2))
                    .and_then(|len| open.read_exact(len));
                text.map(|text| self.new_string(text))
            }
            TAG_BIGNUM => {
                // The precision, then the number as it is stored, which
                // starts with the precision again.
                let number = open
                    .read_exact(2)
                    .map(|prec| usize::from(u16::from_le_bytes([prec[0], prec[1]])))
                    .and_then(|prec| open.read_exact(5 + prec.div_ceil(2)))
                    .and_then(|data| crate::bignum::Big::from_bytes(&data));
                number.map(|number| self.new_big(number))
            }
            TAG_BYTEARRAY => {
                let bytes = open
                    .read_exact(4)
                    .map(|len| int(len) as usize)
                    .and_then(|len| open.read_exact(len));
                bytes.map(|bytes| self.new_bytes(bytes))
            }
            _ => None,
        };
        match value {
            Some(value) => Ok(value),
            None => Err(self.file_exc(FileExc::Io)),
        }
    }

    /// Write a value to a text file: its text, in the file's character
    /// set, with each newline written as the system writes the end of a
    /// line, as the reference does.
    fn file_write_text(&mut self, id: ObjId, value: Value) -> VmResult<()> {
        let charset = self.file(id).map_or(ObjId::INVALID, |file| file.charset);
        let map = self.charset_map(charset)?;
        let (_, text) = self.text_cast(value)?;
        let text = String::from_utf8_lossy(&text);
        let text = if NEWLINE == "\n" {
            text
        } else {
            text.replace('\n', NEWLINE).into()
        };
        let bytes = map.encode(&text);
        self.open_file(id)?.write(&bytes);
        Ok(())
    }

    /// Write a value to a data file, marked with its type.
    fn file_write_data(&mut self, id: ObjId, value: Value) -> VmResult<()> {
        let mut out = Vec::new();
        let string = |out: &mut Vec<u8>, text: &[u8]| {
            out.push(TAG_STRING);
            out.extend_from_slice(&((text.len() + 2) as u16).to_le_bytes());
            out.extend_from_slice(text);
        };
        match value {
            Value::Int(n) => {
                out.push(TAG_INT);
                out.extend_from_slice(&n.to_le_bytes());
            }
            Value::Enum(n) => {
                out.push(TAG_ENUM);
                out.extend_from_slice(&n.to_le_bytes());
            }
            Value::True => out.push(TAG_TRUE),
            Value::SString(_) => {
                let text = self.require_str(value)?;
                string(&mut out, &text);
            }
            Value::Obj(object) => match self.objects.get(object) {
                Some(Object::BigNumber(big)) => {
                    out.push(TAG_BIGNUM);
                    out.extend_from_slice(&(big.prec() as u16).to_le_bytes());
                    out.extend_from_slice(&big.to_bytes());
                }
                Some(Object::ByteArray(bytes)) => {
                    out.push(TAG_BYTEARRAY);
                    out.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
                    out.extend_from_slice(bytes);
                }
                _ => {
                    let (_, text) = self.text_cast(value)?;
                    string(&mut out, &text);
                }
            },
            _ => return Err(VmError::new(code::BAD_TYPE_BIF)),
        }
        self.open_file(id)?.write(&out);
        Ok(())
    }

    /// The optional length argument of the hash methods: nil or nothing
    /// is the size of the file.
    fn file_hash_input(&mut self, id: ObjId, argc: u32) -> VmResult<Vec<u8>> {
        self.check_raw(id, false)?;
        let len = if argc >= 1 && self.arg(0) != Value::Nil {
            Some(self.cast_to_int(self.arg(0))?.max(0) as usize)
        } else {
            None
        };
        let open = self.open_file(id)?;
        let len = len.unwrap_or(open.data.len());
        Ok(open.read(len).to_vec())
    }

    /// Call a method of File on a file object, or with no object on the
    /// class itself.
    pub(crate) fn file_method(
        &mut self,
        id: Option<ObjId>,
        index: u16,
        argc: Option<&mut u32>,
    ) -> VmResult<Option<Value>> {
        let Some(&desc) = FILE_METHODS.get(index as usize).filter(|_| index != 0) else {
            return Ok(None);
        };
        // Only the methods that make or name a file work on the class.
        let class_method = matches!(index, 1 | 2 | 3 | 14 | 15 | 18 | 19);
        if id.is_none() && !class_method {
            return Ok(None);
        }
        self.native_method(argc, desc, |vm, argc| match id {
            Some(id) if !class_method => vm.file_call(id, index, argc),
            _ => vm.file_class_call(index, argc),
        })
        .map(Some)
    }

    fn file_class_call(&mut self, index: u16, argc: u32) -> VmResult<Value> {
        match index {
            1 => self.file_open(MODE_TEXT, false, argc),
            2 => self.file_open(MODE_DATA, false, argc),
            3 => self.file_open(MODE_RAW, false, argc),
            14 => self.file_open(MODE_TEXT, true, argc),
            15 => self.file_open(MODE_RAW, true, argc),
            // getRootName(name)
            18 => {
                let name = self
                    .path_of(self.arg(0))
                    .ok_or_else(|| VmError::new(code::BAD_TYPE_BIF))?;
                Ok(self.new_str(root_name(&name)))
            }
            // deleteFile(name)
            _ => {
                let target = self.file_target(self.arg(0))?;
                self.check_safety(&target, Access::Delete)?;
                self.delete_target(&target)?;
                Ok(Value::Nil)
            }
        }
    }

    fn delete_target(&mut self, target: &FileTarget) -> VmResult<()> {
        self.sync_files();
        let deleted = match &target.name {
            Some(name) => self.host.delete_file(name).is_ok(),
            None => false,
        };
        if deleted {
            Ok(())
        } else {
            Err(VmError::new(code::DELETE_FILE))
        }
    }

    fn file_call(&mut self, id: ObjId, index: u16, argc: u32) -> VmResult<Value> {
        let (access, mode) = self.check_valid_file(id)?;
        let read_only = access == Access::Read.code();
        let write_only = access == Access::Write.code();
        Ok(match index {
            // getCharacterSet()
            4 => match self.file(id).map(|file| file.charset) {
                Some(charset) if charset.is_valid() => Value::Obj(charset),
                _ => Value::Nil,
            },
            // setCharacterSet(charset)
            5 => {
                let charset = self.file_charset_arg(0, 1)?;
                if let Some(Object::File(file)) = self.objects.get_mut(id) {
                    file.charset = charset;
                }
                Value::Nil
            }
            // closeFile()
            6 => {
                let (open, spec) = match self.objects.get_mut(id) {
                    Some(Object::File(file)) => (file.open.take(), file.spec),
                    _ => (None, Value::Nil),
                };
                self.io.open_files.retain(|&open_id| open_id != id);
                if let Some(OpenFile {
                    name: Some(name),
                    data,
                    dirty: true,
                    ..
                }) = open
                {
                    if self.host.write_file(&name, &data).is_err() {
                        return Err(self.file_exc(FileExc::Io));
                    }
                }
                // An object of the program's that stood for the file gets
                // to know that the file is finished with.
                if let Value::Obj(spec_id) = spec {
                    let close_file = self.predef.filespec_close_file;
                    if close_file != INVALID_PROP
                        && matches!(self.objects.get(spec_id), Some(Object::Tads(_)))
                    {
                        let native = NativeCall::named("System.closeFile");
                        self.call_prop(spec, close_file, 0, Some(native))?;
                    }
                }
                Value::Nil
            }
            // readFile()
            7 => {
                if write_only || mode == MODE_RAW {
                    return Err(self.file_exc(FileExc::Mode));
                }
                if mode == MODE_TEXT {
                    self.file_read_line(id)?
                } else {
                    self.file_read_data(id)?
                }
            }
            // writeFile(val)
            8 => {
                if read_only || mode == MODE_RAW {
                    return Err(self.file_exc(FileExc::Mode));
                }
                if mode == MODE_TEXT {
                    self.file_write_text(id, self.arg(0))?;
                } else {
                    self.file_write_data(id, self.arg(0))?;
                }
                Value::Nil
            }
            // readBytes(byteArr, start?, cnt?): how many bytes were read.
            9 => {
                if write_only || mode != MODE_RAW {
                    return Err(self.file_exc(FileExc::Mode));
                }
                let array = self.arg(0);
                let Some(count) = self.as_bytes(array).map(Vec::len) else {
                    return Err(VmError::new(code::BAD_TYPE_BIF));
                };
                let index = if argc >= 2 {
                    self.int_arg(self.arg(1))?
                } else {
                    1
                };
                let len = if argc >= 3 {
                    self.int_arg(self.arg(2))? as u32 as usize
                } else {
                    count
                };
                if index < 1 {
                    return Err(VmError::new(code::INDEX_OUT_OF_RANGE));
                }
                let index = index as usize;
                if index > count || len == 0 {
                    return Ok(Value::Int(0));
                }
                let len = len.min(count - index + 1);
                let bytes = self.open_file(id)?.read(len).to_vec();
                let target = self.bytes_mut(array.obj_id())?;
                target[index - 1..index - 1 + bytes.len()].copy_from_slice(&bytes);
                Value::Int(bytes.len() as i32)
            }
            // writeBytes(source, start?, cnt?)
            10 => {
                if read_only || mode != MODE_RAW {
                    return Err(self.file_exc(FileExc::Mode));
                }
                self.file_write_bytes(id, argc)?;
                Value::Nil
            }
            // getPos()
            11 => Value::Int(self.open_file(id)?.pos as i32),
            // setPos(pos)
            12 => {
                let pos = self.cast_to_int(self.arg(0))?;
                self.open_file(id)?.pos = pos.max(0) as usize;
                Value::Nil
            }
            // setPosEnd()
            13 => {
                let open = self.open_file(id)?;
                open.pos = open.data.len();
                Value::Nil
            }
            // getFileSize()
            16 => Value::Int(self.open_file(id)?.data.len() as i32),
            // getFileMode()
            17 => Value::Int(mode.into()),
            // setFileMode(mode, charset?)
            20 => {
                let new_mode = self.int_arg(self.arg(0))?;
                let charset = match new_mode {
                    1 => self.file_charset_arg(1, argc)?,
                    2 | 3 => ObjId::INVALID,
                    _ => return Err(VmError::new(code::BAD_VAL_BIF)),
                };
                if let Some(Object::File(file)) = self.objects.get_mut(id) {
                    file.mode = new_mode as u8;
                    file.charset = charset;
                }
                Value::Nil
            }
            // packBytes(format, ...): how many bytes were written.
            21 => {
                self.check_raw(id, true)?;
                let open = self.open_file(id)?;
                if open.pos > open.data.len() {
                    let pos = open.pos;
                    open.data.resize(pos, 0);
                }
                let start = open.pos;
                let stream = Stream::growing(std::mem::take(&mut open.data), start);
                let (stream, result) = self.pack_stream(0, argc as usize, stream);
                let open = self.open_file(id)?;
                open.pos = stream.pos;
                open.data = stream.data;
                open.dirty = true;
                result?;
                Value::Int((self.open_file(id)?.pos - start) as i32)
            }
            // unpackBytes(format)
            22 => {
                self.check_raw(id, false)?;
                let format = self.arg(0);
                if self.as_str(format).is_none() {
                    return Err(VmError::new(code::BAD_TYPE_BIF));
                }
                let open = self.open_file(id)?;
                let stream = Stream::fixed(std::mem::take(&mut open.data), open.pos);
                let (stream, result) = self.unpack_stream(format, stream);
                let open = self.open_file(id)?;
                open.pos = open.pos.max(stream.pos);
                open.data = stream.data;
                result?
            }
            // sha256(length?)
            23 => {
                let input = self.file_hash_input(id, argc)?;
                self.new_str(&hash::hex(&hash::sha256(&input)))
            }
            // digestMD5(length?)
            24 => {
                let input = self.file_hash_input(id, argc)?;
                self.new_str(&hash::hex(&hash::md5(&input)))
            }
            _ => Value::Nil,
        })
    }

    /// `writeBytes(source, start?, cnt?)`: write from a byte array, or
    /// copy from another file.
    fn file_write_bytes(&mut self, id: ObjId, argc: u32) -> VmResult<()> {
        let source = self.arg(0);
        let Value::Obj(source_id) = source else {
            return Err(VmError::new(code::OBJ_VAL_REQD));
        };
        let optional = |vm: &Self, index: usize| -> VmResult<Option<i32>> {
            if (index as u32) < argc && vm.arg(index) != Value::Nil {
                vm.int_arg(vm.arg(index)).map(Some)
            } else {
                Ok(None)
            }
        };
        if let Some(count) = self.as_bytes(source).map(Vec::len) {
            let index = optional(self, 1)?.unwrap_or(1);
            let len = optional(self, 2)?.map_or(count, |len| len as u32 as usize);
            if index < 1 {
                return Err(VmError::new(code::INDEX_OUT_OF_RANGE));
            }
            let index = index as usize;
            if index > count {
                return Ok(());
            }
            let len = len.min(count - index + 1);
            let bytes = self
                .as_bytes(source)
                .map_or_else(Vec::new, |bytes| bytes[index - 1..index - 1 + len].to_vec());
            self.open_file(id)?.write(&bytes);
            return Ok(());
        }
        let Some(other) = self.file(source_id) else {
            return Err(VmError::new(code::BAD_TYPE_BIF));
        };
        let usable = !other.out_of_sync && other.open.is_some();
        if other.mode != MODE_RAW {
            return Err(self.file_exc(FileExc::Mode));
        }
        let start = optional(self, 1)?;
        let len = optional(self, 2)?;
        if !usable {
            // The reference VM takes the size of an unusable file to be
            // -1, a very large count, and then fails to read from it.
            return Err(self.file_exc(FileExc::Io));
        }
        let other = self.open_file(source_id)?;
        if let Some(start) = start {
            other.pos = start.max(0) as usize;
        }
        let len = len.map_or(other.data.len(), |len| len as u32 as usize);
        let bytes = other.read(len).to_vec();
        // A file copied onto itself has moved on by what was read.
        self.open_file(id)?.write(&bytes);
        Ok(())
    }

    // ---- FileName ----

    /// `new FileName(...)`.
    pub(crate) fn create_file_name(&mut self, argc: u32) -> VmResult<ObjId> {
        match argc {
            0 => Ok(self.new_file_name(".")),
            1 => {
                let arg = self.arg(0);
                if let Some(text) = self.as_str(arg) {
                    let path = String::from_utf8_lossy(&text).into_owned();
                    return Ok(self.new_file_name(&path));
                }
                if matches!(arg, Value::Int(_)) || self.as_big(arg).is_some() {
                    let special = self.cast_to_int(arg)?;
                    let path = self.special_file_path(special)?.unwrap_or_default();
                    return Ok(self.alloc_obj(Object::FileName(Box::new(FileNameObj {
                        path,
                        special,
                        from_ui: 0,
                    }))));
                }
                Err(VmError::new(code::BAD_TYPE_BIF))
            }
            2 => match (self.path_of(self.arg(0)), self.path_of(self.arg(1))) {
                (Some(dir), Some(name)) => Ok(self.new_file_name(&combine(&dir, &name, false))),
                _ => Err(VmError::new(code::BAD_TYPE_BIF)),
            },
            _ => Err(VmError::new(code::WRONG_NUM_OF_ARGS)),
        }
    }

    /// A file name with another name added to it, for `+`.
    pub(crate) fn file_name_add(&mut self, id: ObjId, other: Value) -> VmResult<Value> {
        let Some(name) = self.path_of(other) else {
            return Err(VmError::new(code::BAD_TYPE_ADD));
        };
        let dir = self
            .file_name(id)
            .map_or_else(String::new, |n| n.path.clone());
        Ok(Value::Obj(self.new_file_name(&combine(&dir, &name, false))))
    }

    /// Whether a file name is the same name as another, or as a string.
    pub(crate) fn file_name_equals(&self, id: ObjId, other: Value) -> bool {
        match (self.file_name(id), self.path_of(other)) {
            (Some(name), Some(other)) => name.path == other,
            _ => false,
        }
    }

    /// Call a method of FileName on a file name, or with no object on the
    /// class itself.
    pub(crate) fn file_name_method(
        &mut self,
        id: Option<ObjId>,
        index: u16,
        argc: Option<&mut u32>,
    ) -> VmResult<Option<Value>> {
        let Some(&desc) = FILE_NAME_METHODS.get(index as usize).filter(|_| index != 0) else {
            return Ok(None);
        };
        let class_method = matches!(index, 4 | 9);
        match id {
            None if !class_method => return Ok(None),
            _ => {}
        }
        self.native_method(argc, desc, |vm, argc| match (id, class_method) {
            (Some(id), false) => vm.file_name_call(id, index, argc),
            _ => vm.file_name_class_call(index),
        })
        .map(Some)
    }

    fn file_name_class_call(&mut self, index: u16) -> VmResult<Value> {
        if index == 4 {
            // fromUniversal(path)
            let Some(text) = self.as_str(self.arg(0)) else {
                return Err(VmError::new(code::BAD_VAL_BIF));
            };
            let path = String::from_utf8_lossy(&text).into_owned();
            return Ok(Value::Obj(self.new_file_name(&path)));
        }
        // getRootDirs(): the ones the program is allowed to look at.
        let mut roots = Vec::new();
        for root in self.host.root_dirs() {
            if self.safety_allows(&root, Access::GetInfo) {
                roots.push(root);
            }
        }
        let list = self.new_list(vec![Value::Nil; roots.len()]);
        let names: Vec<Value> = roots
            .iter()
            .map(|root| Value::Obj(self.new_file_name(root)))
            .collect();
        self.fill_list(list, names);
        Ok(list)
    }

    /// Put the elements into a list created ahead of them, as the
    /// reference VM creates it.
    fn fill_list(&mut self, list: Value, items: Vec<Value>) {
        if let Some(Object::List(stored)) = self.objects.get_mut(list.obj_id()) {
            *stored = items.into();
        }
    }

    /// The file a FileName refers to, checked against the safety levels.
    fn file_name_target(&mut self, id: ObjId, access: Access) -> VmResult<FileTarget> {
        let target = self.file_target(Value::Obj(id))?;
        self.check_safety(&target, access)?;
        Ok(target)
    }

    fn logical_arg(&self, index: usize, argc: u32) -> VmResult<bool> {
        if (index as u32) >= argc {
            return Ok(false);
        }
        match self.arg(index) {
            Value::True => Ok(true),
            Value::Nil => Ok(false),
            _ => Err(VmError::new(code::BAD_TYPE_BIF)),
        }
    }

    fn file_name_call(&mut self, id: ObjId, index: u16, argc: u32) -> VmResult<Value> {
        let Some(name) = self.file_name(id) else {
            return Ok(Value::Nil);
        };
        let path = name.path.clone();
        Ok(match index {
            // getName()
            1 => self.new_str(&path),
            // getBaseName()
            2 => match root_name(&path) {
                "" => Value::Nil,
                root => self.new_str(root),
            },
            // getPath()
            3 => match path_name(&path) {
                "" => Value::Nil,
                dir => Value::Obj(self.new_file_name(dir)),
            },
            // toUniversal()
            5 => self.new_str(&path),
            // addToPath(name)
            6 => {
                let Some(name) = self.path_of(self.arg(0)) else {
                    return Err(VmError::new(code::BAD_TYPE_BIF));
                };
                Value::Obj(self.new_file_name(&combine(&path, &name, false)))
            }
            // isAbsolute()
            7 => Value::logical(is_absolute(&path)),
            // getAbsolutePath()
            8 => match self.host.absolute_path(&path) {
                Some(full) => self.new_str(&full),
                None => Value::Nil,
            },
            // getFileType()
            10 => {
                let target = self.file_name_target(id, Access::GetInfo)?;
                self.sync_files();
                let info = target
                    .name
                    .and_then(|name| self.host.file_info(&name, true));
                match info {
                    Some(info) => Value::Int(info.kind as i32 | special_link_flags(&path)),
                    None => Value::Nil,
                }
            }
            // getFileInfo()
            11 => {
                let target = self.file_name_target(id, Access::GetInfo)?;
                self.sync_files();
                let Some(info) = target
                    .name
                    .and_then(|name| self.host.file_info(&name, true))
                else {
                    return Ok(Value::Nil);
                };
                let target = match &info.link_target {
                    Some(target) if info.kind & file_type::LINK != 0 => self.new_str(target),
                    _ => Value::Nil,
                };
                let size = match i32::try_from(info.size) {
                    Ok(size) => Value::Int(size),
                    Err(_) => {
                        let size = crate::bignum::Big::from_u64(info.size, 20);
                        self.new_big(size)
                    }
                };
                let kind = Value::Int(info.kind as i32 | special_link_flags(&path));
                // The arguments of the FileInfo constructor, first one
                // first. The reference VM creates them last one first.
                let mut time = |seconds: Option<i64>| match seconds {
                    Some(seconds) if seconds != 0 => self.date_from_unix(seconds),
                    _ => Value::Nil,
                };
                let accessed = time(info.accessed);
                let modified = time(info.modified);
                let created = time(info.created);
                let args = [
                    kind,
                    size,
                    created,
                    modified,
                    accessed,
                    target,
                    Value::Int(info.attrs as i32),
                ];
                let class = self.predef.file_info;
                if class.is_valid() {
                    self.check_stack(args.len())?;
                    for &arg in args.iter().rev() {
                        self.push(arg);
                    }
                    self.create_instance(class, args.len() as u32)?;
                    match self.r0 {
                        Value::Obj(_) => self.r0,
                        _ => return Err(VmError::new(code::OBJ_VAL_REQD)),
                    }
                } else {
                    self.new_list(args.to_vec())
                }
            }
            // deleteFile()
            12 => {
                let target = self.file_name_target(id, Access::Delete)?;
                self.delete_target(&target)?;
                Value::Nil
            }
            // renameFile(newName)
            13 => {
                let from = self.file_name_target(id, Access::RenameFrom)?;
                let to = self.file_target(self.arg(0))?;
                self.check_safety(&to, Access::RenameTo)?;
                self.sync_files();
                let renamed = match (&from.name, &to.name) {
                    (Some(from), Some(to)) => {
                        self.host.file_info(to, true).is_none()
                            && self.host.rename_file(from, to).is_ok()
                    }
                    _ => false,
                };
                if !renamed {
                    return Err(VmError::new(code::RENAME_FILE));
                }
                // An open file goes on being written under its new name.
                self.note_renamed(from.name.as_deref(), to.name.as_deref());
                Value::Nil
            }
            // listDir()
            14 => {
                let target = self.file_name_target(id, Access::ReadDir)?;
                self.sync_files();
                let entries = target.name.and_then(|name| self.host.list_dir(&name));
                let Some(entries) = entries else {
                    return Err(self.file_exc(FileExc::OpenDir));
                };
                let list = self.new_list(vec![Value::Nil; entries.len()]);
                let names: Vec<Value> = entries
                    .iter()
                    .map(|entry| Value::Obj(self.new_file_name(&combine(&path, entry, true))))
                    .collect();
                self.fill_list(list, names);
                list
            }
            // forEachFile(func, recursive?)
            15 => {
                let target = self.file_name_target(id, Access::ReadDir)?;
                let recursive = self.logical_arg(1, argc)?;
                let callback = self.arg(0);
                self.sync_files();
                let listed = match &target.name {
                    Some(name) => self.for_each_file(name, &path, callback, recursive)?,
                    None => false,
                };
                if !listed {
                    return Err(self.file_exc(FileExc::OpenDir));
                }
                Value::Nil
            }
            // createDirectory(createParents?)
            16 => {
                let target = self.file_name_target(id, Access::MakeDir)?;
                let parents = self.logical_arg(0, argc)?;
                let created = target
                    .name
                    .is_some_and(|name| self.host.create_dir(&name, parents).is_ok());
                if !created {
                    return Err(VmError::new(code::CREATE_FILE));
                }
                Value::Nil
            }
            // removeDirectory(removeContents?)
            17 => {
                let target = self.file_name_target(id, Access::RemoveDir)?;
                let contents = self.logical_arg(0, argc)?;
                self.sync_files();
                let removed = target
                    .name
                    .is_some_and(|name| self.host.remove_dir(&name, contents).is_ok());
                if !removed {
                    return Err(VmError::new(code::DELETE_FILE));
                }
                Value::Nil
            }
            _ => Value::Nil,
        })
    }

    /// Change the name an open file will be written back under.
    fn note_renamed(&mut self, from: Option<&str>, to: Option<&str>) {
        let (Some(from), Some(to)) = (from, to) else {
            return;
        };
        for id in self.io.open_files.clone() {
            if let Some(Object::File(file)) = self.objects.get_mut(id) {
                if let Some(open) = &mut file.open {
                    if open.name.as_deref() == Some(from) {
                        open.name = Some(to.to_owned());
                    }
                }
            }
        }
    }

    /// Call a function with the FileName of each entry of a directory.
    /// `name` is the directory for the host, and `shown` the same
    /// directory as the program named it. False if it cannot be listed.
    fn for_each_file(
        &mut self,
        name: &str,
        shown: &str,
        callback: Value,
        recursive: bool,
    ) -> VmResult<bool> {
        let Some(entries) = self.host.list_dir(name) else {
            return Ok(false);
        };
        for entry in entries {
            let entry_shown = combine(shown, &entry, true);
            let file_name = Value::Obj(self.new_file_name(&entry_shown));
            // Keep the name alive while the function runs.
            self.push(file_name);
            let native = NativeCall::named("FileName.forEachFile");
            let result = self.call_value(callback, &[file_name], native);
            self.pop();
            result?;
            if recursive && entry != "." && entry != ".." {
                let entry_name = combine(name, &entry, true);
                let is_dir = self
                    .host
                    .file_info(&entry_name, false)
                    .is_some_and(|info| info.kind & file_type::DIR != 0);
                if is_dir {
                    self.for_each_file(&entry_name, &entry_shown, callback, true)?;
                }
            }
        }
        Ok(true)
    }

    // ---- TemporaryFile ----

    /// `new TemporaryFile()`.
    pub(crate) fn create_temp_file(&mut self, argc: u32) -> VmResult<ObjId> {
        if argc != 0 {
            return Err(VmError::new(code::WRONG_NUM_OF_ARGS));
        }
        let Some(name) = self.host.temp_file_name() else {
            return Err(VmError::new(code::CREATE_FILE));
        };
        let id = self.alloc_obj(Object::TempFile(name));
        self.objects.set_transient(id);
        Ok(id)
    }

    pub(crate) fn temp_file_method(
        &mut self,
        id: ObjId,
        index: u16,
        argc: Option<&mut u32>,
    ) -> VmResult<Option<Value>> {
        if !matches!(index, 1 | 2) {
            return Ok(None);
        }
        self.native_method(argc, NativeDesc::fixed(0), |vm, _| {
            let name = match vm.objects.get(id) {
                Some(Object::TempFile(name)) => name.clone(),
                _ => String::new(),
            };
            if name.is_empty() {
                return Ok(Value::Nil);
            }
            Ok(if index == 1 {
                // getFilename()
                vm.new_str(&name)
            } else {
                // deleteFile(). The object goes on naming the file.
                vm.sync_files();
                let _ = vm.host.delete_file(&name);
                Value::Nil
            })
        })
        .map(Some)
    }
}

/// A file's contents as the host has them, for the functions that work
/// on whole files.
pub(crate) type Resources = std::collections::HashMap<String, Rc<[u8]>>;
