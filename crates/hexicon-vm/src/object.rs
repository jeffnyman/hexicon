//! The object table and the kinds of object it holds.

use std::rc::Rc;

use crate::decimal::Big;
use crate::map::Map;
use crate::program_object::ProgramObject;
use crate::regex::Pattern;
use crate::value::{ObjId, Value};

/// A built-in class object: the object that represents one of the VM's
/// built-in classes to the program.
#[derive(Debug, Clone)]
pub struct BuiltinClass {
    /// Index of the class in the image's metaclass dependency list.
    pub meta_index: u16,
    /// The modifier object holding program-defined methods for the class,
    /// or the invalid ID.
    pub modifier: ObjId,
    /// Class-level state some built-in classes keep.
    #[allow(dead_code)]
    pub class_state: Value,
}

/// An object.
#[derive(Debug, Clone)]
pub enum Object {
    /// A program-defined object or class.
    Program(ProgramObject),
    /// The program's extensions to a built-in class. It has the same
    /// structure as a program-defined object.
    IntClassMod(ProgramObject),
    /// A string, as UTF-8 text.
    String(Rc<[u8]>),
    /// A list. Lists never change; operations that alter one make a new list.
    List(Rc<[Value]>),
    /// A vector: a list that can change in place.
    Vector(Vec<Value>),
    /// An anonymous function: the function to call followed by the values
    /// it captured.
    AnonFn(Vec<Value>),
    /// An iterator over a list or vector, by index.
    IndexedIter {
        /// The collection being iterated.
        collection: Value,
        /// Index of the element last returned; indexes start at 1.
        current: i32,
        /// Index of the first element.
        first: i32,
        /// Index of the last element.
        last: i32,
    },
    /// A lookup table.
    Map(Box<Map>),
    /// An iterator over a lookup table.
    MapIter {
        /// The table being iterated.
        table: ObjId,
        /// The slot of the entry last returned, counting from 1; zero
        /// before the first.
        slot: u32,
    },
    /// A character set: a name, and the mapping between text and bytes
    /// if the VM knows a set by that name.
    CharSet {
        name: String,
        map: Option<std::rc::Rc<crate::charset::CharMap>>,
    },
    /// A string comparator: rules for matching words.
    StrComp(std::rc::Rc<crate::strcomp::StrComp>),
    /// A grammar production: a rule of a parser's grammar.
    GrammarRule(Box<crate::gram::GrammarRule>),
    /// A dictionary: a parser's vocabulary.
    Dict(Box<crate::dict::Dictionary>),
    /// A byte array: bytes that can change in place.
    ByteArray(Vec<u8>),
    /// A date: a moment in time, as a day number and milliseconds into
    /// the day, in UTC.
    Date { dayno: i32, daytime: i32 },
    /// A time zone.
    TimeZone(Rc<crate::tz::Zone>),
    /// A file name.
    FileName(Box<crate::file::FileNameObj>),
    /// A file, open or closed.
    File(Box<crate::file::FileObj>),
    /// A temporary file, by name. The file is deleted when the object
    /// goes. An object loaded from an image has no name and no file.
    TempFile(String),
    /// A Decimal: a decimal floating-point number.
    Decimal(Box<Big>),
    /// A string buffer: text that can change in place.
    StringBuilder(StringBuilder),
    /// A function compiled while the program runs.
    CompiledCode(Box<crate::compiled_code::CompiledCode>),
    /// A call of a function, while it runs and after: what a
    /// `StackFrame` refers to.
    FrameRef(Box<crate::frameref::FrameRef>),
    /// A place in a call, as a stack trace gives it.
    FrameDesc(crate::frameref::FrameDesc),
    /// A compiled regular expression.
    Pattern {
        /// The pattern as the program wrote it: a string value.
        source: Value,
        /// The compiled pattern. A pattern from an image is compiled
        /// when it is first used.
        compiled: Option<Rc<Pattern>>,
    },
    /// A built-in class object.
    Class(BuiltinClass),
    /// An object of a built-in class Hexicon does not implement yet. Its
    /// image data is kept so the image still loads.
    Opaque {
        /// Index of the class in the image's metaclass dependency list.
        meta_index: u16,
        /// The object's data as the image stores it.
        #[allow(dead_code)]
        data: Rc<[u8]>,
    },
}

/// Number of table slots added at a time, as in the reference VM. The size
/// matters because it fixes the order in which free IDs are handed out.
const PAGE_SIZE: u32 = 4096;

#[derive(Debug, Clone)]
struct Slot {
    /// The object, or `None` when the slot is free.
    object: Option<Object>,
    /// Next slot on the free list (when free).
    next_free: u32,
    /// Previous slot on the free list (when free).
    prev_free: u32,
    /// The object is always reachable: it came from the image.
    in_root_set: bool,
    /// The object is excluded from saved state and undo.
    transient: bool,
    /// The object existed when the current savepoint was made, so a
    /// change to it has to be undoable.
    in_undo: bool,
    /// The object's state at the current savepoint is already kept.
    saved: bool,
    /// When the object was created, as a count of all objects created.
    serial: u32,
    /// Where the object is in the life of its finalizer.
    finalize: Finalize,
    /// What the collector found in its current pass.
    mark: Mark,
    /// The object's state as loaded from the image is already kept.
    pristine: bool,
    /// A remembered property lookup depends on the object: changing it
    /// forgets them all.
    lookup_dep: std::cell::Cell<bool>,
    /// For a vector, how many elements it has room for. The reference
    /// VM grows a vector by its own rules and writes the room it has
    /// to saved states and images.
    vec_alloc: u32,
    /// For a vector, the size of the reference's heap cell its elements
    /// are in. See [`crate::heap::realloc`].
    vec_cell: u16,
}

/// Whether an object's finalizer has run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Finalize {
    /// The object has never been found unreachable.
    NotYet,
    /// The object is unreachable and has a finalizer waiting to run.
    Pending,
    /// The finalizer has run, or there is none. The object can go as
    /// soon as nothing refers to it.
    Done,
}

/// How the collector reached an object.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mark {
    Unreachable,
    /// Reachable only from an object waiting to be finalized.
    FromFinalizable,
    Reachable,
}

/// The collector runs after this many objects have been created.
const ALLOCS_BETWEEN_GC: i32 = 10_000;

/// Remembered property lookups, by object and property. The keys are
/// small numbers, so they are their own hash.
type LookupCache = std::collections::HashMap<
    crate::value::PropId,
    std::collections::HashMap<u32, Option<(Value, ObjId)>, IdHash>,
    IdHash,
>;

/// A hasher for keys that are already well spread.
#[derive(Debug, Clone, Copy, Default)]
struct IdHash;

impl std::hash::BuildHasher for IdHash {
    type Hasher = IdHasher;
    fn build_hasher(&self) -> IdHasher {
        IdHasher(0)
    }
}

/// See [`IdHash`].
#[derive(Debug, Default)]
struct IdHasher(u64);

impl std::hash::Hasher for IdHasher {
    fn finish(&self) -> u64 {
        // Spread the bits, so that consecutive objects and properties
        // land in different buckets.
        self.0.wrapping_mul(0x9E37_79B9_7F4A_7C15)
    }
    fn write(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.0 = (self.0 << 8) | u64::from(b);
        }
    }
    fn write_u64(&mut self, n: u64) {
        self.0 = n;
    }
}

/// The most savepoints kept. Making another drops the oldest.
const MAX_SAVEPOINTS: usize = 255;

/// The object table: every object in the VM, indexed by ID.
///
/// Free IDs are kept on a list in the same order as the reference VM keeps
/// them, so a program that creates objects sees the same IDs under both.
#[derive(Debug, Clone, Default)]
pub struct ObjectTable {
    slots: Vec<Slot>,
    first_free: u32,
    /// For each savepoint, oldest first, the objects changed since it
    /// was made, each as it was at that moment.
    savepoints: std::collections::VecDeque<Vec<(u32, Object)>>,
    /// The objects the states in the savepoints refer to, each with how
    /// many references there are to it. They are roots for the
    /// collector; counting them as the states come and go spares it
    /// walking every state each time.
    saved_refs: std::collections::HashMap<u32, u32>,
    /// Property lookups through superclasses, remembered: by the object
    /// the lookup starts at and the property, what it found.
    lookups: std::cell::RefCell<LookupCache>,
    /// The objects the remembered lookups depend on.
    lookup_deps: std::cell::RefCell<Vec<u32>>,
    /// How many objects have ever been created.
    serial: u32,
    /// How many have been created since the collector last ran.
    allocs_since_gc: i32,
    /// The value of `serial` when the instruction now running at the
    /// innermost level began. See [`ObjectTable::collect_mark`].
    safe_serial: u32,
    /// What the builtins at outer levels may be holding while program
    /// code they called runs: for each such call, the level of the
    /// built-in, and the serials of the objects it had created since its
    /// instruction began (after the first, up to the second).
    held_ranges: Vec<(u32, u32, u32)>,
    /// The results such calls returned to a built-in, with its level.
    held_results: Vec<(u32, Value)>,
    /// Objects from the image that have changed since it was loaded, as
    /// they were then. Restoring and restarting begin from these.
    image_state: std::collections::HashMap<u32, Object>,
    /// Whether changes to image objects are being recorded yet.
    track_image_state: bool,
    /// File objects that have been freed, for the VM to finish with.
    released: Vec<Object>,
}

impl ObjectTable {
    /// An empty table.
    pub fn new() -> Self {
        Self::default()
    }

    /// The time zone objects that have not been looked up yet.
    pub fn pending_zones(&self) -> Vec<ObjId> {
        (0..self.slots.len() as u32)
            .filter(|&id| {
                matches!(
                    &self.slots[id as usize].object,
                    Some(Object::TimeZone(zone)) if zone.pending.is_some()
                )
            })
            .map(ObjId)
            .collect()
    }

    /// A property lookup from an object, remembered, or made with `look`
    /// and remembered.
    pub fn cached_lookup(
        &self,
        object: ObjId,
        prop: crate::value::PropId,
        look: impl FnOnce() -> Option<(Value, ObjId)>,
    ) -> Option<(Value, ObjId)> {
        if let Some(&found) = self
            .lookups
            .borrow()
            .get(&prop)
            .and_then(|by_object| by_object.get(&object.0))
        {
            return found;
        }
        let found = look();
        for dep in crate::program_object::lookup_deps(self, object) {
            if let Some(slot) = self.slots.get(dep.0 as usize)
                && !slot.lookup_dep.replace(true)
            {
                self.lookup_deps.borrow_mut().push(dep.0);
            }
        }
        self.lookups
            .borrow_mut()
            .entry(prop)
            .or_default()
            .insert(object.0, found);
        found
    }

    /// Forget the remembered lookups, if any depend on an object.
    fn changed(&self, id: u32) {
        if self
            .slots
            .get(id as usize)
            .is_some_and(|slot| slot.lookup_dep.get())
        {
            self.forget_lookups();
        }
    }

    /// Forget the remembered lookups of a property, if any depend on an
    /// object: setting the property on it changes nothing else.
    fn changed_prop(&self, id: u32, prop: crate::value::PropId) {
        if self
            .slots
            .get(id as usize)
            .is_some_and(|slot| slot.lookup_dep.get())
        {
            self.lookups.borrow_mut().remove(&prop);
        }
    }

    /// Forget every remembered lookup.
    fn forget_lookups(&self) {
        self.lookups.borrow_mut().clear();
        for id in self.lookup_deps.borrow_mut().drain(..) {
            if let Some(slot) = self.slots.get(id as usize) {
                slot.lookup_dep.set(false);
            }
        }
    }

    /// Put another object in an object's place, without it counting as
    /// a change to the object.
    pub fn replace(&mut self, id: ObjId, object: Object) {
        self.changed(id.0);
        if let Some(slot) = self.slots.get_mut(id.0 as usize) {
            if slot.object.is_some() {
                slot.object = Some(object);
            }
        }
    }

    /// The file objects freed since this was last asked.
    pub fn take_released(&mut self) -> Vec<Object> {
        std::mem::take(&mut self.released)
    }

    /// Hand over what every file object stands for, as if all of them
    /// had been freed: open files' contents and temporary files' names.
    pub fn release_all_files(&mut self) {
        for slot in &mut self.slots {
            match &mut slot.object {
                Some(Object::File(file)) => {
                    if file.open.is_some() {
                        let mut released = file.clone();
                        released.open = file.open.take();
                        self.released.push(Object::File(released));
                    }
                }
                Some(Object::TempFile(name)) => {
                    self.released.push(Object::TempFile(std::mem::take(name)));
                }
                _ => {}
            }
        }
    }

    fn add_page(&mut self) {
        let start = self.slots.len() as u32;
        for id in start..start + PAGE_SIZE {
            let mut slot = Slot {
                object: None,
                next_free: 0,
                prev_free: 0,
                in_root_set: false,
                transient: false,
                in_undo: false,
                saved: false,
                serial: 0,
                finalize: Finalize::Done,
                mark: Mark::Unreachable,
                pristine: false,
                lookup_dep: std::cell::Cell::new(false),
                vec_alloc: 0,
                vec_cell: 0,
            };
            // ID 0 is the invalid ID and is never on the free list.
            if id != 0 {
                slot.next_free = self.first_free;
                if self.first_free != 0 {
                    self.slots[self.first_free as usize].prev_free = id;
                }
                self.first_free = id;
            }
            self.slots.push(slot);
        }
    }

    fn unlink(&mut self, id: u32) {
        let (next, prev) = {
            let slot = &self.slots[id as usize];
            (slot.next_free, slot.prev_free)
        };
        if prev == 0 {
            self.first_free = next;
        } else {
            self.slots[prev as usize].next_free = next;
        }
        if next != 0 {
            self.slots[next as usize].prev_free = prev;
        }
    }

    /// Store a new object under the next free ID.
    pub fn alloc(&mut self, object: Object) -> ObjId {
        if self.first_free == 0 {
            self.add_page();
        }
        let id = self.first_free;
        if trace_alloc() {
            let kind = format!("{object:?}");
            let kind = kind.split(['(', ' ', '{']).next().unwrap_or("");
            if let Object::String(text) = &object {
                let text: String = String::from_utf8_lossy(text).chars().take(60).collect();
                eprintln!("A {id} {kind} {text:?}");
            } else {
                eprintln!("A {id} {kind}");
            }
        }
        self.unlink(id);
        self.serial = self.serial.wrapping_add(1);
        self.allocs_since_gc += 1;
        let serial = self.serial;
        let slot = &mut self.slots[id as usize];
        slot.serial = serial;
        slot.finalize = Finalize::NotYet;
        slot.mark = Mark::Unreachable;
        slot.vec_alloc = vec_len(&object);
        slot.vec_cell = crate::heap::cell_for(crate::heap::vector_bytes(slot.vec_alloc as usize));
        count_object(&object, slot.vec_alloc);
        slot.object = Some(object);
        slot.in_root_set = false;
        slot.transient = false;
        // A new object is not part of the current savepoint: undoing
        // leaves it as it is, with nothing referring to it.
        slot.in_undo = false;
        slot.saved = false;
        ObjId(id)
    }

    /// Store an object from the image under the ID the image gives it.
    /// Returns false if the ID is invalid or already in use.
    pub fn alloc_with_id(&mut self, id: ObjId, object: Object, transient: bool) -> bool {
        if !id.is_valid() {
            return false;
        }
        while id.0 as usize >= self.slots.len() {
            self.add_page();
        }
        if self.slots[id.0 as usize].object.is_some() {
            return false;
        }
        self.forget_lookups();
        self.unlink(id.0);
        let slot = &mut self.slots[id.0 as usize];
        slot.vec_alloc = vec_len(&object);
        slot.vec_cell = crate::heap::cell_for(crate::heap::vector_bytes(slot.vec_alloc as usize));
        count_object(&object, slot.vec_alloc);
        slot.object = Some(object);
        slot.in_root_set = true;
        slot.transient = transient;
        slot.in_undo = false;
        slot.saved = false;
        slot.serial = 0;
        slot.finalize = Finalize::NotYet;
        slot.mark = Mark::Unreachable;
        true
    }

    /// The object with an ID, if there is one.
    pub fn get(&self, id: ObjId) -> Option<&Object> {
        self.slots.get(id.0 as usize)?.object.as_ref()
    }

    /// The object with an ID, for modification. The first time an object
    /// is taken for modification after a savepoint, a copy of it goes
    /// into the savepoint, which is what makes every kind of change to
    /// every kind of object undoable.
    pub fn get_mut(&mut self, id: ObjId) -> Option<&mut Object> {
        self.changed(id.0);
        self.get_mut_tracked(id)
    }

    /// The object with an ID, to set one property of. Only what is
    /// remembered of that property's lookups is forgotten.
    pub fn get_mut_for_prop(
        &mut self,
        id: ObjId,
        prop: crate::value::PropId,
    ) -> Option<&mut Object> {
        self.changed_prop(id.0, prop);
        self.get_mut_tracked(id)
    }

    /// The object with an ID, for modification, its state kept for undo.
    fn get_mut_tracked(&mut self, id: ObjId) -> Option<&mut Object> {
        let slot = self.slots.get_mut(id.0 as usize)?;
        if slot.in_root_set && !slot.pristine && self.track_image_state {
            if let Some(object) = &slot.object {
                self.image_state.insert(id.0, object.clone());
                slot.pristine = true;
            }
        }
        if slot.in_undo && !slot.saved && !slot.transient {
            if let (Some(changes), Some(object)) = (self.savepoints.back_mut(), &slot.object) {
                count_refs(&mut self.saved_refs, object, true);
                changes.push((id.0, object.clone()));
                slot.saved = true;
            }
        }
        self.slots[id.0 as usize].object.as_mut()
    }

    /// Start tracking changes to every object now in the table.
    fn track_all(&mut self) {
        for slot in &mut self.slots {
            if slot.object.is_some() {
                slot.in_undo = true;
                slot.saved = false;
            }
        }
    }

    /// Make a savepoint: a state `undo` can return to.
    pub fn savepoint(&mut self) {
        prof_scope!(crate::prof::UNDO);
        if self.savepoints.len() == MAX_SAVEPOINTS
            && let Some(changes) = self.savepoints.pop_front()
        {
            for (_, object) in &changes {
                count_refs(&mut self.saved_refs, object, false);
            }
        }
        self.savepoints.push_back(Vec::new());
        self.track_all();
    }

    /// Return every object to its state at the latest savepoint, and
    /// forget that savepoint. Returns false if there is none.
    pub fn undo(&mut self) -> bool {
        prof_scope!(crate::prof::UNDO);
        let Some(changes) = self.savepoints.pop_back() else {
            return false;
        };
        self.forget_lookups();
        for (_, object) in &changes {
            count_refs(&mut self.saved_refs, object, false);
        }
        for (id, mut object) in changes.into_iter().rev() {
            if let Some(slot) = self.slots.get_mut(id as usize) {
                // Undoing changes to a string buffer's text does not take
                // back the room it grew to, in the reference.
                if let (Some(Object::StringBuilder(now)), Object::StringBuilder(then)) =
                    (&slot.object, &mut object)
                {
                    then.room = now.room;
                }
                if slot.object.is_some() {
                    slot.object = Some(object);
                }
            }
        }
        self.track_all();
        true
    }

    /// Forget every savepoint.
    pub fn drop_undo(&mut self) {
        self.savepoints.clear();
        self.saved_refs.clear();
    }

    /// The program-defined object (or built-in class modifier) with an ID.
    pub fn program_object(&self, id: ObjId) -> Option<&ProgramObject> {
        match self.get(id)? {
            Object::Program(object) | Object::IntClassMod(object) => Some(object),
            _ => None,
        }
    }

    /// The program-defined object (or built-in class modifier) with an
    /// ID, for modification.
    pub fn program_object_mut(&mut self, id: ObjId) -> Option<&mut ProgramObject> {
        match self.get_mut(id)? {
            Object::Program(object) | Object::IntClassMod(object) => Some(object),
            _ => None,
        }
    }

    /// Whether an object is transient.
    pub fn is_transient(&self, id: ObjId) -> bool {
        self.slots.get(id.0 as usize).is_some_and(|s| s.transient)
    }

    /// Mark an object transient.
    pub fn set_transient(&mut self, id: ObjId) {
        if let Some(slot) = self.slots.get_mut(id.0 as usize) {
            slot.transient = true;
        }
    }

    /// Whether an object came from the image. Such objects are never
    /// garbage.
    pub fn is_in_root_set(&self, id: ObjId) -> bool {
        self.slots.get(id.0 as usize).is_some_and(|s| s.in_root_set)
    }

    /// One past the highest ID the table has a slot for.
    pub fn id_limit(&self) -> u32 {
        self.slots.len() as u32
    }

    /// How many elements a vector has room for.
    pub fn vector_alloc(&self, id: ObjId) -> usize {
        self.slots
            .get(id.0 as usize)
            .map_or(0, |slot| slot.vec_alloc as usize)
    }

    /// Give a vector room for so many elements, as it is created.
    pub fn set_vector_alloc(&mut self, id: ObjId, count: usize) {
        if let Some(slot) = self.slots.get_mut(id.0 as usize) {
            // It was counted with the room its elements take.
            let before = crate::heap::vector_bytes(slot.vec_alloc as usize);
            crate::heap::adjust(crate::heap::vector_bytes(count) - before, "vector");
            slot.vec_alloc = count as u32;
            slot.vec_cell = crate::heap::cell_for(crate::heap::vector_bytes(count));
        }
    }

    /// Note that a vector's length has changed, or has been this much
    /// for a moment: one that outgrows its room is given 16 more
    /// elements' room, or as much as it needs if that is more. Room is
    /// never given back.
    pub fn note_vector_len(&mut self, id: ObjId, len: usize) {
        if let Some(slot) = self.slots.get_mut(id.0 as usize) {
            let alloc = slot.vec_alloc as usize;
            if len > alloc {
                slot.vec_alloc = (alloc + 16).max(len) as u32;
                let bytes = crate::heap::vector_bytes(slot.vec_alloc as usize);
                crate::heap::realloc(&mut slot.vec_cell, bytes, "vector");
            }
        }
    }

    /// Note a vector's length as it now is. See [`Self::note_vector_len`].
    pub fn note_vector(&mut self, id: ObjId) {
        if let Some(Object::Vector(items)) = self.get(id) {
            let len = items.len();
            self.note_vector_len(id, len);
        }
    }

    /// The IDs in use, in ascending order.
    pub fn ids(&self) -> impl Iterator<Item = ObjId> + '_ {
        self.slots
            .iter()
            .enumerate()
            .filter(|(_, slot)| slot.object.is_some())
            .map(|(id, _)| ObjId(id as u32))
    }
}

/// Whether to list each object ID as it is handed out, and each
/// collection, on standard error (`HEXICON_TRACE_ALLOC`): for comparing
/// the order with the reference VM's.
pub(crate) fn trace_alloc() -> bool {
    static TRACE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *TRACE.get_or_init(|| std::env::var_os("HEXICON_TRACE_ALLOC").is_some())
}

/// Count the bytes the reference allocates for a new object.
fn count_object(object: &Object, vector_room: u32) {
    let kind = match object {
        Object::Program(_) => "hexicon-object",
        Object::IntClassMod(_) => "int-class-mod",
        Object::String(_) => "string",
        Object::List(_) => "list",
        Object::Vector(_) => "vector",
        Object::AnonFn(_) => "anon-func-ptr",
        Object::IndexedIter { .. } => "indexed-iterator",
        Object::MapIter { .. } => "lookuptable-iterator",
        Object::Map(table) if table.weak => "weakreflookuptable",
        Object::Map(_) => "lookuptable",
        Object::Pattern { .. } => "regex-pattern",
        Object::StringBuilder(_) => "stringbuffer",
        Object::Decimal(_) => "bignumber",
        Object::ByteArray(_) => "bytearray",
        Object::CharSet { .. } => "character-set",
        Object::StrComp(_) => "string-comparator",
        Object::GrammarRule(_) => "grammar-production",
        Object::Dict(_) => "dictionary2",
        Object::Class(_) => "intrinsic-class",
        Object::File(_) => "file",
        Object::Date { .. } => "date",
        Object::TimeZone(_) => "timezone",
        Object::FileName(_) => "filename",
        Object::TempFile(_) => "tempfile",
        Object::CompiledCode(_) => "dynamic-func",
        Object::FrameRef(_) => "stack-frame-ref",
        Object::FrameDesc(_) => "stack-frame-desc",
        Object::Opaque { .. } => "-",
    };
    crate::heap::count(
        crate::heap::object_bytes(object, vector_room as usize),
        kind,
    );
}

/// The elements a vector or an anonymous function has, which is the room
/// it has unless something says otherwise.
fn vec_len(object: &Object) -> u32 {
    match object {
        Object::Vector(items) | Object::AnonFn(items) => items.len() as u32,
        _ => 0,
    }
}

// ---- garbage collection ----

/// The values an object refers to, for an object not in the table.
pub(crate) fn references_of(object: &Object, out: &mut Vec<Value>) {
    references(object, out);
}

/// Count up, or down, the references an object's saved state makes.
fn count_refs(counts: &mut std::collections::HashMap<u32, u32>, object: &Object, up: bool) {
    let mut found = Vec::new();
    references(object, &mut found);
    for value in found {
        if up {
            if let Value::Obj(id) | Value::ObjX(id) = value {
                *counts.entry(id.0).or_default() += 1;
            }
        } else {
            uncount(counts, value);
        }
    }
}

/// Count down one reference a saved state makes.
fn uncount(counts: &mut std::collections::HashMap<u32, u32>, value: Value) {
    if let Value::Obj(id) | Value::ObjX(id) = value
        && let Some(n) = counts.get_mut(&id.0)
    {
        *n -= 1;
        if *n == 0 {
            counts.remove(&id.0);
        }
    }
}

/// The values an object refers to. A weak-reference table does not count
/// as referring to its values.
fn references(object: &Object, out: &mut Vec<Value>) {
    match object {
        Object::Program(obj) | Object::IntClassMod(obj) => {
            out.extend(obj.superclasses.iter().map(|&id| Value::Obj(id)));
            out.extend(obj.props().iter().map(|entry| entry.value));
        }
        Object::List(items) => out.extend(items.iter().copied()),
        Object::Vector(items) | Object::AnonFn(items) => out.extend(items.iter().copied()),
        Object::IndexedIter { collection, .. } => out.push(*collection),
        Object::Map(table) => {
            for (key, value) in table.entries() {
                out.push(key);
                if !table.weak {
                    out.push(value);
                }
            }
            if !table.weak {
                out.push(table.default_value);
            }
        }
        Object::MapIter { table, .. } => out.push(Value::Obj(*table)),
        // A dictionary keeps its comparator, but not the objects its
        // words refer to.
        Object::Dict(dictionary) => out.push(Value::Obj(dictionary.comparator)),
        Object::GrammarRule(production) => production.references(out),
        Object::Pattern { source, .. } => out.push(*source),
        Object::Class(class) => {
            out.push(Value::Obj(class.modifier));
            out.push(class.class_state);
        }
        Object::File(file) => {
            out.push(Value::Obj(file.charset));
            out.push(file.spec);
        }
        // What its code refers to, its source, and its method context.
        Object::CompiledCode(func) => {
            out.extend(func.object_refs().map(Value::Obj));
            out.push(func.source);
            out.push(func.method_ctx);
        }
        // The function, the method context, what was called, and the
        // variables kept after the call returned.
        Object::FrameRef(fref) => {
            out.push(fref.entry);
            out.push(fref.self_value);
            out.push(Value::Obj(fref.definer));
            out.push(Value::Obj(fref.target));
            out.push(fref.invokee);
            out.extend(fref.vars.iter().copied());
        }
        Object::FrameDesc(desc) => out.push(Value::Obj(desc.fref)),
        Object::String(_)
        | Object::FileName(_)
        | Object::Date { .. }
        | Object::TimeZone(_)
        | Object::TempFile(_)
        | Object::CharSet { .. }
        | Object::ByteArray(_)
        | Object::Decimal(_)
        | Object::StringBuilder(_)
        | Object::StrComp(_)
        | Object::Opaque { .. } => {}
    }
}

/// The text of a string buffer, and the room the reference would have
/// allocated for it, which an image or a saved game records.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StringBuilder {
    /// The characters.
    pub chars: Vec<char>,
    /// How many characters there is room for.
    pub room: u32,
    /// How many characters the room grows by, at a time.
    pub step: u32,
}

impl StringBuilder {
    /// The most room a buffer can have.
    pub const MAX_ROOM: u32 = 0x7fff_ffff;

    /// An empty buffer with this much room, growing by this much.
    pub fn new(room: u32, step: u32) -> StringBuilder {
        StringBuilder {
            chars: Vec::new(),
            room,
            step,
        }
    }

    /// Make room for the characters there are now, in steps, as the
    /// reference does before it adds them.
    pub fn grow(&mut self) {
        let len = self.chars.len() as u32;
        if len > self.room {
            let step = self.step.max(1);
            self.room = len.div_ceil(step).saturating_mul(step);
            crate::heap::count(crate::heap::string_builder_bytes(self), "stringbuffer");
        }
    }
}

impl std::ops::Deref for StringBuilder {
    type Target = Vec<char>;

    fn deref(&self) -> &Vec<char> {
        &self.chars
    }
}

impl ObjectTable {
    /// Note that an instruction is starting at this level of nested
    /// runs, with no built-in at this level in the middle of its work.
    #[inline]
    pub fn at_safe_point(&mut self, depth: u32) {
        self.safe_serial = self.serial;
        if self
            .held_ranges
            .last()
            .is_some_and(|&(level, _, _)| level >= depth)
            || self
                .held_results
                .last()
                .is_some_and(|&(level, _)| level >= depth)
        {
            self.held_ranges.retain(|&(level, _, _)| level < depth);
            self.held_results.retain(|&(level, _)| level < depth);
        }
    }

    /// Note that a built-in at this level is calling program code,
    /// which runs a level deeper: what it created before the call it may
    /// still be holding.
    pub fn enter_nested(&mut self, depth: u32) {
        self.held_ranges
            .push((depth, self.safe_serial, self.serial));
    }

    /// Note that program code a built-in at this level called has
    /// returned this to it. What the code created and did not return is
    /// garbage, as far as the built-in is concerned.
    pub fn leave_nested(&mut self, depth: u32, result: Value) {
        self.safe_serial = self.serial;
        if matches!(result, Value::Obj(_)) {
            self.held_results.push((depth, result));
        }
    }

    /// Whether enough objects have been created that the collector
    /// should run before the next one.
    #[inline]
    pub fn gc_due(&self) -> bool {
        self.allocs_since_gc >= ALLOCS_BETWEEN_GC
            || crate::heap::since_gc() > crate::heap::BYTES_BETWEEN_GC
    }

    /// Start keeping the loaded state of image objects as they change.
    /// This is for the end of loading: what loading itself does to an
    /// object is part of its loaded state.
    pub fn track_image_state(&mut self) {
        self.track_image_state = true;
    }

    /// Whether an object from the image has been taken for modification
    /// since it was loaded.
    pub fn was_changed(&self, id: ObjId) -> bool {
        self.slots.get(id.0 as usize).is_some_and(|s| s.pristine)
    }

    /// Put every object from the image back as it was loaded, except the
    /// transient ones, which outlast a restore or a restart. Objects the
    /// program created stay until the collector finds them unreachable.
    pub fn reset_to_image(&mut self) {
        self.forget_lookups();
        let slots = &mut self.slots;
        self.image_state.retain(|&id, object| {
            let slot = &mut slots[id as usize];
            if slot.transient {
                return true;
            }
            slot.object = Some(object.clone());
            slot.pristine = false;
            false
        });
    }

    /// Mark what is reachable from the image's objects and the given
    /// roots without going through a transient object, for saving.
    pub fn mark_persistent(&mut self, roots: &[Value]) {
        let mut all_roots: Vec<Value> = roots.to_vec();
        for (id, slot) in self.slots.iter_mut().enumerate() {
            slot.mark = Mark::Unreachable;
            if slot.object.is_some() && slot.in_root_set {
                all_roots.push(Value::Obj(ObjId(id as u32)));
            }
        }
        self.mark_from_with(&all_roots, Mark::Reachable, false);
    }

    /// Whether the last marking reached an object.
    pub fn is_marked(&self, id: ObjId) -> bool {
        self.slots
            .get(id.0 as usize)
            .is_some_and(|s| s.mark == Mark::Reachable)
    }

    /// Leave the next object created out of the count toward the next
    /// collection, as the reference does for the one whose creation set
    /// the collector off.
    pub fn skip_next_alloc_count(&mut self) {
        self.allocs_since_gc = -1;
    }

    fn mark_from(&mut self, roots: &[Value], mark: Mark) {
        self.mark_from_with(roots, mark, true);
    }

    fn mark_from_with(&mut self, roots: &[Value], mark: Mark, through_transient: bool) {
        let mut work: Vec<u32> = Vec::new();
        let mut found: Vec<Value> = Vec::new();
        let visit = |slots: &mut Vec<Slot>, value: Value, work: &mut Vec<u32>| {
            let (Value::Obj(id) | Value::ObjX(id)) = value else {
                return;
            };
            if let Some(slot) = slots.get_mut(id.0 as usize) {
                if slot.object.is_some() && slot.mark == Mark::Unreachable {
                    slot.mark = mark;
                    work.push(id.0);
                }
            }
        };
        for &root in roots {
            visit(&mut self.slots, root, &mut work);
        }
        while let Some(id) = work.pop() {
            found.clear();
            let slot = &self.slots[id as usize];
            if !through_transient && slot.transient {
                continue;
            }
            if let Some(object) = &slot.object {
                references(object, &mut found);
            }
            for &value in &found {
                visit(&mut self.slots, value, &mut work);
            }
        }
    }

    /// The first half of a collection: find what is reachable, and return
    /// the unreachable objects that have never been finalized, for the
    /// caller to say which have finalizers.
    ///
    /// The roots are the values the caller gives, every object from the
    /// image, and what the savepoints refer to. While a built-in
    /// function or method is in the middle of its work, which is when
    /// `in_native` is set, it can be holding objects that nothing else
    /// refers to yet. Those are all newer than the last safe point, so
    /// everything newer than that counts as a root too. So does what an
    /// built-in at an outer level may be holding while program code it
    /// called runs: what it created before the call, and what earlier
    /// calls returned to it.
    pub fn collect_mark(&mut self, roots: &[Value], in_native: bool) -> Vec<ObjId> {
        self.allocs_since_gc = 0;
        crate::heap::reset();
        prof_scope!(crate::prof::GC_ROOTS);
        let mut all_roots: Vec<Value> = roots.to_vec();
        all_roots.extend(self.held_results.iter().map(|&(_, value)| value));
        let safe_serial = self.safe_serial;
        // The serials the builtins at each level may be holding, merged
        // into ranges in order, so that each object is a search away from
        // knowing whether it is in one, however deep the levels go.
        let mut ranges: Vec<(u32, u32)> = self
            .held_ranges
            .iter()
            .filter(|&&(_, low, high)| high > low)
            .map(|&(_, low, high)| (low, high))
            .collect();
        ranges.sort_unstable();
        let mut held: Vec<(u32, u32)> = Vec::with_capacity(ranges.len());
        for (low, high) in ranges {
            match held.last_mut() {
                Some(last) if low <= last.1 => last.1 = last.1.max(high),
                _ => held.push((low, high)),
            }
        }
        let is_held = |serial: u32| {
            let after = held.partition_point(|&(low, _)| low < serial);
            after > 0 && serial <= held[after - 1].1
        };
        for (id, slot) in self.slots.iter_mut().enumerate() {
            slot.mark = Mark::Unreachable;
            if slot.object.is_some()
                && (slot.in_root_set
                    || (in_native && slot.serial > safe_serial)
                    || is_held(slot.serial))
            {
                all_roots.push(Value::Obj(ObjId(id as u32)));
            }
        }
        all_roots.extend(self.saved_refs.keys().map(|&id| Value::Obj(ObjId(id))));
        prof_scope!(crate::prof::GC_MARK);
        self.mark_from(&all_roots, Mark::Reachable);
        self.slots
            .iter()
            .enumerate()
            .filter(|(_, slot)| {
                slot.object.is_some()
                    && !slot.in_root_set
                    && slot.mark == Mark::Unreachable
                    && slot.finalize == Finalize::NotYet
            })
            .map(|(id, _)| ObjId(id as u32))
            .collect()
    }

    /// The second half of a collection. `with_finalizers` are the
    /// candidates from [`ObjectTable::collect_mark`] that have a
    /// finalizer. Everything unreachable that is not waiting for a
    /// finalizer, or kept by something that is, goes. Returns the objects
    /// whose finalizers are to run now, in the order to run them.
    pub fn collect_sweep(&mut self, candidates: &[ObjId], with_finalizers: &[ObjId]) -> Vec<ObjId> {
        prof_scope!(crate::prof::GC_SWEEP);
        for &id in candidates {
            self.slots[id.0 as usize].finalize = Finalize::Done;
        }
        for &id in with_finalizers {
            self.slots[id.0 as usize].finalize = Finalize::Pending;
        }
        // What an object waiting to be finalized refers to has to last
        // until the finalizer has run.
        let pending: Vec<Value> = self
            .slots
            .iter()
            .enumerate()
            .filter(|(_, slot)| {
                slot.object.is_some() && !slot.in_root_set && slot.finalize == Finalize::Pending
            })
            .map(|(id, _)| Value::Obj(ObjId(id as u32)))
            .collect();
        // Mark from them, without marking them, so that the ones that
        // only refer to each other can be told from the rest.
        let mut from_pending = Vec::new();
        for value in &pending {
            if let Value::Obj(id) = value {
                if let Some(object) = &self.slots[id.0 as usize].object {
                    references(object, &mut from_pending);
                }
            }
        }
        self.mark_from(&from_pending, Mark::FromFinalizable);

        let mut finalize_queue = Vec::new();
        // What this collection frees: only these can have left entries
        // behind in weak tables and dictionaries, since the entries for
        // what earlier ones freed went then.
        let mut freed = vec![false; self.slots.len()];
        let mut freed_any = false;
        for id in 0..self.slots.len() as u32 {
            let slot = &self.slots[id as usize];
            if slot.object.is_none() {
                continue;
            }
            if !slot.in_root_set
                && slot.mark == Mark::Unreachable
                && slot.finalize == Finalize::Done
            {
                freed[id as usize] = true;
                freed_any = true;
                self.free(id);
            } else if slot.finalize == Finalize::Pending {
                finalize_queue.push(ObjId(id));
            }
        }
        // The reference queues these as it finds them and takes the last
        // first.
        finalize_queue.reverse();
        for &id in &finalize_queue {
            self.slots[id.0 as usize].finalize = Finalize::Done;
        }

        prof_scope!(crate::prof::GC_WEAK);
        if !freed_any {
            return finalize_queue;
        }
        // Drop what refers to objects that are gone: values in weak
        // tables, and saved states of the objects themselves.
        let live = |slots: &[Slot], value: Value| match value {
            Value::Obj(id) | Value::ObjX(id) => slots
                .get(id.0 as usize)
                .is_some_and(|slot| slot.object.is_some()),
            _ => true,
        };
        let mut stale: Vec<(usize, Vec<usize>, bool)> = Vec::new();
        for (id, slot) in self.slots.iter().enumerate() {
            if let Some(Object::Map(table)) = &slot.object {
                if table.weak {
                    let entries: Vec<usize> = (0..table.slot_count())
                        .filter(|&index| {
                            table
                                .slot(index)
                                .is_some_and(|(_, value)| !live(&self.slots, value))
                        })
                        .collect();
                    let default_gone = !live(&self.slots, table.default_value);
                    if !entries.is_empty() || default_gone {
                        stale.push((id, entries, default_gone));
                    }
                }
            }
        }
        for (id, entries, default_gone) in stale {
            if let Some(Object::Map(table)) = &mut self.slots[id].object {
                for index in entries {
                    table.remove_slot(index);
                }
                if default_gone {
                    table.default_value = Value::Nil;
                }
            }
        }
        // Dictionaries drop their entries for objects that are gone.
        let is_alive = |id: ObjId| !freed.get(id.0 as usize).copied().unwrap_or(false);
        for slot in &mut self.slots {
            match &mut slot.object {
                Some(Object::Dict(dictionary)) => dictionary.remove_dead(&is_alive),
                Some(Object::GrammarRule(production)) => production.forget_comparator(&is_alive),
                _ => {}
            }
        }
        for changes in &mut self.savepoints {
            for (_, object) in changes.iter_mut() {
                if let Object::Dict(dictionary) = object {
                    dictionary.remove_dead(&is_alive);
                }
            }
        }
        let slots = &self.slots;
        let saved_refs = &mut self.saved_refs;
        for changes in &mut self.savepoints {
            changes.retain(|(id, object)| {
                let keep = slots[*id as usize].object.is_some();
                if !keep {
                    count_refs(saved_refs, object, false);
                }
                keep
            });
            for (_, object) in changes.iter_mut() {
                if let Object::Map(table) = object {
                    if table.weak {
                        // The keys of the entries dropped no longer count.
                        for index in 0..table.slot_count() {
                            if let Some((key, value)) = table.slot(index)
                                && !live(slots, value)
                            {
                                uncount(saved_refs, key);
                                table.remove_slot(index);
                            }
                        }
                        if !live(slots, table.default_value) {
                            table.default_value = Value::Nil;
                        }
                    }
                }
            }
        }
        finalize_queue
    }

    /// Release an object. Its ID goes to the front of the free list, so
    /// it is the next one handed out.
    fn free(&mut self, id: u32) {
        self.changed(id);
        let first = self.first_free;
        let slot = &mut self.slots[id as usize];
        // What a file object stands for outlives it until the VM has
        // dealt with it.
        if let Some(object @ (Object::File(_) | Object::TempFile(_))) = slot.object.take() {
            self.released.push(object);
        }
        slot.object = None;
        slot.in_root_set = false;
        slot.transient = false;
        slot.next_free = first;
        slot.prev_free = 0;
        if first != 0 {
            self.slots[first as usize].prev_free = id;
        }
        self.first_free = id;
    }
}
