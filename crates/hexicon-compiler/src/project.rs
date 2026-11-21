//! What to build: the options and the source files of a program, as the
//! reference's `t3make` takes them, from its command line and from a
//! project file.
//!
//! A project file (`.t3m`) is more of the command line, a word or a
//! quoted phrase at a time, with `#` starting a comment line; what it
//! names is relative to the folder it is in. A library (`.tl`) is a
//! list of `source:` and `library:` lines, each relative to the
//! library's own folder, with `$(NAME)` for the value a `-D` option
//! gave a macro. Unless told otherwise (`-nodef`, or `nodef` in a
//! library), the build starts with the system library's `_main`.
//!
//! Files are found and read through [`SourceFiles`], like everything
//! else the compiler reads.

use crate::compile::CompileOptions;
use crate::pp::SourceFiles;

/// The separator of the folders in a path, as paths are made here.
const SEP: char = std::path::MAIN_SEPARATOR;

/// Where the reference looks for what it does not find elsewhere.
#[derive(Debug, Clone, Default)]
pub struct ProjectPaths {
    /// The system's header files (`-FI` changes it).
    pub system_include: String,
    /// The system library: `_main.t`, `system.tl`, and the libraries
    /// built on it (`-FL` changes it).
    pub system_library: String,
    /// More folders to look for libraries and sources in, after any
    /// `-Fs` names and before the system library.
    pub user_libraries: Vec<String>,
}

/// A program to build.
#[derive(Debug, Clone, Default)]
pub struct Project {
    /// The source files, in the order they are put together.
    pub sources: Vec<String>,
    /// How to compile them. The time stamp is the caller's to set.
    pub options: CompileOptions,
    /// Where to write the image, if anything said.
    pub image: Option<String>,
    /// The program is to be run once before the image is written
    /// (preinitialization): `-pre`, or by default unless `-d`.
    pub preinit: bool,
    /// Warnings are reported (`-w0` says not).
    pub warnings: bool,
    /// A warning stops the build (`-we`).
    pub warnings_as_errors: bool,
    /// Say what is being done (`-v`).
    pub verbose: bool,
    /// The program is built for debugging (`-d`): its resources are
    /// linked to their files rather than copied into the image.
    pub debug: bool,
    /// The files to bundle into the image as resources, in order: those
    /// the libraries name, then those after `-res`.
    pub resources: Vec<ResourceSpec>,
    /// What was said about the project that does not stop the build,
    /// in the words the reference uses: problems with libraries.
    pub messages: Vec<String>,
}

/// A file or a folder to bundle as resources.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceSpec {
    /// The file or folder.
    pub file: String,
    /// The name to give it in the image (`file=alias`), if not the
    /// file's own path written as a URL.
    pub alias: Option<String>,
    /// For a folder, whether the folders in it are bundled too.
    pub recurse: bool,
}

/// A file bundled as a resource, and its name in the image.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resource {
    /// The file.
    pub file: String,
    /// The name the program asks for it by.
    pub name: String,
}

/// What is in a folder, as [`expand_resources`] is told it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirEntry {
    /// The file's name, without the folder's.
    pub name: String,
    /// It is a folder.
    pub is_dir: bool,
    /// Hidden, or belonging to the system: such files are left out.
    pub hidden: bool,
}

/// The files the resources name, each folder replaced by the files in
/// it, in the order the system lists them, as the reference does. A
/// file in a folder is named by the folder's name, a `/`, and its own.
/// `list_dir` gives what is in a folder, or nothing for a path that is
/// not one.
pub fn expand_resources(
    specs: &[ResourceSpec],
    list_dir: &mut dyn FnMut(&str) -> Option<Vec<DirEntry>>,
) -> Vec<Resource> {
    fn add(
        file: &str,
        name: &str,
        recurse: bool,
        list_dir: &mut dyn FnMut(&str) -> Option<Vec<DirEntry>>,
        out: &mut Vec<Resource>,
    ) {
        let Some(entries) = list_dir(file) else {
            out.push(Resource {
                file: file.to_owned(),
                name: name.to_owned(),
            });
            return;
        };
        for entry in entries {
            if entry.hidden || entry.name == "." || entry.name == ".." {
                continue;
            }
            let full = build_full_path(file, &entry.name);
            let mut full_name = name.to_owned();
            if !full_name.is_empty() && !full_name.ends_with('/') {
                full_name.push('/');
            }
            full_name.push_str(&entry.name);
            if !entry.is_dir {
                out.push(Resource {
                    file: full,
                    name: full_name,
                });
            } else if recurse {
                add(&full, &full_name, true, list_dir, out);
            }
        }
    }
    let mut out = Vec::new();
    for spec in specs {
        let name = spec
            .alias
            .clone()
            .unwrap_or_else(|| cvt_dir_url(&spec.file));
        add(&spec.file, &name, spec.recurse, list_dir, &mut out);
    }
    out
}

/// A path written as a relative URL, as the reference writes it on
/// Windows: with `/` between folders, a drive as `/C:`, and no `/` at
/// the end.
fn cvt_dir_url(path: &str) -> String {
    let mut out = String::new();
    let bytes = path.as_bytes();
    let mut rest = path;
    if bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' {
        out.push('/');
        out.push_str(&path[..2]);
        rest = &path[2..];
        if !rest.starts_with(is_sep) {
            out.push_str("./");
        }
    } else if bytes.len() >= 2 && is_sep(bytes[0] as char) && is_sep(bytes[1] as char) {
        // A UNC path: the machine's name stays as it is, after a `/`.
        let end = path[2..].find(is_sep).map_or(path.len(), |at| at + 2);
        out.push('/');
        out.push_str(&path[..end]);
        rest = &path[end..];
    }
    out.extend(rest.chars().map(|c| if is_sep(c) { '/' } else { c }));
    while out.len() > 1 && out.ends_with('/') {
        out.pop();
    }
    out
}

/// Why there is nothing to build.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProjectError {
    /// The command line is wrong: the caller should say how it goes.
    /// The text says what was wrong, if anything in particular.
    Usage(String),
    /// What was found wrong, a line at a time, in the words the
    /// reference uses.
    Failed(Vec<String>),
}

// ---- paths, as the reference makes them on Windows ----

fn is_sep(c: char) -> bool {
    c == '/' || c == '\\'
}

/// The folder part of a path, without its last separator.
fn path_name(path: &str) -> &str {
    match path.rfind(|c| is_sep(c) || c == ':') {
        None => "",
        Some(at) if path[at..].starts_with(':') => &path[..=at],
        // The root itself keeps its separator.
        Some(at) if at == 0 || path[..at].ends_with(':') => &path[..=at],
        Some(at) => &path[..at],
    }
}

/// The name of the file a path is to, without its folders.
fn root_name(path: &str) -> &str {
    match path.rfind(|c| is_sep(c) || c == ':') {
        None => path,
        Some(at) => &path[at + 1..],
    }
}

fn is_absolute(path: &str) -> bool {
    let mut chars = path.chars();
    match (chars.next(), chars.next()) {
        (Some(c), _) if is_sep(c) => true,
        (Some(c), Some(':')) => c.is_ascii_alphabetic(),
        _ => false,
    }
}

fn build_full_path(dir: &str, file: &str) -> String {
    if dir.is_empty() {
        file.to_owned()
    } else if dir.ends_with(|c| is_sep(c) || c == ':') {
        format!("{dir}{file}")
    } else {
        format!("{dir}{SEP}{file}")
    }
}

/// A path with an extension added if its file has none.
fn def_ext(path: &str, ext: &str) -> String {
    if root_name(path).contains('.') {
        path.to_owned()
    } else {
        format!("{path}.{ext}")
    }
}

/// A path without its file's extension.
fn rem_ext(path: &str) -> &str {
    let root = root_name(path);
    match root.rfind('.') {
        Some(at) => &path[..path.len() - root.len() + at],
        None => path,
    }
}

/// A path written with `/`, as a library writes them, written with the
/// system's separator.
fn cvt_url_dir(url: &str) -> String {
    url.replace('/', std::path::MAIN_SEPARATOR_STR)
}

/// Where a module comes from, for the message about two modules with
/// one name.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Origin {
    /// The command line or the project file.
    Normal,
    /// The system library's `_main`, which the build starts with.
    SysLib,
    /// A library: its file.
    Lib(String),
}

#[derive(Debug, Clone)]
struct Module {
    /// The name the module was given: on the command line or in the
    /// project file, or in its library.
    orig_name: String,
    /// The source file, with `.t` if the name had no extension.
    source: String,
    /// The name to look for along the source path if the file is not
    /// where `source` says.
    search: String,
    origin: Origin,
    /// Its name in its library: the library's own name for it, after
    /// the names of the libraries it is in (`en_us/en_us/en_us`).
    url: String,
    excluded: bool,
}

struct Reader<'f, 'a> {
    files: &'f mut (dyn SourceFiles + 'a),
    /// The folders to look for sources and libraries in: those `-Fs`
    /// names, and then the system's.
    user_source_paths: Vec<String>,
    sys_source_paths: Vec<String>,
    /// The system library and its headers.
    sys_library: String,
    sys_include: String,
    /// The folders for `#include`, in order, but for the system's.
    include_paths: Vec<String>,
    /// The macros, as `-D` and `-U` gave them, in order.
    defs: Vec<(String, Option<String>)>,
    charset: Option<String>,
    modules: Vec<Module>,
    image: Option<String>,
    nodef: bool,
    preinit: Option<bool>,
    debug: bool,
    warnings: bool,
    warnings_as_errors: bool,
    verbose: bool,
    source_text_group: bool,
    resources: Vec<ResourceSpec>,
    /// Whether a folder named after `-res` has its folders bundled too
    /// (`-recurse`, which is the default, and `-norecurse`).
    res_recurse: bool,
    messages: Vec<String>,
    lib_err: bool,
    opt_file: Option<String>,
    /// The folder of the project file, once it has been read: what it
    /// names is relative to that.
    opt_file_path: Option<String>,
}

/// Read what to build from the command line of `hexicon build`, which
/// takes what `t3make` takes, and from the project file it names.
pub fn read_project(
    files: &mut dyn SourceFiles,
    args: &[String],
    paths: &ProjectPaths,
) -> Result<Project, ProjectError> {
    let mut reader = Reader {
        files,
        user_source_paths: Vec::new(),
        sys_source_paths: paths.user_libraries.clone(),
        sys_library: paths.system_library.clone(),
        sys_include: paths.system_include.clone(),
        include_paths: Vec::new(),
        defs: Vec::new(),
        charset: None,
        modules: Vec::new(),
        image: None,
        nodef: false,
        preinit: None,
        debug: false,
        warnings: true,
        warnings_as_errors: false,
        verbose: false,
        source_text_group: false,
        resources: Vec::new(),
        res_recurse: true,
        messages: Vec::new(),
        lib_err: false,
        opt_file: None,
        opt_file_path: None,
    };
    reader.parse(args)?;
    // With nothing on the command line, a project file of the usual
    // name will do.
    if reader.modules.is_empty() && reader.opt_file.is_none() {
        const DEFAULT_PROJECT: &str = "makefile.t3m";
        if reader.files.exists(DEFAULT_PROJECT) {
            reader.opt_file = Some(DEFAULT_PROJECT.to_owned());
        } else {
            return Err(ProjectError::Usage(String::new()));
        }
    }
    if let Some(opt_file) = reader.opt_file.clone() {
        let opt_file = if reader.files.exists(&opt_file) {
            opt_file
        } else {
            def_ext(&opt_file, "t3m")
        };
        let Some(text) = reader.files.read(&opt_file) else {
            return Err(ProjectError::Failed(vec![format!(
                "error: unable to read option file \"{opt_file}\""
            )]));
        };
        let args = parse_opt_file(&text);
        reader.opt_file_path = Some(path_name(&opt_file).to_owned());
        reader.parse(&args)?;
    }
    if reader.lib_err {
        return Err(ProjectError::Failed(reader.messages));
    }
    reader.finish()
}

/// The arguments in a project file: words, or phrases in quotes with a
/// quote in them stuttered (`""`). A line that starts with `#` is a
/// comment, and everything from a `[Config:...]` line on is the
/// development system's, not the compiler's.
fn parse_opt_file(text: &[u8]) -> Vec<String> {
    let text = String::from_utf8_lossy(text);
    let mut args = Vec::new();
    let mut in_config = false;
    for line in text.lines() {
        let line = line.trim_end_matches(['\r', '\n']);
        let trimmed = line.trim_start();
        if trimmed.len() > 8 && trimmed[..8].eq_ignore_ascii_case("[config:") {
            in_config = true;
            continue;
        }
        if in_config || trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let mut chars = line.chars().peekable();
        loop {
            while chars.next_if(|c| c.is_whitespace()).is_some() {}
            let Some(&first) = chars.peek() else {
                break;
            };
            let mut arg = String::new();
            if first == '"' {
                chars.next();
                while let Some(c) = chars.next() {
                    // A quote stuttered is a quote; one alone ends it.
                    if c == '"' && chars.next_if_eq(&'"').is_none() {
                        break;
                    }
                    arg.push(c);
                }
            } else {
                while let Some(c) = chars.next_if(|c| !c.is_whitespace()) {
                    arg.push(c);
                }
            }
            args.push(arg);
        }
    }
    args
}

impl Reader<'_, '_> {
    /// What follows `-res`: files and folders, each as `file` or
    /// `file=name`, and `-recurse` and `-norecurse` for the folders
    /// after them.
    fn parse_resources(&mut self, args: &[String]) -> Result<(), ProjectError> {
        for arg in args {
            match arg.as_str() {
                "-recurse" => self.res_recurse = true,
                "-norecurse" => self.res_recurse = false,
                _ if arg.starts_with('-') => {
                    return Err(ProjectError::Usage(format!("unknown option \"{arg}\"")));
                }
                _ => {
                    let (file, alias) = match arg.split_once('=') {
                        Some((file, alias)) => (file, Some(alias.to_owned())),
                        None => (arg.as_str(), None),
                    };
                    self.resources.push(ResourceSpec {
                        file: self.relative(file),
                        alias,
                        recurse: self.res_recurse,
                    });
                }
            }
        }
        Ok(())
    }

    /// A path the project file gives, made relative to the folder the
    /// project file is in.
    fn relative(&self, name: &str) -> String {
        match &self.opt_file_path {
            None => name.to_owned(),
            Some(dir) => {
                let local = cvt_url_dir(name);
                if is_absolute(&local) {
                    local
                } else {
                    build_full_path(dir, &local)
                }
            }
        }
    }

    /// The value of a macro, as a library's `$(NAME)` sees it: the last
    /// `-D` or `-U` of the name says.
    fn look_up_pp_sym(&self, name: &str) -> Option<String> {
        let mut found = None;
        for (sym, value) in &self.defs {
            if sym == name {
                found = value.clone();
            }
        }
        found
    }

    /// Where a source or a library is: where its name says, if it is
    /// there, and otherwise along the source path.
    fn find_source(&mut self, source: &str, search: &str) -> String {
        if is_absolute(search) || self.files.exists(source) {
            return source.to_owned();
        }
        let dirs: Vec<String> = self
            .user_source_paths
            .iter()
            .chain(&self.sys_source_paths)
            .chain(std::iter::once(&self.sys_library))
            .cloned()
            .collect();
        for dir in dirs {
            let path = build_full_path(&dir, search);
            if self.files.exists(&path) {
                return path;
            }
        }
        source.to_owned()
    }

    fn add_module(
        &mut self,
        name: &str,
        orig_name: &str,
        search: String,
        origin: Origin,
        url: String,
    ) {
        self.modules.push(Module {
            orig_name: orig_name.to_owned(),
            source: def_ext(name, "t"),
            search,
            origin,
            url,
            excluded: false,
        });
    }

    fn maybe_add_include_path(&mut self, path: &str) {
        if !self.include_paths.iter().any(|have| have == path) && path != self.sys_include {
            self.include_paths.push(path.to_owned());
        }
    }

    fn parse(&mut self, args: &[String]) -> Result<(), ProjectError> {
        let mut at = 0;
        // The value of an option: the rest of the word, or the next one.
        let opt_arg = |at: &mut usize, len: usize| -> Option<String> {
            let arg = &args[*at];
            if arg.len() > len {
                Some(arg[len..].to_owned())
            } else if *at + 1 < args.len() {
                *at += 1;
                Some(args[*at].clone())
            } else {
                None
            }
        };
        let missing = |arg: &str| ProjectError::Usage(format!("option \"{arg}\" needs a value"));
        let bad = |arg: &str| ProjectError::Usage(format!("unknown option \"{arg}\""));
        let unsupported =
            |arg: &str| ProjectError::Usage(format!("option \"{arg}\" is not supported yet"));
        while at < args.len() && args[at].starts_with('-') {
            let arg = args[at].clone();
            if matches!(arg.as_str(), "-source" | "-lib" | "-res") {
                break;
            }
            let bytes = arg.as_bytes();
            let sub = bytes.get(2).copied().unwrap_or(0);
            match bytes.get(1).copied().unwrap_or(0) {
                b'f' => {
                    if self.opt_file.is_some() {
                        return Err(ProjectError::Failed(vec![
                            "error: only one option file (-f) is allowed".to_owned(),
                        ]));
                    }
                    self.opt_file = Some(opt_arg(&mut at, 2).ok_or_else(|| missing(&arg))?);
                }
                b'D' => {
                    let def = opt_arg(&mut at, 2).ok_or_else(|| missing(&arg))?;
                    let (name, value) = def.split_once('=').unwrap_or((&def, ""));
                    self.defs.push((name.to_owned(), Some(value.to_owned())));
                }
                b'U' => {
                    let name = opt_arg(&mut at, 2).ok_or_else(|| missing(&arg))?;
                    self.defs.push((name, None));
                }
                b'c' => match sub {
                    b's' => self.charset = Some(opt_arg(&mut at, 3).ok_or_else(|| missing(&arg))?),
                    _ => return Err(unsupported(&arg)),
                },
                // Hexicon builds everything every time.
                b'a' if matches!(sub, 0 | b'l') => {}
                b'v' => self.verbose = true,
                b'd' => self.debug = true,
                b'I' => {
                    let path = opt_arg(&mut at, 2).ok_or_else(|| missing(&arg))?;
                    let path = self.relative(&path);
                    self.include_paths.push(path);
                }
                b'o' => {
                    let path = opt_arg(&mut at, 2).ok_or_else(|| missing(&arg))?;
                    self.image = Some(self.relative(&path));
                }
                b'F' => {
                    if sub == b'C' {
                        // Folders for the symbol and object files,
                        // which Hexicon does not write.
                        at += 1;
                        continue;
                    }
                    let path = opt_arg(&mut at, 3).ok_or_else(|| missing(&arg))?;
                    let path = self.relative(&path);
                    match sub {
                        b'L' => self.sys_library = path,
                        b'I' => self.sys_include = path,
                        b's' => self.user_source_paths.push(path),
                        b'y' | b'o' => {}
                        b'a' => return Err(unsupported(&arg)),
                        _ => return Err(bad(&arg)),
                    }
                }
                b'n' => match arg.as_str() {
                    "-nopre" => self.preinit = Some(false),
                    "-nobanner" => {}
                    "-nodef" => self.nodef = true,
                    _ => return Err(bad(&arg)),
                },
                b'p' if arg == "-pre" => self.preinit = Some(true),
                b'q' if arg == "-q" => {}
                b'e' if matches!(arg.as_str(), "-errnum" | "-errnum+" | "-errnum-") => {}
                b's' if arg == "-statprefix" => {
                    opt_arg(&mut at, 11).ok_or_else(|| missing(&arg))?;
                }
                b's' if arg == "-statpct" => {}
                b'w' => match sub {
                    b'0' => self.warnings = false,
                    b'1' => self.warnings = true,
                    b'e' => match bytes.get(3).copied().unwrap_or(0) {
                        0 | b'+' => self.warnings_as_errors = true,
                        b'-' => self.warnings_as_errors = false,
                        _ => return Err(bad(&arg)),
                    },
                    b'2' | b'+' | b'-' => return Err(unsupported(&arg)),
                    _ => return Err(bad(&arg)),
                },
                b'?' | b'h' => return Err(ProjectError::Usage(String::new())),
                b'G' if arg == "-Gstg" => self.source_text_group = true,
                b'P' | b'O' | b'G' | b't' | b'q' => return Err(unsupported(&arg)),
                _ => return Err(bad(&arg)),
            }
            at += 1;
        }

        // The modules: sources and libraries.
        while at < args.len() {
            let arg = args[at].clone();
            let (mut is_source, mut is_lib) = (false, false);
            let name = if arg.starts_with('-') {
                match arg.as_str() {
                    "-source" => is_source = true,
                    "-lib" => is_lib = true,
                    "-res" => {
                        // The rest are resources.
                        return self.parse_resources(&args[at + 1..]);
                    }
                    _ => return Err(ProjectError::Usage(format!("unknown option \"{arg}\""))),
                }
                at += 1;
                match args.get(at) {
                    Some(name) => name.clone(),
                    None => {
                        return Err(ProjectError::Usage(format!(
                            "option \"{arg}\" needs a value"
                        )));
                    }
                }
            } else {
                arg.clone()
            };
            if !is_source && !is_lib {
                is_lib = name.len() > 3 && name[name.len() - 3..].eq_ignore_ascii_case(".tl");
            }
            let path = self.relative(&name);
            if !is_lib {
                // The first source names the image, if nothing else
                // has.
                if self.image.is_none() {
                    self.image = Some(format!("{}.t3", rem_ext(&path)));
                }
                self.add_module(
                    &path,
                    &name,
                    def_ext(&name, "t"),
                    Origin::Normal,
                    String::new(),
                );
            } else {
                let fname = def_ext(&path, "tl");
                let orig = def_ext(&name, "tl");
                let found = self.find_source(&fname, &orig);
                let first_lib_module = self.modules.len();
                let nodef = self.process_lib(&found, "");
                self.nodef |= nodef;
                // `-x name` after a library leaves one of its modules
                // out, by its name in the library.
                while args.get(at + 1).is_some_and(|next| next == "-x") {
                    at += 2;
                    let Some(url) = args.get(at) else {
                        return Err(ProjectError::Usage(
                            "option \"-x\" needs a value".to_owned(),
                        ));
                    };
                    if let Some(module) = self.modules[first_lib_module..]
                        .iter_mut()
                        .find(|module| module.url.eq_ignore_ascii_case(url))
                    {
                        module.excluded = true;
                    }
                }
            }
            at += 1;
        }
        Ok(())
    }

    /// Read a library, adding its sources to the modules, and its
    /// libraries' in their places. What comes back is whether it said
    /// `nodef`.
    fn process_lib(&mut self, lib_name: &str, lib_url: &str) -> bool {
        let full_name = def_ext(lib_name, "tl");
        let lib_path = path_name(lib_name).to_owned();
        self.maybe_add_include_path(&lib_path);
        let mut nodef = false;
        let Some(text) = self.files.read(&full_name) else {
            self.messages
                .push(format!("unable to open library file \"{full_name}\""));
            self.lib_err = true;
            return nodef;
        };
        let text = String::from_utf8_lossy(&text).into_owned();
        let say = |reader: &mut Self, line: usize, message: String| {
            reader
                .messages
                .push(format!("{full_name} ({line}): {message}"));
        };
        for (index, line) in text.lines().enumerate() {
            let linenum = index + 1;
            let line = line.trim_end_matches(['\r', '\n']);
            if line.len() >= 1023 {
                say(self, linenum, "line is too long".to_owned());
                self.lib_err = true;
                continue;
            }
            // `$(NAME)` is the value of the macro; `$$` is a dollar.
            let mut expanded = String::new();
            let mut rest = line;
            while let Some(at) = rest.find('$') {
                expanded.push_str(&rest[..at]);
                let after = &rest[at + 1..];
                if let Some(inner) = after.strip_prefix('(') {
                    if let Some(close) = inner.find(')') {
                        let name = &inner[..close];
                        match self.look_up_pp_sym(name) {
                            Some(value) => expanded.push_str(&value),
                            None => say(self, linenum, format!("undefined symbol \"{name}\"")),
                        }
                        rest = &inner[close + 1..];
                        continue;
                    }
                    say(
                        self,
                        linenum,
                        "invalid '$' sequence - use '$$' for a literal dollar sign character"
                            .to_owned(),
                    );
                    expanded.push('$');
                    rest = after;
                } else if let Some(after) = after.strip_prefix('$') {
                    expanded.push('$');
                    rest = after;
                } else {
                    say(
                        self,
                        linenum,
                        "invalid '$' sequence - use '$$' for a literal dollar sign character"
                            .to_owned(),
                    );
                    expanded.push('$');
                    rest = after;
                }
            }
            expanded.push_str(rest);
            let content = expanded.trim_start();
            if content.is_empty() || content.starts_with('#') {
                continue;
            }
            let name_end = content
                .find(|c: char| c == ':' || c.is_whitespace())
                .unwrap_or(content.len());
            let var_name = &content[..name_end];
            let after_name = content[name_end..].trim_start();
            let Some(value) = after_name.strip_prefix(':') else {
                if var_name.eq_ignore_ascii_case("nodef") {
                    nodef = true;
                } else {
                    say(self, linenum, "missing colon".to_owned());
                    self.lib_err = true;
                }
                continue;
            };
            let value = value.trim_start();
            let full = build_full_path(&lib_path, &cvt_url_dir(value));
            match var_name.to_ascii_lowercase().as_str() {
                "name" => {}
                "source" => {
                    let url = format!("{lib_url}{value}");
                    let search = def_ext(&full, "t");
                    self.add_module(&full, value, search, Origin::Lib(full_name.clone()), url);
                }
                "library" => {
                    let url = format!("{lib_url}{value}/");
                    nodef |= self.process_lib(&full, &url);
                }
                "resource" => {
                    // Named as the library writes it, and with what is
                    // in its folders if it is a folder.
                    self.resources.push(ResourceSpec {
                        file: full.clone(),
                        alias: Some(value.to_owned()),
                        recurse: true,
                    });
                }
                "needmacro" => {
                    let mut words = value.splitn(2, char::is_whitespace);
                    let macro_name = words.next().unwrap_or("");
                    let warning = words.next().unwrap_or("").trim_start();
                    if self.look_up_pp_sym(macro_name).is_none() {
                        let shown = &macro_name[..macro_name.len().min(128)];
                        say(
                            self,
                            linenum,
                            format!(
                                "library requires preprocessor symbol \"{shown}\" to be \
                                 defined; use -D {shown}=value to define it ({warning})"
                            ),
                        );
                    }
                }
                _ => say(self, linenum, format!("unknown definition \"{var_name}\"")),
            }
        }
        nodef
    }

    fn finish(mut self) -> Result<Project, ProjectError> {
        // What `-d` changes otherwise is done: no preinitialization by
        // default, and links to the resources.
        if self.debug {
            self.messages.push(
                "hexicon: the records for debugging (\"-d\") are not written yet; \
                 the image is built without them"
                    .to_owned(),
            );
        }
        // The system library's `_main` comes first, unless something
        // said not to.
        if !self.nodef {
            let name = build_full_path(&self.sys_library, "_main");
            self.modules.insert(
                0,
                Module {
                    orig_name: "_main".to_owned(),
                    source: def_ext(&name, "t"),
                    search: "_main.t".to_owned(),
                    origin: Origin::SysLib,
                    url: String::new(),
                    excluded: false,
                },
            );
        }
        // No two modules can have one name: their object files would
        // be one file.
        let mut problems = Vec::new();
        for (index, module) in self.modules.iter().enumerate() {
            let root = root_name(&module.source);
            let Some(earlier) = self.modules[..index]
                .iter()
                .find(|earlier| root_name(&earlier.source).eq_ignore_ascii_case(root))
            else {
                continue;
            };
            let lib_of = |origin: &Origin| match origin {
                Origin::Lib(lib) => lib.clone(),
                // The reference has no name for the system library
                // here.
                _ => "(null)".to_owned(),
            };
            let new_name = match &module.origin {
                Origin::Normal => format!("\"{}\"", module.source),
                origin => format!(
                    "\"{}\" (from library \"{}\")",
                    module.source,
                    lib_of(origin)
                ),
            };
            problems.push(match &earlier.origin {
                Origin::Normal => {
                    format!("error: module {new_name} the has same name as an existing module")
                }
                origin => format!(
                    "error: module {new_name} has the same name as an existing module from \
                     library \"{}\"",
                    lib_of(origin)
                ),
            });
        }
        if !problems.is_empty() {
            let mut messages = self.messages;
            messages.extend(problems);
            return Err(ProjectError::Failed(messages));
        }
        // Each module's place in the build counts those left out too.
        let modules = std::mem::take(&mut self.modules);
        let mut sources = Vec::new();
        let mut module_names = Vec::new();
        for (index, module) in modules.iter().enumerate() {
            if module.excluded {
                continue;
            }
            let path = self.find_source(&module.source, &module.search);
            sources.push(path);
            module_names.push((module.orig_name.clone(), index + 1));
        }
        // What the macros come to: the last `-D` or `-U` of a name.
        let mut defines: Vec<(String, String)> = Vec::new();
        let mut undefines: Vec<String> = Vec::new();
        for (name, value) in &self.defs {
            defines.retain(|(have, _)| have != name);
            undefines.retain(|have| have != name);
            match value {
                Some(value) => defines.push((name.clone(), value.clone())),
                None => undefines.push(name.clone()),
            }
        }
        let mut include_paths = self.include_paths;
        if !self.sys_include.is_empty() {
            include_paths.push(self.sys_include);
        }
        Ok(Project {
            sources,
            options: CompileOptions {
                include_paths,
                defines,
                undefines,
                default_charset: self.charset,
                source_text_group: self.source_text_group,
                module_names,
                ..CompileOptions::default()
            },
            image: self.image,
            preinit: self.preinit.unwrap_or(!self.debug),
            warnings: self.warnings,
            warnings_as_errors: self.warnings_as_errors,
            verbose: self.verbose,
            debug: self.debug,
            resources: self.resources,
            messages: self.messages,
        })
    }
}
