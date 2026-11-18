//! The lookup table intrinsic class: an associative array.
//!
//! Programs can see the order of a table's entries, through iteration and
//! the methods that list keys and values. That order is the order of the
//! table's storage slots, so slots are handed out and reused exactly as in
//! the reference VM. Which hash bucket a key lands in is not visible, so
//! hashing is Hexicon's own.

use crate::error::{VmError, VmResult, code};
use crate::object::Object;
use crate::value::{NativeDesc, ObjId, Value};
use crate::vm::Vm;

/// A link to a slot: its index plus one, or zero for none.
type Link = u32;

/// One storage slot of a table.
#[derive(Debug, Clone, Copy)]
struct Slot {
    /// The key, or `Empty` if the slot is free.
    key: Value,
    value: Value,
    /// The next slot in the same bucket, or on the free list.
    next: Link,
}

/// A lookup table.
#[derive(Debug, Clone)]
pub struct LookupTable {
    /// The first slot of each hash bucket.
    buckets: Vec<Link>,
    slots: Vec<Slot>,
    first_free: Link,
    /// What indexing returns for a key that is not present.
    pub default_value: Value,
    /// Whether this is a weak-reference table, whose entries do not keep
    /// their values alive.
    pub weak: bool,
}

impl LookupTable {
    /// An empty table.
    pub fn new(bucket_count: usize, capacity: usize, weak: bool) -> Self {
        let mut table = Self {
            buckets: vec![0; bucket_count.max(1)],
            slots: Vec::new(),
            first_free: 0,
            default_value: Value::Nil,
            weak,
        };
        table.add_slots(capacity.max(1));
        table
    }

    /// Add free slots at the end, linked in order.
    fn add_slots(&mut self, count: usize) {
        let start = self.slots.len();
        for index in start..start + count {
            self.slots.push(Slot {
                key: Value::Empty,
                value: Value::Empty,
                next: if index + 1 < start + count {
                    index as Link + 2
                } else {
                    0
                },
            });
        }
        self.first_free = start as Link + 1;
    }

    /// Read a table from its image data. The keys are hashed afterward by
    /// [`Vm::rehash_table`], since the image's bucket chains follow the
    /// reference VM's hash function.
    pub fn from_image(data: &[u8], weak: bool) -> Option<Self> {
        let u16_at = |at: usize| Some(u16::from_le_bytes([*data.get(at)?, *data.get(at + 1)?]));
        let bucket_count = u16_at(0)? as usize;
        let slot_count = u16_at(2)? as usize;
        let first_free = Link::from(u16_at(4)?);
        let mut pos = 6 + bucket_count * 2;
        let mut slots = Vec::with_capacity(slot_count);
        const SLOT_SIZE: usize = Value::HOLDER_SIZE * 2 + 2;
        for _ in 0..slot_count {
            let record = data.get(pos..pos + SLOT_SIZE)?;
            slots.push(Slot {
                key: Value::from_holder(&record[..Value::HOLDER_SIZE]),
                value: Value::from_holder(&record[Value::HOLDER_SIZE..]),
                next: Link::from(u16_at(pos + Value::HOLDER_SIZE * 2)?),
            });
            pos += SLOT_SIZE;
        }
        let default_value = match data.get(pos..pos + Value::HOLDER_SIZE) {
            Some(holder) => Value::from_holder(holder),
            None => Value::Nil,
        };
        Some(Self {
            buckets: vec![0; bucket_count.max(1)],
            slots,
            first_free,
            default_value,
            weak,
        })
    }

    /// Number of hash buckets.
    pub fn bucket_count(&self) -> usize {
        self.buckets.len()
    }

    /// Number of storage slots, used or not.
    pub fn slot_count(&self) -> usize {
        self.slots.len()
    }

    /// The key and value in a slot, if the slot is in use.
    pub fn slot(&self, index: usize) -> Option<(Value, Value)> {
        let slot = self.slots.get(index)?;
        (slot.key != Value::Empty).then_some((slot.key, slot.value))
    }

    /// The entries in slot order.
    pub fn entries(&self) -> Vec<(Value, Value)> {
        (0..self.slots.len())
            .filter_map(|index| self.slot(index))
            .collect()
    }

    /// Take a free slot, adding more if there are none.
    fn alloc_slot(&mut self) -> usize {
        if self.first_free == 0 {
            let count = self.slots.len();
            self.add_slots((count / 2).max(16));
        }
        let index = self.first_free as usize - 1;
        self.first_free = self.slots[index].next;
        self.slots[index].next = 0;
        index
    }

    /// Put a key that is known to be absent into a bucket.
    fn insert(&mut self, hash: u32, key: Value, value: Value) {
        let bucket = hash as usize % self.buckets.len();
        let index = self.alloc_slot();
        self.slots[index] = Slot {
            key,
            value,
            next: self.buckets[bucket],
        };
        self.buckets[bucket] = index as Link + 1;
    }

    /// Remove the entry in a slot, given its key's hash.
    fn remove(&mut self, hash: u32, index: usize) {
        let bucket = hash as usize % self.buckets.len();
        let link = index as Link + 1;
        if self.buckets[bucket] == link {
            self.buckets[bucket] = self.slots[index].next;
        } else {
            let mut at = self.buckets[bucket];
            while at != 0 {
                let next = self.slots[at as usize - 1].next;
                if next == link {
                    self.slots[at as usize - 1].next = self.slots[index].next;
                    break;
                }
                at = next;
            }
        }
        // A freed slot is the first to be reused.
        self.slots[index] = Slot {
            key: Value::Empty,
            value: Value::Empty,
            next: self.first_free,
        };
        self.first_free = link;
    }

    /// The slots in a hash's bucket.
    fn chain(&self, hash: u32) -> Vec<usize> {
        let mut chain = Vec::new();
        let mut at = self.buckets[hash as usize % self.buckets.len()];
        while at != 0 {
            chain.push(at as usize - 1);
            at = self.slots[at as usize - 1].next;
        }
        chain
    }
}

/// The name and calling interface of each method, by method number.
const METHODS: &[(&str, NativeDesc)] = &[
    ("", NativeDesc::fixed(0)),
    ("isKeyPresent", NativeDesc::fixed(1)),
    ("removeElement", NativeDesc::fixed(1)),
    ("applyAll", NativeDesc::fixed(1)),
    ("forEach", NativeDesc::fixed(1)),
    ("getBucketCount", NativeDesc::fixed(0)),
    ("getEntryCount", NativeDesc::fixed(0)),
    ("forEachAssoc", NativeDesc::fixed(1)),
    ("keysToList", NativeDesc::fixed(0)),
    ("valsToList", NativeDesc::fixed(0)),
    ("getDefaultValue", NativeDesc::fixed(0)),
    ("setDefaultValue", NativeDesc::fixed(1)),
    ("nthKey", NativeDesc::fixed(1)),
    ("nthVal", NativeDesc::fixed(1)),
];

impl Vm {
    /// A hash for a key. Values that compare equal must hash the same,
    /// which is why strings hash by content and lists by their elements.
    fn key_hash(&self, key: Value) -> u32 {
        if let Some(text) = self.as_str(key) {
            return text.iter().fold(0x811c_9dc5u32, |hash, &byte| {
                (hash ^ u32::from(byte)).wrapping_mul(0x0100_0193)
            });
        }
        if let Some(items) = self.as_list(key) {
            return items
                .iter()
                .fold(7u32, |hash, item| hash.wrapping_add(self.key_hash(item)));
        }
        match key {
            Value::Nil => 0,
            Value::True => 1,
            Value::Int(n) => n as u32,
            Value::Enum(n) => n ^ 0x5555,
            Value::Prop(prop) => u32::from(prop) ^ 0xaaaa,
            Value::FuncPtr(address) | Value::CodeOfs(address) => address,
            Value::BifPtr { set, index } | Value::BifPtrX { set, index } => {
                u32::from(set) << 16 | u32::from(index)
            }
            Value::Obj(id) => match self.objects.get(id) {
                // A BigNumber equals an integer of the same value.
                Some(Object::BigNumber(big)) => match big.to_i32() {
                    (n, false) if big.is_frac_zero() => n as u32,
                    // Equal numbers can differ in precision, so zeros at
                    // the end do not count.
                    _ => {
                        let significant =
                            big.digs.iter().rposition(|&d| d != 0).map_or(0, |i| i + 1);
                        big.digs[..significant]
                            .iter()
                            .fold(big.exp as u32, |hash, &d| {
                                hash.wrapping_mul(31).wrapping_add(u32::from(d))
                            })
                    }
                },
                // Character sets are equal if their names are, in any case.
                Some(Object::CharSet { name, .. }) => name.bytes().fold(0u32, |hash, byte| {
                    hash.wrapping_add(byte.to_ascii_lowercase().into())
                }),
                Some(Object::ByteArray(bytes)) => bytes
                    .iter()
                    .fold(0u32, |hash, &byte| hash.wrapping_add(byte.into())),
                // A string buffer equals a string with the same text.
                Some(Object::StringBuffer(_)) => {
                    let text = self.strbuf_text(id).unwrap_or_default();
                    text.iter().fold(0x811c_9dc5u32, |hash, &byte| {
                        (hash ^ u32::from(byte)).wrapping_mul(0x0100_0193)
                    })
                }
                // A vector equals a list with the same elements.
                Some(Object::Vector(items)) => items
                    .iter()
                    .fold(7u32, |hash, &item| hash.wrapping_add(self.key_hash(item))),
                _ => id.0,
            },
            _ => 3,
        }
    }

    fn table(&self, id: ObjId) -> VmResult<&LookupTable> {
        match self.objects.get(id) {
            Some(Object::LookupTable(table)) => Ok(table),
            _ => Err(VmError::new(code::BAD_TYPE_BIF)),
        }
    }

    fn table_mut(&mut self, id: ObjId) -> VmResult<&mut LookupTable> {
        match self.objects.get_mut(id) {
            Some(Object::LookupTable(table)) => Ok(table),
            _ => Err(VmError::new(code::BAD_TYPE_BIF)),
        }
    }

    /// Rebuild a table's hash buckets from its slots. Tables from an
    /// image need this once all objects are loaded.
    pub(crate) fn rehash_table(&mut self, id: ObjId) {
        let Ok(table) = self.table(id) else { return };
        let hashes: Vec<Option<u32>> = (0..table.slot_count())
            .map(|index| table.slot(index).map(|(key, _)| self.key_hash(key)))
            .collect();
        let Ok(table) = self.table_mut(id) else {
            return;
        };
        table.buckets.fill(0);
        for (index, hash) in hashes.into_iter().enumerate() {
            if let Some(hash) = hash {
                let bucket = hash as usize % table.buckets.len();
                table.slots[index].next = table.buckets[bucket];
                table.buckets[bucket] = index as Link + 1;
            }
        }
    }

    /// Find the slot holding a key.
    fn table_find(&mut self, id: ObjId, key: Value) -> VmResult<Option<usize>> {
        let hash = self.key_hash(key);
        let chain = self.table(id)?.chain(hash);
        for index in chain {
            let Some((candidate, _)) = self.table(id)?.slot(index) else {
                continue;
            };
            if self.equals(candidate, key)? {
                return Ok(Some(index));
            }
        }
        Ok(None)
    }

    /// The value for a key, or `None` if the key is absent.
    pub(crate) fn table_get(&mut self, id: ObjId, key: Value) -> VmResult<Option<Value>> {
        Ok(match self.table_find(id, key)? {
            Some(index) => self.table(id)?.slot(index).map(|(_, value)| value),
            None => None,
        })
    }

    /// The value indexing a table with a key gives: the default value if
    /// the key is absent.
    pub(crate) fn table_index(&mut self, id: ObjId, key: Value) -> VmResult<Value> {
        match self.table_get(id, key)? {
            Some(value) => Ok(value),
            None => Ok(self.table(id)?.default_value),
        }
    }

    /// Set the value for a key, adding the key if it is absent.
    pub(crate) fn table_set(&mut self, id: ObjId, key: Value, value: Value) -> VmResult<()> {
        match self.table_find(id, key)? {
            Some(index) => self.table_mut(id)?.slots[index].value = value,
            None => {
                let hash = self.key_hash(key);
                self.table_mut(id)?.insert(hash, key, value);
            }
        }
        Ok(())
    }

    /// Create a table object.
    pub(crate) fn new_table(&mut self, table: LookupTable) -> ObjId {
        self.objects.alloc(Object::LookupTable(Box::new(table)))
    }

    /// Construct a table from the arguments to `new`: nothing, a list of
    /// alternating keys and values, or a bucket count and a capacity.
    pub(crate) fn create_table(&mut self, argc: u32, weak: bool) -> VmResult<ObjId> {
        let (buckets, capacity, source) = match argc {
            0 => (32, 64, Value::Nil),
            1 => {
                let source = self.arg(0);
                let Some(len) = self.list_like_len(source)? else {
                    return Err(VmError::new(code::BAD_VAL_BIF));
                };
                (len * 3 / 2, len * 3 / 2, source)
            }
            2 => (
                self.int_arg(self.arg(0))?.max(0) as usize,
                self.int_arg(self.arg(1))?.max(0) as usize,
                Value::Nil,
            ),
            _ => return Err(VmError::new(code::WRONG_NUM_OF_ARGS)),
        };
        if buckets == 0 || capacity == 0 {
            return Err(VmError::new(code::BAD_VAL_BIF));
        }
        let id = self.new_table(LookupTable::new(buckets, capacity, weak));
        if !source.is_nil() {
            let len = self.list_like_len(source)?.unwrap_or(0);
            let mut index = 1;
            while index < len {
                let key = self.list_like_get(source, index)?;
                let value = self.list_like_get(source, index + 1)?;
                self.table_set(id, key, value)?;
                index += 2;
            }
            // An odd element at the end is the default value.
            if index <= len {
                let default_value = self.list_like_get(source, index)?;
                self.table_mut(id)?.default_value = default_value;
            }
        }
        self.discard(argc as usize);
        Ok(id)
    }

    /// Call a method of the lookup table class.
    pub(crate) fn table_method(
        &mut self,
        id: ObjId,
        index: u16,
        argc: Option<&mut u32>,
    ) -> VmResult<Option<Value>> {
        let Some(&(_, desc)) = METHODS.get(index as usize).filter(|_| index != 0) else {
            return Ok(None);
        };
        self.native_method(argc, desc, |vm, argc| vm.table_call(id, index, argc))
            .map(Some)
    }

    fn table_call(&mut self, id: ObjId, index: u16, argc: u32) -> VmResult<Value> {
        let self_value = Value::Obj(id);
        Ok(match index {
            // isKeyPresent(key)
            1 => Value::logical(self.table_find(id, self.arg(0))?.is_some()),
            // removeElement(key): returns the value removed.
            2 => {
                let key = self.arg(0);
                match self.table_find(id, key)? {
                    Some(slot) => {
                        let hash = self.key_hash(key);
                        let table = self.table_mut(id)?;
                        let value = table.slots[slot].value;
                        table.remove(hash, slot);
                        value
                    }
                    None => Value::Nil,
                }
            }
            // applyAll(func), forEach(func), forEachAssoc(func). The
            // table can change while these run, so each slot is looked at
            // as it is reached.
            3 | 4 | 7 => {
                let name = match index {
                    3 => "LookupTable.applyAll",
                    4 => "LookupTable.forEach",
                    // The reference VM's name for this one.
                    _ => "List.forEachAssoc",
                };
                let native = self.native_call(name, self_value, index, argc);
                let function = self.arg(0);
                let mut slot = 0;
                while slot < self.table(id)?.slot_count() {
                    if let Some((key, value)) = self.table(id)?.slot(slot) {
                        let args: &[Value] = if index == 4 { &[value] } else { &[key, value] };
                        let result = self.call_value(function, args, native.clone())?;
                        if index == 3
                            && let Some(entry) = self.table_mut(id)?.slots.get_mut(slot)
                            && entry.key != Value::Empty
                        {
                            entry.value = result;
                        }
                    }
                    slot += 1;
                }
                Value::Nil
            }
            // getBucketCount()
            5 => Value::Int(self.table(id)?.bucket_count() as i32),
            // getEntryCount()
            6 => Value::Int(self.table(id)?.entries().len() as i32),
            // keysToList() and valsToList()
            8 | 9 => {
                let items: Vec<Value> = self
                    .table(id)?
                    .entries()
                    .into_iter()
                    .map(|(key, value)| if index == 8 { key } else { value })
                    .collect();
                self.new_list(items)
            }
            // getDefaultValue()
            10 => self.table(id)?.default_value,
            // setDefaultValue(val)
            11 => {
                let value = self.arg(0);
                self.table_mut(id)?.default_value = value;
                self_value
            }
            // nthKey(n) and nthVal(n). Entry zero is the default value.
            12 | 13 => {
                let n = self.int_arg(self.arg(0))?;
                let table = self.table(id)?;
                if n == 0 {
                    if index == 12 {
                        Value::Nil
                    } else {
                        table.default_value
                    }
                } else {
                    let entry = usize::try_from(n - 1)
                        .ok()
                        .and_then(|n| table.entries().get(n).copied());
                    match entry {
                        Some((key, value)) => {
                            if index == 12 {
                                key
                            } else {
                                value
                            }
                        }
                        None => return Err(VmError::new(code::INDEX_OUT_OF_RANGE)),
                    }
                }
            }
            _ => return Err(VmError::new(code::BAD_TYPE_BIF)),
        })
    }

    // ---- iterating ----

    /// Create an iterator over a table. Unless it is to be live, the
    /// iterator works on a copy, so later changes do not affect it.
    pub(crate) fn table_iterator(&mut self, id: ObjId, live: bool) -> VmResult<Value> {
        let table = if live {
            id
        } else {
            let copy = self.table(id)?.clone();
            self.new_table(copy)
        };
        Ok(Value::Obj(
            self.objects.alloc(Object::LookupIter { table, slot: 0 }),
        ))
    }

    /// The first slot in use at or after a one-based position, or zero.
    fn table_next_slot(&self, table: ObjId, from: u32) -> u32 {
        let Ok(table) = self.table(table) else {
            return 0;
        };
        (from as usize..=table.slot_count())
            .find(|&slot| slot >= 1 && table.slot(slot - 1).is_some())
            .map_or(0, |slot| slot as u32)
    }

    /// Advance a table iterator. Returns the next value, or `None` at
    /// the end.
    pub(crate) fn table_iter_next(&mut self, id: ObjId) -> VmResult<Option<Value>> {
        let Some(&Object::LookupIter { table, slot }) = self.objects.get(id) else {
            return Ok(None);
        };
        let next = self.table_next_slot(table, slot + 1);
        if next == 0 {
            return Ok(None);
        }
        if let Some(Object::LookupIter { slot, .. }) = self.objects.get_mut(id) {
            *slot = next;
        }
        Ok(self
            .table(table)?
            .slot(next as usize - 1)
            .map(|(_, value)| value))
    }

    /// Call a method of the iterator class on a table iterator.
    pub(crate) fn table_iter_method(&mut self, id: ObjId, index: u16) -> VmResult<Value> {
        let Some(&Object::LookupIter { table, slot }) = self.objects.get(id) else {
            return Err(VmError::new(code::BAD_TYPE_BIF));
        };
        Ok(match index {
            // getNext()
            1 => self
                .table_iter_next(id)?
                .ok_or_else(|| VmError::new(code::OUT_OF_RANGE))?,
            // isNextAvailable()
            2 => Value::logical(self.table_next_slot(table, slot + 1) != 0),
            // resetIterator()
            3 => {
                if let Some(Object::LookupIter { slot, .. }) = self.objects.get_mut(id) {
                    *slot = 0;
                }
                Value::Nil
            }
            // getCurKey() and getCurVal()
            _ => {
                let entry = (slot as usize)
                    .checked_sub(1)
                    .and_then(|slot| self.table(table).ok()?.slot(slot));
                match entry {
                    Some((key, value)) => {
                        if index == 4 {
                            key
                        } else {
                            value
                        }
                    }
                    None => return Err(VmError::new(code::OUT_OF_RANGE)),
                }
            }
        })
    }
}
