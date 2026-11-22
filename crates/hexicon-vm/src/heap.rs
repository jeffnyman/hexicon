//! What the reference VM counts toward its next collection besides
//! objects: the bytes its heap hands out for their contents.
//!
//! The reference collects before creating an object once 6 MB have been
//! allocated since the last collection, as well as after 10,000 objects.
//! It counts the sizes it asks its heap for, in its own layouts in
//! memory, which depend on the machine. The sizes here are the ones the
//! reference built for 64-bit Windows asks for: a program that makes few,
//! large objects collects when it would there, and so its objects get
//! the same numbers.
//!
//! Each object is counted when it is created, and again when it grows as
//! the reference's would: a program-defined object's property table, a
//! vector's room, a lookup table's slots, a string buffer's room. The
//! count is kept for the thread, as only one VM runs on a thread at a
//! time; a VM resets it when it starts and at each collection.

use std::cell::Cell;

use crate::object::{Object, StrBuf};

/// The bytes after which the reference collects.
pub(crate) const BYTES_BETWEEN_GC: i64 = 6 * 1024 * 1024;

thread_local! {
    static SINCE_GC: Cell<i64> = const { Cell::new(0) };
}

/// Count bytes the reference would allocate, for an object of this kind
/// (its metaclass name, for tracing). A negative count takes back part of
/// an allocation counted too large.
pub(crate) fn count(bytes: i64, kind: &str) {
    if bytes == 0 {
        return;
    }
    if crate::object::trace_alloc() {
        eprintln!("M {bytes} {kind}");
    }
    SINCE_GC.with(|since| since.set(since.get() + bytes));
}

/// Correct the count just made for a new object, which is traced as part
/// of it.
pub(crate) fn adjust(bytes: i64, kind: &str) {
    if bytes == 0 {
        return;
    }
    if crate::object::trace_alloc() {
        eprintln!("M+ {bytes} {kind}");
    }
    SINCE_GC.with(|since| since.set(since.get() + bytes));
}

/// The size of the cells the reference's heap hands out blocks of this
/// size in, or zero for a block too big for a cell, which comes from the
/// system's heap.
pub(crate) fn cell_for(bytes: i64) -> u16 {
    [32, 64, 128, 256, 512]
        .into_iter()
        .find(|&cell| bytes <= i64::from(cell))
        .unwrap_or(0)
}

/// Count a block growing to so many bytes, in a cell of the size given,
/// which it moves out of if it no longer fits. The reference counts the
/// new size, and counts it again when it takes a new block.
pub(crate) fn realloc(cell: &mut u16, bytes: i64, kind: &str) {
    count(bytes, kind);
    if *cell != 0 && bytes > i64::from(*cell) {
        count(bytes, kind);
        *cell = cell_for(bytes);
    }
}

/// The bytes counted since the last collection.
pub(crate) fn since_gc() -> i64 {
    SINCE_GC.with(Cell::get)
}

/// Start counting again, at a collection or when a VM starts.
pub(crate) fn reset() {
    SINCE_GC.with(|since| since.set(0));
}

/// A program-defined object's header: the fixed part, an entry for each
/// superclass, a slot for each hash bucket, and an entry for each
/// property there is room for.
pub(crate) fn tads_bytes(superclasses: usize, hash: usize, room: usize) -> i64 {
    (40 + 16 * superclasses + 8 * hash + 40 * room) as i64
}

/// A vector's elements with room for so many: two counts, the elements,
/// and a byte for each, rounded up to eight.
pub(crate) fn vector_bytes(room: usize) -> i64 {
    (4 + 5 * room + ((room + 7) & !7)) as i64
}

/// A lookup table with so many buckets and slots.
pub(crate) fn lookup_bytes(buckets: usize, slots: usize) -> i64 {
    (32 + 8 * buckets + 40 * slots) as i64
}

/// A string buffer with room for so many characters.
pub(crate) fn strbuf_bytes(buffer: &StrBuf) -> i64 {
    16 + 2 * (i64::from(buffer.room) - 1)
}

/// The bytes the reference allocates when it creates an object like this
/// one, with room for `vector_room` elements if it is a vector.
pub(crate) fn object_bytes(object: &Object, vector_room: usize) -> i64 {
    match object {
        Object::Tads(tads) | Object::IntClassMod(tads) => tads.heap_bytes(),
        Object::String(text) => text.len() as i64 + 2,
        Object::List(items) => 2 + 5 * items.len() as i64,
        Object::Vector(_) | Object::AnonFn(_) => vector_bytes(vector_room),
        Object::IndexedIter { .. } => 21,
        Object::LookupIter { .. } => 9,
        Object::LookupTable(table) => lookup_bytes(table.bucket_count(), table.slot_count()),
        Object::Pattern { .. } => 24,
        Object::StringBuffer(buffer) => strbuf_bytes(buffer),
        Object::BigNumber(big) => 5 + (big.digs.len() as i64 + 1) / 2,
        // A count, and a pointer to each page of 256 MB.
        Object::ByteArray(bytes) => 4 + 8 * bytes.len().div_ceil(1 << 28) as i64,
        Object::CharSet { name, .. } => 29 + name.len() as i64,
        Object::StrComp(comparator) => comparator.heap_bytes(),
        Object::GramProd(_) => 72,
        Object::Dict(_) => 56,
        Object::Class(_) => 32,
        Object::File(_) => 40,
        Object::Date { .. } | Object::TimeZone(_) => 8,
        Object::FileName(name) => 18 + name.path.len() as i64,
        Object::TempFile(name) => 16 + name.len() as i64,
        // The fixed part, a number for each object referred to, the
        // object's number before the code, and the code.
        Object::DynFunc(func) => {
            48 + 2 * (func.obj_refs.len() as i64 - 1).max(0) + 4 + func.code.len() as i64
        }
        Object::Opaque { .. } => 0,
    }
}
