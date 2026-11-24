//! The code generator: a parsed program in, byte code out.
//!
//! Each kind of node in the syntax tree generates its code the way the
//! reference compiler's does, since the instruction writer's peephole
//! rules, the count of stack slots, and the tables after each function
//! all depend on exactly what is written and in what order.

use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use hexicon_bytecode::{Layout, Opcode as Op};

use crate::ast::{
    Arg, AsiOp, BinOp, Catch, Closure, CodeBody, ConstVal, Expr, Finally, ForIn, InlineProp,
    InlineValue, LabelSym, LocalId, Stmt, StmtKind, TabId, UnOp, Value, flow,
};
use crate::codegen::{CodeGen, Label};
use crate::flow::Flow;
use crate::fold::{self, Folded};
use crate::parse::{
    BifSym, Global, GramAlt, GramTokKind, Local, ObjDef, Program, PropValue, Top, enum_order,
};
use crate::pp::{MAX_ERRORS, Severity, TOO_MANY_ERRORS, show_name};

/// The start of one item in a stream: a function in the code, or a
/// string in the constants. An item is never split across pages.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Anchor {
    pub ofs: usize,
}

/// What an address in the code refers to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Target {
    /// A function: an index into the program's functions.
    Func(usize),
    /// An item in the constants: an index into the constants' anchors.
    Const(usize),
    /// A method's code: an index into the program's code bodies.
    Body(usize),
    /// The same, for code that runs before the program starts, which
    /// is kept apart from the rest.
    StaticBody(usize),
    /// A function as another source file defined it, before this
    /// file's `modify` put a definition in front of that one: an index
    /// into the program's functions.
    FuncBase(usize),
    /// An item in the code, whatever name it has now: an index into
    /// the code's anchors.
    Anchor(usize),
}

/// A word in a dictionary: its text, and each object it is a word for
/// with the property.
pub(crate) type DictWord = (Vec<u8>, Vec<(u32, u16)>);

/// An object as it goes into the image.
#[derive(Debug)]
pub(crate) struct GenObj {
    pub id: u32,
    pub is_class: bool,
    pub superclasses: Vec<u32>,
    pub props: Vec<GenProp>,
    /// It holds what the program adds to a built-in class.
    pub icmod: bool,
    /// It is not saved or restored with the rest.
    pub transient: bool,
}

/// A property of an object: its number, and its value as a kind and
/// four bytes. If the value is an address, `target` says of what.
#[derive(Debug)]
pub(crate) struct GenProp {
    pub id: u16,
    pub kind: u8,
    pub value: u32,
    pub target: Option<Target>,
}

/// What a number in the code or the constants is the number of.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum IdKind {
    /// A property: two bytes.
    Prop,
    /// An object: four bytes.
    Obj,
    /// A value of an enumeration: four bytes.
    Enum,
}

/// Four bytes in the code that are to hold an address once it is known
/// where everything goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Fixup {
    pub ofs: usize,
    pub target: Target,
}

/// The kinds of value in a constant list, as the virtual machine
/// numbers them.
mod datatype {
    pub(super) const NIL: u8 = 1;
    pub(super) const TRUE: u8 = 2;
    pub(super) const OBJ: u8 = 5;
    pub(super) const PROP: u8 = 6;
    pub(super) const INT: u8 = 7;
    pub(super) const SSTRING: u8 = 8;
    pub(super) const DSTRING: u8 = 9;
    pub(super) const LIST: u8 = 10;
    pub(super) const CODEOFS: u8 = 11;
    pub(super) const FUNCPTR: u8 = 12;
    pub(super) const ENUM: u8 = 15;
    /// Not a kind of value the image has: the place of a list of the
    /// words an object has for a dictionary property, until the list
    /// is made.
    pub(super) const VOCAB_LIST: u8 = 255;
}

/// The number of the list metaclass among those every image has.
const METACLASS_LIST: u8 = 1;
const METACLASS_VECTOR: u8 = 4;
const METACLASS_ANON_FN: u8 = 5;
const METACLASS_LOOKUP_TABLE: u8 = 7;
/// The number of the property that a production's words are its words
/// by, `miscVocab`.
const MISC_VOCAB_PROP_ID: u16 = 5;
/// The place in a context of the `self` of the method it was made in.
const CTX_METHOD_CTX: u8 = 1;
/// Which part of what a function knows about its call is the function
/// object itself.
const CTX_ELE_INVOKEE: u8 = 4;

/// A problem found while generating, and where.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct GenDiagnostic {
    pub file: usize,
    pub line: u32,
    pub severity: Severity,
    pub message: String,
}

/// What the generator produces.
#[derive(Debug, Default)]
pub(crate) struct Generated {
    /// The code of all the functions, one after another.
    pub code: Vec<u8>,
    pub code_anchors: Vec<Anchor>,
    /// Where each function's code is: an index into `code_anchors`.
    pub func_anchor: HashMap<usize, usize>,
    /// Where each code body's code is, functions and methods alike.
    pub body_anchor: HashMap<usize, usize>,
    /// The code that works out the values of static properties before
    /// the program starts, with what [`Generated::code`] has for the
    /// rest of the code.
    pub static_code: Vec<u8>,
    pub static_anchors: Vec<Anchor>,
    pub static_body_anchor: HashMap<usize, usize>,
    pub static_fixups: Vec<Fixup>,
    pub static_id_fixups: Vec<(usize, IdKind)>,
    pub static_unspecified: Vec<usize>,
    /// The properties that code is the value of: each an object's
    /// number and a property's.
    pub static_inits: Vec<(u32, u16)>,
    /// The objects, in the order they were generated.
    pub objs: Vec<GenObj>,
    /// For each built-in class in `metas`, the object that holds what
    /// the program adds to it, or zero.
    pub meta_mod_objs: Vec<u32>,
    /// The number the next property would get.
    pub next_prop_id: u16,
    /// The generator gave up for too many errors.
    pub gave_up: bool,
    /// Places in the code that refer to a local's name, in the order
    /// they were written. The names go into the constants when the
    /// program is put together.
    pub local_names: Vec<(usize, Vec<u8>)>,
    /// Which object in `objs` each object definition became.
    pub def_objs: HashMap<usize, usize>,
    /// The number each value of an enumeration has in this code, by
    /// its index in the program's.
    pub enum_ids: Vec<u32>,
    /// What the image exports by name, besides what every image does.
    pub exports: Vec<GenExport>,
    /// The built-in classes the image depends on: the virtual
    /// machine's name for each, the numbers of the properties the
    /// program declared it to have, and the number of the object that
    /// stands for the class, if the program declared it.
    pub metas: Vec<(Vec<u8>, Vec<u16>, Option<u32>)>,
    /// Where in the code there are the numbers of properties, objects,
    /// and values of enumerations, and where in the constants. They
    /// are the numbers the source file had for them when it was
    /// compiled; the numbers they have in the image are given when the
    /// program is put together.
    pub id_fixups: Vec<(usize, IdKind)>,
    pub const_id_fixups: Vec<(usize, IdKind)>,
    /// Places in the constants where two bytes are written that mean
    /// nothing.
    pub const_unspecified2: Vec<usize>,
    /// The constants: strings, each with its length before it.
    pub consts: Vec<u8>,
    pub const_anchors: Vec<Anchor>,
    pub fixups: Vec<Fixup>,
    /// Addresses to fill in within the constants, where a list has a
    /// string, a list, or a function in it.
    pub const_fixups: Vec<Fixup>,
    /// The length of the longest function, tables included.
    pub max_bytecode_len: usize,
    /// The length of the longest string.
    pub max_str_len: usize,
    /// The number of elements in the longest list.
    pub max_list_cnt: usize,
    /// The numbers that are not integers, each an object: its number,
    /// and its data as the virtual machine keeps it.
    pub bignums: Vec<(u32, Vec<u8>)>,
    /// The regular expressions, each an object: its number, and where
    /// its text is in the constants.
    pub rexpats: Vec<(u32, Target)>,
    /// The dictionaries, each an object: its number, and its words in
    /// the order the image has them, each with the objects it is a
    /// word for and the property it is their word by.
    pub dicts: Vec<(u32, Vec<DictWord>)>,
    /// The grammar productions, each an object: its number, and its
    /// data as the image has it.
    pub gramprods: Vec<(u32, Vec<u8>)>,
    /// What was found wrong when the program was put together, that
    /// is nowhere in particular in the source.
    pub link_errors: Vec<String>,
    /// The code that tells the library of the versions of
    /// multi-methods has been generated, with the rest of the code
    /// that runs before the program starts.
    pub has_mm_init: bool,
    /// The items in the code that are not to go in the image, a
    /// function in another source file having taken their place with
    /// `replace`: indexes into `code_anchors`.
    pub dead_code: Vec<usize>,
    /// The functions taken out of the code for that, in order: how many
    /// of the functions kept came before each, and its code. They are
    /// not in the image, but the reference keeps a place for each in
    /// its records for debugging.
    pub dropped_code: Vec<(usize, Vec<u8>)>,
    /// The objects that are not to go in the image, likewise: indexes
    /// into `objs`.
    pub dead_objs: HashSet<usize>,
    /// Places in the code where two bytes are written that mean
    /// nothing.
    pub unspecified: Vec<usize>,
    /// Places in the constants where four bytes are written that mean
    /// nothing.
    pub const_unspecified: Vec<usize>,
    pub diagnostics: Vec<GenDiagnostic>,
}

/// How many chains the reference's table of a function's labels has,
/// and its table of global names.
const LABEL_HASH_SIZE: usize = 512;

/// A name the image makes known to the virtual machine, with its value
/// as a kind and four bytes. For a function the value is its address,
/// which is not known yet.
#[derive(Debug)]
pub(crate) struct GenExport {
    pub name: Vec<u8>,
    pub kind: u8,
    pub value: u32,
    pub func: Option<usize>,
}

/// Work out what the program's `export` statements export. The
/// reference does this when it writes the image, so what is wrong
/// with them is reported last, and nowhere in particular.
///
/// A name exported that is nothing else is taken to be a property; such
/// names are returned too, in the order they were found, with their
/// numbers, for the table of global names.
pub(crate) fn resolve_exports(prog: &Program, generated: &mut Generated) -> ResolvedExports {
    let mut problems = Vec::new();
    let mut errors = Vec::new();
    // A name exported that is nothing else is a property.
    let mut assumed: HashMap<&[u8], u16> = HashMap::new();
    let mut assumed_order: Vec<(Vec<u8>, u16)> = Vec::new();
    let mut done: Vec<&[u8]> = Vec::new();
    for (index, (sym, external)) in prog.exports.iter().enumerate() {
        // An external name is exported once: the first time.
        if done.contains(&external.as_slice()) {
            continue;
        }
        done.push(external);
        if external == b"LastProp"
            || external == b"Constructor"
            || external == b"Destructor"
            || external.starts_with(b"operator ")
        {
            errors.push(format!(
                "external name \"{}\" illegal in 'export' - reserved for compiler use",
                show_name(external)
            ));
        }
        problems.extend(errors.drain(..).map(|message| (Severity::Error, message)));
        if !prog.globals.contains_key(sym) && !assumed.contains_key(sym.as_slice()) {
            problems.push((
                Severity::Warning,
                format!(
                    "undefined symbol \"{}\" - assuming this is a property name",
                    show_name(sym)
                ),
            ));
            assumed.insert(sym, generated.next_prop_id);
            assumed_order.push((sym.clone(), generated.next_prop_id));
            generated.next_prop_id += 1;
        }
        let mut dups = 0;
        for (other_sym, other_external) in &prog.exports[index + 1..] {
            if other_external != external || other_sym == sym {
                continue;
            }
            dups += 1;
            errors.push(if dups == 1 {
                format!(
                    "duplicate export for external name \"{}\": \"{}\" and \"{}\"",
                    show_name(external),
                    show_name(sym),
                    show_name(other_sym)
                )
            } else {
                format!(
                    "additional duplicate export for external name \"{}\": \"{}\"",
                    show_name(external),
                    show_name(other_sym)
                )
            });
        }
        let (kind, value, func) = match prog.globals.get(sym) {
            Some(Global::Prop(prop)) => (datatype::PROP, u32::from(prog.props[*prop].id), None),
            Some(Global::Obj(obj)) => (datatype::OBJ, prog.objs[*obj].id, None),
            Some(Global::Func(func)) => (datatype::FUNCPTR, 0, Some(*func)),
            Some(_) => {
                errors.push(format!(
                    "symbol \"{}\" is not a valid type for export",
                    show_name(sym)
                ));
                problems.extend(errors.drain(..).map(|message| (Severity::Error, message)));
                continue;
            }
            None => (datatype::PROP, u32::from(assumed[sym.as_slice()]), None),
        };
        generated.exports.push(GenExport {
            name: external.clone(),
            kind,
            value,
            func,
        });
        problems.extend(errors.drain(..).map(|message| (Severity::Error, message)));
    }
    ResolvedExports {
        problems,
        assumed_props: assumed_order,
    }
}

/// What [`resolve_exports`] found: what is wrong, and the names taken to
/// be properties, in order, with their numbers.
pub(crate) struct ResolvedExports {
    pub problems: Vec<(Severity, String)>,
    pub assumed_props: Vec<(Vec<u8>, u16)>,
}

/// The functions and objects that the program refers to and never
/// defines, in the order the reference reports them: the order of its
/// table of global names once it has put the program together.
pub(crate) fn unresolved_externs(prog: &Program, generated: &Generated) -> Vec<Vec<u8>> {
    prog.globals
        .in_table_order()
        .into_iter()
        .filter(|(_, global)| match global {
            Global::Func(func) => !generated.func_anchor.contains_key(func),
            Global::Obj(obj) => prog.objs[*obj].is_extern,
            _ => false,
        })
        .map(|(name, _)| name.to_vec())
        .collect()
}

/// Strings shorter than this are stored once however often they occur.
const INTERN_THRESHOLD: usize = 40;

/// A statement around the one being generated that a jump out of it
/// has to know about.
#[derive(Clone, Copy)]
enum Enclosing {
    /// A loop: where `break` and `continue` go.
    Loop {
        id: usize,
        break_lbl: Label,
        cont_lbl: Label,
    },
    /// A `switch`: where `break` goes, if anything after the switch can
    /// be reached.
    Switch { id: usize, break_lbl: Option<Label> },
    /// A labeled statement.
    Label {
        id: usize,
        /// The label, unless its name was already taken.
        label: Option<usize>,
        /// Where a `break` that names the label goes, once there has
        /// been one.
        break_lbl: Option<Label>,
        /// The statement labeled is a loop, so `continue` can name the
        /// label.
        labels_loop: bool,
    },
    /// The protected part of a `try`, or one of its `catch` blocks. The
    /// way out is through the `finally` block, if there is one.
    Try {
        id: usize,
        finally_lbl: Option<Label>,
        /// Where the `finally` block ends, for the line record after a
        /// call to it.
        finally_end: Option<(usize, u32)>,
    },
    /// A `finally` block.
    Finally { id: usize },
}

impl Enclosing {
    fn id(&self) -> usize {
        match self {
            Enclosing::Loop { id, .. }
            | Enclosing::Switch { id, .. }
            | Enclosing::Label { id, .. }
            | Enclosing::Try { id, .. }
            | Enclosing::Finally { id } => *id,
        }
    }
}

/// The table of a `switch` being generated: where the next `case` and
/// the `default` write where they are.
struct SwitchSlots {
    next_case: usize,
    default_slot: usize,
}

/// One entry of a function's table of exception handlers. The offsets
/// are from the start of the function.
struct ExcEntry {
    start_ofs: usize,
    end_ofs: usize,
    /// The class of exception caught; zero for all of them.
    class: u32,
    handler_ofs: usize,
}

/// What a call passes: how many arguments by position, whether any of
/// them is a list to be spread into arguments, and, if any are passed
/// by name, which of the function's lists of such names is theirs.
#[derive(Clone, Copy, Default)]
struct CallArgs {
    argc: usize,
    varargs: bool,
    named: Option<usize>,
}

struct Gen<'p> {
    prog: &'p mut Program,
    cg: CodeGen,
    out: Generated,
    /// Where the function being generated starts.
    method_ofs: usize,
    /// The scope whose code is being generated, and the function's
    /// scopes in the order they were first entered.
    cur_frame: Option<TabId>,
    frames: Vec<TabId>,
    /// The same for the code that runs before the program starts, which
    /// the reference generates in a stream of its own, with a current
    /// scope of its own.
    static_frame: Option<TabId>,
    enclosing: Vec<Enclosing>,
    switches: Vec<SwitchSlots>,
    /// The labels of the function being generated, and where each one
    /// is, once something has asked.
    labels: Rc<Vec<LabelSym>>,
    /// The labels of the last function in the source.
    last_labels: Rc<Vec<LabelSym>>,
    goto_labels: Vec<Option<Label>>,
    /// The number each value of an enumeration has in the image.
    enum_ids: Vec<u32>,
    exc_table: Vec<ExcEntry>,
    /// The names of the arguments passed by name in each call of the
    /// function being generated that has any, and the tables of names
    /// to be written after the function's code: a label for each, and
    /// which call's names.
    named_lists: Vec<Vec<Vec<u8>>>,
    named_tabs: Vec<(Label, usize)>,
    /// Names are being looked up in an expression, and nothing else
    /// done to it.
    resolve_only: bool,
    /// How many locals the function has. Generating its code can add to
    /// what the parser counted.
    local_cnt: usize,
    /// The local that holds a value being returned while `finally`
    /// blocks run.
    fin_ret_lcl: Option<usize>,
    /// Where the statement being generated is, for messages.
    cur_file: usize,
    cur_line: u32,
    /// Where the reference's tokenizer was last told the code being
    /// generated is: as `cur_file` and `cur_line`, but moved only by
    /// statements and properties, and not by the start of an object. A
    /// constructor the compiler makes ends there.
    line_info: (usize, u32),
    interned: HashMap<Vec<u8>, usize>,
    /// Places in the code that refer to a local's name, in the order
    /// they were written. The names go into the constants after
    /// everything else.
    local_names: Vec<(usize, Vec<u8>)>,
    errors: usize,
    /// The undefined names reported so far.
    undefined_names: HashSet<Vec<u8>>,
    /// The code being generated is a method's: it has a `self`.
    self_available: bool,
    /// The code being generated is a constructor's.
    in_constructor: bool,
    /// What an assignment to `obj.prop` that changes the value in
    /// place (`+=`) learned when it fetched the value, kept for when
    /// it stores the new one.
    member_ctx: Vec<(bool, Option<u32>, Option<u16>)>,
    /// How many problems had been reported when the generator gave up
    /// for too many errors, if it has.
    gave_up: Option<usize>,
    /// What each body shares with the bodies in and around it, by the
    /// body's index.
    closures: Vec<Closure>,
    /// The definition whose object is being generated, and which
    /// object in the output each definition became.
    cur_def: usize,
    def_objs: HashMap<usize, usize>,
    /// The bodies that were read and turned out not to be wanted: no
    /// code is generated for them.
    dropped: HashSet<usize>,
    /// How many versions of multi-methods the library is told of.
    mm_count: usize,
    /// The code being generated is a parameter's default value.
    in_defval: bool,
    /// The body being generated.
    cur_body: usize,
    /// The program is compiled for debugging: the code has line
    /// records, which say where in the source each statement's code is.
    debug: bool,
    /// The number the first source file of this module has among all
    /// the program's: the files of the modules before it come first.
    file_base: usize,
    /// The line records of the function being generated.
    line_recs: Vec<LineRec>,
    /// The statement of the body being generated, and where it is for
    /// the debugger, if not where it says. See [`CodeBody::stm_pos`].
    top_stm_pos: Option<(*const Stmt, (usize, u32))>,
}

/// Where in the source the code from an offset in a function comes
/// from, for the debugger.
#[derive(Clone, Copy)]
struct LineRec {
    /// The offset from the start of the function.
    ofs: usize,
    /// The source file, by its number in the program, and the line.
    source_id: usize,
    line: u32,
    /// The scope the code is in.
    frame: Option<TabId>,
}

impl<'p> Gen<'p> {
    fn new(prog: &'p mut Program, out: Generated, enum_ids: Vec<u32>) -> Self {
        let mut cg = CodeGen::new();
        let mut out = out;
        cg.code = std::mem::take(&mut out.code);
        Gen {
            prog,
            cg,
            out,
            method_ofs: 0,
            cur_frame: None,
            frames: Vec::new(),
            static_frame: None,
            enclosing: Vec::new(),
            switches: Vec::new(),
            labels: Rc::default(),
            last_labels: Rc::default(),
            goto_labels: Vec::new(),
            enum_ids,
            exc_table: Vec::new(),
            named_lists: Vec::new(),
            named_tabs: Vec::new(),
            resolve_only: false,
            local_cnt: 0,
            fin_ret_lcl: None,
            cur_file: 0,
            cur_line: 0,
            line_info: (usize::MAX, 0),
            interned: HashMap::new(),
            local_names: Vec::new(),
            errors: 0,
            undefined_names: HashSet::new(),
            self_available: false,
            in_constructor: false,
            member_ctx: Vec::new(),
            gave_up: None,
            closures: Vec::new(),
            cur_body: 0,
            cur_def: 0,
            def_objs: HashMap::new(),
            dropped: HashSet::new(),
            mm_count: 0,
            in_defval: false,
            debug: false,
            file_base: 0,
            line_recs: Vec::new(),
            top_stm_pos: None,
        }
    }
}

/// Generate the code for one source file: what the reference writes to
/// an object file. The numbers of objects, properties, and values of
/// enumerations in it are the file's own; [`crate::link`] gives them
/// the numbers they have in the image.
///
/// For a program compiled for debugging, the code has line records;
/// `file_base` is the number of the first of this file's sources among
/// the program's.
pub(crate) fn generate(prog: &mut Program, debug: bool, file_base: usize) -> Generated {
    let mut bodies = std::mem::take(&mut prog.bodies);
    let mut objdefs = std::mem::take(&mut prog.objdefs);
    let top = prog.top.clone();
    let enum_ids = match &prog.dynamic {
        Some(dynamic) => dynamic.enum_ids.clone(),
        None => prog.final_enum_ids(),
    };
    let mut generator = Gen::new(prog, Generated::default(), enum_ids);
    generator.debug = debug;
    generator.file_base = file_base;
    generator.out.max_list_cnt = generator.prog.max_list_cnt;
    generator.last_labels = Rc::clone(&generator.prog.last_labels);
    // Names are looked up, and what has become constant is worked out,
    // for the whole program before any code is generated.
    for item in &top {
        match *item {
            Top::Func(body) => generator.fold_stmt(&mut bodies[body].0.stm),
            Top::Obj(def) => {
                for prop in &mut objdefs[def].props {
                    match &mut prop.value {
                        PropValue::VocabList => {}
                        PropValue::Method(body) => {
                            let body = *body;
                            if bodies[body].0.closure.wraps_prop_expr {
                                // The value is worked out first as the
                                // property's; if that leaves a
                                // constant, the property is that, and
                                // not a method after all.
                                generator.cur_file = prop.file;
                                generator.cur_line = prop.line;
                                generator.line_info = (prop.file, prop.line);
                                let (StmtKind::Return(Some(expr))
                                | StmtKind::Expr(expr)
                                | StmtKind::StaticPropInit(expr, _)) = &mut bodies[body].0.stm.kind
                                else {
                                    unreachable!("a property's value in a statement");
                                };
                                generator.fold_in_place(expr);
                                if expr.is_const() || matches!(expr, Expr::DStr(_)) {
                                    let expr = std::mem::replace(expr, Expr::constant(Value::Nil));
                                    bodies[body].0.replaced = true;
                                    prop.value = PropValue::Expr(expr);
                                    continue;
                                }
                            }
                            generator.fold_stmt(&mut bodies[body].0.stm);
                        }
                        PropValue::Expr(expr) => {
                            generator.cur_file = prop.file;
                            generator.cur_line = prop.line;
                            generator.line_info = (prop.file, prop.line);
                            generator.fold_in_place(expr);
                            if expr.is_const() || matches!(expr, Expr::DStr(_)) {
                                continue;
                            }
                            // A value that has to be worked out when
                            // the program runs is a method that
                            // returns it.
                            let expr = std::mem::replace(expr, Expr::constant(Value::Nil));
                            let kind = if prop.is_static {
                                StmtKind::StaticPropInit(expr, prop.prop)
                            } else if generator.has_return_value(&expr) {
                                StmtKind::Return(Some(expr))
                            } else {
                                StmtKind::Expr(expr)
                            };
                            // For the debugger, the statement is where the
                            // parser's statement position was after the
                            // value, and the body ends at the property,
                            // where the reference makes it as it folds the
                            // value.
                            let (stm_file, stm_line) =
                                prop.body_pos.unwrap_or((prop.file, prop.line));
                            let (end_file, end_line) = (prop.file, prop.line);
                            let body = CodeBody {
                                lcltab: None,
                                stm: Stmt {
                                    kind,
                                    file: prop.file,
                                    line: prop.line,
                                },
                                argc: 0,
                                opt_argc: 0,
                                varargs: false,
                                varargs_list_local: None,
                                local_cnt: 0,
                                self_valid: true,
                                replaced: false,
                                labels: Rc::default(),
                                start: None,
                                end_file,
                                end_line,
                                closure: Closure::default(),
                                stm_pos: Some((stm_file, stm_line)),
                            };
                            prop.value = PropValue::Method(bodies.len());
                            bodies.push((body, None));
                        }
                    }
                }
            }
        }
    }
    // The reference stops here if that found anything wrong.
    if generator.errors != 0 {
        let out = generator.out;
        prog.bodies = bodies;
        prog.objdefs = objdefs;
        return out;
    }
    generator.closures = bodies
        .iter()
        .map(|(body, _)| body.closure.clone())
        .collect();
    // Nothing has a place yet: what there is to say about the start of
    // a function is said of wherever the last thing generated was.
    (generator.cur_file, generator.cur_line) = (usize::MAX, 0);
    generator.line_info = (usize::MAX, 0);
    let mut scopes = Vec::new();
    for item in &top {
        if generator.gave_up.is_some() {
            break;
        }
        match *item {
            Top::Func(body) => {
                generator.gen_code_body(&bodies[body].0, body, bodies[body].1, false);
                scopes.push(std::mem::take(&mut generator.frames));
            }
            Top::Obj(def) => {
                generator.cur_def = def;
                generator.gen_object(&objdefs[def], &bodies, &mut scopes);
            }
        }
    }
    // Locals are checked for use once everything has been generated.
    for frames in scopes {
        for frame in frames {
            generator.check_unreferenced_locals(frame);
        }
    }
    // Giving up for too many errors is the last thing said.
    if let Some(count) = generator.gave_up {
        generator.out.diagnostics.truncate(count);
    }
    generator.out.gave_up = generator.gave_up.is_some();
    generator.out.local_names = std::mem::take(&mut generator.local_names);
    generator.out.def_objs = std::mem::take(&mut generator.def_objs);
    generator.out.enum_ids = std::mem::take(&mut generator.enum_ids);
    let mut out = generator.out;
    out.code = generator.cg.code;
    prog.bodies = bodies;
    prog.objdefs = objdefs;
    out
}

/// Generate what the compiler writes itself once the source files of a
/// program have been put together: the dictionaries and the lists of
/// words, the grammar productions, and the code for multi-methods.
pub(crate) fn link_generate(prog: &mut Program, out: Generated, enum_ids: Vec<u32>) -> Generated {
    let objdefs = std::mem::take(&mut prog.objdefs);
    let gave_up = out.gave_up;
    let mut generator = Gen::new(prog, out, enum_ids);
    generator.def_objs = std::mem::take(&mut generator.out.def_objs);
    generator.local_names = std::mem::take(&mut generator.out.local_names);
    (generator.cur_file, generator.cur_line) = (usize::MAX, 0);
    let mut productions = Vec::new();
    if !gave_up {
        productions = generator.merge_grammar();
        generator.build_dictionaries(&objdefs, &productions);
        generator.gen_multimethods();
    }
    generator.finish_local_names();
    generator.out.next_prop_id = generator.prog.next_prop_id;
    let mut out = generator.out;
    out.code = generator.cg.code;
    let prog = &*generator.prog;
    out.metas = prog
        .meta_table
        .iter()
        .map(|entry| {
            let sym = entry.sym.map(|sym| &prog.metas[sym]);
            let props = sym.map_or_else(Vec::new, |sym| {
                sym.props.iter().map(|prop| prog.props[*prop].id).collect()
            });
            (entry.name.clone(), props, sym.map(|sym| sym.obj_id))
        })
        .collect();
    out.meta_mod_objs = prog
        .meta_table
        .iter()
        .map(|entry| {
            entry
                .sym
                .and_then(|sym| prog.metas[sym].mod_obj)
                .unwrap_or(0)
        })
        .collect();
    // The productions as the image has them: the count of the ways a
    // phrase can be made up, and for each a score that is always zero,
    // how much worse it is than another, the class of the object made
    // when it matches, and the things to match, each as the property
    // the match is kept in, a kind, and what the kind needs.
    for (sym, alts) in &productions {
        let prop_id = |prop: usize| prog.props[prop].id;
        let mut data = Vec::new();
        data.extend_from_slice(&(alts.len() as u16).to_le_bytes());
        for alt in alts {
            data.extend_from_slice(&0u16.to_le_bytes());
            data.extend_from_slice(&(alt.badness as u16).to_le_bytes());
            let obj = alt.obj.map_or(0, |obj| prog.objs[obj].id);
            data.extend_from_slice(&obj.to_le_bytes());
            data.extend_from_slice(&(alt.toks.len() as u16).to_le_bytes());
            for tok in &alt.toks {
                data.extend_from_slice(&tok.prop.map_or(0, prop_id).to_le_bytes());
                match &tok.kind {
                    GramTokKind::Prod(obj) => {
                        data.push(1);
                        data.extend_from_slice(&prog.objs[*obj].id.to_le_bytes());
                    }
                    GramTokKind::Speech(prop) => {
                        data.push(2);
                        data.extend_from_slice(&prop_id(*prop).to_le_bytes());
                    }
                    GramTokKind::Literal(text) => {
                        data.push(3);
                        data.extend_from_slice(&(text.len() as u16).to_le_bytes());
                        data.extend_from_slice(text);
                    }
                    GramTokKind::TokType(sym) => {
                        data.push(4);
                        data.extend_from_slice(&generator.enum_ids[*sym].to_le_bytes());
                    }
                    GramTokKind::Star => data.push(5),
                    GramTokKind::SpeechList(props) => {
                        data.push(6);
                        data.extend_from_slice(&(props.len() as u16).to_le_bytes());
                        for prop in props {
                            data.extend_from_slice(&prop_id(*prop).to_le_bytes());
                        }
                    }
                }
            }
        }
        out.gramprods.push((prog.objs[*sym].id, data));
    }
    generator.prog.objdefs = objdefs;
    out
}

impl Gen<'_> {
    // ---- messages ----

    fn log(&mut self, severity: Severity, message: impl Into<String>) {
        if self.gave_up.is_some() {
            return;
        }
        self.out.diagnostics.push(GenDiagnostic {
            file: self.cur_file,
            line: self.cur_line,
            severity,
            message: message.into(),
        });
        if severity == Severity::Error {
            self.errors += 1;
            if self.errors > MAX_ERRORS {
                self.out.diagnostics.push(GenDiagnostic {
                    file: self.cur_file,
                    line: self.cur_line,
                    severity: Severity::Fatal,
                    message: TOO_MANY_ERRORS.to_owned(),
                });
                self.gave_up = Some(self.out.diagnostics.len());
            }
        }
    }

    fn error(&mut self, message: impl Into<String>) {
        self.log(Severity::Error, message);
    }

    fn set_line(&mut self, stmt: &Stmt) {
        self.cur_file = stmt.file;
        self.cur_line = stmt.line;
        self.line_info = (stmt.file, stmt.line);
    }

    /// Warn about the locals of a scope that are never used or never
    /// given a value.
    fn check_unreferenced_locals(&mut self, tab: TabId) {
        for id in enum_order(&self.prog.tabs[tab], &self.prog.locals) {
            let local = &self.prog.locals[id];
            // A nested function's view of a local: the local itself is
            // what gets reported.
            if local.ctx_orig.is_some() {
                continue;
            }
            let param =
                local.is_param || local.is_list_param || local.is_opt_param || local.is_named_param;
            let name = show_name(&local.name);
            let message = if !local.val_used && !local.val_assigned && !param {
                format!("local variable \"{name}\" is never used")
            } else if !local.val_used {
                if param {
                    // An unused parameter is only worth a mention to
                    // someone who asks for every last warning.
                    continue;
                }
                format!(
                    "value is assigned to local variable \"{name}\" but the value is never used"
                )
            } else if !local.val_assigned && !param {
                format!("no value is assigned to local variable \"{name}\"")
            } else {
                continue;
            };
            self.out.diagnostics.push(GenDiagnostic {
                file: local.file,
                line: local.line,
                severity: Severity::Warning,
                message,
            });
        }
    }

    // ---- constant folding, once names are known ----

    fn fold_stmt(&mut self, stmt: &mut Stmt) {
        self.set_line(stmt);
        match &mut stmt.kind {
            StmtKind::Null | StmtKind::Break(_) | StmtKind::Continue(_) | StmtKind::Goto(_) => {}
            StmtKind::Switch { expr, body, .. } => {
                self.fold_in_place(expr);
                self.fold_stmt(body);
            }
            StmtKind::Case { expr, stm } => {
                self.fold_in_place(expr);
                if !expr.is_const() {
                    self.error("\"case\" expression has a non-constant value");
                }
                if let Some(stm) = stm {
                    self.fold_stmt(stm);
                }
            }
            StmtKind::Default(stm) | StmtKind::Label { stm, .. } => {
                if let Some(stm) = stm {
                    self.fold_stmt(stm);
                }
            }
            StmtKind::Try {
                body,
                catches,
                finally,
                ..
            } => {
                if let Some(body) = body {
                    self.fold_stmt(body);
                }
                for catch in catches {
                    if let Some(body) = &mut catch.body {
                        self.fold_stmt(body);
                    }
                }
                if let Some(body) = finally.as_mut().and_then(|finally| finally.body.as_mut()) {
                    self.fold_stmt(body);
                }
            }
            StmtKind::Throw(expr) | StmtKind::StaticPropInit(expr, _) => self.fold_in_place(expr),
            StmtKind::ImplicitCtor(_) | StmtKind::MmStub(_) | StmtKind::MmInit => {}
            StmtKind::Comp { stmts, .. } => {
                for stmt in stmts {
                    self.fold_stmt(stmt);
                }
            }
            StmtKind::Expr(expr) => self.fold_in_place(expr),
            StmtKind::If {
                cond,
                then_part,
                else_part,
            } => {
                self.fold_in_place(cond);
                if let Some(part) = then_part {
                    self.fold_stmt(part);
                }
                if let Some(part) = else_part {
                    self.fold_stmt(part);
                }
            }
            StmtKind::Return(expr) => {
                if let Some(expr) = expr {
                    self.fold_in_place(expr);
                }
            }
            StmtKind::For {
                init,
                cond,
                reinit,
                body,
                ..
            } => {
                for expr in [init, cond, reinit].into_iter().flatten() {
                    self.fold_in_place(expr);
                }
                if let Some(body) = body {
                    self.fold_stmt(body);
                }
            }
            StmtKind::Foreach {
                iter, coll, body, ..
            } => {
                self.fold_in_place(iter);
                self.fold_in_place(coll);
                if let Some(body) = body {
                    self.fold_stmt(body);
                }
            }
            StmtKind::While { cond, body, .. } | StmtKind::DoWhile { body, cond, .. } => {
                self.fold_in_place(cond);
                if let Some(body) = body {
                    self.fold_stmt(body);
                }
            }
        }
    }

    fn fold_in_place(&mut self, expr: &mut Expr) {
        let taken = std::mem::replace(expr, Expr::constant(Value::Nil));
        *expr = self.fold_expr(taken);
    }

    fn fold_expr(&mut self, expr: Expr) -> Expr {
        match expr {
            Expr::Const(_) | Expr::Local(_) | Expr::AnonFunc { .. } => expr,
            Expr::Sym(name) => match self.prog.globals.get(&name) {
                Some(Global::Func(func)) => Expr::constant(Value::FuncPtr(*func)),
                // Only an object of the program's own kind is a
                // constant. A dictionary, which is an object of a class
                // built into the virtual machine, stays a name.
                Some(Global::Obj(obj))
                    if self.prog.objs[*obj].is_dict
                        || self.prog.objs[*obj].is_gramprod
                        || self.is_func_obj(*obj) =>
                {
                    Expr::Sym(name)
                }
                Some(Global::Obj(obj)) => Expr::constant(Value::Obj(self.prog.objs[*obj].id)),
                Some(Global::Enum(sym)) => Expr::constant(Value::Enum(self.enum_ids[*sym])),
                Some(Global::Bif(_) | Global::Prop(_) | Global::Meta(_) | Global::DynLocal(_))
                | None => Expr::Sym(name),
            },
            Expr::SelfRef
            | Expr::Inh
            | Expr::InhClass(_)
            | Expr::CtxEle(_)
            | Expr::Replaced
            | Expr::InhTypes(..)
            | Expr::ArgCount => expr,
            Expr::InlineObj {
                superclasses,
                props,
            } => {
                let props = props
                    .into_iter()
                    .map(|mut prop| {
                        (self.cur_file, self.cur_line) = (prop.file, prop.line);
                        self.line_info = (prop.file, prop.line);
                        prop.value = match prop.value {
                            InlineValue::Method(method) => InlineValue::Method(method),
                            InlineValue::Eval(expr) => InlineValue::Eval(self.fold_expr(expr)),
                            // A value that is a constant, or text to
                            // display, is given outright, and the
                            // method made to work it out is not wanted.
                            // Not in the default value of a parameter,
                            // though: there names are looked up and
                            // nothing more, and the method stays.
                            InlineValue::Wrapped(expr, method) => {
                                let expr = self.fold_expr(expr);
                                if self.resolve_only {
                                    InlineValue::Method(method)
                                } else if expr.is_const() || matches!(expr, Expr::DStr(_)) {
                                    if let Expr::AnonFunc { body, .. } = method {
                                        self.dropped.insert(body);
                                    }
                                    InlineValue::Eval(expr)
                                } else {
                                    InlineValue::Method(method)
                                }
                            }
                        };
                        prop
                    })
                    .collect();
                Expr::InlineObj {
                    superclasses,
                    props,
                }
            }
            Expr::Delegated(target) => Expr::Delegated(Box::new(self.fold_expr(*target))),
            Expr::IsIn {
                left,
                args,
                not,
                hit,
            } => {
                let left = self.fold_expr(*left);
                let args: Vec<Expr> = args.into_iter().map(|arg| self.fold_expr(arg)).collect();
                // The reference means to compare a constant on the left
                // with the constants in the list here, but looks at the
                // wrong thing to tell whether they are constants, and
                // never finds one. So the test is left for when the
                // program runs, unless the list is empty, when a
                // constant is in it for certain not.
                if args.is_empty() && !self.resolve_only {
                    if let Expr::Const(mut value) = left {
                        value.set_bool(not);
                        return Expr::Const(value);
                    }
                }
                Expr::IsIn {
                    left: Box::new(left),
                    args,
                    not,
                    hit,
                }
            }
            Expr::ForIn(for_in) => Expr::ForIn(Box::new(match *for_in {
                ForIn::Coll {
                    lval,
                    expr,
                    iter_local,
                } => ForIn::Coll {
                    lval: self.fold_expr(lval),
                    expr: self.fold_expr(expr),
                    iter_local,
                },
                ForIn::Range {
                    lval,
                    from,
                    to,
                    step,
                    to_local,
                    step_local,
                } => ForIn::Range {
                    lval: self.fold_expr(lval),
                    from: self.fold_expr(from),
                    to: self.fold_expr(to),
                    step: step.map(|step| self.fold_expr(step)),
                    to_local,
                    step_local,
                },
            })),
            // The address of a property is a constant. What else a
            // name might be the address of is left for the generator.
            Expr::Addr(sub) => match &*sub {
                Expr::Sym(name) => match self.prog.globals.get(name) {
                    Some(Global::Prop(prop)) => {
                        Expr::constant(Value::Prop(self.prog.props[*prop].id))
                    }
                    _ => Expr::Addr(sub),
                },
                _ => Expr::Addr(sub),
            },
            Expr::New(sub, transient) => Expr::New(Box::new(self.fold_expr(*sub)), transient),
            Expr::Member {
                obj,
                prop,
                prop_is_expr,
                args,
            } => {
                let obj = Box::new(self.fold_expr(*obj));
                // The property's name is looked up like any other name,
                // so the name of a function or an object becomes the
                // function or the object, and is then no property.
                let prop = Box::new(self.fold_expr(*prop));
                let args = args.map(|args| {
                    args.into_iter()
                        .map(|arg| Arg {
                            expr: self.fold_expr(arg.expr),
                            varargs: arg.varargs,
                            name: arg.name,
                        })
                        .collect()
                });
                Expr::Member {
                    obj,
                    prop,
                    prop_is_expr,
                    args,
                }
            }
            Expr::DStr(_) => expr,
            Expr::DStrEmbed(sub) => Expr::DStrEmbed(Box::new(self.fold_expr(*sub))),
            Expr::OneOf(dbl, elements, state) => {
                let elements = elements
                    .into_iter()
                    .map(|element| self.fold_expr(element))
                    .collect();
                Expr::OneOf(dbl, elements, state)
            }
            Expr::List(elements) => {
                let elements = elements
                    .into_iter()
                    .map(|element| self.fold_expr(element))
                    .collect();
                if self.resolve_only {
                    Expr::List(elements)
                } else {
                    Expr::list(elements)
                }
            }
            Expr::Lookup(list) => {
                // The list stays one to make when the program runs.
                let list = match *list {
                    Expr::List(elements) => Expr::List(
                        elements
                            .into_iter()
                            .map(|element| self.fold_expr(element))
                            .collect(),
                    ),
                    other => self.fold_expr(other),
                };
                Expr::Lookup(Box::new(list))
            }
            Expr::Index(left, index) => {
                let left = self.fold_expr(*left);
                let index = self.fold_expr(*index);
                if self.resolve_only {
                    return Expr::Index(Box::new(left), Box::new(index));
                }
                let mut errors = Vec::new();
                let folded = fold::subscript(&left, &index, &mut errors);
                for message in errors {
                    self.error(message);
                }
                folded.unwrap_or_else(|| Expr::Index(Box::new(left), Box::new(index)))
            }
            Expr::Unary(op, sub) => {
                let sub = self.fold_expr(*sub);
                if op == UnOp::Not && !self.resolve_only {
                    match fold::not(sub) {
                        Ok(folded) => folded,
                        Err(sub) => Expr::Unary(op, Box::new(sub)),
                    }
                } else {
                    Expr::Unary(op, Box::new(sub))
                }
            }
            Expr::Binary(op, left, right) => {
                let left = self.fold_expr(*left);
                let right = self.fold_expr(*right);
                // Only these operators are tried again; the rest were
                // folded as far as they can be when they were parsed.
                let again = matches!(
                    op,
                    BinOp::Comma
                        | BinOp::Add
                        | BinOp::Sub
                        | BinOp::Eq
                        | BinOp::Ne
                        | BinOp::And
                        | BinOp::Or
                );
                if !again || self.resolve_only {
                    return Expr::Binary(op, Box::new(left), Box::new(right));
                }
                let mut errors = Vec::new();
                let result = fold::binary(op, left, right, &mut errors);
                for message in errors {
                    self.error(message);
                }
                match result {
                    Folded::Yes(expr) => {
                        if let Some(len) = expr.const_list_len() {
                            self.out.max_list_cnt = self.out.max_list_cnt.max(len);
                        }
                        expr
                    }
                    Folded::No(left, right) => Expr::Binary(op, Box::new(left), Box::new(right)),
                }
            }
            Expr::Assign(op, left, right) => {
                let left = Box::new(self.fold_expr(*left));
                Expr::Assign(op, left, Box::new(self.fold_expr(*right)))
            }
            Expr::IncDec(op, sub) => Expr::IncDec(op, Box::new(self.fold_expr(*sub))),
            Expr::If(first, second, third) => {
                let first = self.fold_expr(*first);
                let second = self.fold_expr(*second);
                let third = self.fold_expr(*third);
                if self.resolve_only {
                    return Expr::If(Box::new(first), Box::new(second), Box::new(third));
                }
                match first.as_const().map(ConstVal::is_true) {
                    Some(true) => second,
                    Some(false) => third,
                    None => Expr::If(Box::new(first), Box::new(second), Box::new(third)),
                }
            }
            Expr::IfNil(first, second) => {
                let first = self.fold_expr(*first);
                let second = self.fold_expr(*second);
                if self.resolve_only {
                    return Expr::IfNil(Box::new(first), Box::new(second));
                }
                match first.as_const() {
                    Some(value) if value.value == Value::Nil => second,
                    Some(_) => first,
                    None => Expr::IfNil(Box::new(first), Box::new(second)),
                }
            }
            Expr::Call(func, args) => {
                let func = Box::new(self.fold_expr(*func));
                let args = args
                    .into_iter()
                    .map(|arg| Arg {
                        expr: self.fold_expr(arg.expr),
                        varargs: arg.varargs,
                        name: arg.name,
                    })
                    .collect();
                Expr::Call(func, args)
            }
        }
    }

    // ---- scopes ----

    fn rel_ofs(&self) -> usize {
        self.cg.ofs() - self.method_ofs
    }

    fn add_to_range(&mut self, tab: TabId, ofs: usize) {
        let tab = &mut self.prog.tabs[tab];
        if tab.start_ofs == 0 || ofs < tab.start_ofs {
            tab.start_ofs = ofs;
        }
        if ofs > tab.end_ofs {
            tab.end_ofs = ofs;
        }
    }

    /// Make a scope the one whose code is being generated. The scope
    /// before it is returned, to be put back afterward.
    fn set_local_frame(&mut self, symtab: Option<TabId>) -> Option<TabId> {
        let old_frame = self.cur_frame;
        let ofs = self.rel_ofs();
        if let Some(old) = old_frame {
            self.add_to_range(old, ofs);
        }
        self.cur_frame = symtab;
        if let Some(tab) = symtab {
            if self.prog.tabs[tab].list_index == 0 {
                self.frames.push(tab);
                self.prog.tabs[tab].list_index = self.frames.len();
            }
            self.add_to_range(tab, ofs);
        }
        old_frame
    }

    // ---- functions ----

    /// Generate an object: its superclasses and its properties, with
    /// the code of its methods.
    fn gen_object(
        &mut self,
        def: &ObjDef,
        bodies: &[(CodeBody, Option<usize>)],
        scopes: &mut Vec<Vec<TabId>>,
    ) {
        // An object has a name, or at least a number.
        let id = match (def.mod_id, def.sym) {
            (Some(id), _) => id,
            (None, Some(sym)) => self.prog.objs[sym].id,
            (None, None) => return,
        };
        if def.replaced {
            return;
        }
        let sym = def.sym;
        // Where the reference's tokenizer was before this object, which
        // is where a constructor made for it ends.
        let before = self.line_info;
        (self.cur_file, self.cur_line) = (def.file, def.line);
        // A definition that `modify` has put another in front of is an
        // object of its own, and a class, whatever it was.
        let mut obj = GenObj {
            id,
            is_class: def.is_class || (def.mod_id.is_some() && def.meta.is_none()),
            superclasses: Vec::new(),
            props: Vec::new(),
            icmod: def.meta.is_some(),
            transient: def.transient,
        };
        // And that object is the one superclass of the definition in
        // front of it.
        obj.superclasses.extend(def.mod_base);
        let mut bad_sc = false;
        for name in &def.superclasses {
            match self.prog.globals.get(name) {
                Some(Global::Obj(superclass)) => {
                    obj.superclasses.push(self.prog.objs[*superclass].id);
                }
                Some(_) => {
                    bad_sc = true;
                    self.error(format!(
                        "invalid superclass \"{}\" - not an object",
                        show_name(name)
                    ));
                }
                None => {
                    let obj_name = sym.map_or_else(String::new, |sym| {
                        show_name(&self.prog.objs[sym].name).to_string()
                    });
                    bad_sc = true;
                    self.error(format!(
                        "undefined symbol \"{}\" (used as superclass of \"{obj_name}\")",
                        show_name(name)
                    ));
                }
            }
        }
        // Values no template had properties for are reported now, if
        // the trouble was not with the classes.
        if def.bad_template && !bad_sc {
            self.error(if def.undesc_sc {
                "cannot use template with an unimported extern class as superclass"
            } else {
                "object definition does not match any template"
            });
        }
        // An object with several superclasses and no constructor is
        // given one that calls each superclass's.
        let has_constructor = def
            .props
            .iter()
            .any(|prop| prop.prop == 0 && matches!(prop.value, PropValue::Method(_)));
        let implicit = (def.superclasses.len() > 1 && !has_constructor).then(|| {
            self.prog.locals.push(Local {
                name: b"*".to_vec(),
                is_list_param: true,
                val_used: true,
                val_assigned: true,
                file: def.file,
                line: def.line,
                ..Local::default()
            });
            CodeBody {
                lcltab: None,
                stm: Stmt {
                    kind: StmtKind::ImplicitCtor(obj.superclasses.clone()),
                    file: def.file,
                    line: def.line,
                },
                argc: 0,
                opt_argc: 0,
                varargs: true,
                varargs_list_local: Some(self.prog.locals.len() - 1),
                local_cnt: 1,
                self_valid: true,
                replaced: false,
                // The reference gives this body the table of labels the
                // parser was last using, which is the last function's;
                // so a label there that nothing refers to is reported
                // again.
                labels: Rc::clone(&self.last_labels),
                start: None,
                // It ends where the code generator was when the reference
                // makes it, as it starts on the object: at the last
                // statement whose code it generated.
                end_file: before.0,
                end_line: before.1,
                closure: Closure::default(),
                stm_pos: None,
            }
        });
        for prop in &def.props {
            (self.cur_file, self.cur_line) = (prop.file, prop.line);
            self.line_info = (prop.file, prop.line);
            let id = self.prog.props[prop.prop].id;
            let (kind, value, target) = match &prop.value {
                PropValue::Method(body) if prop.is_static => {
                    // Code that runs before the program starts: it is
                    // generated like any other, and then moved to where
                    // such code is kept.
                    self.out.static_inits.push((obj.id, id));
                    let mark = (
                        self.cg.ofs(),
                        self.out.code_anchors.len(),
                        self.out.fixups.len(),
                        self.out.id_fixups.len(),
                        self.out.unspecified.len(),
                    );
                    let main_frame = std::mem::replace(&mut self.cur_frame, self.static_frame);
                    self.gen_code_body(&bodies[*body].0, *body, None, prop.prop == 0);
                    self.static_frame = std::mem::replace(&mut self.cur_frame, main_frame);
                    scopes.push(std::mem::take(&mut self.frames));
                    self.move_to_static(mark, *body);
                    (datatype::CODEOFS, 0, Some(Target::StaticBody(*body)))
                }
                PropValue::Method(body) => {
                    self.gen_code_body(&bodies[*body].0, *body, None, prop.prop == 0);
                    scopes.push(std::mem::take(&mut self.frames));
                    (datatype::CODEOFS, 0, Some(Target::Body(*body)))
                }
                PropValue::Expr(Expr::DStr(text)) => {
                    (datatype::DSTRING, 0, Some(self.const_str_target(text)))
                }
                PropValue::VocabList => (datatype::VOCAB_LIST, 0, None),
                PropValue::Expr(Expr::Const(value)) => match &value.value {
                    Value::Nil => (datatype::NIL, 0, None),
                    Value::True => (datatype::TRUE, 0, None),
                    Value::Int(int) => (datatype::INT, *int as u32, None),
                    Value::Obj(obj) => (datatype::OBJ, *obj, None),
                    Value::Float { .. } | Value::ReStr(_) => {
                        let id = self.gen_const_obj(&value.value).expect("an object");
                        (datatype::OBJ, id, None)
                    }
                    Value::Enum(id) => (datatype::ENUM, *id, None),
                    Value::Prop(prop) => (datatype::PROP, u32::from(*prop), None),
                    Value::SStr(text) => (datatype::SSTRING, 0, Some(self.const_str_target(text))),
                    Value::List(elements) => {
                        let before = self.out.fixups.len();
                        self.add_const_list(elements, false, 0);
                        let target = self.out.fixups.remove(before).target;
                        (datatype::LIST, 0, Some(target))
                    }
                    Value::FuncPtr(func) => (datatype::FUNCPTR, 0, Some(Target::Func(*func))),
                },
                PropValue::Expr(_) => {
                    self.error("invalid property value for code generation");
                    continue;
                }
            };
            obj.props.push(GenProp {
                id,
                kind,
                value,
                target,
            });
        }
        if let Some(body) = implicit {
            (self.cur_file, self.cur_line) = (def.file, def.line);
            // The reference adds it to the object as a property, at
            // the place the parser finished the file, and generates it
            // as the last of the properties.
            self.line_info = self.prog.end_pos;
            // A number for the body that no body the parser read has.
            let index = bodies.len() + self.out.objs.len();
            self.gen_code_body(&body, index, None, true);
            scopes.push(std::mem::take(&mut self.frames));
            obj.props.push(GenProp {
                id: 1,
                kind: datatype::CODEOFS,
                value: 0,
                target: Some(Target::Body(index)),
            });
        }
        self.def_objs.insert(self.cur_def, self.out.objs.len());
        self.out.objs.push(obj);
    }

    /// Put the words the program's objects have in their dictionaries,
    /// and give each object's dictionary properties their values: a
    /// list of the object's words for the property.
    ///
    /// The reference does this when it puts the program together,
    /// going through its table of global names and then the objects
    /// with no names. An object's words are its own and those of its
    /// classes, which it takes when its turn comes if it is not a
    /// class itself; a class gets its classes' words only if some
    /// object of its kind has had its turn first.
    /// Put together each grammar production's ways of making up its
    /// phrase: those of the statements with no tag, in the order they
    /// were read, and then those kept with the names of the classes
    /// that statements with a tag defined, in the order of the table
    /// of global names. A production with none, that no statement
    /// declared, is probably a name spelled wrong.
    fn merge_grammar(&mut self) -> Vec<(usize, Vec<GramAlt>)> {
        let mut productions: Vec<(usize, Vec<GramAlt>)> = self
            .prog
            .gramprods
            .iter()
            .map(|prod| (prod.sym, prod.alts.clone()))
            .collect();
        let mut named: Vec<(usize, std::cmp::Reverse<u32>, usize)> = self
            .prog
            .objs
            .iter()
            .enumerate()
            .filter(|(index, obj)| {
                obj.gram.is_some() && self.prog.globals.get(&obj.name) == Some(&Global::Obj(*index))
            })
            .map(|(index, obj)| {
                let chain = crate::parse::global_hash(&obj.name);
                (chain, std::cmp::Reverse(obj.id), index)
            })
            .collect();
        named.sort_unstable();
        for (_, _, obj) in named {
            if let Some((Some(prod), alts)) = &self.prog.objs[obj].gram {
                productions[*prod].1.extend(alts.iter().cloned());
            }
        }
        for (index, (sym, alts)) in productions.iter().enumerate() {
            if alts.is_empty() && !self.prog.gramprods[index].declared {
                self.out.link_errors.push(format!(
                    "grammar production \"{}\" has no alternatives defined",
                    show_name(&self.prog.objs[*sym].name)
                ));
            }
            if alts.len() > 65535 {
                self.out.link_errors.push(format!(
                    "grammar production \"{}\" has too many (>65,535) rules",
                    show_name(&self.prog.objs[*sym].name)
                ));
            }
        }
        productions
    }

    fn build_dictionaries(&mut self, objdefs: &[ObjDef], productions: &[(usize, Vec<GramAlt>)]) {
        const BUCKETS: usize = 128;
        // The definitions in the order of the table as it is when the
        // program is put together: chain by chain, and in a chain the
        // one with the highest number first, since the table is filled
        // in the order of the numbers and the newest name in a chain
        // is the first. A definition that `modify` set aside is in the
        // table under a name made of its number.
        let mut named: Vec<(usize, std::cmp::Reverse<u32>, usize)> = Vec::new();
        for (index, obj) in self.prog.objs.iter().enumerate() {
            if self.prog.globals.get(&obj.name) != Some(&Global::Obj(index)) {
                continue;
            }
            if let Some(def) = obj.def {
                let chain = crate::parse::global_hash(&obj.name);
                named.push((chain, std::cmp::Reverse(obj.id), def));
            }
        }
        for (index, def) in objdefs.iter().enumerate() {
            if let (Some(id), None, false) = (def.mod_id, def.meta, def.replaced) {
                let name = format!(" {id:x}");
                let chain = crate::parse::global_hash(name.as_bytes());
                named.push((chain, std::cmp::Reverse(id), index));
            }
        }
        named.sort_unstable();
        // Then the objects with no names, in the order they were made.
        let unnamed = self
            .prog
            .objs
            .iter()
            .filter(|obj| obj.name == b".anon")
            .filter_map(|obj| obj.def);
        let order: Vec<usize> = named
            .into_iter()
            .map(|(_, _, def)| def)
            .chain(unnamed)
            .collect();

        // Each definition's words, the first to be put in a dictionary
        // first, and whether it has taken its classes' yet.
        let mut words: Vec<Vec<(Vec<u8>, usize)>> =
            objdefs.iter().map(|def| def.vocab.clone()).collect();
        // The lists of words are put in the constants when the program
        // is put together, and the reference then knows nothing of
        // the strings that are there from before: a word is in the
        // constants once for all the lists, but again if it is also a
        // string somewhere in the program.
        self.interned.clear();
        let mut inherited = vec![false; objdefs.len()];
        // Each dictionary's words: chains of them by the sum of their
        // letters, the newest first in a chain, as an object's
        // references are in a word.
        let dict_objs = self.prog.dicts.clone();
        let mut tables: Vec<Vec<Vec<DictWord>>> = vec![vec![Vec::new(); BUCKETS]; dict_objs.len()];
        // The words the productions have go in first: each is a word
        // for its production, by a property kept for that.
        let add_word =
            |tables: &mut Vec<Vec<Vec<DictWord>>>, dict: usize, word: &[u8], item: (u32, u16)| {
                let hash = word
                    .iter()
                    .map(|byte| usize::from(byte.to_ascii_uppercase()))
                    .sum::<usize>()
                    % BUCKETS;
                let chain = &mut tables[dict][hash];
                let at = match chain.iter().position(|(text, _)| text == word) {
                    Some(at) => at,
                    None => {
                        chain.insert(0, (word.to_vec(), Vec::new()));
                        0
                    }
                };
                if !chain[at].1.contains(&item) {
                    chain[at].1.insert(0, item);
                }
            };
        for (sym, alts) in productions {
            let id = self.prog.objs[*sym].id;
            for alt in alts {
                let dict = alt
                    .dict
                    .and_then(|dict| dict_objs.iter().position(|obj| *obj == dict));
                let Some(dict) = dict else {
                    continue;
                };
                for tok in &alt.toks {
                    if let GramTokKind::Literal(word) = &tok.kind {
                        add_word(&mut tables, dict, word, (id, MISC_VOCAB_PROP_ID));
                    }
                }
            }
        }
        for def_index in order {
            let def = &objdefs[def_index];
            let id = match (def.mod_id, def.sym) {
                (Some(id), _) => id,
                (None, Some(sym)) => self.prog.objs[sym].id,
                (None, None) => continue,
            };
            let dict = def
                .dict
                .and_then(|dict| dict_objs.iter().position(|obj| *obj == dict));
            if let (Some(dict), false) = (dict, def.is_class) {
                self.inherit_vocab(def_index, objdefs, &mut words, &mut inherited);
                for (word, prop) in &words[def_index] {
                    add_word(&mut tables, dict, word, (id, self.prog.props[*prop].id));
                }
            }
            // The values of its dictionary properties.
            let Some(&gen_obj) = self.def_objs.get(&def_index) else {
                continue;
            };
            let mut at = 0;
            while at < self.out.objs[gen_obj].props.len() {
                let prop = &self.out.objs[gen_obj].props[at];
                if prop.kind != datatype::VOCAB_LIST {
                    at += 1;
                    continue;
                }
                let prop_id = prop.id;
                let list: Vec<ConstVal> = words[def_index]
                    .iter()
                    .filter(|(_, prop)| self.prog.props[*prop].id == prop_id)
                    .map(|(word, _)| ConstVal::new(Value::SStr(word.clone())))
                    .collect();
                if list.is_empty() {
                    // No words: no property.
                    self.out.objs[gen_obj].props.remove(at);
                    continue;
                }
                self.out.max_list_cnt = self.out.max_list_cnt.max(list.len());
                let before = self.out.fixups.len();
                self.add_const_list(&list, false, 0);
                let target = self.out.fixups.remove(before).target;
                let prop = &mut self.out.objs[gen_obj].props[at];
                prop.kind = datatype::LIST;
                prop.target = Some(target);
                at += 1;
            }
        }
        // The dictionaries go in the image in the order of their
        // numbers.
        for (dict, table) in dict_objs.iter().zip(tables) {
            let id = self.prog.objs[*dict].id;
            self.out
                .dicts
                .push((id, table.into_iter().flatten().collect()));
        }
        self.out.dicts.sort_by_key(|(id, _)| *id);
    }

    /// Give a definition the words of its classes, each of which takes
    /// its own classes' first. They go in front of what it has, one at
    /// a time, so that the last comes first.
    fn inherit_vocab(
        &self,
        def_index: usize,
        objdefs: &[ObjDef],
        words: &mut [Vec<(Vec<u8>, usize)>],
        inherited: &mut [bool],
    ) {
        if std::mem::replace(&mut inherited[def_index], true) {
            return;
        }
        let def = &objdefs[def_index];
        let superclasses: Vec<usize> = if def.is_modify {
            def.prev_def.into_iter().collect()
        } else {
            def.superclasses
                .iter()
                .filter_map(|name| match self.prog.globals.get(name) {
                    Some(Global::Obj(obj)) => self.prog.objs[*obj].def,
                    _ => None,
                })
                .collect()
        };
        for superclass in superclasses {
            self.inherit_vocab(superclass, objdefs, words, inherited);
            for word in words[superclass].clone() {
                words[def_index].insert(0, word);
            }
        }
    }

    /// Put a string in the constants for something outside the code to
    /// refer to.
    /// Make the object a constant number that is not an integer is.
    /// Each place such a constant is used gets an object of its own.
    fn gen_bignum_obj(&mut self, text: &[u8], promoted: bool) -> u32 {
        if promoted {
            self.log(
                Severity::Warning,
                "constant value exceeds integer range; promoted to BigNumber",
            );
        }
        let id = self.prog.next_obj_id;
        self.prog.next_obj_id += 1;
        let big = hexicon_bignum::Big::parse_source(&String::from_utf8_lossy(text));
        self.out.bignums.push((id, big.to_bytes()));
        id
    }

    /// Make the object a regular expression in the source is.
    fn gen_rexpat_obj(&mut self, text: &[u8]) -> u32 {
        let id = self.prog.next_obj_id;
        self.prog.next_obj_id += 1;
        let target = self.const_str_target(text);
        self.out.rexpats.push((id, target));
        id
    }

    /// The object a constant is, if it is one of the kinds that are
    /// made objects when code is generated.
    fn gen_const_obj(&mut self, value: &Value) -> Option<u32> {
        match value {
            Value::Float { text, promoted } => Some(self.gen_bignum_obj(text, *promoted)),
            Value::ReStr(text) => Some(self.gen_rexpat_obj(text)),
            _ => None,
        }
    }

    fn const_str_target(&mut self, text: &[u8]) -> Target {
        let before = self.out.fixups.len();
        self.add_const_str(text, false, 0);
        self.out.fixups.remove(before).target
    }

    /// Take the code generated since a mark was made out of the
    /// program's code and put it with the code that runs before the
    /// program starts, along with everything noted about it. The mark
    /// is how long the code was, and how many anchors, addresses to
    /// fill in, property numbers, and meaningless fields had been
    /// noted.
    fn move_to_static(&mut self, mark: (usize, usize, usize, usize, usize), body: usize) {
        let (ofs, anchors, fixups, prop_fixups, unspecified) = mark;
        let base = self.out.static_code.len();
        let moved = |at: usize| at - ofs + base;
        self.out.static_code.extend(self.cg.code.drain(ofs..));
        self.cg.clear_peephole();
        if self.out.body_anchor.remove(&body).is_some() {
            self.out
                .static_body_anchor
                .insert(body, self.out.static_anchors.len());
        }
        for anchor in self.out.code_anchors.drain(anchors..) {
            self.out.static_anchors.push(Anchor {
                ofs: moved(anchor.ofs),
            });
        }
        for fixup in self.out.fixups.drain(fixups..) {
            self.out.static_fixups.push(Fixup {
                ofs: moved(fixup.ofs),
                target: fixup.target,
            });
        }
        for (at, kind) in self.out.id_fixups.drain(prop_fixups..) {
            self.out.static_id_fixups.push((moved(at), kind));
        }
        for at in self.out.unspecified.drain(unspecified..) {
            self.out.static_unspecified.push(moved(at));
        }
    }

    fn gen_code_body(
        &mut self,
        body: &CodeBody,
        body_index: usize,
        func: Option<usize>,
        is_constructor: bool,
    ) {
        self.in_constructor = is_constructor;
        if body.replaced || self.dropped.contains(&body_index) {
            return;
        }
        // The header. The counts of locals and stack slots and the
        // offsets of the tables are filled in at the end.
        self.cg.start_method();
        self.frames.clear();
        self.enclosing.clear();
        self.switches.clear();
        self.labels = Rc::clone(&body.labels);
        self.goto_labels = vec![None; body.labels.len()];
        self.exc_table.clear();
        self.named_lists.clear();
        self.named_tabs.clear();
        self.local_cnt = body.local_cnt;
        self.fin_ret_lcl = None;
        self.method_ofs = self.cg.ofs();
        let method_ofs = self.method_ofs;
        self.self_available = body.self_valid;
        self.cur_body = body_index;
        if let Some(func) = func {
            self.out
                .func_anchor
                .insert(func, self.out.code_anchors.len());
        }
        self.out
            .body_anchor
            .insert(body_index, self.out.code_anchors.len());
        self.out.code_anchors.push(Anchor { ofs: method_ofs });
        self.cg
            .write(body.argc as u8 | if body.varargs { 0x80 } else { 0 });
        self.cg.write(body.opt_argc as u8);
        for _ in 0..4 {
            self.cg.write2(0);
        }
        let code_start = self.cg.ofs();
        // A line record for the code that starts the function, which can
        // fail. The reference adds it without the usual care for the
        // peephole, as nothing has been written yet.
        self.line_recs.clear();
        if let Some((file, line)) = body.start {
            self.add_line_rec_raw(file, line);
        }

        // The scopes around the function's own come first in its list.
        if let Some(lcltab) = body.lcltab {
            let mut tab = self.prog.tabs[lcltab].parent;
            while let Some(id) = tab {
                self.set_local_frame(Some(id));
                tab = self.prog.tabs[id].parent;
            }
        }
        self.set_local_frame(body.lcltab);

        // A function with locals that nested functions use makes a
        // vector to keep them in, and moves into it any parameters
        // among them.
        let closure = &body.closure;
        if closure.has_local_ctx {
            self.gen_int(closure.local_ctx_arr_size as i32);
            self.cg.write_op(Op::Dup);
            self.cg.write_op(Op::New1);
            self.cg.write(2);
            self.cg.write(METACLASS_VECTOR);
            self.cg.write_op(Op::GetR0);
            self.cg.note_push();
            self.cg.note_pop();
            self.gen_setlcl_stk(closure.local_ctx_var, false);
            if let Some(lcltab) = body.lcltab {
                for id in enum_order(&self.prog.tabs[lcltab], &self.prog.locals) {
                    let local = &self.prog.locals[id];
                    if local.is_param && local.is_ctx_local {
                        self.gen_getlcl(local.var_num, true);
                        self.gen_local_asi(id, true, 1, AsiOp::Simple, None, false);
                    }
                }
            }
        }

        if let Some(local) = body.varargs_list_local {
            self.cg.write_op(Op::PushParLst);
            self.cg.write(body.argc as u8);
            self.cg.note_push();
            self.gen_setlcl(local);
        }

        // Parameters passed by name and optional parameters are locals,
        // filled in here. Those with default values come last, in the
        // order they were written, since one's value can use another;
        // the rest are in the order of the table of locals.
        if let Some(lcltab) = body.lcltab {
            let mut with_defval = Vec::new();
            for id in enum_order(&self.prog.tabs[lcltab], &self.prog.locals) {
                let local = &self.prog.locals[id];
                if local.is_param {
                    continue;
                }
                if local.defval.is_some() {
                    with_defval.push(id);
                } else if local.is_named_param {
                    self.gen_named_param(id);
                } else if local.is_opt_param {
                    self.gen_opt_positional(id);
                }
            }
            with_defval.sort_by_key(|&id| self.prog.locals[id].defval_seqno);
            for id in with_defval {
                if self.prog.locals[id].is_named_param {
                    self.gen_named_param(id);
                } else {
                    self.gen_opt_positional(id);
                }
            }
        }

        // Code compiled while the program runs, given as a function,
        // takes the method context of the function it was compiled in,
        // which its object keeps as its first element.
        let main_ctx = self.prog.dynamic.as_ref().map(|dynamic| dynamic.main_ctx);
        if let Some((main, ctx_use)) = main_ctx
            && main == body_index
            && ctx_use != crate::dynamic::DynCtxUse::None
        {
            self.cg.write_op(Op::PushCtxEle);
            self.cg.write(CTX_ELE_INVOKEE);
            self.gen_int(1);
            self.cg.write_op(Op::Index);
            self.cg.note_push();
            self.cg.note_pop();
            self.cg
                .write_op(if ctx_use == crate::dynamic::DynCtxUse::Full {
                    Op::LoadCtx
                } else {
                    Op::SetSelf
                });
            self.cg.note_pop();
        }

        // A nested function keeps in locals the contexts it was made
        // with, which it finds in the function object it is run as.
        let outer_has_full = self.outermost_needs_full(closure);
        for (index, &(level, var)) in closure.ctx_levels.iter().enumerate() {
            self.cg.write_op(Op::PushCtxEle);
            self.cg.write(CTX_ELE_INVOKEE);
            self.gen_int(index as i32 + 2);
            self.cg.write_op(Op::Index);
            self.cg.note_push();
            self.cg.note_pop();
            // The context of the function just outside has the `self`
            // of the method all this is in.
            if level == 1
                && !closure.is_anon_method
                && body.self_valid
                && (closure.self_referenced || closure.full_ctx_referenced)
            {
                self.cg.write_op(Op::Dup);
                self.cg.note_push();
                self.gen_int(i32::from(CTX_METHOD_CTX));
                self.cg.write_op(Op::Index);
                self.cg.write_op(if outer_has_full {
                    Op::LoadCtx
                } else {
                    Op::SetSelf
                });
                self.cg.note_pops(2);
            }
            self.gen_setlcl_stk(var, false);
        }

        // And a method whose nested functions use its `self` puts that
        // in its context for them.
        if closure.has_local_ctx
            && body.self_valid
            && (closure.local_ctx_needs_self || closure.local_ctx_needs_full)
        {
            self.cg
                .write_op(if closure.local_ctx_needs_full || outer_has_full {
                    Op::StoreCtx
                } else {
                    Op::PushSelf
                });
            self.cg.note_push();
            if closure.local_ctx_var <= 255 {
                self.cg.write_op(Op::SetIndLcl1I8);
                self.cg.write(closure.local_ctx_var as u8);
                self.cg.write(CTX_METHOD_CTX);
                self.cg.note_pop();
            } else {
                self.gen_getlcl(closure.local_ctx_var, false);
                self.gen_int(i32::from(CTX_METHOD_CTX));
                self.cg.write_op(Op::SetInd);
                self.cg.write_op(Op::Disc);
                self.cg.note_pops(3);
            }
        }

        self.top_stm_pos = body.stm_pos.map(|pos| (&body.stm as *const Stmt, pos));
        self.gen_stmt(&body.stm);
        self.top_stm_pos = None;

        // Falling off the end returns nil, or from a constructor the
        // object constructed.
        if !self.cg.can_skip_op() {
            self.add_line_rec(body.end_file, body.end_line);
            if self.in_constructor {
                self.cg.write_op(Op::PushSelf);
                self.cg.write_op(Op::RetVal);
            } else {
                self.cg.write_op(Op::RetNil);
            }
        }
        // The names of arguments passed by name, for each call that
        // has any, the latest call first.
        for (label, list) in std::mem::take(&mut self.named_tabs).into_iter().rev() {
            let names = std::mem::take(&mut self.named_lists[list]);
            self.cg.def_label(label);
            self.cg.write_op(Op::NamedArgTab);
            let start = self.cg.ofs();
            self.cg.write2(0);
            self.cg.write2(names.len() as u16);
            // Where each name starts, and where the last one ends.
            let index = self.cg.ofs();
            for _ in 0..=names.len() {
                self.cg.write2(0);
            }
            for (at, name) in names.iter().enumerate() {
                self.cg
                    .write2_at(index + 2 * at, (self.cg.ofs() - index) as u16);
                for &byte in name {
                    self.cg.write(byte);
                }
            }
            self.cg
                .write2_at(index + 2 * names.len(), (self.cg.ofs() - index) as u16);
            self.cg.write2_at(start, (self.cg.ofs() - start - 2) as u16);
        }
        if self.errors == 0 {
            self.remove_jumps_to_jumps(code_start);
        }
        self.cg.write2_at(method_ofs + 2, self.local_cnt as u16);
        let stack = self.cg.max_sp_depth() as usize + self.local_cnt;
        self.cg.write2_at(method_ofs + 4, stack as u16);
        let end = self.rel_ofs();
        let mut tab = body.lcltab;
        while let Some(id) = tab {
            self.add_to_range(id, end);
            tab = self.prog.tabs[id].parent;
        }

        // The table of exception handlers, if the function has any.
        if !self.exc_table.is_empty() {
            self.cg.write2_at(method_ofs + 6, end as u16);
            self.cg.write2(self.exc_table.len() as u16);
            for entry in std::mem::take(&mut self.exc_table) {
                self.cg.write2(entry.start_ofs as u16);
                self.cg.write2(entry.end_ofs as u16);
                self.write_obj_id(entry.class);
                self.cg.write2(entry.handler_ofs as u16);
            }
        }

        // A jump to a label that was never come to: a `goto` into
        // code that was left out because nothing could reach it. The
        // reference finds this when it lets go of the function's
        // labels, and calls it its own error.
        let unresolved = self.cg.take_unresolved();
        if unresolved != 0 {
            self.errors += 1;
            self.out.diagnostics.push(GenDiagnostic {
                file: usize::MAX,
                line: 0,
                severity: Severity::Internal,
                message: format!("{unresolved} unresolved branches"),
            });
        }

        self.build_debug_table(method_ofs);

        // Labels nothing jumps to. The reference goes through them in
        // the order of its table, as it does with locals; but the table
        // of labels is made the size of the table of global names.
        let bucket = |label: &&LabelSym| {
            label
                .name
                .iter()
                .map(|&byte| usize::from(byte))
                .sum::<usize>()
                % LABEL_HASH_SIZE
        };
        let mut labels: Vec<&LabelSym> = body.labels.iter().rev().collect();
        labels.sort_by_key(bucket);
        for label in labels {
            if label.flags.get() & (flow::GOTO | flow::BREAK | flow::CONT) == 0 {
                self.out.diagnostics.push(GenDiagnostic {
                    file: label.file,
                    line: label.line,
                    severity: Severity::Warning,
                    message: format!("label \"{}\" is not referenced", show_name(&label.name)),
                });
            }
        }
        let len = self.cg.ofs() - method_ofs;
        self.out.max_bytecode_len = self.out.max_bytecode_len.max(len);
    }

    /// Write the table of local names that follows a function's code.
    /// It is there even in a program not compiled for debugging, so that
    /// a running program can ask about its own locals.
    fn build_debug_table(&mut self, method_ofs: usize) {
        // Scopes without locals are left out, and the rest numbered.
        let frames = self.frames.clone();
        let mut frame_cnt = 0;
        for &frame in &frames {
            let tab = &mut self.prog.tabs[frame];
            if tab.entries.is_empty() {
                tab.list_index = 0;
            } else {
                frame_cnt += 1;
                tab.list_index = frame_cnt;
            }
        }
        if frame_cnt == 0 && !self.debug {
            return;
        }
        let table_ofs = self.cg.ofs() - method_ofs;
        self.cg.write2_at(method_ofs + 8, table_ofs as u16);
        // The line records, in a program compiled for debugging: where
        // each starts, the source file and line, and the scope, the
        // nearest one around it that is in the table if it is not.
        let recs = std::mem::take(&mut self.line_recs);
        self.cg.write2(recs.len() as u16);
        for rec in &recs {
            self.cg.write2(rec.ofs as u16);
            self.cg.write2(rec.source_id as u16);
            self.cg.write4(rec.line);
            let mut frame = rec.frame;
            while let Some(id) = frame {
                if self.prog.tabs[id].list_index != 0 {
                    break;
                }
                frame = self.prog.tabs[id].parent;
            }
            let index = frame.map_or(0, |id| self.prog.tabs[id].list_index);
            self.cg.write2(index as u16);
        }
        let post_ptr_ofs = self.cg.ofs();
        self.cg.write2(0);
        self.cg.write2(frame_cnt as u16);
        let mut index_ofs = self.cg.ofs();
        for _ in 0..frame_cnt {
            self.cg.write2(0);
        }
        for &frame in &frames {
            if self.prog.tabs[frame].list_index == 0 {
                continue;
            }
            let here = self.cg.ofs();
            self.cg.write2_at(index_ofs, (here - index_ofs) as u16);
            index_ofs += 2;

            // The nearest scope around this one that is in the table.
            let mut parent = self.prog.tabs[frame].parent;
            while let Some(id) = parent {
                if self.prog.tabs[id].list_index != 0 {
                    break;
                }
                parent = self.prog.tabs[id].parent;
            }
            let parent_index = parent.map_or(0, |id| self.prog.tabs[id].list_index);
            self.cg.write2(parent_index as u16);
            let order = enum_order(&self.prog.tabs[frame], &self.prog.locals);
            self.cg.write2(order.len() as u16);
            self.cg.write2(self.prog.tabs[frame].start_ofs as u16);
            self.cg.write2(self.prog.tabs[frame].end_ofs as u16);
            for id in order {
                let local = &self.prog.locals[id];
                let name = local.name.clone();
                // Code compiled while the program runs has the names in
                // itself.
                let inline = name.len() <= 2 || self.prog.dynamic.is_some();
                let mut flags = 0u16;
                if local.is_param {
                    flags |= 0x0001;
                }
                if !inline {
                    flags |= 0x0004;
                }
                if local.is_ctx_local {
                    flags |= 0x0002;
                }
                let is_ctx_local = local.is_ctx_local;
                self.cg.write2(if is_ctx_local {
                    local.ctx_var_num
                } else {
                    local.var_num
                } as u16);
                self.cg.write2(flags);
                // The index within a shared context, for a local that is
                // shared with nested functions. For any other local the
                // reference writes whatever its memory holds.
                if is_ctx_local {
                    self.cg.write2(self.prog.ctx_arr_idx(id) as u16);
                } else {
                    self.out.unspecified.push(self.cg.ofs());
                    self.cg.write2(0);
                }
                if inline {
                    self.cg.write2(name.len() as u16);
                    for byte in name {
                        self.cg.write(byte);
                    }
                } else {
                    self.local_names.push((self.cg.ofs(), name));
                    self.cg.write4(0);
                }
            }
        }
        let here = self.cg.ofs();
        self.cg
            .write2_at(post_ptr_ofs, (here - post_ptr_ofs) as u16);
        self.cg.write4(0);
    }

    /// Put the names of locals into the constants, each once, and point
    /// the tables at them.
    ///
    /// The reference first collects the names in a stream of their own,
    /// every use of every name, and then moves them to the constants. A
    /// name longer than a symbol can be (the tokenizer warns about those
    /// but passes them on whole) is not moved, and the table is left
    /// pointing at where the name was in that stream. Hexicon writes
    /// the same number there.
    fn finish_local_names(&mut self) {
        const SYM_MAX_LEN: usize = 80;
        const NAME_PAGE: usize = 65536;
        let mut known: HashMap<Vec<u8>, usize> = HashMap::new();
        // Where the next name would go in the reference's stream, which
        // it lays out in pages like a pool.
        let mut stream_ofs = 0;
        let mut page_rem = NAME_PAGE;
        for (ofs, name) in std::mem::take(&mut self.local_names) {
            let len = 2 + name.len();
            if len > page_rem {
                stream_ofs += page_rem;
                page_rem = NAME_PAGE;
            }
            let stream_addr = stream_ofs;
            stream_ofs += len;
            page_rem -= len;
            // A name from a function that a later source file replaced
            // still has its place in the stream, and in the constants,
            // but there is nowhere left that refers to it.
            let gone = ofs == usize::MAX;
            if name.len() > SYM_MAX_LEN {
                if !gone {
                    self.cg.write4_at(ofs, stream_addr as u32);
                }
                continue;
            }
            let anchor = match known.get(&name) {
                Some(anchor) => *anchor,
                None => {
                    let anchor = self.out.const_anchors.len();
                    self.out.const_anchors.push(Anchor {
                        ofs: self.out.consts.len(),
                    });
                    self.out
                        .consts
                        .extend_from_slice(&(name.len() as u16).to_le_bytes());
                    self.out.consts.extend_from_slice(&name);
                    known.insert(name, anchor);
                    anchor
                }
            };
            if !gone {
                self.out.fixups.push(Fixup {
                    ofs,
                    target: Target::Const(anchor),
                });
            }
        }
    }

    /// Shorten jumps that land on jumps, and turn jumps that land on a
    /// return into the return. This goes over the finished code of one
    /// function.
    fn remove_jumps_to_jumps(&mut self, start_ofs: usize) {
        let end_ofs = self.cg.ofs();
        let code = &mut self.cg.code;
        let read2 =
            |code: &[u8], at: usize| i64::from(i16::from_le_bytes([code[at], code[at + 1]]));
        let readu2 =
            |code: &[u8], at: usize| usize::from(u16::from_le_bytes([code[at], code[at + 1]]));
        let jump_target = |code: &[u8], at: usize| (at as i64 + 1 + read2(code, at + 1)) as usize;
        let is = |byte: u8, op: Op| byte == op as u8;
        let mut prv_op = Op::Nop as u8;
        let mut ofs = start_ofs;
        while ofs < end_ofs {
            let mut op = code[ofs];
            match Op::from_byte(op) {
                Some(Op::RetVal) => {
                    if is(prv_op, Op::PushTrue) {
                        code[ofs - 1] = Op::RetTrue as u8;
                    } else if is(prv_op, Op::PushNil) {
                        code[ofs - 1] = Op::RetNil as u8;
                    } else if is(prv_op, Op::GetR0) {
                        code[ofs - 1] = Op::Ret as u8;
                    }
                    ofs += 1;
                }
                Some(Op::PushStrI) => ofs += 3 + readu2(code, ofs + 1),
                Some(Op::Switch) => ofs += 1 + 2 + 7 * readu2(code, ofs + 1) + 2,
                Some(Op::NamedArgTab) => ofs += 1 + 2 + readu2(code, ofs + 1),
                Some(Op::Jmp) => {
                    let orig_target = jump_target(code, ofs);
                    let mut target = orig_target;
                    let mut chain_len = 0;
                    while chain_len < 20 {
                        chain_len += 1;
                        let target_op = code[target];
                        match Op::from_byte(target_op) {
                            Some(Op::RetVal) if is(prv_op, Op::GetR0) => {
                                code[ofs - 1] = Op::Ret as u8;
                                break;
                            }
                            Some(Op::RetVal | Op::RetNil | Op::RetTrue | Op::Ret | Op::Throw) => {
                                code[ofs] = target_op;
                                code[ofs + 1] = Op::Nop as u8;
                                code[ofs + 2] = Op::Nop as u8;
                                break;
                            }
                            Some(Op::Jmp) => {
                                target = jump_target(code, target);
                                if target == ofs {
                                    break;
                                }
                            }
                            _ => {
                                if target != orig_target {
                                    let offset = target as i64 - (ofs as i64 + 1);
                                    code[ofs + 1..ofs + 3]
                                        .copy_from_slice(&(offset as i16).to_le_bytes());
                                }
                                break;
                            }
                        }
                    }
                    ofs += 3;
                }
                Some(
                    Op::Jt
                    | Op::Jf
                    | Op::Je
                    | Op::Jne
                    | Op::Jgt
                    | Op::Jge
                    | Op::Jlt
                    | Op::Jle
                    | Op::Jst
                    | Op::Jsf
                    | Op::JNil
                    | Op::JNotNil
                    | Op::JR0T
                    | Op::JR0F,
                ) => {
                    let orig_target = jump_target(code, ofs);
                    let mut target = orig_target;
                    let mut chain_len = 0;
                    while chain_len < 20 {
                        chain_len += 1;
                        let target_op = Op::from_byte(code[target]);
                        if target_op == Some(Op::Jmp) {
                            target = jump_target(code, target);
                            continue;
                        }
                        // A jump that keeps its value when taken, landing
                        // on another test of the same value, already
                        // knows how that test comes out.
                        let (same, opposite, plain) = if is(op, Op::Jsf) {
                            (Op::Jsf, Op::Jst, Op::Jf)
                        } else if is(op, Op::Jst) {
                            (Op::Jst, Op::Jsf, Op::Jt)
                        } else {
                            break;
                        };
                        let plain_opposite = if plain == Op::Jf { Op::Jt } else { Op::Jf };
                        match target_op {
                            Some(found) if found == same => target = jump_target(code, target),
                            Some(found) if found == opposite || found == plain_opposite => {
                                code[ofs] = plain as u8;
                                op = plain as u8;
                                target += 3;
                            }
                            Some(found) if found == plain => {
                                code[ofs] = plain as u8;
                                op = plain as u8;
                                target = jump_target(code, target);
                            }
                            _ => break,
                        }
                    }
                    if target != orig_target {
                        let offset = target as i64 - (ofs as i64 + 1);
                        code[ofs + 1..ofs + 3].copy_from_slice(&(offset as i16).to_le_bytes());
                    }
                    ofs += 3;
                }
                Some(other) => {
                    ofs += match other.layout() {
                        Layout::Fixed(operands) => {
                            1 + operands.iter().map(|kind| kind.size()).sum::<usize>()
                        }
                        _ => 1,
                    };
                }
                None => ofs += 1,
            }
            prv_op = op;
        }
    }

    // ---- labels ----

    /// Write a jump to a place further on, unless the jump could never
    /// be reached.
    fn gen_jump_ahead(&mut self, op: Op) -> Option<Label> {
        if self.cg.can_skip_op() {
            return None;
        }
        let label = self.cg.new_label();
        self.cg.write_jump(op, label);
        Some(label)
    }

    fn def_label_pos(&mut self, label: Option<Label>) {
        if let Some(label) = label {
            self.cg.def_label(label);
        }
    }

    // ---- statements ----

    fn gen_substm(&mut self, sub: &Stmt, outer: &Stmt) {
        self.gen_substm_at(sub, outer.file, outer.line);
    }

    /// Generate a statement that is part of another, which is at the
    /// given place in the source.
    fn gen_substm_at(&mut self, sub: &Stmt, file: usize, line: u32) {
        self.set_line(sub);
        self.gen_stmt(sub);
        self.cur_file = file;
        self.cur_line = line;
        self.line_info = (file, line);
    }

    /// How control can leave a statement, as far as can be told now
    /// that constants are known.
    fn flow_of(&self, stmt: &Stmt) -> u32 {
        Flow {
            warn: false,
            syntax_only: false,
            labels: &self.labels,
            mark: &self.last_labels,
            warnings: Vec::new(),
        }
        .control_flow(stmt)
    }

    fn gen_stmt(&mut self, stmt: &Stmt) {
        match &stmt.kind {
            StmtKind::Null => {}
            StmtKind::Comp {
                stmts,
                symtab,
                end_file,
                end_line,
            } => {
                let old_frame = self.set_local_frame(*symtab);
                self.add_stmt_line_rec(stmt);
                for sub in stmts {
                    self.set_line(sub);
                    self.gen_stmt(sub);
                }
                self.add_line_rec(*end_file, *end_line);
                self.set_local_frame(old_frame);
            }
            StmtKind::Expr(expr) => {
                self.set_line(stmt);
                self.add_stmt_line_rec(stmt);
                self.gen_expr(expr, true, true);
            }
            StmtKind::If {
                cond,
                then_part,
                else_part,
            } => {
                self.add_stmt_line_rec(stmt);
                self.gen_if(stmt, cond, then_part.as_deref(), else_part.as_deref());
            }
            StmtKind::Return(expr) => {
                self.add_stmt_line_rec(stmt);
                self.gen_return(expr.as_ref());
            }
            StmtKind::For {
                init,
                cond,
                reinit,
                body,
                symtab,
                id,
            } => {
                let old_frame = self.set_local_frame(*symtab);
                self.add_stmt_line_rec(stmt);
                let mut for_ins = Vec::new();
                if let Some(init) = init {
                    self.gen_expr(init, true, true);
                    init.for_ins(&mut for_ins);
                }
                let top_lbl = self.cg.new_label_here();
                let cont_lbl = self.cg.new_label();
                let end_lbl = self.cg.new_label();
                self.enclosing.push(Enclosing::Loop {
                    id: *id,
                    break_lbl: end_lbl,
                    cont_lbl,
                });
                for for_in in &for_ins {
                    self.gen_for_in_cond(for_in, end_lbl);
                }
                if let Some(cond) = cond {
                    self.gen_cond(cond, None, Some(end_lbl));
                }
                if let Some(body) = body {
                    self.gen_substm(body, stmt);
                }
                self.cg.def_label(cont_lbl);
                if let Some(reinit) = reinit {
                    self.gen_expr(reinit, true, true);
                }
                for for_in in &for_ins {
                    self.gen_for_in_reinit(for_in);
                }
                self.cg.write_jump(Op::Jmp, top_lbl);
                self.cg.def_label(end_lbl);
                self.enclosing.pop();
                self.set_local_frame(old_frame);
            }
            StmtKind::Foreach {
                iter,
                coll,
                body,
                symtab,
                id,
                iter_local,
            } => {
                let old_frame = self.set_local_frame(*symtab);
                self.add_stmt_line_rec(stmt);
                let local = self.prog.locals[*iter_local].var_num;
                self.gen_iter_init(coll, local, "foreach");
                let top_lbl = self.cg.new_label_here();
                let end_lbl = self.cg.new_label();
                self.enclosing.push(Enclosing::Loop {
                    id: *id,
                    break_lbl: end_lbl,
                    cont_lbl: top_lbl,
                });
                self.gen_iter_cond(iter, local, end_lbl);
                if let Some(body) = body {
                    self.gen_substm(body, stmt);
                }
                self.cg.write_jump(Op::Jmp, top_lbl);
                self.cg.def_label(end_lbl);
                self.enclosing.pop();
                self.set_local_frame(old_frame);
            }
            StmtKind::While { cond, body, id } => {
                self.add_stmt_line_rec(stmt);
                let top_lbl = self.cg.new_label_here();
                let end_lbl = self.cg.new_label();
                self.enclosing.push(Enclosing::Loop {
                    id: *id,
                    break_lbl: end_lbl,
                    cont_lbl: top_lbl,
                });
                self.gen_cond(cond, None, Some(end_lbl));
                if let Some(body) = body {
                    self.gen_substm(body, stmt);
                }
                self.cg.write_jump(Op::Jmp, top_lbl);
                self.cg.def_label(end_lbl);
                self.enclosing.pop();
            }
            StmtKind::DoWhile {
                body,
                cond,
                id,
                while_file,
                while_line,
            } => {
                self.add_stmt_line_rec(stmt);
                let top_lbl = self.cg.new_label_here();
                let end_lbl = self.cg.new_label();
                let cont_lbl = self.cg.new_label();
                self.enclosing.push(Enclosing::Loop {
                    id: *id,
                    break_lbl: end_lbl,
                    cont_lbl,
                });
                if let Some(body) = body {
                    self.gen_substm(body, stmt);
                }
                self.add_line_rec(*while_file, *while_line);
                self.cg.def_label(cont_lbl);
                self.gen_cond(cond, Some(top_lbl), None);
                self.cg.def_label(end_lbl);
                self.enclosing.pop();
            }
            StmtKind::Break(label) => {
                self.add_stmt_line_rec(stmt);
                self.gen_break_or_continue(label.as_deref(), true);
            }
            StmtKind::Continue(label) => {
                self.add_stmt_line_rec(stmt);
                self.gen_break_or_continue(label.as_deref(), false);
            }
            StmtKind::Switch {
                expr,
                body,
                case_cnt,
                has_default,
                id,
            } => {
                self.add_stmt_line_rec(stmt);
                self.gen_switch(stmt, expr, body, *case_cnt, *has_default, *id);
            }
            StmtKind::Case { expr, stm } => {
                let Some(slots) = self.switches.last_mut() else {
                    self.error("case/default without switch or non-constant case expression");
                    return;
                };
                let slot = slots.next_case;
                slots.next_case += 7;
                if let Some(value) = expr.as_const() {
                    self.write_const_at(slot, value);
                }
                // The jump is measured from where it is written.
                let jump = self.cg.ofs() - (slot + 5);
                self.cg.write2_at(slot + 5, jump as u16);
                self.cg.clear_peephole();
                if let Some(stm) = stm {
                    self.gen_substm(stm, stmt);
                }
            }
            StmtKind::Default(stm) => {
                let Some(slots) = self.switches.last() else {
                    self.error("case/default without switch or non-constant case expression");
                    return;
                };
                let slot = slots.default_slot;
                let jump = self.cg.ofs() - slot;
                self.cg.write2_at(slot, jump as u16);
                self.cg.clear_peephole();
                if let Some(stm) = stm {
                    self.gen_substm(stm, stmt);
                }
            }
            StmtKind::Label { label, stm, id } => {
                // `continue` can name the label if what it is on is a
                // loop, possibly under more labels.
                let mut inner = stm.as_deref();
                while let Some(Stmt {
                    kind: StmtKind::Label { stm, .. },
                    ..
                }) = inner
                {
                    inner = stm.as_deref();
                }
                let labels_loop = matches!(
                    inner.map(|stmt| &stmt.kind),
                    Some(
                        StmtKind::For { .. }
                            | StmtKind::Foreach { .. }
                            | StmtKind::While { .. }
                            | StmtKind::DoWhile { .. }
                    )
                );
                self.enclosing.push(Enclosing::Label {
                    id: *id,
                    label: *label,
                    break_lbl: None,
                    labels_loop,
                });
                // A label whose name was taken still gets a place, which
                // nothing can jump to.
                let goto_lbl = match label {
                    Some(label) => self.goto_label(*label),
                    None => self.cg.new_label(),
                };
                self.cg.def_label(goto_lbl);
                self.add_stmt_line_rec(stmt);
                if let Some(stm) = stm {
                    self.gen_substm(stm, stmt);
                }
                if let Some(Enclosing::Label {
                    break_lbl: Some(break_lbl),
                    ..
                }) = self.enclosing.pop()
                {
                    self.cg.def_label(break_lbl);
                }
            }
            StmtKind::Goto(name) => {
                self.add_stmt_line_rec(stmt);
                self.gen_goto(name);
            }
            StmtKind::Try {
                id,
                body,
                catches,
                finally,
            } => {
                self.add_stmt_line_rec(stmt);
                self.gen_try(stmt, *id, body.as_deref(), catches, finally.as_deref());
            }
            StmtKind::MmStub(func) => {
                // All the arguments as a list, and the function.
                self.cg.write_op(Op::PushParLst);
                self.cg.write(0);
                self.cg.note_push();
                self.cg.write_op(Op::PushFnPtr);
                self.gen_func_ref(*func);
                self.cg.note_pushes(2);
                if let Some(Global::Func(call)) = self
                    .prog
                    .globals
                    .get(b"_multiMethodCall".as_slice())
                    .copied()
                {
                    let call = Expr::constant(Value::FuncPtr(call));
                    self.gen_call(&call, false, 2, false, None);
                }
                self.cg.write_op(Op::RetVal);
                self.cg.note_pop();
            }
            StmtKind::MmInit => self.gen_mm_init(),
            StmtKind::ImplicitCtor(superclasses) => {
                for superclass in superclasses {
                    // The arguments, which are in the first local as a
                    // list, and a count of the others, which is zero.
                    self.cg.write_op(Op::Push0);
                    self.cg.note_push();
                    self.gen_getlcl(0, false);
                    self.cg.write_op(Op::MakeLstPar);
                    self.cg.note_push();
                    self.cg.note_pop();
                    self.cg.write_op(Op::VarArgc);
                    self.cg.write_op(Op::ExpInherit);
                    self.cg.write(0);
                    self.cg.write2(1);
                    self.write_obj_id(*superclass);
                    self.cg.note_pops(2);
                }
            }
            StmtKind::Throw(expr) => {
                self.add_stmt_line_rec(stmt);
                self.gen_expr(expr, false, false);
                self.cg.write_op(Op::Throw);
                self.cg.note_pop();
            }
            StmtKind::StaticPropInit(expr, prop) => {
                self.add_stmt_line_rec(stmt);
                // The value, or nil if the expression has none; it is
                // both stored and returned.
                let depth = self.cg.sp_depth();
                self.gen_expr(expr, false, false);
                if self.cg.sp_depth() <= depth {
                    self.cg.write_op(Op::PushNil);
                    self.cg.note_push();
                }
                self.cg.write_op(Op::Dup);
                self.cg.note_push();
                self.cg.write_op(Op::SetPropSelf);
                self.write_prop_id(self.prog.props[*prop].id);
                self.cg.note_pop();
                self.cg.write_op(Op::RetVal);
                self.cg.note_pop();
            }
        }
    }

    fn find_label(&self, name: &[u8]) -> Option<usize> {
        self.labels.iter().position(|label| label.name == name)
    }

    /// Where a label is, for jumping to.
    fn goto_label(&mut self, label: usize) -> Label {
        if let Some(found) = self.goto_labels[label] {
            return found;
        }
        let made = self.cg.new_label();
        self.goto_labels[label] = Some(made);
        made
    }

    /// Call a `finally` block, as a jump to it that it comes back from.
    /// What follows is, for the debugger, at the end of the block.
    fn gen_jsr_finally(&mut self, finally_lbl: Option<Label>, end: Option<(usize, u32)>) {
        if let Some(label) = finally_lbl {
            self.cg.write_jump(Op::LJsr, label);
            // The jump pushes where to come back to, and the block pops
            // it.
            self.cg.note_push();
            self.cg.note_pop();
            if let Some((file, line)) = end {
                self.add_line_rec(file, line);
            }
        }
    }

    /// Add a line record for a statement, where it starts.
    fn add_stmt_line_rec(&mut self, stmt: &Stmt) {
        let (file, line) = match self.top_stm_pos {
            Some((top, pos)) if std::ptr::eq(top, stmt) => pos,
            _ => (stmt.file, stmt.line),
        };
        self.add_line_rec(file, line);
    }

    /// Add a line record for what is generated next, in a program
    /// compiled for debugging. The peephole optimizer is told not to
    /// combine what is before it with what is after.
    fn add_line_rec(&mut self, file: usize, line: u32) {
        if self.debug {
            self.cg.clear_peephole();
            self.add_line_rec_raw(file, line);
        }
    }

    /// Add a line record, as the reference's code stream does: one at
    /// the same offset as the last takes its place, and one at the same
    /// place in the source as the last is not needed.
    fn add_line_rec_raw(&mut self, file: usize, line: u32) {
        // A place with no file, or line zero, which no line is, is no
        // place: what the parser has before it has seen a statement.
        if !self.debug || file == usize::MAX || line == 0 {
            return;
        }
        let ofs = self.cg.ofs() - self.method_ofs;
        let source_id = self.file_base + file;
        let rec = LineRec {
            ofs,
            source_id,
            line,
            frame: self.cur_frame,
        };
        match self.line_recs.last_mut() {
            Some(last) if last.source_id == source_id && last.line == line => {}
            Some(last) if last.ofs == ofs => *last = rec,
            _ => self.line_recs.push(rec),
        }
    }

    fn gen_break_or_continue(&mut self, label: Option<&[u8]>, is_break: bool) {
        if self.gen_jump_out(label, is_break) {
            return;
        }
        let message = match (label, is_break) {
            (None, true) => "invalid \"break\" - not in a for/while/do/switch".to_owned(),
            (None, false) => "invalid \"continue\" - not in a for/while/do".to_owned(),
            (Some(label), true) => format!(
                "invalid \"break\" - no label \"{}\" on any enclosing statement",
                show_name(label)
            ),
            (Some(label), false) => format!(
                "invalid \"continue\" - no label \"{}\" on any enclosing loop",
                show_name(label)
            ),
        };
        self.error(message);
    }

    /// Generate a `break` or a `continue`: each statement around it,
    /// from the nearest outward, either does the jump or does what
    /// leaving it takes and passes the job on. Nothing may take it.
    /// The statements a jump out of the one being generated leaves, from
    /// the nearest outward, as places in `enclosing`. The nearest is
    /// the one being generated; after that the reference goes by what
    /// the parser took each to be inside of, and the parser takes a
    /// `catch` block to be outside its `try`. So a jump out of a loop
    /// in a `catch` block does not leave the `try`, and does not call
    /// its `finally` block, though a jump straight out of the `catch`
    /// block does.
    fn leaving(&self) -> Vec<usize> {
        let mut chain = Vec::new();
        let mut next = self.enclosing.len().checked_sub(1);
        while let Some(index) = next {
            chain.push(index);
            let parent = self
                .prog
                .enclosure_parents
                .get(&self.enclosing[index].id())
                .copied()
                .flatten();
            next = parent.and_then(|parent| {
                self.enclosing[..index]
                    .iter()
                    .rposition(|enclosing| enclosing.id() == parent)
            });
        }
        chain
    }

    fn gen_jump_out(&mut self, label: Option<&[u8]>, is_break: bool) -> bool {
        // The label named, and whether the function has it.
        let named = label.map(|name| self.find_label(name));
        for index in self.leaving() {
            match self.enclosing[index] {
                Enclosing::Loop {
                    break_lbl,
                    cont_lbl,
                    ..
                } => {
                    if label.is_none() {
                        let target = if is_break { break_lbl } else { cont_lbl };
                        self.cg.write_jump(Op::Jmp, target);
                        return true;
                    }
                }
                Enclosing::Switch { break_lbl, .. } => {
                    if is_break && label.is_none() {
                        // With nowhere to go, the `break` cannot be
                        // reached, and there is nothing to do.
                        if let Some(target) = break_lbl {
                            self.cg.write_jump(Op::Jmp, target);
                        }
                        return true;
                    }
                }
                Enclosing::Label {
                    id,
                    label: own,
                    break_lbl,
                    labels_loop,
                } => {
                    // A statement whose label name was taken answers to
                    // any name the function does not have.
                    if named != Some(own) || self.labels.is_empty() {
                        continue;
                    }
                    if is_break {
                        let target = break_lbl.unwrap_or_else(|| self.cg.new_label());
                        self.enclosing[index] = Enclosing::Label {
                            id,
                            label: own,
                            break_lbl: Some(target),
                            labels_loop,
                        };
                        self.cg.write_jump(Op::Jmp, target);
                        return true;
                    }
                    // `continue` goes to the loop the label is on.
                    if !labels_loop {
                        return false;
                    }
                    let cont_lbl = self.enclosing[index..].iter().find_map(|enclosing| {
                        if let Enclosing::Loop { cont_lbl, .. } = enclosing {
                            Some(*cont_lbl)
                        } else {
                            None
                        }
                    });
                    let Some(target) = cont_lbl else {
                        return false;
                    };
                    self.cg.write_jump(Op::Jmp, target);
                    return true;
                }
                Enclosing::Try {
                    finally_lbl,
                    finally_end,
                    ..
                } => self.gen_jsr_finally(finally_lbl, finally_end),
                Enclosing::Finally { .. } => {}
            }
        }
        false
    }

    fn gen_switch(
        &mut self,
        stmt: &Stmt,
        expr: &Expr,
        body: &Stmt,
        case_cnt: usize,
        has_default: bool,
        id: usize,
    ) {
        self.gen_expr(expr, false, false);
        // If nothing after the switch can be reached, there is no end
        // to jump to.
        let end_lbl = (self.flow_of(stmt) & flow::NEXT != 0).then(|| self.cg.new_label());
        self.enclosing.push(Enclosing::Switch {
            id,
            break_lbl: end_lbl,
        });
        self.cg.write_op(Op::Switch);
        self.cg.note_pop();
        // The table: for each case a value and a jump, filled in as the
        // cases are come to, and then the jump for no match.
        self.cg.write2(case_cnt as u16);
        let next_case = self.cg.ofs();
        for _ in 0..case_cnt * 7 {
            self.cg.write(0);
        }
        let default_slot = self.cg.ofs();
        match end_lbl {
            Some(end_lbl) if !has_default => self.cg.write_label_ofs(end_lbl),
            _ => self.cg.write2(0),
        }
        self.switches.push(SwitchSlots {
            next_case,
            default_slot,
        });
        self.gen_substm(body, stmt);
        if let Some(end_lbl) = end_lbl {
            self.cg.def_label(end_lbl);
        }
        self.switches.pop();
        self.enclosing.pop();
    }

    /// Write a constant into the code as a kind and four bytes, at a
    /// place already there.
    fn write_const_at(&mut self, ofs: usize, value: &ConstVal) {
        let (kind, bytes) = match &value.value {
            Value::Nil => (datatype::NIL, 0),
            Value::True => (datatype::TRUE, 0),
            Value::Int(int) => (datatype::INT, *int as u32),
            Value::SStr(text) => {
                self.add_const_str(text, false, ofs + 1);
                (datatype::SSTRING, 0)
            }
            Value::List(elements) => {
                self.add_const_list(elements, false, ofs + 1);
                (datatype::LIST, 0)
            }
            Value::FuncPtr(func) => {
                self.add_fixup(false, ofs + 1, Target::Func(*func));
                (datatype::FUNCPTR, 0)
            }
            Value::Obj(obj) => {
                self.out.id_fixups.push((ofs + 1, IdKind::Obj));
                (datatype::OBJ, *obj)
            }
            Value::Float { .. } | Value::ReStr(_) => {
                let id = self.gen_const_obj(&value.value).expect("an object");
                self.out.id_fixups.push((ofs + 1, IdKind::Obj));
                (datatype::OBJ, id)
            }
            Value::Enum(id) => {
                self.out.id_fixups.push((ofs + 1, IdKind::Enum));
                (datatype::ENUM, *id)
            }
            Value::Prop(prop) => {
                // A property's number is two bytes; the other two are
                // as the reference's memory had them.
                self.out.id_fixups.push((ofs + 1, IdKind::Prop));
                self.out.unspecified.push(ofs + 3);
                (datatype::PROP, u32::from(*prop))
            }
        };
        if matches!(value.value, Value::Nil | Value::True) {
            // The reference writes the kind and leaves the rest as its
            // memory had it.
            self.out.unspecified.push(ofs + 1);
            self.out.unspecified.push(ofs + 3);
        }
        self.cg.code[ofs] = kind;
        self.cg.write4_at(ofs + 1, bytes);
    }

    fn gen_goto(&mut self, name: &[u8]) {
        let Some(label) = self.find_label(name) else {
            self.error(format!(
                "invalid \"goto\" - label \"{}\" is not defined in this function or method",
                show_name(name)
            ));
            return;
        };
        // Leave each statement around the `goto`, from the nearest
        // outward, until one that the label is inside as well.
        let within = self.labels[label].within.clone();
        let mut shared = None;
        for index in self.leaving() {
            let id = self.enclosing[index].id();
            if let Some(found) = within.iter().position(|enclosure| enclosure.id == id) {
                shared = Some(found);
                break;
            }
            if let Enclosing::Try {
                finally_lbl,
                finally_end,
                ..
            } = self.enclosing[index]
            {
                self.gen_jsr_finally(finally_lbl, finally_end);
            }
        }
        // The jump goes into everything around the label from there
        // inward, and a `finally` block has no way to be left after
        // being entered like that.
        let entered = match shared {
            Some(found) => &within[found + 1..],
            None => &within[..],
        };
        if entered.iter().any(|enclosure| enclosure.is_finally) {
            self.error("'goto' cannot transfer control into a 'finally' block");
        }
        let target = self.goto_label(label);
        self.cg.write_jump(Op::Jmp, target);
    }

    fn gen_try(
        &mut self,
        stmt: &Stmt,
        id: usize,
        body: Option<&Stmt>,
        catches: &[Catch],
        finally: Option<&Finally>,
    ) {
        let finally_lbl = finally.map(|_| self.cg.new_label());
        let finally_end = finally.and_then(|finally| finally.end);
        self.enclosing.push(Enclosing::Try {
            id,
            finally_lbl,
            finally_end,
        });
        let start_ofs = self.rel_ofs();
        if let Some(body) = body {
            self.gen_substm(body, stmt);
        }
        // If the `finally` block never finishes, nothing that calls it
        // gets control back.
        let finally_never_returns = finally.is_some_and(|finally| {
            finally
                .body
                .as_deref()
                .is_some_and(|body| self.flow_of(body) & flow::NEXT == 0)
        });
        self.gen_jsr_finally(finally_lbl, finally_end);
        let mut end_lbl = None;
        if !finally_never_returns {
            end_lbl = self.gen_jump_ahead(Op::Jmp);
        }
        // The protected code is everything up to the byte before here.
        let end_ofs = self.rel_ofs() - 1;
        for (index, catch) in catches.iter().enumerate() {
            self.gen_catch(stmt, catch, start_ofs, end_ofs);
            self.gen_jsr_finally(finally_lbl, finally_end);
            if !finally_never_returns && (finally.is_some() || index + 1 < catches.len()) {
                match end_lbl {
                    None => end_lbl = self.gen_jump_ahead(Op::Jmp),
                    Some(end_lbl) => self.cg.write_jump(Op::Jmp, end_lbl),
                }
            }
        }
        // The `finally` block is not inside the `try` it belongs to: it
        // is not called again as it is left.
        self.enclosing.pop();
        if let (Some(finally), Some(finally_lbl)) = (finally, finally_lbl) {
            let end_ofs = self.rel_ofs() - 1;
            self.add_line_rec(finally.file, finally.line);
            self.enclosing.push(Enclosing::Finally { id: finally.id });
            // First, what happens when an exception arrives: keep it,
            // run the block, and throw it again.
            self.exc_table.push(ExcEntry {
                start_ofs,
                end_ofs,
                class: 0,
                handler_ofs: self.rel_ofs(),
            });
            self.cg.clear_peephole();
            self.cg.note_push();
            self.gen_setlcl_stk(finally.exc_local, false);
            self.gen_jsr_finally(Some(finally_lbl), finally_end);
            self.gen_getlcl(finally.exc_local, false);
            self.cg.write_op(Op::Throw);
            self.cg.note_pop();
            self.add_line_rec(finally.file, finally.line);
            // Then the block itself. It is entered with where to go
            // back to on the stack.
            self.cg.def_label(finally_lbl);
            self.cg.note_push();
            self.gen_setlcl_stk(finally.jsr_local, false);
            if let Some(body) = &finally.body {
                self.gen_substm_at(body, finally.file, finally.line);
            }
            self.cg.write_op(Op::LRet);
            self.cg.write2(finally.jsr_local as u16);
            self.enclosing.pop();
        }
        self.def_label_pos(end_lbl);
    }

    fn gen_catch(&mut self, stmt: &Stmt, catch: &Catch, start_ofs: usize, end_ofs: usize) {
        self.add_line_rec(catch.file, catch.line);
        let old_frame = self.set_local_frame(catch.symtab);
        // The class of exception caught has to be an object.
        let mut class = 0;
        if let Some(Global::Obj(obj)) = self.prog.globals.get(&catch.class) {
            class = self.prog.objs[*obj].id;
        } else if self.prog.globals.contains_key(&catch.class) {
            self.out.diagnostics.push(GenDiagnostic {
                file: catch.file,
                line: catch.line,
                severity: Severity::Error,
                message: format!(
                    "exception class symbol \"{}\" in \"catch\" clause is not an object",
                    show_name(&catch.class)
                ),
            });
            self.errors += 1;
        } else {
            self.undefined(&catch.class);
        }
        self.exc_table.push(ExcEntry {
            start_ofs,
            end_ofs,
            class,
            handler_ofs: self.rel_ofs(),
        });
        self.cg.clear_peephole();
        // The exception is on the stack on arrival.
        self.cg.note_push();
        self.gen_setlcl(catch.var);
        if let Some(body) = &catch.body {
            self.gen_substm(body, stmt);
        }
        self.set_local_frame(old_frame);
    }

    fn gen_if(
        &mut self,
        stmt: &Stmt,
        cond: &Expr,
        then_part: Option<&Stmt>,
        else_part: Option<&Stmt>,
    ) {
        // With a constant condition, only one part is generated.
        if let Some(value) = cond.as_const() {
            if value.is_true() {
                if else_part.is_some() {
                    self.log(
                        Severity::Warning,
                        "\"if\" condition is always true - \"else\" part is unreachable",
                    );
                }
                if let Some(part) = then_part {
                    self.gen_substm(part, stmt);
                }
            } else {
                if !value.ctc {
                    self.log(Severity::Warning, "\"if\" condition is always false");
                }
                if let Some(part) = else_part {
                    self.gen_substm(part, stmt);
                }
            }
            return;
        }
        match (then_part, else_part) {
            // Nothing depends on the condition; it is evaluated for
            // whatever else it does.
            (None, None) => self.gen_expr(cond, true, true),
            (Some(then_part), else_part) => {
                let lbl_else = self.cg.new_label();
                self.gen_cond(cond, None, Some(lbl_else));
                self.gen_substm(then_part, stmt);
                if let Some(else_part) = else_part {
                    let lbl_end = self.gen_jump_ahead(Op::Jmp);
                    self.cg.def_label(lbl_else);
                    self.gen_substm(else_part, stmt);
                    self.def_label_pos(lbl_end);
                } else {
                    self.cg.def_label(lbl_else);
                }
            }
            (None, Some(else_part)) => {
                let lbl_end = self.cg.new_label();
                self.gen_cond(cond, Some(lbl_end), None);
                self.gen_substm(else_part, stmt);
                self.cg.def_label(lbl_end);
            }
        }
    }

    fn gen_return(&mut self, expr: Option<&Expr>) {
        // Returning nil or true has an instruction of its own; any
        // other value is worked out first.
        let constant = expr.and_then(|expr| expr.as_const().map(|value| &value.value));
        let need_gen = expr.is_some() && !matches!(constant, Some(Value::Nil | Value::True));
        let mut val_on_stack = false;
        if self.in_constructor && expr.is_some() {
            self.error("\"construct\" cannot return a value");
        }
        if let (true, Some(expr)) = (need_gen, expr) {
            let depth = self.cg.sp_depth();
            self.gen_expr(expr, false, false);
            val_on_stack = self.cg.sp_depth() > depth;
        }

        // Then the `finally` blocks around the statement run, with the
        // value put aside in a local meanwhile.
        if !self.enclosing.is_empty() {
            let finally_lbls: Vec<(Label, Option<(usize, u32)>)> = self
                .leaving()
                .into_iter()
                .filter_map(|index| match self.enclosing[index] {
                    Enclosing::Try {
                        finally_lbl,
                        finally_end,
                        ..
                    } => finally_lbl.map(|label| (label, finally_end)),
                    _ => None,
                })
                .collect();
            let saved = (val_on_stack && !finally_lbls.is_empty()).then(|| {
                *self.fin_ret_lcl.get_or_insert_with(|| {
                    self.local_cnt += 1;
                    self.local_cnt - 1
                })
            });
            if let Some(local) = saved {
                self.gen_setlcl_stk(local, false);
            }
            for (label, end) in finally_lbls {
                self.gen_jsr_finally(Some(label), end);
            }
            if let Some(local) = saved {
                self.gen_getlcl(local, false);
            }
        }

        if self.in_constructor {
            // A constructor returns what it constructed.
            self.cg.write_op(Op::PushSelf);
            self.cg.write_op(Op::RetVal);
            return;
        }
        match constant {
            _ if expr.is_none() => self.cg.write_op(Op::RetNil),
            Some(Value::Nil) => self.cg.write_op(Op::RetNil),
            Some(Value::True) => self.cg.write_op(Op::RetTrue),
            _ if val_on_stack => {
                self.cg.write_op(Op::RetVal);
                self.cg.note_pop();
            }
            // The expression left nothing to return.
            _ => self.cg.write_op(Op::RetNil),
        }
    }

    // ---- expressions ----

    /// Generate a test of a condition: a jump to `then_label` if it is
    /// true, or else a jump to `else_label` if it is false. Exactly one
    /// of the two is given; the other case falls through.
    fn gen_cond(&mut self, expr: &Expr, then_label: Option<Label>, else_label: Option<Label>) {
        match expr {
            Expr::Binary(BinOp::Or, left, right) => {
                // If the left side is true, the whole is.
                let internal_then = then_label.unwrap_or_else(|| self.cg.new_label());
                self.gen_cond(left, Some(internal_then), None);
                self.gen_cond(right, then_label, else_label);
                if then_label.is_none() {
                    self.cg.def_label(internal_then);
                }
            }
            Expr::Binary(BinOp::And, left, right) => {
                let internal_else = else_label.unwrap_or_else(|| self.cg.new_label());
                self.gen_cond(left, None, Some(internal_else));
                self.gen_cond(right, then_label, else_label);
                if else_label.is_none() {
                    self.cg.def_label(internal_else);
                }
            }
            _ => {
                self.gen_expr(expr, false, true);
                match (then_label, else_label) {
                    (Some(label), _) => self.cg.write_jump(Op::Jt, label),
                    (None, Some(label)) => self.cg.write_jump(Op::Jf, label),
                    (None, None) => unreachable!("a condition needs somewhere to go"),
                }
                self.cg.note_pop();
            }
        }
    }

    fn gen_int(&mut self, value: i32) {
        if value == 0 {
            self.cg.write_op(Op::Push0);
        } else if value == 1 {
            self.cg.write_op(Op::Push1);
        } else if (-128..127).contains(&value) {
            self.cg.write_op(Op::PushInt8);
            self.cg.write(value as u8);
        } else {
            self.cg.write_op(Op::PushInt);
            self.cg.write4(value as u32);
        }
        self.cg.note_push();
    }

    /// Record that four bytes are to hold an address: in the code, or
    /// in the constants.
    fn add_fixup(&mut self, in_consts: bool, ofs: usize, target: Target) {
        let fixup = Fixup { ofs, target };
        if in_consts {
            self.out.const_fixups.push(fixup);
        } else {
            self.out.fixups.push(fixup);
        }
    }

    /// Put a string in the constants, unless it is a short one that is
    /// there already, and note where its address is wanted.
    fn add_const_str(&mut self, text: &[u8], in_consts: bool, ofs: usize) {
        let known = (text.len() < INTERN_THRESHOLD)
            .then(|| self.interned.get(text).copied())
            .flatten();
        let anchor = known.unwrap_or_else(|| {
            let anchor = self.out.const_anchors.len();
            self.out.const_anchors.push(Anchor {
                ofs: self.out.consts.len(),
            });
            if text.len() < INTERN_THRESHOLD {
                self.interned.insert(text.to_vec(), anchor);
            }
            self.out
                .consts
                .extend_from_slice(&(text.len() as u16).to_le_bytes());
            self.out.consts.extend_from_slice(text);
            self.out.max_str_len = self.out.max_str_len.max(text.len());
            anchor
        });
        self.add_fixup(in_consts, ofs, Target::Const(anchor));
    }

    /// Put a list in the constants, and note where its address is
    /// wanted. The list is a count and then five bytes for each
    /// element; the strings and lists among the elements follow it.
    fn add_const_list(&mut self, elements: &[ConstVal], in_consts: bool, ofs: usize) {
        let anchor = self.out.const_anchors.len();
        let start = self.out.consts.len();
        self.out.const_anchors.push(Anchor { ofs: start });
        self.add_fixup(in_consts, ofs, Target::Const(anchor));
        self.out
            .consts
            .extend_from_slice(&(elements.len() as u16).to_le_bytes());
        self.out.consts.resize(start + 2 + 5 * elements.len(), 0);
        for (index, element) in elements.iter().enumerate() {
            let at = start + 2 + 5 * index;
            let (kind, value) = match &element.value {
                Value::Nil => (datatype::NIL, 0),
                Value::True => (datatype::TRUE, 0),
                Value::Int(int) => (datatype::INT, *int as u32),
                Value::SStr(text) => {
                    self.add_const_str(text, true, at + 1);
                    (datatype::SSTRING, 0)
                }
                Value::List(inner) => {
                    self.add_const_list(inner, true, at + 1);
                    (datatype::LIST, 0)
                }
                Value::FuncPtr(func) => {
                    self.add_fixup(true, at + 1, Target::Func(*func));
                    (datatype::FUNCPTR, 0)
                }
                Value::Obj(obj) => {
                    self.out.const_id_fixups.push((at + 1, IdKind::Obj));
                    (datatype::OBJ, *obj)
                }
                Value::Float { .. } | Value::ReStr(_) => {
                    let id = self.gen_const_obj(&element.value).expect("an object");
                    self.out.const_id_fixups.push((at + 1, IdKind::Obj));
                    (datatype::OBJ, id)
                }
                Value::Enum(id) => {
                    self.out.const_id_fixups.push((at + 1, IdKind::Enum));
                    (datatype::ENUM, *id)
                }
                Value::Prop(prop) => {
                    self.out.const_id_fixups.push((at + 1, IdKind::Prop));
                    self.out.const_unspecified2.push(at + 3);
                    (datatype::PROP, u32::from(*prop))
                }
            };
            if matches!(element.value, Value::Nil | Value::True) {
                // The reference writes the kind and leaves the rest as
                // it found it.
                self.out.const_unspecified.push(at + 1);
            }
            self.out.consts[at] = kind;
            self.out.consts[at + 1..at + 5].copy_from_slice(&value.to_le_bytes());
        }
        self.out.max_list_cnt = self.out.max_list_cnt.max(elements.len());
    }

    fn gen_str(&mut self, text: &[u8]) {
        // Code compiled while the program runs has its strings in itself.
        if self.prog.dynamic.is_some() {
            self.gen_str_inline(text);
            return;
        }
        self.cg.write_op(Op::PushStr);
        self.cg.note_push();
        self.add_const_str(text, false, self.cg.ofs());
        self.cg.write4(0);
    }

    /// Push a string written into the code (`pushstri`).
    fn gen_str_inline(&mut self, text: &[u8]) {
        self.cg.write_op(Op::PushStrI);
        self.cg.write2(text.len() as u16);
        for &byte in text {
            self.cg.write(byte);
        }
        self.cg.note_push();
    }

    /// Make an object of a class built into the VM from a string written
    /// into the code, as code compiled while the program runs makes its
    /// numbers and patterns.
    fn gen_dynamic_from_text(&mut self, class: &[u8], text: &[u8]) {
        let meta = match self.prog.globals.get(class) {
            Some(Global::Meta(meta)) => Some(self.prog.metas[*meta].meta),
            _ => None,
        };
        let Some(meta) = meta else {
            self.error("This expression cannot be evaluated while the program runs");
            return;
        };
        self.gen_str_inline(text);
        self.cg.write_op(Op::New2);
        self.cg.write2(1);
        self.cg.write2(meta as u16);
        self.cg.note_pop();
        self.cg.write_op(Op::GetR0);
        self.cg.note_push();
    }

    fn gen_func_ref(&mut self, func: usize) {
        // A function of the program that is running is where it is.
        if let Some(&addr) = self
            .prog
            .dynamic
            .as_ref()
            .and_then(|dynamic| dynamic.func_addrs.get(&func))
        {
            self.cg.write4(addr);
            return;
        }
        self.out.fixups.push(Fixup {
            ofs: self.cg.ofs(),
            target: Target::Func(func),
        });
        self.cg.write4(0);
    }

    /// What `replaced` means where code is being generated: whether
    /// it means anything, which body it is if that is known, and how
    /// many arguments the function that has it takes. It means the
    /// same in a nested function as in the function around it.
    fn replaced_func(&self) -> (bool, Option<usize>, (usize, usize, bool)) {
        const NONE: (bool, Option<usize>, (usize, usize, bool)) = (false, None, (0, 0, false));
        let Some(closure) = self.func_closure() else {
            return NONE;
        };
        // A body its function's name has not been taken from goes by
        // what the function modifies now, which a later `replace` or
        // `modify` may have changed.
        if let Some(func) = closure.func_id {
            let func = &self.prog.funcs[func];
            let base = func.mod_bases.first().copied();
            return (base.is_some() || func.mod_extern, base, func.mod_proto);
        }
        (
            closure.has_replaced,
            closure.replaced_base,
            closure.func_proto,
        )
    }

    /// What the function the code being generated is in shares with
    /// the bodies in and around it: the function's own, however deep
    /// in it the code is.
    fn func_closure(&self) -> Option<&Closure> {
        let mut closure = self.closures.get(self.cur_body)?;
        if !closure.is_func {
            let mut next = closure.enclosing;
            loop {
                let body_ref = &self.prog.body_refs[next?];
                if let Some(body) = body_ref.func_body {
                    closure = &self.closures[body];
                    break;
                }
                next = body_ref.enclosing;
            }
        }
        Some(closure)
    }

    /// Where `replaced` goes: the body of the definition that `modify`
    /// put this one in front of, or, if that is in another source
    /// file, the function as it was before this file changed it.
    fn replaced_target(&self, base: Option<usize>) -> Option<Target> {
        if let Some(base) = base {
            return Some(Target::Body(base));
        }
        let closure = self.func_closure()?;
        match closure.func_id {
            Some(func) => self.prog.funcs[func]
                .mod_extern
                .then_some(Target::FuncBase(func)),
            None => closure.replaced_ext.map(Target::FuncBase),
        }
    }

    fn gen_expr(&mut self, expr: &Expr, discard: bool, for_condition: bool) {
        match expr {
            // The address of the function as it was before `modify`.
            Expr::Replaced => {
                let (available, base, _) = self.replaced_func();
                if !available {
                    self.error("'replaced' is not available in this context");
                }
                self.cg.write_op(Op::PushFnPtr);
                if let Some(target) = self.replaced_target(base) {
                    self.out.fixups.push(Fixup {
                        ofs: self.cg.ofs(),
                        target,
                    });
                }
                self.cg.write4(0);
                self.cg.note_push();
                let _ = (discard, for_condition);
            }
            Expr::Const(value) => {
                if discard {
                    return;
                }
                if self.prog.dynamic.is_some() {
                    // Code compiled while the program runs makes its
                    // lists as it runs, and its numbers and patterns.
                    let elements = value.list_node.as_ref().or(match &value.value {
                        Value::List(elements) => Some(elements),
                        _ => None,
                    });
                    if let Some(elements) = elements {
                        let list = Expr::List(elements.iter().cloned().map(Expr::Const).collect());
                        self.gen_expr(&list, false, false);
                        return;
                    }
                    match &value.value {
                        Value::Float { text, .. } => {
                            self.gen_dynamic_from_text(b"BigNumber", text);
                            return;
                        }
                        Value::ReStr(text) => {
                            self.gen_dynamic_from_text(b"RexPattern", text);
                            return;
                        }
                        _ => {}
                    }
                }
                if let Some(elements) = &value.list_node {
                    // A list whose value was overwritten is still
                    // generated as the list.
                    self.cg.write_op(Op::PushLst);
                    self.add_const_list(elements, false, self.cg.ofs());
                    self.cg.write4(0);
                    self.cg.note_push();
                    return;
                }
                match &value.value {
                    Value::Nil => self.cg.write_op(Op::PushNil),
                    Value::True => self.cg.write_op(Op::PushTrue),
                    Value::Int(int) => {
                        self.gen_int(*int);
                        self.cg.note_pop();
                    }
                    Value::SStr(text) => {
                        self.gen_str(text);
                        self.cg.note_pop();
                    }
                    Value::FuncPtr(func) => {
                        self.cg.write_op(Op::PushFnPtr);
                        self.gen_func_ref(*func);
                    }
                    Value::List(elements) => {
                        self.cg.write_op(Op::PushLst);
                        self.add_const_list(elements, false, self.cg.ofs());
                        self.cg.write4(0);
                    }
                    Value::Obj(obj) => {
                        self.cg.write_op(Op::PushObj);
                        self.write_obj_id(*obj);
                    }
                    Value::Float { .. } | Value::ReStr(_) => {
                        self.cg.write_op(Op::PushObj);
                        let id = self.gen_const_obj(&value.value).expect("an object");
                        self.write_obj_id(id);
                    }
                    Value::Enum(id) => {
                        self.cg.write_op(Op::PushEnum);
                        self.write_enum_id(*id);
                    }
                    Value::Prop(prop) => {
                        self.cg.write_op(Op::PushPropId);
                        self.write_prop_id(*prop);
                    }
                }
                self.cg.note_push();
            }
            Expr::Addr(sub) => {
                if !discard {
                    self.gen_addr(sub);
                }
            }
            Expr::New(sub, transient) => {
                self.gen_new(sub, *transient, discard, CallArgs::default(), false)
            }
            Expr::AnonFunc { body, .. } => {
                if discard {
                    return;
                }
                // The contexts the function needs, last first: each is
                // one this function has, from one function less far
                // out. Then the code; and if there are contexts, or the
                // function makes one of its own, an object of the two.
                let levels = self.closures[*body].ctx_levels.clone();
                let mut argc = 1;
                for &(level, _) in levels.iter().rev() {
                    let cur = &self.closures[self.cur_body];
                    let var = if level == 1 {
                        cur.local_ctx_var
                    } else {
                        cur.ctx_levels
                            .iter()
                            .find(|&&(have, _)| have == level - 1)
                            .map_or(0, |&(_, var)| var)
                    };
                    self.gen_getlcl(var, false);
                    argc += 1;
                }
                // Compiled while the program runs, the function is an
                // object of its own.
                let dyn_id = self
                    .prog
                    .dynamic
                    .as_ref()
                    .and_then(|dynamic| dynamic.body_ids.get(body).copied());
                if let Some(id) = dyn_id {
                    self.cg.write_op(Op::PushObj);
                    self.write_obj_id(id);
                } else {
                    self.cg.write_op(Op::PushFnPtr);
                    self.out.fixups.push(Fixup {
                        ofs: self.cg.ofs(),
                        target: Target::Body(*body),
                    });
                    self.cg.write4(0);
                }
                self.cg.note_push();
                if argc > 1 || self.closures[*body].has_local_ctx {
                    if argc <= 255 {
                        self.cg.write_op(Op::New1);
                        self.cg.write(argc as u8);
                        self.cg.write(METACLASS_ANON_FN);
                    } else {
                        self.cg.write_op(Op::New2);
                        self.cg.write2(argc as u16);
                        self.cg.write2(u16::from(METACLASS_ANON_FN));
                    }
                    self.cg.write_op(Op::GetR0);
                    self.cg.note_pops(argc - 1);
                }
            }
            Expr::Inh | Expr::InhClass(_) | Expr::InhTypes(..) => {
                self.error("\"inherited\" cannot be generated on its own");
            }
            Expr::Delegated(_) => {
                self.error("attempting to generate 'delegated' node directly");
            }
            Expr::ArgCount => {
                if !discard {
                    self.cg.write_op(Op::GetArgc);
                    self.cg.note_push();
                }
            }
            Expr::InlineObj {
                superclasses,
                props,
            } => self.gen_inline_obj(superclasses, props),
            Expr::CtxEle(which) => {
                if !self.self_available {
                    match which {
                        1 => self.error("'targetprop' is not available in this context"),
                        2 => self.error("'targetobj' is not available in this context"),
                        3 => self.error("'definingobj' is not available in this context"),
                        _ => {}
                    }
                }
                if !discard {
                    self.cg.write_op(Op::PushCtxEle);
                    self.cg.write(*which);
                    self.cg.note_push();
                }
            }
            Expr::SelfRef => {
                self.need_self();
                if !discard {
                    self.cg.write_op(Op::PushSelf);
                    self.cg.note_push();
                }
            }
            Expr::IsIn {
                left,
                args,
                not,
                hit,
            } => {
                // The left side is compared with each in turn. If the
                // answer is known already, there is only the one way
                // out.
                let lbl_found = self.cg.new_label();
                let lbl_done = (!hit && !discard).then(|| self.cg.new_label());
                self.gen_expr(left, false, false);
                for arg in args {
                    self.cg.write_op(Op::Dup);
                    self.gen_expr(arg, false, false);
                    self.cg.write_jump(Op::Je, lbl_found);
                    self.cg.note_push();
                    self.cg.note_pops(2);
                }
                if let Some(lbl_done) = lbl_done {
                    self.cg.write_op(Op::Disc);
                    self.cg
                        .write_op(if *not { Op::PushTrue } else { Op::PushNil });
                    self.cg.write_jump(Op::Jmp, lbl_done);
                }
                self.cg.def_label(lbl_found);
                self.cg.write_op(Op::Disc);
                if discard {
                    self.cg.note_pop();
                } else {
                    self.cg
                        .write_op(if *not { Op::PushNil } else { Op::PushTrue });
                }
                if let Some(lbl_done) = lbl_done {
                    self.cg.def_label(lbl_done);
                }
            }
            Expr::ForIn(for_in) => match &**for_in {
                ForIn::Coll {
                    expr, iter_local, ..
                } => {
                    let local = self.prog.locals[*iter_local].var_num;
                    self.gen_iter_init(expr, local, "for");
                }
                ForIn::Range {
                    lval,
                    from,
                    to,
                    step,
                    to_local,
                    step_local,
                } => {
                    self.gen_asi(lval, true, 1, AsiOp::Simple, Some(from), false, true);
                    if let (false, Some(local)) = (to.is_const(), to_local) {
                        self.gen_expr(to, false, false);
                        self.gen_setlcl_stk(self.prog.locals[*local].var_num, false);
                    }
                    if let (Some(step), Some(local)) = (step, step_local) {
                        if !step.is_const() {
                            self.gen_expr(step, false, false);
                            self.gen_setlcl_stk(self.prog.locals[*local].var_num, false);
                        }
                    }
                }
            },
            Expr::Member {
                obj,
                prop,
                prop_is_expr,
                args,
            } => {
                let args = match args {
                    Some(args) => self.gen_arglist(args),
                    None => CallArgs::default(),
                };
                self.gen_member(obj, prop, *prop_is_expr, args, discard);
            }
            // A built-in function named without parentheses is called
            // with no arguments.
            Expr::Sym(name) => match self.implied_global(name) {
                Some(Global::DynLocal(local)) => self.gen_dyn_local(local, discard),
                Some(Global::Bif(bif)) => self.gen_bif_call(bif, discard, 0, false, None),
                Some(Global::Prop(prop)) => self.gen_prop_self(prop, discard),
                // A built-in class: the object that stands for it,
                // which the reference pushes wanted or not.
                Some(Global::Meta(meta)) => {
                    self.cg.write_op(Op::PushObj);
                    self.write_obj_id(self.prog.metas[meta].obj_id);
                    self.cg.note_push();
                }
                // A dictionary.
                Some(Global::Obj(obj)) if !discard => {
                    self.cg.write_op(Op::PushObj);
                    self.write_obj_id(self.prog.objs[obj].id);
                    self.cg.note_push();
                }
                _ => {}
            },
            Expr::DStr(text) => {
                if !discard {
                    self.error(format!(
                        "a double-quoted string (\"{}\") is not valid within an expression",
                        show_name(text)
                    ));
                }
                // Code compiled while the program runs displays a
                // string written into it.
                if self.prog.dynamic.is_some() {
                    self.gen_str_inline(text);
                    self.cg.write_op(Op::SayVal);
                    self.cg.note_pop();
                    return;
                }
                self.cg.write_op(Op::Say);
                self.add_const_str(text, false, self.cg.ofs());
                self.cg.write4(0);
            }
            Expr::DStrEmbed(sub) => {
                // Whatever value the expression leaves is displayed. It
                // need not leave one: a call to a function that returns
                // nothing is there for what it does.
                let depth = self.cg.sp_depth();
                self.gen_expr(sub, !self.has_return_value(sub), false);
                if self.cg.sp_depth() > depth {
                    self.cg.write_op(Op::SayVal);
                    self.cg.note_pop();
                }
            }
            Expr::OneOf(_, elements, state) => {
                self.gen_one_of(elements, *state, discard, for_condition)
            }
            Expr::List(elements) => {
                // The elements, last first, and then a list made of
                // them.
                for element in elements.iter().rev() {
                    self.gen_expr(element, false, false);
                }
                if elements.len() <= 255 {
                    self.cg.write_op(Op::New1);
                    self.cg.write(elements.len() as u8);
                    self.cg.write(METACLASS_LIST);
                } else {
                    self.cg.write_op(Op::New2);
                    self.cg.write2(elements.len() as u16);
                    self.cg.write2(u16::from(METACLASS_LIST));
                }
                self.cg.note_pops(elements.len() as i32);
                if !discard {
                    self.cg.write_op(Op::GetR0);
                    self.cg.note_push();
                }
            }
            Expr::Lookup(list) => {
                // The list of keys and values, and then, if the table
                // is wanted, a table made of it.
                self.gen_expr(list, discard, false);
                if !discard {
                    self.cg.write_op(Op::New1);
                    self.cg.write(1);
                    self.cg.write(METACLASS_LOOKUP_TABLE);
                    self.cg.note_pop();
                    self.cg.write_op(Op::GetR0);
                    self.cg.note_push();
                }
            }
            Expr::Index(left, index) => self.gen_binary(Op::Index, left, index, discard),
            Expr::Local(id) => self.gen_local(*id, discard),
            Expr::Unary(op, sub) => match op {
                UnOp::Not => self.gen_unary(Op::Not, sub, discard, true),
                UnOp::Boolize => {
                    if for_condition {
                        // Whatever tests the value does the same
                        // conversion.
                        self.gen_expr(sub, discard, for_condition);
                    } else if !sub.is_bool() {
                        self.gen_unary(Op::Boolize, sub, discard, true);
                    }
                }
                UnOp::BNot => self.gen_unary(Op::BNot, sub, discard, false),
                UnOp::Pos => self.gen_expr(sub, discard, false),
                UnOp::Neg => self.gen_unary(Op::Neg, sub, discard, false),
            },
            Expr::Binary(op, left, right) => {
                self.gen_binary_expr(*op, left, right, discard, for_condition)
            }
            Expr::Assign(AsiOp::Simple, left, right) => {
                self.gen_asi(left, discard, 1, AsiOp::Simple, Some(right), false, true);
            }
            Expr::Assign(op, left, right) => {
                if !self.gen_asi(left, discard, 1, *op, Some(right), false, true) {
                    // Nothing special for this target: work out the new
                    // value, then store it.
                    let opc = match op {
                        AsiOp::Add => Op::Add,
                        AsiOp::Sub => Op::Sub,
                        AsiOp::Mul => Op::Mul,
                        AsiOp::Div => Op::Div,
                        AsiOp::Mod => Op::Mod,
                        AsiOp::BAnd => Op::BAnd,
                        AsiOp::BOr => Op::BOr,
                        AsiOp::BXor => Op::Xor,
                        AsiOp::Shl => Op::Shl,
                        AsiOp::AShr => Op::AShr,
                        _ => Op::LShr,
                    };
                    self.gen_expr(right, false, false);
                    self.cg.write_op(opc);
                    self.cg.note_pop();
                    self.gen_asi(left, discard, 2, *op, None, false, true);
                }
            }
            Expr::IncDec(op, sub) => {
                let (opc, post) = match op {
                    AsiOp::PreInc => (Op::Inc, false),
                    AsiOp::PreDec => (Op::Dec, false),
                    AsiOp::PostInc => (Op::Inc, true),
                    _ => (Op::Dec, true),
                };
                if !self.gen_asi(sub, discard, 1, *op, None, false, true) {
                    if post {
                        // The result is the value from before.
                        if !discard {
                            self.cg.write_op(Op::Dup);
                            self.cg.note_push();
                        }
                        self.cg.write_op(opc);
                        self.gen_asi(sub, true, 2, *op, None, false, true);
                    } else {
                        self.cg.write_op(opc);
                        self.gen_asi(sub, discard, 2, *op, None, false, true);
                    }
                }
            }
            Expr::If(first, second, third) => {
                self.gen_expr(first, false, true);
                let lbl_else = self.gen_jump_ahead(Op::Jf);
                self.cg.note_pop();
                // A branch that has no value still has to leave one if
                // the other does.
                let second_val = self.has_return_value(second);
                let third_val = self.has_return_value(third);
                self.gen_expr(second, discard || !second_val, for_condition);
                if !discard && !second_val {
                    self.cg.write_op(Op::PushNil);
                    self.cg.note_push();
                }
                let lbl_end = self.gen_jump_ahead(Op::Jmp);
                self.def_label_pos(lbl_else);
                self.gen_expr(third, discard || !third_val, for_condition);
                if !discard && !third_val {
                    self.cg.write_op(Op::PushNil);
                    self.cg.note_push();
                }
                // Only one branch runs, so only one value is left.
                if second_val || third_val {
                    self.cg.note_pop();
                }
                self.def_label_pos(lbl_end);
            }
            Expr::IfNil(first, second) => {
                self.gen_expr(first, false, false);
                if !discard {
                    self.cg.write_op(Op::Dup);
                    self.cg.note_push();
                }
                let lbl_end = self.gen_jump_ahead(Op::JNotNil);
                self.cg.note_pop();
                if !discard {
                    self.cg.write_op(Op::Disc);
                    self.cg.note_pop();
                }
                self.gen_expr(second, discard, for_condition);
                self.def_label_pos(lbl_end);
            }
            Expr::Call(func, args)
                if self.is_rand(func)
                    && args.len() >= 2
                    && args.iter().all(|arg| !arg.varargs && arg.name.is_none()) =>
            {
                self.gen_rand(func, args, discard, for_condition);
            }
            Expr::Call(func, args) => {
                let CallArgs {
                    argc,
                    varargs,
                    named,
                } = self.gen_arglist(args);
                self.gen_call(func, discard, argc, varargs, named);
            }
        }
    }

    /// Whether what a call calls is `rand` by name: the built-in
    /// function, or a local of that name. (A function the program
    /// defines has become a constant by now, and is not.)
    fn is_rand(&self, func: &Expr) -> bool {
        match func {
            Expr::Sym(name) => name == b"rand",
            Expr::Local(id) => self.prog.locals[*id].name == b"rand",
            _ => false,
        }
    }

    /// Generate `rand(a, b, ...)` with two or more arguments: the
    /// reference evaluates only the one chosen. It calls `rand` with
    /// the number of arguments, and switches on what comes back, from
    /// 0 for the first; the cases are written last first, as the
    /// reference keeps its arguments.
    fn gen_rand(&mut self, func: &Expr, args: &[Arg], discard: bool, for_condition: bool) {
        let count = args.len();
        self.gen_int(count as i32);
        self.gen_call(func, false, 1, false, None);
        self.cg.write_op(Op::Switch);
        self.cg.write2(count as u16);
        self.cg.note_pop();
        let table = self.cg.ofs();
        for index in (0..count).rev() {
            self.cg.write(datatype::INT);
            self.cg.write4(index as u32);
            self.cg.write2(0);
        }
        let default_slot = self.cg.ofs();
        self.cg.write2(0);
        let end_lbl = self.cg.new_label();
        for (place, arg) in args.iter().rev().enumerate() {
            let slot = table + 7 * place + 5;
            let jump = self.cg.ofs() - slot;
            self.cg.write2_at(slot, jump as u16);
            self.gen_expr(&arg.expr, discard, for_condition);
            self.cg.write_jump(Op::Jmp, end_lbl);
            if !discard {
                self.cg.note_pop();
            }
        }
        let jump = self.cg.ofs() - default_slot;
        self.cg.write2_at(default_slot, jump as u16);
        if !discard {
            self.cg.write_op(Op::PushNil);
            self.cg.note_push();
        }
        self.cg.def_label(end_lbl);
    }

    /// Whether evaluating an expression leaves a value. A call to a
    /// function that returns nothing does not; but by now the name of a
    /// function has become a constant, and the reference takes a call
    /// through a constant to leave a value whatever the function does.
    /// So only a call by a name that was never defined is looked up.
    fn has_return_value(&self, expr: &Expr) -> bool {
        self.prog.has_return_value(expr, self.in_defval)
    }

    /// Whether the outermost of the bodies a body is nested in keeps
    /// all of what a method knows of its call in its context, not just
    /// `self`.
    fn outermost_needs_full(&self, closure: &Closure) -> bool {
        let mut outermost = None;
        let mut next = closure.enclosing;
        while let Some(body) = next {
            outermost = Some(body);
            next = self.prog.body_refs[body].enclosing;
        }
        outermost.is_some_and(|body| self.prog.body_refs[body].local_ctx_needs_full)
    }

    // ---- loops over collections and ranges ----

    /// Make an iterator for a collection, and keep it in a local. How
    /// is the first thing the built-in class `collection` is declared
    /// to do.
    fn gen_iter_init(&mut self, coll: &Expr, local: usize, keyword: &str) {
        let collection = self
            .prog
            .meta_table
            .iter()
            .position(|entry| entry.name.starts_with(b"collection/"));
        // Code compiled while the program runs takes the property from
        // the classes the program has.
        let create_iter = match &self.prog.dynamic {
            Some(dynamic) => collection
                .and_then(|index| dynamic.meta_props.get(index))
                .and_then(|props| props.first().copied())
                .filter(|&prop| prop != 0),
            None => collection
                .and_then(|index| self.prog.meta_table[index].sym)
                .and_then(|sym| self.prog.metas[sym].props.first().copied())
                .map(|prop| self.prog.props[prop].id),
        };
        let Some(prop) = create_iter else {
            self.error(format!(
                "Container.createIterator is not defined - {keyword}..in is not valid"
            ));
            return;
        };
        let prop = Expr::constant(Value::Prop(prop));
        self.gen_member(coll, &prop, false, CallArgs::default(), false);
        self.gen_setlcl_stk(local, false);
    }

    /// Take the next value from an iterator, or leave the loop if there
    /// is none.
    fn gen_iter_cond(&mut self, lval: &Expr, local: usize, end_lbl: Label) {
        self.cg.write_op(Op::IterNext);
        self.cg.write2(local as u16);
        self.cg.write_label_ofs(end_lbl);
        self.cg.note_push();
        self.gen_asi(lval, true, 1, AsiOp::Simple, None, false, true);
    }

    /// What an `in` clause of a `for` does at the top of each turn.
    fn gen_for_in_cond(&mut self, for_in: &ForIn, end_lbl: Label) {
        match for_in {
            ForIn::Coll {
                lval, iter_local, ..
            } => {
                let local = self.prog.locals[*iter_local].var_num;
                self.gen_iter_cond(lval, local, end_lbl);
            }
            ForIn::Range {
                lval,
                to,
                step,
                to_local,
                step_local,
                ..
            } => {
                self.gen_expr(lval, false, false);
                match (to.is_const(), to_local) {
                    (false, Some(local)) => {
                        self.gen_getlcl(self.prog.locals[*local].var_num, false);
                    }
                    _ => self.gen_expr(to, false, false),
                }
                // Which way the range runs is known if the step is.
                let sign = match step.as_ref().map(|step| step.as_const()) {
                    None => 1,
                    Some(Some(ConstVal {
                        value: Value::Int(step),
                        ..
                    })) => {
                        if *step < 0 {
                            -1
                        } else {
                            1
                        }
                    }
                    Some(Some(ConstVal {
                        value: Value::Float { text, .. },
                        ..
                    })) => {
                        if text.first() == Some(&b'-') {
                            -1
                        } else {
                            1
                        }
                    }
                    Some(_) => 0,
                };
                if sign > 0 {
                    self.cg.write_jump(Op::Jgt, end_lbl);
                    self.cg.note_pops(2);
                } else if sign < 0 {
                    self.cg.write_jump(Op::Jlt, end_lbl);
                    self.cg.note_pops(2);
                } else {
                    // Find out when the program runs.
                    self.gen_range_step(step.as_ref(), *step_local);
                    self.cg.write_op(Op::Push0);
                    self.cg.note_push();
                    let down = self.gen_jump_ahead(Op::Jlt);
                    self.cg.note_pops(2);
                    self.cg.write_jump(Op::Jgt, end_lbl);
                    let done = self.gen_jump_ahead(Op::Jmp);
                    self.def_label_pos(down);
                    self.cg.write_jump(Op::Jlt, end_lbl);
                    self.def_label_pos(done);
                    self.cg.note_pops(2);
                }
            }
        }
    }

    /// Push a range's step: the local it was put in, if it is not a
    /// constant.
    fn gen_range_step(&mut self, step: Option<&Expr>, step_local: Option<LocalId>) {
        match (step, step_local) {
            (Some(step), Some(local)) if !step.is_const() => {
                self.gen_getlcl(self.prog.locals[local].var_num, false);
            }
            (Some(step), _) => self.gen_expr(step, false, false),
            (None, _) => {}
        }
    }

    /// What an `in` clause of a `for` does at the end of each turn.
    fn gen_for_in_reinit(&mut self, for_in: &ForIn) {
        let ForIn::Range {
            lval,
            step,
            step_local,
            ..
        } = for_in
        else {
            return;
        };
        match (step, step_local) {
            (None, _) => {
                let inc = Expr::IncDec(AsiOp::PreInc, Box::new(lval.clone()));
                self.gen_expr(&inc, true, false);
            }
            (Some(step), local) => {
                let step = match local {
                    Some(local) if !step.is_const() => Expr::Local(*local),
                    _ => step.clone(),
                };
                let add = Expr::Assign(AsiOp::Add, Box::new(lval.clone()), Box::new(step));
                self.gen_expr(&add, true, false);
            }
        }
    }

    // ---- objects and properties ----

    /// What a name is, when the name stands alone in an expression. A
    /// name that is nothing yet is taken to be a property where there
    /// is a `self` for it to be a property of, with a warning; where
    /// there is none, it is reported as undefined.
    fn implied_global(&mut self, name: &[u8]) -> Option<Global> {
        match self.prog.globals.get(name) {
            Some(global) => Some(*global),
            None if self.self_available && !self.undefined_names.contains(name) => {
                self.assume_prop(name);
                Some(Global::Prop(self.prog.def_prop(name)))
            }
            None => {
                self.undefined(name);
                None
            }
        }
    }

    /// The value of a variable of a running function: the object for its
    /// call, indexed by the variable's number, and then, for a variable
    /// in a context, by its index there.
    fn gen_dyn_local(&mut self, local: usize, discard: bool) {
        if discard {
            return;
        }
        let (frame_ref, number, context) = self.dyn_local(local);
        self.cg.write_op(Op::PushObj);
        self.write_obj_id(frame_ref);
        self.cg.note_push();
        self.gen_int(i32::from(number));
        self.cg.write_op(Op::Index);
        self.cg.note_pop();
        if context != 0 {
            self.gen_int(i32::from(context));
            self.cg.write_op(Op::Index);
            self.cg.note_pop();
        }
    }

    /// Assign to a variable of a running function, through the object
    /// for its call.
    fn gen_dyn_local_asi(
        &mut self,
        local: usize,
        discard: bool,
        phase: u8,
        typ: AsiOp,
        rhs: Option<&Expr>,
    ) -> bool {
        // Any assignment but a plain one fetches the value first.
        if typ != AsiOp::Simple && phase == 1 {
            self.gen_dyn_local(local, false);
            return false;
        }
        if let Some(rhs) = rhs {
            self.gen_expr(rhs, false, false);
        }
        if !discard {
            self.cg.write_op(Op::Dup);
            self.cg.note_push();
        }
        let (frame_ref, number, context) = self.dyn_local(local);
        self.cg.write_op(Op::PushObj);
        self.write_obj_id(frame_ref);
        self.cg.note_push();
        self.gen_int(i32::from(number));
        if context != 0 {
            self.cg.write_op(Op::Index);
            self.cg.note_pop();
            self.gen_int(i32::from(context));
        }
        self.cg.write_op(Op::SetInd);
        self.cg.write_op(Op::Disc);
        self.cg.note_pops(3);
        true
    }

    /// Whether an object can be called as a function: in code compiled
    /// while the program runs, one such as a function compiled before.
    fn is_func_obj(&self, obj: usize) -> bool {
        self.prog
            .dynamic
            .as_ref()
            .is_some_and(|dynamic| dynamic.func_objs.contains(&obj))
    }

    /// A variable of a running function: the object for its call, its
    /// number, and its index in a context object or 0.
    fn dyn_local(&self, local: usize) -> (u32, u16, u16) {
        self.prog
            .dynamic
            .as_ref()
            .and_then(|dynamic| dynamic.locals.get(local).copied())
            .unwrap_or_default()
    }

    /// Write a property's number into the code, and note where, for
    /// the number is not the one the image will have.
    fn write_prop_id(&mut self, id: u16) {
        self.out.id_fixups.push((self.cg.ofs(), IdKind::Prop));
        self.cg.write2(id);
    }

    /// Write an object's number in the code. Zero is no object.
    fn write_obj_id(&mut self, id: u32) {
        // An object code compiled while the program runs had the program
        // make has its number now.
        let id = self
            .prog
            .dynamic
            .as_ref()
            .and_then(|dynamic| dynamic.gen_obj_ids.get(&id).copied())
            .unwrap_or(id);
        if id != 0 {
            self.out.id_fixups.push((self.cg.ofs(), IdKind::Obj));
        }
        self.cg.write4(id);
    }

    fn write_enum_id(&mut self, id: u32) {
        self.out.id_fixups.push((self.cg.ofs(), IdKind::Enum));
        self.cg.write4(id);
    }

    /// Push the address of what a name stands for.
    fn gen_addr(&mut self, sub: &Expr) {
        let Expr::Sym(name) = sub else {
            return;
        };
        // A name that is nothing yet is a property's.
        let global = match self.prog.globals.get(name) {
            Some(global) => *global,
            None => Global::Prop(self.prog.def_prop(name)),
        };
        match global {
            Global::Prop(prop) => {
                self.prog.props[prop].weak = false;
                self.cg.write_op(Op::PushPropId);
                self.write_prop_id(self.prog.props[prop].id);
                self.cg.note_push();
            }
            Global::Obj(obj) => {
                self.cg.write_op(Op::PushObj);
                self.write_obj_id(self.prog.objs[obj].id);
                self.cg.note_push();
            }
            Global::Func(func) => {
                self.cg.write_op(Op::PushFnPtr);
                self.gen_func_ref(func);
                self.cg.note_push();
            }
            // A built-in function: a pointer to it, by its place in its
            // set and the set's.
            Global::Bif(bif) => {
                let (index, set) = (self.prog.bifs[bif].index, self.prog.bifs[bif].set);
                self.cg.write_op(Op::PushBifPtr);
                self.cg.write2(index as u16);
                self.cg.write2(set as u16);
                self.cg.note_push();
            }
            Global::Meta(_) | Global::Enum(_) | Global::DynLocal(_) => {
                self.error(format!("cannot take address of \"{}\"", show_name(name)));
            }
        }
    }

    /// Generate `new` of an operand. A call there is the class with
    /// the constructor's arguments; `from_call` says the operand was
    /// what such a call calls.
    fn gen_new(
        &mut self,
        sub: &Expr,
        transient: bool,
        discard: bool,
        args: CallArgs,
        from_call: bool,
    ) {
        let CallArgs {
            argc,
            varargs,
            named,
        } = args;
        const INVALID: &str = "cannot apply operator 'new' to this expression";
        let obj = match sub {
            Expr::Call(func, args) => {
                if from_call {
                    self.error(INVALID);
                    return;
                }
                let args = self.gen_arglist(args);
                self.gen_new(func, transient, discard, args, true);
                return;
            }
            Expr::Const(ConstVal {
                value: Value::Obj(obj),
                ..
            }) => *obj,
            Expr::Sym(name) => {
                match self.prog.globals.get(name).copied() {
                    Some(Global::Meta(meta)) => {
                        // An object of a built-in class.
                        let index = self.prog.metas[meta].meta;
                        if varargs {
                            self.cg.write_op(Op::VarArgc);
                        }
                        if index <= 255 && argc <= 255 {
                            self.cg
                                .write_op(if transient { Op::TrNew1 } else { Op::New1 });
                            self.cg.write(argc as u8);
                            self.cg.write(index as u8);
                        } else {
                            self.cg
                                .write_op(if transient { Op::TrNew1 } else { Op::New2 });
                            self.cg.write2(argc as u16);
                            self.cg.write2(index as u16);
                        }
                        self.cg.note_pops(argc as i32);
                        self.post_call_cleanup(named);
                        if !discard {
                            self.cg.write_op(Op::GetR0);
                            self.cg.note_push();
                        }
                    }
                    // A dictionary: the program cannot make another.
                    Some(Global::Obj(_)) => {
                        self.error("invalid 'new' - cannot create an instance of this type");
                    }
                    Some(_) => self.error(INVALID),
                    None => self.undefined(name),
                }
                return;
            }
            _ => {
                self.error(INVALID);
                return;
            }
        };
        self.cg.write_op(Op::PushObj);
        self.write_obj_id(obj);
        self.cg.note_push();
        if argc > 126 {
            self.error("'new' argument list exceeds machine limit (maximum 126)");
        }
        if varargs {
            self.cg.write_op(Op::Swap);
            self.cg.write_op(Op::Inc);
            self.cg.write_op(Op::VarArgc);
        }
        self.cg
            .write_op(if transient { Op::TrNew1 } else { Op::New1 });
        self.cg.write(argc as u8 + 1);
        self.cg.write(0);
        self.cg.note_pops(argc as i32 + 1);
        self.post_call_cleanup(named);
        if !discard {
            self.cg.write_op(Op::GetR0);
            self.cg.note_push();
        }
    }

    /// Whether a constant is a list as the source wrote it, whatever
    /// value it has now.
    fn is_list_node(value: &ConstVal) -> bool {
        matches!(value.value, Value::List(_)) || value.list_node.is_some()
    }

    fn assume_prop(&mut self, name: &[u8]) {
        self.log(
            Severity::Warning,
            format!(
                "undefined symbol \"{}\" - assuming this is a property name",
                show_name(name)
            ),
        );
    }

    fn need_self(&mut self) {
        if !self.self_available {
            self.error("'self' is not valid in this context");
        }
    }

    fn prop_needs_obj(&mut self, prop: usize) {
        let name = show_name(&self.prog.props[prop].name);
        self.error(format!(
            "cannot call property \"{name}\" without \"object.\" prefix - no \"self\""
        ));
    }

    /// Get a property of `self`.
    fn gen_prop_self(&mut self, prop: usize, discard: bool) {
        if !self.self_available {
            self.prop_needs_obj(prop);
            return;
        }
        self.cg.write_op(Op::GetPropSelf);
        self.write_prop_id(self.prog.props[prop].id);
        if !discard {
            self.cg.write_op(Op::GetR0);
            self.cg.note_push();
        }
    }

    /// Which property the right side of a `.` is, if that is known
    /// when compiling. If it is not, and `check_only` is not set, the
    /// code that works it out is generated.
    fn gen_propid(&mut self, prop: &Expr, is_expr: bool, check_only: bool) -> Option<u16> {
        let name = match prop {
            Expr::Sym(name) => name,
            Expr::Const(ConstVal {
                value: Value::Prop(prop),
                ..
            }) => return Some(*prop),
            // A constant list is still a list to the reference, and is
            // generated like any expression.
            Expr::Const(value) if !Self::is_list_node(value) => {
                if !check_only {
                    self.error("invalid '.' expression - right side is not a property");
                }
                return None;
            }
            _ => {
                if !check_only {
                    self.gen_expr(prop, false, false);
                }
                return None;
            }
        };
        // A name is a property's; if it is nothing yet, it becomes
        // one.
        let global = match self.prog.globals.get(name) {
            Some(global) => *global,
            // A name already found to be nothing is noted as that, and
            // is no property.
            None if self.undefined_names.contains(name) => {
                if !check_only {
                    self.error(format!(
                        "invalid '.' expression - symbol \"{}\" is not a property",
                        show_name(name)
                    ));
                }
                return None;
            }
            None => {
                self.assume_prop(name);
                Global::Prop(self.prog.def_prop(name))
            }
        };
        match global {
            Global::Prop(prop) if is_expr => {
                // In parentheses, it is the property of `self` whose
                // value says which property is meant.
                if !check_only {
                    self.cg.write_op(Op::GetPropSelf);
                    self.write_prop_id(self.prog.props[prop].id);
                    self.cg.write_op(Op::GetR0);
                    self.cg.note_push();
                }
                None
            }
            Global::Prop(prop) => Some(self.prog.props[prop].id),
            _ => {
                if !check_only {
                    self.error(format!(
                        "invalid '.' expression - symbol \"{}\" is not a property",
                        show_name(name)
                    ));
                }
                None
            }
        }
    }

    /// Generate `obj.prop` or `obj.prop(args)`. The arguments are on
    /// the stack already.
    fn gen_member(
        &mut self,
        obj: &Expr,
        prop: &Expr,
        is_expr: bool,
        args: CallArgs,
        discard: bool,
    ) {
        let CallArgs {
            argc,
            varargs,
            named,
        } = args;
        // A property of a dictionary is got as any object's is whose
        // number is known.
        let as_const;
        let obj = match obj {
            Expr::Sym(name) => match self.prog.globals.get(name) {
                Some(Global::Obj(dict)) => {
                    as_const = Expr::constant(Value::Obj(self.prog.objs[*dict].id));
                    &as_const
                }
                _ => obj,
            },
            other => other,
        };
        // In a version of a multi-method, `inherited` is the next
        // version, which the library finds, or with a list of types
        // the version for those.
        if let Expr::Inh | Expr::InhTypes(..) = obj {
            if let Some(func) = self.enclosing_mm() {
                self.gen_mm_inherited(func, obj, argc, varargs, named, discard);
                return;
            }
        }
        match obj {
            Expr::InhTypes(..) => {
                self.need_self();
                self.error(
                    "'inherited<>' is not valid here; it can only be used in a multi-method",
                );
                self.gen_propid(prop, is_expr, false);
                return;
            }
            Expr::Inh => {
                self.need_self();
                match self.gen_propid(prop, is_expr, false) {
                    Some(id) => {
                        if varargs {
                            self.cg.write_op(Op::VarArgc);
                        }
                        self.cg.write_op(Op::Inherit);
                        self.cg.write(argc as u8);
                        self.write_prop_id(id);
                        self.cg.note_pops(argc as i32);
                    }
                    None => {
                        if varargs {
                            self.cg.write_op(Op::Swap);
                            self.cg.write_op(Op::VarArgc);
                        }
                        self.cg.write_op(Op::PtrInherit);
                        self.cg.write(argc as u8);
                        self.cg.note_pops(argc as i32 + 1);
                    }
                }
            }
            Expr::Delegated(target) => {
                self.need_self();
                self.gen_expr(target, false, false);
                // The count of arguments goes back on top.
                if varargs {
                    self.cg.write_op(Op::Swap);
                }
                match self.gen_propid(prop, is_expr, false) {
                    Some(id) => {
                        if varargs {
                            self.cg.write_op(Op::VarArgc);
                        }
                        self.cg.write_op(Op::Delegate);
                        self.cg.write(argc as u8);
                        self.write_prop_id(id);
                        self.cg.note_pops(argc as i32 + 1);
                    }
                    None => {
                        if varargs {
                            self.cg.write_op(Op::Swap);
                            self.cg.write_op(Op::VarArgc);
                        }
                        self.cg.write_op(Op::PtrDelegate);
                        self.cg.write(argc as u8);
                        self.cg.note_pops(argc as i32 + 2);
                    }
                }
            }
            Expr::InhClass(name) => {
                self.need_self();
                let class = match self.prog.globals.get(name).copied() {
                    Some(Global::Obj(class)) => self.prog.objs[class].id,
                    other => {
                        if other.is_none() {
                            self.undefined(name);
                        }
                        self.error(format!(
                            "illegal 'inherited' - \"{}\" is not a class",
                            show_name(name)
                        ));
                        return;
                    }
                };
                match self.gen_propid(prop, is_expr, false) {
                    Some(id) => {
                        if varargs {
                            self.cg.write_op(Op::VarArgc);
                        }
                        self.cg.write_op(Op::ExpInherit);
                        self.cg.write(argc as u8);
                        self.write_prop_id(id);
                        self.write_obj_id(class);
                        self.cg.note_pops(argc as i32);
                    }
                    None => {
                        if varargs {
                            self.cg.write_op(Op::Swap);
                            self.cg.write_op(Op::VarArgc);
                        }
                        self.cg.write_op(Op::PtrExpInherit);
                        self.cg.write(argc as u8);
                        self.write_obj_id(class);
                        self.cg.note_pops(argc as i32 + 1);
                    }
                }
            }
            Expr::SelfRef => {
                self.need_self();
                match self.gen_propid(prop, is_expr, false) {
                    Some(id) => {
                        if argc == 0 {
                            self.cg.write_op(Op::GetPropSelf);
                        } else {
                            if varargs {
                                self.cg.write_op(Op::VarArgc);
                            }
                            self.cg.write_op(Op::CallPropSelf);
                            self.cg.write(argc as u8);
                        }
                        self.write_prop_id(id);
                        self.cg.note_pops(argc as i32);
                    }
                    None => {
                        if varargs {
                            self.cg.write_op(Op::Swap);
                            self.cg.write_op(Op::VarArgc);
                        }
                        self.cg.write_op(Op::PtrCallPropSelf);
                        self.cg.write(argc as u8);
                        self.cg.note_pops(argc as i32 + 1);
                    }
                }
            }
            Expr::Const(ConstVal {
                value: Value::Obj(obj),
                ..
            }) => match self.gen_propid(prop, is_expr, true) {
                Some(id) => {
                    if argc == 0 {
                        self.cg.write_op(Op::ObjGetProp);
                    } else {
                        if varargs {
                            self.cg.write_op(Op::VarArgc);
                        }
                        self.cg.write_op(Op::ObjCallProp);
                        self.cg.write(argc as u8);
                    }
                    self.write_obj_id(*obj);
                    self.write_prop_id(id);
                    self.cg.note_pops(argc as i32);
                }
                None => {
                    self.cg.write_op(Op::PushObj);
                    self.write_obj_id(*obj);
                    self.cg.note_push();
                    if varargs {
                        self.cg.write_op(Op::Swap);
                    }
                    self.gen_propid(prop, is_expr, false);
                    if varargs {
                        self.cg.write_op(Op::Swap);
                        self.cg.write_op(Op::VarArgc);
                    }
                    self.cg.write_op(Op::PtrCallProp);
                    self.cg.write(argc as u8);
                    self.cg.note_pops(argc as i32 + 2);
                }
            },
            Expr::Const(ConstVal {
                value:
                    Value::Nil
                    | Value::True
                    | Value::Int(_)
                    | Value::FuncPtr(_)
                    | Value::Prop(_)
                    | Value::Enum(_),
                ..
            }) => {
                self.error("illegal expression - left of '.' is not an object");
                return;
            }
            Expr::Sym(name) => {
                // A property of `self`, whose value is the object.
                match self.prog.globals.get(name).copied() {
                    Some(Global::Prop(obj_prop)) => {
                        if !self.self_available {
                            self.prop_needs_obj(obj_prop);
                        } else {
                            self.gen_prop_self(obj_prop, false);
                        }
                        self.gen_member_rhs(prop, is_expr, argc, varargs);
                    }
                    Some(Global::Meta(meta)) => {
                        self.cg.write_op(Op::PushObj);
                        self.write_obj_id(self.prog.metas[meta].obj_id);
                        self.cg.note_push();
                        self.gen_member_rhs(prop, is_expr, argc, varargs);
                    }
                    Some(_) => {
                        self.error(format!(
                            "symbol \"{}\" is not an object - illegal on left of '.'",
                            show_name(name)
                        ));
                        return;
                    }
                    None => {
                        self.undefined(name);
                        return;
                    }
                }
            }
            _ => {
                self.gen_expr(obj, false, false);
                self.gen_member_rhs(prop, is_expr, argc, varargs);
            }
        }
        self.post_call_cleanup(named);
        if !discard {
            self.cg.write_op(Op::GetR0);
            self.cg.note_push();
        }
    }

    /// The right side of a `.`, with the object on the stack.
    fn gen_member_rhs(&mut self, prop: &Expr, is_expr: bool, argc: usize, varargs: bool) {
        if varargs {
            self.cg.write_op(Op::Swap);
        }
        match self.gen_propid(prop, is_expr, false) {
            Some(id) if argc == 0 => {
                self.cg.write_op(Op::GetProp);
                self.write_prop_id(id);
                self.cg.note_pop();
            }
            Some(id) => {
                self.cg.write_callprop(argc, varargs);
                self.write_prop_id(id);
            }
            None => {
                if varargs {
                    self.cg.write_op(Op::Swap);
                    self.cg.write_op(Op::VarArgc);
                }
                self.cg.write_op(Op::PtrCallProp);
                self.cg.write(argc as u8);
                self.cg.note_pops(argc as i32 + 2);
            }
        }
    }

    /// The object on the left of a `.` that is being assigned to:
    /// whether it is `self`, and which object it is if that is known
    /// when compiling. If neither, the code that works it out is
    /// generated.
    fn gen_obj_predot(&mut self, obj: &Expr) -> (bool, Option<u32>) {
        match obj {
            Expr::SelfRef => {
                self.need_self();
                (true, None)
            }
            Expr::Const(ConstVal {
                value: Value::Obj(obj),
                ..
            }) => (false, Some(*obj)),
            Expr::Const(value) if !Self::is_list_node(value) => {
                self.error("illegal expression - left of '.' is not an object");
                (false, None)
            }
            Expr::Sym(name) => {
                match self.prog.globals.get(name).copied() {
                    Some(Global::Prop(prop)) => self.gen_prop_self(prop, false),
                    Some(Global::Obj(dict)) => return (false, Some(self.prog.objs[dict].id)),
                    Some(_) => self.error(format!(
                        "symbol \"{}\" is not an object - illegal on left of '.'",
                        show_name(name)
                    )),
                    None => self.undefined(name),
                }
                (false, None)
            }
            _ => {
                self.gen_expr(obj, false, false);
                (false, None)
            }
        }
    }

    /// Assign to `obj.prop`. See [`Gen::gen_asi`] for the phases.
    #[allow(clippy::too_many_arguments)]
    fn gen_member_asi(
        &mut self,
        obj: &Expr,
        prop: &Expr,
        is_expr: bool,
        discard: bool,
        phase: u8,
        typ: AsiOp,
        rhs: Option<&Expr>,
    ) -> bool {
        let simple = typ == AsiOp::Simple;
        let (mut is_self, mut obj_id, mut prop_id) = (false, None, None);
        if phase == 2 {
            (is_self, obj_id, prop_id) = self.member_ctx.pop().unwrap_or((false, None, None));
        }
        if let (true, Some(rhs)) = (simple, rhs) {
            self.gen_expr(rhs, false, false);
        }
        if !discard && (simple || phase == 2) {
            self.cg.write_op(Op::Dup);
            self.cg.note_push();
        }
        if simple || phase == 1 {
            (is_self, obj_id) = self.gen_obj_predot(obj);
            prop_id = self.gen_propid(prop, is_expr, true);
            if prop_id.is_none() {
                if is_self {
                    self.cg.write_op(Op::PushSelf);
                    self.cg.note_push();
                } else if let Some(id) = obj_id {
                    self.cg.write_op(Op::PushObj);
                    self.write_obj_id(id);
                    self.cg.note_push();
                }
                self.gen_propid(prop, is_expr, false);
                if !simple {
                    self.cg.write_op(Op::Dup2);
                    self.cg.note_pushes(2);
                }
            } else if !simple && !is_self && obj_id.is_none() {
                self.cg.write_op(Op::Dup);
                self.cg.note_push();
            }
        }
        if !simple && phase == 1 {
            // The value the property has now.
            match (prop_id, obj_id) {
                (None, _) => {
                    self.cg.write_op(Op::PtrCallProp);
                    self.cg.write(0);
                    self.cg.note_pops(2);
                }
                (Some(prop), _) if is_self => {
                    self.cg.write_op(Op::GetPropSelf);
                    self.write_prop_id(prop);
                }
                (Some(prop), Some(obj)) => {
                    self.cg.write_op(Op::ObjGetProp);
                    self.write_obj_id(obj);
                    self.write_prop_id(prop);
                }
                (Some(prop), None) => {
                    self.cg.write_op(Op::GetProp);
                    self.write_prop_id(prop);
                    self.cg.note_pop();
                }
            }
            self.cg.write_op(Op::GetR0);
            self.cg.note_push();
            self.member_ctx.push((is_self, obj_id, prop_id));
            return false;
        }
        match prop_id {
            None => {
                if !simple && phase == 2 {
                    // Put the new value under the object and the
                    // property.
                    if discard {
                        self.cg.write_op(Op::SwapN);
                        self.cg.write(0);
                        self.cg.write(2);
                        self.cg.write_op(Op::Swap);
                    } else {
                        self.cg.write_op(Op::Swap2);
                    }
                }
                self.cg.write_op(Op::PtrSetProp);
                self.cg.note_pops(3);
            }
            Some(prop) => {
                if phase == 2 && !is_self && obj_id.is_none() {
                    if discard {
                        self.cg.write_op(Op::Swap);
                    } else {
                        self.cg.write_op(Op::SwapN);
                        self.cg.write(0);
                        self.cg.write(2);
                    }
                }
                if is_self {
                    self.cg.write_op(Op::SetPropSelf);
                    self.write_prop_id(prop);
                    self.cg.note_pop();
                } else if let Some(obj) = obj_id {
                    self.cg.write_op(Op::ObjSetProp);
                    self.write_obj_id(obj);
                    self.write_prop_id(prop);
                    self.cg.note_pop();
                } else {
                    self.cg.write_op(Op::SetProp);
                    self.write_prop_id(prop);
                    self.cg.note_pops(2);
                }
            }
        }
        true
    }

    /// Report a name that nothing defines. It is reported once: the
    /// reference then notes the name as one that is known to be
    /// undefined, and says nothing about it again.
    fn undefined(&mut self, name: &[u8]) {
        if self.undefined_names.insert(name.to_vec()) {
            self.error(format!("undefined symbol \"{}\"", show_name(name)));
        }
    }

    /// Generate a choice among alternatives in a string. The object
    /// the compiler defined for it is asked which one, by number.
    fn gen_one_of(
        &mut self,
        elements: &[Expr],
        state: Option<u32>,
        discard: bool,
        for_condition: bool,
    ) {
        const GET_NEXT: &[u8] = b"getNextIndex";
        let prop = match self.prog.globals.get(GET_NEXT).copied() {
            Some(Global::Prop(prop)) => prop,
            None => self.prog.def_prop(GET_NEXT),
            Some(_) => {
                self.error("<<one of>> requires getNextIndex to be defined as a property");
                return;
            }
        };
        // Without the object, the parser has said why.
        let Some(state) = state else {
            return;
        };
        let state = Expr::constant(Value::Obj(state));
        let get_next = Expr::constant(Value::Prop(self.prog.props[prop].id));
        let no_args = || CallArgs {
            argc: 0,
            varargs: false,
            named: None,
        };
        if elements.iter().all(Expr::is_const) {
            // Constants: a list of them, indexed by the number.
            let list = Expr::list(elements.to_vec());
            self.gen_expr(&list, false, false);
            self.gen_member(&state, &get_next, false, no_args(), false);
            self.cg.write_op(Op::Index);
            self.cg.note_pop();
            return;
        }
        // Otherwise a switch on the number, with a case for each
        // alternative that does what the alternative says.
        self.gen_member(&state, &get_next, false, no_args(), false);
        self.cg.write_op(Op::Switch);
        self.cg.write2(elements.len() as u16);
        self.cg.note_pop();
        let table = self.cg.ofs();
        for index in 1..=elements.len() {
            self.cg.write(datatype::INT);
            self.cg.write4(index as u32);
            self.cg.write2(0);
        }
        let default_slot = self.cg.ofs();
        self.cg.write2(0);
        let end_lbl = self.cg.new_label();
        for (index, element) in elements.iter().enumerate() {
            let slot = table + 7 * index + 5;
            let jump = self.cg.ofs() - slot;
            self.cg.write2_at(slot, jump as u16);
            self.gen_expr(element, discard, for_condition);
            self.cg.write_jump(Op::Jmp, end_lbl);
            // Only one of them will have left a value.
            if !discard {
                self.cg.note_pop();
            }
        }
        // No such alternative: nil.
        let jump = self.cg.ofs() - default_slot;
        self.cg.write2_at(default_slot, jump as u16);
        if !discard {
            self.cg.write_op(Op::PushNil);
            self.cg.note_push();
        }
        self.cg.def_label(end_lbl);
    }

    /// Generate an object written in an expression: a new object of
    /// its superclasses, with no constructor run, then each property
    /// set on it, and then its own constructor if it has one. The
    /// object is left on the stack, wanted or not.
    fn gen_inline_obj(&mut self, superclasses: &[Vec<u8>], props: &[InlineProp]) {
        for name in superclasses.iter().rev() {
            match self.prog.globals.get(name).copied() {
                Some(Global::Obj(obj)) => {
                    self.cg.write_op(Op::PushObj);
                    self.write_obj_id(self.prog.objs[obj].id);
                    self.cg.note_push();
                }
                Some(Global::Meta(meta)) => {
                    self.cg.write_op(Op::PushObj);
                    self.write_obj_id(self.prog.metas[meta].obj_id);
                    self.cg.note_push();
                }
                _ => self.undefined(name),
            }
        }
        // The class is TadsObject in TADS 3, and ProgramObject in
        // Hexicon's system library.
        let class = [b"TadsObject".as_slice(), b"ProgramObject"]
            .into_iter()
            .find_map(|name| self.prog.globals.get(name).copied());
        let Some(Global::Meta(program_object)) = class else {
            self.error("\"TadsObject\" is not defined or is not an intrinsic class");
            return;
        };
        let Some(create) = self.named_prop(b"createInstanceOf") else {
            return;
        };
        self.cg.write_op(Op::ObjCallProp);
        self.cg.write(superclasses.len() as u8);
        self.write_obj_id(self.prog.metas[program_object].obj_id);
        self.write_prop_id(create);
        self.cg.note_pops(superclasses.len() as i32);
        self.cg.write_op(Op::GetR0);
        self.cg.note_push();

        let mut has_ctor = false;
        for prop in props {
            (self.cur_file, self.cur_line) = (prop.file, prop.line);
            self.line_info = (prop.file, prop.line);
            let id = self.prog.props[prop.prop].id;
            match &prop.value {
                // A method is bound to the object.
                InlineValue::Method(method) | InlineValue::Wrapped(_, method) => {
                    self.gen_expr(method, false, false);
                    self.gen_set_method(id);
                }
                // Text to display is bound like a method.
                InlineValue::Eval(Expr::DStr(text)) => {
                    self.gen_str(text);
                    self.gen_set_method(id);
                }
                InlineValue::Eval(expr) => {
                    self.cg.write_op(Op::Dup);
                    self.cg.note_push();
                    self.gen_expr(expr, false, false);
                    if !self.has_return_value(expr) {
                        self.cg.write_op(Op::PushNil);
                        self.cg.note_push();
                    }
                    self.cg.write_op(Op::Swap);
                    self.cg.write_op(Op::SetProp);
                    self.write_prop_id(id);
                    self.cg.note_pops(2);
                }
            }
            has_ctor |= prop.prop == 0;
        }
        if has_ctor {
            self.cg.write_op(Op::Dup);
            self.cg.write_op(Op::CallProp);
            self.cg.write(0);
            self.write_prop_id(1);
        }
    }

    /// The number of a property the compiler needs by name, which the
    /// program has to have.
    fn named_prop(&mut self, name: &[u8]) -> Option<u16> {
        match self.prog.globals.get(name) {
            Some(Global::Prop(prop)) => Some(self.prog.props[*prop].id),
            _ => {
                self.error(format!(
                    "invalid '.' expression - symbol \"{}\" is not a property",
                    show_name(name)
                ));
                None
            }
        }
    }

    /// Bind the value on top of the stack to the object under it as
    /// the method of a property: `obj.setMethod(prop, value)`.
    fn gen_set_method(&mut self, prop: u16) {
        let Some(set_method) = self.named_prop(b"setMethod") else {
            return;
        };
        self.cg.write_op(Op::PushPropId);
        self.write_prop_id(prop);
        self.cg.note_push();
        self.cg.write_op(Op::GetSpN);
        self.cg.write(2);
        self.cg.note_push();
        self.cg.write_op(Op::CallProp);
        self.cg.write(2);
        self.write_prop_id(set_method);
        self.cg.note_pops(3);
    }

    /// A function of the library's that the compiler calls: the one of
    /// that name, which is declared if it is nothing yet, to be found
    /// missing when the program is put together.
    fn internal_func(&mut self, name: &[u8], argc: usize, varargs: bool) -> Option<usize> {
        match self.prog.globals.get(name).copied() {
            Some(Global::Func(func)) => Some(func),
            None => {
                self.prog.funcs.push(crate::parse::FuncSym {
                    name: name.to_vec(),
                    argc,
                    opt_argc: 0,
                    varargs,
                    has_retval: true,
                    is_extern: true,
                    has_proto: true,
                    body: None,
                    mod_bases: Vec::new(),
                    mod_extern: false,
                    mod_proto: (0, 0, false),
                    ext_redef: false,
                    is_mm: false,
                    is_mm_base: false,
                });
                let func = self.prog.funcs.len() - 1;
                self.prog.globals.insert(name.to_vec(), Global::Func(func));
                Some(func)
            }
            Some(_) => {
                self.error(format!(
                    "symbol \"{}\" is already defined - can't redefine as function",
                    show_name(name)
                ));
                None
            }
        }
    }

    /// The version of a multi-method the code being generated is in,
    /// however deep, if it is in one.
    fn enclosing_mm(&self) -> Option<usize> {
        let mut closure = self.closures.get(self.cur_body)?;
        loop {
            if closure.is_func {
                // A body that `modify` has put another in front of is
                // still the version it was defined as.
                let func = closure.defined_as?;
                return self.prog.funcs[func].is_mm.then_some(func);
            }
            let mut next = closure.enclosing;
            loop {
                let body_ref = &self.prog.body_refs[next?];
                if let Some(body) = body_ref.func_body {
                    closure = &self.closures[body];
                    break;
                }
                next = body_ref.enclosing;
            }
        }
    }

    /// Generate `inherited(args)` or `inherited<types>(args)` in a
    /// version of a multi-method. The arguments are on the stack.
    fn gen_mm_inherited(
        &mut self,
        func: usize,
        obj: &Expr,
        argc: usize,
        varargs: bool,
        named: Option<usize>,
        discard: bool,
    ) {
        match obj {
            Expr::InhTypes(types, more) => {
                // The version for the types given, called outright.
                let name = &self.prog.funcs[func].name;
                let base_len = name.iter().position(|&c| c == b'*').unwrap_or(name.len());
                let base = name[..base_len].to_vec();
                let target = crate::parse::decorate_name(&base, types, *more);
                match self.prog.globals.get(&target).copied() {
                    Some(Global::Func(target)) => {
                        let target = Expr::constant(Value::FuncPtr(target));
                        self.gen_call(&target, discard, argc, varargs, named);
                    }
                    _ => self.error(format!(
                        "'inherited<>' function \"{}\" undefined",
                        show_name(&base)
                    )),
                }
            }
            _ => {
                // The library finds the next version, given this one.
                self.cg.write_op(Op::PushFnPtr);
                self.gen_func_ref(func);
                self.cg.note_push();
                const CALL_INHERITED: &[u8] = b"_multiMethodCallInherited";
                match self.prog.globals.get(CALL_INHERITED).copied() {
                    Some(Global::Func(call)) => {
                        let call = Expr::constant(Value::FuncPtr(call));
                        self.gen_call(&call, discard, argc + 1, varargs, named);
                    }
                    _ => self.error(
                        "'inherited<>' requires \"_multiMethodCallInherited\", \
                         which is not defined or not a function",
                    ),
                }
            }
        }
    }

    /// The functions in the order of the table of global names as it
    /// is when the program is put together.
    fn funcs_in_table_order(&self) -> Vec<usize> {
        let mut order: Vec<(usize, std::cmp::Reverse<usize>)> = self
            .prog
            .funcs
            .iter()
            .enumerate()
            .filter(|(index, func)| {
                self.prog.globals.get(&func.name) == Some(&Global::Func(*index))
            })
            .map(|(index, func)| {
                (
                    crate::parse::global_hash(&func.name),
                    std::cmp::Reverse(index),
                )
            })
            .collect();
        order.sort_unstable();
        order.into_iter().map(|(_, index)| index.0).collect()
    }

    /// Give a built-in class an object to hold what is added to it, if
    /// it has none: the library keeps what it knows of multi-methods
    /// in properties of the types of their parameters.
    fn ensure_mod_obj(&mut self, meta: usize) {
        if self.prog.metas[meta].mod_obj.is_none() {
            let id = self.prog.next_obj_id;
            self.prog.next_obj_id += 1;
            self.prog.metas[meta].mod_obj = Some(id);
            // It has a name in the table of global names, as the
            // objects `modify` sets aside do, which no source can write.
            self.prog
                .add_synthetic_obj(format!(" {id:x}").into_bytes(), id);
            self.out.objs.push(GenObj {
                id,
                is_class: false,
                superclasses: Vec::new(),
                props: Vec::new(),
                icmod: true,
                transient: false,
            });
        }
    }

    /// Generate the code that tells the library of each version of
    /// each multi-method: for each, a list of the types of its
    /// parameters, the version, and the multi-method. A parameter with
    /// no type is nil in the list, and `...` the string.
    fn gen_mm_init(&mut self) {
        const REGISTER: &[u8] = b"_multiMethodRegister";
        const BUILD: &[u8] = b"_multiMethodBuildBindings";
        let register = match self.prog.globals.get(REGISTER).copied() {
            Some(Global::Func(func)) => Some(func),
            _ => None,
        };
        let object = match self.prog.globals.get(b"Object".as_slice()).copied() {
            Some(Global::Meta(meta)) => Some(meta),
            _ => None,
        };
        let mut count = 0;
        'funcs: for func in self.funcs_in_table_order() {
            // A version's name is the multi-method's, `*`, and the
            // types.
            let name = self.prog.funcs[func].name.clone();
            let Some(star) = name.iter().position(|&c| c == b'*') else {
                continue;
            };
            if !name[..star].iter().all(|&c| crate::lex::is_sym(c)) {
                continue;
            }
            let Some(Global::Func(base)) = self.prog.globals.get(&name[..star]).copied() else {
                continue;
            };
            // The types, last first. The reference counts each value
            // but nil twice over as it works out how deep the stack
            // gets.
            let mut types: Vec<&[u8]> = name[star + 1..].split(|&c| c == b';').collect();
            types.pop();
            for kind in types.iter().rev() {
                if kind.is_empty() {
                    self.cg.write_op(Op::PushNil);
                    if let Some(object) = object {
                        self.ensure_mod_obj(object);
                    }
                } else if *kind == b"..." {
                    self.gen_str(b"...");
                    if let Some(object) = object {
                        self.ensure_mod_obj(object);
                    }
                } else {
                    match self.prog.globals.get(kind).copied() {
                        Some(Global::Obj(obj)) if !self.prog.objs[obj].is_extern => {
                            self.cg.write_op(Op::PushObj);
                            self.write_obj_id(self.prog.objs[obj].id);
                            self.cg.note_push();
                        }
                        Some(Global::Meta(meta)) => {
                            self.cg.write_op(Op::PushObj);
                            self.write_obj_id(self.prog.metas[meta].obj_id);
                            self.cg.note_push();
                            self.ensure_mod_obj(meta);
                        }
                        None | Some(Global::Obj(_)) => {
                            self.out
                                .link_errors
                                .push(format!("undefined symbol \"{}\"", show_name(kind)));
                            continue 'funcs;
                        }
                        Some(_) => {
                            self.out.link_errors.push(format!(
                                "parameter type \"{}\" is not an object",
                                show_name(kind)
                            ));
                            continue 'funcs;
                        }
                    }
                }
                self.cg.note_push();
            }
            let argc = types.len();
            if argc <= 255 {
                self.cg.write_op(Op::New1);
                self.cg.write(argc as u8);
                self.cg.write(METACLASS_LIST);
            } else {
                self.cg.write_op(Op::New2);
                self.cg.write2(argc as u16);
                self.cg.write2(u16::from(METACLASS_LIST));
            }
            self.cg.write_op(Op::GetR0);
            self.cg.note_pops(argc as i32);
            self.cg.note_push();
            for pushed in [func, base] {
                self.cg.write_op(Op::PushFnPtr);
                self.gen_func_ref(pushed);
                self.cg.note_push();
            }
            if let Some(register) = register {
                self.cg.write_op(Op::Call);
                self.cg.write(3);
                self.gen_func_ref(register);
            }
            self.cg.note_pops(3);
            count += 1;
        }
        self.mm_count = count;
        if count == 0 {
            return;
        }
        const MISSING: &str = " is missing or invalid - \
            check that the system library is included in the build";
        match self.prog.globals.get(BUILD).copied() {
            Some(Global::Func(build)) => {
                self.cg.write_op(Op::Call);
                self.cg.write(0);
                self.gen_func_ref(build);
            }
            _ => {
                self.out.link_errors.push(format!(
                    "library support function \"_multiMethodBuildBindings\"{MISSING}"
                ));
                self.mm_count = 0;
                return;
            }
        }
        if register.is_none() {
            self.out.link_errors.push(format!(
                "library support function \"_multiMethodRegister\"{MISSING}"
            ));
            self.mm_count = 0;
        }
    }

    /// Write the code the compiler writes itself when the program is
    /// put together: each multi-method's own function, after the rest
    /// of the code, and the code that tells the library of the
    /// versions, with an object to run it before the program starts.
    fn gen_multimethods(&mut self) {
        let synthetic = |kind: StmtKind, varargs: bool| CodeBody {
            lcltab: None,
            stm: Stmt {
                kind,
                file: usize::MAX,
                line: 0,
            },
            argc: 0,
            opt_argc: 0,
            varargs,
            varargs_list_local: None,
            local_cnt: 0,
            self_valid: false,
            replaced: false,
            labels: Rc::default(),
            start: None,
            end_file: usize::MAX,
            end_line: 0,
            closure: Closure::default(),
            stm_pos: None,
        };
        // Numbers for the bodies that no body the parser read has.
        const FIRST: usize = usize::MAX / 2;
        (self.cur_file, self.cur_line) = (usize::MAX, 0);
        let mut stubs = 0;
        for func in self.funcs_in_table_order() {
            let sym = &self.prog.funcs[func];
            if !sym.is_mm || sym.name.contains(&b'*') {
                continue;
            }
            let body = synthetic(StmtKind::MmStub(func), true);
            self.gen_code_body(&body, FIRST + 1 + stubs, Some(func), false);
            self.prog.funcs[func].is_extern = false;
            stubs += 1;
        }
        if stubs != 0
            && !matches!(
                self.prog.globals.get(b"_multiMethodCall".as_slice()),
                Some(Global::Func(_))
            )
        {
            self.out.link_errors.push(
                "library support function \"_multiMethodCall\" is missing or invalid - \
                 check that the system library is included in the build"
                    .to_owned(),
            );
        }

        // The code for the versions is always there, if only to return.
        let mark = (
            self.cg.ofs(),
            self.out.code_anchors.len(),
            self.out.fixups.len(),
            self.out.id_fixups.len(),
            self.out.unspecified.len(),
        );
        let body = synthetic(StmtKind::MmInit, false);
        self.gen_code_body(&body, FIRST, None, false);
        self.move_to_static(mark, FIRST);
        self.out.has_mm_init = true;
        if self.mm_count != 0 {
            let id = self.prog.next_obj_id;
            self.prog.next_obj_id += 1;
            self.prog
                .add_synthetic_obj(b"<multiMethodInit>".to_vec(), id);
            self.out.objs.push(GenObj {
                id,
                is_class: false,
                superclasses: Vec::new(),
                props: vec![GenProp {
                    id: 1,
                    kind: datatype::CODEOFS,
                    value: 0,
                    target: Some(Target::StaticBody(FIRST)),
                }],
                icmod: false,
                transient: false,
            });
            // It runs before the code for static properties.
            self.out.static_inits.insert(0, (id, 1));
        }
    }

    fn gen_unary(&mut self, opc: Op, sub: &Expr, discard: bool, for_condition: bool) {
        self.gen_expr(sub, discard, for_condition);
        if !discard {
            self.cg.write_op(opc);
        }
    }

    fn gen_binary(&mut self, opc: Op, left: &Expr, right: &Expr, discard: bool) {
        self.gen_expr(left, discard, false);
        self.gen_expr(right, discard, false);
        if !discard {
            self.cg.write_op(opc);
            self.cg.note_pop();
        }
    }

    /// Evaluate one side for whatever it does, and leave zero.
    fn gen_zero_after(&mut self, side: &Expr) {
        self.gen_expr(side, true, true);
        self.cg.write_op(Op::Push0);
        self.cg.note_push();
    }

    fn gen_binary_expr(
        &mut self,
        op: BinOp,
        left: &Expr,
        right: &Expr,
        discard: bool,
        for_condition: bool,
    ) {
        match op {
            BinOp::Comma => {
                self.gen_expr(left, true, true);
                self.gen_expr(right, discard, for_condition);
            }
            BinOp::Or | BinOp::And => {
                // The right side is only evaluated if the left does not
                // settle it. When the value is wanted, the jump keeps
                // the left side's value if it is taken.
                self.gen_expr(left, false, true);
                let opc = match (op, discard) {
                    (BinOp::Or, true) => Op::Jt,
                    (BinOp::Or, false) => Op::Jst,
                    (_, true) => Op::Jf,
                    (_, false) => Op::Jsf,
                };
                let lbl = self.gen_jump_ahead(opc);
                self.gen_expr(right, discard, true);
                self.cg.note_pop();
                self.def_label_pos(lbl);
                if !for_condition && !(left.is_bool() && right.is_bool()) {
                    self.cg.write_op(Op::Boolize);
                }
            }
            BinOp::BOr => self.gen_binary(Op::BOr, left, right, discard),
            BinOp::BAnd => self.gen_binary(Op::BAnd, left, right, discard),
            BinOp::BXor => self.gen_binary(Op::Xor, left, right, discard),
            BinOp::Eq => self.gen_binary(Op::Eq, left, right, discard),
            BinOp::Ne => self.gen_binary(Op::Ne, left, right, discard),
            BinOp::Gt => self.gen_binary(Op::Gt, left, right, discard),
            BinOp::Ge => self.gen_binary(Op::Ge, left, right, discard),
            BinOp::Lt => self.gen_binary(Op::Lt, left, right, discard),
            BinOp::Le => self.gen_binary(Op::Le, left, right, discard),
            BinOp::Shl => self.gen_binary(Op::Shl, left, right, discard),
            BinOp::AShr => self.gen_binary(Op::AShr, left, right, discard),
            BinOp::LShr => self.gen_binary(Op::LShr, left, right, discard),
            BinOp::Mul => {
                // Multiplying by zero or one needs no multiplication.
                // The zero is pushed even when the value is not wanted,
                // as the reference does.
                if left.is_const_int(0) {
                    self.gen_zero_after(right);
                } else if right.is_const_int(0) {
                    self.gen_zero_after(left);
                } else if left.is_const_int(1) {
                    self.gen_expr(right, discard, false);
                } else if right.is_const_int(1) {
                    self.gen_expr(left, discard, false);
                } else {
                    self.gen_binary(Op::Mul, left, right, discard);
                }
            }
            BinOp::Div => {
                if right.is_const_int(1) {
                    self.gen_expr(left, discard, for_condition);
                } else if left.is_const_int(0) {
                    self.gen_zero_after(right);
                } else {
                    self.gen_binary(Op::Div, left, right, discard);
                }
            }
            BinOp::Mod => {
                if right.is_const_int(1) {
                    self.gen_expr(left, discard, for_condition);
                    self.cg.write_op(Op::Push0);
                    self.cg.note_push();
                } else if left.is_const_int(0) {
                    self.gen_zero_after(right);
                } else {
                    self.gen_binary(Op::Mod, left, right, discard);
                }
            }
            BinOp::Sub | BinOp::Add => {
                let (plain, by_one) = if op == BinOp::Add {
                    (Op::Add, Op::Inc)
                } else {
                    (Op::Sub, Op::Dec)
                };
                if right.is_const_int(1) {
                    self.gen_expr(left, discard, false);
                    if !discard {
                        self.cg.write_op(by_one);
                    }
                } else {
                    self.gen_binary(plain, left, right, discard);
                }
            }
        }
    }

    // ---- calls ----

    /// Push a call's arguments, last first.
    fn gen_arglist(&mut self, args: &[Arg]) -> CallArgs {
        let varargs = args.iter().any(|arg| arg.varargs);
        let argc = args.iter().filter(|arg| arg.name.is_none()).count();
        let fixed_cnt = args
            .iter()
            .filter(|arg| !arg.varargs && arg.name.is_none())
            .count();
        // Arguments passed by name go on the stack first, under the
        // others.
        let names: Vec<Vec<u8>> = args
            .iter()
            .rev()
            .filter(|arg| !arg.varargs)
            .filter_map(|arg| arg.name.clone())
            .collect();
        let named = (!names.is_empty()).then(|| {
            for arg in args.iter().rev().filter(|arg| arg.name.is_some()) {
                self.gen_expr(&arg.expr, false, false);
                if arg.varargs {
                    self.cg.write_op(Op::MakeLstPar);
                    self.cg.note_push();
                    self.cg.note_pop();
                }
            }
            self.named_lists.push(names);
            self.named_lists.len() - 1
        });
        let mut pushed_varargs_counter = false;
        for (index, arg) in args.iter().enumerate().rev() {
            if arg.name.is_some() {
                continue;
            }
            let depth = self.cg.sp_depth();
            if arg.varargs && !pushed_varargs_counter {
                // The count starts at the number of plain arguments; the
                // lists add their lengths as the program runs.
                self.gen_int(fixed_cnt as i32);
                pushed_varargs_counter = true;
                self.cg.note_pop();
            }
            self.gen_expr(&arg.expr, false, false);
            if arg.varargs {
                self.cg.write_op(Op::MakeLstPar);
                self.cg.note_push();
                self.cg.note_pop();
            }
            if pushed_varargs_counter && !arg.varargs {
                self.cg.write_op(Op::Swap);
            }
            if self.cg.sp_depth() <= depth {
                // The reference counts down from the number passed by
                // position, over all of them.
                let number = argc as i64 - (args.len() - 1 - index) as i64;
                self.error(format!("argument expression {number} yields no value"));
            }
        }
        CallArgs {
            argc,
            varargs,
            named,
        }
    }

    /// After a call's instruction: say where the names of the
    /// arguments passed by name are, which also takes those arguments
    /// off the stack.
    fn post_call_cleanup(&mut self, named: Option<usize>) {
        let Some(list) = named else {
            return;
        };
        let cnt = self.named_lists[list].len();
        let label = self.cg.new_label();
        self.named_tabs.push((label, list));
        self.cg.write_op(Op::NamedArgPtr);
        self.cg.write(cnt as u8);
        self.cg.write_label_ofs(label);
        self.cg.note_pops(cnt as i32);
    }

    /// Call a function the compiler itself needs, which a system
    /// header has to have declared.
    fn call_internal_bif(&mut self, name: &[u8], what: &str, argc: usize) {
        // The function may have Hexicon's name for it, as it has in
        // Hexicon's system library.
        const HEXICON_NAMES: [(&[u8], &[u8]); 1] = [(b"t3GetNamedArg", b"namedArg")];
        let hexicon_name = HEXICON_NAMES
            .iter()
            .find(|(old, _)| *old == name)
            .map(|(_, new)| *new);
        // Found by neither, the message names the TADS 3 name, as the
        // reference's does.
        let found = std::iter::once(name)
            .chain(hexicon_name)
            .find_map(|have| Some((have, self.prog.globals.get(have).copied()?)));
        let (name, global) = match found {
            Some((have, global)) => (have, Some(global)),
            None => (name, None),
        };
        match global {
            None => self.error(format!(
                "{what} require support function \"{}\", which is undefined",
                show_name(name)
            )),
            Some(Global::Func(func)) => {
                let func = Expr::constant(Value::FuncPtr(func));
                self.gen_call(&func, false, argc, false, None);
            }
            Some(Global::Obj(_)) => {}
            Some(_) => self.gen_call(&Expr::Sym(name.to_vec()), false, argc, false, None),
        }
    }

    /// Generate a parameter's default value. The names in it are
    /// looked up now, but it is not folded, as other expressions are.
    fn gen_defval(&mut self, id: LocalId) {
        let Some(expr) = self.prog.locals[id].defval.clone() else {
            return;
        };
        self.resolve_only = true;
        let expr = self.fold_expr(expr);
        self.resolve_only = false;
        // So a call in it to a function that returns nothing is still
        // known for one, as it would not be elsewhere, where the
        // function's name has become a constant.
        self.in_defval = true;
        self.gen_expr(&expr, false, false);
        self.in_defval = false;
    }

    /// Fill in an optional parameter: the argument if there is one,
    /// and otherwise its default value, or nothing, which leaves it
    /// nil.
    fn gen_opt_positional(&mut self, id: LocalId) {
        let number = self.prog.locals[id].param_index as i32 + 1;
        let has_defval = self.prog.locals[id].defval.is_some();
        self.gen_int(number);
        self.cg.write_op(Op::GetArgc);
        self.cg.note_push();
        let not_found = self.cg.new_label();
        self.cg.write_jump(Op::Jgt, not_found);
        self.cg.note_pops(2);
        self.gen_int(number);
        self.call_internal_bif(b"getArg", "optional parameters", 1);
        if has_defval {
            let assign = self.cg.new_label();
            self.cg.write_jump(Op::Jmp, assign);
            self.cg.def_label(not_found);
            self.gen_defval(id);
            self.cg.def_label(assign);
            self.cg.note_pop();
        }
        self.gen_local_asi(id, true, 1, AsiOp::Simple, None, false);
        if !has_defval {
            self.cg.def_label(not_found);
        }
    }

    /// Fill in a parameter passed by name.
    fn gen_named_param(&mut self, id: LocalId) {
        const WHAT: &str = "named parameters";
        let local = &self.prog.locals[id];
        let name = local.name.clone();
        if local.defval.is_none() {
            // Without the argument, an optional one is nil, and one
            // that is not optional is an error when the program runs.
            let mut argc = 1;
            if local.is_opt_param {
                self.cg.write_op(Op::PushNil);
                self.cg.note_push();
                argc += 1;
            }
            self.gen_str(&name);
            self.call_internal_bif(b"t3GetNamedArg", WHAT, argc);
        } else if local.defval_is_const {
            self.gen_defval(id);
            self.gen_str(&name);
            self.call_internal_bif(b"t3GetNamedArg", WHAT, 2);
        } else {
            // The default value is only worked out if it is needed:
            // ask for the argument, and catch the error if it is not
            // there.
            let start_ofs = self.rel_ofs();
            self.gen_str(&name);
            self.call_internal_bif(b"t3GetNamedArg", WHAT, 1);
            let end_ofs = self.rel_ofs() - 1;
            let assign = self.gen_jump_ahead(Op::Jmp);
            self.cg.note_pop();
            self.exc_table.push(ExcEntry {
                start_ofs,
                end_ofs,
                class: 0,
                handler_ofs: self.rel_ofs(),
            });
            self.cg.note_push();
            self.cg.write_op(Op::Disc);
            self.cg.note_pop();
            self.gen_defval(id);
            self.def_label_pos(assign);
        }
        self.gen_local_asi(id, true, 1, AsiOp::Simple, None, false);
    }

    fn gen_call(
        &mut self,
        func: &Expr,
        discard: bool,
        argc: usize,
        varargs: bool,
        named: Option<usize>,
    ) {
        match func {
            Expr::Const(ConstVal {
                value: Value::FuncPtr(id),
                ..
            }) => {
                let mut id = *id;
                if !self.prog.funcs[id].has_proto {
                    let name = show_name(&self.prog.funcs[id].name);
                    self.error(format!(
                        "cannot call extern function {name} without an argument list definition"
                    ));
                    return;
                }
                // A call to a multi-method is a call to the library,
                // with the arguments as a list and the multi-method.
                // (With arguments spread from a list, the reference
                // leaves out the instruction that makes the list.)
                let (mut argc, mut varargs) = (argc, varargs);
                if self.prog.funcs[id].is_mm_base {
                    if varargs {
                        self.cg.write_op(Op::VarArgc);
                    }
                    if argc <= 255 {
                        self.cg.write_op(Op::New1);
                        self.cg.write(argc as u8);
                        self.cg.write(METACLASS_LIST);
                    } else {
                        self.cg.write_op(Op::New2);
                        self.cg.write2(argc as u16);
                        self.cg.write2(u16::from(METACLASS_LIST));
                    }
                    self.cg.note_pops(argc as i32);
                    self.cg.write_op(Op::GetR0);
                    self.cg.note_push();
                    self.cg.write_op(Op::PushFnPtr);
                    self.gen_func_ref(id);
                    self.cg.note_push();
                    let Some(call) = self.internal_func(b"_multiMethodCall", 1, true) else {
                        return;
                    };
                    (id, argc, varargs) = (call, 2, false);
                }
                if varargs {
                    self.cg.write_op(Op::VarArgc);
                }
                self.cg.write_op(Op::Call);
                self.cg.write(argc as u8);
                self.gen_func_ref(id);
                self.cg.note_pops(argc as i32);
                self.post_call_cleanup(named);
                let func = &self.prog.funcs[id];
                if !func.argc_ok(argc) {
                    let message = format!(
                        "wrong number of arguments to function \"{}\": {} required, {} actual",
                        show_name(&func.name),
                        func.argc_desc(),
                        argc
                    );
                    self.error(message);
                }
                if !discard {
                    self.cg.write_op(Op::GetR0);
                    self.cg.note_push();
                }
            }
            Expr::Replaced => {
                let (available, base, (min, opt, more)) = self.replaced_func();
                if !available {
                    self.error("'replaced' is not available in this context");
                }
                if varargs {
                    self.cg.write_op(Op::VarArgc);
                }
                self.cg.write_op(Op::Call);
                self.cg.write(argc as u8);
                if let Some(target) = self.replaced_target(base) {
                    self.out.fixups.push(Fixup {
                        ofs: self.cg.ofs(),
                        target,
                    });
                }
                self.cg.write4(0);
                self.cg.note_pops(argc as i32);
                self.post_call_cleanup(named);
                // The function as it was takes what the function as it
                // is takes; it has no name to give in the message.
                if available && (argc < min || (!more && argc > min + opt)) {
                    let desc = if more {
                        format!("{min}+")
                    } else if opt != 0 {
                        format!("{min}-{}", min + opt)
                    } else {
                        min.to_string()
                    };
                    self.error(format!(
                        "wrong number of arguments to function \"\": {desc} required, {argc} actual"
                    ));
                }
                if !discard {
                    self.cg.write_op(Op::GetR0);
                    self.cg.note_push();
                }
            }
            Expr::Const(_) => {
                self.error("this constant expression cannot be called as a function");
            }
            Expr::Sym(name) => match self.implied_global(name) {
                Some(Global::Bif(bif)) => self.gen_bif_call(bif, discard, argc, varargs, named),
                Some(Global::Prop(prop)) => {
                    // A property of `self`, as a method.
                    if !self.self_available {
                        self.prop_needs_obj(prop);
                        return;
                    }
                    let id = self.prog.props[prop].id;
                    if argc == 0 {
                        self.cg.write_op(Op::GetPropSelf);
                    } else {
                        if varargs {
                            self.cg.write_op(Op::VarArgc);
                        }
                        self.cg.write_op(Op::CallPropSelf);
                        self.cg.write(argc as u8);
                    }
                    self.write_prop_id(id);
                    self.cg.note_pops(argc as i32);
                    self.post_call_cleanup(named);
                    if !discard {
                        self.cg.write_op(Op::GetR0);
                        self.cg.note_push();
                    }
                }
                // An object that can be called, such as a function
                // compiled while the program runs.
                Some(Global::Obj(obj)) if self.is_func_obj(obj) => {
                    if varargs {
                        self.cg.write_op(Op::VarArgc);
                    }
                    self.cg.write_op(Op::PushObj);
                    self.write_obj_id(self.prog.objs[obj].id);
                    self.cg.note_push();
                    self.cg.write_op(Op::PtrCall);
                    self.cg.write(argc as u8);
                    self.cg.note_pops(argc as i32 + 1);
                    self.post_call_cleanup(named);
                    if !discard {
                        self.cg.write_op(Op::GetR0);
                        self.cg.note_push();
                    }
                }
                Some(Global::Meta(_) | Global::DynLocal(_)) => {
                    self.error(format!(
                        "cannot call \"{}\" as a method or function",
                        show_name(name)
                    ));
                }
                _ => {}
            },
            _ => {
                // Anything else is taken to be a pointer to a function.
                self.gen_expr(func, false, false);
                if varargs {
                    self.cg.write_op(Op::Swap);
                    self.cg.write_op(Op::VarArgc);
                }
                self.cg.write_op(Op::PtrCall);
                self.cg.write(argc as u8);
                self.cg.note_pops(argc as i32 + 1);
                self.post_call_cleanup(named);
                if !discard {
                    self.cg.write_op(Op::GetR0);
                    self.cg.note_push();
                }
            }
        }
    }

    /// Call a function built into the virtual machine.
    fn gen_bif_call(
        &mut self,
        bif: usize,
        discard: bool,
        argc: usize,
        varargs: bool,
        named: Option<usize>,
    ) {
        let BifSym {
            name,
            set,
            index,
            min_argc,
            max_argc,
            varargs: takes_more,
        } = self.prog.bifs[bif].clone();
        if argc < min_argc {
            self.error(format!(
                "too few arguments to function \"{}\"",
                show_name(&name)
            ));
        } else if !takes_more && argc > max_argc {
            self.error(format!(
                "too many arguments to function \"{}\"",
                show_name(&name)
            ));
        }
        if varargs {
            self.cg.write_op(Op::VarArgc);
        }
        // The first four sets have short instructions of their own.
        if set < 4 && index < 256 {
            let opc = [Op::BuiltinA, Op::BuiltinB, Op::BuiltinC, Op::BuiltinD][set];
            self.cg.write_op(opc);
            self.cg.write(argc as u8);
            self.cg.write(index as u8);
        } else {
            if index < 256 {
                self.cg.write_op(Op::Builtin1);
                self.cg.write(argc as u8);
                self.cg.write(index as u8);
            } else {
                self.cg.write_op(Op::Builtin2);
                self.cg.write(argc as u8);
                self.cg.write2(index as u16);
            }
            self.cg.write(set as u8);
        }
        self.cg.note_pops(argc as i32);
        self.post_call_cleanup(named);
        if !discard {
            self.cg.write_op(Op::GetR0);
            self.cg.note_push();
        }
    }

    // ---- locals ----

    fn gen_local(&mut self, id: LocalId, discard: bool) {
        if !discard {
            let local = &self.prog.locals[id];
            if local.is_ctx_local {
                // It is in a context: an element of a vector.
                let var = local.ctx_var_num;
                let index = self.prog.ctx_arr_idx(id);
                if var <= 255 && index <= 255 {
                    self.cg.write_op(Op::IdxLcl1Int8);
                    self.cg.write(var as u8);
                    self.cg.write(index as u8);
                    self.cg.note_push();
                } else {
                    self.gen_getlcl(var, false);
                    self.gen_int(index as i32);
                    self.cg.write_op(Op::Index);
                    self.cg.note_pop();
                }
            } else {
                self.gen_getlcl(local.var_num, local.is_param);
            }
        }
        // The variable has been used, whether or not its value was
        // wanted here.
        self.prog.set_val_used(id);
    }

    /// Store the value on the stack in a local, wherever it lives.
    fn gen_setlcl(&mut self, id: LocalId) {
        let local = &self.prog.locals[id];
        if !local.is_ctx_local {
            self.gen_setlcl_stk(local.var_num, local.is_param);
            return;
        }
        let var = local.ctx_var_num;
        let index = self.prog.ctx_arr_idx(id);
        if var <= 255 && index <= 255 {
            self.cg.write_op(Op::SetIndLcl1I8);
            self.cg.write(var as u8);
            self.cg.write(index as u8);
            self.cg.note_pop();
        } else {
            self.gen_getlcl(var, false);
            self.gen_int(index as i32);
            self.cg.write_op(Op::SetInd);
            self.cg.write_op(Op::Disc);
            self.cg.note_pops(3);
        }
    }

    fn gen_getlcl(&mut self, var_num: usize, is_param: bool) {
        if is_param && var_num < 4 {
            let opc = [Op::GetArgN0, Op::GetArgN1, Op::GetArgN2, Op::GetArgN3][var_num];
            self.cg.write_op(opc);
        } else if !is_param && var_num < 6 {
            let opc = [
                Op::GetLclN0,
                Op::GetLclN1,
                Op::GetLclN2,
                Op::GetLclN3,
                Op::GetLclN4,
                Op::GetLclN5,
            ][var_num];
            self.cg.write_op(opc);
        } else if var_num <= 255 {
            self.cg
                .write_op(if is_param { Op::GetArg1 } else { Op::GetLcl1 });
            self.cg.write(var_num as u8);
        } else {
            self.cg
                .write_op(if is_param { Op::GetArg2 } else { Op::GetLcl2 });
            self.cg.write2(var_num as u16);
        }
        self.cg.note_push();
    }

    fn gen_setlcl_stk(&mut self, var_num: usize, is_param: bool) {
        if var_num <= 255 {
            self.cg
                .write_op(if is_param { Op::SetArg1 } else { Op::SetLcl1 });
            self.cg.write(var_num as u8);
        } else {
            self.cg
                .write_op(if is_param { Op::SetArg2 } else { Op::SetLcl2 });
            self.cg.write2(var_num as u16);
        }
        self.cg.note_pop();
    }

    /// Generate an assignment to an expression. In phase 1, the target
    /// may do the whole job with instructions of its own, and says so
    /// by returning true; otherwise it leaves its present value on the
    /// stack, the caller computes the new value, and phase 2 stores it.
    #[allow(clippy::too_many_arguments)]
    fn gen_asi(
        &mut self,
        target: &Expr,
        discard: bool,
        phase: u8,
        typ: AsiOp,
        rhs: Option<&Expr>,
        ignore_error: bool,
        explicit: bool,
    ) -> bool {
        match target {
            Expr::Local(id) => self.gen_local_asi(*id, discard, phase, typ, rhs, explicit),
            Expr::Index(left, index) => {
                self.gen_index_asi(left, index, discard, phase, typ, rhs, explicit)
            }
            Expr::Sym(name) => {
                // A name that is nothing yet is taken to be a property
                // of `self`, where there is a `self`.
                let known = self.prog.globals.get(name).copied();
                if known.is_none() && !self.self_available {
                    if ignore_error || phase > 1 {
                        return false;
                    }
                    self.undefined(name);
                    return false;
                }
                if let Some(Global::DynLocal(local)) = known {
                    return self.gen_dyn_local_asi(local, discard, phase, typ, rhs);
                }
                let Some(Global::Prop(prop)) = self.implied_global(name) else {
                    if ignore_error || phase > 1 {
                        return false;
                    }
                    self.error(format!("cannot assign to \"{}\"", show_name(name)));
                    return true;
                };
                if !self.self_available {
                    if phase == 1 {
                        self.error(format!(
                            "cannot assign to property \"{}\" without \"object.\" prefix - \
                             no \"self\"",
                            show_name(name)
                        ));
                    }
                    return true;
                }
                let id = self.prog.props[prop].id;
                if typ == AsiOp::Simple || phase == 2 {
                    if let (AsiOp::Simple, Some(rhs)) = (typ, rhs) {
                        self.gen_expr(rhs, false, false);
                    }
                    if !discard {
                        self.cg.write_op(Op::Dup);
                        self.cg.note_push();
                    }
                    self.cg.write_op(Op::SetPropSelf);
                    self.write_prop_id(id);
                    self.cg.note_pop();
                    true
                } else {
                    self.gen_prop_self(prop, false);
                    false
                }
            }
            Expr::Member {
                obj,
                prop,
                prop_is_expr,
                args: None,
            } => self.gen_member_asi(obj, prop, *prop_is_expr, discard, phase, typ, rhs),
            _ => {
                if !ignore_error && phase == 1 {
                    let message = if matches!(
                        typ,
                        AsiOp::PreInc | AsiOp::PreDec | AsiOp::PostInc | AsiOp::PostDec
                    ) {
                        format!(
                            "invalid lvalue - cannot apply \"{}\" operator to expression",
                            typ.text()
                        )
                    } else {
                        format!(
                            "invalid lvalue - cannot assign to expression on left of \"{}\"",
                            typ.text()
                        )
                    };
                    self.error(message);
                }
                false
            }
        }
    }

    /// Assign to an element: `left[index] = value`. Storing into an
    /// element gives a new container, which is then stored back into
    /// `left` if `left` can be assigned to.
    #[allow(clippy::too_many_arguments)]
    fn gen_index_asi(
        &mut self,
        left: &Expr,
        index: &Expr,
        discard: bool,
        phase: u8,
        typ: AsiOp,
        rhs: Option<&Expr>,
        explicit: bool,
    ) -> bool {
        if typ == AsiOp::Simple {
            if let Some(rhs) = rhs {
                self.gen_expr(rhs, false, false);
            }
            if !discard {
                self.cg.write_op(Op::Dup);
                self.cg.note_push();
            }
            // Have the container left on the stack. What is being
            // indexed may itself be an element of something, and then
            // there is more on the stack than the container.
            let rhs_depth = self.cg.sp_depth();
            self.gen_asi(left, false, 1, AsiOp::Idx, None, true, explicit);
            let mut extra_disc = 0;
            if self.cg.sp_depth() == rhs_depth {
                self.gen_expr(left, false, false);
            } else if self.cg.sp_depth() > rhs_depth + 1 {
                let dist = self.cg.sp_depth() - rhs_depth;
                if dist > 255 {
                    self.error("expression too complex");
                }
                self.cg.write_op(Op::GetSpN);
                self.cg.write(dist as u8);
                self.cg.note_push();
                self.cg.write_op(Op::Swap);
                extra_disc += 1;
            }
            self.gen_expr(index, false, false);
            self.cg.write_op(Op::SetInd);
            self.cg.note_pops(2);
            if self.cg.last_op() == Op::SetIndLcl1I8 as u8 {
                // The instruction writer made it one instruction that
                // stores back into the local too.
                self.cg.note_pop();
            } else if !self.gen_asi(left, true, 2, AsiOp::Idx, None, true, explicit) {
                self.cg.write_op(Op::Disc);
                self.cg.note_pop();
            }
            for _ in 0..extra_disc {
                self.cg.write_op(Op::Disc);
                self.cg.note_pop();
            }
            true
        } else if phase == 1 {
            // Leave the container, the index, and the element's present
            // value for the caller to work on.
            self.gen_asi(left, false, 1, AsiOp::Idx, None, true, explicit);
            self.gen_expr(index, false, false);
            self.cg.write_op(Op::Dup2);
            self.cg.note_pushes(2);
            self.cg.write_op(Op::Index);
            self.cg.note_pop();
            false
        } else if phase == 2 {
            if !discard {
                self.cg.write_op(Op::Dup);
                self.cg.note_push();
            }
            // Bring the container and the index back above the value.
            if discard {
                self.cg.write_op(Op::SwapN);
                self.cg.write(0);
                self.cg.write(2);
                self.cg.write_op(Op::Swap);
            } else {
                self.cg.write_op(Op::Swap2);
            }
            self.cg.write_op(Op::SetInd);
            self.cg.note_pops(2);
            if !self.gen_asi(left, true, 2, AsiOp::Idx, None, true, explicit) {
                self.cg.write_op(Op::Disc);
                self.cg.note_pop();
            }
            true
        } else {
            false
        }
    }

    fn gen_local_asi(
        &mut self,
        id: LocalId,
        discard: bool,
        phase: u8,
        typ: AsiOp,
        rhs: Option<&Expr>,
        explicit: bool,
    ) -> bool {
        if explicit {
            self.prog.set_val_assigned(id);
        }
        if typ != AsiOp::Simple {
            self.prog.set_val_used(id);
        }
        let var_num = self.prog.locals[id].var_num;
        let is_param = self.prog.locals[id].is_param;
        let is_ctx_local = self.prog.locals[id].is_ctx_local;
        // A local in a context has no instructions of its own: its
        // value is fetched, worked on, and stored.
        if is_ctx_local && typ != AsiOp::Simple && phase == 1 {
            self.gen_local(id, false);
            return false;
        }
        // The second phase of any assignment is a plain store.
        let typ = if phase == 2 { AsiOp::Simple } else { typ };
        match typ {
            AsiOp::Simple => {
                // Setting a local to nil, zero or one has instructions
                // of its own.
                let short = if is_param || is_ctx_local {
                    None
                } else {
                    match rhs.and_then(Expr::as_const).map(|value| &value.value) {
                        Some(Value::Nil) => Some((Op::NilLcl1, Op::NilLcl2, Op::PushNil)),
                        Some(Value::Int(0)) => Some((Op::ZeroLcl1, Op::ZeroLcl2, Op::Push0)),
                        Some(Value::Int(1)) => Some((Op::OneLcl1, Op::OneLcl2, Op::Push1)),
                        _ => None,
                    }
                };
                if let Some((op1, op2, push)) = short {
                    if var_num <= 255 {
                        self.cg.write_op(op1);
                        self.cg.write(var_num as u8);
                    } else {
                        self.cg.write_op(op2);
                        self.cg.write2(var_num as u16);
                    }
                    if !discard {
                        self.cg.write_op(push);
                        self.cg.note_push();
                    }
                    return true;
                }
                if let Some(rhs) = rhs {
                    self.gen_expr(rhs, false, false);
                }
                if !discard {
                    self.cg.write_op(Op::Dup);
                    self.cg.note_push();
                }
                self.gen_setlcl(id);
                true
            }
            AsiOp::Add | AsiOp::Sub => {
                if is_param {
                    if phase == 1 {
                        self.gen_local(id, false);
                    }
                    return false;
                }
                let adding = typ == AsiOp::Add;
                let const_int = rhs
                    .and_then(Expr::as_const)
                    .and_then(|value| match value.value {
                        Value::Int(int) => Some(int),
                        _ => None,
                    })
                    .filter(|_| adding);
                if let Some(int) = const_int {
                    if int == 1 {
                        self.cg.write_op(Op::IncLcl);
                        self.cg.write2(var_num as u16);
                    } else if int == -1 {
                        self.cg.write_op(Op::DecLcl);
                        self.cg.write2(var_num as u16);
                    } else if (-128..=127).contains(&int) && var_num <= 255 {
                        self.cg.write_op(Op::AddILcl1);
                        self.cg.write(var_num as u8);
                        self.cg.write(int as u8);
                    } else {
                        self.cg.write_op(Op::AddILcl4);
                        self.cg.write2(var_num as u16);
                        self.cg.write4(int as u32);
                    }
                } else {
                    if let Some(rhs) = rhs {
                        self.gen_expr(rhs, false, false);
                    }
                    self.cg
                        .write_op(if adding { Op::AddToLcl } else { Op::SubFromLcl });
                    self.cg.write2(var_num as u16);
                    self.cg.note_pop();
                }
                if !discard {
                    self.gen_local(id, false);
                }
                true
            }
            AsiOp::PreInc | AsiOp::PreDec | AsiOp::PostInc | AsiOp::PostDec => {
                if is_param {
                    if phase == 1 {
                        self.gen_local(id, false);
                    }
                    return false;
                }
                let post = matches!(typ, AsiOp::PostInc | AsiOp::PostDec);
                let opc = if matches!(typ, AsiOp::PreInc | AsiOp::PostInc) {
                    Op::IncLcl
                } else {
                    Op::DecLcl
                };
                if post && !discard {
                    self.gen_local(id, false);
                }
                self.cg.write_op(opc);
                self.cg.write2(var_num as u16);
                if !post && !discard {
                    self.gen_local(id, false);
                }
                true
            }
            _ => {
                // The other kinds have nothing special: leave the value
                // for the caller to work on.
                self.gen_local(id, false);
                false
            }
        }
    }
}
