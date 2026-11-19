//! The `hexicon` command-line tool.

mod terminal;
mod windows_zones;

use std::fmt::Write as _;
use std::io::Write as _;
use std::path::{Component, Path, PathBuf};
use std::process::ExitCode;

use hexicon_bytecode::listing::{Names, write_function};
use hexicon_bytecode::{Extent, discover};
use hexicon_host::{FileError, FileInfo, Host, Input, Sandbox, file_attr, file_type};
use hexicon_image::{Image, Pool, Value};
use hexicon_vm::Vm;

const USAGE: &str = "\
usage: hexicon <command>

commands:
  run [options] <image.t3> [args...]
                               run a program; the options are:
      --norand                 make the same random choices on every run
      --plain                  treat a terminal as plain input and output:
                               80 columns, no pauses, and keys read as lines
      --nomore                 do not pause when a screenful has gone by
      -I, --script <file>      read input from a command script before
                               asking for any
      --sandbox <dir>          the folder the program's file access is
                               confined to (default: the image's folder)
      --safety <read><write>   how far file access is confined, as two
                               digits: 0 for anywhere, 1 for reading
                               anywhere but writing only in the sandbox,
                               2 for the sandbox only, 3 for reading in
                               the sandbox only, 4 for nowhere (default 22)
  dump <image.t3>              describe the contents of a program image file
  disasm <image.t3>            list the byte code of every function found
  disasm --summary <image.t3>  report how much of the code was accounted for
";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    if let ["run", rest @ ..] = args.as_slice() {
        let mut options = RunOptions {
            seed_random: true,
            plain: false,
            paging: true,
            script: None,
            sandbox: None,
            safety: (2, 2),
        };
        let mut rest = rest;
        loop {
            match rest {
                ["--norand", tail @ ..] => {
                    options.seed_random = false;
                    rest = tail;
                }
                ["--plain", tail @ ..] => {
                    options.plain = true;
                    rest = tail;
                }
                ["--nomore", tail @ ..] => {
                    options.paging = false;
                    rest = tail;
                }
                ["-I" | "--script", file, tail @ ..] => {
                    options.script = Some(file);
                    rest = tail;
                }
                ["--sandbox", dir, tail @ ..] => {
                    options.sandbox = Some(dir);
                    rest = tail;
                }
                ["--safety", levels, tail @ ..] => {
                    let digits: Vec<u8> = levels
                        .bytes()
                        .filter(|digit| (b'0'..=b'4').contains(digit))
                        .map(|digit| digit - b'0')
                        .collect();
                    options.safety = match (digits.as_slice(), levels.len()) {
                        ([both], 1) => (*both, *both),
                        ([read, write], 2) => (*read, *write),
                        _ => {
                            eprint!("{USAGE}");
                            return ExitCode::from(2);
                        }
                    };
                    rest = tail;
                }
                [path, program_args @ ..] if !path.starts_with('-') => {
                    return run(path, program_args, &options);
                }
                _ => {
                    eprint!("{USAGE}");
                    return ExitCode::from(2);
                }
            }
        }
    }
    let (path, result) = match args.as_slice() {
        ["dump", path] => (path, dump(path)),
        ["disasm", "--summary", path] => (path, disasm(path, true)),
        ["disasm", path] => (path, disasm(path, false)),
        ["help" | "--help" | "-h"] => {
            emit(USAGE);
            return ExitCode::SUCCESS;
        }
        _ => {
            eprint!("{USAGE}");
            return ExitCode::from(2);
        }
    };
    match result {
        Ok(text) => {
            emit(&text);
            ExitCode::SUCCESS
        }
        Err(message) => {
            eprintln!("hexicon: {path}: {message}");
            ExitCode::FAILURE
        }
    }
}

/// Write to standard output. A reader that stops early, such as a pager, is
/// not an error.
fn emit(text: &str) {
    let _ = std::io::stdout().lock().write_all(text.as_bytes());
}

/// How to run a program.
struct RunOptions<'a> {
    /// Seed the random number generator from the system.
    seed_random: bool,
    /// Make nothing of standard input and output being a terminal.
    plain: bool,
    /// Pause when a screenful of text has gone by.
    paging: bool,
    /// A command script to read input from.
    script: Option<&'a str>,
    /// The folder file access is confined to, if not the image's.
    sandbox: Option<&'a str>,
    /// The file safety levels for reading and for writing.
    safety: (u8, u8),
}

/// A host that writes program output to standard output, reads input
/// from standard input, and keeps the program's files beside its image,
/// as the reference interpreter does.
struct Console {
    /// The folder a file name without one is taken to be in.
    file_base: PathBuf,
    /// The folder the program's file access is confined to.
    sandbox: PathBuf,
    /// How many temporary file names have been handed out.
    temp_count: u32,
    /// The terminal, when that is what the program is being run in.
    terminal: Option<terminal::Terminal>,
    /// Make the program stop and resume at every read, to test that.
    stall: bool,
    stalled: bool,
}

impl Console {
    /// The file a program's name for it refers to.
    fn path(&self, name: &str) -> PathBuf {
        self.file_base.join(name)
    }
}

/// A path from its root, with `.` and `..` worked out by name alone, so
/// that it has an answer for a file that does not exist yet.
fn full_path(path: &Path) -> PathBuf {
    // An empty path is the current folder.
    let path = if path.as_os_str().is_empty() {
        Path::new(".")
    } else {
        path
    };
    let absolute = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
    let mut out = PathBuf::new();
    for part in absolute.components() {
        match part {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other),
        }
    }
    out
}

/// A path in the syntax programs use, with `/` between the parts.
fn program_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

/// Whether two paths are the same, on a system where the case of
/// letters does not tell names apart.
fn same_part(a: &std::ffi::OsStr, b: &std::ffi::OsStr) -> bool {
    if cfg!(windows) {
        a.to_string_lossy().to_lowercase() == b.to_string_lossy().to_lowercase()
    } else {
        a == b
    }
}

/// Whether `path` is `dir` or somewhere within it.
fn starts_with(path: &Path, dir: &Path) -> bool {
    let mut parts = path.components();
    dir.components().all(|part| {
        parts
            .next()
            .is_some_and(|other| same_part(other.as_os_str(), part.as_os_str()))
    })
}

fn seconds(time: std::io::Result<std::time::SystemTime>) -> Option<i64> {
    let since = time.ok()?.duration_since(std::time::UNIX_EPOCH).ok()?;
    Some(since.as_secs() as i64)
}

impl Host for Console {
    fn print(&mut self, text: &str) {
        match &mut self.terminal {
            Some(terminal) => terminal.print(text),
            None => emit(text),
        }
    }

    fn console_width(&mut self) -> Option<usize> {
        self.terminal.as_ref()?.width()
    }

    fn script_input(&mut self) {
        if let Some(terminal) = &mut self.terminal {
            terminal.start_page();
        }
    }

    fn random_bytes(&mut self, buffer: &mut [u8]) {
        // The standard library seeds each hasher state from the system's
        // entropy source, which is the only access to it without another
        // dependency.
        use std::hash::{BuildHasher, Hasher};
        for chunk in buffer.chunks_mut(8) {
            let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
            hasher.write_u64(0);
            let bytes = hasher.finish().to_le_bytes();
            chunk.copy_from_slice(&bytes[..chunk.len()]);
        }
    }

    // A file name is a path, relative to the folder the image is in.
    fn read_file(&mut self, name: &str) -> Result<Vec<u8>, FileError> {
        std::fs::read(self.path(name)).map_err(file_error)
    }

    fn write_file(&mut self, name: &str, data: &[u8]) -> Result<(), FileError> {
        std::fs::write(self.path(name), data).map_err(file_error)
    }

    fn file_info(&mut self, name: &str, follow_links: bool) -> Option<FileInfo> {
        let path = self.path(name);
        let meta = if follow_links {
            std::fs::metadata(&path)
        } else {
            std::fs::symlink_metadata(&path)
        }
        .ok()?;
        let kind = if meta.is_dir() {
            file_type::DIR
        } else if meta.is_file() {
            file_type::FILE
        } else if meta.file_type().is_symlink() {
            file_type::LINK
        } else {
            0
        };
        let hidden = Path::new(name)
            .file_name()
            .is_some_and(|base| base.to_string_lossy().starts_with('.'));
        let mut attrs = file_attr::READ;
        if !meta.permissions().readonly() {
            attrs |= file_attr::WRITE;
        }
        if hidden {
            attrs |= file_attr::HIDDEN;
        }
        Some(FileInfo {
            kind,
            attrs,
            size: if meta.is_dir() { 0 } else { meta.len() },
            accessed: seconds(meta.accessed()),
            modified: seconds(meta.modified()),
            created: seconds(meta.created()),
            link_target: std::fs::read_link(&path)
                .ok()
                .map(|target| program_path(&target)),
        })
    }

    fn delete_file(&mut self, name: &str) -> Result<(), FileError> {
        std::fs::remove_file(self.path(name)).map_err(file_error)
    }

    fn rename_file(&mut self, from: &str, to: &str) -> Result<(), FileError> {
        std::fs::rename(self.path(from), self.path(to)).map_err(file_error)
    }

    fn create_dir(&mut self, name: &str, parents: bool) -> Result<(), FileError> {
        let path = self.path(name);
        if path.exists() {
            return Err(FileError::Failed);
        }
        if parents {
            std::fs::create_dir_all(path)
        } else {
            std::fs::create_dir(path)
        }
        .map_err(file_error)
    }

    fn remove_dir(&mut self, name: &str, contents: bool) -> Result<(), FileError> {
        let path = self.path(name);
        if contents {
            std::fs::remove_dir_all(path)
        } else {
            std::fs::remove_dir(path)
        }
        .map_err(file_error)
    }

    // The entries for the directory itself and its parent come first, as
    // systems that list them do.
    fn list_dir(&mut self, name: &str) -> Option<Vec<String>> {
        let mut names = vec![".".to_owned(), "..".to_owned()];
        for entry in std::fs::read_dir(self.path(name)).ok()? {
            names.push(entry.ok()?.file_name().to_string_lossy().into_owned());
        }
        Some(names)
    }

    fn absolute_path(&mut self, name: &str) -> Option<String> {
        Some(program_path(&full_path(&self.path(name))))
    }

    fn root_dirs(&mut self) -> Vec<String> {
        if cfg!(windows) {
            (b'A'..=b'Z')
                .map(|letter| format!("{}:/", char::from(letter)))
                .filter(|root| Path::new(root).exists())
                .collect()
        } else {
            vec!["/".to_owned()]
        }
    }

    fn sandbox(&mut self, name: &str) -> Sandbox {
        let path = full_path(&self.path(name));
        let sandbox = full_path(&self.sandbox);
        let inside = starts_with(&path, &sandbox);
        let contains = starts_with(&sandbox, &path);
        match (inside, contains) {
            (true, true) => Sandbox::Itself,
            (true, false) => Sandbox::Inside,
            (false, true) => Sandbox::Contains,
            (false, false) => Sandbox::Outside,
        }
    }

    fn temp_file_name(&mut self) -> Option<String> {
        let dir = std::env::temp_dir();
        loop {
            self.temp_count += 1;
            let name = format!("hexicon-{}-{}.tmp", std::process::id(), self.temp_count);
            let path = dir.join(name);
            if !path.exists() {
                return Some(program_path(&path));
            }
        }
    }

    // The settings that games share are kept in the user's own
    // configuration folder.
    fn special_file(&mut self, id: i32) -> Option<String> {
        let name = match id {
            1 => "settings.txt",
            2 => "webprefs.txt",
            _ => return None,
        };
        let base = if cfg!(windows) {
            PathBuf::from(std::env::var_os("APPDATA")?)
        } else if let Some(config) = std::env::var_os("XDG_CONFIG_HOME") {
            PathBuf::from(config)
        } else {
            PathBuf::from(std::env::var_os("HOME")?).join(".config")
        };
        let dir = base.join(if cfg!(windows) { "Hexicon" } else { "hexicon" });
        // The folder has to be there before a file can be written in it.
        let _ = std::fs::create_dir_all(&dir);
        Some(program_path(&dir.join(name)))
    }

    fn append_file(&mut self, name: &str, data: &[u8]) -> Result<(), FileError> {
        std::fs::OpenOptions::new()
            .append(true)
            .create(true)
            .open(self.path(name))
            .and_then(|mut file| file.write_all(data))
            .map_err(file_error)
    }

    fn read_line(&mut self) -> Input {
        // For testing the VM's ability to stop and resume: have nothing
        // ready the first time each line is asked for.
        if self.stall {
            self.stalled = !self.stalled;
            if self.stalled {
                return Input::Pending;
            }
        }
        let _ = std::io::stdout().flush();
        if let Some(terminal) = &mut self.terminal {
            terminal.start_page();
        }
        let mut line = String::new();
        match std::io::stdin().read_line(&mut line) {
            Ok(0) | Err(_) => Input::End,
            Ok(_) => {
                let len = line.trim_end_matches(['\r', '\n']).len();
                line.truncate(len);
                Input::Text(line)
            }
        }
    }

    // A terminal delivers keys as they are pressed. Standard input
    // otherwise delivers whole lines, so a key is the first character of a
    // line, and an empty line is the Enter key.
    fn read_key(&mut self) -> Input {
        if let Some(terminal) = &mut self.terminal {
            return match terminal.read_key() {
                Some(key) => Input::Text(key),
                None => Input::End,
            };
        }
        match self.read_line() {
            Input::Text(line) => Input::Text(match line.chars().next() {
                Some(key) => key.to_string(),
                None => "\n".to_owned(),
            }),
            other => other,
        }
    }

    // A form feed, which is all a stream of text can do, and what the
    // reference interpreter's plain console writes.
    fn clear_screen(&mut self) {
        match &mut self.terminal {
            Some(terminal) => terminal.clear(),
            None => emit("\x0c\n"),
        }
    }

    // The system's name for its zone is found by asking it, since the
    // standard library does not know.
    fn local_time_zone(&mut self) -> Option<String> {
        if let Ok(zone) = std::env::var("TZ") {
            let zone = zone.trim_start_matches(':');
            if zone.contains('/') {
                return Some(zone.to_owned());
            }
        }
        if cfg!(windows) {
            // Windows has names of its own, which a table maps to the
            // database's.
            let output = std::process::Command::new("tzutil")
                .arg("/g")
                .output()
                .ok()?;
            let name = String::from_utf8_lossy(&output.stdout);
            let name = name.trim().trim_end_matches("_dstoff");
            windows_zones::WINDOWS_ZONES
                .iter()
                .find(|(windows, _)| *windows == name)
                .map(|(_, zone)| (*zone).to_owned())
        } else {
            // The system's zone is a link into the zoneinfo folder, or
            // is named in a file.
            if let Ok(target) = std::fs::read_link("/etc/localtime") {
                let target = target.to_string_lossy().into_owned();
                if let Some((_, zone)) = target.split_once("zoneinfo/") {
                    return Some(zone.to_owned());
                }
            }
            let name = std::fs::read_to_string("/etc/timezone").ok()?;
            Some(name.trim().to_owned()).filter(|name| !name.is_empty())
        }
    }

    // The standard library does not know the local time zone's offset,
    // so without a zone name local time is UTC.
    fn time(&mut self) -> hexicon_host::Time {
        let unix_millis = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_millis() as i64);
        hexicon_host::Time {
            unix_millis,
            utc_offset_seconds: 0,
        }
    }

    fn sleep(&mut self, milliseconds: u32) -> bool {
        let _ = std::io::stdout().flush();
        std::thread::sleep(std::time::Duration::from_millis(milliseconds.into()));
        true
    }
}

fn file_error(error: std::io::Error) -> FileError {
    match error.kind() {
        std::io::ErrorKind::NotFound => FileError::NotFound,
        std::io::ErrorKind::PermissionDenied => FileError::Denied,
        _ => FileError::Failed,
    }
}

/// Run a program. The program sees its own file name as its first
/// argument, followed by the ones given on the command line.
fn run(path: &str, program_args: &[&str], options: &RunOptions<'_>) -> ExitCode {
    let fail = |message: String| {
        eprintln!("hexicon: {path}: {message}");
        ExitCode::FAILURE
    };
    let data = match std::fs::read(path) {
        Ok(data) => data,
        Err(error) => return fail(error.to_string()),
    };
    let image = match Image::parse(&data) {
        Ok(image) => image,
        Err(error) => return fail(error.to_string()),
    };
    let file_base = Path::new(path)
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_default();
    let console = Console {
        sandbox: options
            .sandbox
            .map_or_else(|| file_base.clone(), PathBuf::from),
        file_base,
        temp_count: 0,
        terminal: if options.plain {
            None
        } else {
            terminal::Terminal::detect(options.paging)
        },
        stall: std::env::var_os("HEXICON_TEST_SUSPEND").is_some(),
        stalled: false,
    };
    let mut vm = match Vm::new(&image, Box::new(console)) {
        Ok(vm) => vm,
        Err(error) => return fail(error.to_string()),
    };
    vm.set_file_safety(options.safety.0, options.safety.1);
    for resource in &image.resources {
        if let Some(contents) = data.get(resource.offset..resource.offset + resource.size as usize)
        {
            vm.add_resource(&resource.name, contents);
        }
    }
    vm.set_error_trace(std::env::var_os("HEXICON_TRACE_ERRORS").is_some());
    vm.set_image_name(path);
    if options.seed_random {
        vm.seed_random();
    }
    if let Some(script) = options.script {
        match std::fs::read(script) {
            // Output is not paused while a script supplies the input.
            Ok(data) => vm.push_input_script(&data, false, false),
            Err(error) => {
                eprintln!("hexicon: {script}: {error}");
                return ExitCode::FAILURE;
            }
        }
    }
    // As under the reference interpreter, the program sees the image's
    // file name without its directory.
    let name = std::path::Path::new(path).file_name().map_or_else(
        || path.to_owned(),
        |name| name.to_string_lossy().into_owned(),
    );
    let mut args = vec![name];
    args.extend(program_args.iter().map(|arg| (*arg).to_owned()));
    let result = vm.run(&args);
    let code = match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            // The reference interpreter reports the error before it
            // writes out the last, unfinished line of output.
            let message = vm.error_message(&error, true);
            emit(&format!(
                "{message}
"
            ));
            ExitCode::FAILURE
        }
    };
    vm.flush_output();
    vm.close_files();
    code
}

fn load(path: &str) -> Result<(Image, usize), String> {
    let data = std::fs::read(path).map_err(|e| e.to_string())?;
    let image = Image::parse(&data).map_err(|e| e.to_string())?;
    Ok((image, data.len()))
}

fn dump(path: &str) -> Result<String, String> {
    let (image, file_size) = load(path)?;
    Ok(describe(&image, file_size))
}

fn disasm(path: &str, summary_only: bool) -> Result<String, String> {
    let (image, _) = load(path)?;
    let found = discover(&image);
    let mut out = String::new();

    if !summary_only {
        let names = Names::new(&image);
        for function in found.functions() {
            write_function(&mut out, &image, &names, function);
            out.push('\n');
        }
    }

    // How much of the code pool the functions account for: their headers,
    // code, exception tables, and debug records.
    let total = image.code_pool.data_len();
    let mut covered = 0usize;
    let mut instructions = 0usize;
    let mut gaps = 0usize;
    let mut gap_bytes = 0usize;
    for function in found.functions() {
        covered += function.size as usize;
        instructions += function.instructions.len();
        if let Extent::Gap { len, .. } = function.extent {
            gaps += 1;
            gap_bytes += len as usize;
        }
    }
    let errors: Vec<_> = found.errors().collect();
    let _ = writeln!(
        out,
        "functions      {}",
        found.by_address.len() - errors.len()
    );
    let _ = writeln!(out, "instructions   {instructions}");
    let _ = writeln!(
        out,
        "code covered   {covered} of {total} bytes ({:.1}%)",
        if total == 0 {
            100.0
        } else {
            covered as f64 * 100.0 / total as f64
        }
    );
    let _ = writeln!(out, "gaps           {gaps} ({gap_bytes} bytes not decoded)");
    let known = image.debug.method_headers.len();
    if known != 0 {
        let _ = writeln!(out, "debug list     {known} method headers in the image");
    }
    let stray: usize = found.functions().map(|f| f.stray_branches.len()).sum();
    let _ = writeln!(out, "stray branches {stray}");
    let _ = writeln!(out, "unreferenced   {}", found.unreferenced.len());
    let _ = writeln!(
        out,
        "dangling       {} references to discarded code",
        found.dangling.len()
    );
    let _ = writeln!(out, "failures       {}", errors.len());
    for (address, error) in errors.iter().take(20) {
        let _ = writeln!(out, "  {address:#x}: {error}");
    }
    Ok(out)
}

/// A plain-text description of an image.
fn describe(image: &Image, file_size: usize) -> String {
    let mut out = String::new();
    // Writing to a String cannot fail.
    let w = &mut out;

    let header = &image.header;
    let _ = writeln!(w, "format version   {}", header.version);
    let _ = writeln!(w, "created          {}", header.timestamp);
    let config = header.build_configuration;
    let _ = writeln!(
        w,
        "build config     {:02x}{:02x}{:02x}{:02x}",
        config[0], config[1], config[2], config[3]
    );
    let end = image.blocks.last().map_or(0, |b| b.offset + 10);
    let _ = writeln!(
        w,
        "image size       {end} bytes ({} after the image)",
        file_size - end
    );

    let entry = &image.entry_point;
    let _ = writeln!(w, "\nentry point      code offset {:#x}", entry.code_offset);
    let _ = writeln!(w, "method header    {} bytes", entry.method_header_size);
    let _ = writeln!(w, "exception entry  {} bytes", entry.exception_entry_size);
    let _ = writeln!(
        w,
        "debug records    version {}, line {} / table {} / local {} / frame {} bytes",
        entry.debug_version,
        entry.debug_line_entry_size,
        entry.debug_table_header_size,
        entry.debug_local_header_size,
        entry.debug_frame_header_size
    );

    let _ = writeln!(w, "\npools");
    pool(w, "code", &image.code_pool);
    pool(w, "constant", &image.constant_pool);

    let _ = writeln!(w, "\nmetaclasses ({})", image.metaclasses.len());
    for (index, metaclass) in image.metaclasses.iter().enumerate() {
        let objects: usize = image
            .object_blocks
            .iter()
            .filter(|b| b.metaclass_index as usize == index)
            .map(|b| b.objects.len())
            .sum();
        let _ = writeln!(
            w,
            "  {index:>3}  {:<34} {:>3} properties  {objects:>6} objects",
            metaclass.name,
            metaclass.properties.len()
        );
    }

    let _ = writeln!(w, "\nfunction sets ({})", image.function_sets.len());
    for (index, name) in image.function_sets.iter().enumerate() {
        let _ = writeln!(w, "  {index:>3}  {name}");
    }

    let transient: usize = image
        .object_blocks
        .iter()
        .filter(|b| b.transient)
        .map(|b| b.objects.len())
        .sum();
    let object_bytes: usize = image
        .object_blocks
        .iter()
        .flat_map(|b| &b.objects)
        .map(|o| o.data.len())
        .sum();
    let _ = writeln!(
        w,
        "\nstatic objects   {} in {} blocks ({} transient), {} bytes of data",
        image.object_count(),
        image.object_blocks.len(),
        transient,
        object_bytes
    );

    match &image.static_initializers {
        Some(list) => {
            let _ = writeln!(
                w,
                "static init      {} initializers, static code from {:#x}",
                list.initializers.len(),
                list.static_code_offset
            );
        }
        None => {
            let _ = writeln!(w, "static init      none (image is pre-initialized)");
        }
    }

    let _ = writeln!(w, "\nexported symbols ({})", image.symbols.len());
    for symbol in &image.symbols {
        let _ = writeln!(w, "  {:<28} {}", symbol.name, value(symbol.value));
    }

    if !image.resources.is_empty() {
        let _ = writeln!(w, "\nresources ({})", image.resources.len());
        for resource in &image.resources {
            let _ = writeln!(w, "  {:<40} {:>9} bytes", resource.name, resource.size);
        }
    }
    if !image.resource_links.is_empty() {
        let _ = writeln!(w, "\nresource links ({})", image.resource_links.len());
        for link in &image.resource_links {
            let _ = writeln!(w, "  {:<40} -> {}", link.name, link.file_name);
        }
    }

    let debug = &image.debug;
    if !debug.source_files.is_empty()
        || !debug.global_symbols.is_empty()
        || !debug.method_headers.is_empty()
        || !debug.macros.is_empty()
    {
        let lines: usize = debug.source_files.iter().map(|f| f.lines.len()).sum();
        let _ = writeln!(w, "\ndebug information");
        let _ = writeln!(
            w,
            "  source files    {} ({lines} line records)",
            debug.source_files.len()
        );
        let _ = writeln!(w, "  global symbols  {}", debug.global_symbols.len());
        let _ = writeln!(w, "  method headers  {}", debug.method_headers.len());
        let _ = writeln!(w, "  macros          {}", debug.macros.len());
    }

    let _ = writeln!(w, "\nblocks ({})", image.blocks.len());
    let _ = writeln!(w, "  type    offset       size  flags");
    for block in &image.blocks {
        let _ = writeln!(
            w,
            "  {}  {:>8}  {:>9}  {}",
            block.kind,
            block.offset,
            block.size,
            if block.is_mandatory() {
                "mandatory"
            } else {
                "optional"
            }
        );
    }
    out
}

fn pool(w: &mut String, name: &str, pool: &Pool) {
    let _ = writeln!(
        w,
        "  {name:<9} {:>4} pages of {:>5} bytes, {:>4} loaded, {:>9} bytes of data",
        pool.page_count(),
        pool.page_size(),
        pool.loaded_page_count(),
        pool.data_len()
    );
}

fn value(value: Value) -> String {
    match value {
        Value::Nil => "nil".into(),
        Value::True => "true".into(),
        Value::Empty => "empty".into(),
        Value::Object(id) => format!("object {id}"),
        Value::Property(id) => format!("property {id}"),
        Value::Int(n) => format!("integer {n}"),
        Value::Enum(id) => format!("enum {id}"),
        Value::String(offset) => format!("string at {offset:#x}"),
        Value::SelfPrintingString(offset) => format!("self-printing string at {offset:#x}"),
        Value::List(offset) => format!("list at {offset:#x}"),
        Value::CodeOffset(offset) => format!("code at {offset:#x}"),
        Value::FunctionPointer(offset) => format!("function at {offset:#x}"),
        Value::IntrinsicFunction { set, index } => format!("intrinsic function {set}:{index}"),
        Value::Other { type_code, raw } => format!("type {type_code}, raw {raw:02x?}"),
    }
}
