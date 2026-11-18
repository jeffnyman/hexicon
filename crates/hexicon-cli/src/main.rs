//! The `hexicon` command-line tool.

use std::fmt::Write as _;
use std::io::Write as _;
use std::process::ExitCode;

use hexicon_bytecode::listing::{Names, write_function};
use hexicon_bytecode::{Extent, discover};
use hexicon_host::{FileError, Host};
use hexicon_image::{Image, Pool, Value};
use hexicon_vm::Vm;

const USAGE: &str = "\
usage: hexicon <command>

commands:
  run [--norand] <image.t3> [args...]
                               run a program; with --norand, random
                               choices are the same on every run
  dump <image.t3>              describe the contents of a program image file
  disasm <image.t3>            list the byte code of every function found
  disasm --summary <image.t3>  report how much of the code was accounted for
";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    match args.as_slice() {
        ["run", "--norand", path, program_args @ ..] => return run(path, program_args, false),
        ["run", path, program_args @ ..] => return run(path, program_args, true),
        _ => {}
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

/// A host that writes program output to standard output, and keeps the
/// program's files beside its image, as the reference interpreter does.
struct Console {
    /// The folder a file name without one is taken to be in.
    file_base: std::path::PathBuf,
}

impl Host for Console {
    fn print(&mut self, text: &str) {
        emit(text);
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
        std::fs::read(self.file_base.join(name)).map_err(file_error)
    }

    fn write_file(&mut self, name: &str, data: &[u8]) -> Result<(), FileError> {
        std::fs::write(self.file_base.join(name), data).map_err(file_error)
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
fn run(path: &str, program_args: &[&str], seed_random: bool) -> ExitCode {
    let fail = |message: String| {
        eprintln!("hexicon: {path}: {message}");
        ExitCode::FAILURE
    };
    let image = match load(path) {
        Ok((image, _)) => image,
        Err(message) => return fail(message),
    };
    let file_base = std::path::Path::new(path)
        .parent()
        .map(std::path::Path::to_path_buf)
        .unwrap_or_default();
    let mut vm = match Vm::new(&image, Box::new(Console { file_base })) {
        Ok(vm) => vm,
        Err(error) => return fail(error.to_string()),
    };
    vm.set_error_trace(std::env::var_os("HEXICON_TRACE_ERRORS").is_some());
    vm.set_image_name(path);
    if seed_random {
        vm.seed_random();
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
