//! The dynamic compiler: code compiled while a program runs, as
//! `DynamicFunc` and the system library's `Compiler` object ask, against
//! the names of the program that is running.
//!
//! The reference compiles such code as it compiles a program, with a few
//! differences that come of there being no image to add to: strings are
//! written into the code itself, lists are made when the code runs, a
//! function is called at the address it already has, and each function
//! nested in the code is an object of its own, which the code refers to
//! by its number. The names come from the program's table of global
//! names, and the macros from its table of macros.

use std::collections::HashMap;

use crate::pp::{LinkMacro, Severity, SourceFiles};

/// What a global name stands for in the program that is running.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DynSymbol {
    /// An object.
    Object {
        /// Its number.
        id: u32,
        /// What kind of object it is, as far as the compiler cares.
        kind: DynObjKind,
    },
    /// The object that stands for a class built into the VM.
    Class {
        /// The class's place in the image's list of classes.
        meta_index: u16,
        /// The object's number.
        id: u32,
    },
    /// A property, by its number.
    Property(u16),
    /// A function.
    Function {
        /// Its address in the code.
        addr: u32,
        /// How many arguments it needs.
        argc: usize,
        /// How many more it can take.
        opt_argc: usize,
        /// It takes any number more.
        varargs: bool,
    },
    /// A value of an enumeration, by its number.
    Enum(u32),
    /// A function built into the VM.
    BuiltIn {
        /// Its set.
        set: u16,
        /// Its place in the set.
        index: u16,
        /// How many arguments it needs.
        min_argc: usize,
        /// How many more it can take.
        opt_argc: usize,
        /// It takes any number more.
        varargs: bool,
    },
}

/// The kinds of object the compiler treats differently.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DynObjKind {
    /// An object the program defines.
    TadsObject,
    /// An object that can be called like a function.
    Invokable,
    /// A grammar production.
    GrammarProd,
    /// A dictionary.
    Dictionary,
    /// An object that holds what the program adds to a built-in class.
    ClassModifier,
    /// Any other.
    Other,
}

/// What the code is compiled against.
#[derive(Debug, Clone, Default)]
pub struct DynContext {
    /// The global names.
    pub symbols: Vec<(Vec<u8>, DynSymbol)>,
    /// The macros.
    pub macros: Vec<LinkMacro>,
    /// The classes built into the VM that the image uses, in its order:
    /// the VM's names for them, with their versions.
    pub metaclasses: Vec<Vec<u8>>,
}

/// How to read the source.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DynMode {
    /// `function(...) { ... }`, `method(...) { ... }`, or an expression,
    /// whichever the source starts like.
    Auto,
    /// An expression.
    Expression,
}

/// The code of one function compiled, and where in it the numbers of
/// objects are.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DynCode {
    /// The function: its header, its code, and its tables.
    pub code: Vec<u8>,
    /// The offsets of the objects' numbers in it.
    pub obj_refs: Vec<usize>,
}

/// What the dynamic compiler made.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DynCompiled {
    /// The functions, the one the source is first, then those nested in
    /// it, each with the number given for it.
    pub funcs: Vec<(u32, DynCode)>,
    /// The source was `method(...) { ... }`.
    pub is_method: bool,
}

/// What the compiler asks of the program that is running.
pub trait DynHost {
    /// Numbers for so many new objects, which the compiled functions are
    /// to be: the code refers to them by these.
    fn new_func_ids(&mut self, count: usize) -> Vec<u32>;
}

/// A source held in memory, for the preprocessor to read.
struct OneSource<'s> {
    text: &'s [u8],
}

/// The name the source goes by, which no file has.
const SOURCE_NAME: &str = "";

impl SourceFiles for OneSource<'_> {
    fn read(&mut self, name: &str) -> Option<Vec<u8>> {
        (name == SOURCE_NAME).then(|| self.text.to_vec())
    }

    fn exists(&mut self, name: &str) -> bool {
        name == SOURCE_NAME
    }
}

/// Compile source code while a program runs. A failure is the messages
/// the reference gives, a line each.
pub fn compile_dynamic(
    source: &[u8],
    context: &DynContext,
    mode: DynMode,
    host: &mut dyn DynHost,
) -> Result<DynCompiled, String> {
    let mut files = OneSource { text: source };
    let mut pp = crate::pp::Preprocessor::new(&mut files);
    pp.set_default_charset("utf-8");
    for mac in &context.macros {
        pp.define_macro(mac);
    }
    if !pp.set_source(SOURCE_NAME) {
        return Err("error: unable to read the source".to_owned());
    }
    pp.set_compile_mode();
    let parser = crate::parse::Parser::new_dynamic(&mut pp, context);
    let (mut prog, parsed) = parser.parse_dynamic(mode);
    // Only errors are reported: a line each.
    let mut messages = Vec::new();
    let note = |messages: &mut Vec<String>, severity: Severity, message: &str| {
        if matches!(
            severity,
            Severity::Error | Severity::Fatal | Severity::Internal
        ) {
            messages.push(format!("error: {message}"));
        }
    };
    for diagnostic in pp.take_diagnostics() {
        note(&mut messages, diagnostic.severity, &diagnostic.message);
    }
    let Some(is_method) = parsed else {
        return Err(messages.join("\n"));
    };
    if !messages.is_empty() {
        return Err(messages.join("\n"));
    }

    // A number for each function: the one the source is, then those in
    // it, in the order they will be made.
    let bodies: Vec<usize> = prog
        .top
        .iter()
        .filter_map(|item| match item {
            crate::parse::Top::Func(body) => Some(*body),
            crate::parse::Top::Obj(_) => None,
        })
        .collect();
    let ids = host.new_func_ids(bodies.len());
    if let Some(dynamic) = prog.dynamic.as_mut() {
        dynamic.body_ids = bodies.iter().copied().zip(ids.iter().copied()).collect();
    }

    let generated = crate::emit::generate(&mut prog, false, 0);
    for diagnostic in &generated.diagnostics {
        note(&mut messages, diagnostic.severity, &diagnostic.message);
    }
    // No name can be made in code compiled while the program runs: a
    // name that is nothing in the program is not a property, as it would
    // be in a program being compiled.
    let known = prog
        .dynamic
        .as_ref()
        .map_or(0, |dynamic| dynamic.known_props);
    for prop in &prog.props[known..] {
        messages.push(format!(
            "error: undefined symbol \"{}\"",
            crate::pp::show_name(&prop.name)
        ));
    }
    if !messages.is_empty() {
        return Err(messages.join("\n"));
    }
    if !generated.fixups.is_empty() || !generated.consts.is_empty() {
        return Err("error: this code cannot be compiled while the program runs yet".to_owned());
    }

    // Each function's code runs from its place to the next one's.
    let code = &generated.code;
    let mut funcs = Vec::new();
    for (index, &body) in bodies.iter().enumerate() {
        let Some(&anchor) = generated.body_anchor.get(&body) else {
            continue;
        };
        let start = generated.code_anchors[anchor].ofs;
        let end = generated
            .code_anchors
            .get(anchor + 1)
            .map_or(code.len(), |next| next.ofs);
        let obj_refs = generated
            .id_fixups
            .iter()
            .filter(|(ofs, kind)| *kind == crate::emit::IdKind::Obj && (start..end).contains(ofs))
            .map(|(ofs, _)| ofs - start)
            .collect();
        funcs.push((
            ids[index],
            DynCode {
                code: code[start..end].to_vec(),
                obj_refs,
            },
        ));
    }
    Ok(DynCompiled { funcs, is_method })
}

/// What the parser and code generator keep for dynamic code.
#[derive(Debug, Default)]
pub(crate) struct DynProgram {
    /// The functions of the running program, by their index in the
    /// program's functions: their addresses.
    pub func_addrs: HashMap<usize, u32>,
    /// The number each value of an enumeration has.
    pub enum_ids: Vec<u32>,
    /// The number of the object each function compiled is, by its body.
    pub body_ids: HashMap<usize, u32>,
    /// How many properties the program has: any made after these are for
    /// names it does not have.
    pub known_props: usize,
}
