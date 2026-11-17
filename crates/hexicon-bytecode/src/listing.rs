//! Text listings of disassembled functions.

use std::collections::HashMap;
use std::fmt::Write as _;

use hexicon_image::{GlobalSymbolKind, Image, Value};

use crate::decode::{Instruction, Operand, Payload};
use crate::function::{Extent, Function};
use crate::opcode::Opcode;

/// Names for the functions, objects, and properties of an image.
///
/// Names come from the image's debug symbols, when it has them, and from
/// the symbols every image exports to the VM.
#[derive(Debug, Clone, Default)]
pub struct Names {
    functions: HashMap<u32, String>,
    objects: HashMap<u32, String>,
    properties: HashMap<u16, String>,
    enums: HashMap<u32, String>,
}

impl Names {
    /// Collect the names an image provides.
    pub fn new(image: &Image) -> Self {
        let mut names = Self::default();
        for symbol in &image.symbols {
            match symbol.value {
                Value::FunctionPointer(address) | Value::CodeOffset(address) => {
                    names.functions.insert(address, symbol.name.clone());
                }
                Value::Object(id) => {
                    names.objects.insert(id, symbol.name.clone());
                }
                Value::Property(id) => {
                    names.properties.insert(id, symbol.name.clone());
                }
                _ => {}
            }
        }
        // Debug symbols are the program's own names, so they take priority.
        for symbol in &image.debug.global_symbols {
            let name = symbol.name.clone();
            match symbol.kind {
                GlobalSymbolKind::Function { code_offset, .. } => {
                    names.functions.insert(code_offset, name);
                }
                GlobalSymbolKind::Object { id, .. } => {
                    names.objects.insert(id, name);
                }
                GlobalSymbolKind::Property { id, .. } => {
                    names.properties.insert(id, name);
                }
                GlobalSymbolKind::Enumerator { id, .. } => {
                    names.enums.insert(id, name);
                }
                _ => {}
            }
        }
        names
    }

    /// The name of the function at a code pool address.
    pub fn function(&self, address: u32) -> Option<&str> {
        self.functions.get(&address).map(String::as_str)
    }

    fn function_label(&self, address: u32) -> String {
        self.function(address)
            .map_or_else(|| format!("function {address:#x}"), str::to_owned)
    }

    fn object_label(&self, id: u32) -> String {
        self.objects
            .get(&id)
            .map_or_else(|| format!("object {id}"), Clone::clone)
    }

    fn property_label(&self, id: u16) -> String {
        self.properties
            .get(&id)
            .map_or_else(|| format!("property {id}"), Clone::clone)
    }

    fn enum_label(&self, id: u32) -> String {
        self.enums
            .get(&id)
            .map_or_else(|| format!("enum {id}"), Clone::clone)
    }
}

/// Append a listing of one function.
pub fn write_function(out: &mut String, image: &Image, names: &Names, function: &Function) {
    // Writing to a String cannot fail.
    let header = &function.header;
    let _ = write!(out, "function {:#x}", function.address);
    if let Some(name) = names.function(function.address) {
        let _ = write!(out, "  {name}");
    }
    let _ = write!(
        out,
        "\n  parameters {}{}",
        header.param_count,
        if header.varargs { "+" } else { "" }
    );
    if header.optional_param_count != 0 {
        let _ = write!(out, " (+{} optional)", header.optional_param_count);
    }
    let _ = writeln!(
        out,
        ", locals {}, stack {}",
        header.local_count, header.stack_depth
    );

    for instruction in &function.instructions {
        let _ = write!(
            out,
            "  {:04x}  {:<16}",
            instruction.offset,
            instruction.opcode.name()
        );
        let text = operands(image, names, instruction);
        let _ = writeln!(out, "{}", text.trim_end());
    }

    for handler in &function.handlers {
        let class = if handler.exception_class == 0 {
            "anything".to_owned()
        } else {
            names.object_label(handler.exception_class)
        };
        let _ = writeln!(
            out,
            "  catch {class} in {:04x}..={:04x} -> {:04x}",
            handler.start, handler.end, handler.handler
        );
    }
    if let Extent::Gap { len, .. } = function.extent {
        let _ = writeln!(out, "  ; {len} bytes after this function were not decoded");
    }
}

fn operands(image: &Image, names: &Names, instruction: &Instruction) -> String {
    let mut parts: Vec<String> = instruction
        .operands
        .iter()
        .map(|operand| match *operand {
            Operand::Unsigned(n) => n.to_string(),
            Operand::Signed(n) => n.to_string(),
            Operand::Argc(n) => format!("argc {n}"),
            Operand::Local(n) => format!("local {n}"),
            Operand::Arg(n) => format!("arg {n}"),
            Operand::String(offset) => string_label(image, offset),
            Operand::List(offset) => format!("list {offset:#x}"),
            Operand::Object(id) => names.object_label(id),
            Operand::Property(id) => names.property_label(id),
            Operand::Function(address) => names.function_label(address),
            Operand::Enum(id) => names.enum_label(id),
            Operand::Branch(target) => format!("-> {target:04x}"),
            Operand::Table(offset) => format!("table {offset:04x}"),
            Operand::Metaclass(index) => image.metaclasses.get(index as usize).map_or_else(
                || format!("metaclass {index}"),
                |m| m.base_name().to_owned(),
            ),
            Operand::FunctionSet(index) => function_set_label(image, index),
            Operand::FunctionIndex(index) => format!("#{index}"),
        })
        .collect();

    // The four short built-in calls name their function set in the opcode.
    let implied_set = match instruction.opcode {
        Opcode::BuiltinA => Some(0),
        Opcode::BuiltinB => Some(1),
        Opcode::BuiltinC => Some(2),
        Opcode::BuiltinD => Some(3),
        _ => None,
    };
    if let Some(set) = implied_set {
        parts.push(function_set_label(image, set));
    }

    match &instruction.payload {
        Payload::None => {}
        Payload::InlineString(text) => parts.push(quote(text)),
        Payload::NamedArgs(args) => parts.push(args.join(", ")),
        Payload::Switch { cases, default } => {
            for (value, target) in cases {
                parts.push(format!(
                    "{} -> {target:04x}",
                    value_label(image, names, *value)
                ));
            }
            parts.push(format!("default -> {default:04x}"));
        }
    }
    parts.join(", ")
}

fn function_set_label(image: &Image, index: u16) -> String {
    image.function_sets.get(index as usize).map_or_else(
        || format!("function set {index}"),
        |name| name.split('/').next().unwrap_or(name).to_owned(),
    )
}

fn value_label(image: &Image, names: &Names, value: Value) -> String {
    match value {
        Value::Nil => "nil".into(),
        Value::True => "true".into(),
        Value::Empty => "empty".into(),
        Value::Int(n) => n.to_string(),
        Value::Object(id) => names.object_label(id),
        Value::Property(id) => format!("&{}", names.property_label(id)),
        Value::Enum(id) => names.enum_label(id),
        Value::String(offset) | Value::SelfPrintingString(offset) => string_label(image, offset),
        Value::List(offset) => format!("list {offset:#x}"),
        Value::CodeOffset(address) | Value::FunctionPointer(address) => {
            names.function_label(address)
        }
        Value::IntrinsicFunction { set, index } => {
            format!("{} #{index}", function_set_label(image, set))
        }
        Value::Other { type_code, .. } => format!("type {type_code}"),
    }
}

/// A constant string: a 16-bit length followed by its text.
fn string_label(image: &Image, offset: u32) -> String {
    let text = image.constant_pool.at(offset).and_then(|data| {
        let len = u16::from_le_bytes([*data.first()?, *data.get(1)?]) as usize;
        data.get(2..2 + len)
    });
    match text {
        Some(text) => quote(text),
        None => format!("string {offset:#x}"),
    }
}

/// Quote text for a listing, shortening long strings.
fn quote(text: &[u8]) -> String {
    const LIMIT: usize = 48;
    let text = String::from_utf8_lossy(text);
    let mut out = String::from("\"");
    for (count, c) in text.chars().enumerate() {
        if count == LIMIT {
            out.push_str("...");
            break;
        }
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            c if c.is_control() => {
                let _ = write!(out, "\\x{:02x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}
