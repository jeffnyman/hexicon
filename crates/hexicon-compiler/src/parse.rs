//! The parser: tokens in, a syntax tree and the program's symbols out.
//!
//! It follows the reference compiler's parser routine by routine,
//! including how it carries on after an error, so that the same source
//! gives the same tree. So far it reads functions: their parameters,
//! local variables, the statements that need no objects, and
//! expressions over numbers, strings, locals and function calls.

use std::collections::{HashMap, HashSet};

use crate::ast::{
    Arg, AsiOp, BinOp, Catch, Closure, CodeBody, ConstVal, Enclosure, Expr, Finally, ForIn,
    InlineProp, InlineValue, LabelSym, LocalId, Stmt, StmtKind, TabId, UnOp, Value, flow,
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
    /// The definitions `modify` has put this one in front of, the
    /// latest first: indexes into [`Program::bodies`].
    pub mod_bases: Vec<usize>,
    /// `modify` was used on it when it had no definition yet in what
    /// had been read, so what it modified is not in the source.
    pub mod_extern: bool,
    /// The parameters of the latest definition made with `modify`.
    pub mod_proto: (usize, usize, bool),
    /// `replace` or `modify` was used on it before its definition.
    pub ext_redef: bool,
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
    /// It is defined with `object` for its superclass.
    pub sc_is_root: bool,
    /// The templates defined for it (`Name template ...;`).
    pub templates: Vec<Vec<TemplateItem>>,
    /// Its definition as it now stands: an index into
    /// [`Program::objdefs`].
    pub def: Option<usize>,
    /// How many times `modify` has been applied to it, and the numbers
    /// set aside for the definitions that were modified, the first
    /// definition's first. Each `modify` leaves the definition before
    /// it as an object of its own, with a number just before this
    /// one's.
    pub mod_count: usize,
    pub base_ids: Vec<u32>,
    /// It was made by a `replace` or `modify` of a name that was
    /// nothing, on the first reading, which passes no such name on.
    pub undefined_redef: bool,
    /// It has only been declared, with `extern`, and has no number.
    pub declared_only: bool,
    /// `replace` or `modify` was used on it before its definition.
    pub ext_redef: bool,
    /// It is a dictionary (`dictionary name;`): an object of a class
    /// built into the virtual machine that holds the words the program
    /// gives its objects.
    pub is_dict: bool,
    /// On the second reading, for an object the first reading knew:
    /// the name of the dictionary its words went in then.
    pub dict_hint: Option<Vec<u8>>,
    /// It is a grammar production (`grammar name: ...`): an object of
    /// a class built into the virtual machine that holds the ways a
    /// phrase of its kind can be made up.
    pub is_gramprod: bool,
    /// For an object that a `grammar` statement with a tag defines, to
    /// be made when a phrase is matched: the production it is for, as
    /// an index into [`Program::gramprods`], and its ways of making
    /// the phrase up. They are the name's, and join the production's
    /// own when the program is put together.
    pub gram: Option<(Option<usize>, Vec<GramAlt>)>,
}

/// One way a phrase can be made up: a row of things to match.
#[derive(Debug, Clone)]
pub(crate) struct GramAlt {
    /// The object made when it matches: an index into
    /// [`Program::objs`].
    pub obj: Option<usize>,
    /// The dictionary that was current, which the words the row has
    /// go in.
    pub dict: Option<usize>,
    /// How much worse than another it is (`[badness 50]`).
    pub badness: i32,
    pub toks: Vec<GramTok>,
}

/// One thing to match in such a row, and the property the match is
/// kept in (`-> prop`).
#[derive(Debug, Clone)]
pub(crate) struct GramTok {
    pub kind: GramTokKind,
    pub prop: Option<usize>,
}

#[derive(Debug, Clone)]
pub(crate) enum GramTokKind {
    /// A word, as it is written.
    Literal(Vec<u8>),
    /// Any word the dictionary has for a property.
    Speech(usize),
    /// Any word the dictionary has for one of several properties.
    SpeechList(Vec<usize>),
    /// A phrase of another production: an index into
    /// [`Program::objs`].
    Prod(usize),
    /// A token of a kind: an index into [`Program::enums`].
    TokType(usize),
    /// Whatever is left of the input.
    Star,
}

/// A grammar production: the object that stands for it, whether a
/// statement declared it without saying how it is made up, and the
/// ways it can be, from the statements with no tag.
#[derive(Debug)]
pub(crate) struct GramProd {
    pub sym: usize,
    pub declared: bool,
    pub alts: Vec<GramAlt>,
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
    /// The built-in class it is declared to be a kind of: an index
    /// into [`Program::metas`].
    pub super_meta: Option<usize>,
    /// How many times `modify` has been applied to it, and the numbers
    /// set aside for the objects that hold what the program adds to
    /// the class: one with nothing in it, and then one for each
    /// `modify`, each the superclass of the next.
    pub mod_count: usize,
    pub mod_ids: Vec<u32>,
    /// The latest of those objects, which is the one the class is told
    /// about.
    pub mod_obj: Option<u32>,
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
    /// Its values are words for a dictionary
    /// (`dictionary property noun;`).
    pub vocab: bool,
}

/// Where the count of names first met on the second reading starts.
pub(crate) const SECOND_READING: u32 = 1 << 24;

/// The value an object definition gives a property.
#[derive(Debug)]
pub(crate) enum PropValue {
    Expr(Expr),
    /// The words the object has for a dictionary property, which are
    /// known when the program is put together.
    VocabList,
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
    /// The definition is a `modify` of an earlier one, which is its
    /// only superclass: that definition's number as an object, if it
    /// has one, and its index in [`Program::objdefs`].
    pub is_modify: bool,
    pub mod_base: Option<u32>,
    pub prev_def: Option<usize>,
    /// A later `modify` took the object's name and number for a new
    /// definition, and left this one as an object with this number.
    pub mod_id: Option<u32>,
    /// A later `replace` took its place, so nothing is generated for
    /// it.
    pub replaced: bool,
    /// It is not an object of the program's but what the program adds
    /// to a built-in class with `modify`: an index into
    /// [`Program::metas`].
    pub meta: Option<usize>,
    /// It is not saved or restored with the rest (`transient`).
    pub transient: bool,
    /// It starts with values that no template gives properties to;
    /// and that because one of its classes is only declared, so that
    /// its templates cannot be known.
    pub bad_template: bool,
    pub undesc_sc: bool,
    /// The dictionary its words go in: the one that was current when
    /// it was defined, as an index into [`Program::objs`].
    pub dict: Option<usize>,
    /// Its words, in the order the source has them, each with the
    /// property it is a word for.
    pub vocab: Vec<(Vec<u8>, usize)>,
}

/// One value in an object template: the kind of token the value starts
/// with, and the property it is for. `inherited` in a template, which
/// stands for the templates of the class's superclasses, has no
/// property.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TemplateItem {
    pub prop: Option<usize>,
    pub tok: Tok,
    /// The next item is an alternative to this one (`'name' | "desc"`).
    pub is_alt: bool,
    /// The value can be left out (`'name'?`).
    pub is_opt: bool,
}

/// A value at the start of an object definition, to be matched against
/// the templates.
struct TemplateInst {
    def_tok: Tok,
    value: Option<PropValue>,
    prop: Option<usize>,
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
/// Which of them is `sourceTextOrder`, and which `lexicalParent`.
const SRC_ORDER_PROP: usize = 6;
const LEXICAL_PARENT_PROP: usize = 5;
/// And `grammarInfo` and `grammarTag`.
const GRAMMAR_INFO_PROP: usize = 2;
const GRAMMAR_TAG_PROP: usize = 3;

/// How deep `propertyset` can be inside `propertyset`.
const MAX_PROPSET_DEPTH: usize = 10;

/// The longest a name can be.
const TOK_SYM_MAX_LEN: usize = 80;

/// A `propertyset` being read: the pattern its properties' names go
/// into in place of the `*`, and the tokens of its parameter list,
/// where `*` stands for each method's own parameters.
#[derive(Debug, Default)]
struct PropSet {
    pattern: Vec<u8>,
    params: Vec<SrcToken>,
}

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
    /// The templates any object can use (`object template ...;`).
    pub templates: Vec<Vec<TemplateItem>>,
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
    pub nested: Vec<Top>,
    /// What bodies know of the bodies around them.
    pub body_refs: Vec<BodyRef>,
    /// The labels of the last function or method read.
    pub last_labels: std::rc::Rc<Vec<LabelSym>>,
    /// The dictionaries, in the order they were first declared: indexes
    /// into [`Program::objs`].
    pub dicts: Vec<usize>,
    /// The dictionary properties, the last declared first.
    pub dict_props: Vec<usize>,
    /// The grammar productions, in the order they were first named.
    pub gramprods: Vec<GramProd>,
}

/// What is known of a body that others are nested in.
#[derive(Debug, Clone, Default)]
pub(crate) struct BodyRef {
    pub enclosing: Option<usize>,
    pub local_ctx_needs_full: bool,
    /// For a function's body: its index in [`Program::bodies`].
    pub func_body: Option<usize>,
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
            Expr::OneOf(dbl, ..) => !dbl,
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
            // The next reading takes what is known of each one's
            // parameters for its whole declaration, even if all there
            // was to go on was an `extern` that said nothing of them.
            .map(|func| FuncSym {
                is_extern: true,
                has_proto: true,
                body: None,
                mod_bases: Vec::new(),
                mod_extern: false,
                ext_redef: false,
                ..func.clone()
            })
            .collect();
        // Objects are numbered in the order of the table of global
        // names as it is when the program is put together; in a chain
        // of that table, they are in the order they were defined.
        let mut objs: Vec<ObjSym> = self
            .objs
            .iter()
            .filter(|obj| {
                !obj.is_extern && !obj.undefined_redef && self.globals.contains_key(&obj.name)
            })
            .map(|obj| ObjSym {
                is_extern: true,
                gram: None,
                dict_hint: obj
                    .def
                    .and_then(|def| self.objdefs[def].dict)
                    .map(|dict| self.objs[dict].name.clone()),
                def: None,
                base_ids: Vec::new(),
                ext_redef: false,
                templates: Vec::new(),
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
                    super_meta: None,
                    mod_ids: Vec::new(),
                    mod_obj: None,
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
            vocab: false,
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
    /// The property being read has `replace` before it.
    prop_replace: bool,
    /// The object about to be read has `transient` before it.
    transient: bool,
    /// The dictionary the words of the objects being read go in.
    cur_dict: Option<usize>,
    /// While the properties of an object written in an expression are
    /// being read: what each is given, in step with the properties.
    inline_values: Option<Vec<InlineValue>>,
    /// The dictionary properties the object being read has given
    /// words for. An object written inside it starts the count again,
    /// and the object outside does not get its own back.
    dict_defined: HashSet<usize>,
    /// The property `+` before an object gives a value to
    /// (`+ property location;`).
    plus_prop: Option<usize>,
    /// For each number of `+` signs, the last definition that had so
    /// many: an index into [`Program::objdefs`].
    plus_stack: Vec<Option<usize>>,
    /// How many `+` signs the object about to be read has before it.
    plus_cnt: usize,
    /// The object about to be read is the value of a property of
    /// another; and where its definition was put, once it is read.
    obj_nested: bool,
    nested_def: Option<usize>,
    /// The definition being read was found to have ended without
    /// saying so; and where its property being read started, which is
    /// where that is reported.
    unterm: bool,
    term_pos: (usize, u32),
    /// The `propertyset` groups being read, the outermost first, and
    /// how many are open, which can be more than are kept.
    propsets: Vec<PropSet>,
    propset_depth: usize,
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

/// What each of them tells the object that does the choosing, in the
/// same order.
const ONE_OF_ATTRS: [&str; 12] = [
    "rand",
    "seq,rand",
    "rand-norpt",
    "seq,rand-norpt",
    "rand,stop",
    "rand-wt",
    "shuffle",
    "seq,shuffle",
    "shuffle2",
    "seq,shuffle2",
    "seq",
    "seq,stop",
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
        let mut dict_numbered = HashSet::new();
        for (_, _, is_meta, index) in numbered {
            if is_meta {
                // The objects that hold what `modify` adds to the class
                // come just after the object that stands for it.
                let meta = &mut metas[index].0;
                meta.obj_id = prog.next_obj_id;
                if meta.mod_count != 0 {
                    for _ in 0..=meta.mod_count {
                        prog.next_obj_id += 1;
                        meta.mod_ids.push(prog.next_obj_id);
                    }
                }
                meta.mod_count = 0;
            } else {
                // A dictionary is numbered just before the first object
                // whose words go in it, if that comes before its own
                // turn: the reference writes it out ahead of the
                // object, and numbers things as it reads them back.
                if objs[index].is_dict {
                    if !dict_numbered.insert(index) {
                        continue;
                    }
                } else if let Some(hint) = objs[index].dict_hint.take() {
                    let dict = objs.iter().position(|obj| obj.is_dict && obj.name == hint);
                    if let Some(dict) = dict {
                        if dict_numbered.insert(dict) {
                            objs[dict].id = prog.next_obj_id;
                            prog.next_obj_id += 1;
                        }
                    }
                }
                // The definitions a `modify` set aside come just
                // before the object, the oldest first.
                let obj = &mut objs[index];
                for _ in 0..obj.mod_count {
                    obj.base_ids.push(prog.next_obj_id);
                    prog.next_obj_id += 1;
                }
                obj.mod_count = 0;
                obj.id = prog.next_obj_id;
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
            prop_replace: false,
            transient: false,
            cur_dict: None,
            inline_values: None,
            dict_defined: HashSet::new(),
            plus_prop: None,
            plus_stack: Vec::new(),
            plus_cnt: 0,
            obj_nested: false,
            nested_def: None,
            unterm: false,
            term_pos: (0, 0),
            propsets: Vec::new(),
            propset_depth: 0,
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

    /// A count as a message shows a place in a row: 1st, 2nd, 3rd, 4th,
    /// 11th, 21st.
    fn ordinal(count: usize) -> String {
        let suffix = match (count % 100, count % 10) {
            (10..=20, _) => "th",
            (_, 1) => "st",
            (_, 2) => "nd",
            (_, 3) => "rd",
            _ => "th",
        };
        format!("{count}{suffix}")
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
                Tok::Function | Tok::Method => self.parse_function(true, Redef::No),
                Tok::Sym => {
                    self.parse_object_or_func(Redef::No, suppress_error, &mut suppress_next_error)
                }
                Tok::Replace => self.parse_replace(),
                Tok::Modify => self.parse_modify(),
                Tok::Sem => {
                    self.next();
                    Ok(())
                }
                Tok::Intrinsic => self.parse_intrinsic(),
                Tok::Class => {
                    self.next();
                    self.parse_object(true, Redef::No)
                }
                Tok::Object => {
                    if self.next() == Tok::Template {
                        self.parse_template_def(None)
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
                Tok::Extern => self.parse_extern(),
                Tok::Transient => self.parse_transient(suppress_error, &mut suppress_next_error),
                Tok::Plus | Tok::Inc => self.parse_plus_object(),
                Tok::Dictionary => self.parse_dict(),
                Tok::Grammar => self.parse_grammar(Redef::No),
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
        // The anonymous functions come after everything else, and
        // with them the empty objects that the additions to built-in
        // classes start from.
        for nested in std::mem::take(&mut self.prog.nested) {
            self.prog.top.push(nested);
        }
        self.prog
    }

    /// Read an object template, from `template` on: `object template
    /// 'name' "desc"? @location;` says that an object definition that
    /// starts with values of those kinds means those properties by
    /// them. With a class's name for `object`, the template is for
    /// objects of that class.
    fn parse_template_def(&mut self, class: Option<Vec<u8>>) -> Parsed<()> {
        let mut class_sym = None;
        if let Some(name) = class {
            match self.prog.globals.get(&name).copied() {
                Some(Global::Obj(obj)) => class_sym = Some(obj),
                Some(_) => {
                    self.error_curtok("symbol \"%\" is already defined - can't redefine as object");
                }
                None => {
                    // A class there has been no word of yet.
                    self.prog.objs.push(ObjSym {
                        name: name.clone(),
                        id: 0,
                        is_extern: true,
                        is_class: false,
                        sc_names: Vec::new(),
                        sc_is_root: false,
                        templates: Vec::new(),
                        def: None,
                        mod_count: 0,
                        base_ids: Vec::new(),
                        undefined_redef: false,
                        declared_only: true,
                        ext_redef: false,
                        is_dict: false,
                        dict_hint: None,
                        is_gramprod: false,
                        gram: None,
                    });
                    class_sym = Some(self.prog.objs.len() - 1);
                    self.prog
                        .globals
                        .insert(name, Global::Obj(self.prog.objs.len() - 1));
                }
            }
        }
        self.next();

        let mut items: Vec<TemplateItem> = Vec::new();
        let mut alt_group_head: Option<usize> = None;
        let mut found_inh = false;
        let mut all_ok = true;
        let mut done = false;
        while !done {
            let mut ok = true;
            let def_tok = self.cur();
            let mut is_inh = false;
            let mut prop_name = Vec::new();
            match def_tok {
                Tok::Sem => {
                    self.next();
                    done = true;
                }
                Tok::Inherited => {
                    is_inh = true;
                    found_inh = true;
                }
                // A string stands for a value in quotes of its kind,
                // and what is in the string is the property's name.
                Tok::SStr | Tok::DStr => {
                    prop_name = self.cur_text();
                    let valid = prop_name
                        .first()
                        .is_some_and(|&c| crate::lex::is_syminit(c))
                        && prop_name.iter().all(|&c| crate::lex::is_sym(c))
                        && crate::lex::keyword(&prop_name).is_none();
                    if !valid {
                        let found = self.tok.cur_token_shown();
                        self.error(format!(
                            "property name required in object template string - found {found}"
                        ));
                        ok = false;
                    }
                }
                Tok::LBrack => {
                    if self.next() != Tok::Sym {
                        let found = Self::name(&self.cur_text());
                        self.error(format!(
                            "property name required after \"[\" in object template - found \"{found}\""
                        ));
                        ok = false;
                        if !matches!(
                            self.cur(),
                            Tok::Sem | Tok::Eof | Tok::LBrace | Tok::RBrace | Tok::RBrack
                        ) {
                            self.next();
                        }
                    } else {
                        prop_name = self.cur_text();
                        if self.next() != Tok::RBrack {
                            self.error_curtok(
                                "expected ']' after object template property name - found \"%\"",
                            );
                        }
                    }
                }
                Tok::At
                | Tok::Plus
                | Tok::Minus
                | Tok::Times
                | Tok::Div
                | Tok::Mod
                | Tok::Arrow
                | Tok::And
                | Tok::Not
                | Tok::BNot
                | Tok::Comma => {
                    if self.next() != Tok::Sym {
                        let found = Self::name(&self.cur_text());
                        self.error(format!(
                            "property name required after \"{}\" in object template - found \"{found}\"",
                            Self::template_op_text(def_tok)
                        ));
                        ok = false;
                    }
                    prop_name = self.cur_text();
                }
                Tok::LBrace | Tok::RBrace | Tok::Eof | Tok::Object => {
                    self.error_curtok("unexpected token \"%\" in object template");
                    done = true;
                    ok = false;
                }
                _ => {
                    self.error_curtok("unexpected token \"%\" in object template");
                    self.next();
                    ok = false;
                }
            }

            // After the item: `?` if it can be left out, `|` if the
            // next is an alternative to it.
            let mut is_opt = false;
            let mut is_alt = false;
            if !done {
                self.next();
                if self.cur() == Tok::Question {
                    is_opt = true;
                    self.next();
                }
                if self.cur() == Tok::Or {
                    is_alt = true;
                    self.next();
                }
            }
            // Alternatives are optional together or not at all.
            if items.last().is_some_and(|tail| tail.is_alt) {
                if is_opt {
                    if let Some(head) = alt_group_head {
                        for item in &mut items[head..] {
                            item.is_opt = true;
                        }
                    }
                }
                if items.last().is_some_and(|tail| tail.is_opt) {
                    is_opt = true;
                }
            }
            if !ok {
                all_ok = false;
            }
            if done || !ok {
                continue;
            }
            let item = if is_inh {
                Some(TemplateItem {
                    prop: None,
                    tok: Tok::Inherited,
                    is_alt: false,
                    is_opt: false,
                })
            } else {
                // The name becomes a property's if it is nothing yet.
                let prop = match self.prog.globals.get(&prop_name) {
                    Some(Global::Prop(prop)) => Some(*prop),
                    Some(_) => None,
                    None => Some(self.prog.def_prop(&prop_name)),
                };
                match prop {
                    None => {
                        self.error(format!(
                            "symbol \"{}\" in object template is not a property",
                            Self::name(&prop_name)
                        ));
                        None
                    }
                    Some(prop) if self.prog.props[prop].vocab => {
                        self.error(format!(
                            "property \"{}\" is a dictionary property - not valid in templates",
                            Self::name(&prop_name)
                        ));
                        None
                    }
                    Some(prop) => {
                        self.prog.props[prop].weak = false;
                        // A property can only come twice as one of the
                        // alternatives this one is among.
                        for (index, earlier) in items.iter().enumerate() {
                            if earlier.prop == Some(prop) {
                                let ends = items[index..].iter().position(|item| !item.is_alt);
                                if ends.is_some() {
                                    self.error(format!(
                                        "property \"{}\" duplicated in object template",
                                        Self::name(&prop_name)
                                    ));
                                    break;
                                }
                            }
                        }
                        Some(TemplateItem {
                            prop: Some(prop),
                            tok: def_tok,
                            is_alt,
                            is_opt,
                        })
                    }
                }
            };
            if let Some(item) = item {
                if is_alt && !items.last().is_some_and(|tail| tail.is_alt) {
                    alt_group_head = Some(items.len());
                } else if !is_alt {
                    alt_group_head = None;
                }
                items.push(item);
            }
        }
        if items.is_empty() {
            self.error("empty template definition");
            all_ok = false;
        }
        if !all_ok {
            return Ok(());
        }
        if !found_inh {
            self.add_template_def(class_sym, items);
            return Ok(());
        }
        // `inherited` stands for each template of the class's
        // superclasses in turn, then each template any object can use,
        // and then nothing: the template is defined once for each.
        let mut inherited = Vec::new();
        if let Some(class) = class_sym {
            let mut seen = Vec::new();
            self.build_super_template_list(class, &mut seen, 0);
            for (obj, index) in seen {
                inherited.push(Some(self.prog.objs[obj].templates[index].clone()));
            }
        }
        inherited.extend(self.prog.templates.iter().cloned().map(Some));
        inherited.push(None);
        for sc_tpl in inherited {
            let mut expanded = Vec::new();
            for item in &items {
                if item.tok != Tok::Inherited {
                    expanded.push(item.clone());
                } else if let Some(sc_tpl) = &sc_tpl {
                    expanded.extend(sc_tpl.iter().cloned());
                }
            }
            self.add_template_def(class_sym, expanded);
        }
        Ok(())
    }

    fn add_template_def(&mut self, class: Option<usize>, items: Vec<TemplateItem>) {
        match class {
            Some(class) => self.prog.objs[class].templates.push(items),
            None => self.prog.templates.push(items),
        }
    }

    /// The templates of a class's superclasses, and theirs, each once,
    /// as an object and the template's place among that object's.
    fn build_super_template_list(
        &self,
        class: usize,
        list: &mut Vec<(usize, usize)>,
        depth: usize,
    ) {
        if self.prog.objs[class].sc_is_root || depth > 256 {
            return;
        }
        for name in &self.prog.objs[class].sc_names {
            let Some(Global::Obj(sc)) = self.prog.globals.get(name).copied() else {
                continue;
            };
            for index in 0..self.prog.objs[sc].templates.len() {
                if !list.contains(&(sc, index)) {
                    list.push((sc, index));
                }
            }
            self.build_super_template_list(sc, list, depth + 1);
        }
    }

    /// How an operator that starts a template value is written.
    fn template_op_text(tok: Tok) -> &'static str {
        match tok {
            Tok::At => "@",
            Tok::Plus => "+",
            Tok::Minus => "-",
            Tok::Times => "*",
            Tok::Div => "/",
            Tok::Mod => "%",
            Tok::Arrow => "->",
            Tok::And => "&",
            Tok::Not => "!",
            Tok::BNot => "~",
            Tok::Comma => ",",
            _ => "[",
        }
    }

    /// Read the values an object definition starts with, if it starts
    /// with any, and give them to the properties a template says they
    /// are for: a template of one of the object's classes, or failing
    /// that one any object can use.
    fn parse_obj_template(&mut self, def: &mut ObjDef) -> Parsed<()> {
        const STARTS: [Tok; 16] = [
            Tok::SStr,
            Tok::SStrStart,
            Tok::DStr,
            Tok::DStrStart,
            Tok::LBrack,
            Tok::At,
            Tok::Plus,
            Tok::Minus,
            Tok::Times,
            Tok::Div,
            Tok::Mod,
            Tok::Arrow,
            Tok::And,
            Tok::Not,
            Tok::BNot,
            Tok::Comma,
        ];
        if !STARTS.contains(&self.cur()) {
            return Ok(());
        }
        let mut insts: Vec<TemplateInst> = Vec::new();
        loop {
            let (file, line) = self.tok.last_pos();
            (self.cur_file, self.cur_line) = (file, line);
            let mut def_tok = self.cur();
            let saved = self.begin_prop_value();
            let expr = match def_tok {
                Tok::SStr => self.parse_primary(),
                Tok::SStrStart => {
                    def_tok = Tok::SStr;
                    self.parse_primary()
                }
                Tok::DStr => self.parse_expr(),
                Tok::DStrStart => {
                    def_tok = Tok::DStr;
                    self.parse_expr()
                }
                Tok::LBrack => self.parse_list(),
                Tok::Eof => return Ok(()),
                _ if STARTS.contains(&def_tok) => {
                    self.next();
                    self.parse_primary()
                }
                _ => {
                    self.cur_code_body = saved;
                    break;
                }
            };
            let value = match expr {
                Some(expr) => Some(self.finish_prop_value(expr, None, file, line, saved).0),
                None => {
                    self.cur_code_body = saved;
                    None
                }
            };
            insts.push(TemplateInst {
                def_tok,
                value,
                prop: None,
            });
        }

        // The classes' templates come first; a class that is only
        // declared has templates nobody here knows, so none will do.
        let mut undesc = false;
        let mut found = self
            .find_class_template(&def.superclasses, &mut insts, &mut undesc, 0)
            .map(|(tpl, _)| tpl);
        def.undesc_sc = undesc;
        if found.is_none() && !undesc {
            found = self
                .prog
                .templates
                .iter()
                .find(|tpl| Self::match_template(tpl, 0, &mut insts, 0))
                .cloned();
        }
        let Some(tpl) = found else {
            def.bad_template = true;
            return Ok(());
        };
        Self::match_template(&tpl, 0, &mut insts, 0);
        for inst in insts {
            if let (Some(value), Some(prop)) = (inst.value, inst.prop) {
                def.props.push(ObjProp {
                    prop,
                    value,
                    overwritable: false,
                    is_static: false,
                    file: self.cur_file,
                    line: self.cur_line,
                });
            }
        }
        Ok(())
    }

    /// Find the template for some values among those of the classes
    /// named and of their superclasses: the template, and the class it
    /// was found at. Of two classes with one each, the template of the
    /// later class is taken only if that class is a kind of the
    /// earlier.
    fn find_class_template(
        &self,
        classes: &[Vec<u8>],
        insts: &mut [TemplateInst],
        undesc: &mut bool,
        depth: usize,
    ) -> Option<(Vec<TemplateItem>, Vec<u8>)> {
        let mut best: Option<(Vec<TemplateItem>, Vec<u8>)> = None;
        if depth > 256 {
            return None;
        }
        for name in classes {
            let Some(Global::Obj(sc)) = self.prog.globals.get(name).copied() else {
                return None;
            };
            let sym = &self.prog.objs[sc];
            let own = sym
                .templates
                .iter()
                .find(|tpl| Self::match_template(tpl, 0, insts, 0));
            let cur = match own {
                Some(tpl) => Some((tpl.clone(), name.clone())),
                None => {
                    if sym.sc_names.is_empty() && !sym.sc_is_root {
                        *undesc = true;
                        return None;
                    }
                    let cur = self.find_class_template(&sym.sc_names, insts, undesc, depth + 1);
                    if *undesc {
                        return None;
                    }
                    cur
                }
            };
            if let Some((tpl, def_sc)) = cur {
                let better = match &best {
                    None => true,
                    Some((_, best_sc)) => self.is_subclass_of(&def_sc, best_sc, 0),
                };
                if better {
                    best = Some((tpl, def_sc));
                }
            }
        }
        best
    }

    /// Whether a class is another, or a kind of it, going by names.
    fn is_subclass_of(&self, class: &[u8], other: &[u8], depth: usize) -> bool {
        if class == other {
            return true;
        }
        let Some(Global::Obj(sym)) = self.prog.globals.get(class).copied() else {
            return false;
        };
        depth <= 256
            && self.prog.objs[sym]
                .sc_names
                .iter()
                .any(|sc| self.is_subclass_of(sc, other, depth + 1))
    }

    /// Whether some values fit a template, from one of its items and
    /// one of the values on. Each value that fits an item is noted as
    /// being for the item's property; of alternatives that fit, the
    /// last. The notes of a try that fails are left as they fell, as
    /// in the reference, which is why a template that fits is gone
    /// through once more at the end.
    fn match_template(
        items: &[TemplateItem],
        mut item: usize,
        insts: &mut [TemplateInst],
        mut inst: usize,
    ) -> bool {
        while item < items.len() && inst < insts.len() {
            let is_opt = items[item].is_opt;
            let mut matched = false;
            loop {
                if items[item].tok == insts[inst].def_tok {
                    matched = true;
                    insts[inst].prop = items[item].prop;
                }
                if !items[item].is_alt || item + 1 == items.len() {
                    break;
                }
                item += 1;
            }
            if is_opt {
                if matched && Self::match_template(items, item + 1, insts, inst + 1) {
                    return true;
                }
            } else {
                if !matched {
                    return false;
                }
                inst += 1;
            }
            item += 1;
        }
        while item < items.len() && items[item].is_opt {
            item += 1;
        }
        item == items.len() && inst == insts.len()
    }

    /// Get ready to read a property's value, which is read as an
    /// expression of its own, outside any function. What comes back is
    /// to be handed to [`Parser::finish_prop_value`].
    fn begin_prop_value(&mut self) -> Option<usize> {
        let old_cur_code_body = self.cur_code_body;
        self.clear_local_ctx();
        self.cur_code_body = None;
        self.local_symtab = None;
        self.enclosing_local_symtab = None;
        self.local_cnt = 0;
        self.max_local_cnt = 0;
        self.enclosing.clear();
        old_cur_code_body
    }

    /// Make a property's value of the expression read for it, and say
    /// whether it had to be made a method already: a value with an
    /// anonymous function in it that uses `self` has to be one, to
    /// make the context the function finds `self` in. `static_prop` is
    /// the property, if the value is to be worked out once before the
    /// program starts.
    fn finish_prop_value(
        &mut self,
        expr: Expr,
        static_prop: Option<usize>,
        file: usize,
        line: u32,
        old_cur_code_body: Option<usize>,
    ) -> (PropValue, bool) {
        let has_local_ctx = self.has_local_ctx;
        let value = if has_local_ctx {
            let kind = if let Some(prop) = static_prop {
                StmtKind::StaticPropInit(expr, prop)
            } else if self.prog.has_return_value(&expr) {
                StmtKind::Return(Some(expr))
            } else {
                StmtKind::Expr(expr)
            };
            // The reference takes the statement to be where the last
            // statement read was, wherever that is; but it looks up the
            // names in the value as the property's, and that is where
            // what there is to say gets said.
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
                    // The reference has the last body read as the one
                    // this is in.
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
        (value, has_local_ctx)
    }

    /// Read `extern function name(params);`, `extern object name;` or
    /// `extern class name;`: the name is that of something defined
    /// somewhere else.
    fn parse_extern(&mut self) -> Parsed<()> {
        let is_class = match self.next() {
            Tok::Function | Tok::Method => return self.parse_extern_function(),
            Tok::Class => true,
            Tok::Object => false,
            _ => {
                self.error_curtok("invalid \"extern\" type specifier \"%\"");
                return self.skip_to_sem();
            }
        };
        if self.next() != Tok::Sym {
            self.error_curtok(
                "expected object name symbol in \"extern\" statement but found \"%\"",
            );
            return self.skip_to_sem();
        }
        let name = self.cur_text();
        match self.prog.globals.get(&name).copied() {
            None => {
                // It has no number until it is defined. One that never
                // is can be named but not used.
                self.prog.objs.push(ObjSym {
                    name: name.clone(),
                    id: 0,
                    is_extern: true,
                    is_class: false,
                    sc_names: Vec::new(),
                    sc_is_root: false,
                    templates: Vec::new(),
                    def: None,
                    mod_count: 0,
                    base_ids: Vec::new(),
                    undefined_redef: false,
                    declared_only: true,
                    ext_redef: false,
                    is_dict: false,
                    dict_hint: None,
                    is_gramprod: false,
                    gram: None,
                });
                self.prog
                    .globals
                    .insert(name, Global::Obj(self.prog.objs.len() - 1));
            }
            Some(Global::Obj(obj)) if self.prog.objs[obj].is_class == is_class => {}
            Some(_) => {
                self.error_curtok("object \"%\" is already defined - can't redefine as object");
            }
        }
        self.next();
        self.parse_req_sem()
    }

    /// Read the rest of `extern function name(params);`, from the
    /// keyword on.
    fn parse_extern_function(&mut self) -> Parsed<()> {
        const NO_BODY: &str = "code body is not allowed in an \"extern function\" declaration";
        if self.next() != Tok::Sym {
            self.error_curtok("expected symbol after 'function', but found \"%\"");
            return Ok(());
        }
        let name = self.cur_text();
        let (sym_file, sym_line) = self.tok.last_pos();
        let mut formals = Formals {
            count_only: true,
            ..Formals::default()
        };
        let mut has_proto = true;
        match self.next() {
            Tok::Sem => {
                // Nothing is said of its parameters.
                self.next();
                has_proto = false;
            }
            Tok::LPar => {
                self.next();
                self.parse_formal_list(&mut formals)?;
                if matches!(self.cur(), Tok::LBrace | Tok::Eq) {
                    self.error(NO_BODY);
                    self.parse_code_body(false, CbType::Normal, None, None)?;
                    return Ok(());
                }
                self.parse_req_sem()?;
            }
            Tok::LBrace => {
                self.error(NO_BODY);
                self.parse_code_body(false, CbType::Normal, None, None)?;
                return Ok(());
            }
            _ => {
                self.error_curtok("expected ';', '(', or '{', but found \"%\"");
                return Ok(());
            }
        }
        if let Some(Global::Prop(prop)) = self.prog.globals.get(&name) {
            if self.prog.props[*prop].weak {
                self.prog.globals.remove(&name);
            }
        }
        match self.prog.globals.get(&name).copied() {
            Some(Global::Func(id)) => {
                let func = &self.prog.funcs[id];
                if has_proto
                    && func.has_proto
                    && (func.argc != formals.argc || func.varargs != formals.varargs)
                {
                    self.tok.log_at(
                        sym_file,
                        sym_line,
                        Severity::Error,
                        format!(
                            "function \"{}\" has an incompatible previous declaration",
                            Self::name(&name)
                        ),
                    );
                }
            }
            Some(_) => {
                self.tok.log_at(
                    sym_file,
                    sym_line,
                    Severity::Error,
                    format!(
                        "symbol \"{}\" is already defined - can't redefine as function",
                        Self::name(&name)
                    ),
                );
            }
            None => {
                self.prog.funcs.push(FuncSym {
                    name: name.clone(),
                    argc: formals.argc,
                    opt_argc: formals.opt_argc,
                    varargs: formals.varargs,
                    has_retval: true,
                    is_extern: true,
                    has_proto,
                    body: None,
                    mod_bases: Vec::new(),
                    mod_extern: false,
                    mod_proto: (0, 0, false),
                    ext_redef: false,
                });
                self.prog
                    .globals
                    .insert(name, Global::Func(self.prog.funcs.len() - 1));
            }
        }
        Ok(())
    }

    /// Read `transient` and the object definition after it. Such an
    /// object is not saved or restored with the rest.
    fn parse_transient(
        &mut self,
        suppress_error: bool,
        suppress_next_error: &mut bool,
    ) -> Parsed<()> {
        self.transient = true;
        let result = match self.next() {
            Tok::Object => {
                if self.next() == Tok::Template {
                    self.transient = false;
                    self.parse_template_def(None)
                } else {
                    self.tok.unget_token();
                    self.parse_anon_object()
                }
            }
            Tok::Sym => self.parse_object_or_func(Redef::No, suppress_error, suppress_next_error),
            _ => {
                // Whatever this is, it is passed over, and what comes
                // after it is what gets the blame.
                self.next();
                if !suppress_error {
                    self.error_curtok("expected function or object definition, but found \"%\"");
                }
                *suppress_next_error = true;
                Ok(())
            }
        };
        self.transient = false;
        result
    }

    /// Read `replace` and the definition after it, which takes the
    /// place of an earlier definition of the same function or object.
    fn parse_replace(&mut self) -> Parsed<()> {
        match self.next() {
            Tok::Function | Tok::Method => self.parse_function(true, Redef::Replace),
            Tok::Class => {
                self.next();
                self.parse_object(true, Redef::Replace)
            }
            Tok::Sym => self.parse_object_or_func(Redef::Replace, false, &mut false),
            Tok::Transient => {
                self.transient = true;
                let result = match self.next() {
                    Tok::Object => {
                        if self.next() == Tok::Template {
                            self.transient = false;
                            self.parse_template_def(None)
                        } else {
                            self.tok.unget_token();
                            self.parse_anon_object()
                        }
                    }
                    Tok::Sym => self.parse_object_or_func(Redef::Replace, false, &mut false),
                    _ => {
                        self.next();
                        self.error_curtok(
                            "expected function or object definition, but found \"%\"",
                        );
                        Ok(())
                    }
                };
                self.transient = false;
                result
            }
            Tok::Grammar => self.parse_grammar(Redef::Replace),
            _ => {
                self.error_curtok(
                    "expected \"function\" or object name after \"replace\", but found \"%\"",
                );
                self.next();
                Ok(())
            }
        }
    }

    /// Read `modify` and the definition after it, which is added in
    /// front of an earlier definition of the same function or object.
    fn parse_modify(&mut self) -> Parsed<()> {
        match self.next() {
            Tok::Grammar => return self.parse_grammar(Redef::Modify),
            Tok::Function | Tok::Method => return self.parse_function(true, Redef::Modify),
            Tok::Sym => {
                let is_lpar = self.next() == Tok::LPar;
                self.tok.unget_token();
                if is_lpar {
                    return self.parse_function(false, Redef::Modify);
                }
            }
            _ => {}
        }
        // Anything else is taken for an object's name, whatever it is.
        self.parse_object(false, Redef::Modify)
    }

    fn parse_object_or_func(
        &mut self,
        redef: Redef,
        suppress_error: bool,
        suppress_next_error: &mut bool,
    ) -> Parsed<()> {
        let first = self.cur_text();
        match self.next() {
            Tok::LPar => {
                if std::mem::take(&mut self.transient) {
                    self.error("'transient' is not allowed here");
                }
                self.tok.unget_token();
                self.parse_function(false, redef)
            }
            Tok::Template => {
                if std::mem::take(&mut self.transient) {
                    self.error("'transient' is not allowed here");
                }
                if first == b"string" {
                    self.parse_string_template()
                } else {
                    self.parse_template_def(Some(first))
                }
            }
            Tok::Colon => {
                self.tok.unget_token();
                self.parse_object(false, redef)
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
                if redef == Redef::Replace {
                    self.error("base class list required for object 'replace' definition");
                }
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

    fn parse_function(&mut self, func_kw_present: bool, redef: Redef) -> Parsed<()> {
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
        let body_ref = self.prog.body_refs.len();
        let Some((mut body, has_retval)) =
            self.parse_code_body(false, CbType::Normal, None, None)?
        else {
            return Ok(());
        };
        body.closure.is_func = true;
        body.closure.func_proto = (body.argc, body.opt_argc, body.varargs);
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
                let mut clash = !func.is_extern && redef == Redef::No;
                if func.has_proto {
                    clash |= func.argc != argc || func.varargs != varargs;
                }
                if clash {
                    let message = if func.is_extern || redef != Redef::No {
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
                let was_extern = func.is_extern;
                let old_body = func.body;
                match redef {
                    // Nothing is generated for the definition replaced,
                    // nor for any that `modify` had put it in front of.
                    Redef::Replace if !was_extern => {
                        let func = &mut self.prog.funcs[id];
                        let gone = std::mem::take(&mut func.mod_bases);
                        if std::mem::take(&mut func.mod_extern) {
                            func.ext_redef = true;
                        }
                        for old in old_body.into_iter().chain(gone) {
                            self.prog.bodies[old].0.replaced = true;
                        }
                    }
                    Redef::Replace => self.prog.funcs[id].ext_redef = true,
                    // The definition there was stays, as what
                    // `replaced` means in the new one. It is no longer
                    // what the function's name means.
                    Redef::Modify => {
                        body.closure.has_replaced = true;
                        let func = &mut self.prog.funcs[id];
                        func.mod_proto = (argc, opt_argc, varargs);
                        if let Some(old) = old_body {
                            body.closure.replaced_base = Some(old);
                            func.mod_bases.insert(0, old);
                            let old = &mut self.prog.bodies[old];
                            old.1 = None;
                            old.0.closure.func_id = None;
                        } else {
                            func.mod_extern = true;
                            func.ext_redef = true;
                        }
                    }
                    _ => {}
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
                    mod_bases: Vec::new(),
                    mod_extern: false,
                    mod_proto: (0, 0, false),
                    ext_redef: false,
                });
                let id = self.prog.funcs.len() - 1;
                self.prog.globals.insert(name, Global::Func(id));
                // There has to be something to replace or modify. The
                // first reading does not mind.
                if redef != Redef::No && !self.syntax_only {
                    self.error(
                        "\"replace\"/\"modify\" cannot be used with a function not previously defined",
                    );
                }
                id
            }
        };
        body.closure.func_id = Some(func_id);
        self.prog.body_refs[body_ref].func_body = Some(self.prog.bodies.len());
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
                    mod_bases: Vec::new(),
                    mod_extern: false,
                    mod_proto: (0, 0, false),
                    ext_redef: false,
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
        self.next();
        self.parse_anon_func_at(cb_type, cb_type == CbType::AnonMethod)
    }

    /// The name an expression is, if it is only a name: one of the
    /// program's, or a local's.
    fn sym_text(&self, expr: &Expr) -> Option<Vec<u8>> {
        match expr {
            Expr::Sym(name) => Some(name.clone()),
            Expr::Local(id) => Some(self.prog.locals[*id].name.clone()),
            _ => None,
        }
    }

    /// Read an anonymous function from its parameters or its body on.
    /// `is_method` says it has the `self` it is called with, which is
    /// the same thing as the kind of body being a method's except for
    /// a method of an object written in an expression (see there).
    fn parse_anon_func_at(&mut self, cb_type: CbType, is_method: bool) -> Option<Expr> {
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
        self.prog.nested.push(Top::Func(index));
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
            func_body: None,
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
            _ => self.parse_postfix(true, true),
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
                if !matches!(value.value, Value::Int(_) | Value::Float { .. }) {
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
                    // The one integer whose negative is not one.
                    Value::Int(i32::MIN) => {
                        let big = hexicon_bignum::Big::from_u64(2_147_483_648, 10);
                        value.value = fold::float_value(&big, true);
                    }
                    Value::Int(int) => value.value = Value::Int(-int),
                    // The sign is part of the text; and `-2147483648`
                    // is an integer after all.
                    Value::Float { ref mut text, .. } => {
                        if text.first() == Some(&b'-') {
                            text.remove(0);
                        } else {
                            text.insert(0, b'-');
                        }
                        fold::demote(&mut value.value);
                    }
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

    fn parse_postfix(&mut self, allow_member: bool, allow_call: bool) -> Option<Expr> {
        let mut sub = self.parse_primary()?;
        loop {
            match self.cur() {
                Tok::LPar if allow_call => sub = self.parse_call(sub)?,
                Tok::LBrack => sub = self.parse_subscript(sub)?,
                Tok::Dot if allow_member => sub = self.parse_member(sub)?,
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
        // With `->` between a key and its value, the list is a lookup
        // table's, and every pair has to have one. `* -> value` at the
        // end is what the table gives for a key it does not have.
        let mut is_lookup = false;
        let mut at_default = false;
        let nil = || Expr::constant(Value::Nil);
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
                Tok::Times => {
                    if self.next() == Tok::Arrow {
                        self.next();
                        if !is_lookup && !elements.is_empty() {
                            self.error(format!(
                                "unexpected -> found in list (after {} element)",
                                Self::ordinal(elements.len())
                            ));
                        } else {
                            is_lookup = true;
                            at_default = true;
                        }
                    } else {
                        self.tok.unget_token();
                    }
                }
                _ => {}
            }
            elements.push(self.parse_asi_expr()?);
            if at_default && self.cur() != Tok::RBrack {
                self.error_curtok(
                    "LookupTable list must end after default *-> value, but found \"%\"",
                );
                at_default = false;
                // More pairs, it seems: a value to keep them in step.
                if self.cur() == Tok::Comma {
                    elements.push(nil());
                }
            }
            match self.cur() {
                Tok::Comma => {
                    self.next();
                    // A key with no value.
                    if is_lookup && elements.len() % 2 != 0 {
                        self.error(format!(
                            "missing -> in {} element in LookupTable list",
                            Self::ordinal(elements.len().div_ceil(2))
                        ));
                        elements.push(nil());
                    }
                    if self.cur() == Tok::RBrack {
                        self.error(
                            "extra list element expected after comma, but found end of list",
                        );
                        break 'list;
                    }
                }
                Tok::Arrow => {
                    self.next();
                    if elements.len() == 1 {
                        is_lookup = true;
                    } else if !is_lookup {
                        self.error(format!(
                            "unexpected -> found in list (after {} element)",
                            Self::ordinal(elements.len())
                        ));
                    } else if elements.len() % 2 == 0 {
                        // A value with no key.
                        self.error(format!(
                            "misplaced -> in LookupTable list after {} element - \
                             expected comma",
                            Self::ordinal(elements.len() / 2)
                        ));
                        elements.push(nil());
                    }
                }
                Tok::RBrack => {
                    self.next();
                    if is_lookup && elements.len() % 2 != 0 && !at_default {
                        self.error(format!(
                            "missing -> in {} element in LookupTable list",
                            Self::ordinal(elements.len().div_ceil(2))
                        ));
                    }
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
        // A lookup table's list is made when the program runs, even if
        // everything in it is a constant.
        Some(if is_lookup {
            Expr::Lookup(Box::new(Expr::List(elements)))
        } else {
            Expr::list(elements)
        })
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
        // `defined(name)` asks whether a name means anything, and is
        // true or nil when the program is compiled.
        if matches!(&lhs, Expr::Sym(name) if name == b"defined") {
            // The name can be a local's; it is asked about all the
            // same as a name of the program's.
            match args.as_slice() {
                [arg] if self.sym_text(&arg.expr).is_some() => {
                    let name = self.sym_text(&arg.expr).unwrap_or_default();
                    let mut value = ConstVal::new(Value::Nil);
                    value.set_bool(self.prog.globals.contains_key(&name));
                    value.ctc = true;
                    return Some(Expr::Const(value));
                }
                _ => self.error(
                    "invalid syntax for defined() operator - argument must be a symbol name",
                ),
            }
        }
        // `__objref(name)` is the object, if the name is one's, and nil
        // if not; `__objref(name, warn)` or `error` says so as well.
        if matches!(&lhs, Expr::Sym(name) if name == b"__objref") {
            const SYNTAX: &str = "invalid syntax for __objref() operator";
            if args.len() > 2 {
                self.error(SYNTAX);
                return Some(lhs);
            }
            let mode = args.get(1).map(|arg| self.sym_text(&arg.expr));
            let handling = match mode {
                None => 0,
                Some(Some(mode)) if mode == b"warn" => 1,
                Some(Some(mode)) if mode == b"error" => 2,
                Some(_) => {
                    self.error(SYNTAX);
                    return Some(lhs);
                }
            };
            let first = args.first().map(|arg| self.sym_text(&arg.expr));
            match first {
                Some(Some(name)) => {
                    let name = &name;
                    let global = self.prog.globals.get(name).copied();
                    if matches!(global, Some(Global::Obj(_))) {
                        return Some(args[0].expr.clone());
                    }
                    if handling != 0 && !self.syntax_only {
                        let message = if global.is_none() {
                            format!("undefined symbol \"{}\"", Self::name(name))
                        } else {
                            format!(
                                "symbol \"{}\" is not an object - illegal on left of '.'",
                                Self::name(name)
                            )
                        };
                        if handling == 1 {
                            self.tok.log_warning(message);
                        } else {
                            self.error(message);
                        }
                    }
                    let mut value = ConstVal::new(Value::Nil);
                    value.ctc = true;
                    return Some(Expr::Const(value));
                }
                // The reference reads an argument that is not there.
                None => return Some(lhs),
                Some(_) => self.error(SYNTAX),
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
                // A number with a point or an exponent, or an integer
                // too large to be one.
                Tok::Float | Tok::BigInt => Value::Float {
                    text: self.cur_text(),
                    promoted: self.cur() == Tok::BigInt,
                },
                Tok::ReStr => Value::ReStr(self.cur_text()),
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
                Tok::Replaced => {
                    self.next();
                    return Some(Expr::Replaced);
                }
                Tok::ArgCount => {
                    self.next();
                    return Some(Expr::ArgCount);
                }
                Tok::Delegated => return self.parse_delegated(),
                Tok::Object => return self.parse_inline_object(false),
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
                            super_meta: None,
                            mod_count: 0,
                            mod_ids: Vec::new(),
                            mod_obj: None,
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
                    Some(Global::Meta(super_meta)) => {
                        if let Some(index) = sym {
                            self.prog.metas[index].super_meta = Some(*super_meta);
                        }
                    }
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
            // `&operator +`: the property an operator's method is.
            let (name, _, ok) = self.parse_op_name();
            if ok {
                self.next();
            }
            Expr::Sym(name)
        } else {
            self.parse_unary()?
        };
        if !matches!(sub, Expr::Sym(_)) {
            self.error("invalid address expression - can't apply '&' operator");
        }
        Some(Expr::Addr(Box::new(sub)))
    }

    /// Read the name of an operator's property, from `operator` on:
    /// `operator +`, `operator []=`, `operator negate`. The last token
    /// of the name is left current. Returns the property's name, how
    /// many operands the operator has, and whether it was an operator
    /// at all.
    fn parse_op_name(&mut self) -> (Vec<u8>, usize, bool) {
        let (name, op_args) = match self.next() {
            Tok::Sym if self.cur_text() == b"negate" => ("operator negate", 1),
            Tok::Plus => ("operator +", 2),
            Tok::Minus => ("operator -", 2),
            Tok::Times => ("operator *", 2),
            Tok::Div => ("operator /", 2),
            Tok::Mod => ("operator %", 2),
            Tok::Xor => ("operator ^", 2),
            Tok::Shl => ("operator <<", 2),
            Tok::AShr => ("operator >>", 2),
            Tok::LShr => ("operator >>>", 2),
            Tok::BNot => ("operator ~", 1),
            Tok::Or => ("operator |", 2),
            Tok::And => ("operator &", 2),
            Tok::LBrack => {
                if self.next() != Tok::RBrack {
                    self.error_curtok("expected ']' in 'operator []', found '%'");
                }
                if self.next() == Tok::Eq {
                    ("operator []=", 3)
                } else {
                    self.tok.unget_token();
                    ("operator []", 2)
                }
            }
            _ => {
                self.error_curtok("invalid overloaded operator '%'");
                return (b"unknown_operator".to_vec(), 0, false);
            }
        };
        (name.as_bytes().to_vec(), op_args, true)
    }

    /// Read an object written in an expression, from `object` on, or
    /// from its superclasses if the colon has been read:
    /// `object: Class { properties }`. It has no name and no templates,
    /// and is made when the expression is evaluated.
    fn parse_inline_object(&mut self, mut has_colon: bool) -> Option<Expr> {
        if !has_colon && self.next() == Tok::Colon {
            has_colon = true;
            self.next();
        }
        let (file, line) = self.tok.last_pos();
        let mut def = ObjDef {
            sym: None,
            is_class: false,
            superclasses: Vec::new(),
            props: Vec::new(),
            file,
            line,
            is_modify: false,
            mod_base: None,
            prev_def: None,
            mod_id: None,
            replaced: false,
            meta: None,
            transient: false,
            bad_template: false,
            undesc_sc: false,
            dict: None,
            vocab: Vec::new(),
        };
        if has_colon {
            self.parse_superclass_list(None, &mut def);
        }
        if self.cur() != Tok::LBrace {
            self.error_curtok("expected inline object property list starting with '{' (found '%')");
            return None;
        }
        self.next();
        // What the object is written in may be an object's definition
        // that is itself being read.
        let outer_values = self.inline_values.replace(Vec::new());
        let outer_unterm = self.unterm;
        let outer_term_pos = self.term_pos;
        let outer_replace = std::mem::take(&mut self.prop_replace);
        self.unterm = false;
        let result = self.parse_obj_prop_list(&mut def, true, false);
        let values = std::mem::replace(&mut self.inline_values, outer_values);
        self.unterm = outer_unterm;
        self.term_pos = outer_term_pos;
        self.prop_replace = outer_replace;
        result.ok()?;
        let props = def
            .props
            .into_iter()
            .zip(values.unwrap_or_default())
            .map(|(prop, value)| InlineProp {
                prop: prop.prop,
                value,
                file: prop.file,
                line: prop.line,
            })
            .collect();
        Some(Expr::InlineObj {
            superclasses: def.superclasses,
            props,
        })
    }

    /// Read the value of a property of an object written in an
    /// expression. It is worked out each time the property is asked
    /// for, by an anonymous method made of it, which can use the
    /// locals of the code the object is written in.
    fn parse_inline_prop_value(&mut self, file: usize, line: u32) -> Parsed<InlineValue> {
        let old_local_symtab = self.local_symtab;
        let old_enclosing_local_symtab = self.enclosing_local_symtab;
        let old_enclosing = std::mem::take(&mut self.enclosing);
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

        self.clear_local_ctx();
        let (enc, lcltab) = self.anon_symtab();
        self.local_symtab = Some(lcltab);
        self.enclosing_local_symtab = self.prog.tabs[lcltab].parent;
        self.self_valid = true;
        self.local_cnt = 0;
        self.max_local_cnt = 0;

        let expr = self.parse_expr();
        let value = expr.map(|expr| {
            let has_retval = self.prog.has_return_value(&expr);
            let kind = if has_retval {
                StmtKind::Return(Some(expr.clone()))
            } else {
                StmtKind::Expr(expr.clone())
            };
            let mut body = CodeBody {
                lcltab: Some(lcltab),
                stm: Stmt { kind, file, line },
                argc: 0,
                opt_argc: 0,
                varargs: false,
                varargs_list_local: None,
                local_cnt: self.local_cnt,
                self_valid: true,
                replaced: false,
                labels: std::rc::Rc::default(),
                end_file: file,
                end_line: line,
                closure: Closure {
                    enclosing: self.cur_code_body,
                    is_anon_method: true,
                    ..Closure::default()
                },
            };
            self.finish_local_ctx(&mut body, true);
            body.closure.self_referenced = self.self_referenced;
            body.closure.full_ctx_referenced = self.full_ctx_referenced;
            body.closure.local_ctx_needs_self = self.local_ctx_needs_self;
            body.closure.local_ctx_needs_full = self.local_ctx_needs_full;
            (expr, body, has_retval)
        });

        self.local_symtab = old_local_symtab;
        self.enclosing_local_symtab = old_enclosing_local_symtab;
        self.enclosing = old_enclosing;
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
        self.anon_finish(enc);

        let Some((expr, body, has_retval)) = value else {
            return Err(Stop);
        };
        let index = self.prog.bodies.len();
        self.prog.bodies.push((body, None));
        self.prog.nested.push(Top::Func(index));
        Ok(InlineValue::Wrapped(
            expr,
            Expr::AnonFunc {
                body: index,
                has_retval,
            },
        ))
    }

    /// Read `delegated target.prop(args)`: the target's method, run
    /// with this method's `self`. The target is one operand; a `.` or
    /// an argument list after it belongs to `delegated`.
    fn parse_delegated(&mut self) -> Option<Expr> {
        self.self_referenced = true;
        self.next();
        let target = self.parse_postfix(false, false)?;
        self.parse_member(Expr::Delegated(Box::new(target)))
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
    fn parse_object(&mut self, mut is_class: bool, redef: Redef) -> Parsed<()> {
        let name = self.cur_text();
        let found = self.find_or_def_obj(&name, &mut is_class, redef);
        self.next();
        match found {
            FoundObj::Meta(base, meta, id) => {
                self.parse_object_body(None, is_class, false, Some((base, None)), Some((meta, id)))
            }
            FoundObj::Obj(sym, modify) => {
                self.parse_object_body(sym, is_class, false, modify, None)
            }
        }
    }

    /// Find the object a definition is of, or make it: a new one, or
    /// one there is already if the definition is to take its place or
    /// go in front of it. `is_class` comes back as what the object is,
    /// if it is known for one already.
    fn find_or_def_obj(&mut self, name: &[u8], is_class: &mut bool, redef: Redef) -> FoundObj {
        let name = name.to_vec();
        let mut modify = None;
        let sym = match self.prog.globals.get(&name).copied() {
            Some(Global::Obj(obj))
                if redef != Redef::No
                    && (self.prog.objs[obj].is_dict || self.prog.objs[obj].is_gramprod) =>
            {
                self.error("objects of this type cannot be modified or replaced");
                None
            }
            Some(Global::Meta(meta)) if redef == Redef::Modify => {
                // What the program adds to a built-in class is kept in
                // objects of a kind of their own. The first is empty,
                // and each `modify` makes another, whose superclass is
                // the one before.
                let done = self.prog.metas[meta].mod_count;
                let id_for = |parser: &mut Self, place: usize| match parser.prog.metas[meta]
                    .mod_ids
                    .get(place)
                {
                    Some(id) => *id,
                    None => {
                        parser.prog.next_obj_id += 1;
                        parser.prog.next_obj_id - 1
                    }
                };
                let base = match self.prog.metas[meta].mod_obj {
                    Some(base) => base,
                    None => {
                        let root = id_for(self, 0);
                        let (file, line) = self.tok.last_pos();
                        self.prog.nested.push(Top::Obj(self.prog.objdefs.len()));
                        self.prog.objdefs.push(ObjDef {
                            sym: None,
                            is_class: false,
                            superclasses: Vec::new(),
                            props: Vec::new(),
                            file,
                            line,
                            is_modify: false,
                            mod_base: None,
                            prev_def: None,
                            mod_id: Some(root),
                            replaced: false,
                            meta: Some(meta),
                            transient: false,
                            bad_template: false,
                            undesc_sc: false,
                            dict: None,
                            vocab: Vec::new(),
                        });
                        root
                    }
                };
                let id = id_for(self, done + 1);
                let meta_sym = &mut self.prog.metas[meta];
                meta_sym.mod_count += 1;
                meta_sym.mod_obj = Some(id);
                return FoundObj::Meta(base, meta, id);
            }
            Some(Global::Obj(obj)) if redef == Redef::Modify => {
                // The definition there is becomes an object of its own,
                // with a number of its own, and the only superclass of
                // the new definition, which takes the name.
                let sym = &mut self.prog.objs[obj];
                let done = sym.mod_count;
                sym.mod_count += 1;
                if std::mem::take(&mut sym.is_extern) {
                    sym.ext_redef = true;
                }
                *is_class = sym.is_class;
                let prev = sym.def;
                let base_id = match sym.base_ids.get(done) {
                    Some(id) => *id,
                    None => {
                        self.prog.next_obj_id += 1;
                        self.prog.next_obj_id - 1
                    }
                };
                if let Some(prev) = prev {
                    let prev = &mut self.prog.objdefs[prev];
                    prev.mod_id = Some(base_id);
                    *is_class = prev.is_class;
                }
                modify = Some((base_id, prev));
                Some(obj)
            }
            Some(Global::Obj(obj)) if self.prog.objs[obj].is_extern => {
                let sym = &mut self.prog.objs[obj];
                sym.is_extern = false;
                if redef == Redef::Replace {
                    sym.ext_redef = true;
                }
                // One that `extern` declared gets its number now.
                if std::mem::take(&mut sym.declared_only) {
                    sym.id = self.prog.next_obj_id;
                    self.prog.next_obj_id += 1;
                }
                Some(obj)
            }
            Some(Global::Obj(obj)) if redef == Redef::Replace => {
                // Nothing is generated for the definition there was.
                if let Some(def) = self.prog.objs[obj].def {
                    self.prog.objdefs[def].replaced = true;
                }
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
                    sc_is_root: false,
                    templates: Vec::new(),
                    def: None,
                    mod_count: 0,
                    base_ids: Vec::new(),
                    undefined_redef: redef != Redef::No,
                    declared_only: false,
                    ext_redef: false,
                    is_dict: false,
                    dict_hint: None,
                    is_gramprod: false,
                    gram: None,
                });
                self.prog.next_obj_id += 1;
                self.prog
                    .globals
                    .insert(name, Global::Obj(self.prog.objs.len() - 1));
                // There has to be something to replace or modify. The
                // first reading does not mind.
                if redef != Redef::No && !self.syntax_only {
                    self.error(
                        "replace/modify cannot be used with an object not previously defined",
                    );
                }
                if redef == Redef::Modify {
                    modify = Some((0, None));
                }
                Some(self.prog.objs.len() - 1)
            }
        };
        if let (true, Some(obj)) = (*is_class, sym) {
            self.prog.objs[obj].is_class = true;
        }
        if redef == Redef::Modify && modify.is_none() {
            modify = Some((0, None));
        }
        FoundObj::Obj(sym, modify)
    }

    /// Read an object without a name: `superclasses properties ;`.
    fn parse_anon_object(&mut self) -> Parsed<()> {
        let obj = self.new_anon_obj();
        self.parse_object_body(Some(obj), false, true, None, None)
    }

    /// Find the grammar production a name is, or make it one. Nothing
    /// comes back if the name is something else.
    fn declare_gramprod(&mut self, name: &[u8]) -> Option<usize> {
        let obj = match self.prog.globals.get(name).copied() {
            None => {
                self.prog.objs.push(ObjSym {
                    name: name.to_vec(),
                    id: self.prog.next_obj_id,
                    is_extern: false,
                    is_class: false,
                    sc_names: Vec::new(),
                    sc_is_root: false,
                    templates: Vec::new(),
                    def: None,
                    mod_count: 0,
                    base_ids: Vec::new(),
                    undefined_redef: false,
                    declared_only: false,
                    ext_redef: false,
                    is_dict: false,
                    dict_hint: None,
                    is_gramprod: true,
                    gram: None,
                });
                self.prog.next_obj_id += 1;
                let obj = self.prog.objs.len() - 1;
                self.prog.globals.insert(name.to_vec(), Global::Obj(obj));
                obj
            }
            Some(Global::Obj(obj)) if self.prog.objs[obj].is_gramprod => {
                self.prog.objs[obj].is_extern = false;
                obj
            }
            Some(Global::Obj(_)) => {
                self.error(format!(
                    "redefining symbol \"{}\" as grammar production object",
                    Self::name(name)
                ));
                return None;
            }
            Some(_) => {
                self.error(format!(
                    "symbol \"{}\" is already defined - can't redefine as object",
                    Self::name(name)
                ));
                return None;
            }
        };
        let known = self.prog.gramprods.iter().position(|prod| prod.sym == obj);
        Some(known.unwrap_or_else(|| {
            self.prog.gramprods.push(GramProd {
                sym: obj,
                declared: false,
                alts: Vec::new(),
            });
            self.prog.gramprods.len() - 1
        }))
    }

    /// Read a `grammar` statement, from `grammar` on:
    /// `grammar name(tag): tokens : Class properties ;`. It says how a
    /// phrase of the production `name` can be made up, and defines the
    /// class of the object made when the phrase is matched.
    /// `grammar name;` only declares the production.
    fn parse_grammar(&mut self, redef: Redef) -> Parsed<()> {
        let mut prod_name = b"?".to_vec();
        let mut prod = None;
        if self.next() == Tok::Sym {
            prod_name = self.cur_text();
            prod = self.declare_gramprod(&prod_name);
        } else {
            self.error_curtok("expected symbol name after 'grammar', but found \"%\"");
        }
        let mut name_tag = prod_name.clone();
        let mut sub_tag = Vec::new();
        let mut modify = None;
        let mut is_named = false;
        let gram_obj = if self.next() == Tok::LPar {
            // With a tag, the class has a name: `name(tag)`.
            self.next();
            sub_tag = self.cur_text();
            let room = (TOK_SYM_MAX_LEN * 2 + 32).saturating_sub(prod_name.len() + 2);
            name_tag.push(b'(');
            name_tag.extend_from_slice(&sub_tag[..sub_tag.len().min(room)]);
            name_tag.push(b')');
            if self.next() == Tok::RPar {
                self.next();
            } else {
                self.error_curtok(
                    "missing right parenthesis after name in 'grammar' - found \"%\"",
                );
            }
            is_named = true;
            let mut is_class = true;
            match self.find_or_def_obj(&name_tag, &mut is_class, redef) {
                FoundObj::Obj(sym, found_modify) => {
                    modify = found_modify;
                    sym
                }
                FoundObj::Meta(..) => None,
            }
        } else if redef == Redef::No && self.cur() == Tok::Sem {
            // Only a declaration.
            if let Some(prod) = prod {
                self.prog.gramprods[prod].declared = true;
            }
            self.next();
            return Ok(());
        } else {
            let obj = self.new_anon_obj();
            if redef != Redef::No {
                self.error("'modify/replace grammar' requires name-tag");
            }
            Some(obj)
        };
        if redef == Redef::Modify && modify.is_none() {
            modify = Some((0, None));
        }
        if self.cur() == Tok::Colon {
            self.next();
        } else {
            self.error_curtok("expected ':' after production name, but found \"%\"");
        }
        // The object is a class: the objects are made as phrases are
        // matched.
        if let Some(obj) = gram_obj {
            self.prog.objs[obj].is_class = true;
        }
        let mut arrows = Vec::new();
        if redef == Redef::Modify && self.cur() == Tok::Colon {
            // A `modify` with nothing between the colons keeps the
            // ways there were.
            self.next();
        } else {
            // With a tag, the ways are kept with the name until the
            // program is put together, in case a `modify` or a
            // `replace` changes them; the production is looked up
            // again for that.
            let private = is_named && gram_obj.is_some();
            if private {
                prod = self.declare_gramprod(&prod_name);
            }
            let alts = self.parse_gram_alts(gram_obj, &mut arrows)?;
            match (private, gram_obj, prod) {
                (true, Some(obj), _) => self.prog.objs[obj].gram = Some((prod, alts)),
                (false, _, Some(prod)) => self.prog.gramprods[prod].alts.extend(alts),
                _ => {}
            }
        }

        self.parse_object_body(gram_obj, true, true, modify, None)?;

        // The class says what it is: its name, and then the properties
        // its ways give values to; and its tag.
        let Some(def) = gram_obj.and_then(|obj| self.prog.objs[obj].def) else {
            return Ok(());
        };
        let (file, line) = (self.prog.objdefs[def].file, self.prog.objdefs[def].line);
        let mut info = vec![Expr::constant(Value::SStr(name_tag))];
        info.extend(
            arrows
                .into_iter()
                .map(|prop: usize| Expr::Sym(self.prog.props[prop].name.clone())),
        );
        for (prop, value) in [
            (GRAMMAR_INFO_PROP, Expr::list(info)),
            (GRAMMAR_TAG_PROP, Expr::constant(Value::SStr(sub_tag))),
        ] {
            self.prog.objdefs[def].props.push(ObjProp {
                prop,
                value: PropValue::Expr(value),
                overwritable: false,
                is_static: false,
                file,
                line,
            });
        }
        Ok(())
    }

    /// Read the ways a phrase can be made up, to the colon that ends
    /// them. Parentheses group alternatives inside a row
    /// (`('take' | 'get') nounPhrase`); the rows are first written out
    /// in full, one for every choice, and then read.
    fn parse_gram_alts(
        &mut self,
        gram_obj: Option<usize>,
        arrows: &mut Vec<usize>,
    ) -> Parsed<Vec<GramAlt>> {
        const INVAL_TOK: &str = "invalid token \"%\" in 'grammar' token list";
        const ARROW_REQ_PROP: &str =
            "'->' in 'grammar' token list must be followed by a property name (found \"%\")";
        let new_alt = |parser: &Self| GramAlt {
            obj: gram_obj,
            dict: parser.cur_dict,
            badness: 0,
            toks: Vec::new(),
        };
        let mut alts = vec![new_alt(self)];
        // The rows in full go in front of the token that ended them.
        let rows = self.parse_gram_or(0);
        let mut list = Vec::new();
        for (index, row) in rows.into_iter().enumerate() {
            if index != 0 {
                let mut or = SrcToken::new(Tok::Or);
                or.text = b"|".to_vec();
                list.push(or);
            }
            list.extend(row);
        }
        let current = self.tok.cur_token().clone();
        self.tok.push_token(current);
        for token in list.into_iter().rev() {
            self.tok.push_token(token);
        }
        self.next();

        loop {
            let last = alts.len() - 1;
            // What was read to match, if anything, for an arrow to
            // give a property to.
            let tok;
            match self.cur() {
                Tok::LBrack => {
                    // `[badness number]`, before anything else.
                    if !alts[last].toks.is_empty() {
                        self.tok
                            .log_warning("grammar qualifiers must precede all tokens");
                    }
                    let mut skip_to_close = false;
                    let mut done = false;
                    if self.next() == Tok::Sym {
                        if self.cur_matches(b"badness") {
                            alts[last].badness = self.parse_gram_qual_int("badness", &mut done);
                        } else {
                            self.error_curtok("invalid grammar qualifier \"%\"");
                            skip_to_close = true;
                        }
                        if !skip_to_close {
                            if self.cur() == Tok::RBrack {
                                self.next();
                            } else {
                                self.error_curtok(
                                    "']' required after grammar qualifier - found \"%\"",
                                );
                                skip_to_close = true;
                            }
                        }
                    } else {
                        self.error_curtok(
                            "keyword required after '[' in grammar qualifier - found \"%\"",
                        );
                        skip_to_close = true;
                    }
                    if skip_to_close {
                        self.parse_gram_qual_skip(&mut done);
                    }
                    if done {
                        return Ok(alts);
                    }
                    continue;
                }
                Tok::Colon => {
                    self.next();
                    return Ok(alts);
                }
                Tok::Or => {
                    alts.push(new_alt(self));
                    self.next();
                    continue;
                }
                Tok::SStr | Tok::DStr => {
                    alts[last].toks.push(GramTok {
                        kind: GramTokKind::Literal(self.cur_text()),
                        prop: None,
                    });
                    tok = true;
                }
                Tok::Lt => {
                    // `<noun adjective>`: a word of any of these.
                    self.next();
                    let mut props = Vec::new();
                    loop {
                        match self.cur() {
                            Tok::Gt => break,
                            Tok::Eof => {
                                self.error_curtok(INVAL_TOK);
                                return Err(Stop);
                            }
                            Tok::Sym => {
                                let name = self.cur_text();
                                let prop = match self.prog.globals.get(&name).copied() {
                                    Some(Global::Prop(prop)) => {
                                        self.prog.props[prop].weak = false;
                                        Some(prop)
                                    }
                                    None => Some(self.prog.def_prop(&name)),
                                    Some(_) => None,
                                };
                                match prop {
                                    Some(prop) => props.push(prop),
                                    None => self.error_curtok(
                                        "property name required in list in 'grammar', \
                                         but found \"%\"",
                                    ),
                                }
                                self.next();
                            }
                            _ => {
                                self.error_curtok(
                                    "missing '>' in 'grammar' property list - found \"%\"",
                                );
                                break;
                            }
                        }
                    }
                    alts[last].toks.push(GramTok {
                        kind: GramTokKind::SpeechList(props),
                        prop: None,
                    });
                    tok = true;
                }
                Tok::Sym => {
                    // A property is a word of that kind; an object, a
                    // phrase of that production; a value of an
                    // enumeration, a token of that kind. A name that
                    // is nothing becomes a production, but not on the
                    // first reading, which passes it over.
                    let name = self.cur_text();
                    let mut global = self.prog.globals.get(&name).copied();
                    if global.is_none() && !self.syntax_only {
                        let Some(prod) = self.declare_gramprod(&name) else {
                            return Ok(alts);
                        };
                        global = Some(Global::Obj(self.prog.gramprods[prod].sym));
                    }
                    let kind = match global {
                        None => None,
                        Some(Global::Obj(obj)) => {
                            if !self.prog.objs[obj].is_gramprod {
                                self.error_curtok(
                                    "object \"%\" is not valid in a grammar rule - \
                                     only production names are allowed",
                                );
                            }
                            Some(GramTokKind::Prod(obj))
                        }
                        Some(Global::Prop(prop)) => Some(GramTokKind::Speech(prop)),
                        Some(Global::Enum(sym)) => {
                            if !self.prog.enums[sym].is_token {
                                self.error_curtok(
                                    "enumerator \"%\" in 'grammar' list is not declared \
                                     with 'enum token'",
                                );
                            }
                            Some(GramTokKind::TokType(sym))
                        }
                        Some(_) => {
                            self.error_curtok(
                                "object or property symbol required in 'grammar' token list \
                                 (found \"%\")",
                            );
                            None
                        }
                    };
                    tok = kind.is_some();
                    if let Some(kind) = kind {
                        alts[last].toks.push(GramTok { kind, prop: None });
                    }
                }
                Tok::Times => {
                    alts[last].toks.push(GramTok {
                        kind: GramTokKind::Star,
                        prop: None,
                    });
                    self.next();
                    if !matches!(self.cur(), Tok::Or | Tok::Colon) {
                        self.error_curtok(
                            "'*' must be the last token in a 'grammar' alternative list \
                             (found \"%\")",
                        );
                    }
                    continue;
                }
                Tok::Eof => {
                    self.error_curtok(INVAL_TOK);
                    return Err(Stop);
                }
                Tok::LBrace | Tok::RBrace | Tok::Sem => {
                    // The end of the statement, it seems.
                    self.error_curtok(INVAL_TOK);
                    return Ok(alts);
                }
                _ => {
                    self.error_curtok(INVAL_TOK);
                    self.next();
                    continue;
                }
            }
            // `-> prop` after something to match keeps the match in
            // the property.
            if self.next() == Tok::Arrow {
                if self.next() == Tok::Sym {
                    let name = self.cur_text();
                    let prop = match self.prog.globals.get(&name).copied() {
                        Some(Global::Prop(prop)) => {
                            self.prog.props[prop].weak = false;
                            Some(prop)
                        }
                        None => Some(self.prog.def_prop(&name)),
                        Some(_) => None,
                    };
                    match prop {
                        Some(prop) => {
                            if tok {
                                if let Some(last_tok) = alts[last].toks.last_mut() {
                                    last_tok.prop = Some(prop);
                                }
                            }
                            if !arrows.contains(&prop) && arrows.len() < 100 {
                                arrows.push(prop);
                            }
                        }
                        None => self.error_curtok(ARROW_REQ_PROP),
                    }
                    self.next();
                } else {
                    self.error_curtok(ARROW_REQ_PROP);
                }
            }
        }
    }

    /// Read alternatives separated by `|`, each a row, and write them
    /// out in full: one row of tokens for every choice among the
    /// groups in parentheses, the first group's choices changing
    /// fastest.
    fn parse_gram_or(&mut self, level: usize) -> Vec<Vec<SrcToken>> {
        if level == 0 && self.cur() == Tok::Colon && !self.syntax_only {
            self.tok
                .log_warning("grammar rule is empty (add '()' if this is intentional)");
        }
        let mut rows = self.parse_gram_cat(level + 1);
        while self.cur() == Tok::Or {
            self.next();
            if level == 0 && self.cur() == Tok::Colon && !self.syntax_only {
                self.tok.log_warning(
                    "grammar rule ends with '|' (add '()' if empty last rule is intentional)",
                );
            }
            rows.extend(self.parse_gram_cat(level + 1));
        }
        rows
    }

    /// Read one row, up to a `|` or whatever ends the rows.
    fn parse_gram_cat(&mut self, level: usize) -> Vec<Vec<SrcToken>> {
        // The row's items, each as the rows it can be.
        let mut items: Vec<Vec<Vec<SrcToken>>> = Vec::new();
        loop {
            match self.cur() {
                Tok::LPar => {
                    self.next();
                    items.push(self.parse_gram_or(level + 1));
                    if self.cur() == Tok::RPar {
                        self.next();
                    } else {
                        self.error_curtok(
                            "missing right parentheses after group in 'grammar' - found \"%\"",
                        );
                    }
                    if self.cur() == Tok::Arrow {
                        self.error(
                            "'->' is not allowed after parenthesized group in 'grammar' statement",
                        );
                    }
                }
                Tok::Or
                | Tok::Colon
                | Tok::Sem
                | Tok::LBrace
                | Tok::RBrace
                | Tok::Eof
                | Tok::RPar => break,
                _ => {
                    items.push(vec![vec![self.tok.cur_token().clone()]]);
                    self.next();
                }
            }
        }
        // Every choice: the first item that has several goes through
        // them fastest.
        let mut rows = Vec::new();
        let mut choice = vec![0; items.len()];
        loop {
            rows.push(
                items
                    .iter()
                    .zip(&choice)
                    .flat_map(|(item, at)| item[*at].iter().cloned())
                    .collect(),
            );
            let mut index = 0;
            loop {
                if index == items.len() {
                    return rows;
                }
                choice[index] += 1;
                if choice[index] < items[index].len() {
                    break;
                }
                choice[index] = 0;
                index += 1;
            }
        }
    }

    /// Read the number in `[badness number]`, from the word on. It has
    /// to be a constant integer.
    fn parse_gram_qual_int(&mut self, qual: &str, done: &mut bool) -> i32 {
        self.next();
        let expr = if self.cur() == Tok::RBrack {
            None
        } else {
            self.parse_expr()
        };
        match expr
            .as_ref()
            .and_then(Expr::as_const)
            .map(|value| &value.value)
        {
            Some(Value::Int(value)) => *value,
            _ => {
                self.error(format!(
                    "grammar qualifier \"[{qual}]\" requires integer constant value"
                ));
                self.parse_gram_qual_skip(done);
                0
            }
        }
    }

    /// After a mistake in `[...]`, pass over what is left of the
    /// statement: the reference means to stop at the `]`, but goes on
    /// to a semicolon or a brace.
    fn parse_gram_qual_skip(&mut self, done: &mut bool) {
        loop {
            match self.next() {
                Tok::RBrack => {
                    self.next();
                }
                Tok::Eof | Tok::RBrace | Tok::LBrace | Tok::Sem => {
                    *done = true;
                    return;
                }
                _ => {}
            }
        }
    }

    /// Read `dictionary name;`, which makes the dictionary the one that
    /// the words of the objects that follow go in, or
    /// `dictionary property name, name;`, which says which properties
    /// have words for values.
    fn parse_dict(&mut self) -> Parsed<()> {
        match self.next() {
            Tok::Property => {
                self.next();
                self.parse_dict_props();
                Ok(())
            }
            Tok::Sym => {
                let name = self.cur_text();
                match self.prog.globals.get(&name).copied() {
                    None => {
                        self.prog.objs.push(ObjSym {
                            name: name.clone(),
                            id: self.prog.next_obj_id,
                            is_extern: false,
                            is_class: false,
                            sc_names: Vec::new(),
                            sc_is_root: false,
                            templates: Vec::new(),
                            def: None,
                            mod_count: 0,
                            base_ids: Vec::new(),
                            undefined_redef: false,
                            declared_only: false,
                            ext_redef: false,
                            is_dict: true,
                            dict_hint: None,
                            is_gramprod: false,
                            gram: None,
                        });
                        self.prog.next_obj_id += 1;
                        let obj = self.prog.objs.len() - 1;
                        self.prog.globals.insert(name, Global::Obj(obj));
                        self.prog.dicts.push(obj);
                        self.cur_dict = Some(obj);
                    }
                    Some(Global::Obj(obj)) if self.prog.objs[obj].is_dict => {
                        // Declared before, here or on the first
                        // reading.
                        self.prog.objs[obj].is_extern = false;
                        if !self.prog.dicts.contains(&obj) {
                            self.prog.dicts.push(obj);
                        }
                        self.cur_dict = Some(obj);
                    }
                    // The reference reports these and then stops with
                    // a memory fault.
                    Some(Global::Obj(_)) => {
                        self.error_curtok("redefining symbol \"%\" as dictionary object");
                    }
                    Some(_) => {
                        self.error_curtok(
                            "symbol \"%\" is already defined - can't redefine as object",
                        );
                    }
                }
                self.next();
                self.parse_req_sem()
            }
            _ => {
                self.error_curtok(
                    "expected 'property' or object name after 'dictionary', but found \"%\"",
                );
                Ok(())
            }
        }
    }

    /// Read the names in `dictionary property name, name;`.
    fn parse_dict_props(&mut self) {
        const REQ_SYM: &str =
            "expected property name in 'dictionary property' list, but found \"%\"";
        const REQ_COMMA: &str = "expected comma in 'dictionary property' list, but found \"%\"";
        loop {
            if self.cur() == Tok::Sym {
                let name = self.cur_text();
                if let Some(prop) = self.look_up_prop(&name) {
                    if !self.prog.props[prop].vocab {
                        self.prog.props[prop].vocab = true;
                        self.prog.dict_props.insert(0, prop);
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
                    // A comma left out, it seems.
                    Tok::Sym => self.error_curtok(REQ_COMMA),
                    Tok::LBrace | Tok::RBrace | Tok::Eof => {
                        self.error_curtok("expected ';', but found \"%\"");
                        return;
                    }
                    _ => {
                        self.error_curtok(REQ_COMMA);
                        self.next();
                        if self.cur() == Tok::Comma {
                            self.next();
                        } else if self.cur() == Tok::Sem {
                            self.next();
                            return;
                        }
                    }
                }
            } else {
                self.error_curtok(REQ_SYM);
                match self.cur() {
                    Tok::Sem | Tok::LBrace | Tok::RBrace | Tok::Eof => return,
                    Tok::Comma => {
                        self.next();
                    }
                    _ => {
                        self.next();
                        if self.cur() == Tok::Comma {
                            self.next();
                        }
                    }
                }
                if self.cur() == Tok::Sem {
                    self.next();
                    return;
                }
            }
        }
    }

    /// Read an object that starts with `+` signs, which say what it is
    /// in: the last object with one sign fewer. Or `+ property name;`,
    /// which says which property that is the value of.
    fn parse_plus_object(&mut self) -> Parsed<()> {
        let mut cnt = 0;
        while matches!(self.cur(), Tok::Plus | Tok::Inc) {
            cnt += if self.cur() == Tok::Inc { 2 } else { 1 };
            self.next();
        }
        if cnt == 1 && self.cur() == Tok::Property {
            if self.next() == Tok::Sym {
                let name = self.cur_text();
                self.plus_prop = self.look_up_prop(&name);
                self.next();
            } else {
                self.error_curtok("symbol expected after '+ property', but found \"%\" instead");
                if self.cur() != Tok::Sem {
                    self.next();
                }
            }
            return self.parse_req_sem();
        }
        let mut is_class = false;
        if self.cur() == Tok::Class {
            self.next();
            is_class = true;
        } else if self.cur() == Tok::Transient {
            self.next();
            self.transient = true;
        }
        // A name with no colon after it is a class's, and the object
        // has no name.
        let anon = match self.cur() {
            Tok::Sym => {
                let after = self.next();
                self.tok.unget_token();
                after != Tok::Colon
            }
            Tok::Object => true,
            _ => false,
        };
        self.plus_cnt = cnt;
        let result = if anon {
            self.parse_anon_object()
        } else {
            self.parse_object(is_class, Redef::No)
        };
        self.plus_cnt = 0;
        self.transient = false;
        result
    }

    /// Make the object a definition without a name defines.
    fn new_anon_obj(&mut self) -> usize {
        self.prog.objs.push(ObjSym {
            name: b".anon".to_vec(),
            id: self.prog.next_obj_id,
            is_extern: false,
            is_class: false,
            sc_names: Vec::new(),
            sc_is_root: false,
            templates: Vec::new(),
            def: None,
            mod_count: 0,
            base_ids: Vec::new(),
            undefined_redef: false,
            declared_only: false,
            ext_redef: false,
            is_dict: false,
            dict_hint: None,
            is_gramprod: false,
            gram: None,
        });
        self.prog.next_obj_id += 1;
        self.prog.objs.len() - 1
    }

    fn parse_object_body(
        &mut self,
        sym: Option<usize>,
        is_class: bool,
        is_anon: bool,
        modify: Option<(u32, Option<usize>)>,
        meta: Option<(usize, u32)>,
    ) -> Parsed<()> {
        let old_self_valid = self.self_valid;
        self.self_valid = true;
        let result = self.parse_object_body_in(sym, is_class, is_anon, modify, meta);
        self.self_valid = old_self_valid;
        result
    }

    fn parse_object_body_in(
        &mut self,
        sym: Option<usize>,
        is_class: bool,
        is_anon: bool,
        modify: Option<(u32, Option<usize>)>,
        meta: Option<(usize, u32)>,
    ) -> Parsed<()> {
        let plus_cnt = std::mem::take(&mut self.plus_cnt);
        let is_nested = std::mem::take(&mut self.obj_nested);
        if !is_nested {
            self.unterm = false;
        }
        self.dict_defined.clear();
        let mut def = ObjDef {
            sym,
            is_class,
            superclasses: Vec::new(),
            props: Vec::new(),
            file: 0,
            line: 0,
            is_modify: modify.is_some(),
            mod_base: None,
            prev_def: None,
            mod_id: meta.map(|(_, id)| id),
            replaced: false,
            meta: meta.map(|(meta, _)| meta),
            transient: std::mem::take(&mut self.transient),
            bad_template: false,
            undesc_sc: false,
            dict: if meta.is_some() { None } else { self.cur_dict },
            vocab: Vec::new(),
        };
        // A definition that takes another's place with `replace` keeps
        // the words the other had, and the dictionary they go in: the
        // reference keeps both with the name.
        if let Some(old) = sym.and_then(|obj| self.prog.objs[obj].def) {
            let old = &self.prog.objdefs[old];
            if old.replaced && modify.is_none() {
                def.vocab = old.vocab.clone();
                def.dict = old.dict;
            }
        }
        // A `modify` is transient if what it modifies is.
        if let Some((_, Some(prev))) = modify {
            def.transient = self.prog.objdefs[prev].transient;
        }
        let (file, line) = self.tok.last_pos();
        (def.file, def.line) = (file, line);
        if let Some((base_id, prev)) = modify {
            // No superclasses are written: the definition modified is
            // the one there is.
            if base_id != 0 {
                def.mod_base = Some(base_id);
            }
            def.prev_def = prev;
        } else {
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
        }

        // With `+` signs, the object is in the last one that had one
        // sign fewer.
        if plus_cnt != 0 {
            let loc = self.plus_stack.get(plus_cnt - 1).copied().flatten();
            match (self.plus_prop, loc) {
                (Some(prop), Some(loc)) => {
                    // That definition's number: its own if a `modify`
                    // has taken its name since.
                    let loc = &self.prog.objdefs[loc];
                    let id = loc.mod_id.or(loc.sym.map(|obj| self.prog.objs[obj].id));
                    if let Some(id) = id {
                        def.props.push(ObjProp {
                            prop,
                            value: PropValue::Expr(Expr::constant(Value::Obj(id))),
                            overwritable: true,
                            is_static: false,
                            file,
                            line,
                        });
                    }
                }
                _ => self.error("too many '+' signs in object definition - location not defined"),
            }
        }

        let mut braces = false;
        if self.cur() == Tok::LBrace {
            self.next();
            braces = true;
        }
        // Values with no property names: the object's classes say, in
        // templates, which properties they are for.
        self.parse_obj_template(&mut def)?;
        if !braces && self.cur() == Tok::LBrace {
            braces = true;
            self.next();
        }
        // An object that is a property's value has to be in braces.
        // Without them it is taken to be the next definition, the one
        // being read having ended without saying so.
        if is_nested && !braces {
            let (file, line) = self.term_pos;
            self.tok.log_at(
                file,
                line,
                Severity::Error,
                "unterminated object definition".to_owned(),
            );
            self.unterm = true;
        }

        // An object that is not a class knows where it comes in the
        // source.
        if !is_class && modify.is_none() {
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

        self.parse_obj_prop_list(&mut def, braces, is_nested)?;
        // A dictionary property the object has given no words for is
        // there all the same, in case the object's classes have some.
        if meta.is_none() {
            for index in 0..self.prog.dict_props.len() {
                let prop = self.prog.dict_props[index];
                if !self.dict_defined.contains(&prop) {
                    def.props.push(ObjProp {
                        prop,
                        value: PropValue::VocabList,
                        overwritable: false,
                        is_static: false,
                        file,
                        line,
                    });
                }
            }
        }
        if is_nested {
            // The object it is in puts it with the definitions that
            // are generated after the rest, unless it turned out not
            // to be one.
            if !self.unterm {
                if let Some(obj) = sym {
                    self.prog.objs[obj].def = Some(self.prog.objdefs.len());
                }
                self.nested_def = Some(self.prog.objdefs.len());
                self.prog.objdefs.push(def);
            }
            return Ok(());
        }
        if let Some(obj) = sym {
            self.prog.objs[obj].def = Some(self.prog.objdefs.len());
        }
        // The next object with one `+` sign more is in this one. A
        // class counts only if it has `+` signs itself.
        if !is_class || plus_cnt != 0 {
            if self.plus_stack.len() <= plus_cnt {
                self.plus_stack.resize(plus_cnt + 1, None);
            }
            self.plus_stack[plus_cnt] = Some(self.prog.objdefs.len());
        }
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
                    if let Some(obj) = sym {
                        self.prog.objs[obj].sc_is_root = true;
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
    fn parse_obj_prop_list(
        &mut self,
        def: &mut ObjDef,
        braces: bool,
        is_nested: bool,
    ) -> Parsed<()> {
        // The groups of properties open around this object, if it is a
        // property's value, are nothing to it.
        let outer_sets = std::mem::take(&mut self.propsets);
        let outer_depth = std::mem::take(&mut self.propset_depth);
        let result = self.parse_obj_prop_list_in(def, braces, is_nested);
        self.propsets = outer_sets;
        self.propset_depth = outer_depth;
        result
    }

    fn parse_obj_prop_list_in(
        &mut self,
        def: &mut ObjDef,
        braces: bool,
        is_nested: bool,
    ) -> Parsed<()> {
        const REQ_RBRACE: &str = "missing right brace at end of object definition - found \"%\"";
        const REQ_SEM: &str = "missing semicolon at end of object definition - found \"%\"";
        loop {
            let missing_end = if braces || self.propset_depth != 0 {
                REQ_RBRACE
            } else {
                REQ_SEM
            };
            match self.cur() {
                Tok::PropertySet => self.parse_propertyset()?,
                Tok::Sem => {
                    // Inside braces a semicolon is an empty property.
                    self.next();
                    if !braces && self.propset_depth == 0 {
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
                    if !is_nested {
                        self.term_pos = self.tok.last_pos();
                    }
                    self.parse_obj_prop(def, is_nested)?;
                    if self.unterm {
                        return Ok(());
                    }
                }
                // `replace` before a property of a `modify` takes the
                // property out of the definitions modified.
                Tok::Replace => {
                    if !def.is_modify {
                        self.error(
                            "\"replace\" can be used with a property only in a \"modify\" object",
                        );
                    }
                    self.next();
                    if !is_nested {
                        self.term_pos = self.tok.last_pos();
                    }
                    self.prop_replace = true;
                    let result = self.parse_obj_prop(def, is_nested);
                    self.prop_replace = false;
                    result?;
                    if self.unterm {
                        return Ok(());
                    }
                }
                // The method that stands for an operator.
                Tok::Operator => {
                    if !is_nested {
                        self.term_pos = self.tok.last_pos();
                    }
                    self.parse_obj_prop(def, is_nested)?;
                    if self.unterm {
                        return Ok(());
                    }
                }
                // The end of a group of properties, if one is open.
                Tok::RBrace if self.propset_depth != 0 => {
                    self.propset_depth -= 1;
                    self.propsets.truncate(self.propset_depth);
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

    /// Read the start of a group of properties, up to its opening
    /// brace: `propertyset 'pattern*' (params, *) {`. Each property in
    /// the group has the name the pattern makes of the name it is
    /// written with, and each method the group's parameters around its
    /// own.
    fn parse_propertyset(&mut self) -> Parsed<()> {
        // A group too deep is read, and that is all.
        let kept = self.propset_depth < MAX_PROPSET_DEPTH;
        if self.propset_depth == MAX_PROPSET_DEPTH {
            self.error("'propertyset' nesting is too deep");
        }
        self.propset_depth += 1;
        let mut set = PropSet::default();
        if self.next() == Tok::SStr {
            set.pattern = self.cur_text();
            // The characters of a name, and one `*`.
            let stars = set.pattern.iter().filter(|&&c| c == b'*').count();
            let bad = set
                .pattern
                .iter()
                .any(|&c| c != b'*' && !crate::lex::is_sym(c));
            let bad_first = set
                .pattern
                .first()
                .is_some_and(|&c| c != b'*' && !crate::lex::is_syminit(c));
            if stars != 1 || bad || bad_first {
                self.error_curtok("propertyset pattern string \"%\" is invalid");
            }
        } else {
            self.error_curtok(
                "expected property name pattern string after 'propertyset' - found \"%\"",
            );
        }
        if self.next() == Tok::LPar {
            self.next();
            // What came last: nothing, a parameter, or a comma.
            let mut state = 0;
            let mut stars = 0;
            loop {
                match self.cur() {
                    Tok::LBrace | Tok::RBrace | Tok::Sem => {
                        self.error_curtok(
                            "missing right parenthesis ')' at end of formal parameter list - \
                             found \"%\"",
                        );
                        break;
                    }
                    Tok::RPar => {
                        if state == 2 {
                            self.error("missing parameter name at end of formal parameter list");
                        }
                        self.next();
                        break;
                    }
                    Tok::Sym | Tok::Times => {
                        if state == 1 {
                            self.error_curtok(
                                "expected a comma in formal parameter list, but found \"%\"",
                            );
                        }
                        state = 1;
                        if self.cur() == Tok::Times {
                            stars += 1;
                        }
                        set.params.push(self.tok.cur_token().clone());
                        self.next();
                    }
                    Tok::Comma if state == 1 => {
                        state = 2;
                        set.params.push(self.tok.cur_token().clone());
                        self.next();
                    }
                    _ => {
                        self.error_curtok(if state == 1 {
                            "expected a comma in formal parameter list, but found \"%\""
                        } else {
                            "expected a symbol in formal parameter list, but found \"%\""
                        });
                        self.next();
                        // At the end of the source this goes round for
                        // ever; what stops the reference is its limit
                        // on errors.
                        if self.tok.gave_up() {
                            return Err(Stop);
                        }
                    }
                }
            }
            if stars != 1 {
                self.error("invalid formal parameters in propertyset definition");
            }
        }
        if self.cur() == Tok::LBrace {
            self.next();
        } else {
            self.error_curtok("missing open brace '{' in propertyset definition - found \"%\"");
        }
        if kept {
            self.propsets.push(set);
        }
        Ok(())
    }

    /// The name a property written inside groups of properties has:
    /// each group's pattern with the name so far in place of its `*`,
    /// the innermost group's first.
    fn propset_name(&mut self, mut name: Vec<u8>) -> Vec<u8> {
        for index in (0..self.propsets.len()).rev() {
            let pattern = &self.propsets[index].pattern;
            if name.len() + pattern.len() > TOK_SYM_MAX_LEN {
                self.error("expanded property name in propertyset is too long");
                break;
            }
            let star = pattern.iter().position(|&c| c == b'*');
            let (before, after) = match star {
                Some(star) => (&pattern[..star], &pattern[star + 1..]),
                None => (&pattern[..], &pattern[..0]),
            };
            name = [before, &name, after].concat();
        }
        name
    }

    /// Put the parameters the open groups of properties give a method
    /// around the method's own, which are about to be read: what each
    /// group has before its `*`, the outermost group's first; the
    /// method's own; and what each has after its `*`, the innermost
    /// group's first.
    ///
    /// The reference builds this list in a variable of a function that
    /// has returned by the time the list is read, and stops with a
    /// memory fault.
    fn insert_propset_params(&mut self) {
        if self.propsets.iter().all(|set| set.params.is_empty()) {
            return;
        }
        let split = |set: &PropSet| -> (Vec<SrcToken>, Vec<SrcToken>) {
            let params = &set.params;
            let star = params.iter().position(|token| token.typ == Tok::Times);
            let Some(star) = star else {
                return (params.clone(), Vec::new());
            };
            let mut before = params[..star].to_vec();
            if before.last().is_some_and(|token| token.typ == Tok::Comma) {
                before.pop();
            }
            let mut after = params[star + 1..].to_vec();
            if after.first().is_some_and(|token| token.typ == Tok::Comma) {
                after.remove(0);
            }
            (before, after)
        };
        let mut pieces = Vec::new();
        let mut tail = Vec::new();
        for set in &self.propsets {
            let (before, after) = split(set);
            pieces.push(before);
            tail.push(after);
        }
        // The method's own, if it has any.
        let mut own = Vec::new();
        if self.cur() == Tok::LPar {
            let mut depth = 0;
            loop {
                match self.next() {
                    Tok::Eof => break,
                    Tok::LPar => depth += 1,
                    Tok::RPar if depth == 0 => {
                        self.next();
                        break;
                    }
                    Tok::RPar => depth -= 1,
                    _ => {}
                }
                own.push(self.tok.cur_token().clone());
            }
        }
        pieces.push(own);
        pieces.extend(tail.into_iter().rev());

        let token = |typ: Tok, text: &[u8]| {
            let mut token = SrcToken::new(typ);
            token.text = text.to_vec();
            token
        };
        let mut list = vec![token(Tok::LPar, b"(")];
        for piece in pieces.into_iter().filter(|piece| !piece.is_empty()) {
            if list.len() > 1 {
                list.push(token(Tok::Comma, b","));
            }
            list.extend(piece);
        }
        list.push(token(Tok::RPar, b")"));
        // The list goes in front of the token that came after the
        // method's own.
        let current = self.tok.cur_token().clone();
        self.tok.push_token(current);
        for token in list.into_iter().rev() {
            self.tok.push_token(token);
        }
        self.next();
    }

    /// Read one property of an object, from its name on. If the
    /// definition turns out to have ended without saying so, with the
    /// next definition starting here, that is noted.
    fn parse_obj_prop(&mut self, def: &mut ObjDef, obj_is_nested: bool) -> Parsed<()> {
        let (file, line) = self.tok.last_pos();
        (self.cur_file, self.cur_line) = (file, line);
        let mut name = self.cur_text();
        let is_op = self.cur() == Tok::Operator;
        if self.propset_depth != 0 {
            if is_op {
                self.error("an 'operator' property cannot be defined within a 'propertyset'");
            }
            name = self.propset_name(name);
        }
        // `operator +` and the like: the name is the operator's, and a
        // method has to have the operator's operands, but for the first,
        // which is the object.
        let mut op_args = 0;
        if is_op {
            (name, op_args, _) = self.parse_op_name();
        }
        let existing = self.prog.globals.get(&name).copied();
        let mut new_prop = None;
        let mut is_value = false;
        let mut no_value = false;
        // The property the name turned out to be, whatever came of
        // reading its value.
        let mut defined_prop = None;
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
                let (term_file, term_line) = self.term_pos;
                if matches!(existing, Some(Global::Obj(_))) {
                    // `name:` where the name is an object's: the next
                    // definition, this one's end having been left out.
                    self.tok.log_at(
                        term_file,
                        term_line,
                        Severity::Error,
                        "unterminated object definition".to_owned(),
                    );
                    self.tok.unget_token();
                    self.unterm = true;
                    return Ok(());
                }
                // An object of its own as the property's value, in
                // braces: `name: Class { ... }`.
                self.next();
                if self.inline_values.is_some() {
                    // In an object written in an expression it is
                    // another such, made when that one is, and told
                    // that the object it is written in is none.
                    let Some(mut nested) = self.parse_inline_object(true) else {
                        return Err(Stop);
                    };
                    if let Expr::InlineObj { props, .. } = &mut nested {
                        props.push(InlineProp {
                            prop: LEXICAL_PARENT_PROP,
                            value: InlineValue::Eval(Expr::constant(Value::Obj(0))),
                            file,
                            line,
                        });
                    }
                    let prop = self.look_up_prop(&name);
                    if prop.is_some_and(|prop| self.prog.props[prop].vocab) {
                        self.error("vocabulary property requires string value, but found \":\"");
                    }
                    defined_prop = prop;
                    if let Some(prop) = prop {
                        new_prop = Some(def.props.len());
                        self.push_inline_prop(def, prop, InlineValue::Eval(nested), file, line);
                    }
                    no_value = true;
                }
                if self.inline_values.is_none() {
                    let obj = self.new_anon_obj();
                    let replace = std::mem::take(&mut self.prop_replace);
                    self.obj_nested = true;
                    self.nested_def = None;
                    self.parse_object_body(Some(obj), false, true, None, None)?;
                    self.prop_replace = replace;
                    // It was the next definition after all, and the name
                    // perhaps that definition's.
                    if self.unterm {
                        return Ok(());
                    }
                    let Some(nested) = self.nested_def.take() else {
                        return Err(Stop);
                    };
                    let prop = self.look_up_prop(&name);
                    if prop.is_some_and(|prop| self.prog.props[prop].vocab) {
                        self.error("vocabulary property requires string value, but found \":\"");
                    }
                    defined_prop = prop;
                    if let Some(prop) = prop {
                        let id = self.prog.objs[obj].id;
                        new_prop = Some(def.props.len());
                        def.props.push(ObjProp {
                            prop,
                            value: PropValue::Expr(Expr::constant(Value::Obj(id))),
                            overwritable: false,
                            is_static: false,
                            file,
                            line,
                        });
                    }
                    // The object knows which object it is written in.
                    let parent = def.sym.map(|obj| self.prog.objs[obj].id).or(def.mod_id);
                    self.prog.objdefs[nested].props.push(ObjProp {
                        prop: LEXICAL_PARENT_PROP,
                        value: PropValue::Expr(Expr::constant(Value::Obj(parent.unwrap_or(0)))),
                        overwritable: false,
                        is_static: false,
                        file,
                        line,
                    });
                    self.prog.nested.push(Top::Obj(nested));
                    no_value = true;
                }
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
        if !no_value {
            defined_prop = prop;
        }
        let is_vocab = prop.is_some_and(|prop| self.prog.props[prop].vocab);
        if no_value {
            // Nothing to read; but the name is still checked below.
        } else if let (true, true, Tok::SStr, Some(prop)) = (is_value, is_vocab, self.cur(), prop) {
            // Words for a dictionary: one string after another. What
            // the property's value is waits until the words the
            // object's classes have are known.
            if self.cur_dict.is_none() {
                self.error("vocabulary property cannot be defined - no dictionary is active");
            }
            loop {
                if def.sym.is_some() {
                    def.vocab.push((self.cur_text(), prop));
                }
                if self.next() != Tok::SStr {
                    break;
                }
            }
            new_prop = Some(def.props.len());
            def.props.push(ObjProp {
                prop,
                value: PropValue::VocabList,
                overwritable: false,
                is_static: false,
                file,
                line,
            });
        } else if let (true, true) = (is_value, self.inline_values.is_some()) {
            // In an object written in an expression: a value to work
            // out each time the property is asked for, or with
            // `static`, once when the object is made.
            if is_vocab {
                self.error_curtok("vocabulary property requires string value, but found \"%\"");
            }
            let value = if self.cur() == Tok::Static {
                self.next();
                let Some(expr) = self.parse_expr() else {
                    return Err(Stop);
                };
                InlineValue::Eval(expr)
            } else {
                self.parse_inline_prop_value(file, line)?
            };
            if let Some(prop) = prop {
                new_prop = Some(def.props.len());
                self.push_inline_prop(def, prop, value, file, line);
            }
        } else if is_value {
            if is_vocab {
                self.error_curtok("vocabulary property requires string value, but found \"%\"");
            }
            let is_static = self.cur() == Tok::Static;
            if is_static {
                self.next();
            }
            let saved = self.begin_prop_value();
            // A value that cannot be read ends the reading of the whole
            // source.
            let Some(expr) = self.parse_expr() else {
                return Err(Stop);
            };
            let static_prop = if is_static { prop } else { None };
            let (value, has_local_ctx) =
                self.finish_prop_value(expr, static_prop, file, line, saved);
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
            if is_vocab {
                self.error_curtok("vocabulary property requires string value, but found \"%\"");
            }
            if self.inline_values.is_some() {
                // In an object written in an expression, a method is
                // an anonymous method. The reference tells what kind
                // of body it is reading by whether the word before it
                // was `method`, which here it is not: so the body is
                // read as an anonymous function's, and one that uses
                // `self` asks for the context of the code around it,
                // though as a method it has a `self` of its own.
                if let (Some(method), Some(prop)) =
                    (self.parse_anon_func_at(CbType::AnonFn, true), prop)
                {
                    new_prop = Some(def.props.len());
                    self.push_inline_prop(def, prop, InlineValue::Method(method), file, line);
                }
                return self.finish_obj_prop(
                    def,
                    existing,
                    new_prop,
                    defined_prop,
                    &name,
                    obj_is_nested,
                );
            }
            if self.propset_depth != 0 {
                self.insert_propset_params();
            }
            self.body_is_obj_prop = true;
            let Some((body, _)) = self.parse_code_body(true, CbType::Normal, None, None)? else {
                return Ok(());
            };
            self.prog.last_labels = std::rc::Rc::clone(&body.labels);
            self.self_valid = true;
            if op_args != 0 && (body.varargs || body.argc != op_args - 1) {
                self.error(format!(
                    "wrong number of parameters defined for overloaded operator ({} required)",
                    op_args - 1
                ));
            }
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

        self.finish_obj_prop(def, existing, new_prop, defined_prop, &name, obj_is_nested)
    }

    /// Give an object written in an expression a property.
    fn push_inline_prop(
        &mut self,
        def: &mut ObjDef,
        prop: usize,
        value: InlineValue,
        file: usize,
        line: u32,
    ) {
        def.props.push(ObjProp {
            prop,
            value: PropValue::Expr(Expr::constant(Value::Nil)),
            overwritable: false,
            is_static: false,
            file,
            line,
        });
        if let Some(values) = &mut self.inline_values {
            values.push(value);
        }
    }

    /// What is checked of a property once it has been read: that the
    /// object can have it, and has it only once.
    fn finish_obj_prop(
        &mut self,
        def: &mut ObjDef,
        existing: Option<Global>,
        new_prop: Option<usize>,
        defined_prop: Option<usize>,
        name: &[u8],
        obj_is_nested: bool,
    ) -> Parsed<()> {
        // What a built-in class does itself, or a class it is a kind
        // of does, the program cannot do in its place.
        if let (Some(meta), Some(new_prop)) = (def.meta, new_prop) {
            let prop = def.props[new_prop].prop;
            let mut cur = Some(meta);
            while let Some(index) = cur {
                let meta = &self.prog.metas[index];
                let (native, super_meta) = (meta.props.contains(&prop), meta.super_meta);
                if native {
                    self.error(format!(
                        "intrinsic class property \"{}\" cannot be modified",
                        Self::name(name)
                    ));
                }
                cur = super_meta;
            }
        }

        // With `replace`, the property goes from every definition this
        // one modifies. The first reading leaves them be.
        if let (true, Some(new_prop), false) = (self.prop_replace, new_prop, self.syntax_only) {
            let prop = def.props[new_prop].prop;
            let mut prev = def.prev_def;
            while let Some(index) = prev {
                let base = &mut self.prog.objdefs[index];
                if let Some(at) = base.props.iter().position(|other| other.prop == prop) {
                    base.props.remove(at);
                }
                base.vocab.retain(|(_, word_prop)| *word_prop != prop);
                prev = base.prev_def;
            }
        }

        // A property the name already was can be here only once, but
        // that the source's value replaces one the compiler put in. An
        // object that is a property's value cannot say what it is
        // written in: the compiler does.
        if obj_is_nested && existing == Some(Global::Prop(LEXICAL_PARENT_PROP)) {
            self.error(format!(
                "property \"{}\" already defined in object",
                Self::name(name)
            ));
        } else if let Some(Global::Prop(prop)) = existing {
            let earlier = def
                .props
                .iter()
                .enumerate()
                .position(|(index, other)| Some(index) != new_prop && other.prop == prop);
            if let Some(earlier) = earlier {
                if def.props[earlier].overwritable {
                    def.props.remove(earlier);
                    def.vocab.retain(|(_, word_prop)| *word_prop != prop);
                } else {
                    self.error(format!(
                        "property \"{}\" already defined in object",
                        Self::name(name)
                    ));
                }
            }
        }
        if let Some(prop) = defined_prop {
            if self.prog.props[prop].vocab {
                self.dict_defined.insert(prop);
            }
        }
        Ok(())
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
        let ending = ONE_OF_ENDINGS
            .iter()
            .position(|ending| self.embed_match(ending));
        let attrs = match ending {
            Some(ending) => ONE_OF_ATTRS[ending],
            None => {
                self.embed_unget(0);
                "rand"
            }
        };
        Some(self.oneof_node(dbl, elements, attrs))
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
        Some(self.oneof_node(dbl, vec![element, nothing], "seq,stop"))
    }

    /// A choice among alternatives is made by an object the compiler
    /// defines for it: one with no name, of the class `OneOfIndexGen`,
    /// which the program has to have, that is told how many
    /// alternatives there are and how to choose. The first reading
    /// defines nothing.
    fn oneof_node(&mut self, dbl: bool, elements: Vec<Expr>, attrs: &str) -> Expr {
        const CLASS: &[u8] = b"OneOfIndexGen";
        if self.syntax_only {
            return Expr::OneOf(dbl, elements, None);
        }
        if !matches!(self.prog.globals.get(CLASS), Some(Global::Obj(_))) {
            self.error("<<one of>> requires class OneOfIndexGen to be defined in the program");
            return Expr::OneOf(dbl, elements, None);
        }
        let obj = self.new_anon_obj();
        let (file, line) = self.tok.last_pos();
        let mut def = ObjDef {
            sym: Some(obj),
            is_class: false,
            superclasses: vec![CLASS.to_vec()],
            props: Vec::new(),
            file,
            line,
            is_modify: false,
            mod_base: None,
            prev_def: None,
            mod_id: None,
            replaced: false,
            meta: None,
            transient: false,
            bad_template: false,
            undesc_sc: false,
            dict: None,
            vocab: Vec::new(),
        };
        let values = [
            (&b"numItems"[..], Value::Int(elements.len() as i32)),
            (&b"listAttrs"[..], Value::SStr(attrs.as_bytes().to_vec())),
        ];
        for (name, value) in values {
            // A name that is something else is passed over.
            let prop = match self.prog.globals.get(name).copied() {
                Some(Global::Prop(prop)) => {
                    self.prog.props[prop].weak = false;
                    prop
                }
                None => self.prog.def_prop(name),
                Some(_) => continue,
            };
            def.props.push(ObjProp {
                prop,
                value: PropValue::Expr(Expr::constant(value)),
                overwritable: false,
                is_static: false,
                file,
                line,
            });
        }
        // It is generated after everything at the outermost level.
        self.prog.objs[obj].def = Some(self.prog.objdefs.len());
        self.prog.nested.push(Top::Obj(self.prog.objdefs.len()));
        self.prog.objdefs.push(def);
        Expr::OneOf(dbl, elements, Some(self.prog.objs[obj].id))
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

/// The object a definition is of.
enum FoundObj {
    /// An object of the program's, if the name could be one's; and if
    /// the definition is a `modify`, the number of the definition it
    /// goes in front of, if that has one, and its index in
    /// [`Program::objdefs`].
    Obj(Option<usize>, Option<(u32, Option<usize>)>),
    /// What a `modify` adds to a built-in class: the number of the
    /// object it goes in front of, the class as an index into
    /// [`Program::metas`], and the new object's number.
    Meta(u32, usize, u32),
}

/// Whether a definition is of something new, or of something there is
/// already.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Redef {
    No,
    /// `replace`: the new definition takes the old one's place.
    Replace,
    /// `modify`: the new definition goes in front of the old one.
    Modify,
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
