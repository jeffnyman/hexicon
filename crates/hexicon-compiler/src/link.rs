//! Putting a program together from its source files.
//!
//! The reference compiles each source file to an object file, in which
//! objects, properties, and the values of enumerations have numbers of
//! the file's own, and then reads the object files one after another
//! into one program, giving everything the number it has in the image
//! as it comes to it. What a file's names get depends on the order
//! they are in its object file, which is the order of the table of
//! global names the compiler had when it finished the file; and a name
//! the file knew of from another file is in its object file only if
//! the file used it.
//!
//! Hexicon writes no object files. What it has for each source file is
//! what the parser and the code generator made of it; this goes
//! through each as the reference goes through the object file it would
//! have written, and comes to the same numbers.

use std::collections::{HashMap, HashSet};

use crate::emit::{Anchor, Fixup, GenObj, Generated, IdKind, Target};
use crate::parse::{
    FuncSym, Global, GramAlt, GramProd, GramTok, GramTokKind, MetaEntry, MetaSym, ObjDef, ObjSym,
    PREDEFINED_METACLASSES, Program, SPECIAL_PROPS,
};
use crate::pp::show_name;

/// The kinds of value a property of an object can have that are
/// numbers to be changed: an object, a property, a value of an
/// enumeration.
const KIND_OBJ: u8 = 5;
const KIND_PROP: u8 = 6;
const KIND_ENUM: u8 = 15;

/// One source file, compiled.
pub(crate) struct Module {
    pub prog: Program,
    pub out: Generated,
    /// The name the reference would give its object file, for
    /// messages.
    pub object_file: String,
}

/// A program put together.
pub(crate) struct Linked {
    pub prog: Program,
    pub out: Generated,
    /// The number each value of an enumeration has in the image, by
    /// its index in the program's.
    pub enum_ids: Vec<u32>,
    /// What was found wrong, in the order the reference finds it.
    pub errors: Vec<String>,
}

struct Linker {
    prog: Program,
    out: Generated,
    enum_ids: Vec<u32>,
    errors: Vec<String>,
    /// How many code bodies the files read so far had between them:
    /// each file's are numbered after those of the files before it.
    body_base: usize,
    /// For each function that `modify` has put a definition in front
    /// of, the definitions behind the one it has now: indexes into the
    /// code's anchors. A `replace` takes them all away.
    mod_base_anchors: HashMap<usize, Vec<usize>>,
}

/// What kind of thing a name is, as the reference says it in a
/// message.
fn kind_name(global: &Global) -> &'static str {
    match global {
        Global::Func(_) => "function",
        Global::Bif(_) => "intrinsic function",
        Global::Obj(_) => "object",
        Global::Prop(_) => "property",
        Global::Meta(_) => "intrinsic class",
        Global::Enum(_) => "enum",
        Global::DynLocal(_) => "local variable",
    }
}

/// Put the source files of a program together, in order.
pub(crate) fn link(modules: Vec<Module>) -> Linked {
    let mut prog = Program {
        next_obj_id: 1,
        next_prop_id: 1,
        ..Program::default()
    };
    for name in SPECIAL_PROPS {
        prog.def_prop(name.as_bytes());
    }
    for name in PREDEFINED_METACLASSES {
        prog.meta_table.push(MetaEntry {
            name: name.as_bytes().to_vec(),
            sym: None,
        });
    }
    let mut linker = Linker {
        prog,
        out: Generated::default(),
        enum_ids: Vec::new(),
        errors: Vec::new(),
        body_base: 0,
        mod_base_anchors: HashMap::new(),
    };
    for module in modules {
        linker.load(module);
    }
    drop_dead_code(&mut linker.out);
    Linked {
        prog: linker.prog,
        out: linker.out,
        enum_ids: linker.enum_ids,
        errors: linker.errors,
    }
}

/// Take out of the code the functions that a `replace` in a later
/// source file took the place of. The reference does not write them to
/// the image.
fn drop_dead_code(out: &mut Generated) {
    if out.dead_code.is_empty() {
        return;
    }
    let dead: HashSet<usize> = out.dead_code.drain(..).collect();
    let count = out.code_anchors.len();
    // Each item: where it was, and where it is now if it is kept.
    let mut items: Vec<(usize, Option<usize>)> = Vec::with_capacity(count);
    let mut anchor_map: Vec<Option<usize>> = vec![None; count];
    let mut code = Vec::with_capacity(out.code.len());
    let mut anchors = Vec::new();
    for index in 0..count {
        let start = out.code_anchors[index].ofs;
        let end = out
            .code_anchors
            .get(index + 1)
            .map_or(out.code.len(), |next| next.ofs);
        if dead.contains(&index) {
            items.push((start, None));
            out.dropped_code
                .push((anchors.len(), out.code[start..end].to_vec()));
            continue;
        }
        anchor_map[index] = Some(anchors.len());
        items.push((start, Some(code.len())));
        anchors.push(Anchor { ofs: code.len() });
        code.extend_from_slice(&out.code[start..end]);
    }
    let moved = |ofs: usize| -> Option<usize> {
        let index = items
            .partition_point(|(start, _)| *start <= ofs)
            .checked_sub(1)?;
        let (start, new) = items[index];
        new.map(|new| new + ofs - start)
    };
    out.code = code;
    out.code_anchors = anchors;
    out.fixups = std::mem::take(&mut out.fixups)
        .into_iter()
        .filter_map(|fixup| {
            moved(fixup.ofs).map(|ofs| Fixup {
                ofs,
                target: fixup.target,
            })
        })
        .collect();
    out.unspecified = out
        .unspecified
        .iter()
        .filter_map(|ofs| moved(*ofs))
        .collect();
    // The names of the locals of what is gone keep their places among
    // the names: see `finish_local_names`.
    out.local_names = std::mem::take(&mut out.local_names)
        .into_iter()
        .map(|(ofs, name)| (moved(ofs).unwrap_or(usize::MAX), name))
        .collect();
    out.func_anchor = std::mem::take(&mut out.func_anchor)
        .into_iter()
        .filter_map(|(func, anchor)| anchor_map[anchor].map(|anchor| (func, anchor)))
        .collect();
    out.body_anchor = std::mem::take(&mut out.body_anchor)
        .into_iter()
        .filter_map(|(body, anchor)| anchor_map[anchor].map(|anchor| (body, anchor)))
        .collect();
    let renumber = |target: &mut Target| {
        if let Target::Anchor(anchor) = target {
            *anchor = anchor_map[*anchor].unwrap_or(0);
        }
    };
    for fixup in out
        .fixups
        .iter_mut()
        .chain(&mut out.static_fixups)
        .chain(&mut out.const_fixups)
    {
        renumber(&mut fixup.target);
    }
    for obj in &mut out.objs {
        for prop in &mut obj.props {
            if let Some(target) = &mut prop.target {
                renumber(target);
            }
        }
    }
}

/// What the names of one source file are in the whole program.
#[derive(Default)]
struct Maps {
    /// For each function the file modifies that another file defined,
    /// the definition there was: an index into the code's anchors.
    func_bases: HashMap<usize, usize>,
    /// The objects the file modifies that another file defined, each
    /// with the definition there was, as an index into the program's
    /// definitions; and those it replaces, likewise.
    ext_mods: Vec<(usize, usize)>,
    replaced_objs: Vec<(usize, usize)>,
    funcs: Vec<Option<usize>>,
    objs: Vec<Option<usize>>,
    props: Vec<Option<usize>>,
    enums: Vec<Option<usize>>,
    metas: Vec<Option<usize>>,
    /// The number in the image of each object the file has a number
    /// for.
    obj_ids: HashMap<u32, u32>,
    /// The functions the file defines, as against those it only
    /// refers to: indexes into the file's.
    defined_funcs: HashSet<usize>,
    /// The objects it defines.
    defined_objs: HashSet<usize>,
    /// For each built-in class the file adds to, the objects that hold
    /// what it adds, by the file's numbers for them: an empty one, and
    /// then one for each `modify`.
    meta_chains: HashMap<usize, Vec<u32>>,
}

impl Linker {
    fn error(&mut self, message: String) {
        self.errors.push(message);
    }

    fn conflict(&mut self, name: &[u8], old: &Global, new: &Global, object_file: &str) {
        let (old, new) = (kind_name(old), kind_name(new));
        self.error(if old == new {
            format!(
                "symbol \"{}\" (type: {old}) redefined in object file \"{object_file}\"",
                show_name(name)
            )
        } else {
            format!(
                "symbol \"{}\" (type: {old}) redefined (type: {new}) in object file \
                 \"{object_file}\"",
                show_name(name)
            )
        });
    }

    fn new_obj_id(&mut self) -> u32 {
        self.prog.next_obj_id += 1;
        self.prog.next_obj_id - 1
    }

    /// A number for an object the compiler made in front of another,
    /// which goes in the table of global names under its number.
    fn new_synthetic_id(&mut self) -> u32 {
        let id = self.new_obj_id();
        self.prog
            .add_synthetic_obj(format!(" {id:x}").into_bytes(), id);
        id
    }

    /// The property a name is in the whole program, which it becomes
    /// if it is nothing yet. Nothing comes back if it is something
    /// else.
    fn find_or_def_prop(&mut self, name: &[u8]) -> Option<usize> {
        match self.prog.globals.peek(name).copied() {
            Some(Global::Prop(prop)) => Some(prop),
            Some(_) => None,
            None => Some(self.prog.def_prop(name)),
        }
    }

    fn load(&mut self, module: Module) {
        let Module {
            prog: m,
            out: mut g,
            object_file,
        } = module;
        let object_file = object_file.as_str();
        let errors_before = self.errors.len();

        // The sets of built-in functions and the built-in classes the
        // file depends on, which every file lists in the same order.
        for (index, name) in m.fnsets.iter().enumerate() {
            let base = |name: &[u8]| {
                let len = name.iter().position(|&c| c == b'/').unwrap_or(name.len());
                name[..len].to_vec()
            };
            match self.prog.fnsets.get_mut(index) {
                Some(have) => {
                    if base(have) != base(name) {
                        self.errors.push(format!(
                            "conflicting intrinsic function set \"{}\" found in object file \
                             \"{object_file}\"",
                            show_name(name)
                        ));
                    } else if name.len() <= have.len() && *name > *have {
                        have.clone_from(name);
                    }
                }
                None => self.prog.fnsets.push(name.clone()),
            }
        }
        for entry in &m.meta_table {
            self.prog.find_or_add_meta(&entry.name, None);
        }

        // Which of the file's functions something in it refers to.
        let mut func_refs: HashSet<usize> = HashSet::new();
        for fixup in g
            .fixups
            .iter()
            .chain(&g.static_fixups)
            .chain(&g.const_fixups)
        {
            if let Target::Func(func) = fixup.target {
                func_refs.insert(func);
            }
        }
        for obj in &g.objs {
            for prop in &obj.props {
                if let Some(Target::Func(func)) = prop.target {
                    func_refs.insert(func);
                }
            }
        }

        let mut maps = Maps {
            funcs: vec![None; m.funcs.len()],
            objs: vec![None; m.objs.len()],
            props: vec![None; m.props.len()],
            enums: vec![None; m.enums.len()],
            metas: vec![None; m.metas.len()],
            ..Maps::default()
        };
        for (index, slot) in maps.props.iter_mut().enumerate().take(SPECIAL_PROPS.len()) {
            *slot = Some(index);
        }

        // The names, in the order of the file's table.
        let names: Vec<(Vec<u8>, Global)> = m
            .globals
            .in_table_order()
            .into_iter()
            .map(|(name, global)| (name.to_vec(), global))
            .collect();
        for (name, global) in &names {
            match *global {
                Global::Func(func) => {
                    self.load_func(&m, func, func_refs.contains(&func), &mut maps, object_file);
                }
                Global::Obj(obj) => {
                    let sym = &m.objs[obj];
                    if sym.is_extern && !m.globals.is_used(name) {
                        continue;
                    }
                    self.load_obj(&m, obj, &mut maps, object_file);
                }
                Global::Prop(prop) => {
                    if prop < SPECIAL_PROPS.len() || !m.globals.is_used(name) {
                        continue;
                    }
                    match self.prog.globals.peek(name).copied() {
                        None | Some(Global::Prop(_)) => {
                            maps.props[prop] = self.find_or_def_prop(name);
                        }
                        Some(old) => self.conflict(name, &old, global, object_file),
                    }
                }
                Global::DynLocal(_) => {}
                Global::Enum(sym) => {
                    if !m.globals.is_used(name) {
                        continue;
                    }
                    match self.prog.globals.peek(name).copied() {
                        Some(Global::Enum(found)) => {
                            self.prog.enums[found].is_token |= m.enums[sym].is_token;
                            maps.enums[sym] = Some(found);
                        }
                        Some(old) => self.conflict(name, &old, global, object_file),
                        None => {
                            let index = self.prog.enums.len();
                            self.prog.enums.push(m.enums[sym].clone());
                            self.enum_ids.push(index as u32 + 1);
                            self.prog
                                .globals
                                .insert_known(name.clone(), Global::Enum(index));
                            maps.enums[sym] = Some(index);
                        }
                    }
                }
                Global::Meta(meta) => {
                    if m.metas[meta].is_ext && !m.globals.is_used(name) {
                        continue;
                    }
                    self.load_meta(&m, meta, &mut maps, object_file);
                }
                Global::Bif(bif) => match self.prog.globals.peek(name).copied() {
                    None => {
                        let index = self.prog.bifs.len();
                        self.prog.bifs.push(m.bifs[bif].clone());
                        self.prog
                            .globals
                            .insert_known(name.clone(), Global::Bif(index));
                    }
                    Some(Global::Bif(_)) => {}
                    Some(old) => self.conflict(name, &old, global, object_file),
                },
            }
        }

        // The objects with no names, in the order they were made, and
        // then the objects that constants were made into.
        for (index, sym) in m.objs.iter().enumerate() {
            if sym.name == b".anon" && maps.objs[index].is_none() {
                let id = self.new_obj_id();
                maps.obj_ids.insert(sym.id, id);
                maps.objs[index] = Some(self.prog.objs.len());
                maps.defined_objs.insert(index);
                self.prog.objs.push(ObjSym {
                    id,
                    def: None,
                    gram: None,
                    ..sym.clone()
                });
            }
        }
        let mut nonsym: Vec<u32> = g
            .bignums
            .iter()
            .map(|(id, _)| *id)
            .chain(g.rexpats.iter().map(|(id, _)| *id))
            .collect();
        nonsym.sort_unstable();
        for local in nonsym {
            let id = self.new_obj_id();
            maps.obj_ids.insert(local, id);
        }

        // The file's code bodies are numbered after those of the files
        // before it; the generator makes some of its own, with numbers
        // past the parser's.
        let bodies = g
            .body_anchor
            .keys()
            .chain(g.static_body_anchor.keys())
            .map(|body| body + 1)
            .max()
            .unwrap_or(0)
            .max(m.bodies.len());
        let clean = self.errors.len() == errors_before;
        self.merge(&m, &mut g, &maps, object_file, clean);
        self.body_base += bodies;
    }

    fn load_func(
        &mut self,
        m: &Program,
        func: usize,
        referenced: bool,
        maps: &mut Maps,
        object_file: &str,
    ) {
        let sym = &m.funcs[func];
        // A function that is only declared, and that nothing refers
        // to, is not in the object file. A multi-method itself is
        // never defined in a source file.
        let defined = !sym.is_extern;
        if !defined && !referenced {
            return;
        }
        let redef = sym.ext_redef || sym.mod_extern;
        let index = match self.prog.globals.peek(&sym.name).copied() {
            None => {
                let index = self.prog.funcs.len();
                self.prog.funcs.push(FuncSym {
                    body: None,
                    mod_bases: Vec::new(),
                    ext_redef: false,
                    mod_extern: false,
                    ..sym.clone()
                });
                self.prog
                    .globals
                    .insert_known(sym.name.clone(), Global::Func(index));
                if redef {
                    self.errors.push(format!(
                        "modified or replaced function \"{}\" found in object file \
                         \"{object_file}\" before original function definition was loaded",
                        show_name(&sym.name)
                    ));
                }
                index
            }
            Some(Global::Func(index)) => {
                let old = &self.prog.funcs[index];
                if !old.is_extern && defined && !redef {
                    self.conflict(
                        &sym.name,
                        &Global::Func(index),
                        &Global::Func(index),
                        object_file,
                    );
                    return;
                } else if old.argc != sym.argc
                    || old.varargs != sym.varargs
                    || old.has_retval != sym.has_retval
                {
                    self.errors.push(format!(
                        "function \"{}\" has different definition in object file \
                         \"{object_file}\"",
                        show_name(&sym.name)
                    ));
                } else if old.is_mm != sym.is_mm || old.is_mm_base != sym.is_mm_base {
                    self.errors.push(format!(
                        "function \"{}\" has conflicting definitions as multi-method and as \
                         regular function - object file {object_file}",
                        show_name(&sym.name)
                    ));
                }
                if defined && redef {
                    if old.is_extern {
                        self.errors.push(format!(
                            "modified or replaced function \"{}\" found in object file \
                             \"{object_file}\" before original function definition was loaded",
                            show_name(&sym.name)
                        ));
                    } else if sym.mod_extern {
                        // The definition there was stays in the
                        // program, as what `replaced` means in this
                        // file's.
                        if let Some(anchor) = self.out.func_anchor.get(&index).copied() {
                            maps.func_bases.insert(func, anchor);
                            self.mod_base_anchors.entry(index).or_default().push(anchor);
                        }
                    } else {
                        // The definition there was goes, with any that
                        // `modify` had put it in front of.
                        let mut dead = self.mod_base_anchors.remove(&index).unwrap_or_default();
                        dead.extend(self.out.func_anchor.get(&index).copied());
                        self.out.dead_code.extend(dead);
                    }
                }
                index
            }
            Some(old) => {
                self.conflict(&sym.name, &old, &Global::Func(0), object_file);
                return;
            }
        };
        if defined {
            self.prog.funcs[index].is_extern = false;
            maps.defined_funcs.insert(func);
        }
        maps.funcs[func] = Some(index);
    }

    /// Read an object's name. Its dictionary comes just before it, if
    /// the file has not had it yet, and the definitions that `modify`
    /// set aside get their numbers before it gets its own.
    fn load_obj(&mut self, m: &Program, obj: usize, maps: &mut Maps, object_file: &str) {
        if maps.objs[obj].is_some() {
            return;
        }
        let sym = &m.objs[obj];
        let defs: Vec<usize> = {
            // The object's definitions, the latest first.
            let mut defs = Vec::new();
            let mut next = sym.def;
            while let Some(def) = next {
                defs.push(def);
                next = m.objdefs[def].prev_def.filter(|_| m.objdefs[def].is_modify);
            }
            defs
        };
        if let Some(dict) = defs.first().and_then(|def| m.objdefs[*def].dict) {
            if maps.objs[dict].is_none() {
                self.load_obj(m, dict, maps, object_file);
            }
        }
        let defined = !sym.is_extern;
        let own = || {
            m.objdefs
                .iter()
                .filter(move |def| def.sym == Some(obj) && def.meta.is_none())
        };
        // The definitions set aside, the oldest first: each `modify`
        // sets aside the definition there was, and they stay objects
        // even if a `replace` comes after. If the first the file has
        // is itself a `modify`, of a definition in another file, that
        // one is set aside too, and comes first.
        if defined {
            if let Some(local) = own()
                .find(|def| def.is_modify && def.prev_def.is_none())
                .and_then(|def| def.mod_base)
            {
                let id = self.new_synthetic_id();
                maps.obj_ids.insert(local, id);
            }
            for local in own().filter_map(|def| def.mod_id) {
                let id = self.new_synthetic_id();
                maps.obj_ids.insert(local, id);
            }
        }
        let this = Global::Obj(0);
        let same_shared =
            |old: &ObjSym| (old.is_dict && sym.is_dict) || (old.is_gramprod && sym.is_gramprod);
        let index = match self.prog.globals.peek(&sym.name).copied() {
            None => {
                if defined && sym.ext_redef {
                    self.errors.push(format!(
                        "modified or replaced object \"{}\" found in object file \
                         \"{object_file}\" before original object definition was loaded",
                        show_name(&sym.name)
                    ));
                }
                let id = self.new_obj_id();
                let index = self.prog.objs.len();
                self.prog.objs.push(ObjSym {
                    id,
                    def: None,
                    gram: None,
                    ext_redef: false,
                    ..sym.clone()
                });
                self.prog
                    .globals
                    .insert_known(sym.name.clone(), Global::Obj(index));
                index
            }
            Some(Global::Obj(index)) => {
                let old = &self.prog.objs[index];
                if same_shared(old) {
                    // A dictionary or a grammar production can be
                    // defined in any number of files.
                } else if old.is_dict != sym.is_dict || old.is_gramprod != sym.is_gramprod {
                    self.conflict(&sym.name, &Global::Obj(index), &this, object_file);
                    return;
                } else if defined && sym.ext_redef {
                    if old.is_extern {
                        self.errors.push(format!(
                            "modified or replaced object \"{}\" found in object file \
                             \"{object_file}\" before original object definition was loaded",
                            show_name(&sym.name)
                        ));
                    } else if let Some(old_def) = old.def {
                        let gen_obj = self.out.def_objs.get(&old_def).copied();
                        let first = own().next();
                        let base = first
                            .filter(|def| def.is_modify && def.prev_def.is_none())
                            .and_then(|def| def.mod_base)
                            .and_then(|local| maps.obj_ids.get(&local).copied());
                        if let Some(base) = base {
                            // `modify`: the definition there was
                            // becomes an object of its own, a class,
                            // with the number set aside for it, and
                            // the object's number is the new
                            // definition's.
                            if let Some(gen_obj) = gen_obj {
                                self.out.objs[gen_obj].id = base;
                                self.out.objs[gen_obj].is_class = true;
                            }
                            self.prog.objdefs[old_def].mod_id = Some(base);
                            maps.ext_mods.push((obj, old_def));
                        } else {
                            // `replace`: the definition there was goes.
                            if let Some(gen_obj) = gen_obj {
                                self.out.dead_objs.insert(gen_obj);
                            }
                            self.prog.objdefs[old_def].replaced = true;
                            maps.replaced_objs.push((obj, old_def));
                        }
                    }
                } else if defined && !old.is_extern {
                    self.conflict(&sym.name, &Global::Obj(index), &this, object_file);
                    return;
                }
                index
            }
            Some(old) => {
                self.conflict(&sym.name, &old, &this, object_file);
                return;
            }
        };
        if defined {
            let old = &mut self.prog.objs[index];
            if old.is_extern || !(old.is_dict || old.is_gramprod) {
                maps.defined_objs.insert(obj);
            }
            old.is_extern = false;
            old.is_class |= sym.is_class;
        }
        maps.obj_ids.insert(sym.id, self.prog.objs[index].id);
        maps.objs[obj] = Some(index);
    }

    /// Read a built-in class's name: the object that stands for it,
    /// the properties it declares, and the objects that hold what the
    /// file adds to it.
    fn load_meta(&mut self, m: &Program, meta: usize, maps: &mut Maps, object_file: &str) {
        let sym = &m.metas[meta];
        let vm_name = &m.meta_table[sym.meta].name;
        let this = Global::Meta(0);
        let (index, was_defined) = match self.prog.globals.peek(&sym.name).copied() {
            None => {
                let obj_id = self.new_obj_id();
                let index = self.prog.metas.len();
                let table = self.prog.find_or_add_meta(vm_name, Some(index));
                self.prog.metas.push(MetaSym {
                    obj_id,
                    meta: table,
                    props: Vec::new(),
                    is_ext: false,
                    super_meta: None,
                    mod_count: 0,
                    mod_obj: None,
                    ..sym.clone()
                });
                self.prog
                    .globals
                    .insert_known(sym.name.clone(), Global::Meta(index));
                (index, false)
            }
            Some(Global::Meta(index)) => {
                let table = self.prog.find_or_add_meta(vm_name, Some(index));
                if self.prog.metas[index].meta != table {
                    self.errors.push(format!(
                        "intrinsic class \"{}\" in object file {object_file} out of order with \
                         previous definition",
                        show_name(&sym.name)
                    ));
                }
                (index, true)
            }
            Some(old) => {
                self.conflict(&sym.name, &old, &this, object_file);
                return;
            }
        };
        maps.obj_ids
            .insert(sym.obj_id, self.prog.metas[index].obj_id);
        maps.metas[meta] = Some(index);
        for (place, prop) in sym.props.iter().enumerate() {
            let name = &m.props[*prop].name;
            let have = self.prog.metas[index].props.get(place).copied();
            match have {
                Some(have) if was_defined => {
                    if self.prog.props[have].name != *name {
                        self.errors.push(format!(
                            "intrinsic class property \"{}\" does not match previous name \
                             (\"{}\") in object file {object_file}",
                            show_name(name),
                            show_name(&self.prog.props[have].name)
                        ));
                    }
                }
                _ => match self.find_or_def_prop(name) {
                    Some(found) => self.prog.metas[index].props.push(found),
                    None => {
                        let old = self.prog.globals.peek(name).copied();
                        if let Some(old) = old {
                            self.conflict(name, &old, &Global::Prop(0), object_file);
                        }
                    }
                },
            }
        }
        // The objects that hold what the file adds to the class: an
        // empty one, and one for each `modify`, each the superclass
        // of the next.
        let chain: Vec<u32> = m
            .objdefs
            .iter()
            .filter(|def| def.meta == Some(meta))
            .filter_map(|def| def.mod_id)
            .collect();
        for local in &chain {
            let id = self.new_synthetic_id();
            maps.obj_ids.insert(*local, id);
        }
        maps.meta_chains.insert(meta, chain);
    }

    /// Add what a file generated to the program, with everything
    /// given the numbers it has there.
    fn merge(
        &mut self,
        m: &Program,
        g: &mut Generated,
        maps: &Maps,
        object_file: &str,
        clean: bool,
    ) {
        let mut missing: Vec<String> = Vec::new();
        // The numbers the file used, and what they become.
        let prop_by_id: HashMap<u16, usize> = m
            .props
            .iter()
            .enumerate()
            .map(|(index, prop)| (prop.id, index))
            .collect();
        let enum_by_id: HashMap<u32, usize> = g
            .enum_ids
            .iter()
            .enumerate()
            .map(|(index, id)| (*id, index))
            .collect();
        let global_prop_ids: Vec<Option<u16>> = maps
            .props
            .iter()
            .map(|to| to.map(|index| self.prog.props[index].id))
            .collect();
        let global_enum_ids: Vec<Option<u32>> = maps
            .enums
            .iter()
            .map(|to| to.map(|index| self.enum_ids[index]))
            .collect();
        let mut prop_id = |local: u16| -> u16 {
            let found = prop_by_id
                .get(&local)
                .and_then(|index| global_prop_ids[*index]);
            found.unwrap_or_else(|| {
                let name = prop_by_id
                    .get(&local)
                    .map_or_else(String::new, |index| show_name(&m.props[*index].name));
                missing.push(format!("property \"{name}\" ({local})"));
                0
            })
        };
        let mut missing_objs: Vec<u32> = Vec::new();
        let mut obj_id = |local: u32| -> u32 {
            if local == 0 {
                return 0;
            }
            maps.obj_ids.get(&local).copied().unwrap_or_else(|| {
                missing_objs.push(local);
                0
            })
        };
        let mut missing_enums: Vec<u32> = Vec::new();
        let mut enum_id = |local: u32| -> u32 {
            let found = enum_by_id
                .get(&local)
                .and_then(|index| global_enum_ids[*index]);
            found.unwrap_or_else(|| {
                missing_enums.push(local);
                0
            })
        };

        // The numbers in the code and the constants.
        for (stream, fixups) in [
            (&mut g.code, &g.id_fixups),
            (&mut g.static_code, &g.static_id_fixups),
            (&mut g.consts, &g.const_id_fixups),
        ] {
            for &(ofs, kind) in fixups {
                match kind {
                    IdKind::Prop => {
                        let local = u16::from_le_bytes([stream[ofs], stream[ofs + 1]]);
                        stream[ofs..ofs + 2].copy_from_slice(&prop_id(local).to_le_bytes());
                    }
                    IdKind::Obj | IdKind::Enum => {
                        let bytes = [
                            stream[ofs],
                            stream[ofs + 1],
                            stream[ofs + 2],
                            stream[ofs + 3],
                        ];
                        let local = u32::from_le_bytes(bytes);
                        let id = if kind == IdKind::Obj {
                            obj_id(local)
                        } else {
                            enum_id(local)
                        };
                        stream[ofs..ofs + 4].copy_from_slice(&id.to_le_bytes());
                    }
                }
            }
        }

        // Where the file's code and constants go, and what its
        // functions and code bodies are in the program.
        let code_base = self.out.code.len();
        let static_base = self.out.static_code.len();
        let const_base = self.out.consts.len();
        let code_anchor_base = self.out.code_anchors.len();
        let static_anchor_base = self.out.static_anchors.len();
        let const_anchor_base = self.out.const_anchors.len();
        let body_base = self.body_base;
        let mut missing_funcs: Vec<usize> = Vec::new();
        let mut target = |target: &Target| -> Target {
            match target {
                Target::Func(func) => Target::Func(maps.funcs[*func].unwrap_or_else(|| {
                    missing_funcs.push(*func);
                    0
                })),
                Target::Const(anchor) => Target::Const(anchor + const_anchor_base),
                Target::Body(body) => Target::Body(body + body_base),
                Target::StaticBody(body) => Target::StaticBody(body + body_base),
                Target::FuncBase(func) => {
                    Target::Anchor(maps.func_bases.get(func).copied().unwrap_or(0))
                }
                Target::Anchor(anchor) => Target::Anchor(anchor + code_anchor_base),
            }
        };

        self.out.code.append(&mut g.code);
        self.out.static_code.append(&mut g.static_code);
        self.out.consts.append(&mut g.consts);
        let moved = |anchors: &[Anchor], base: usize| -> Vec<Anchor> {
            anchors
                .iter()
                .map(|anchor| Anchor {
                    ofs: anchor.ofs + base,
                })
                .collect()
        };
        self.out
            .code_anchors
            .extend(moved(&g.code_anchors, code_base));
        self.out
            .static_anchors
            .extend(moved(&g.static_anchors, static_base));
        self.out
            .const_anchors
            .extend(moved(&g.const_anchors, const_base));
        for (func, anchor) in &g.func_anchor {
            if !maps.defined_funcs.contains(func) {
                continue;
            }
            if let Some(index) = maps.funcs[*func] {
                self.out
                    .func_anchor
                    .insert(index, anchor + code_anchor_base);
            }
        }
        // The definitions a `modify` in the file itself set aside go
        // with the function too, if a later file replaces it.
        for func in &maps.defined_funcs {
            let Some(index) = maps.funcs[*func] else {
                continue;
            };
            for body in &m.funcs[*func].mod_bases {
                if let Some(anchor) = g.body_anchor.get(body) {
                    self.mod_base_anchors
                        .entry(index)
                        .or_default()
                        .push(anchor + code_anchor_base);
                }
            }
        }
        for (body, anchor) in &g.body_anchor {
            self.out
                .body_anchor
                .insert(body + body_base, anchor + code_anchor_base);
        }
        for (body, anchor) in &g.static_body_anchor {
            self.out
                .static_body_anchor
                .insert(body + body_base, anchor + static_anchor_base);
        }
        for (from, to, base) in [
            (&g.fixups, &mut self.out.fixups, code_base),
            (&g.static_fixups, &mut self.out.static_fixups, static_base),
            (&g.const_fixups, &mut self.out.const_fixups, const_base),
        ] {
            for fixup in from {
                to.push(Fixup {
                    ofs: fixup.ofs + base,
                    target: target(&fixup.target),
                });
            }
        }
        self.out
            .unspecified
            .extend(g.unspecified.iter().map(|ofs| ofs + code_base));
        self.out
            .static_unspecified
            .extend(g.static_unspecified.iter().map(|ofs| ofs + static_base));
        self.out
            .const_unspecified
            .extend(g.const_unspecified.iter().map(|ofs| ofs + const_base));
        self.out
            .const_unspecified2
            .extend(g.const_unspecified2.iter().map(|ofs| ofs + const_base));
        self.out.local_names.extend(
            g.local_names
                .drain(..)
                .map(|(ofs, name)| (ofs + code_base, name)),
        );
        self.out.max_bytecode_len = self.out.max_bytecode_len.max(g.max_bytecode_len);
        self.out.max_str_len = self.out.max_str_len.max(g.max_str_len);
        self.out.max_list_cnt = self.out.max_list_cnt.max(g.max_list_cnt);
        self.out.gave_up |= g.gave_up;

        // The objects.
        let objs_base = self.out.objs.len();
        for obj in g.objs.drain(..) {
            let mut obj: GenObj = obj;
            obj.id = obj_id(obj.id);
            for superclass in &mut obj.superclasses {
                *superclass = obj_id(*superclass);
            }
            for prop in &mut obj.props {
                prop.id = prop_id(prop.id);
                match prop.kind {
                    KIND_OBJ => prop.value = obj_id(prop.value),
                    KIND_PROP => prop.value = u32::from(prop_id(prop.value as u16)),
                    KIND_ENUM => prop.value = enum_id(prop.value),
                    _ => {}
                }
                if let Some(to) = &prop.target {
                    prop.target = Some(target(to));
                }
            }
            self.out.objs.push(obj);
        }
        for (obj, prop) in &g.static_inits {
            self.out.static_inits.push((obj_id(*obj), prop_id(*prop)));
        }
        for (id, data) in g.bignums.drain(..) {
            self.out.bignums.push((obj_id(id), data));
        }
        for (id, to) in g.rexpats.drain(..) {
            self.out.rexpats.push((obj_id(id), target(&to)));
        }

        // The definitions, which are what the dictionaries are made
        // from when everything has been read.
        let def_base = self.prog.objdefs.len();
        let obj_index = |obj: usize| maps.objs[obj];
        let prop_index = |prop: usize| maps.props[prop].unwrap_or(0);
        for def in &m.objdefs {
            // Nothing is generated for a definition that `replace` took
            // the place of, and whatever numbers it had are gone.
            let mut def_obj_id = |local: u32| {
                if def.replaced {
                    maps.obj_ids.get(&local).copied().unwrap_or(0)
                } else {
                    obj_id(local)
                }
            };
            self.prog.objdefs.push(ObjDef {
                sym: def.sym.and_then(obj_index),
                is_class: def.is_class,
                superclasses: def.superclasses.clone(),
                props: Vec::new(),
                file: def.file,
                line: def.line,
                is_modify: def.is_modify,
                mod_base: def.mod_base.map(&mut def_obj_id),
                prev_def: def.prev_def.map(|prev| prev + def_base),
                mod_id: def.mod_id.map(&mut def_obj_id),
                replaced: def.replaced,
                meta: def.meta.and_then(|meta| maps.metas[meta]),
                transient: def.transient,
                bad_template: def.bad_template,
                undesc_sc: def.undesc_sc,
                dict: def.dict.and_then(obj_index),
                vocab: def
                    .vocab
                    .iter()
                    .map(|(word, prop)| (word.clone(), prop_index(*prop)))
                    .collect(),
            });
        }
        for (def, obj) in &g.def_objs {
            self.out.def_objs.insert(def + def_base, obj + objs_base);
        }
        // A `modify` of an object another file defined: the file's
        // first definition of it goes in front of the one there was,
        // and the properties it has `replace` before go from that one
        // and from any it was in front of.
        for (obj, old_def) in &maps.ext_mods {
            let first = m
                .objdefs
                .iter()
                .position(|def| def.sym == Some(*obj) && def.meta.is_none());
            if let Some(first) = first {
                self.prog.objdefs[first + def_base].prev_def = Some(*old_def);
            }
            // A `modify` is transient if what it modifies is, as it is
            // when both are in one file.
            if self.prog.objdefs[*old_def].transient {
                for (index, def) in m.objdefs.iter().enumerate() {
                    if def.sym != Some(*obj) || def.meta.is_some() {
                        continue;
                    }
                    self.prog.objdefs[index + def_base].transient = true;
                    if let Some(&gen_obj) = self.out.def_objs.get(&(index + def_base)) {
                        self.out.objs[gen_obj].transient = true;
                    }
                }
            }
            for prop in &m.objs[*obj].del_props {
                let Some(prop) = maps.props[*prop] else {
                    continue;
                };
                let id = self.prog.props[prop].id;
                let mut next = Some(*old_def);
                while let Some(def) = next {
                    if let Some(gen_obj) = self.out.def_objs.get(&def) {
                        self.out.objs[*gen_obj].props.retain(|have| have.id != id);
                    }
                    let def = &mut self.prog.objdefs[def];
                    def.vocab.retain(|(_, word_prop)| *word_prop != prop);
                    next = def.prev_def.filter(|_| def.is_modify);
                }
            }
        }
        // A `replace` of an object another file defined: the words the
        // object had are still its words, after the new ones.
        for (obj, old_def) in &maps.replaced_objs {
            if let Some(new_def) = m.objs[*obj].def {
                let old = self.prog.objdefs[*old_def].vocab.clone();
                self.prog.objdefs[new_def + def_base].vocab.extend(old);
            }
        }

        // What the names the file defines are, now that everything in
        // it has its place.
        // The words in a row go in the dictionary that was current
        // when the row was read, but the reference loses track of
        // which that was unless some object of the file's has its
        // words in the same dictionary: only then is the dictionary
        // given a place in the object file that the row can refer to.
        // Any object defined while the dictionary was current will
        // do, whether it has words or not, except the classes with no
        // names that `grammar` statements with no tag define.
        let dicts_placed: HashSet<usize> = m
            .objdefs
            .iter()
            .filter(|def| !def.replaced && def.meta.is_none())
            .filter(|def| {
                def.sym
                    .is_some_and(|sym| !(m.objs[sym].name == b".anon" && def.is_class))
            })
            .filter_map(|def| def.dict)
            .collect();
        let alt = |alt: &GramAlt| GramAlt {
            obj: alt.obj.and_then(obj_index),
            dict: alt
                .dict
                .filter(|dict| dicts_placed.contains(dict))
                .and_then(obj_index),
            badness: alt.badness,
            toks: alt
                .toks
                .iter()
                .map(|tok| GramTok {
                    prop: tok.prop.and_then(|prop| maps.props[prop]),
                    kind: match &tok.kind {
                        GramTokKind::Literal(text) => GramTokKind::Literal(text.clone()),
                        GramTokKind::Speech(prop) => GramTokKind::Speech(prop_index(*prop)),
                        GramTokKind::SpeechList(props) => {
                            GramTokKind::SpeechList(props.iter().map(|p| prop_index(*p)).collect())
                        }
                        GramTokKind::Prod(obj) => GramTokKind::Prod(obj_index(*obj).unwrap_or(0)),
                        GramTokKind::TokType(sym) => {
                            GramTokKind::TokType(maps.enums[*sym].unwrap_or(0))
                        }
                        GramTokKind::Star => GramTokKind::Star,
                    },
                })
                .collect(),
        };
        // The productions, each with what the file's statements with
        // no tag say of it.
        let mut prod_map: Vec<Option<usize>> = Vec::new();
        for prod in &m.gramprods {
            let Some(sym) = obj_index(prod.sym) else {
                prod_map.push(None);
                continue;
            };
            let index = match self.prog.gramprods.iter().position(|have| have.sym == sym) {
                Some(index) => index,
                None => {
                    self.prog.gramprods.push(GramProd {
                        sym,
                        declared: false,
                        alts: Vec::new(),
                    });
                    self.prog.gramprods.len() - 1
                }
            };
            self.prog.gramprods[index].declared |= prod.declared;
            self.prog.gramprods[index]
                .alts
                .extend(prod.alts.iter().map(&alt));
            prod_map.push(Some(index));
        }
        for (index, sym) in m.objs.iter().enumerate() {
            if !maps.defined_objs.contains(&index) {
                continue;
            }
            let Some(to) = maps.objs[index] else {
                continue;
            };
            let merged = &mut self.prog.objs[to];
            merged.def = sym.def.map(|def| def + def_base);
            if sym.gram.is_none() {
                continue;
            }
            merged.gram = sym.gram.as_ref().map(|(prod, alts)| {
                (
                    prod.and_then(|prod| prod_map[prod]),
                    alts.iter().map(&alt).collect(),
                )
            });
            merged.sc_names.clone_from(&sym.sc_names);
        }
        for dict in &m.dicts {
            if let Some(dict) = obj_index(*dict) {
                if !self.prog.dicts.contains(&dict) {
                    self.prog.dicts.push(dict);
                }
            }
        }
        for prop in &m.dict_props {
            if let Some(prop) = maps.props[*prop] {
                self.prog.props[prop].vocab = true;
                if !self.prog.dict_props.contains(&prop) {
                    self.prog.dict_props.push(prop);
                }
            }
        }
        for (index, sym) in m.metas.iter().enumerate() {
            let Some(to) = maps.metas[index] else {
                continue;
            };
            let Some(chain) = maps.meta_chains.get(&index) else {
                continue;
            };
            let Some(local_top) = sym.mod_obj.or(chain.last().copied()) else {
                continue;
            };
            let top = obj_id(local_top);
            // If a file before this one added to the class, what this
            // one adds goes on top of that: the first of its objects
            // that has anything in it gets, for its superclass, the
            // last of the earlier file's.
            if let (Some(old_top), Some(first)) = (
                self.prog.metas[to].mod_obj,
                chain.get(1).map(|local| obj_id(*local)),
            ) {
                if let Some(obj) = self.out.objs[objs_base..]
                    .iter_mut()
                    .find(|obj| obj.id == first)
                {
                    obj.superclasses = vec![old_top];
                }
            }
            self.prog.metas[to].mod_obj = Some(top);
        }
        self.prog.exports.extend(m.exports.iter().cloned());
        self.prog.max_list_cnt = self.prog.max_list_cnt.max(m.max_list_cnt);

        // Anything the file used that it had no name for in its object
        // file is a mistake of the compiler's.
        drop(prop_id);
        drop(obj_id);
        drop(enum_id);
        drop(target);
        // (A name that could not be read, for being something else
        // already, leaves what the file says of it with no place.)
        if !clean {
            return;
        }
        for what in missing {
            self.errors.push(format!(
                "internal error: {what} is used in object file \"{object_file}\" but is not \
                 in it"
            ));
        }
        missing_objs.sort_unstable();
        missing_objs.dedup();
        for id in missing_objs {
            self.errors.push(format!(
                "internal error: object {id} is used in object file \"{object_file}\" but is \
                 not in it"
            ));
        }
        missing_enums.sort_unstable();
        missing_enums.dedup();
        for id in missing_enums {
            self.errors.push(format!(
                "internal error: enum {id} is used in object file \"{object_file}\" but is not \
                 in it"
            ));
        }
        missing_funcs.sort_unstable();
        missing_funcs.dedup();
        for func in missing_funcs {
            self.errors.push(format!(
                "internal error: function \"{}\" is used in object file \"{object_file}\" but \
                 is not in it",
                show_name(&m.funcs[func].name)
            ));
        }
    }
}
