//! The object table and the kinds of object it holds.

use std::rc::Rc;

use crate::bignum::Big;
use crate::lookup::LookupTable;
use crate::regex::Pattern;
use crate::tads::TadsObject;
use crate::value::{ObjId, Value};

/// An intrinsic class object: the object that represents one of the VM's
/// built-in classes to the program.
#[derive(Debug, Clone)]
pub struct IntrinsicClass {
    /// Index of the class in the image's metaclass dependency list.
    pub meta_index: u16,
    /// The modifier object holding program-defined methods for the class,
    /// or the invalid ID.
    pub modifier: ObjId,
    /// Class-level state some intrinsic classes keep.
    #[allow(dead_code)]
    pub class_state: Value,
}

/// An object.
#[derive(Debug, Clone)]
pub enum Object {
    /// A program-defined object or class.
    Tads(TadsObject),
    /// The program's extensions to an intrinsic class. It has the same
    /// structure as a program-defined object.
    IntClassMod(TadsObject),
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
    LookupTable(Box<LookupTable>),
    /// An iterator over a lookup table.
    LookupIter {
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
    /// A byte array: bytes that can change in place.
    ByteArray(Vec<u8>),
    /// A BigNumber: a decimal floating-point number.
    BigNumber(Box<Big>),
    /// A string buffer: text that can change in place.
    StringBuffer(Vec<char>),
    /// A compiled regular expression.
    Pattern {
        /// The pattern as the program wrote it: a string value.
        source: Value,
        /// The compiled pattern. A pattern from an image is compiled
        /// when it is first used.
        compiled: Option<Rc<Pattern>>,
    },
    /// An intrinsic class object.
    Class(IntrinsicClass),
    /// An object of an intrinsic class Hexicon does not implement yet. Its
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
    /// How many objects have ever been created.
    serial: u32,
    /// How many have been created since the collector last ran.
    allocs_since_gc: i32,
    /// The value of `serial` when the instruction now running at the
    /// outermost level began. See [`ObjectTable::collect`].
    safe_serial: u32,
    /// Objects from the image that have changed since it was loaded, as
    /// they were then. Restoring and restarting begin from these.
    image_state: std::collections::HashMap<u32, Object>,
    /// Whether changes to image objects are being recorded yet.
    track_image_state: bool,
}

impl ObjectTable {
    /// An empty table.
    pub fn new() -> Self {
        Self::default()
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
        self.unlink(id);
        self.serial = self.serial.wrapping_add(1);
        self.allocs_since_gc += 1;
        let serial = self.serial;
        let slot = &mut self.slots[id as usize];
        slot.serial = serial;
        slot.finalize = Finalize::NotYet;
        slot.mark = Mark::Unreachable;
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
        self.unlink(id.0);
        let slot = &mut self.slots[id.0 as usize];
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
        let slot = self.slots.get_mut(id.0 as usize)?;
        if slot.in_root_set && !slot.pristine && self.track_image_state {
            if let Some(object) = &slot.object {
                self.image_state.insert(id.0, object.clone());
                slot.pristine = true;
            }
        }
        if slot.in_undo && !slot.saved && !slot.transient {
            if let (Some(changes), Some(object)) = (self.savepoints.back_mut(), &slot.object) {
                changes.push((id.0, object.clone()));
                slot.saved = true;
            }
        }
        slot.object.as_mut()
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
        if self.savepoints.len() == MAX_SAVEPOINTS {
            self.savepoints.pop_front();
        }
        self.savepoints.push_back(Vec::new());
        self.track_all();
    }

    /// Return every object to its state at the latest savepoint, and
    /// forget that savepoint. Returns false if there is none.
    pub fn undo(&mut self) -> bool {
        let Some(changes) = self.savepoints.pop_back() else {
            return false;
        };
        for (id, object) in changes.into_iter().rev() {
            if let Some(slot) = self.slots.get_mut(id as usize) {
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
    }

    /// The program-defined object (or intrinsic class modifier) with an ID.
    pub fn tads(&self, id: ObjId) -> Option<&TadsObject> {
        match self.get(id)? {
            Object::Tads(object) | Object::IntClassMod(object) => Some(object),
            _ => None,
        }
    }

    /// The program-defined object (or intrinsic class modifier) with an
    /// ID, for modification.
    pub fn tads_mut(&mut self, id: ObjId) -> Option<&mut TadsObject> {
        match self.get_mut(id)? {
            Object::Tads(object) | Object::IntClassMod(object) => Some(object),
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

    /// The IDs in use, in ascending order.
    pub fn ids(&self) -> impl Iterator<Item = ObjId> + '_ {
        self.slots
            .iter()
            .enumerate()
            .filter(|(_, slot)| slot.object.is_some())
            .map(|(id, _)| ObjId(id as u32))
    }
}

// ---- garbage collection ----

/// The values an object refers to. A weak-reference table does not count
/// as referring to its values.
fn references(object: &Object, out: &mut Vec<Value>) {
    match object {
        Object::Tads(tads) | Object::IntClassMod(tads) => {
            out.extend(tads.superclasses.iter().map(|&id| Value::Obj(id)));
            out.extend(tads.props().iter().map(|entry| entry.value));
        }
        Object::List(items) => out.extend(items.iter().copied()),
        Object::Vector(items) | Object::AnonFn(items) => out.extend(items.iter().copied()),
        Object::IndexedIter { collection, .. } => out.push(*collection),
        Object::LookupTable(table) => {
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
        Object::LookupIter { table, .. } => out.push(Value::Obj(*table)),
        Object::Pattern { source, .. } => out.push(*source),
        Object::Class(class) => {
            out.push(Value::Obj(class.modifier));
            out.push(class.class_state);
        }
        Object::String(_)
        | Object::CharSet { .. }
        | Object::ByteArray(_)
        | Object::BigNumber(_)
        | Object::StringBuffer(_)
        | Object::Opaque { .. } => {}
    }
}

impl ObjectTable {
    /// Note that an instruction is starting at the outermost level, with
    /// no intrinsic function or method in the middle of its work.
    #[inline]
    pub fn at_safe_point(&mut self) {
        self.safe_serial = self.serial;
    }

    /// Whether enough objects have been created that the collector
    /// should run before the next one.
    #[inline]
    pub fn gc_due(&self) -> bool {
        self.allocs_since_gc >= ALLOCS_BETWEEN_GC
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
    /// image, and what the savepoints refer to. While an intrinsic
    /// function or method is in the middle of its work, which is when
    /// `in_native` is set, it can be holding objects that nothing else
    /// refers to yet. Those are all newer than the last safe point, so
    /// everything newer than that counts as a root too.
    pub fn collect_mark(&mut self, roots: &[Value], in_native: bool) -> Vec<ObjId> {
        self.allocs_since_gc = 0;
        let mut all_roots: Vec<Value> = roots.to_vec();
        let safe_serial = self.safe_serial;
        for (id, slot) in self.slots.iter_mut().enumerate() {
            slot.mark = Mark::Unreachable;
            if slot.object.is_some()
                && (slot.in_root_set || (in_native && slot.serial > safe_serial))
            {
                all_roots.push(Value::Obj(ObjId(id as u32)));
            }
        }
        for changes in &self.savepoints {
            for (_, object) in changes {
                references(object, &mut all_roots);
            }
        }
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
        for id in 0..self.slots.len() as u32 {
            let slot = &self.slots[id as usize];
            if slot.object.is_none() {
                continue;
            }
            if !slot.in_root_set
                && slot.mark == Mark::Unreachable
                && slot.finalize == Finalize::Done
            {
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
            if let Some(Object::LookupTable(table)) = &slot.object {
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
            if let Some(Object::LookupTable(table)) = &mut self.slots[id].object {
                for index in entries {
                    table.remove_slot(index);
                }
                if default_gone {
                    table.default_value = Value::Nil;
                }
            }
        }
        let slots = &self.slots;
        for changes in &mut self.savepoints {
            changes.retain(|(id, _)| slots[*id as usize].object.is_some());
            for (_, object) in changes.iter_mut() {
                if let Object::LookupTable(table) = object {
                    if table.weak {
                        for index in 0..table.slot_count() {
                            if table
                                .slot(index)
                                .is_some_and(|(_, value)| !live(slots, value))
                            {
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
        let first = self.first_free;
        let slot = &mut self.slots[id as usize];
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
