//! Program-defined objects: superclass lists, property tables, and the
//! inheritance search.

use std::cell::RefCell;
use std::rc::Rc;

use crate::object::{Object, ObjectTable};
use crate::value::{ObjId, PropId, Value};

/// The object was defined with `class`.
pub const OBJ_FLAG_CLASS: u16 = 0x0001;

/// One property of an object.
#[derive(Debug, Clone, Copy)]
pub struct PropEntry {
    /// The property ID.
    pub prop: PropId,
    /// The property's value.
    pub value: Value,
    /// The value has changed since the object was loaded.
    pub modified: bool,
    /// Next entry in the same hash bucket, plus one; zero ends the chain.
    next: u32,
}

/// A program-defined object.
///
/// Properties keep the order they were added in, which programs can see
/// through `getPropList`.
#[derive(Debug, Clone, Default)]
pub struct TadsObject {
    /// The direct superclasses, in declaration order.
    pub superclasses: Vec<ObjId>,
    /// Flags from the image; see [`OBJ_FLAG_CLASS`].
    pub load_flags: u16,
    /// The object has changed since it was loaded.
    pub modified: bool,
    entries: Vec<PropEntry>,
    /// Hash bucket heads: an index into `entries` plus one, or zero.
    buckets: Vec<u32>,
    /// The order in which to search superclasses when there is more than
    /// one, computed on first use.
    search_path: RefCell<Option<Rc<[ObjId]>>>,
    /// The reference's hash table size and the number of properties it
    /// has room for, which decide how many bytes it allocates. See
    /// [`crate::heap`].
    ref_hash: usize,
    ref_room: usize,
}

/// The properties a new object has room for unless it says otherwise, in
/// the reference.
const PROP_ROOM: usize = 16;

/// The reference's hash table size for an object with room asked for so
/// many properties, and the room it gives it.
fn ref_table(count: usize) -> (usize, usize) {
    let hash = count.next_power_of_two().clamp(2, 256);
    (hash, count.max(hash))
}

impl TadsObject {
    /// An object with the given superclasses and no properties.
    pub fn new(superclasses: Vec<ObjId>) -> Self {
        Self::with_room(superclasses, PROP_ROOM)
    }

    /// An object with the given superclasses and no properties, that the
    /// reference would give room for so many.
    pub fn with_room(superclasses: Vec<ObjId>, props: usize) -> Self {
        let (ref_hash, ref_room) = ref_table(props);
        Self {
            superclasses,
            ref_hash,
            ref_room,
            ..Self::default()
        }
    }

    /// The bytes the reference's header for this object takes.
    pub(crate) fn heap_bytes(&self) -> i64 {
        crate::heap::tads_bytes(self.superclasses.len(), self.ref_hash, self.ref_room)
    }

    /// Grow the reference's header to so many superclasses and room for
    /// so many properties, counting what it allocates.
    fn ref_expand(&mut self, superclasses: usize, props: usize) {
        (self.ref_hash, self.ref_room) = ref_table(props);
        crate::heap::count(
            crate::heap::tads_bytes(superclasses, self.ref_hash, self.ref_room),
            "tads-object",
        );
    }

    /// Read an object from its image data: a superclass count, a property
    /// count, flags, the superclass IDs, and the properties.
    pub fn from_image(data: &[u8]) -> Option<Self> {
        let u16_at = |at: usize| Some(u16::from_le_bytes([*data.get(at)?, *data.get(at + 1)?]));
        let superclass_count = u16_at(0)? as usize;
        let prop_count = u16_at(2)? as usize;
        let (ref_hash, ref_room) = ref_table(prop_count);
        let mut object = Self {
            load_flags: u16_at(4)?,
            ref_hash,
            ref_room,
            ..Self::default()
        };
        let mut pos = 6;
        for _ in 0..superclass_count {
            let b = data.get(pos..pos + 4)?;
            object
                .superclasses
                .push(ObjId(u32::from_le_bytes([b[0], b[1], b[2], b[3]])));
            pos += 4;
        }
        object.entries.reserve(prop_count);
        for _ in 0..prop_count {
            let prop = u16_at(pos)?;
            let holder = data.get(pos + 2..pos + 2 + Value::HOLDER_SIZE)?;
            object.insert(prop, Value::from_holder(holder));
            pos += 2 + Value::HOLDER_SIZE;
        }
        Some(object)
    }

    /// Whether the object was defined as a class.
    pub fn is_class(&self) -> bool {
        self.load_flags & OBJ_FLAG_CLASS != 0
    }

    fn bucket(&self, prop: PropId) -> usize {
        prop as usize & (self.buckets.len() - 1)
    }

    fn rehash(&mut self, size: usize) {
        self.buckets = vec![0; size];
        for index in 0..self.entries.len() {
            let bucket = self.bucket(self.entries[index].prop);
            self.entries[index].next = self.buckets[bucket];
            self.buckets[bucket] = index as u32 + 1;
        }
    }

    /// Append a property that is known not to be present.
    fn insert(&mut self, prop: PropId, value: Value) -> &mut PropEntry {
        if self.entries.len() >= self.ref_room {
            // The reference doubles the room, or adds half once it is
            // large.
            let room = self.ref_room;
            let props = if room <= 128 {
                room * 2
            } else {
                room + room / 2
            };
            self.ref_expand(self.superclasses.len(), props.max(1));
        }
        if self.entries.len() >= self.buckets.len() {
            self.rehash((self.buckets.len() * 2).max(4));
        }
        let bucket = self.bucket(prop);
        self.entries.push(PropEntry {
            prop,
            value,
            modified: false,
            next: self.buckets[bucket],
        });
        self.buckets[bucket] = self.entries.len() as u32;
        self.entries.last_mut().expect("just pushed")
    }

    fn index_of(&self, prop: PropId) -> Option<usize> {
        if self.buckets.is_empty() {
            return None;
        }
        let mut link = self.buckets[self.bucket(prop)];
        while link != 0 {
            let entry = &self.entries[link as usize - 1];
            if entry.prop == prop {
                return Some(link as usize - 1);
            }
            link = entry.next;
        }
        None
    }

    /// The value of a property this object defines directly.
    pub fn get(&self, prop: PropId) -> Option<Value> {
        self.index_of(prop).map(|index| self.entries[index].value)
    }

    /// Set a property on this object, adding it if it is not defined here.
    /// Returns the value it replaced.
    pub fn set(&mut self, prop: PropId, value: Value) -> Option<Value> {
        self.modified = true;
        match self.index_of(prop) {
            Some(index) => {
                let entry = &mut self.entries[index];
                entry.modified = true;
                Some(std::mem::replace(&mut entry.value, value))
            }
            None => {
                self.insert(prop, value).modified = true;
                None
            }
        }
    }

    /// Give a property this object defines a new value, as it is when
    /// the image is written: no change the program made.
    pub fn set_value_in_place(&mut self, prop: PropId, value: Value) {
        if let Some(index) = self.index_of(prop) {
            self.entries[index].value = value;
        }
    }

    /// The properties this object defines directly, in the order they were
    /// added.
    pub fn props(&self) -> &[PropEntry] {
        &self.entries
    }

    /// Replace the superclass list.
    pub fn set_superclasses(&mut self, superclasses: Vec<ObjId>) {
        if superclasses.len() > self.superclasses.len() {
            self.ref_expand(superclasses.len(), self.ref_room);
        }
        self.superclasses = superclasses;
        self.invalidate_search_path();
    }

    /// Forget the computed superclass search order.
    pub fn invalidate_search_path(&self) {
        *self.search_path.borrow_mut() = None;
    }

    /// Whether the computed search order mentions an object.
    pub fn search_path_contains(&self, id: ObjId) -> bool {
        self.search_path
            .borrow()
            .as_ref()
            .is_some_and(|path| path.contains(&id))
    }

    /// The order in which to search this object's superclasses, direct and
    /// indirect, for an inherited property.
    ///
    /// The order is breadth-first by declaration, except that a class
    /// reachable by more than one route takes its last position, so a
    /// shared base class comes after everything derived from it.
    fn search_path(&self, table: &ObjectTable) -> Rc<[ObjId]> {
        if let Some(path) = self.search_path.borrow().as_ref() {
            return path.clone();
        }
        const NONE: usize = usize::MAX;
        // A singly linked queue. Removing an element leaves a tombstone.
        let mut nodes: Vec<(ObjId, usize)> = Vec::new();
        let mut head = NONE;
        let mut position = NONE;
        let mut current = self;
        'build: loop {
            let mut insert_after = position;
            for &superclass in &current.superclasses {
                if !matches!(table.get(superclass), Some(Object::Tads(_))) {
                    continue;
                }
                let mut node = head;
                while node != NONE {
                    if nodes[node].0 == superclass {
                        nodes[node].0 = ObjId::INVALID;
                        break;
                    }
                    node = nodes[node].1;
                }
                let new = nodes.len();
                if insert_after == NONE {
                    nodes.push((superclass, head));
                    head = new;
                } else {
                    nodes.push((superclass, nodes[insert_after].1));
                    nodes[insert_after].1 = new;
                }
                insert_after = new;
            }
            loop {
                position = if position == NONE {
                    head
                } else {
                    nodes[position].1
                };
                if position == NONE {
                    break 'build;
                }
                if nodes[position].0.is_valid() {
                    break;
                }
            }
            match table.tads(nodes[position].0) {
                Some(object) => current = object,
                None => break,
            }
        }
        let mut path = Vec::new();
        let mut node = head;
        while node != NONE {
            if nodes[node].0.is_valid() {
                path.push(nodes[node].0);
            }
            node = nodes[node].1;
        }
        let path: Rc<[ObjId]> = path.into();
        *self.search_path.borrow_mut() = Some(path.clone());
        path
    }
}

/// A walk from an object up through its superclasses in inheritance order.
pub struct Search<'a> {
    table: &'a ObjectTable,
    /// The object the walk is at.
    pub current: ObjId,
    object: Option<&'a TadsObject>,
    path: Option<(Rc<[ObjId]>, usize)>,
}

impl<'a> Search<'a> {
    /// Start a walk at an object.
    pub fn new(table: &'a ObjectTable, start: ObjId) -> Self {
        Self {
            table,
            current: start,
            object: table.tads(start),
            path: None,
        }
    }

    /// Move to the next object in inheritance order. Returns false at the
    /// end.
    pub fn advance(&mut self) -> bool {
        loop {
            if let Some((path, position)) = &mut self.path {
                let Some(&next) = path.get(*position) else {
                    return false;
                };
                *position += 1;
                self.current = next;
                self.object = self.table.tads(next);
                return true;
            }
            let Some(object) = self.object else {
                return false;
            };
            match object.superclasses.len() {
                0 => return false,
                // With one superclass there is nothing to order. This is
                // by far the most common case.
                1 => {
                    self.current = object.superclasses[0];
                    self.object = self.table.tads(self.current);
                    return self.object.is_some();
                }
                _ => self.path = Some((object.search_path(self.table), 0)),
            }
        }
    }

    /// Find the nearest definition of a property at or after the current
    /// position. Returns its value and the object that defines it.
    pub fn find(&mut self, prop: PropId) -> Option<(Value, ObjId)> {
        loop {
            if let Some(value) = self.object.and_then(|object| object.get(prop)) {
                return Some((value, self.current));
            }
            if !self.advance() {
                return None;
            }
        }
    }

    /// Move forward to a given object. Returns false if the walk ends
    /// without reaching it.
    pub fn skip_to(&mut self, target: ObjId) -> bool {
        while self.current != target {
            if !self.advance() {
                return false;
            }
        }
        true
    }
}

/// Find a property on an object or its superclasses.
pub fn find_prop(table: &ObjectTable, object: ObjId, prop: PropId) -> Option<(Value, ObjId)> {
    Search::new(table, object).find(prop)
}

/// Find the definition of a property that the one in `definer` overrides,
/// in the inheritance order of `target`. With an invalid `definer`, find
/// the first definition.
pub fn find_inherited_prop(
    table: &ObjectTable,
    target: ObjId,
    definer: ObjId,
    prop: PropId,
) -> Option<(Value, ObjId)> {
    let mut search = Search::new(table, target);
    if definer.is_valid() && (!search.skip_to(definer) || !search.advance()) {
        return None;
    }
    search.find(prop)
}

/// Whether an object inherits, directly or indirectly, from another
/// program-defined object.
pub fn inherits_from(table: &ObjectTable, object: ObjId, ancestor: ObjId) -> bool {
    let mut search = Search::new(table, object);
    while search.advance() {
        if search.current == ancestor {
            return true;
        }
    }
    false
}
