//! The `hexicon` command-line tool.

use std::fmt::Write as _;
use std::process::ExitCode;

use hexicon_image::{Image, Pool, Value};

const USAGE: &str = "\
usage: hexicon <command>

commands:
  dump <image.t3>    describe the contents of a program image file
";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .as_slice()
    {
        ["dump", path] => match dump(path) {
            Ok(text) => {
                print!("{text}");
                ExitCode::SUCCESS
            }
            Err(message) => {
                eprintln!("hexicon: {path}: {message}");
                ExitCode::FAILURE
            }
        },
        ["help" | "--help" | "-h"] => {
            print!("{USAGE}");
            ExitCode::SUCCESS
        }
        _ => {
            eprint!("{USAGE}");
            ExitCode::from(2)
        }
    }
}

fn dump(path: &str) -> Result<String, String> {
    let data = std::fs::read(path).map_err(|e| e.to_string())?;
    let image = Image::parse(&data).map_err(|e| e.to_string())?;
    Ok(describe(&image, data.len()))
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
