//! The parser: tokens in, a syntax tree and the program's symbols out.
//!
//! It follows the reference compiler's parser routine by routine,
//! including how it carries on after an error, so that the same source
//! gives the same tree. So far it reads functions: their parameters,
//! local variables, the statements that need no objects, and
//! expressions over numbers, strings, locals and function calls.

use std::collections::HashMap;

use crate::ast::{
    Arg, AsiOp, BinOp, Catch, Closure, CodeBody, Enclosure, Expr, Finally, ForIn, LabelSym,
    LocalId, Stmt, StmtKind, TabId, UnOp, Value, flow,
};
use crate::flow::Flow;
use crate::fold::{self, Folded};
use crate::lex::Tok;
use crate::pp::{Preprocessor, Severity, SrcToken, show_name};

/// A local variable or a parameter.
#[derive(Debug, Clone, Default)]
pub(crate) struct Local {
    pub name: Vec<u8>,
    pub is_param: bool,
    /// Its slot: a parameter number or a local number.
    pub var_num: usize,
    /// It is the parameter that collects variable arguments in a list.
    pub is_list_param: bool,
    pub val_used: bool,
    pub val_assigned: bool,
    /// Where it is declared.
    pub file: usize,
    pub line: u32,
    /// It is shared with a nested function, and so lives in a context
    /// rather than on the stack.
    pub is_ctx_local: bool,
    /// For a nested function's view of a local of a function around
    /// it: the local it is a view of.
    pub ctx_orig: Option<LocalId>,
    /// The local that holds the context it lives in, and whether that
    /// has been settled.
    pub ctx_var_num: usize,
    pub ctx_var_num_set: bool,
    /// How many functions out the local is.
    pub ctx_level: usize,
    /// Its place in its context. A view has its original's.
    pub ctx_arr_idx: usize,
    /// The name has been looked up.
    pub referenced: bool,
    /// It is a parameter passed by name (`f(count:)`). Such a parameter
    /// and an optional one are locals, which the function fills in as
    /// it starts.
    pub is_named_param: bool,
    /// It is a parameter that can be left out (`f(count?)`,
    /// `f(count = 1)`).
    pub is_opt_param: bool,
    /// For a parameter passed by position, its place in the list, from
    /// zero.
    pub param_index: usize,
    /// The value an optional parameter has when it is left out, and
    /// whether that was a constant as written.
    pub defval: Option<Expr>,
    pub defval_is_const: bool,
    /// Where its default value comes among the default values of the
    /// list.
    pub defval_seqno: usize,
}

/// The locals of one scope.
#[derive(Debug, Clone, Default)]
pub(crate) struct LocalTab {
    pub parent: Option<TabId>,
    /// The scope's locals, in the order they were declared.
    pub entries: Vec<LocalId>,
    /// The part of the function's code the scope covers, as offsets from
    /// the start of the function. Filled in as code is generated.
    pub start_ofs: usize,
    pub end_ofs: usize,
    /// The scope's number in the function's list of scopes, from one,
    /// once it is in the list.
    pub list_index: usize,
}

/// How many chains the reference's table of a scope's locals has. The
/// order locals are written to the image in comes from it.
const LOCAL_HASH_SIZE: usize = 32;

/// A scope's locals in the order the reference goes through them: by
/// the sum of the name's bytes modulo the table size, and within one
/// sum, the latest declared first.
pub(crate) fn enum_order(tab: &LocalTab, locals: &[Local]) -> Vec<LocalId> {
    let bucket = |id: &LocalId| {
        locals[*id]
            .name
            .iter()
            .map(|&byte| usize::from(byte))
            .sum::<usize>()
            % LOCAL_HASH_SIZE
    };
    let mut order: Vec<LocalId> = tab.entries.iter().rev().copied().collect();
    order.sort_by_key(bucket);
    order
}

/// A function.
#[derive(Debug, Clone)]
pub(crate) struct FuncSym {
    pub name: Vec<u8>,
    pub argc: usize,
    pub opt_argc: usize,
    pub varargs: bool,
    pub has_retval: bool,
    /// It is declared but not defined yet in what has been read.
    pub is_extern: bool,
    pub has_proto: bool,
    /// Its definition: an index into [`Program::bodies`].
    pub body: Option<usize>,
}

impl FuncSym {
    /// Whether a call with this many arguments is right.
    pub(crate) fn argc_ok(&self, argc: usize) -> bool {
        argc >= self.argc && (self.varargs || argc <= self.argc + self.opt_argc)
    }

    /// The number of arguments wanted, for a message.
    pub(crate) fn argc_desc(&self) -> String {
        if self.varargs {
            format!("{}+", self.argc)
        } else if self.opt_argc != 0 {
            format!("{}-{}", self.argc, self.argc + self.opt_argc)
        } else {
            self.argc.to_string()
        }
    }
}

/// A function built into the virtual machine.
#[derive(Debug, Clone)]
pub(crate) struct BifSym {
    pub name: Vec<u8>,
    /// Which set of functions it is in: an index into
    /// [`Program::fnsets`].
    pub set: usize,
    /// Its place in the set.
    pub index: usize,
    pub min_argc: usize,
    pub max_argc: usize,
    pub varargs: bool,
}

/// An object, as a name.
#[derive(Debug, Clone)]
pub(crate) struct ObjSym {
    pub name: Vec<u8>,
    /// The object's number in the program.
    pub id: u32,
    /// It is known of but not defined yet in what has been read.
    pub is_extern: bool,
    pub is_class: bool,
    /// The names of its superclasses.
    pub sc_names: Vec<Vec<u8>>,
}

/// A class built into the virtual machine, as the name the program
/// gives it.
#[derive(Debug, Clone)]
pub(crate) struct MetaSym {
    pub name: Vec<u8>,
    /// The number of the object that stands for the class.
    pub obj_id: u32,
    /// Which built-in class it is: an index into
    /// [`Program::meta_table`].
    pub meta: usize,
    /// The properties the class has, in the order it declares them:
    /// indexes into [`Program::props`].
    pub props: Vec<usize>,
    /// It is known of from the reading before, and not declared yet in
    /// this one.
    pub is_ext: bool,
    /// When the name was first met: see [`PropSym::seq`].
    pub seq: u32,
}

/// A built-in class the image depends on.
#[derive(Debug, Clone)]
pub(crate) struct MetaEntry {
    /// The virtual machine's name for it, with its version:
    /// `list/030008`.
    pub name: Vec<u8>,
    /// The name the program declared it under: an index into
    /// [`Program::metas`].
    pub sym: Option<usize>,
}

/// The built-in classes every image depends on, whether it uses them
/// or not. One more, `intrinsic-class`, is added after whatever the
/// program declares.
const PREDEFINED_METACLASSES: [&str; 8] = [
    "tads-object/000000",
    "list/000000",
    "dictionary2/030000",
    "grammar-production/030000",
    "vector/000000",
    "anon-func-ptr/000000",
    "int-class-mod/030000",
    "lookuptable/030003",
];

/// A property, as a name.
#[derive(Debug, Clone)]
pub(crate) struct PropSym {
    pub name: Vec<u8>,
    /// The property's number while the program is compiled. The number
    /// it has in the image is given when everything is known: see
    /// [`Program::final_prop_ids`].
    pub id: u16,
    /// The name is a property's only because its address was taken
    /// (`&name`) when it was nothing else. A function defined with the
    /// name afterward takes the name over.
    pub weak: bool,
    /// When the name was first met, among the properties and built-in
    /// classes: a count that goes on from [`SECOND_READING`] for those
    /// first met on the second reading of the source.
    pub seq: u32,
}

/// Where the count of names first met on the second reading starts.
pub(crate) const SECOND_READING: u32 = 1 << 24;

/// The value an object definition gives a property.
#[derive(Debug)]
pub(crate) enum PropValue {
    Expr(Expr),
    /// A method: an index into [`Program::bodies`].
    Method(usize),
}

/// A property in an object definition.
#[derive(Debug)]
pub(crate) struct ObjProp {
    /// An index into [`Program::props`].
    pub prop: usize,
    pub value: PropValue,
    /// The compiler put it there, and the source can give the property
    /// a value of its own.
    pub overwritable: bool,
    /// The value is to be worked out once, before the program starts
    /// (`prop = static expr`).
    pub is_static: bool,
    pub file: usize,
    pub line: u32,
}

/// An object definition.
#[derive(Debug)]
pub(crate) struct ObjDef {
    /// The object defined: an index into [`Program::objs`]. There is
    /// none if the name could not be an object's.
    pub sym: Option<usize>,
    pub is_class: bool,
    pub superclasses: Vec<Vec<u8>>,
    pub props: Vec<ObjProp>,
    pub file: usize,
    pub line: u32,
}

/// A definition at the outermost level of the source that code or data
/// is generated for.
#[derive(Debug, Clone, Copy)]
pub(crate) enum Top {
    /// A function: an index into [`Program::bodies`].
    Func(usize),
    /// An object: an index into [`Program::objdefs`].
    Obj(usize),
}

/// What a name means at the outermost level of the program.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Global {
    /// An index into [`Program::funcs`].
    Func(usize),
    /// An index into [`Program::bifs`].
    Bif(usize),
    /// An index into [`Program::objs`].
    Obj(usize),
    /// An index into [`Program::props`].
    Prop(usize),
    /// An index into [`Program::metas`].
    Meta(usize),
    /// An index into [`Program::enums`].
    Enum(usize),
}

/// A value of an enumeration (`enum red, green;`).
#[derive(Debug, Clone)]
pub(crate) struct EnumSym {
    pub name: Vec<u8>,
    /// It was declared with `enum token`, as a kind of token for a
    /// grammar to match.
    pub is_token: bool,
    /// When the name was first met: see [`PropSym::seq`].
    pub seq: u32,
}

/// The properties every program has, in the order of their numbers,
/// which start at 1.
pub(crate) const SPECIAL_PROPS: [&str; 25] = [
    "construct",
    "finalize",
    "grammarInfo",
    "grammarTag",
    "miscVocab",
    "lexicalParent",
    "sourceTextOrder",
    "sourceTextGroup",
    "sourceTextGroupName",
    "sourceTextGroupOrder",
    "operator +",
    "operator -",
    "operator *",
    "operator /",
    "operator %",
    "operator ^",
    "operator <<",
    "operator >>",
    "operator >>>",
    "operator ~",
    "operator |",
    "operator &",
    "operator negate",
    "operator []",
    "operator []=",
];
/// Which of them is `sourceTextOrder`.
const SRC_ORDER_PROP: usize = 6;

/// How many chains the reference's table of global names has. A name's
/// chain is the sum of its bytes, and a chain has its newest name
/// first.
pub(crate) const GLOBAL_HASH_SIZE: usize = 512;

pub(crate) fn global_hash(name: &[u8]) -> usize {
    name.iter().map(|&byte| usize::from(byte)).sum::<usize>() % GLOBAL_HASH_SIZE
}

/// What one reading of the source passes on to the next: the names it
/// defined, in the order the reference writes them to its symbol file,
/// which is the order of its table of global names.
#[derive(Debug, Default)]
pub(crate) struct Exports {
    pub funcs: Vec<FuncSym>,
    pub objs: Vec<ObjSym>,
    /// The built-in classes declared, in the order they were, each with
    /// the virtual machine's name for it.
    pub metas: Vec<(MetaSym, Vec<u8>)>,
    /// Each with whether it is a property only for want of anything
    /// else (see [`PropSym::weak`]), and when it was first met.
    pub props: Vec<(Vec<u8>, bool, u32)>,
    pub enums: Vec<EnumSym>,
}

/// A program as the parser leaves it.
#[derive(Debug, Default)]
pub(crate) struct Program {
    pub enums: Vec<EnumSym>,
    pub locals: Vec<Local>,
    pub tabs: Vec<LocalTab>,
    pub funcs: Vec<FuncSym>,
    pub bifs: Vec<BifSym>,
    /// The sets of built-in functions the program uses, by name, in the
    /// order they were first declared.
    pub fnsets: Vec<Vec<u8>>,
    /// The number of elements in the longest list in the source.
    pub max_list_cnt: usize,
    pub globals: HashMap<Vec<u8>, Global>,
    /// The bodies of functions and methods, in the order the source has
    /// them, each with the function it defines if it is a function's.
    pub bodies: Vec<(CodeBody, Option<usize>)>,
    pub objs: Vec<ObjSym>,
    pub props: Vec<PropSym>,
    pub metas: Vec<MetaSym>,
    /// The built-in classes the image depends on, in the order the
    /// image lists them.
    pub meta_table: Vec<MetaEntry>,
    pub objdefs: Vec<ObjDef>,
    /// The functions and objects in the order the source defines them,
    /// which is the order they are generated in.
    pub top: Vec<Top>,
    /// The number the next object gets, and the next property.
    pub next_obj_id: u32,
    pub next_prop_id: u16,
    /// For each statement that jumps have to know about, by its
    /// number, the one it is directly inside of as the parser saw it.
    /// A `catch` block is outside its `try` there.
    pub enclosure_parents: HashMap<usize, Option<usize>>,
    /// The count that says when a property or a built-in class was
    /// first met.
    pub next_seq: u32,
    /// The names the program makes known to the virtual machine
    /// (`export`): each a name in the program, and the name the
    /// virtual machine knows it by.
    pub exports: Vec<(Vec<u8>, Vec<u8>)>,
    /// The bodies of anonymous functions, in the order they were
    /// finished. They are generated after everything else.
    pub nested: Vec<usize>,
    /// What bodies know of the bodies around them.
    pub body_refs: Vec<BodyRef>,
    /// The labels of the last function or method read.
    pub last_labels: std::rc::Rc<Vec<LabelSym>>,
}

/// What is known of a body that others are nested in.
#[derive(Debug, Clone, Default)]
pub(crate) struct BodyRef {
    pub enclosing: Option<usize>,
    pub local_ctx_needs_full: bool,
}

impl Program {
    /// Note that a local's value is used: for a nested function's view
    /// of a local, that the local's is.
    pub(crate) fn set_val_used(&mut self, mut id: LocalId) {
        loop {
            self.locals[id].val_used = true;
            match self.locals[id].ctx_orig {
                Some(orig) => id = orig,
                None => break,
            }
        }
    }

    pub(crate) fn set_val_assigned(&mut self, mut id: LocalId) {
        loop {
            self.locals[id].val_assigned = true;
            match self.locals[id].ctx_orig {
                Some(orig) => id = orig,
                None => break,
            }
        }
    }

    /// A shared local's place in its context.
    pub(crate) fn ctx_arr_idx(&self, mut id: LocalId) -> usize {
        while let Some(orig) = self.locals[id].ctx_orig {
            id = orig;
        }
        self.locals[id].ctx_arr_idx
    }

    /// Make a local one that lives in a context, because a nested
    /// function uses it.
    fn convert_to_ctx_var(&mut self, mut id: LocalId, used: bool, assigned: bool) {
        loop {
            let local = &mut self.locals[id];
            if !local.is_ctx_local {
                local.is_ctx_local = true;
                local.ctx_var_num = 0;
                local.ctx_arr_idx = 0;
            }
            local.referenced = true;
            if used {
                self.set_val_used(id);
            }
            if assigned {
                self.set_val_assigned(id);
            }
            match self.locals[id].ctx_orig {
                Some(orig) => id = orig,
                None => break,
            }
        }
    }

    /// Whether an expression leaves a value. A call to a function that
    /// is known not to return one does not, nor does a string that
    /// displays itself.
    pub(crate) fn has_return_value(&self, expr: &Expr) -> bool {
        match expr {
            Expr::Call(func, _) => match &**func {
                Expr::Sym(name) => match self.globals.get(name) {
                    Some(Global::Func(func)) => self.funcs[*func].has_retval,
                    _ => true,
                },
                Expr::AnonFunc { has_retval, .. } => *has_retval,
                _ => true,
            },
            Expr::Binary(BinOp::Comma, _, right) => self.has_return_value(right),
            Expr::If(_, second, third) => {
                self.has_return_value(second) || self.has_return_value(third)
            }
            Expr::IfNil(first, second) => {
                self.has_return_value(first) || self.has_return_value(second)
            }
            Expr::DStr(_) | Expr::DStrEmbed(_) => false,
            Expr::OneOf(dbl, _) => !dbl,
            _ => true,
        }
    }

    /// The names as another reading of the same source should see them
    /// before it starts: known, but not yet defined. A function or
    /// object that was only ever declared is not passed on.
    pub(crate) fn exports(&self) -> Exports {
        let funcs = self
            .funcs
            .iter()
            .filter(|func| !func.is_extern)
            .map(|func| FuncSym {
                is_extern: true,
                body: None,
                ..func.clone()
            })
            .collect();
        // Objects are numbered in the order of the table of global
        // names as it is when the program is put together; in a chain
        // of that table, they are in the order they were defined.
        let mut objs: Vec<ObjSym> = self
            .objs
            .iter()
            .filter(|obj| !obj.is_extern && self.globals.contains_key(&obj.name))
            .map(|obj| ObjSym {
                is_extern: true,
                ..obj.clone()
            })
            .collect();
        objs.sort_by_key(|obj| global_hash(&obj.name));
        let metas = self
            .metas
            .iter()
            .map(|meta| {
                let sym = MetaSym {
                    is_ext: true,
                    props: Vec::new(),
                    ..meta.clone()
                };
                (sym, self.meta_table[meta.meta].name.clone())
            })
            .collect();
        let props = self
            .props
            .iter()
            .enumerate()
            .skip(SPECIAL_PROPS.len())
            .filter(|(index, prop)| self.globals.get(&prop.name) == Some(&Global::Prop(*index)))
            .map(|(_, prop)| (prop.name.clone(), prop.weak, prop.seq))
            .collect();
        Exports {
            funcs,
            objs,
            metas,
            props,
            enums: self.enums.clone(),
        }
    }

    /// The number each value of an enumeration has in the image. Like
    /// properties, they are numbered when the program is put together,
    /// from one, in the order of the table of global names: see
    /// [`Program::final_prop_ids`].
    pub(crate) fn final_enum_ids(&self) -> Vec<u32> {
        let mut order: Vec<usize> = (0..self.enums.len()).collect();
        order.sort_by_key(|&index| {
            let sym = &self.enums[index];
            let new = sym.seq >= SECOND_READING;
            let within = if new { u32::MAX - sym.seq } else { sym.seq };
            (global_hash(&sym.name), !new, within)
        });
        let mut ids = vec![0; self.enums.len()];
        for (place, index) in order.into_iter().enumerate() {
            ids[index] = place as u32 + 1;
        }
        ids
    }

    /// The number each property has in the image, by the number it had
    /// while compiling. The reference numbers properties when it puts
    /// the program together, in the order of its table of global names
    /// then: chain by chain, and in a chain the names first met on the
    /// last reading of the source, newest first, and then those of the
    /// reading before, oldest first. A built-in class's name there
    /// brings the properties the class declares, in the order it
    /// declares them. The properties every program has keep their
    /// numbers.
    ///
    /// A name that stopped being a property gets no number. The second
    /// value is the number the next property would get.
    pub(crate) fn final_prop_ids(&self) -> (Vec<u16>, u16) {
        let special = SPECIAL_PROPS.len();
        let is_prop = |index: usize| {
            index >= special
                && self.globals.get(&self.props[index].name) == Some(&Global::Prop(index))
        };
        // The names in the table's order: properties, and built-in
        // classes.
        let key = |name: &[u8], seq: u32| {
            let new = seq >= SECOND_READING;
            let within = if new { u32::MAX - seq } else { seq };
            (global_hash(name), !new, within)
        };
        // Each name's place in the table, and the property or the
        // built-in class it is.
        type Placed = ((usize, bool, u32), Option<usize>, Option<usize>);
        let mut names: Vec<Placed> = (special..self.props.len())
            .filter(|index| is_prop(*index))
            .map(|index| {
                let prop = &self.props[index];
                (key(&prop.name, prop.seq), Some(index), None)
            })
            .chain(
                self.metas
                    .iter()
                    .enumerate()
                    .map(|(index, meta)| (key(&meta.name, meta.seq), None, Some(index))),
            )
            .collect();
        names.sort_unstable();
        let mut order: Vec<usize> = Vec::new();
        let mut placed = vec![false; self.props.len()];
        for (_, prop, meta) in names {
            let props: &[usize] = match (&prop, meta) {
                (Some(prop), _) => std::slice::from_ref(prop),
                (None, Some(meta)) => &self.metas[meta].props,
                (None, None) => &[],
            };
            for &index in props {
                if is_prop(index) && !placed[index] {
                    placed[index] = true;
                    order.push(index);
                }
            }
        }
        // Indexed by the compile-time number, which starts at 1.
        let mut ids: Vec<u16> = (0..=special as u16).collect();
        ids.resize(self.props.len() + 1, 0);
        let next = (special + 1 + order.len()) as u16;
        for (place, index) in order.into_iter().enumerate() {
            ids[usize::from(self.props[index].id)] = (special + 1 + place) as u16;
        }
        (ids, next)
    }

    /// Find a built-in class in the table of them by the virtual
    /// machine's name for it, or add it. The name has a version after a
    /// slash; of two versions, the table keeps the later.
    pub(crate) fn find_or_add_meta(&mut self, name: &[u8], sym: Option<usize>) -> usize {
        let split = |name: &[u8]| {
            let slash = name.iter().position(|&byte| byte == b'/');
            (slash.unwrap_or(name.len()), slash.map(|at| at + 1))
        };
        let (name_len, version) = split(name);
        for (index, entry) in self.meta_table.iter_mut().enumerate() {
            let (entry_len, entry_version) = split(&entry.name);
            if entry.name[..entry_len] != name[..name_len] {
                continue;
            }
            if let (Some(version), Some(entry_version)) = (version, entry_version) {
                let (new, old) = (&name[version..], &entry.name[entry_version..]);
                if new.len() == old.len() && new > old {
                    entry.name.truncate(entry_version);
                    entry.name.extend_from_slice(&name[version..]);
                }
            }
            if entry.sym.is_none() {
                entry.sym = sym;
            }
            return index;
        }
        // A name without a version, or with a short one, is filled out
        // with zeros.
        let mut full = name.to_vec();
        match version {
            None => full.extend_from_slice(b"/000000"),
            Some(version) => {
                let have = name.len() - version + 1;
                if have < 7 {
                    full.extend_from_slice(&b"000000"[..7 - have]);
                }
            }
        }
        self.meta_table.push(MetaEntry { name: full, sym });
        self.meta_table.len() - 1
    }

    /// Give a property name a number, if it is not a property already.
    pub(crate) fn def_prop(&mut self, name: &[u8]) -> usize {
        self.props.push(PropSym {
            name: name.to_vec(),
            id: self.next_prop_id,
            weak: false,
            seq: self.next_seq,
        });
        self.next_seq += 1;
        self.next_prop_id += 1;
        self.globals
            .insert(name.to_vec(), Global::Prop(self.props.len() - 1));
        self.props.len() - 1
    }
}

/// What to put back when a scope is left.
struct Scope {
    local_symtab: Option<TabId>,
    enclosing_symtab: Option<TabId>,
}

pub(crate) struct Parser<'p, 'a> {
    tok: &'p mut Preprocessor<'a>,
    /// Reading only to learn what the program defines: nothing is
    /// generated from this pass, and some warnings are left for the pass
    /// that does generate.
    syntax_only: bool,
    prog: Program,

    // The function being read.
    local_symtab: Option<TabId>,
    enclosing_local_symtab: Option<TabId>,
    local_cnt: usize,
    max_local_cnt: usize,
    /// Where the statement being read starts.
    cur_file: usize,
    cur_line: u32,
    /// The code body about to be read is a method's in an object
    /// definition.
    body_is_obj_prop: bool,
    /// The labels of the function being read.
    labels: Vec<LabelSym>,
    /// The loops, `switch` statements, labeled statements, and `try`
    /// and `finally` blocks being read, outermost first.
    enclosing: Vec<Enclosure>,
    next_enclosure_id: usize,
    /// The `switch` statements being read, outermost first.
    switches: Vec<SwitchInfo>,
    /// The tokens of the expression embedded in a string that was last
    /// looked at, read ahead to see what kind of thing it is.
    embed_toks: Vec<SrcToken>,
    /// The string templates, in the order they were defined.
    str_templates: Vec<StrTemplate>,
    /// The code being read is a method's or a property's: it has a
    /// `self`.
    self_valid: bool,
    /// The number the next object that is not a class gets for its
    /// place in the source.
    src_order_idx: i32,

    // What the function being read shares with functions nested in it.
    has_local_ctx: bool,
    local_ctx_var_num: usize,
    next_ctx_arr_idx: usize,
    self_referenced: bool,
    full_ctx_referenced: bool,
    local_ctx_needs_self: bool,
    local_ctx_needs_full: bool,
    /// The body being read, as a number in `Program::body_refs`; or
    /// the last one read, at the outermost level.
    cur_code_body: Option<usize>,
}

/// The kinds of code body.
#[derive(Clone, Copy, PartialEq, Eq)]
enum CbType {
    Normal,
    AnonFn,
    ShortAnonFn,
    AnonMethod,
}

/// The first place in a context that a local can have: the one before
/// it is for the method's `self`.
const FIRST_CTX_ARR_IDX: usize = 2;

/// A string template: an embedded expression made of these tokens is a
/// call to a function.
struct StrTemplate {
    /// The tokens' text. `*` stands for one or more tokens that are an
    /// expression, the function's argument.
    toks: Vec<Vec<u8>>,
    star: bool,
    func: Vec<u8>,
}

/// What an embedded expression is inside of, within its string.
#[derive(Clone, Copy, PartialEq, Eq)]
enum EmbedKind {
    Outer,
    If,
    OneOf,
    FirstTime,
}

struct EmbedLevel<'l> {
    kind: EmbedKind,
    parent: Option<&'l EmbedLevel<'l>>,
}

impl EmbedLevel<'_> {
    fn is_in(&self, kind: EmbedKind) -> bool {
        self.kind == kind || self.parent.is_some_and(|parent| parent.is_in(kind))
    }
}

/// The phrases that end `<<one of>>`, each saying how the alternatives
/// are chosen.
const ONE_OF_ENDINGS: [&str; 12] = [
    "purely at random",
    "then purely at random",
    "at random",
    "then at random",
    "sticky random",
    "as decreasingly likely outcomes",
    "shuffled",
    "then shuffled",
    "half shuffled",
    "then half shuffled",
    "cycling",
    "stopping",
];

/// What a `switch` learns from its body as the body is read.
#[derive(Default)]
struct SwitchInfo {
    case_cnt: usize,
    has_default: bool,
}

type Parsed<T> = Result<T, Stop>;

/// The parser cannot go on: the source has ended in the middle of
/// something.
#[derive(Debug)]
pub(crate) struct Stop;

impl<'p, 'a> Parser<'p, 'a> {
    pub(crate) fn new(tok: &'p mut Preprocessor<'a>, syntax_only: bool, known: Exports) -> Self {
        let mut prog = Program {
            next_obj_id: 1,
            next_prop_id: 1,
            ..Program::default()
        };
        for name in SPECIAL_PROPS {
            prog.def_prop(name.as_bytes());
        }
        for func in known.funcs {
            prog.globals
                .insert(func.name.clone(), Global::Func(prog.funcs.len()));
            prog.funcs.push(func);
        }
        for name in PREDEFINED_METACLASSES {
            prog.meta_table.push(MetaEntry {
                name: name.as_bytes().to_vec(),
                sym: None,
            });
        }
        // Objects and the objects that stand for built-in classes are
        // numbered together, in the order of the table of global names:
        // chain by chain, and in a chain as they were first met.
        let mut numbered: Vec<(usize, u32, bool, usize)> =
            known
                .objs
                .iter()
                .enumerate()
                .map(|(index, obj)| (global_hash(&obj.name), obj.id, false, index))
                .chain(
                    known.metas.iter().enumerate().map(|(index, (meta, _))| {
                        (global_hash(&meta.name), meta.obj_id, true, index)
                    }),
                )
                .collect();
        numbered.sort_unstable();
        let mut objs = known.objs;
        let mut metas = known.metas;
        for (_, _, is_meta, index) in numbered {
            if is_meta {
                metas[index].0.obj_id = prog.next_obj_id;
            } else {
                objs[index].id = prog.next_obj_id;
            }
            prog.next_obj_id += 1;
        }
        for obj in objs {
            prog.globals
                .insert(obj.name.clone(), Global::Obj(prog.objs.len()));
            prog.objs.push(obj);
        }
        for (mut meta, vm_name) in metas {
            let index = prog.metas.len();
            meta.meta = prog.find_or_add_meta(&vm_name, Some(index));
            prog.globals.insert(meta.name.clone(), Global::Meta(index));
            prog.metas.push(meta);
        }
        for sym in known.enums {
            prog.globals
                .insert(sym.name.clone(), Global::Enum(prog.enums.len()));
            prog.enums.push(sym);
        }
        for (name, weak, seq) in known.props {
            if !prog.globals.contains_key(&name) {
                let prop = prog.def_prop(&name);
                prog.props[prop].weak = weak;
                prog.props[prop].seq = seq;
            }
        }
        if !syntax_only {
            prog.next_seq = SECOND_READING;
        }
        Self {
            tok,
            syntax_only,
            prog,
            local_symtab: None,
            enclosing_local_symtab: None,
            local_cnt: 0,
            max_local_cnt: 0,
            cur_file: 0,
            cur_line: 0,
            body_is_obj_prop: false,
            labels: Vec::new(),
            enclosing: Vec::new(),
            next_enclosure_id: 0,
            switches: Vec::new(),
            embed_toks: Vec::new(),
            str_templates: Vec::new(),
            self_valid: false,
            src_order_idx: 1,
            has_local_ctx: false,
            local_ctx_var_num: 0,
            next_ctx_arr_idx: FIRST_CTX_ARR_IDX,
            self_referenced: false,
            full_ctx_referenced: false,
            local_ctx_needs_self: false,
            local_ctx_needs_full: false,
            cur_code_body: None,
        }
    }

    // ---- tokens and messages ----

    fn cur(&self) -> Tok {
        self.tok.cur_token().typ
    }

    fn next(&mut self) -> Tok {
        self.tok.next_token()
    }

    fn cur_text(&self) -> Vec<u8> {
        self.tok.cur_token().text.clone()
    }

    /// Whether the current token is this text, whatever kind of token
    /// it is.
    fn cur_matches(&self, text: &[u8]) -> bool {
        self.tok.cur_token().text == text
    }

    fn cur_is_sym(&self, text: &[u8]) -> bool {
        self.cur() == Tok::Sym && self.tok.cur_token().text == text
    }

    fn error(&mut self, message: impl Into<String>) {
        self.tok.log_error(message);
    }

    /// Report an error that names the current token: `%` in the message
    /// is where the token goes.
    fn error_curtok(&mut self, message: &str) {
        let shown = self.tok.cur_token_shown();
        self.tok.log_error(message.replacen('%', &shown, 1));
    }

    /// Something the compiler does not do yet.
    fn unsupported(&mut self, what: &str) {
        self.tok
            .log_error(format!("{what} is not supported by this compiler yet"));
    }

    /// A name as a message shows it.
    fn name(text: &[u8]) -> String {
        show_name(text)
    }

    fn stmt(&self, kind: StmtKind) -> Stmt {
        Stmt {
            kind,
            file: self.cur_file,
            line: self.cur_line,
        }
    }

    // ---- scopes and locals ----

    fn enter_scope(&mut self) -> Scope {
        let scope = Scope {
            local_symtab: self.local_symtab,
            enclosing_symtab: self.enclosing_local_symtab,
        };
        self.enclosing_local_symtab = self.local_symtab;
        scope
    }

    fn leave_scope(&mut self, scope: Scope) {
        self.local_symtab = scope.local_symtab;
        self.enclosing_local_symtab = scope.enclosing_symtab;
    }

    /// Give the current scope a table of its own, if it is still using
    /// the one of the scope around it.
    fn create_scope_local_symtab(&mut self) {
        if self.local_symtab == self.enclosing_local_symtab {
            self.prog.tabs.push(LocalTab {
                parent: self.local_symtab,
                ..LocalTab::default()
            });
            self.local_symtab = Some(self.prog.tabs.len() - 1);
        }
    }

    fn alloc_local(&mut self) -> usize {
        self.max_local_cnt = self.max_local_cnt.max(self.local_cnt + 1);
        self.local_cnt += 1;
        self.local_cnt - 1
    }

    fn find_direct(&self, tab: TabId, name: &[u8]) -> Option<LocalId> {
        self.prog.tabs[tab]
            .entries
            .iter()
            .copied()
            .find(|&id| self.prog.locals[id].name == name)
    }

    /// Look a name up in the scopes of the function being read.
    fn find_local(&self, name: &[u8]) -> Option<LocalId> {
        let mut tab = self.local_symtab;
        while let Some(id) = tab {
            if let Some(found) = self.find_direct(id, name) {
                return Some(found);
            }
            tab = self.prog.tabs[id].parent;
        }
        None
    }

    fn add_local_entry(&mut self, name: &[u8], is_param: bool, var_num: usize) -> Option<LocalId> {
        let tab = self.local_symtab.expect("a scope's own table");
        if self.find_direct(tab, name).is_some() {
            let what = if is_param {
                "formal parameter"
            } else {
                "local variable"
            };
            self.error(format!(
                "{what} \"{}\" defined more than once",
                Self::name(name)
            ));
            return None;
        }
        let (file, line) = self.tok.last_pos();
        self.prog.locals.push(Local {
            name: name.to_vec(),
            is_param,
            var_num,
            file,
            line,
            ..Local::default()
        });
        let id = self.prog.locals.len() - 1;
        self.prog.tabs[tab].entries.push(id);
        Some(id)
    }

    /// Declare a local variable. The reference takes the next slot
    /// whether or not the name turns out to be usable.
    fn add_local(&mut self, name: &[u8]) -> Option<LocalId> {
        let var_num = self.alloc_local();
        self.add_local_entry(name, false, var_num)
    }

    // ---- the top level ----

    /// Read the whole source.
    pub(crate) fn parse_top(mut self) -> Program {
        self.next();
        let mut suppress_error = false;
        loop {
            let mut suppress_next_error = false;
            let result = match self.cur() {
                Tok::Eof => break,
                Tok::Function | Tok::Method => self.parse_function(true),
                Tok::Sym => self.parse_object_or_func(suppress_error, &mut suppress_next_error),
                Tok::Sem => {
                    self.next();
                    Ok(())
                }
                Tok::Intrinsic => self.parse_intrinsic(),
                Tok::Class => {
                    self.next();
                    self.parse_object(true)
                }
                Tok::Object => {
                    if self.next() == Tok::Template {
                        self.unsupported("an object template");
                        suppress_next_error = true;
                        Ok(())
                    } else {
                        self.tok.unget_token();
                        self.parse_anon_object()
                    }
                }
                Tok::Property => self.parse_property(),
                Tok::Enum => {
                    self.parse_enum();
                    Ok(())
                }
                Tok::Export => {
                    self.parse_export();
                    Ok(())
                }
                Tok::Extern
                | Tok::Transient
                | Tok::Plus
                | Tok::Inc
                | Tok::Replace
                | Tok::Modify
                | Tok::Dictionary
                | Tok::Grammar => {
                    if !suppress_error {
                        let shown = self.tok.cur_token_shown();
                        self.unsupported(&format!("a definition starting with \"{shown}\""));
                    }
                    suppress_next_error = true;
                    self.next();
                    Ok(())
                }
                _ => {
                    if !suppress_error {
                        self.error_curtok(
                            "expected function or object definition, but found \"%\"",
                        );
                    }
                    suppress_next_error = true;
                    self.next();
                    Ok(())
                }
            };
            if result.is_err() {
                break;
            }
            suppress_error = suppress_next_error;
        }
        // The anonymous functions come after everything else.
        for body in std::mem::take(&mut self.prog.nested) {
            self.prog.top.push(Top::Func(body));
        }
        self.prog
    }

    fn parse_object_or_func(
        &mut self,
        suppress_error: bool,
        suppress_next_error: &mut bool,
    ) -> Parsed<()> {
        let first = self.cur_text();
        match self.next() {
            Tok::LPar => {
                self.tok.unget_token();
                self.parse_function(false)
            }
            Tok::Template if first == b"string" => self.parse_string_template(),
            Tok::Template => {
                self.unsupported("an object template");
                *suppress_next_error = true;
                Ok(())
            }
            Tok::Colon => {
                self.tok.unget_token();
                self.parse_object(false)
            }
            // A name that is an object's, with no colon after it,
            // starts an object without a name: the name is the
            // object's superclass. On the first reading, a name not
            // defined yet is taken to be an object's.
            _ if matches!(
                (self.prog.globals.get(&first), self.syntax_only),
                (Some(Global::Obj(_)), _) | (None, true)
            ) =>
            {
                self.tok.unget_token();
                self.parse_anon_object()
            }
            _ => {
                if !suppress_error {
                    self.error_curtok("expected function or object definition, but found \"%\"");
                }
                *suppress_next_error = true;
                Ok(())
            }
        }
    }

    fn parse_function(&mut self, func_kw_present: bool) -> Parsed<()> {
        if func_kw_present {
            self.next();
        }
        if self.cur() != Tok::Sym {
            self.error_curtok("expected symbol after 'function', but found \"%\"");
            return Ok(());
        }
        let name = self.cur_text();
        let (sym_file, sym_line) = self.tok.last_pos();
        match self.next() {
            Tok::Sem => {
                // A declaration ahead of the definition says nothing new.
                self.next();
                return Ok(());
            }
            Tok::LPar | Tok::LBrace => {}
            _ => {
                self.error_curtok("expected ';', '(', or '{', but found \"%\"");
                return Ok(());
            }
        }
        let Some((body, has_retval)) = self.parse_code_body(false, CbType::Normal, None, None)?
        else {
            return Ok(());
        };
        self.prog.last_labels = std::rc::Rc::clone(&body.labels);
        let (argc, opt_argc, varargs) = (body.argc, body.opt_argc, body.varargs);

        // A name that is a property only because its address was
        // taken is the function's now.
        if let Some(Global::Prop(prop)) = self.prog.globals.get(&name) {
            if self.prog.props[*prop].weak {
                self.prog.globals.remove(&name);
            }
        }
        let existing = self.prog.globals.get(&name).copied();
        let func_id = match existing {
            Some(
                Global::Bif(_)
                | Global::Obj(_)
                | Global::Prop(_)
                | Global::Meta(_)
                | Global::Enum(_),
            ) => {
                self.tok.log_at(
                    sym_file,
                    sym_line,
                    Severity::Error,
                    format!(
                        "symbol \"{}\" is already defined - can't redefine as function",
                        Self::name(&name)
                    ),
                );
                return Ok(());
            }
            Some(Global::Func(id)) => {
                let func = &self.prog.funcs[id];
                let mut redef = !func.is_extern;
                if func.has_proto {
                    redef |= func.argc != argc || func.varargs != varargs;
                }
                if redef {
                    let message = if func.is_extern {
                        "function \"%\" has an incompatible previous declaration"
                    } else {
                        "function \"%\" is already defined"
                    };
                    self.tok.log_at(
                        sym_file,
                        sym_line,
                        Severity::Error,
                        message.replacen('%', &Self::name(&name), 1),
                    );
                    return Ok(());
                }
                self.prog.funcs[id].is_extern = false;
                id
            }
            None => {
                self.prog.funcs.push(FuncSym {
                    name: name.clone(),
                    argc,
                    opt_argc,
                    varargs,
                    has_retval,
                    is_extern: false,
                    has_proto: true,
                    body: None,
                });
                let id = self.prog.funcs.len() - 1;
                self.prog.globals.insert(name, Global::Func(id));
                id
            }
        };
        self.prog.funcs[func_id].body = Some(self.prog.bodies.len());
        self.prog.top.push(Top::Func(self.prog.bodies.len()));
        self.prog.bodies.push((body, Some(func_id)));
        Ok(())
    }

    /// Read `string template <<tokens>> function;`. From here on, an
    /// expression embedded in a string that is these tokens is a call
    /// to the function. One token can be `*`, for an expression that is
    /// the function's argument.
    fn parse_string_template(&mut self) -> Parsed<()> {
        let mut toks = Vec::new();
        let mut star = false;
        if self.next() == Tok::Shl {
            self.next();
        } else {
            self.error_curtok("expected '<<' at start of string template, but found '%'");
        }
        loop {
            match self.cur() {
                Tok::AShr => {
                    self.next();
                    break;
                }
                Tok::Sem | Tok::RBrace => {
                    self.error_curtok("expected '>>' at end of string template, but found '%'");
                    return Ok(());
                }
                Tok::Eof => {
                    // The reference never stops reading here.
                    self.error_curtok("expected '>>' at end of string template, but found '%'");
                    return Err(Stop);
                }
                typ => {
                    toks.push(self.cur_text());
                    if typ == Tok::Times {
                        if star {
                            self.error("only one '*' is allowed in a string template");
                        }
                        star = true;
                    }
                    self.next();
                }
            }
        }
        if self.cur() != Tok::Sym {
            return self.skip_to_sem();
        }

        // The function takes one argument if there is a `*`, and none
        // if not. If nothing is known of it yet, that is what it is
        // declared to be.
        let argc = usize::from(star);
        let name = self.cur_text();
        match self.prog.globals.get(&name).copied() {
            Some(Global::Func(id)) => {
                let func = &self.prog.funcs[id];
                if !func.argc_ok(argc) || (!func.is_extern && !func.has_retval) {
                    self.error(format!(
                        "string template processor function {}() does not match template - \
                         must take {argc} parameter(s) and return a value",
                        String::from_utf8_lossy(&name)
                    ));
                }
            }
            Some(_) => {
                // The reference takes whatever has the name to be a
                // function, and reads what is not there.
                self.error(format!(
                    "string template processor function {}() does not match template - \
                     must take {argc} parameter(s) and return a value",
                    String::from_utf8_lossy(&name)
                ));
            }
            None => {
                self.prog.funcs.push(FuncSym {
                    name: name.clone(),
                    argc,
                    opt_argc: 0,
                    varargs: false,
                    has_retval: true,
                    is_extern: true,
                    has_proto: true,
                    body: None,
                });
                self.prog
                    .globals
                    .insert(name.clone(), Global::Func(self.prog.funcs.len() - 1));
            }
        }
        self.str_templates.push(StrTemplate {
            toks,
            star,
            func: name,
        });
        self.next();
        self.parse_req_sem()
    }

    /// Note that the program uses a set of built-in functions, and give
    /// the set's number. A set's name is a name proper, a slash, and a
    /// version; of two declarations of a set, the later version is
    /// kept.
    fn add_fnset(&mut self, name: &[u8]) -> usize {
        // No more of a version is kept than the slash and six characters.
        const VERSION_LEN: usize = 7;
        let split = |text: &[u8]| text.iter().position(|&c| c == b'/').unwrap_or(text.len());
        let slash = split(name);
        let version = &name[slash..];
        for (index, known) in self.prog.fnsets.iter_mut().enumerate() {
            let known_slash = split(known);
            if known[..known_slash] != name[..slash] {
                continue;
            }
            if known_slash < known.len() && !version.is_empty() {
                // Both have versions. The reference compares as many
                // bytes as the new one has, and if the new is greater,
                // writes those bytes over the start of the old:
                // `/123456789` and then `/2` leave `/223456789`.
                let version = &version[..version.len().min(VERSION_LEN)];
                let old: Vec<u8> = (0..version.len())
                    .map(|i| known.get(known_slash + i).copied().unwrap_or(0))
                    .collect();
                if version > old.as_slice() {
                    let end = known_slash + version.len();
                    if known.len() < end {
                        known.resize(end, 0);
                    }
                    known[known_slash..end].copy_from_slice(version);
                }
            } else if known_slash == known.len() {
                known.extend_from_slice(&version[..version.len().min(VERSION_LEN)]);
            }
            return index;
        }
        self.prog.fnsets.push(name.to_vec());
        self.prog.fnsets.len() - 1
    }

    /// Read `intrinsic 'set-name' { name(parameters); ... }`: the
    /// functions of a set built into the virtual machine, in the order
    /// the set has them.
    fn parse_intrinsic(&mut self) -> Parsed<()> {
        let set = match self.next() {
            Tok::SStr => {
                let name = self.cur_text();
                self.next();
                self.add_fnset(&name)
            }
            Tok::Class => return self.parse_intrinsic_class(),
            _ => {
                self.error_curtok(
                    "expected function set name string after \"intrinsic\", but found \"%\"",
                );
                0
            }
        };
        if self.cur() == Tok::LBrace {
            self.next();
        } else {
            self.error_curtok("expected '{' after intrinsic name, but found \"%\"");
        }
        let mut index = 0;
        loop {
            match self.cur() {
                Tok::RBrace => {
                    self.next();
                    return Ok(());
                }
                Tok::Eof => {
                    self.error("end of file in \"intrinsic\" list - '}' is probably missing");
                    return Err(Stop);
                }
                Tok::Sem => {
                    self.next();
                }
                Tok::Sym => {
                    let name = self.cur_text();
                    if self.next() == Tok::LPar {
                        self.next();
                    } else {
                        self.error_curtok(
                            "expected '(' after function name in intrinsic list, but found \"%\"",
                        );
                    }
                    // Only the number of parameters matters.
                    let mut formals = Formals {
                        count_only: true,
                        ..Formals::default()
                    };
                    self.parse_formal_list(&mut formals)?;
                    self.parse_req_sem()?;
                    self.prog
                        .globals
                        .insert(name.clone(), Global::Bif(self.prog.bifs.len()));
                    self.prog.bifs.push(BifSym {
                        name,
                        set,
                        index,
                        min_argc: formals.argc,
                        max_argc: formals.argc + formals.opt_argc,
                        varargs: formals.varargs,
                    });
                    index += 1;
                }
                _ => {
                    self.error_curtok("expected function name in intrinsic list, but found \"%\"");
                    self.next();
                }
            }
        }
    }

    /// Read a function's parameters and body. Nothing comes back if the
    /// body could not be made sense of.
    /// Forget what the last function shared with functions nested in
    /// it.
    fn clear_local_ctx(&mut self) {
        self.has_local_ctx = false;
        self.local_ctx_var_num = 0;
        self.next_ctx_arr_idx = FIRST_CTX_ARR_IDX;
        self.self_referenced = false;
        self.full_ctx_referenced = false;
        self.local_ctx_needs_self = false;
        self.local_ctx_needs_full = false;
    }

    /// Give the function being read a context for its locals, if it has
    /// none yet.
    fn init_local_ctx(&mut self) {
        if !self.has_local_ctx {
            self.has_local_ctx = true;
            self.local_ctx_var_num = self.alloc_local();
        }
    }

    /// The local in which a body keeps the context from so many
    /// functions out.
    fn ctx_var_for_level(&mut self, body: &mut CodeBody, level: usize) -> usize {
        if let Some(&(_, var)) = body
            .closure
            .ctx_levels
            .iter()
            .find(|&&(have, _)| have == level)
        {
            return var;
        }
        let var = self.alloc_local();
        body.local_cnt = self.max_local_cnt;
        body.closure.ctx_levels.push((level, var));
        var
    }

    /// Settle, at the end of a body, what it shares: its own context,
    /// and which of the locals of the functions around it it used.
    fn finish_local_ctx(&mut self, body: &mut CodeBody, has_outer: bool) {
        if self.has_local_ctx {
            body.closure.has_local_ctx = true;
            body.closure.local_ctx_var = self.local_ctx_var_num;
            body.closure.local_ctx_arr_size = self.next_ctx_arr_idx - 1;
        }
        if !has_outer {
            return;
        }
        // The outermost table is the one with the views of the locals
        // of the functions around this one.
        let Some(mut tab) = self.local_symtab else {
            return;
        };
        while let Some(parent) = self.prog.tabs[tab].parent {
            tab = parent;
        }
        for id in enum_order(&self.prog.tabs[tab], &self.prog.locals) {
            let local = &self.prog.locals[id];
            if !local.referenced {
                // Never used here: it is not shared on this function's
                // account.
                self.prog.tabs[tab].entries.retain(|&entry| entry != id);
                continue;
            }
            if let Some(orig) = local.ctx_orig {
                let (used, assigned, level) = (local.val_used, local.val_assigned, local.ctx_level);
                self.prog.convert_to_ctx_var(orig, used, assigned);
                let var = self.ctx_var_for_level(body, level);
                let local = &mut self.prog.locals[id];
                local.ctx_var_num = var;
                local.ctx_var_num_set = true;
            }
        }
    }

    /// Make the tables of locals for an anonymous function: one with a
    /// view of each local of the functions around it, and inside that
    /// one, its own.
    fn anon_symtab(&mut self) -> (TabId, TabId) {
        self.prog.tabs.push(LocalTab::default());
        let enc = self.prog.tabs.len() - 1;
        let (file, line) = self.tok.last_pos();
        let mut tab = self.local_symtab;
        while let Some(id) = tab {
            for orig in enum_order(&self.prog.tabs[id], &self.prog.locals) {
                let name = self.prog.locals[orig].name.clone();
                // A local of the same name further in hides this one.
                if self.find_direct(enc, &name).is_some() {
                    continue;
                }
                let level = if self.prog.locals[orig].is_ctx_local {
                    self.prog.locals[orig].ctx_level + 1
                } else {
                    1
                };
                self.prog.locals.push(Local {
                    name,
                    file,
                    line,
                    is_ctx_local: true,
                    ctx_orig: Some(orig),
                    ctx_level: level,
                    ..Local::default()
                });
                let copy = self.prog.locals.len() - 1;
                self.prog.tabs[enc].entries.push(copy);
            }
            tab = self.prog.tabs[id].parent;
        }
        self.prog.tabs.push(LocalTab {
            parent: Some(enc),
            ..LocalTab::default()
        });
        (enc, self.prog.tabs.len() - 1)
    }

    /// After an anonymous function has been read: the locals of the
    /// function around it that it used get their places in that
    /// function's context.
    fn anon_finish(&mut self, enc: TabId) {
        let mut tab = self.local_symtab;
        while let Some(id) = tab {
            for local in enum_order(&self.prog.tabs[id], &self.prog.locals) {
                let name = self.prog.locals[local].name.clone();
                if self.find_direct(enc, &name).is_none() {
                    continue;
                }
                let sym = &self.prog.locals[local];
                if !sym.is_ctx_local || sym.ctx_var_num_set {
                    continue;
                }
                self.init_local_ctx();
                let var = self.local_ctx_var_num;
                let sym = &mut self.prog.locals[local];
                sym.ctx_var_num = var;
                sym.ctx_var_num_set = true;
                if sym.ctx_arr_idx == 0 {
                    sym.ctx_arr_idx = self.next_ctx_arr_idx;
                    self.next_ctx_arr_idx += 1;
                }
            }
            tab = self.prog.tabs[id].parent;
        }
    }

    /// Read an anonymous function: `{params: expression}`,
    /// `function(params) { ... }`, or `method(params) { ... }`. The
    /// current token is the brace or the keyword.
    fn parse_anon_func(&mut self, short_form: bool) -> Option<Expr> {
        let cb_type = if short_form {
            CbType::ShortAnonFn
        } else if self.cur() == Tok::Method {
            CbType::AnonMethod
        } else {
            CbType::AnonFn
        };
        let is_method = cb_type == CbType::AnonMethod;
        self.next();
        let (enc, lcltab) = self.anon_symtab();

        // A function shares the `self` of the code it is in, if that
        // has one. A method has whatever `self` it is called with.
        let self_valid = is_method || self.self_valid;
        let (mut body, has_retval) = self
            .parse_nested_code_body(self_valid, lcltab, cb_type)
            .ok()??;
        if is_method {
            body.closure.is_anon_method = true;
        } else {
            if body.closure.self_referenced {
                self.self_referenced = true;
                self.local_ctx_needs_self = true;
            }
            if body.closure.full_ctx_referenced {
                self.full_ctx_referenced = true;
                self.local_ctx_needs_full = true;
            }
        }
        self.anon_finish(enc);
        if !is_method
            && self.self_valid
            && (body.closure.self_referenced || body.closure.full_ctx_referenced)
        {
            self.init_local_ctx();
        }
        let index = self.prog.bodies.len();
        self.prog.bodies.push((body, None));
        self.prog.nested.push(index);
        Some(Expr::AnonFunc {
            body: index,
            has_retval,
        })
    }

    /// Read the body of a function inside another, and go back to
    /// reading the outer one.
    fn parse_nested_code_body(
        &mut self,
        self_valid: bool,
        lcltab: TabId,
        cb_type: CbType,
    ) -> Parsed<Option<(CodeBody, bool)>> {
        let old_local_symtab = self.local_symtab;
        let old_enclosing_local_symtab = self.enclosing_local_symtab;
        let old_enclosing = std::mem::take(&mut self.enclosing);
        let old_switches = std::mem::take(&mut self.switches);
        let old_labels = std::mem::take(&mut self.labels);
        let old_local_cnt = self.local_cnt;
        let old_max_local_cnt = self.max_local_cnt;
        let old_has_local_ctx = self.has_local_ctx;
        let old_local_ctx_var_num = self.local_ctx_var_num;
        let old_next_ctx_arr_idx = self.next_ctx_arr_idx;
        let old_self_valid = self.self_valid;
        let old_self_referenced = self.self_referenced;
        let old_full_ctx_referenced = self.full_ctx_referenced;
        let old_local_ctx_needs_self = self.local_ctx_needs_self;
        let old_local_ctx_needs_full = self.local_ctx_needs_full;
        let old_cur_code_body = self.cur_code_body;

        let result = self.parse_code_body(self_valid, cb_type, Some(lcltab), old_cur_code_body);

        self.cur_code_body = old_cur_code_body;
        self.local_symtab = old_local_symtab;
        self.enclosing_local_symtab = old_enclosing_local_symtab;
        self.enclosing = old_enclosing;
        self.switches = old_switches;
        self.labels = old_labels;
        self.local_cnt = old_local_cnt;
        self.max_local_cnt = old_max_local_cnt;
        self.has_local_ctx = old_has_local_ctx;
        self.local_ctx_var_num = old_local_ctx_var_num;
        self.next_ctx_arr_idx = old_next_ctx_arr_idx;
        self.self_valid = old_self_valid;
        self.self_referenced = old_self_referenced;
        self.full_ctx_referenced = old_full_ctx_referenced;
        self.local_ctx_needs_self = old_local_ctx_needs_self;
        self.local_ctx_needs_full = old_local_ctx_needs_full;
        // Where the statement being read started is not put back: the
        // reference takes the statement around the function to have
        // started where the last one inside it did.
        result
    }

    /// Read the inside of `{params: expression}`: the parameters, any
    /// locals, and the expression, up to the closing brace.
    fn parse_short_anon_body(&mut self, formals: &mut Formals) -> Parsed<Option<Stmt>> {
        formals.short_form = true;
        self.parse_formal_list(formals)?;
        let mut expr: Option<Expr> = None;
        while self.cur() == Tok::Local {
            if self.next() == Tok::Sym {
                let name = self.cur_text();
                let local = self.add_local(&name);
                if matches!(self.next(), Tok::Asi | Tok::Eq) {
                    // A value that cannot be read is the end of the
                    // source, which is found again below.
                    if let Some(sub) = self.parse_local_initializer(local) {
                        expr = Some(match expr {
                            None => sub,
                            Some(first) => {
                                Expr::Binary(BinOp::Comma, Box::new(first), Box::new(sub))
                            }
                        });
                    }
                }
                if self.cur() == Tok::Comma {
                    self.next();
                }
            } else {
                self.error_curtok(
                    "expected symbol name in local variable declaration, but found \"%\"",
                );
            }
        }
        if expr.is_none() || self.cur() != Tok::RBrace {
            if let Some(sub) = self.parse_expr() {
                expr = Some(match expr {
                    None => sub,
                    Some(first) => Expr::Binary(BinOp::Comma, Box::new(first), Box::new(sub)),
                });
            }
        }
        if self.cur() == Tok::Sem {
            self.error("semicolon is not allowed in a short anonymous function");
            self.next();
        }
        // The reference has two messages under this error's number,
        // and the one it finds is not the one about the brace.
        const NEED_RBRACE: &str = "semicolon is not allowed in a short-form anonymous function";
        match self.cur() {
            Tok::RBrace => {
                self.next();
            }
            Tok::Eof => {
                self.error(NEED_RBRACE);
                return Ok(None);
            }
            _ => self.error(NEED_RBRACE),
        }
        // The function returns the expression's value, if it has one.
        let Some(expr) = expr else {
            return Ok(None);
        };
        let kind = if self.prog.has_return_value(&expr) {
            StmtKind::Return(Some(expr))
        } else {
            StmtKind::Expr(expr)
        };
        let inner = self.stmt(kind);
        Ok(Some(self.stmt(StmtKind::Comp {
            stmts: vec![inner],
            symtab: self.local_symtab,
        })))
    }

    fn parse_code_body(
        &mut self,
        self_valid: bool,
        cb_type: CbType,
        local_symtab: Option<TabId>,
        enclosing: Option<usize>,
    ) -> Parsed<Option<(CodeBody, bool)>> {
        self.prog.body_refs.push(BodyRef::default());
        let body_ref = self.prog.body_refs.len() - 1;
        self.cur_code_body = Some(body_ref);
        let parsing_anon_fn = matches!(cb_type, CbType::AnonFn | CbType::ShortAnonFn);
        self.self_valid = self_valid;
        self.clear_local_ctx();
        self.local_symtab = local_symtab;
        self.enclosing_local_symtab = local_symtab.and_then(|tab| self.prog.tabs[tab].parent);
        self.local_cnt = 0;
        self.max_local_cnt = 0;
        self.labels.clear();
        self.enclosing.clear();
        self.switches.clear();

        let mut formals = Formals {
            typed_ok: cb_type == CbType::Normal,
            ..Formals::default()
        };
        let mut empty = false;
        let stm = if cb_type == CbType::ShortAnonFn {
            let Some(stm) = self.parse_short_anon_body(&mut formals)? else {
                return Ok(None);
            };
            stm
        } else {
            if self.cur() == Tok::LPar {
                self.next();
                self.parse_formal_list(&mut formals)?;
            }
            // `name(args) = { ... }` was how an older language wrote a
            // method.
            if self.cur() == Tok::Eq && std::mem::take(&mut self.body_is_obj_prop) {
                self.error("'=' is not allowed with a method definition");
                self.next();
            }
            self.body_is_obj_prop = false;
            if self.cur() != Tok::LBrace {
                self.error_curtok("expected '{' at start of method code body, but found \"%\"");
                empty = matches!(self.cur(), Tok::Sem | Tok::RBrace);
            }
            if empty {
                // It looks like the end of the definition: take the body
                // to be empty.
                self.stmt(StmtKind::Comp {
                    stmts: Vec::new(),
                    symtab: None,
                })
            } else {
                self.parse_compound(true, true, false, true)?
            }
        };
        let (mut body, has_retval) = self.check_body_flow(formals, stm);

        body.closure.enclosing = enclosing;
        self.finish_local_ctx(&mut body, local_symtab.is_some());
        // An anonymous function that uses its method's `self` finds it
        // in the context of the function just outside it.
        if self_valid && parsing_anon_fn && (self.self_referenced || self.full_ctx_referenced) {
            self.ctx_var_for_level(&mut body, 1);
        }
        body.closure.self_referenced = self.self_referenced;
        body.closure.full_ctx_referenced = self.full_ctx_referenced;
        body.closure.local_ctx_needs_self = self.local_ctx_needs_self;
        body.closure.local_ctx_needs_full = self.local_ctx_needs_full;
        self.prog.body_refs[body_ref] = BodyRef {
            enclosing,
            local_ctx_needs_full: self.local_ctx_needs_full,
        };
        Ok(Some((body, has_retval)))
    }

    /// Work out how a body can be left, warn of what is odd about it,
    /// and make the body.
    fn check_body_flow(&mut self, formals: Formals, stm: Stmt) -> (CodeBody, bool) {
        let mut flow = Flow {
            warn: true,
            syntax_only: self.syntax_only,
            labels: &self.labels,
            mark: &self.labels,
            warnings: Vec::new(),
        };
        let flags = flow.control_flow(&stm);
        for (file, line, message) in flow.warnings {
            self.tok
                .log_at(file, line, Severity::Warning, message.to_owned());
        }
        if !self.syntax_only {
            let warning = if flags & flow::RET_VAL != 0 && flags & flow::RET_VOID != 0 {
                Some("code has \"return\" statements both with and without values")
            } else if flags & flow::RET_VAL != 0 && flags & flow::NEXT != 0 {
                Some("code has \"return\" with value but also falls off end")
            } else {
                None
            };
            if let Some(message) = warning {
                self.tok
                    .log_at(stm.file, stm.line, Severity::Warning, message.to_owned());
            }
        }
        let has_retval = flags & flow::RET_VAL != 0;
        (self.finish_code_body(formals, stm), has_retval)
    }

    fn finish_code_body(&mut self, formals: Formals, stm: Stmt) -> CodeBody {
        let (end_file, end_line) = self.tok.last_pos();
        CodeBody {
            lcltab: self.local_symtab,
            stm,
            argc: formals.argc,
            opt_argc: formals.opt_argc,
            varargs: formals.varargs,
            varargs_list_local: formals.varargs_list_local,
            local_cnt: self.max_local_cnt,
            self_valid: self.self_valid,
            replaced: false,
            labels: std::rc::Rc::new(std::mem::take(&mut self.labels)),
            end_file,
            end_line,
            closure: Closure::default(),
        }
    }

    /// Read a parameter list. The opening parenthesis has been read.
    fn parse_formal_list(&mut self, formals: &mut Formals) -> Parsed<()> {
        // The parameters of `{a, b: expression}` end at the colon.
        let (end_tok, missing_end): (Tok, &str) = if formals.short_form {
            (
                Tok::Colon,
                "missing colon at end of anonymous function formal list - found \"%\"",
            )
        } else {
            (
                Tok::RPar,
                "missing right parenthesis ')' at end of formal parameter list - found \"%\"",
            )
        };
        let mut done = false;
        if self.cur() == end_tok {
            done = true;
            self.next();
        }
        while !done {
            match self.cur() {
                Tok::Ellipsis => {
                    formals.varargs = true;
                    done = self.after_ellipsis(end_tok, missing_end);
                }
                Tok::LBrack => {
                    formals.varargs = true;
                    match self.next() {
                        Tok::RBrack => done = self.after_ellipsis(end_tok, missing_end),
                        Tok::Sym => {
                            if !formals.count_only {
                                self.create_scope_local_symtab();
                                let name = self.cur_text();
                                let id = self.add_local(&name);
                                if let Some(id) = id {
                                    let local = &mut self.prog.locals[id];
                                    local.val_assigned = true;
                                    local.is_list_param = true;
                                }
                                formals.varargs_list_local = id;
                            }
                            match self.next() {
                                Tok::RBrack => {
                                    if self.next() == end_tok {
                                        self.next();
                                        done = true;
                                    } else {
                                        match self.cur() {
                                            Tok::Comma => self.error(
                                                "variable-argument list parameter must be the last parameter",
                                            ),
                                            Tok::LBrace | Tok::RBrace | Tok::LPar | Tok::Eof => {
                                                self.error_curtok(missing_end);
                                                done = true;
                                            }
                                            _ => {
                                                self.error_curtok(missing_end);
                                                if self.next() == end_tok {
                                                    self.next();
                                                }
                                                done = true;
                                            }
                                        }
                                    }
                                }
                                Tok::LBrace | Tok::RBrace | Tok::Eof => {
                                    self.error_curtok(
                                        "expected ']' after variable-argument list parameter, but found \"%\"",
                                    );
                                    done = true;
                                }
                                _ => {
                                    self.error_curtok(
                                        "expected ']' after variable-argument list parameter, but found \"%\"",
                                    );
                                    self.next();
                                    done = true;
                                }
                            }
                        }
                        _ => {
                            self.error_curtok(
                                "variable-argument list parameter name expected, but found \"%\"",
                            );
                            match self.cur() {
                                tok if tok == end_tok => {
                                    self.next();
                                    done = true;
                                }
                                Tok::RBrace | Tok::LBrace | Tok::Eof => done = true,
                                _ => {
                                    self.next();
                                }
                            }
                        }
                    }
                }
                Tok::Sym => {
                    let name = self.cur_text();
                    self.next();
                    if !formals.count_only
                        && formals.typed_ok
                        && self.cur() == Tok::Sym
                        && !self.cur_is_sym(b"multimethod")
                    {
                        self.unsupported("a parameter with a type");
                    }
                    // A colon after the name makes it a parameter
                    // passed by name.
                    let mut named = false;
                    if self.cur() == Tok::Colon {
                        named = true;
                        if formals.short_form {
                            // Here the colon names the parameter only
                            // if more of the list follows; otherwise
                            // it ends the list.
                            if !matches!(self.next(), Tok::Comma | Tok::Colon) {
                                named = false;
                                self.tok.unget_token();
                            }
                        } else {
                            self.next();
                        }
                    }
                    // `?` after it makes it optional, and so does a
                    // default value.
                    let suffix = self.cur();
                    let optional = matches!(suffix, Tok::Question | Tok::Eq);
                    let mut defval = None;
                    if optional {
                        if !named {
                            formals.opt_argc += 1;
                        }
                        self.next();
                        if suffix == Tok::Eq {
                            defval = self.parse_asi_expr();
                        }
                    }
                    if formals.opt_argc != 0 && !optional && !named {
                        self.error(format!(
                            "parameter '{}' must be optional because it follows an optional parameter",
                            String::from_utf8_lossy(&name)
                        ));
                    }
                    if !formals.count_only {
                        self.create_scope_local_symtab();
                        if named || optional {
                            // Not a parameter the caller's arguments
                            // fill in: a local, set by code at the
                            // start of the function.
                            if let Some(id) = self.add_local(&name) {
                                let local = &mut self.prog.locals[id];
                                local.is_named_param = named;
                                local.is_opt_param = optional;
                                if !named {
                                    local.param_index = formals.argc;
                                }
                                if let Some(defval) = defval {
                                    local.defval_is_const = defval.is_const();
                                    local.defval = Some(defval);
                                    local.defval_seqno = formals.defval_cnt;
                                    formals.defval_cnt += 1;
                                }
                            }
                        } else {
                            self.add_local_entry(&name, true, formals.argc);
                        }
                    }
                    if !named {
                        formals.argc += 1;
                    }
                    match self.cur() {
                        tok if tok == end_tok => {
                            self.next();
                            done = true;
                        }
                        Tok::Comma => {
                            self.next();
                        }
                        Tok::Sem | Tok::RBrace | Tok::LBrace | Tok::Eq | Tok::Eof => {}
                        Tok::Sym => self.error_curtok(
                            "expected a comma in formal parameter list, but found \"%\"",
                        ),
                        _ => {
                            self.error_curtok(
                                "expected a comma in formal parameter list, but found \"%\"",
                            );
                            self.next();
                        }
                    }
                }
                Tok::Sem | Tok::RBrace | Tok::LBrace | Tok::Eof => {
                    self.error_curtok(missing_end);
                    done = true;
                }
                tok if tok == end_tok => {
                    self.error("missing parameter name at end of formal parameter list");
                    self.next();
                    done = true;
                }
                _ if formals.short_form => {
                    // The colon was probably left out: the list ends
                    // here.
                    self.error_curtok(missing_end);
                    done = true;
                }
                _ => {
                    self.error_curtok(
                        "expected a symbol in formal parameter list, but found \"%\"",
                    );
                    self.next();
                }
            }
        }
        if self.cur_is_sym(b"multimethod") {
            if formals.typed_ok || formals.count_only {
                self.unsupported("a multi-method");
            } else {
                self.error("'multimethod' is not allowed here");
            }
            self.next();
        }
        // The count is of the parameters that have to be there.
        formals.argc -= formals.opt_argc;
        Ok(())
    }

    /// What follows `...` in a parameter list. True if the list is over.
    fn after_ellipsis(&mut self, end_tok: Tok, missing_rpar: &str) -> bool {
        if self.next() == end_tok {
            self.next();
            return true;
        }
        if self.cur() == Tok::Comma {
            self.next();
            self.error("'...' cannot be followed by additional formal parameters");
            false
        } else {
            self.error_curtok(missing_rpar);
            true
        }
    }

    // ---- statements ----

    fn parse_compound(
        &mut self,
        skip_lbrace: bool,
        need_rbrace: bool,
        in_switch: bool,
        use_enclosing_scope: bool,
    ) -> Parsed<Stmt> {
        let (file, line) = self.tok.last_pos();
        if skip_lbrace && self.cur() == Tok::LBrace {
            self.next();
        }
        let scope = (!use_enclosing_scope).then(|| self.enter_scope());
        let mut stmts = Vec::new();
        let mut skip_rbrace = false;
        loop {
            match self.cur() {
                Tok::RBrace => {
                    skip_rbrace = true;
                    break;
                }
                Tok::Eof => {
                    if need_rbrace {
                        self.error("unexpected end of file in code block - '}' missing");
                    }
                    break;
                }
                _ => {
                    if let Some(stmt) = self.parse_stm(in_switch, false)? {
                        stmts.push(stmt);
                    }
                }
            }
        }
        if stmts.is_empty() {
            stmts.push(self.stmt(StmtKind::Null));
        }
        let comp = Stmt {
            kind: StmtKind::Comp {
                stmts,
                symtab: self.local_symtab,
            },
            file,
            line,
        };
        if skip_rbrace {
            self.next();
        }
        if let Some(scope) = scope {
            self.leave_scope(scope);
        }
        Ok(comp)
    }

    /// Read a statement. Some statements leave nothing behind: an empty
    /// one, or a `local` with no initial values. `in_switch` says that
    /// the statement is directly in the body of a `switch`, where `case`
    /// and `default` can go.
    fn parse_stm(
        &mut self,
        in_switch: bool,
        compound_use_enclosing_scope: bool,
    ) -> Parsed<Option<Stmt>> {
        (self.cur_file, self.cur_line) = self.tok.last_pos();
        let (file, line) = (self.cur_file, self.cur_line);
        loop {
            return match self.cur() {
                Tok::Eof => {
                    self.error("unexpected end of file in code block - '}' missing");
                    Err(Stop)
                }
                Tok::DStrMid | Tok::DStrEnd | Tok::RBrace => {
                    self.error_curtok("expected start of statement, but found \"%\"");
                    self.next();
                    Ok(None)
                }
                Tok::Sem => {
                    self.next();
                    Ok(None)
                }
                Tok::Local => {
                    self.create_scope_local_symtab();
                    self.parse_local()
                }
                Tok::LBrace => self
                    .parse_compound(true, true, false, compound_use_enclosing_scope)
                    .map(Some),
                Tok::If => self.parse_if(),
                Tok::Return => self.parse_return(),
                Tok::For => self.parse_for(),
                Tok::While => self.parse_while(),
                Tok::Do => self.parse_do_while(),
                Tok::Break => self.parse_break_or_continue(true),
                Tok::Continue => self.parse_break_or_continue(false),
                Tok::Else => {
                    self.error("misplaced \"else\" - no corresponding \"if\" statement");
                    self.next();
                    Ok(None)
                }
                Tok::Foreach => self.parse_foreach(),
                Tok::Switch => self.parse_switch(),
                Tok::Goto => self.parse_goto(),
                Tok::Try => self.parse_try(),
                Tok::Throw => self.parse_throw(),
                Tok::Catch => {
                    self.error("misplaced \"catch\" clause - must be associated with \"try\"");
                    // Skip `(Class name)` if that is what follows.
                    if self.next() == Tok::LPar
                        && self.next() == Tok::Sym
                        && self.next() == Tok::Sym
                        && self.next() == Tok::RPar
                    {
                        self.next();
                    }
                    Ok(None)
                }
                Tok::Finally => {
                    self.error("misplaced \"finally\" clause - must be associated with \"try\"");
                    self.next();
                    Ok(None)
                }
                Tok::Case if in_switch => self.parse_case(),
                Tok::Case => {
                    self.error("misplaced \"case\" keyword - not in a \"switch\" statement");
                    self.next();
                    self.parse_expr();
                    if self.cur() == Tok::Colon {
                        self.next();
                    }
                    Ok(None)
                }
                Tok::Default if in_switch => self.parse_default(),
                Tok::Default => {
                    self.error("misplaced \"default\" keyword - not in a \"switch\" statement");
                    if self.next() == Tok::Colon {
                        self.next();
                    }
                    Ok(None)
                }
                Tok::Sym => {
                    let name = self.cur_text();
                    if self.next() == Tok::Colon {
                        let id = self.enclose(false);
                        let label = self.add_code_label(&name, file, line);
                        self.next();
                        let stm = self.parse_stm(in_switch, false);
                        self.enclosing.pop();
                        let stm = stm?.map(Box::new);
                        return Ok(Some(Stmt {
                            kind: StmtKind::Label { label, stm, id },
                            file,
                            line,
                        }));
                    }
                    self.tok.unget_token();
                    self.parse_expr_stm()
                }
                Tok::RPar => {
                    self.error("unexpected ')' found - ignored");
                    self.next();
                    continue;
                }
                _ => self.parse_expr_stm(),
            };
        }
    }

    fn parse_expr_stm(&mut self) -> Parsed<Option<Stmt>> {
        let expr = self.parse_expr();
        self.parse_req_sem()?;
        Ok(expr.map(|expr| self.stmt(StmtKind::Expr(expr))))
    }

    /// Read the semicolon that ends a statement.
    fn parse_req_sem(&mut self) -> Parsed<()> {
        match self.cur() {
            Tok::Sem => {
                self.next();
                Ok(())
            }
            Tok::RPar => {
                self.error("unexpected ')' found - ignored");
                self.skip_to_sem()
            }
            Tok::RBrack => {
                self.error("unexpected ']' found - ignored");
                self.skip_to_sem()
            }
            Tok::Eof => {
                self.error("expected ';', but found \"<end of file>\"");
                Err(Stop)
            }
            _ => {
                self.error_curtok("expected ';', but found \"%\"");
                Ok(())
            }
        }
    }

    /// Skip what is left of a statement that has gone wrong.
    fn skip_to_sem(&mut self) -> Parsed<()> {
        loop {
            match self.cur() {
                Tok::Eof => return Err(Stop),
                Tok::Sem => {
                    self.next();
                    return Ok(());
                }
                Tok::LBrace | Tok::RBrace => return Ok(()),
                _ => {
                    self.next();
                }
            }
        }
    }

    fn parse_local(&mut self) -> Parsed<Option<Stmt>> {
        let mut stmts = Vec::new();
        self.next();
        loop {
            if self.cur() == Tok::Sym {
                let name = self.cur_text();
                let local = self.add_local(&name);
                // An initializer that could not be read ends the
                // statement, after a look at what follows.
                let mut done = false;
                if matches!(self.next(), Tok::Eq | Tok::Asi) {
                    match self.parse_local_initializer(local) {
                        Some(expr) => {
                            let (file, line) = self.tok.last_pos();
                            stmts.push(Stmt {
                                kind: StmtKind::Expr(expr),
                                file,
                                line,
                            });
                        }
                        None => done = true,
                    }
                }
                match self.cur() {
                    Tok::Comma => {
                        self.next();
                    }
                    Tok::Sem => {
                        self.next();
                        break;
                    }
                    Tok::Sym => self
                        .error_curtok("expected ',' or ';' after local variable, but found \"%\""),
                    _ => {
                        self.error_curtok(
                            "expected ',' or ';' after local variable, but found \"%\"",
                        );
                        break;
                    }
                }
                if done {
                    break;
                }
            } else {
                self.error_curtok(
                    "expected symbol name in local variable declaration, but found \"%\"",
                );
                match self.cur() {
                    Tok::Sem => {
                        self.next();
                        break;
                    }
                    Tok::Eof => return Err(Stop),
                    _ => {
                        self.next();
                    }
                }
            }
        }
        Ok(match stmts.len() {
            0 => None,
            1 => stmts.pop(),
            _ => Some(self.stmt(StmtKind::Comp {
                stmts,
                symtab: self.local_symtab,
            })),
        })
    }

    /// Read `= value` after a local's name. The current token is the
    /// assignment operator.
    fn parse_local_initializer(&mut self, local: Option<LocalId>) -> Option<Expr> {
        self.next();
        let expr = self.parse_asi_expr()?;
        Some(match local {
            Some(id) => Expr::Assign(AsiOp::Simple, Box::new(Expr::Local(id)), Box::new(expr)),
            None => expr,
        })
    }

    fn parse_if(&mut self) -> Parsed<Option<Stmt>> {
        let (file, line) = self.tok.last_pos();
        if self.next() == Tok::LPar {
            self.next();
        } else {
            self.error_curtok("expected '(' after \"if\", but found \"%\"");
        }
        let Some(cond) = self.parse_cond_expr() else {
            return Err(Stop);
        };
        if self.cur() == Tok::RPar {
            self.next();
        } else {
            self.error_curtok("expected ')' after the \"if\" condition, but found \"%\"");
        }
        let then_part = self.parse_stm(false, false)?.map(Box::new);
        let else_part = if self.cur() == Tok::Else {
            self.next();
            self.parse_stm(false, false)?.map(Box::new)
        } else {
            None
        };
        Ok(Some(Stmt {
            kind: StmtKind::If {
                cond,
                then_part,
                else_part,
            },
            file,
            line,
        }))
    }

    fn parse_return(&mut self) -> Parsed<Option<Stmt>> {
        match self.next() {
            Tok::Sem => {
                self.next();
                Ok(Some(self.stmt(StmtKind::Return(None))))
            }
            Tok::LBrace | Tok::RBrace => {
                self.error_curtok("expected ';' or expression after \"return\", but found \"%\"");
                Ok(None)
            }
            _ => {
                // The reference makes the statement even when the
                // expression is missing; a missing expression here means
                // the source has ended, which stops everything below.
                let expr = self.parse_expr();
                self.parse_req_sem()?;
                Ok(Some(self.stmt(StmtKind::Return(expr))))
            }
        }
    }

    fn parse_for(&mut self) -> Parsed<Option<Stmt>> {
        const MISSING_PART: &str =
            "missing expression in \"for\" statement - expected ';', found '%'";
        let (file, line) = self.tok.last_pos();
        let scope = self.enter_scope();
        if self.next() == Tok::LPar {
            self.next();
        } else {
            self.error_curtok("expected '(' after \"for\", but found \"%\"");
        }
        let mut init: Option<Expr> = None;
        let mut cond = None;
        let mut reinit = None;
        let mut found_in = false;
        loop {
            let mut done = false;
            let mut expr = None;
            match self.cur() {
                Tok::Local => {
                    self.create_scope_local_symtab();
                    if self.next() == Tok::Sym {
                        let name = self.cur_text();
                        let local = self.add_local(&name);
                        match self.next() {
                            Tok::Asi | Tok::Eq => expr = self.parse_local_initializer(local),
                            Tok::Sym if self.cur_is_sym(b"in") => {
                                self.next();
                                let lval = match local {
                                    Some(local) => Expr::Local(local),
                                    None => Expr::constant(Value::Nil),
                                };
                                expr = self.parse_for_in_clause(lval);
                                found_in = true;
                                if expr.is_none() {
                                    done = true;
                                }
                            }
                            _ => self.error_curtok(
                                "initializer expected after local variable name in \"for\", but found \"%\"",
                            ),
                        }
                    } else {
                        self.error_curtok(
                            "expected symbol name in local variable declaration, but found \"%\"",
                        );
                    }
                }
                Tok::Sem => {
                    done = true;
                    if init.is_some() {
                        self.error("missing expression after comma in \"for\" initializer");
                    }
                }
                // With an `in` clause, there need be nothing more.
                Tok::RPar if found_in => done = true,
                Tok::RPar | Tok::LBrace | Tok::RBrace => {
                    self.error_curtok(MISSING_PART);
                    done = true;
                }
                _ => {
                    expr = self.parse_asi_expr();
                    if expr.is_none() {
                        done = true;
                    }
                    if self.cur_is_sym(b"in") {
                        self.next();
                        expr = match expr {
                            Some(lval) => self.parse_for_in_clause(lval),
                            None => None,
                        };
                        found_in = true;
                        if expr.is_none() {
                            done = true;
                        }
                    }
                }
            }
            if let Some(expr) = expr {
                init = Some(match init.take() {
                    Some(so_far) => Expr::Binary(BinOp::Comma, Box::new(so_far), Box::new(expr)),
                    None => expr,
                });
            }
            if done {
                break;
            }
            match self.cur() {
                Tok::Sem => break,
                Tok::Comma => {
                    self.next();
                }
                Tok::RPar if found_in => break,
                Tok::RPar | Tok::LBrace | Tok::RBrace => {
                    self.error_curtok(MISSING_PART);
                    break;
                }
                _ => {
                    self.error_curtok(
                        "expected ',' or ';' in \"for\" initializer, but found \"%\"",
                    );
                    self.next();
                    // At the end of the source this goes round for
                    // ever; what stops the reference is its limit on
                    // errors.
                    if self.tok.gave_up() {
                        return Err(Stop);
                    }
                }
            }
        }
        if self.cur() == Tok::Sem {
            let mut cont_to_reinit = true;
            self.next();
            if self.cur() != Tok::Sem {
                cond = self.parse_cond_expr();
            }
            match self.cur() {
                Tok::Sem => {
                    self.next();
                }
                Tok::RPar | Tok::LBrace | Tok::RBrace => {
                    self.error_curtok(MISSING_PART);
                    cont_to_reinit = false;
                }
                _ => self.error_curtok("expected ';' after \"for\" condition, but found \"%\""),
            }
            if cont_to_reinit && self.cur() != Tok::RPar {
                reinit = self.parse_expr();
            }
            if self.cur() == Tok::RPar {
                self.next();
            } else {
                self.error_curtok("missing ')' at end of \"for\" expression list - found \"%\"");
            }
        } else if self.cur() == Tok::RPar {
            self.next();
        }
        let symtab = self.local_symtab;
        let id = self.enclose(false);
        let body = self.parse_stm(false, false);
        self.enclosing.pop();
        let body = body?.map(Box::new);
        self.leave_scope(scope);
        Ok(Some(Stmt {
            kind: StmtKind::For {
                init,
                cond,
                reinit,
                body,
                symtab,
                id,
            },
            file,
            line,
        }))
    }

    /// A local that the source has no name for.
    fn hidden_local(&mut self, name: &[u8]) -> LocalId {
        let var_num = self.alloc_local();
        let (file, line) = self.tok.last_pos();
        self.prog.locals.push(Local {
            name: name.to_vec(),
            var_num,
            val_used: true,
            val_assigned: true,
            file,
            line,
            ..Local::default()
        });
        self.prog.locals.len() - 1
    }

    /// Read what follows `in` among a `for`'s initializers: a
    /// collection, or a range `from..to` with an optional `step`.
    fn parse_for_in_clause(&mut self, lval: Expr) -> Option<Expr> {
        let expr = self.parse_asi_expr()?;
        let for_in = if self.cur() == Tok::DotDot {
            self.next();
            let to = self.parse_asi_expr()?;
            let step = if self.cur_is_sym(b"step") {
                self.next();
                self.parse_asi_expr()
            } else {
                None
            };
            // An end or a step that is not a constant is worked out
            // once, and kept in a local.
            let to_local = (!to.is_const()).then(|| self.hidden_local(b".to"));
            let step_local = step
                .as_ref()
                .is_some_and(|step| !step.is_const())
                .then(|| self.hidden_local(b".step"));
            ForIn::Range {
                lval,
                from: expr,
                to,
                step,
                to_local,
                step_local,
            }
        } else {
            ForIn::Coll {
                lval,
                expr,
                iter_local: self.hidden_local(b".iter"),
            }
        };
        Some(Expr::ForIn(Box::new(for_in)))
    }

    /// Read the list after `is in` or `not in`, from `in` on.
    fn parse_inlist(&mut self) -> Option<Vec<Expr>> {
        if self.next() == Tok::LPar {
            self.next();
        } else {
            self.error_curtok("missing '(' after 'in' operator - found \"%\"");
        }
        let mut args = Vec::new();
        loop {
            if self.cur() == Tok::RPar {
                break;
            }
            args.push(self.parse_asi_expr()?);
            match self.cur() {
                Tok::RPar => break,
                Tok::Comma => {
                    self.next();
                }
                Tok::Sem | Tok::LBrace | Tok::RBrace | Tok::DStrMid | Tok::DStrEnd => {
                    self.error_curtok("expected ')' at end of 'in' list, but found \"%\"");
                    return Some(args);
                }
                _ => {
                    self.error_curtok("expected ',' or ')' in 'in' list, but found \"%\"");
                    if self.cur() == Tok::Eof {
                        return Some(args);
                    }
                }
            }
        }
        self.next();
        Some(args)
    }

    fn parse_foreach(&mut self) -> Parsed<Option<Stmt>> {
        let (file, line) = self.tok.last_pos();
        let scope = self.enter_scope();
        if self.next() == Tok::LPar {
            self.next();
        } else {
            self.error_curtok("expected '(' after \"foreach\", but found \"%\"");
        }
        match self.cur() {
            Tok::Local => {
                self.create_scope_local_symtab();
                if self.next() == Tok::Sym {
                    let name = self.cur_text();
                    self.add_local(&name);
                } else {
                    self.error_curtok(
                        "expected symbol name in local variable declaration, but found \"%\"",
                    );
                }
            }
            Tok::LPar | Tok::Sym => {}
            _ => {
                self.error_curtok("missing expression in \"foreach\" - found \"%\"");
                return Ok(None);
            }
        }
        let Some(iter) = self.parse_expr() else {
            return Ok(None);
        };
        if self.cur_is_sym(b"in") {
            self.next();
        } else {
            self.error_curtok("\"in\" required in \"foreach\" - found \"%\"");
            match self.cur() {
                Tok::LBrace | Tok::RBrace | Tok::Sem | Tok::Eof => return Ok(None),
                Tok::RPar => {
                    self.next();
                }
                _ => {}
            }
        }
        let Some(coll) = self.parse_expr() else {
            return Ok(None);
        };
        if self.cur() == Tok::RPar {
            self.next();
        } else {
            self.error_curtok("missing ')' at end of \"foreach\" expression - found \"%\"");
        }
        let symtab = self.local_symtab;
        let iter_local = self.hidden_local(b".iter");
        let id = self.enclose(false);
        let body = self.parse_stm(false, false);
        self.enclosing.pop();
        let body = body?.map(Box::new);
        self.leave_scope(scope);
        Ok(Some(Stmt {
            kind: StmtKind::Foreach {
                iter,
                coll,
                body,
                symtab,
                id,
                iter_local,
            },
            file,
            line,
        }))
    }

    fn parse_while(&mut self) -> Parsed<Option<Stmt>> {
        if self.next() == Tok::LPar {
            self.next();
        } else {
            self.error_curtok("missing '(' after \"while\" - found \"%\"");
        }
        let Some(cond) = self.parse_cond_expr() else {
            return Err(Stop);
        };
        // The statement takes its position from where the parser is
        // when it is made, which is here.
        let (file, line) = (self.cur_file, self.cur_line);
        if self.cur() == Tok::RPar {
            self.next();
        } else {
            self.error_curtok("missing ')' after \"while\" expression - found \"%\"");
        }
        let id = self.enclose(false);
        let body = self.parse_stm(false, false);
        self.enclosing.pop();
        let body = body?.map(Box::new);
        Ok(Some(Stmt {
            kind: StmtKind::While { cond, body, id },
            file,
            line,
        }))
    }

    fn parse_do_while(&mut self) -> Parsed<Option<Stmt>> {
        let (file, line) = (self.cur_file, self.cur_line);
        self.next();
        let id = self.enclose(false);
        let body = self.parse_stm(false, false);
        self.enclosing.pop();
        let body = body?.map(Box::new);
        if self.cur() == Tok::While {
            self.next();
        } else {
            self.error_curtok("expected \"while\" in \"do\" statement, but found \"%\"");
        }
        if self.cur() == Tok::LPar {
            self.next();
        } else {
            self.error_curtok("missing '(' after \"while\" - found \"%\"");
        }
        let Some(cond) = self.parse_cond_expr() else {
            return Err(Stop);
        };
        if self.cur() == Tok::RPar {
            self.next();
        } else {
            self.error_curtok("missing ')' after \"while\" expression - found \"%\"");
        }
        self.parse_req_sem()?;
        Ok(Some(Stmt {
            kind: StmtKind::DoWhile { body, cond, id },
            file,
            line,
        }))
    }

    /// Give the function being read a label. Nothing comes back if it
    /// has one of that name already.
    fn add_code_label(&mut self, name: &[u8], file: usize, line: u32) -> Option<usize> {
        if self.labels.iter().any(|label| label.name == name) {
            self.error(format!(
                "code label \"{}\" already defined in this function or method",
                Self::name(name)
            ));
            return None;
        }
        self.labels.push(LabelSym {
            name: name.to_vec(),
            flags: std::cell::Cell::new(0),
            file,
            line,
            within: self.enclosing.clone(),
        });
        Some(self.labels.len() - 1)
    }

    fn parse_switch(&mut self) -> Parsed<Option<Stmt>> {
        let (file, line) = (self.cur_file, self.cur_line);
        if self.next() == Tok::LPar {
            self.next();
        } else {
            self.error_curtok("missing '(' after \"switch\" - found \"%\"");
        }
        let Some(expr) = self.parse_expr() else {
            return Err(Stop);
        };
        if self.cur() == Tok::RPar {
            self.next();
        } else {
            self.error_curtok("missing ')' after \"switch\" expression - found \"%\"");
        }
        if self.cur() == Tok::LBrace {
            self.next();
        } else {
            self.error_curtok("missing '{' after \"switch\" expression - found \"%\"");
        }
        self.switches.push(SwitchInfo::default());
        // Nothing before the first `case` or `default` could be reached.
        // Such statements are read and dropped.
        let mut unreachable_shown = false;
        loop {
            match self.cur() {
                Tok::Case | Tok::Default | Tok::RBrace => break,
                Tok::Eof => {
                    self.error("end of file in \"switch\" body");
                    return Err(Stop);
                }
                _ => {
                    if !unreachable_shown && !self.syntax_only {
                        self.error(
                            "code before first \"case\" or \"default\" label in \"switch\" is not allowed",
                        );
                        unreachable_shown = true;
                    }
                    self.parse_stm(true, false)?;
                }
            }
        }
        let id = self.enclose(false);
        let body = self.parse_compound(false, true, true, false);
        self.enclosing.pop();
        let body = Box::new(body?);
        let info = self.switches.pop().expect("the switch being read");
        Ok(Some(Stmt {
            kind: StmtKind::Switch {
                expr,
                body,
                case_cnt: info.case_cnt,
                has_default: info.has_default,
                id,
            },
            file,
            line,
        }))
    }

    fn parse_case(&mut self) -> Parsed<Option<Stmt>> {
        let (file, line) = (self.cur_file, self.cur_line);
        self.next();
        let Some(expr) = self.parse_expr() else {
            return Err(Stop);
        };
        if self.cur() == Tok::Colon {
            self.next();
        } else {
            self.error_curtok("missing ':' after \"case\" expression - found \"%\"");
        }
        // What follows is still directly in the switch: another `case`
        // can come straight after.
        let stm = self.parse_stm(true, false)?.map(Box::new);
        if let Some(info) = self.switches.last_mut() {
            info.case_cnt += 1;
        }
        Ok(Some(Stmt {
            kind: StmtKind::Case { expr, stm },
            file,
            line,
        }))
    }

    fn parse_default(&mut self) -> Parsed<Option<Stmt>> {
        let (file, line) = (self.cur_file, self.cur_line);
        if self.switches.last().is_some_and(|info| info.has_default) {
            self.error("this \"switch\" already has a \"default:\" label");
        }
        if let Some(info) = self.switches.last_mut() {
            info.has_default = true;
        }
        if self.next() == Tok::Colon {
            self.next();
        } else {
            self.error_curtok("missing ':' after \"default\" - found \"%\"");
        }
        let stm = self.parse_stm(true, false)?.map(Box::new);
        Ok(Some(Stmt {
            kind: StmtKind::Default(stm),
            file,
            line,
        }))
    }

    fn parse_goto(&mut self) -> Parsed<Option<Stmt>> {
        let stmt = if self.next() == Tok::Sym {
            let stmt = self.stmt(StmtKind::Goto(self.cur_text()));
            self.next();
            Some(stmt)
        } else {
            // The reference crashes here, but at the end of the source,
            // where it shows the token as nothing.
            let found = if self.cur() == Tok::Eof {
                String::new()
            } else {
                self.tok.cur_token_shown()
            };
            self.error(format!(
                "label name expected after \"goto\", but found \"{found}\""
            ));
            None
        };
        self.parse_req_sem()?;
        Ok(stmt)
    }

    /// Start reading the inside of a statement that jumps have to know
    /// about. The caller pops it from `enclosing` when the inside has
    /// been read.
    fn enclose(&mut self, is_finally: bool) -> usize {
        self.next_enclosure_id += 1;
        let parent = self.enclosing.last().map(|enclosure| enclosure.id);
        self.prog
            .enclosure_parents
            .insert(self.next_enclosure_id, parent);
        self.enclosing.push(Enclosure {
            id: self.next_enclosure_id,
            is_finally,
        });
        self.next_enclosure_id
    }

    fn parse_try(&mut self) -> Parsed<Option<Stmt>> {
        let (file, line) = (self.cur_file, self.cur_line);
        let id = self.enclose(false);
        self.next();
        let body = self.parse_stm(false, false);
        self.enclosing.pop();
        let body = body?.map(Box::new);

        let mut catches = Vec::new();
        while self.cur() == Tok::Catch {
            // The clause has a scope of its own, for its variable.
            let scope = self.enter_scope();
            self.create_scope_local_symtab();
            let (catch_file, catch_line) = self.tok.last_pos();
            let mut class = None;
            let mut var = None;
            if self.next() == Tok::LPar {
                self.next();
            } else {
                self.error_curtok("missing '(' after \"catch\" - found \"%\"");
            }
            if self.cur() == Tok::Sym {
                class = Some(self.cur_text());
                self.next();
            } else {
                self.error_curtok("expected class name in \"catch\" clause - found \"%\"");
                if self.cur() != Tok::RPar {
                    self.next();
                }
            }
            if self.cur() == Tok::Sym {
                // Entering the clause gives the variable its value, and
                // nobody minds if it is never looked at.
                let name = self.cur_text();
                var = self.add_local(&name);
                if let Some(id) = var {
                    self.prog.locals[id].val_assigned = true;
                    self.prog.locals[id].val_used = true;
                }
                self.next();
            } else {
                self.error_curtok("expected variable name in \"catch\" clause - found \"%\"");
                if self.cur() != Tok::RPar {
                    self.next();
                }
            }
            if self.cur() == Tok::RPar {
                self.next();
            } else {
                self.error_curtok("missing ')' after \"catch\" variable name - found \"%\"");
            }
            // A block here uses the clause's scope instead of starting
            // one.
            let catch_body = self.parse_stm(false, true);
            self.leave_scope(scope);
            let catch_body = catch_body?.map(Box::new);
            if let (Some(class), Some(var)) = (class, var) {
                catches.push(Catch {
                    class,
                    var,
                    body: catch_body,
                    // The reference notes the scope after leaving the
                    // clause's own, so this is the one around it.
                    symtab: self.local_symtab,
                    file: catch_file,
                    line: catch_line,
                });
            }
        }

        let mut finally = None;
        if self.cur() == Tok::Finally {
            let scope = self.enter_scope();
            // Two locals: one for where to go back to, and one for an
            // exception that is passing through.
            let jsr_local = self.alloc_local();
            let exc_local = self.alloc_local();
            let (fin_file, fin_line) = self.tok.last_pos();
            self.next();
            let fin_id = self.enclose(true);
            let fin_body = self.parse_stm(false, false);
            self.enclosing.pop();
            self.leave_scope(scope);
            finally = Some(Box::new(Finally {
                id: fin_id,
                body: fin_body?.map(Box::new),
                exc_local,
                jsr_local,
                file: fin_file,
                line: fin_line,
            }));
        }
        if catches.is_empty() && finally.is_none() {
            self.tok.log_at(
                file,
                line,
                Severity::Error,
                "\"try\" statement has no \"catch\" or \"finally\" clauses".to_owned(),
            );
        }
        Ok(Some(Stmt {
            kind: StmtKind::Try {
                id,
                body,
                catches,
                finally,
            },
            file,
            line,
        }))
    }

    fn parse_throw(&mut self) -> Parsed<Option<Stmt>> {
        self.next();
        let Some(expr) = self.parse_expr() else {
            return Err(Stop);
        };
        let stmt = self.stmt(StmtKind::Throw(expr));
        self.parse_req_sem()?;
        Ok(Some(stmt))
    }

    fn parse_break_or_continue(&mut self, is_break: bool) -> Parsed<Option<Stmt>> {
        let (file, line) = (self.cur_file, self.cur_line);
        let mut label = None;
        match self.next() {
            Tok::Sym => {
                label = Some(self.cur_text());
                self.next();
            }
            Tok::Sem | Tok::LBrace | Tok::RBrace => {}
            _ => {
                let word = if is_break { "break" } else { "continue" };
                self.error_curtok(&format!(
                    "label name expected after \"{word}\", but found \"%\""
                ));
            }
        }
        self.parse_req_sem()?;
        let kind = if is_break {
            StmtKind::Break(label)
        } else {
            StmtKind::Continue(label)
        };
        Ok(Some(Stmt { kind, file, line }))
    }

    // ---- expressions ----

    /// Read an expression, commas and all.
    fn parse_expr(&mut self) -> Option<Expr> {
        self.parse_binary_level(0)
    }

    /// Read a condition. An assignment there is probably a slip for a
    /// comparison.
    fn parse_cond_expr(&mut self) -> Option<Expr> {
        let cond = self.parse_expr()?;
        if matches!(cond, Expr::Assign(AsiOp::Simple, ..)) && !self.syntax_only {
            self.tok.log_warning(
                "assignment in condition (possible use of '=' where '==' was intended)",
            );
        }
        Some(cond)
    }

    /// Read an expression in which a comma ends it: an argument, or an
    /// initial value.
    fn parse_asi_expr(&mut self) -> Option<Expr> {
        let lhs = self.parse_if_expr()?;
        let op = match self.cur() {
            Tok::PlusEq => AsiOp::Add,
            Tok::MinEq => AsiOp::Sub,
            Tok::TimesEq => AsiOp::Mul,
            Tok::DivEq => AsiOp::Div,
            Tok::ModEq => AsiOp::Mod,
            Tok::AndEq => AsiOp::BAnd,
            Tok::OrEq => AsiOp::BOr,
            Tok::XorEq => AsiOp::BXor,
            Tok::ShlEq => AsiOp::Shl,
            Tok::AShrEq => AsiOp::AShr,
            Tok::LShrEq => AsiOp::LShr,
            Tok::Eq => AsiOp::Simple,
            _ => return Some(lhs),
        };
        if !lhs.check_lvalue() {
            self.error(format!(
                "invalid lvalue - cannot assign to expression on left of \"{}\"",
                op.text()
            ));
        }
        self.next();
        // Assignments group from the right.
        let rhs = self.parse_asi_expr()?;
        Some(Expr::Assign(op, Box::new(lhs), Box::new(rhs)))
    }

    fn parse_if_expr(&mut self) -> Option<Expr> {
        let first = self.parse_ifnil_expr()?;
        if self.cur() != Tok::Question {
            return Some(first);
        }
        self.next();
        // The middle can be anything, a comma expression included.
        let second = self.parse_expr()?;
        if self.cur() != Tok::Colon {
            self.error("missing ':' in '?' conditional expression");
            if self.cur() == Tok::Eof {
                return None;
            }
        }
        self.next();
        let third = self.parse_asi_expr()?;
        Some(match first.as_const() {
            Some(value) => {
                if value.is_true() {
                    second
                } else {
                    third
                }
            }
            None => Expr::If(Box::new(first), Box::new(second), Box::new(third)),
        })
    }

    fn parse_ifnil_expr(&mut self) -> Option<Expr> {
        let first = self.parse_binary_level(1)?;
        if self.cur() != Tok::QQ {
            return Some(first);
        }
        self.next();
        let second = self.parse_asi_expr()?;
        Some(match first.as_const() {
            Some(value) => {
                if value.value == Value::Nil {
                    second
                } else {
                    first
                }
            }
            None => Expr::IfNil(Box::new(first), Box::new(second)),
        })
    }

    /// The operators of one level of precedence, from the loosest (the
    /// comma) to the tightest (multiplication).
    fn level_ops(level: usize) -> &'static [(Tok, BinOp)] {
        match level {
            0 => &[(Tok::Comma, BinOp::Comma)],
            1 => &[(Tok::OrOr, BinOp::Or)],
            2 => &[(Tok::AndAnd, BinOp::And)],
            3 => &[(Tok::Or, BinOp::BOr)],
            4 => &[(Tok::Xor, BinOp::BXor)],
            5 => &[(Tok::And, BinOp::BAnd)],
            6 => &[(Tok::EqEq, BinOp::Eq), (Tok::Ne, BinOp::Ne)],
            7 => &[
                (Tok::Gt, BinOp::Gt),
                (Tok::Ge, BinOp::Ge),
                (Tok::Lt, BinOp::Lt),
                (Tok::Le, BinOp::Le),
            ],
            8 => &[
                (Tok::Shl, BinOp::Shl),
                (Tok::AShr, BinOp::AShr),
                (Tok::LShr, BinOp::LShr),
            ],
            9 => &[(Tok::Plus, BinOp::Add), (Tok::Minus, BinOp::Sub)],
            _ => &[
                (Tok::Times, BinOp::Mul),
                (Tok::Div, BinOp::Div),
                (Tok::Mod, BinOp::Mod),
            ],
        }
    }

    const LAST_LEVEL: usize = 10;

    fn parse_operand_of(&mut self, level: usize) -> Option<Expr> {
        match level {
            // Under the comma is assignment, and under `||` there is
            // nothing left but the operators with two operands.
            0 => self.parse_asi_expr(),
            Self::LAST_LEVEL => self.parse_unary(),
            _ => self.parse_binary_level(level + 1),
        }
    }

    fn parse_binary_level(&mut self, level: usize) -> Option<Expr> {
        let mut lhs = self.parse_operand_of(level)?;
        loop {
            let cur = self.cur();
            let found = Self::level_ops(level)
                .iter()
                .find(|(tok, _)| *tok == cur)
                .map(|(_, op)| *op);
            let Some(op) = found else {
                // `is in` and `not in` are tests of equality too. Neither
                // word is a keyword.
                if level == 6 && (self.cur_is_sym(b"is") || self.cur_is_sym(b"not")) {
                    let not = self.cur_is_sym(b"not");
                    self.next();
                    if self.cur_is_sym(b"in") {
                        let args = self.parse_inlist()?;
                        lhs = Expr::IsIn {
                            left: Box::new(lhs),
                            args,
                            not,
                            hit: false,
                        };
                        continue;
                    }
                    self.tok.unget_token();
                }
                return Some(lhs);
            };
            self.next();
            let rhs = self.parse_operand_of(level)?;
            lhs = self.fold_or_build(op, lhs, rhs);
        }
    }

    fn fold_or_build(&mut self, op: BinOp, lhs: Expr, rhs: Expr) -> Expr {
        let mut errors = Vec::new();
        let result = fold::binary(op, lhs, rhs, &mut errors);
        for message in errors {
            self.error(message);
        }
        match result {
            Folded::Yes(expr) => {
                if let Some(len) = expr.const_list_len() {
                    self.prog.max_list_cnt = self.prog.max_list_cnt.max(len);
                }
                expr
            }
            Folded::No(lhs, rhs) => Expr::Binary(op, Box::new(lhs), Box::new(rhs)),
        }
    }

    fn parse_unary(&mut self) -> Option<Expr> {
        let op = self.cur();
        match op {
            Tok::And => {
                self.next();
                self.parse_addr()
            }
            Tok::Delete => {
                self.unsupported("\"delete\"");
                self.next();
                self.parse_unary()
            }
            Tok::Not | Tok::BNot | Tok::Plus | Tok::Minus | Tok::Inc | Tok::Dec => {
                self.next();
                let sub = self.parse_unary()?;
                Some(match op {
                    Tok::Not => match fold::not(sub) {
                        Ok(folded) => folded,
                        Err(sub) => Expr::Unary(UnOp::Not, Box::new(sub)),
                    },
                    Tok::BNot => self.parse_bnot(sub),
                    Tok::Plus => self.parse_pos(sub),
                    Tok::Minus => self.parse_neg(sub),
                    Tok::Inc => self.parse_inc_dec(AsiOp::PreInc, sub),
                    _ => self.parse_inc_dec(AsiOp::PreDec, sub),
                })
            }
            _ => self.parse_postfix(),
        }
    }

    fn parse_bnot(&mut self, sub: Expr) -> Expr {
        match sub {
            Expr::Const(mut value) => {
                if let Value::Int(int) = value.value {
                    value.value = Value::Int(!int);
                } else {
                    self.error("unary '~' operator requires numeric value in constant expression");
                }
                Expr::Const(value)
            }
            other => Expr::Unary(UnOp::BNot, Box::new(other)),
        }
    }

    fn parse_pos(&mut self, sub: Expr) -> Expr {
        match sub {
            Expr::Const(value) => {
                if !matches!(value.value, Value::Int(_)) {
                    self.error("unary '+' operator requires numeric value in constant expression");
                }
                Expr::Const(value)
            }
            other => Expr::Unary(UnOp::Pos, Box::new(other)),
        }
    }

    fn parse_neg(&mut self, sub: Expr) -> Expr {
        match sub {
            Expr::Const(mut value) => {
                match value.value {
                    Value::Int(i32::MIN) => {
                        self.unsupported("a number too large for an integer");
                    }
                    Value::Int(int) => value.value = Value::Int(-int),
                    _ => self
                        .error("unary '-' operator requires numeric value in constant expression"),
                }
                Expr::Const(value)
            }
            other => Expr::Unary(UnOp::Neg, Box::new(other)),
        }
    }

    fn parse_inc_dec(&mut self, op: AsiOp, sub: Expr) -> Expr {
        if !sub.check_lvalue() {
            self.error(format!(
                "invalid lvalue - cannot apply \"{}\" operator to expression",
                op.text()
            ));
        }
        Expr::IncDec(op, Box::new(sub))
    }

    fn parse_postfix(&mut self) -> Option<Expr> {
        let mut sub = self.parse_primary()?;
        loop {
            match self.cur() {
                Tok::LPar => sub = self.parse_call(sub)?,
                Tok::LBrack => sub = self.parse_subscript(sub)?,
                Tok::Dot => sub = self.parse_member(sub)?,
                Tok::Inc => {
                    self.next();
                    sub = self.parse_inc_dec(AsiOp::PostInc, sub);
                }
                Tok::Dec => {
                    self.next();
                    sub = self.parse_inc_dec(AsiOp::PostDec, sub);
                }
                _ => return Some(sub),
            }
        }
    }

    /// Read `[index]` after an expression.
    fn parse_subscript(&mut self, lhs: Expr) -> Option<Expr> {
        self.next();
        let index = self.parse_expr()?;
        if self.cur() == Tok::RBrack {
            self.next();
        } else {
            self.error_curtok("expected ']' at end of subscript, but found \"%\"");
        }
        let mut errors = Vec::new();
        let folded = fold::subscript(&lhs, &index, &mut errors);
        for message in errors {
            self.error(message);
        }
        Some(folded.unwrap_or_else(|| Expr::Index(Box::new(lhs), Box::new(index))))
    }

    /// Read a list. The current token is the opening bracket.
    fn parse_list(&mut self) -> Option<Expr> {
        const MISSING_RBRACK: &str = "expected ']' or a list element, but found \"%\"";
        self.next();
        let mut elements = Vec::new();
        'list: loop {
            match self.cur() {
                Tok::RBrack => {
                    self.next();
                    break;
                }
                Tok::Eof | Tok::RBrace | Tok::Sem | Tok::DStrMid | Tok::DStrEnd => {
                    self.error_curtok(MISSING_RBRACK);
                    break;
                }
                Tok::RPar => {
                    self.error("found extraneous ')' in list; ignored");
                    self.next();
                }
                _ => {}
            }
            elements.push(self.parse_asi_expr()?);
            match self.cur() {
                Tok::Comma => {
                    self.next();
                    if self.cur() == Tok::RBrack {
                        self.error(
                            "extra list element expected after comma, but found end of list",
                        );
                        break 'list;
                    }
                }
                Tok::Arrow => {
                    self.unsupported("a lookup table list");
                    self.next();
                }
                Tok::RBrack => {
                    self.next();
                    break;
                }
                Tok::Eof | Tok::LBrace | Tok::RBrace | Tok::Sem | Tok::DStrMid | Tok::DStrEnd => {
                    self.error_curtok(MISSING_RBRACK);
                    break;
                }
                _ => {
                    self.error_curtok("expected ',' separating list elements, but found \"%\"");
                }
            }
        }
        self.prog.max_list_cnt = self.prog.max_list_cnt.max(elements.len());
        Some(Expr::list(elements))
    }

    /// Read a call's arguments. The current token is the opening
    /// parenthesis.
    fn parse_arg_list(&mut self) -> Option<Vec<Arg>> {
        self.next();
        let mut args = Vec::new();
        loop {
            if self.cur() == Tok::RPar {
                break;
            }
            // `name: value` passes the value by name.
            let mut name = None;
            if self.cur() == Tok::Sym {
                let text = self.cur_text();
                if self.next() == Tok::Colon {
                    name = Some(text);
                    self.next();
                } else {
                    self.tok.unget_token();
                }
            }
            let expr = self.parse_asi_expr()?;
            let mut arg = Arg {
                expr,
                varargs: false,
                name,
            };
            if self.cur() == Tok::Ellipsis {
                if arg.name.is_some() {
                    self.error("a pass-by-name argument value cannot use an ellipsis '...'");
                }
                self.next();
                arg.varargs = true;
            }
            args.push(arg);
            match self.cur() {
                Tok::RPar => break,
                Tok::Comma => {
                    self.next();
                }
                Tok::Sem | Tok::LBrace | Tok::RBrace | Tok::DStrMid | Tok::DStrEnd => {
                    self.error_curtok("expected ')' at end of argument list, but found \"%\"");
                    return Some(args);
                }
                _ => {
                    self.error_curtok("expected ',' or ')' in argument list, but found \"%\"");
                    if self.cur() == Tok::Eof {
                        return Some(args);
                    }
                }
            }
        }
        self.next();
        Some(args)
    }

    fn parse_call(&mut self, lhs: Expr) -> Option<Expr> {
        let args = self.parse_arg_list()?;
        if let Expr::Sym(name) = &lhs {
            if name == b"defined" || name == b"__objref" {
                self.unsupported(&format!("\"{}\"", Self::name(name)));
            }
        }
        if args.iter().filter(|arg| arg.name.is_none()).count() > 127 {
            self.error("function call argument list exceeds machine limit (maximum 127)");
        }
        Some(Expr::Call(Box::new(lhs), args))
    }

    fn parse_primary(&mut self) -> Option<Expr> {
        loop {
            let value = match self.cur() {
                Tok::LPar => {
                    self.next();
                    let sub = self.parse_expr()?;
                    if self.cur() == Tok::RPar {
                        self.next();
                    } else {
                        self.error_curtok("expected ')' but found \"%\"");
                    }
                    return Some(sub);
                }
                Tok::Nil => Value::Nil,
                Tok::True => Value::True,
                Tok::Int => Value::Int(self.tok.cur_token().int_val as i32),
                Tok::SStr => Value::SStr(self.cur_text()),
                Tok::DStr => {
                    // Displaying a string might go through `self`.
                    self.self_referenced = true;
                    let text = self.cur_text();
                    self.next();
                    return Some(Expr::DStr(text));
                }
                Tok::SelfKw => {
                    self.self_referenced = true;
                    self.next();
                    return Some(Expr::SelfRef);
                }
                Tok::TargetProp => {
                    self.full_ctx_referenced = true;
                    self.next();
                    return Some(Expr::CtxEle(1));
                }
                Tok::TargetObj => {
                    self.full_ctx_referenced = true;
                    self.next();
                    return Some(Expr::CtxEle(2));
                }
                Tok::DefiningObj => {
                    self.full_ctx_referenced = true;
                    self.next();
                    return Some(Expr::CtxEle(3));
                }
                Tok::LBrace => return self.parse_anon_func(true),
                Tok::Function | Tok::Method => return self.parse_anon_func(false),
                Tok::Invokee => {
                    self.next();
                    return Some(Expr::CtxEle(4));
                }
                Tok::Inherited => return self.parse_inherited(),
                Tok::New => {
                    let transient = match self.next() {
                        Tok::Function | Tok::Method => return self.parse_anon_func(false),
                        Tok::Transient => {
                            self.next();
                            true
                        }
                        _ => false,
                    };
                    let mut sub = self.parse_primary()?;
                    if self.cur() == Tok::LPar {
                        sub = self.parse_call(sub)?;
                    }
                    return Some(Expr::New(Box::new(sub), transient));
                }
                Tok::DStrStart => {
                    self.self_referenced = true;
                    return self.parse_embedding(true);
                }
                Tok::SStrStart => return self.parse_embedding(false),
                Tok::LBrack => return self.parse_list(),
                Tok::Sym => {
                    let name = self.cur_text();
                    self.next();
                    return Some(match self.find_local(&name) {
                        Some(id) => {
                            self.prog.locals[id].referenced = true;
                            Expr::Local(id)
                        }
                        None => {
                            // It might be a property of `self`.
                            self.self_referenced = true;
                            Expr::Sym(name)
                        }
                    });
                }
                Tok::RPar => {
                    self.error("unexpected ')' found - ignored");
                    self.next();
                    continue;
                }
                Tok::RBrack => {
                    self.error("unexpected ']' found - ignored");
                    self.next();
                    continue;
                }
                Tok::DStrMid | Tok::DStrEnd | Tok::Sem | Tok::RBrace => {
                    // The statement seems to be over where an operand
                    // should be. Stay here, so that whoever asked can
                    // carry on from this token.
                    self.error_curtok("expected an operand, but found \"%\"");
                    return Some(Expr::constant(Value::Int(
                        self.tok.cur_token().int_val as i32,
                    )));
                }
                Tok::Float
                | Tok::BigInt
                | Tok::ReStr
                | Tok::Object
                | Tok::Replaced
                | Tok::ArgCount
                | Tok::Delegated => {
                    let shown = self.tok.cur_token_shown();
                    self.unsupported(&format!("\"{shown}\" in an expression"));
                    self.next();
                    return Some(Expr::constant(Value::Nil));
                }
                _ => {
                    self.error_curtok(
                        "expected integer, string, symbol, '[', or '(', but found \"%\"",
                    );
                    self.next();
                    Value::Int(self.tok.cur_token().int_val as i32)
                }
            };
            self.next();
            return Some(Expr::constant(value));
        }
    }
}

// ---- objects ----
impl Parser<'_, '_> {
    /// Read `.prop` after an expression, with arguments if there are
    /// any. The property is a name, or an expression in parentheses
    /// that says which property.
    /// Read `intrinsic class Name 'vm-name/version': Superclass { ... }`,
    /// from `class` on: the name the program has for a class built into
    /// the virtual machine, and the names of the class's properties, in
    /// the virtual machine's order.
    fn parse_intrinsic_class(&mut self) -> Parsed<()> {
        let mut sym = None;
        let mut name = None;
        if self.next() == Tok::Sym {
            let text = self.cur_text();
            match self.prog.globals.get(&text) {
                Some(Global::Meta(meta)) if self.prog.metas[*meta].is_ext => {
                    sym = Some(*meta);
                    name = Some(text);
                }
                Some(_) => self.error_curtok(
                    "symbol \"%\" is already defined - can't redefine as intrinsic class",
                ),
                None => name = Some(text),
            }
            self.next();
        } else {
            self.error_curtok(
                "expected class name symbol after \"intrinsic class\", but found \"%\"",
            );
        }

        if self.cur() == Tok::SStr {
            if let Some(name) = name {
                let index = match sym {
                    Some(index) => {
                        self.prog.metas[index].is_ext = false;
                        index
                    }
                    None => {
                        self.prog.metas.push(MetaSym {
                            name: name.clone(),
                            obj_id: self.prog.next_obj_id,
                            meta: 0,
                            props: Vec::new(),
                            is_ext: false,
                            seq: self.prog.next_seq,
                        });
                        self.prog.next_seq += 1;
                        self.prog.next_obj_id += 1;
                        let index = self.prog.metas.len() - 1;
                        self.prog.globals.insert(name, Global::Meta(index));
                        index
                    }
                };
                let vm_name = self.cur_text();
                let meta = self.prog.find_or_add_meta(&vm_name, Some(index));
                if let Some(other) = self.prog.meta_table[meta].sym {
                    if other != index {
                        let other = Self::name(&self.prog.metas[other].name);
                        self.error(format!(
                            "intrinsic class has been previously defined as \"{other}\""
                        ));
                    }
                }
                self.prog.metas[index].meta = meta;
                sym = Some(index);
            }
            self.next();
        } else {
            self.error_curtok(
                "expected class name string after class name symbol, but found \"%\"",
            );
        }

        if self.cur() == Tok::Colon {
            if self.next() != Tok::Sym {
                self.error_curtok(
                    "expected intrinsic superclass name after colon, but found \"%\"",
                );
            } else {
                match self.prog.globals.get(&self.cur_text()) {
                    None => {
                        self.error_curtok("intrinsic class superclass \"%\" is not defined");
                    }
                    Some(Global::Meta(_)) => {}
                    Some(_) => self
                        .error_curtok("intrinsic class superclass \"%\" is not an intrinsic class"),
                }
                self.next();
            }
        }
        if self.cur() == Tok::LBrace {
            self.next();
        } else {
            self.error_curtok("expected '{' after intrinsic class name, but found \"%\"");
        }

        const NEED_PROP: &str = "expected property name in intrinsic class list, but found \"%\"";
        loop {
            match self.cur() {
                Tok::RBrace => {
                    self.next();
                    return Ok(());
                }
                Tok::Eof => {
                    self.error("end of file in \"intrinsic class\" list - '}' is probably missing");
                    return Err(Stop);
                }
                Tok::Sem => {
                    self.next();
                }
                Tok::Static if self.next() != Tok::Sym => {
                    let shown = self.tok.cur_token_shown();
                    self.error(NEED_PROP.replacen('%', &shown, 1));
                }
                Tok::Sym | Tok::Static => {
                    let prop_name = self.cur_text();
                    if let Some(index) = sym {
                        let prop = match self.prog.globals.get(&prop_name) {
                            Some(Global::Prop(prop)) => Some(*prop),
                            Some(_) => {
                                self.error_curtok(
                                    "symbol \"%\" is already defined - can't redefine as property",
                                );
                                None
                            }
                            None => Some(self.prog.def_prop(&prop_name)),
                        };
                        if let Some(prop) = prop {
                            self.prog.metas[index].props.push(prop);
                        }
                    }
                    if self.next() == Tok::LPar {
                        // Parameters are for the reader; only that
                        // they parse matters.
                        self.next();
                        let mut formals = Formals {
                            count_only: true,
                            ..Formals::default()
                        };
                        self.parse_formal_list(&mut formals)?;
                    }
                    self.parse_req_sem()?;
                }
                _ => {
                    self.error_curtok(NEED_PROP);
                    self.next();
                }
            }
        }
    }

    /// Read `property a, b, c;`: names that are properties, whatever
    /// else the program does with them.
    /// Read `enum name, name;`, which makes each name a value unlike
    /// any other. `enum token` makes them kinds of token as well.
    fn parse_enum(&mut self) {
        const NEED_SEM: &str = "expected ';', but found \"%\"";
        const NEED_SYM: &str = "symbol expected in 'enum' list - found \"%\"";
        let mut is_token = false;
        self.next();
        if self.cur_is_sym(b"token") {
            is_token = true;
            self.next();
        }
        loop {
            match self.cur() {
                Tok::Sym => {
                    let name = self.cur_text();
                    match self.prog.globals.get(&name).copied() {
                        Some(Global::Enum(sym)) => {
                            if is_token {
                                self.prog.enums[sym].is_token = true;
                            }
                        }
                        Some(_) => self.error_curtok(
                            "symbol \"%\" is already defined - can't be used as an enum name",
                        ),
                        None => {
                            self.prog
                                .globals
                                .insert(name.clone(), Global::Enum(self.prog.enums.len()));
                            self.prog.enums.push(EnumSym {
                                name,
                                is_token,
                                seq: self.prog.next_seq,
                            });
                            self.prog.next_seq += 1;
                        }
                    }
                    match self.next() {
                        Tok::Comma => {
                            self.next();
                        }
                        Tok::Sem => {
                            self.next();
                            return;
                        }
                        Tok::LBrace | Tok::RBrace | Tok::Eof => {
                            self.error_curtok(NEED_SEM);
                            return;
                        }
                        _ => {
                            self.error_curtok(
                                "comma expected in 'enum' symbol list, but found \"%\"",
                            );
                            self.next();
                        }
                    }
                }
                Tok::LBrace | Tok::RBrace | Tok::Eof => {
                    self.error_curtok(NEED_SEM);
                    return;
                }
                Tok::Sem => {
                    self.error_curtok(NEED_SYM);
                    self.next();
                    return;
                }
                _ => {
                    self.error_curtok(NEED_SYM);
                    self.next();
                }
            }
        }
    }

    fn parse_property(&mut self) -> Parsed<()> {
        const NEED_COMMA: &str = "expected comma in 'property' list, but found \"%\"";
        self.next();
        loop {
            if self.cur() == Tok::Sym {
                let name = self.cur_text();
                self.look_up_prop(&name);
                match self.next() {
                    Tok::Comma => {
                        self.next();
                    }
                    Tok::Sem => {
                        self.next();
                        return Ok(());
                    }
                    Tok::Sym => self.error_curtok(NEED_COMMA),
                    Tok::LBrace | Tok::RBrace | Tok::Eof => {
                        self.error_curtok("expected ';', but found \"%\"");
                        return Ok(());
                    }
                    _ => {
                        self.error_curtok(NEED_COMMA);
                        match self.next() {
                            Tok::Comma => {
                                self.next();
                            }
                            Tok::Sem => {
                                self.next();
                                return Ok(());
                            }
                            _ => {}
                        }
                    }
                }
            } else {
                self.error_curtok("expected property name in 'property' list, but found \"%\"");
                match self.cur() {
                    Tok::Sem | Tok::LBrace | Tok::RBrace | Tok::Eof => return Ok(()),
                    Tok::Comma => {
                        self.next();
                    }
                    _ => {
                        if self.next() == Tok::Comma {
                            self.next();
                        }
                    }
                }
                if self.cur() == Tok::Sem {
                    self.next();
                    return Ok(());
                }
            }
        }
    }

    /// Read `export name;` or `export name 'external name';`: a
    /// property, object, or function the virtual machine is to know
    /// by name.
    fn parse_export(&mut self) {
        const NEED_SYM: &str = "expected symbol in 'export' statement, but found \"%\"";
        match self.next() {
            Tok::Sym => {
                let name = self.cur_text();
                let mut external = name.clone();
                match self.next() {
                    Tok::Sem => {
                        self.next();
                    }
                    Tok::SStr => {
                        external = self.cur_text();
                        if external.len() > 80 {
                            self.error_curtok("external name \"%\" in 'export' is too long");
                        }
                        // The semicolon is left for whatever reads
                        // next, which takes it for an empty statement.
                        if self.next() != Tok::Sem {
                            self.error_curtok("expected ';', but found \"%\"");
                        }
                    }
                    _ => self.error_curtok("expected ';', but found \"%\""),
                }
                self.prog.exports.push((name, external));
            }
            Tok::Sem | Tok::LBrace | Tok::RBrace | Tok::Eof => self.error_curtok(NEED_SYM),
            _ => {
                self.error_curtok(NEED_SYM);
                self.next();
            }
        }
    }

    /// Read `&name`, from after the `&`.
    fn parse_addr(&mut self) -> Option<Expr> {
        let sub = if self.cur() == Tok::Sym {
            // A name whose address is taken is a property's, if it is
            // nothing else, but one that a function can still claim.
            let name = self.cur_text();
            if !self.prog.globals.contains_key(&name) {
                let prop = self.prog.def_prop(&name);
                self.prog.props[prop].weak = true;
            }
            self.next();
            Expr::Sym(name)
        } else if self.cur() == Tok::Operator {
            self.unsupported("the address of an operator");
            self.next();
            return Some(Expr::constant(Value::Nil));
        } else {
            self.parse_unary()?
        };
        if !matches!(sub, Expr::Sym(_)) {
            self.error("invalid address expression - can't apply '&' operator");
        }
        Some(Expr::Addr(Box::new(sub)))
    }

    /// Read `inherited`, which is followed by `.prop` or stands for
    /// the method being overridden, and can name a superclass first.
    fn parse_inherited(&mut self) -> Option<Expr> {
        // What is inherited depends on how the method was called.
        self.full_ctx_referenced = true;
        let lhs = match self.next() {
            Tok::Sym => {
                let name = self.cur_text();
                self.next();
                Expr::InhClass(name)
            }
            Tok::Lt => {
                self.unsupported("\"inherited\" with a list of types");
                return Some(Expr::constant(Value::Nil));
            }
            _ => Expr::Inh,
        };
        self.parse_member(lhs)
    }

    fn parse_member(&mut self, lhs: Expr) -> Option<Expr> {
        if self.cur() != Tok::Dot {
            // `inherited(args)`: the property is the one being run.
            let args = if self.cur() == Tok::LPar {
                Some(self.parse_arg_list()?)
            } else {
                None
            };
            self.full_ctx_referenced = true;
            return Some(Expr::Member {
                obj: Box::new(lhs),
                prop: Box::new(Expr::CtxEle(1)),
                prop_is_expr: true,
                args,
            });
        }
        self.next();
        let mut prop_is_expr = false;
        let prop = match self.cur() {
            Tok::Sym => {
                // The name is a property's, whatever locals there are.
                let name = self.cur_text();
                self.next();
                Expr::Sym(name)
            }
            Tok::LPar => {
                self.next();
                prop_is_expr = true;
                let prop = self.parse_expr()?;
                if self.cur() == Tok::RPar {
                    self.next();
                } else {
                    self.error_curtok("expected ')' but found \"%\"");
                    if matches!(self.cur(), Tok::Sem | Tok::Eof) {
                        return Some(lhs);
                    }
                }
                prop
            }
            Tok::TargetProp => {
                self.next();
                prop_is_expr = true;
                self.full_ctx_referenced = true;
                Expr::CtxEle(1)
            }
            _ => {
                self.error_curtok(
                    "expected property name or parenthesized expression after '.', found \"%\"",
                );
                self.next();
                return Some(lhs);
            }
        };
        let args = if self.cur() == Tok::LPar {
            Some(self.parse_arg_list()?)
        } else {
            None
        };
        Some(Expr::Member {
            obj: Box::new(lhs),
            prop: Box::new(prop),
            prop_is_expr,
            args,
        })
    }

    /// The property a name stands for in an object definition. A name
    /// that is nothing yet becomes a property.
    fn look_up_prop(&mut self, name: &[u8]) -> Option<usize> {
        match self.prog.globals.get(name) {
            Some(Global::Prop(prop)) => {
                self.prog.props[*prop].weak = false;
                Some(*prop)
            }
            Some(_) => {
                self.error(format!(
                    "symbol \"{}\" is already defined - can't redefine as property",
                    Self::name(name)
                ));
                None
            }
            None => Some(self.prog.def_prop(name)),
        }
    }

    /// Whether an object has another among its superclasses, however
    /// far up.
    fn has_superclass(&self, obj: usize, other: usize, depth: usize) -> bool {
        if depth > 100 {
            return false;
        }
        self.prog.objs[obj].sc_names.iter().any(|name| {
            matches!(self.prog.globals.get(name), Some(Global::Obj(sc))
                if *sc == other || self.has_superclass(*sc, other, depth + 1))
        })
    }

    /// Read `name: superclasses properties ;`, from the name on.
    fn parse_object(&mut self, is_class: bool) -> Parsed<()> {
        let name = self.cur_text();
        let sym = match self.prog.globals.get(&name).copied() {
            Some(Global::Obj(obj)) if self.prog.objs[obj].is_extern => {
                self.prog.objs[obj].is_extern = false;
                Some(obj)
            }
            Some(Global::Obj(_)) => {
                self.error(format!(
                    "object \"{}\" is already defined - can't redefine as object",
                    Self::name(&name)
                ));
                None
            }
            Some(_) => {
                self.error_curtok("symbol \"%\" is already defined - can't redefine as object");
                None
            }
            None => {
                self.prog.objs.push(ObjSym {
                    name: name.clone(),
                    id: self.prog.next_obj_id,
                    is_extern: false,
                    is_class: false,
                    sc_names: Vec::new(),
                });
                self.prog.next_obj_id += 1;
                self.prog
                    .globals
                    .insert(name, Global::Obj(self.prog.objs.len() - 1));
                Some(self.prog.objs.len() - 1)
            }
        };
        if let (true, Some(obj)) = (is_class, sym) {
            self.prog.objs[obj].is_class = true;
        }
        self.next();
        self.parse_object_body(sym, is_class, false)
    }

    /// Read an object without a name: `superclasses properties ;`.
    fn parse_anon_object(&mut self) -> Parsed<()> {
        self.prog.objs.push(ObjSym {
            name: b".anon".to_vec(),
            id: self.prog.next_obj_id,
            is_extern: false,
            is_class: false,
            sc_names: Vec::new(),
        });
        self.prog.next_obj_id += 1;
        self.parse_object_body(Some(self.prog.objs.len() - 1), false, true)
    }

    fn parse_object_body(
        &mut self,
        sym: Option<usize>,
        is_class: bool,
        is_anon: bool,
    ) -> Parsed<()> {
        let old_self_valid = self.self_valid;
        self.self_valid = true;
        let result = self.parse_object_body_in(sym, is_class, is_anon);
        self.self_valid = old_self_valid;
        result
    }

    fn parse_object_body_in(
        &mut self,
        sym: Option<usize>,
        is_class: bool,
        is_anon: bool,
    ) -> Parsed<()> {
        let mut def = ObjDef {
            sym,
            is_class,
            superclasses: Vec::new(),
            props: Vec::new(),
            file: 0,
            line: 0,
        };
        let (file, line) = self.tok.last_pos();
        (def.file, def.line) = (file, line);
        if is_anon {
            // The superclasses come first.
        } else if self.cur() == Tok::Colon {
            self.next();
        } else {
            self.error_curtok(
                "expected ':' after object name in object definition, but found \"%\"",
            );
        }
        if let Some(obj) = sym {
            self.prog.objs[obj].sc_names.clear();
        }
        self.parse_superclass_list(sym, &mut def);

        let mut braces = false;
        if self.cur() == Tok::LBrace {
            self.next();
            braces = true;
        }
        // Values with no property names: the object's classes say, in
        // templates, which properties they are for.
        if matches!(
            self.cur(),
            Tok::SStr
                | Tok::SStrStart
                | Tok::DStr
                | Tok::DStrStart
                | Tok::LBrack
                | Tok::At
                | Tok::Plus
                | Tok::Minus
                | Tok::Times
                | Tok::Div
                | Tok::Mod
                | Tok::Arrow
                | Tok::And
                | Tok::Not
                | Tok::BNot
                | Tok::Comma
        ) {
            self.unsupported("an object definition that uses a template");
            self.next();
        }
        if !braces && self.cur() == Tok::LBrace {
            braces = true;
            self.next();
        }

        // An object that is not a class knows where it comes in the
        // source.
        if !is_class {
            let order = Expr::constant(Value::Int(self.src_order_idx));
            self.src_order_idx += 1;
            def.props.push(ObjProp {
                prop: SRC_ORDER_PROP,
                value: PropValue::Expr(order),
                overwritable: true,
                is_static: false,
                file,
                line,
            });
        }

        self.parse_obj_prop_list(&mut def, braces, false)?;
        self.prog.top.push(Top::Obj(self.prog.objdefs.len()));
        self.prog.objdefs.push(def);
        Ok(())
    }

    fn parse_superclass_list(&mut self, sym: Option<usize>, def: &mut ObjDef) {
        loop {
            match self.cur() {
                Tok::Sym => {
                    let name = self.cur_text();
                    let circular = match (self.prog.globals.get(&name), sym) {
                        (Some(Global::Obj(sc)), Some(obj)) => {
                            *sc == obj || self.has_superclass(*sc, obj, 0)
                        }
                        _ => false,
                    };
                    if circular {
                        let obj_name = sym.map(|obj| self.prog.objs[obj].name.clone());
                        self.error(format!(
                            "circular class definition: {} is a subclass of {}",
                            Self::name(&name),
                            Self::name(&obj_name.unwrap_or_default())
                        ));
                    } else {
                        if let Some(obj) = sym {
                            self.prog.objs[obj].sc_names.push(name.clone());
                        }
                        def.superclasses.push(name);
                    }
                    if self.next() == Tok::Comma {
                        self.next();
                    } else {
                        return;
                    }
                }
                Tok::Object => {
                    if !def.superclasses.is_empty() {
                        self.error(
                            "superclasses cannot be specified with \"object\" as the base class",
                        );
                    }
                    self.next();
                    return;
                }
                _ => {
                    self.error_curtok(
                        "expected superclass name in object definition, but found \"%\"",
                    );
                    return;
                }
            }
        }
    }

    /// Read an object's properties, up to the semicolon that ends the
    /// definition, or the closing brace if it has braces.
    ///
    /// With `one_prop`, the list is what follows a definition found to
    /// have ended without saying so, and reading stops after the first
    /// property, as the reference's does: it goes by a flag that was
    /// set when the missing end was noticed.
    fn parse_obj_prop_list(
        &mut self,
        def: &mut ObjDef,
        braces: bool,
        one_prop: bool,
    ) -> Parsed<()> {
        let missing_end = if braces {
            "missing right brace at end of object definition - found \"%\""
        } else {
            "missing semicolon at end of object definition - found \"%\""
        };
        loop {
            match self.cur() {
                Tok::Sem => {
                    // Inside braces a semicolon is an empty property.
                    self.next();
                    if !braces {
                        return Ok(());
                    }
                }
                // These start another definition, so this one's end
                // must have been left out.
                Tok::Class
                | Tok::Extern
                | Tok::Modify
                | Tok::Dictionary
                | Tok::Property
                | Tok::Plus
                | Tok::Inc
                | Tok::Intrinsic
                | Tok::Object
                | Tok::Grammar
                | Tok::Enum => {
                    self.error_curtok(missing_end);
                    return Ok(());
                }
                Tok::Sym => {
                    // A name and then something a template value could
                    // start with: if the name is an object's, it is
                    // probably the start of the next definition.
                    let after = self.next();
                    self.tok.unget_token();
                    if matches!(
                        after,
                        Tok::Sym
                            | Tok::At
                            | Tok::Plus
                            | Tok::Minus
                            | Tok::Times
                            | Tok::Div
                            | Tok::Mod
                            | Tok::Arrow
                            | Tok::And
                            | Tok::Not
                            | Tok::BNot
                            | Tok::Comma
                    ) && matches!(
                        self.prog.globals.get(&self.cur_text()),
                        Some(Global::Obj(_))
                    ) {
                        self.error_curtok(missing_end);
                        return Ok(());
                    }
                    if self.parse_obj_prop(def)? || one_prop {
                        return Ok(());
                    }
                }
                Tok::PropertySet | Tok::Operator | Tok::Replace => {
                    let shown = self.tok.cur_token_shown();
                    self.unsupported(&format!("\"{shown}\" in an object definition"));
                    self.next();
                }
                Tok::RBrace if braces => {
                    self.next();
                    return Ok(());
                }
                _ => {
                    let found = Self::name(&self.cur_text());
                    self.error(format!(
                        "expected property name in object definition, but found \"{found}\""
                    ));
                    self.next();
                    if self.cur() == Tok::Eof {
                        return Err(Stop);
                    }
                }
            }
        }
    }

    /// Read one property of an object, from its name on. Returns
    /// whether the definition turned out to have ended without saying
    /// so, with the next definition starting here.
    fn parse_obj_prop(&mut self, def: &mut ObjDef) -> Parsed<bool> {
        let (file, line) = self.tok.last_pos();
        (self.cur_file, self.cur_line) = (file, line);
        let name = self.cur_text();
        let existing = self.prog.globals.get(&name).copied();
        let mut new_prop = None;
        let mut is_value = false;
        let mut no_value = false;
        match self.next() {
            Tok::LPar | Tok::LBrace => {}
            Tok::Sem | Tok::RBrace | Tok::RPar | Tok::RBrack | Tok::Comma => {
                let found = Self::name(&self.cur_text());
                self.error(format!(
                    "expected property value or method after property name \"{}\", \
                     but found \"{found}\"",
                    Self::name(&name)
                ));
                if self.cur() != Tok::Sem {
                    self.next();
                }
                no_value = true;
            }
            Tok::Colon => {
                if matches!(existing, Some(Global::Obj(_))) {
                    // `name:` where the name is an object's: the next
                    // definition, this one's end having been left out.
                    self.tok.log_at(
                        file,
                        line,
                        Severity::Error,
                        "unterminated object definition".to_owned(),
                    );
                    self.tok.unget_token();
                    return Ok(true);
                }
                // An object of its own as the property's value. It
                // has to be in braces; without them, this is taken to
                // be the next definition, this one's end having been
                // left out, and is read as that.
                self.next();
                let mut nested = ObjDef {
                    sym: None,
                    is_class: false,
                    superclasses: Vec::new(),
                    props: Vec::new(),
                    file,
                    line,
                };
                self.parse_superclass_list(None, &mut nested);
                if self.cur() == Tok::LBrace {
                    self.unsupported("an object nested in an object definition");
                    self.next();
                    self.parse_obj_prop_list(&mut nested, true, false)?;
                    return Ok(false);
                }
                self.tok.log_at(
                    file,
                    line,
                    Severity::Error,
                    "unterminated object definition".to_owned(),
                );
                self.parse_obj_prop_list(&mut nested, false, true)?;
                return Ok(true);
            }
            Tok::Eq => {
                self.next();
                if self.cur() == Tok::LBrace {
                    self.error("'=' is not allowed with a method definition");
                } else {
                    is_value = true;
                }
            }
            _ => {
                self.error_curtok("'=' required between property name and value - found \"%\"");
                is_value = true;
            }
        }

        let prop = if no_value {
            None
        } else {
            self.look_up_prop(&name)
        };
        if no_value {
            // Nothing to read; but the name is still checked below.
        } else if is_value {
            let is_static = self.cur() == Tok::Static;
            if is_static {
                self.next();
            }
            // The value is read as an expression of its own, outside
            // any function.
            let old_cur_code_body = self.cur_code_body;
            self.clear_local_ctx();
            self.cur_code_body = None;
            self.local_symtab = None;
            self.enclosing_local_symtab = None;
            self.local_cnt = 0;
            self.max_local_cnt = 0;
            self.enclosing.clear();
            // A value that cannot be read ends the reading of the whole
            // source.
            let Some(expr) = self.parse_expr() else {
                return Err(Stop);
            };
            // A value with an anonymous function in it that uses `self`
            // has to be a method, to make the context the function
            // finds `self` in.
            let has_local_ctx = self.has_local_ctx;
            let value = if has_local_ctx {
                let kind = if let (true, Some(prop)) = (is_static, prop) {
                    StmtKind::StaticPropInit(expr, prop)
                } else if self.prog.has_return_value(&expr) {
                    StmtKind::Return(Some(expr))
                } else {
                    StmtKind::Expr(expr)
                };
                // The reference takes the statement to be where the
                // last statement read was, wherever that is; but it
                // looks up the names in the value as the property's,
                // and that is where what there is to say gets said.
                let mut body = CodeBody {
                    lcltab: None,
                    stm: Stmt { kind, file, line },
                    argc: 0,
                    opt_argc: 0,
                    varargs: false,
                    varargs_list_local: None,
                    local_cnt: self.local_cnt,
                    self_valid: self.self_valid,
                    replaced: false,
                    labels: std::rc::Rc::default(),
                    end_file: file,
                    end_line: line,
                    closure: Closure {
                        // The reference has the last body read as the
                        // one this is in.
                        enclosing: old_cur_code_body,
                        wraps_prop_expr: true,
                        ..Closure::default()
                    },
                };
                self.finish_local_ctx(&mut body, false);
                body.closure.self_referenced = self.self_referenced;
                body.closure.full_ctx_referenced = self.full_ctx_referenced;
                body.closure.local_ctx_needs_self = self.local_ctx_needs_self;
                body.closure.local_ctx_needs_full = self.local_ctx_needs_full;
                self.prog.bodies.push((body, None));
                PropValue::Method(self.prog.bodies.len() - 1)
            } else {
                PropValue::Expr(expr)
            };
            self.cur_code_body = old_cur_code_body;
            if let Some(prop) = prop {
                new_prop = Some(def.props.len());
                // The reference forgets that the property is static when
                // its value has had to be made a method already.
                def.props.push(ObjProp {
                    prop,
                    value,
                    overwritable: false,
                    is_static: is_static && !has_local_ctx,
                    file,
                    line,
                });
            }
        } else {
            self.body_is_obj_prop = true;
            let Some((body, _)) = self.parse_code_body(true, CbType::Normal, None, None)? else {
                return Ok(false);
            };
            self.prog.last_labels = std::rc::Rc::clone(&body.labels);
            self.self_valid = true;
            if let Some(prop) = prop {
                new_prop = Some(def.props.len());
                // The method is taken to be where the last statement
                // read started: the last one in its body, if it has
                // any.
                def.props.push(ObjProp {
                    prop,
                    value: PropValue::Method(self.prog.bodies.len()),
                    overwritable: false,
                    is_static: false,
                    file: self.cur_file,
                    line: self.cur_line,
                });
            }
            self.prog.bodies.push((body, None));
        }

        // A property the name already was can be here only once, but
        // that the source's value replaces one the compiler put in.
        if let Some(Global::Prop(prop)) = existing {
            let earlier = def
                .props
                .iter()
                .enumerate()
                .position(|(index, other)| Some(index) != new_prop && other.prop == prop);
            if let Some(earlier) = earlier {
                if def.props[earlier].overwritable {
                    def.props.remove(earlier);
                } else {
                    self.error(format!(
                        "property \"{}\" already defined in object",
                        Self::name(&name)
                    ));
                }
            }
        }
        Ok(false)
    }
}

// ---- expressions embedded in strings ----
//
// A string with expressions in it comes from the tokenizer as its
// pieces: the text up to the first expression, the expression's tokens,
// the text up to the next, and so on. In double quotes,
// `"one <<two>> three"` is `"one ", say(two), " three"`; in single
// quotes, `'one <<two>> three'` is `'one ' + (two) + ' three'`.
impl Parser<'_, '_> {
    fn embed_mid(dbl: bool) -> Tok {
        if dbl { Tok::DStrMid } else { Tok::SStrMid }
    }

    fn embed_end(dbl: bool) -> Tok {
        if dbl { Tok::DStrEnd } else { Tok::SStrEnd }
    }

    /// The text that starts a string or a part of one, which is the
    /// current token. Empty text in double quotes displays nothing, so
    /// it is left out; in single quotes it still makes what follows a
    /// string.
    fn embed_lead(&self, dbl: bool) -> Option<Expr> {
        let text = self.cur_text();
        if !dbl {
            Some(Expr::constant(Value::SStr(text)))
        } else if text.is_empty() {
            None
        } else {
            Some(Expr::DStr(text))
        }
    }

    fn embed_finish(dbl: bool, cur: Option<Expr>) -> Expr {
        cur.unwrap_or_else(|| {
            if dbl {
                Expr::DStr(Vec::new())
            } else {
                Expr::constant(Value::SStr(Vec::new()))
            }
        })
    }

    /// Add an embedded expression to what there is of the string so
    /// far.
    fn embed_add(dbl: bool, cur: Option<Expr>, sub: Expr) -> Expr {
        let op = if dbl { BinOp::Comma } else { BinOp::Add };
        let sub = if dbl {
            Expr::DStrEmbed(Box::new(sub))
        } else {
            sub
        };
        match cur {
            Some(cur) => Expr::Binary(op, Box::new(cur), Box::new(sub)),
            None => sub,
        }
    }

    /// Add the text after an embedded expression, which is the current
    /// token.
    fn embed_add_segment(&self, dbl: bool, cur: Option<Expr>) -> Option<Expr> {
        let text = self.cur_text();
        if text.is_empty() {
            return cur;
        }
        let (op, segment) = if dbl {
            (BinOp::Comma, Expr::DStr(text))
        } else {
            (BinOp::Add, Expr::constant(Value::SStr(text)))
        };
        Some(match cur {
            Some(cur) => Expr::Binary(op, Box::new(cur), Box::new(segment)),
            // Text after an embedding with nothing in it, at the very
            // start of a string. The reference crashes.
            None => segment,
        })
    }

    /// Whether tokens are a pattern: the same text, token for token,
    /// but that `*` in the pattern is one or more tokens.
    fn toks_match(toks: &[SrcToken], pattern: &[&[u8]]) -> bool {
        let mut tokn = toks.len();
        let mut argn = pattern.len();
        if tokn < argn {
            return false;
        }
        let mut tok = 0;
        let mut arg = 0;
        while tokn != 0 && argn != 0 {
            if pattern[arg] == b"*" {
                while tokn > argn {
                    tok += 1;
                    tokn -= 1;
                }
            } else if toks[tok].text != pattern[arg] {
                return false;
            }
            tok += 1;
            arg += 1;
            tokn -= 1;
            argn -= 1;
        }
        tokn == 0 && argn == 0
    }

    /// Whether the tokens read ahead are a phrase, given as its words
    /// with spaces between.
    fn embed_match(&self, phrase: &str) -> bool {
        let pattern: Vec<&[u8]> = phrase.split(' ').map(str::as_bytes).collect();
        Self::toks_match(&self.embed_toks, &pattern)
    }

    /// Read ahead through an embedded expression, to the next part of
    /// the string or to what looks like the end of the statement.
    fn embed_capture(&mut self, dbl: bool) {
        self.embed_toks.clear();
        // Only braces are counted: a semicolon inside them is not the
        // end of the statement.
        let mut braces = 0i32;
        loop {
            match self.cur() {
                Tok::LBrace => braces += 1,
                Tok::RBrace => {
                    braces -= 1;
                    if braces < 0 {
                        return;
                    }
                }
                Tok::Sem if braces == 0 => return,
                Tok::Eof => return,
                typ if typ == Self::embed_mid(dbl) || typ == Self::embed_end(dbl) => return,
                _ => {}
            }
            self.embed_toks.push(self.tok.cur_token().clone());
            self.next();
        }
    }

    /// Put the tokens read ahead back to be read again, but for the
    /// first `skip` of them.
    fn embed_unget(&mut self, skip: usize) {
        for index in (skip..self.embed_toks.len()).rev() {
            let token = self.embed_toks[index].clone();
            self.tok.unget_to(token);
        }
    }

    /// Read a string with embedded expressions, from its first part on.
    fn parse_embedding(&mut self, dbl: bool) -> Option<Expr> {
        loop {
            let mut eos = false;
            let level = EmbedLevel {
                kind: EmbedKind::Outer,
                parent: None,
            };
            let tree = self.parse_embedding_list(dbl, &mut eos, &level)?;
            if eos {
                return Some(tree);
            }
            // At the end of the source this would go on for ever; what
            // stops the reference is its limit on errors.
            if self.tok.gave_up() {
                return None;
            }
            // Something that ends a construct no construct is open
            // for: `<<end>>`, `<<else>>`.
            let (_, open_kw) = self.embed_end_tok_ahead(dbl, &level);
            let found = String::from_utf8_lossy(&self.cur_text()).into_owned();
            self.error(format!(
                "<<{found}>> found in string without a matching {open_kw}"
            ));
        }
    }

    /// Read text and embedded expressions up to the end of the string,
    /// or up to something that ends a construct this is inside of.
    fn parse_embedding_list(
        &mut self,
        dbl: bool,
        eos: &mut bool,
        parent: &EmbedLevel<'_>,
    ) -> Option<Expr> {
        *eos = false;
        let mut curtyp = self.cur();
        let mut cur = self.embed_lead(dbl);
        self.next();
        if curtyp == Self::embed_end(dbl) {
            *eos = true;
            return Some(Self::embed_finish(dbl, cur));
        }

        loop {
            let nerr = self.tok.error_count();
            self.embed_capture(dbl);

            let sub;
            let mut check_end = true;
            if self.embed_match("if *") {
                self.embed_unget(1);
                sub = self.parse_embedded_if(dbl, false, eos, parent);
            } else if self.embed_match("unless *") {
                self.embed_unget(1);
                sub = self.parse_embedded_if(dbl, true, eos, parent);
            } else if self.embed_match("one of") {
                sub = self.parse_embedded_oneof(dbl, eos, parent);
            } else if self.embed_match("first time") {
                sub = self.parse_embedded_firsttime(dbl, eos, parent);
            } else if self.embed_end_tok(parent).0 {
                // The end of a construct this is inside of: whoever is
                // reading that takes it from here.
                self.embed_unget(0);
                return Some(Self::embed_finish(dbl, cur));
            } else if self.embed_toks.is_empty() {
                sub = None;
            } else {
                // An expression, perhaps after how to format it
                // (`<<%2d x>>`).
                let fmt =
                    (self.embed_toks[0].typ == Tok::FmtSpec).then(|| self.embed_toks.remove(0));
                let expr = self.parse_embedded_expr()?;
                check_end = false;
                sub = Some(match fmt {
                    Some(fmt) => {
                        let fmt = Arg {
                            expr: Expr::constant(Value::SStr(fmt.text)),
                            varargs: false,
                            name: None,
                        };
                        let arg = Arg {
                            expr,
                            varargs: false,
                            name: None,
                        };
                        Expr::Call(Box::new(Expr::Sym(b"sprintf".to_vec())), vec![fmt, arg])
                    }
                    None => expr,
                });
            }
            if let Some(sub) = sub {
                cur = Some(Self::embed_add(dbl, cur, sub));
            }

            // Then comes more of the string.
            if *eos {
                return Some(Self::embed_finish(dbl, cur));
            }
            curtyp = self.cur();
            if curtyp == Self::embed_mid(dbl) {
                cur = self.embed_add_segment(dbl, cur);
                self.next();
            } else if curtyp == Self::embed_end(dbl) {
                cur = self.embed_add_segment(dbl, cur);
                self.next();
                *eos = true;
                return Some(Self::embed_finish(dbl, cur));
            } else if check_end && self.embed_end_tok_ahead(dbl, parent).0 {
                // A construct that was inside this one stopped at the
                // end of an outer one.
                return Some(Self::embed_finish(dbl, cur));
            } else if self.tok.error_count() != nerr {
                // The expression went wrong, so it is no surprise that
                // the string does not go on here. Look for where it
                // does, or for the end of the statement.
                loop {
                    if matches!(curtyp, Tok::Sem | Tok::RBrace) {
                        return Some(Self::embed_finish(dbl, cur));
                    }
                    if curtyp == Tok::Eof {
                        return None;
                    }
                    if curtyp == Self::embed_mid(dbl) || curtyp == Self::embed_end(dbl) {
                        break;
                    }
                    curtyp = self.next();
                }
            } else {
                self.error_curtok("expected '>>' and the string continuation, but found \"%\"");
                self.embed_capture(dbl);
                if self.cur() != Self::embed_mid(dbl) && self.cur() != Self::embed_end(dbl) {
                    return Some(Self::embed_finish(dbl, cur));
                }
            }
        }
    }

    /// Read one embedded expression, from the tokens read ahead. If
    /// they are a string template, it is a call to the template's
    /// function.
    fn parse_embedded_expr(&mut self) -> Option<Expr> {
        for index in 0..self.str_templates.len() {
            let template = &self.str_templates[index];
            let pattern: Vec<&[u8]> = template.toks.iter().map(Vec::as_slice).collect();
            if !Self::toks_match(&self.embed_toks, &pattern) {
                continue;
            }
            let func = Expr::Sym(template.func.clone());
            let mut args = Vec::new();
            if template.star {
                let expr = if pattern.len() > 1 {
                    // Leave only the tokens that the `*` stands for,
                    // and see whether they are a template in turn.
                    let before = pattern
                        .iter()
                        .position(|tok| *tok == b"*")
                        .unwrap_or(pattern.len());
                    let after = pattern.len().saturating_sub(before + 1);
                    let end = self.embed_toks.len().saturating_sub(after);
                    self.embed_toks.truncate(end);
                    self.embed_toks.drain(..before.min(end));
                    self.parse_embedded_expr()?
                } else {
                    self.embed_unget(0);
                    self.parse_expr()?
                };
                args.push(Arg {
                    expr,
                    varargs: false,
                    name: None,
                });
            }
            return Some(Expr::Call(Box::new(func), args));
        }
        self.embed_unget(0);
        self.parse_expr()
    }

    /// Read `<<if cond>>...<<else if cond>>...<<else>>...<<end>>`, from
    /// the first condition on. `unless` is `if` the other way round.
    fn parse_embedded_if(
        &mut self,
        dbl: bool,
        unless: bool,
        eos: &mut bool,
        parent: &EmbedLevel<'_>,
    ) -> Option<Expr> {
        let level = EmbedLevel {
            kind: EmbedKind::If,
            parent: Some(parent),
        };
        let not = |cond: Expr, unless: bool| {
            if unless {
                Expr::Unary(UnOp::Not, Box::new(cond))
            } else {
                cond
            }
        };
        let cond = self.parse_cond_expr()?;
        let then = self.parse_embedding_list(dbl, eos, &level)?;
        let mut branches = vec![(not(cond, unless), then)];
        let mut last = None;
        while !*eos && (self.cur() == Tok::Else || self.cur_matches(b"otherwise")) {
            if self.next() == Tok::If || self.cur_matches(b"unless") {
                let unless = self.cur() != Tok::If;
                self.next();
                let cond = self.parse_cond_expr()?;
                let then = self.parse_embedding_list(dbl, eos, &level)?;
                branches.push((not(cond, unless), then));
            } else {
                last = Some(self.parse_embedding_list(dbl, eos, &level)?);
                break;
            }
        }
        // Without an `else`, there is nothing where it would be.
        let mut tree = last.unwrap_or_else(|| Self::embed_finish(dbl, None));
        for (cond, then) in branches.into_iter().rev() {
            tree = Expr::If(Box::new(cond), Box::new(then), Box::new(tree));
        }
        // `<<end>>` can be left out, at the end of the string or of
        // whatever this is inside of.
        if !*eos && self.cur_matches(b"end") {
            self.next();
        }
        Some(tree)
    }

    /// Read `<<one of>>...<<or>>...`, and how to choose among them.
    fn parse_embedded_oneof(
        &mut self,
        dbl: bool,
        eos: &mut bool,
        parent: &EmbedLevel<'_>,
    ) -> Option<Expr> {
        let level = EmbedLevel {
            kind: EmbedKind::OneOf,
            parent: Some(parent),
        };
        let mut elements = Vec::new();
        loop {
            elements.push(self.parse_embedding_list(dbl, eos, &level)?);
            self.embed_capture(dbl);
            if !self.embed_match("or") && !self.embed_match("||") {
                break;
            }
        }
        // If what follows is not one of the endings, it is for whatever
        // this is inside of.
        if !ONE_OF_ENDINGS.iter().any(|ending| self.embed_match(ending)) {
            self.embed_unget(0);
        }
        Some(self.oneof_node(dbl, elements))
    }

    /// Read `<<first time>>...<<only>>`: text for the first time only,
    /// which is a choice between it and nothing that stops at the
    /// second.
    fn parse_embedded_firsttime(
        &mut self,
        dbl: bool,
        eos: &mut bool,
        parent: &EmbedLevel<'_>,
    ) -> Option<Expr> {
        let level = EmbedLevel {
            kind: EmbedKind::FirstTime,
            parent: Some(parent),
        };
        let element = self.parse_embedding_list(dbl, eos, &level)?;
        self.embed_capture(dbl);
        if !self.embed_match("only") {
            self.embed_unget(0);
        }
        let nothing = Expr::constant(Value::SStr(Vec::new()));
        Some(self.oneof_node(dbl, vec![element, nothing]))
    }

    /// A choice among alternatives keeps its state in an object of the
    /// class `OneOfIndexGen`, and there are no objects yet.
    fn oneof_node(&mut self, dbl: bool, elements: Vec<Expr>) -> Expr {
        if !self.syntax_only {
            self.error("<<one of>> requires class OneOfIndexGen to be defined in the program");
        }
        Expr::OneOf(dbl, elements)
    }

    /// Whether what comes next ends a construct this is inside of. The
    /// tokens are looked at and put back.
    fn embed_end_tok_ahead(&mut self, dbl: bool, parent: &EmbedLevel<'_>) -> (bool, &'static str) {
        self.embed_capture(dbl);
        let result = self.embed_end_tok(parent);
        self.embed_unget(0);
        result
    }

    /// Whether the tokens read ahead end a construct this is inside of,
    /// and what kind of construct they would end.
    fn embed_end_tok(&self, parent: &EmbedLevel<'_>) -> (bool, &'static str) {
        let mut open_kw = "unknown";
        // `else` can be nothing else, wherever it is.
        if self.embed_match("else")
            || self.embed_match("else if *")
            || self.embed_match("else unless *")
        {
            return (true, "<<if>> or <<unless>>");
        }
        if self.embed_match("end")
            || self.embed_match("otherwise")
            || self.embed_match("otherwise if *")
            || self.embed_match("otherwise unless *")
        {
            open_kw = "<<if>> or <<unless>>";
            if parent.is_in(EmbedKind::If) {
                return (true, open_kw);
            }
        }
        if self.embed_match("or") || self.embed_match("||") {
            open_kw = "<<one of>>";
            if parent.is_in(EmbedKind::OneOf) {
                return (true, open_kw);
            }
        }
        if ONE_OF_ENDINGS.iter().any(|ending| self.embed_match(ending)) {
            open_kw = "<<one of>>";
            if parent.is_in(EmbedKind::OneOf) {
                return (true, open_kw);
            }
        }
        if self.embed_match("only") {
            open_kw = "<<first time>>";
            if parent.is_in(EmbedKind::FirstTime) {
                return (true, open_kw);
            }
        }
        (false, open_kw)
    }
}

/// What a parameter list says about a function.
#[derive(Default)]
struct Formals {
    /// The list is only being read for how many parameters there are;
    /// they are not declared.
    count_only: bool,
    /// The list is that of `{a, b: expression}`, which ends at a colon.
    short_form: bool,
    /// A parameter can be given a type.
    typed_ok: bool,
    argc: usize,
    opt_argc: usize,
    varargs: bool,
    varargs_list_local: Option<LocalId>,
    /// How many default values the list has had.
    defval_cnt: usize,
}
