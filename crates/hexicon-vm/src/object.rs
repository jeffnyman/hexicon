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
}

/// The object table: every object in the VM, indexed by ID.
///
/// Free IDs are kept on a list in the same order as the reference VM keeps
/// them, so a program that creates objects sees the same IDs under both.
#[derive(Debug, Clone, Default)]
pub struct ObjectTable {
    slots: Vec<Slot>,
    first_free: u32,
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
        let slot = &mut self.slots[id as usize];
        slot.object = Some(object);
        slot.in_root_set = false;
        slot.transient = false;
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
        true
    }

    /// The object with an ID, if there is one.
    pub fn get(&self, id: ObjId) -> Option<&Object> {
        self.slots.get(id.0 as usize)?.object.as_ref()
    }

    /// The object with an ID, for modification.
    pub fn get_mut(&mut self, id: ObjId) -> Option<&mut Object> {
        self.slots.get_mut(id.0 as usize)?.object.as_mut()
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
    #[allow(dead_code)]
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
