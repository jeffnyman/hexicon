//! The dynamic compiler: code compiled while a program runs, as
//! `CompiledCode` and the system library's `Compiler` object ask, against
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
    ProgramObject,
    /// An object that can be called like a function.
    Invokable,
    /// A grammar production.
    GrammarRule,
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
    /// For each of those classes, the properties of its methods, the
    /// first method's first.
    pub metaclass_props: Vec<Vec<u16>>,
    /// The variables of running functions the code can use, which hide
    /// global names. Of two with one name, the first counts.
    pub locals: Vec<DynLocal>,
    /// The innermost of those functions has a `self`, which code given
    /// as `function(...) { ... }` then has as well.
    pub frame_has_self: bool,
}

/// A variable of a running function, which the code reads and sets
/// through the object that stands for the function's call.
#[derive(Debug, Clone)]
pub struct DynLocal {
    /// The variable's name.
    pub name: Vec<u8>,
    /// The number of the object that stands for the call.
    pub frame_ref: u32,
    /// The variable's number among the call's: its locals, then its
    /// arguments.
    pub number: u16,
    /// For a variable shared with an anonymous function, its index in
    /// the context object the variable holds; otherwise 0.
    pub context: u16,
}

/// What of the method context of the function the code was compiled in
/// the code uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DynCtxUse {
    /// None of it.
    #[default]
    None,
    /// `self`.
    SelfOnly,
    /// All of it, as `loadctx` takes it.
    Full,
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
    /// What of the method context of the function it was compiled in the
    /// code uses, which its object keeps for it.
    pub ctx_use: DynCtxUse,
}

/// One way a phrase of a grammar production can be made up, as a rule
/// compiled while the program runs gives it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DynGramAlt {
    /// How much worse than another it is (`[badness 50]`).
    pub badness: i16,
    /// The things to match, in order.
    pub toks: Vec<DynGramTok>,
}

/// One thing to match in a rule, and the property the match goes in, or
/// 0.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DynGramTok {
    /// The property of the match object that gets what this matched.
    pub prop: u16,
    /// What it matches.
    pub kind: DynGramMatch,
}

/// What one thing in a rule matches.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DynGramMatch {
    /// A word, as it is written.
    Literal(Vec<u8>),
    /// Any word the dictionary has for a property.
    Speech(u16),
    /// Any word the dictionary has for one of several properties.
    SpeechList(Vec<u16>),
    /// A phrase of another production: its object.
    Prod(u32),
    /// A token of a kind: its enumerator.
    TokType(u32),
    /// Whatever is left of the input.
    Star,
}

/// Compile the rules of a grammar production, as `GrammarRule.addAlt`
/// gives them: alternatives as a `grammar` statement has them before its
/// colon, against the program's global names. The error messages, if it
/// cannot, a line each.
pub fn compile_grammar(source: &[u8], context: &DynContext) -> Result<Vec<DynGramAlt>, String> {
    use crate::parse::GramTokKind;
    let mut files = OneSource { text: source };
    let mut pp = crate::pp::Preprocessor::new(&mut files);
    pp.set_default_charset("utf-8");
    if !pp.set_source(SOURCE_NAME) {
        return Err("error: unable to read the source".to_owned());
    }
    pp.set_compile_mode();
    let parser = crate::parse::Parser::new_dynamic(&mut pp, context);
    let (prog, alts) = parser.parse_dynamic_grammar();
    let mut messages: Vec<String> = pp
        .take_diagnostics()
        .into_iter()
        .filter(|d| {
            matches!(
                d.severity,
                Severity::Error | Severity::Fatal | Severity::Internal
            )
        })
        .map(|d| format!("error: {}", d.message))
        .collect();
    let known = prog
        .dynamic
        .as_ref()
        .map_or(0, |dynamic| dynamic.known_props);
    for prop in &prog.props[known..] {
        messages.push(format!(
            "error: undefined name \"{}\"",
            crate::pp::show_name(&prop.name)
        ));
    }
    let Some(alts) = alts else {
        return Err(messages.join("\n"));
    };
    if !messages.is_empty() {
        return Err(messages.join("\n"));
    }
    let enum_ids = prog
        .dynamic
        .as_ref()
        .map(|dynamic| dynamic.enum_ids.clone())
        .unwrap_or_default();
    let prop_id = |prop: usize| prog.props[prop].id;
    Ok(alts
        .into_iter()
        .map(|alt| DynGramAlt {
            badness: alt.badness as i16,
            toks: alt
                .toks
                .into_iter()
                .map(|tok| DynGramTok {
                    prop: tok.prop.map_or(0, prop_id),
                    kind: match tok.kind {
                        GramTokKind::Literal(text) => DynGramMatch::Literal(text),
                        GramTokKind::Speech(prop) => DynGramMatch::Speech(prop_id(prop)),
                        GramTokKind::SpeechList(props) => {
                            DynGramMatch::SpeechList(props.into_iter().map(prop_id).collect())
                        }
                        GramTokKind::Prod(obj) => DynGramMatch::Prod(prog.objs[obj].id),
                        GramTokKind::TokType(sym) => {
                            DynGramMatch::TokType(enum_ids.get(sym).copied().unwrap_or(0))
                        }
                        GramTokKind::Star => DynGramMatch::Star,
                    },
                })
                .collect(),
        })
        .collect())
}

/// A value the compiler gives a property of an object it has the
/// program make.
#[derive(Debug, Clone)]
pub enum DynValue {
    /// An integer.
    Int(i32),
    /// A string.
    Str(Vec<u8>),
}

/// What the compiler asks of the program that is running.
pub trait DynHost {
    /// Numbers for so many new objects, which the compiled functions are
    /// to be: the code refers to them by these.
    fn new_func_ids(&mut self, count: usize) -> Vec<u32>;

    /// Make an object of the program's own kind, of the class given, with
    /// the properties given, and say its number. The code refers to it.
    fn new_object(&mut self, class: u32, props: &[(u16, DynValue)]) -> u32;
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

    // The objects the code needs, such as those that choose among the
    // alternatives of a `<<one of>>`, are made before its functions.
    if let Some(dynamic) = prog.dynamic.as_mut() {
        for (index, (class, props)) in std::mem::take(&mut dynamic.gen_objs)
            .into_iter()
            .enumerate()
        {
            let props: Vec<(u16, DynValue)> = props
                .into_iter()
                .map(|(prop, value)| (prog.props[prop].id, value))
                .collect();
            let id = host.new_object(class, &props);
            dynamic.gen_obj_ids.insert(gen_obj_id(index), id);
        }
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

    // Code given as a function, where the function it is compiled in has
    // a `self`, takes that function's method context as its own.
    let mut ctx_use = DynCtxUse::None;
    if let Some(&main) = bodies.first() {
        let body = &prog.bodies[main].0;
        if context.frame_has_self && !is_method && body.self_valid {
            if body.closure.full_ctx_referenced {
                ctx_use = DynCtxUse::Full;
            } else if body.closure.self_referenced {
                ctx_use = DynCtxUse::SelfOnly;
            }
        }
        if let Some(dynamic) = prog.dynamic.as_mut() {
            dynamic.main_ctx = (main, ctx_use);
        }
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
            "error: undefined name \"{}\"",
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
    Ok(DynCompiled {
        funcs,
        is_method,
        ctx_use,
    })
}

/// The number the code has for an object it needs until the program
/// has made it.
pub(crate) fn gen_obj_id(index: usize) -> u32 {
    0xF000_0000 + index as u32
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
    /// The variables of running functions: the object for the call, the
    /// variable's number, and its index in a context object or 0.
    pub locals: Vec<(u32, u16, u16)>,
    /// The innermost running function has a `self`.
    pub frame_has_self: bool,
    /// The properties of the methods of each class built into the VM
    /// that the image uses.
    pub meta_props: Vec<Vec<u16>>,
    /// The objects the code needs, to be made before code is generated:
    /// each one's class, and its properties, by index into the program's
    /// properties, with their values.
    pub gen_objs: Vec<(u32, Vec<(usize, DynValue)>)>,
    /// The numbers of those objects, by the number the code has for each
    /// until it is made.
    pub gen_obj_ids: HashMap<u32, u32>,
    /// The objects that can be called as functions, such as functions
    /// compiled before: indexes into the program's objects.
    pub func_objs: std::collections::HashSet<usize>,
    /// The body the source is, and what of the method context it takes
    /// from the function it was compiled in.
    pub main_ctx: (usize, DynCtxUse),
}
