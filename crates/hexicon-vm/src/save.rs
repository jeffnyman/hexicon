//! Saving and restoring the state of a program, and restarting it.
//!
//! A saved state holds the objects a program has created or changed
//! since it was loaded. It does not hold the stack: `restoreGame` returns
//! to its caller, in a world where the objects are as they were when
//! `saveGame` ran. The file format is the reference VM's, so that the two
//! can read each other's files for the same image.
//!
//! The layout: a signature; the size and checksum of everything after
//! them; the image's timestamp and file name, which tie the state to the
//! program; descriptive text the program supplied; the table of intrinsic
//! classes; the IDs of the objects the program created; each saved
//! object's data; and the objects and properties the VM itself added to
//! the program's symbols.

use std::collections::HashMap;
use std::rc::Rc;

use hexicon_host::FileError;

use crate::bignum::Big;
use crate::error::{VmError, VmResult, code};
use crate::lookup::LookupTable;
use crate::meta::MetaKind;
use crate::object::{IntrinsicClass, Object};
use crate::tads::TadsObject;
use crate::value::{ObjId, PropId, Value};
use crate::vm::Vm;

const SIGNATURE: &[u8] = b"T3-state-v000A\r\n\x1a";

/// The names under which the objects and properties the VM creates for
/// itself are saved.
const LAST_PROP_OBJ: &[u8] = b"*LastPropObj";
const CONST_STR_OBJ: &[u8] = b"*ConstStrObj";
const CONST_LST_OBJ: &[u8] = b"*ConstLstObj";
const EXCEPTION_MESSAGE: &[u8] = b"exceptionMessage";

/// The checksum of a saved state: CRC-32 as the reference computes it,
/// starting from zero and not inverted at the end.
pub(crate) fn checksum(bytes: &[u8]) -> u32 {
    let mut acc: u32 = 0;
    for &byte in bytes {
        let mut entry = (acc ^ u32::from(byte)) & 0xff;
        for _ in 0..8 {
            entry = if entry & 1 != 0 {
                (entry >> 1) ^ 0xedb8_8320
            } else {
                entry >> 1
            };
        }
        acc = entry ^ (acc >> 8);
    }
    acc
}

/// Bytes being written.
#[derive(Default)]
struct Writer {
    out: Vec<u8>,
}

impl Writer {
    fn u8(&mut self, value: u8) {
        self.out.push(value);
    }

    fn u16(&mut self, value: usize) {
        self.out.extend_from_slice(&(value as u16).to_le_bytes());
    }

    fn u32(&mut self, value: u32) {
        self.out.extend_from_slice(&value.to_le_bytes());
    }

    fn bytes(&mut self, bytes: &[u8]) {
        self.out.extend_from_slice(bytes);
    }

    fn value(&mut self, value: Value) {
        self.out.extend_from_slice(&value.to_holder());
    }

    /// Bytes with their length in front.
    fn counted(&mut self, bytes: &[u8]) {
        self.u16(bytes.len());
        self.bytes(bytes);
    }

    /// Fill in a 32-bit number left blank earlier.
    fn patch_u32(&mut self, at: usize, value: u32) {
        self.out[at..at + 4].copy_from_slice(&value.to_le_bytes());
    }

    fn patch_u16(&mut self, at: usize, value: usize) {
        self.out[at..at + 2].copy_from_slice(&(value as u16).to_le_bytes());
    }
}

/// Bytes being read. Running off the end is a read error.
struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn bytes(&mut self, len: usize) -> VmResult<&'a [u8]> {
        let end = self
            .pos
            .checked_add(len)
            .filter(|&end| end <= self.data.len());
        let Some(end) = end else {
            return Err(VmError::new(code::READ_FILE));
        };
        let bytes = &self.data[self.pos..end];
        self.pos = end;
        Ok(bytes)
    }

    fn u8(&mut self) -> VmResult<u8> {
        Ok(self.bytes(1)?[0])
    }

    fn u16(&mut self) -> VmResult<usize> {
        let b = self.bytes(2)?;
        Ok(usize::from(u16::from_le_bytes([b[0], b[1]])))
    }

    fn u32(&mut self) -> VmResult<u32> {
        let b = self.bytes(4)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    fn counted(&mut self) -> VmResult<&'a [u8]> {
        let len = self.u16()?;
        self.bytes(len)
    }
}

/// The new IDs of the objects in a saved state. An object the program
/// created gets whatever ID is free when the state is restored, so every
/// reference to it has to be changed to match.
struct Fixups {
    /// The new ID for each old one, or `None` for an object that was
    /// transient and so was not saved.
    map: HashMap<u32, Option<ObjId>>,
}

impl Fixups {
    /// The new ID for an old one. An ID that is not in the table belongs
    /// to an object from the image, which keeps its ID.
    fn id(&self, old: ObjId) -> ObjId {
        match self.map.get(&old.0) {
            Some(new) => new.unwrap_or(ObjId::INVALID),
            None => old,
        }
    }

    fn value(&self, value: Value) -> Value {
        match value {
            Value::Obj(id) => Value::obj_or_nil(self.id(id)),
            Value::ObjX(id) => match self.id(id) {
                ObjId::INVALID => Value::Nil,
                id => Value::ObjX(id),
            },
            other => other,
        }
    }

    fn read_value(&self, reader: &mut Reader) -> VmResult<Value> {
        Ok(self.value(Value::from_holder(reader.bytes(Value::HOLDER_SIZE)?)))
    }
}

impl Vm {
    /// Whether an object from the image has to be saved: whether it can
    /// have changed since it was loaded.
    fn changed_since_load(&self, id: ObjId) -> bool {
        match self.objects.get(id) {
            Some(Object::Tads(object) | Object::IntClassMod(object)) => object.modified,
            Some(
                Object::Vector(_)
                | Object::AnonFn(_)
                | Object::LookupTable(_)
                | Object::StringBuffer(_)
                | Object::ByteArray(_)
                | Object::IndexedIter { .. }
                | Object::LookupIter { .. },
            ) => true,
            Some(Object::Dict(dictionary)) => dictionary.modified,
            Some(Object::Class(_)) => self.objects.was_changed(id),
            _ => false,
        }
    }

    /// Whether a kind of lookup table is saved with its default value,
    /// which depends on the version of the class the image was built
    /// for. The two kinds of table have versions of their own.
    fn table_has_default(&self, kind: MetaKind) -> bool {
        self.meta
            .entry(kind)
            .and_then(|entry| entry.image_name.split_once('/'))
            .is_some_and(|(_, version)| version >= "030003")
    }

    /// Write one object's data.
    fn save_object(&self, id: ObjId, w: &mut Writer) -> VmResult<()> {
        let Some(object) = self.objects.get(id) else {
            return Ok(());
        };
        match object {
            Object::Tads(tads) | Object::IntClassMod(tads) => {
                // The superclasses, and the properties that have changed.
                let changed: Vec<_> = tads.props().iter().filter(|p| p.modified).collect();
                w.u16(changed.len());
                w.u16(tads.superclasses.len());
                for superclass in &tads.superclasses {
                    w.u32(superclass.0);
                }
                for entry in changed {
                    w.u16(entry.prop.into());
                    w.value(entry.value);
                }
            }
            Object::String(text) => w.counted(text),
            Object::List(items) => {
                w.u16(items.len());
                for &item in items.iter() {
                    w.value(item);
                }
            }
            Object::Vector(items) | Object::AnonFn(items) => {
                // The space allocated, then the number in use.
                w.u16(items.len().max(1));
                w.u16(items.len());
                for &item in items {
                    w.value(item);
                }
            }
            Object::IndexedIter {
                collection,
                current,
                first,
                last,
            } => {
                w.value(*collection);
                w.u32(*current as u32);
                w.u32(*first as u32);
                w.u32(*last as u32);
                w.u32(0);
            }
            Object::LookupTable(table) => {
                let (buckets, slots, first_free) = table.save_parts();
                w.u16(buckets.len());
                w.u16(slots.len());
                w.u16(first_free);
                let kind = if table.weak {
                    MetaKind::WeakRefLookupTable
                } else {
                    MetaKind::LookupTable
                };
                if self.table_has_default(kind) {
                    w.value(table.default_value);
                }
                for bucket in buckets {
                    w.u16(bucket);
                }
                for (key, value, next) in slots {
                    w.value(key);
                    w.value(value);
                    w.u16(next);
                }
            }
            Object::LookupIter { table, slot } => {
                w.value(Value::Obj(*table));
                w.u16(*slot as usize);
                w.u16(0);
            }
            Object::CharSet { name, .. } => w.counted(name.as_bytes()),
            Object::FileName(name) => {
                w.u32(name.special as u32);
                if name.special == 0 {
                    w.counted(name.path.as_bytes());
                }
            }
            // These are transient, so they are never saved.
            Object::File(_) | Object::TempFile(_) => {}
            Object::StrComp(comparator) => w.bytes(&comparator.to_bytes()),
            // A production from the image is never saved, so one that is
            // being saved is one the program made.
            Object::GramProd(production) => w.bytes(&production.to_bytes()),
            Object::Dict(dictionary) => {
                // The comparator, then each word with its entries for
                // objects that are saved too.
                w.u32(dictionary.comparator.0);
                let at = w.out.len();
                w.u32(0);
                let mut count = 0;
                for word in dictionary.words() {
                    let entries: Vec<_> = word
                        .entries
                        .iter()
                        .filter(|&&(object, _)| {
                            !self.objects.is_transient(object)
                                && (self.objects.is_in_root_set(object)
                                    || self.objects.is_marked(object))
                        })
                        .collect();
                    if entries.is_empty() {
                        continue;
                    }
                    count += 1;
                    w.counted(&word.text);
                    w.u16(entries.len());
                    for &&(object, prop) in &entries {
                        w.u32(object.0);
                        w.u16(prop.into());
                    }
                }
                w.patch_u32(at, count);
            }
            Object::ByteArray(bytes) => {
                w.u32(bytes.len() as u32);
                w.bytes(bytes);
            }
            Object::BigNumber(big) => w.bytes(&big.to_bytes()),
            Object::StringBuffer(buffer) => {
                // The space allocated, the amount to grow by, the length,
                // and the text in UTF-16.
                w.u32(buffer.len().max(1) as u32);
                w.u32(16);
                w.u32(buffer.len() as u32);
                for &c in buffer {
                    w.u16(u32::from(c) as usize);
                }
            }
            Object::Pattern { source, .. } => w.value(*source),
            Object::Class(class) => {
                w.u16(2 + 2 + 4 + Value::HOLDER_SIZE);
                w.u16(class.meta_index.into());
                w.u32(class.modifier.0);
                w.value(class.class_state);
            }
            Object::Opaque { meta_index, .. } => {
                let name = self
                    .meta
                    .get(usize::from(*meta_index))
                    .map_or("unknown", |entry| entry.kind.name());
                return Err(VmError::unimplemented(format!(
                    "saving an object of intrinsic class {name}"
                )));
            }
        }
        Ok(())
    }

    /// The state of the program as a saved state file. `metadata` is the
    /// table of descriptive text the program passed to `saveGame`, or
    /// nil.
    pub(crate) fn save_state(&mut self, metadata: Value) -> VmResult<Vec<u8>> {
        let mut w = Writer::default();
        w.bytes(SIGNATURE);
        let start = w.out.len();
        w.u32(0);
        w.u32(0);
        let mut timestamp = self.image_timestamp.clone();
        timestamp.resize(24, 0);
        w.bytes(&timestamp);
        w.counted(self.image_name.clone().as_bytes());

        // The program's description of the state: pairs of strings.
        match metadata {
            Value::Obj(id) => {
                let entries = match self.objects.get(id) {
                    Some(Object::LookupTable(table)) => table.entries(),
                    _ => return Err(VmError::new(code::BAD_TYPE_BIF)),
                };
                let at = w.out.len();
                w.u16(0);
                w.u16(0);
                let mut count = 0;
                for (key, value) in entries {
                    if let (Some(key), Some(value)) = (self.as_str(key), self.as_str(value)) {
                        w.counted(&key);
                        w.counted(&value);
                        count += 1;
                    }
                }
                let size = w.out.len() - at;
                if size > 0xffff {
                    return Err(VmError::new(code::DESC_TAB_OVERFLOW));
                }
                w.patch_u16(at, size - 2);
                w.patch_u16(at + 2, count);
            }
            _ => w.u16(0),
        }

        // Only what a restored program could still reach is worth saving.
        self.collect_garbage(false);

        // Find what will be saved. A class the image does not list can
        // still have instances, which the table of classes has to cover.
        let roots = self.gc_global_roots();
        self.objects.mark_persistent(&roots);
        let ids: Vec<ObjId> = self.objects.ids().collect();
        for &id in &ids {
            if self.objects.is_marked(id) {
                if let Some(kind) = self.kind_of(id) {
                    self.meta.add_if_new(kind);
                }
            }
        }

        // The intrinsic classes, as the image numbers them.
        w.u16(self.meta.len());
        for index in 0..self.meta.len() {
            let Some(entry) = self.meta.get(index) else {
                continue;
            };
            w.counted(entry.image_name.as_bytes());
            w.u32(entry.class_obj.0);
            w.u16(entry.method_props.len());
            let (first, count) = entry.prop_range();
            w.u16(first.into());
            w.u16(usize::from(first) + count);
            for &prop in &entry.method_props {
                w.u16(prop.into());
            }
        }

        // Every object the program created that is still reachable, so
        // that each can be given a new ID when the state is restored.
        // Reaching an object through a transient one does not count:
        // transient objects are not saved.
        let at = w.out.len();
        w.u32(0);
        let mut listed = 0;
        let mut to_save = Vec::new();
        for &id in &ids {
            let root = self.objects.is_in_root_set(id);
            let transient = self.objects.is_transient(id);
            let reachable = self.objects.is_marked(id);
            if reachable && !root {
                w.u32(id.0);
                w.u32(u32::from(transient));
                listed += 1;
            }
            if reachable && !transient && (!root || self.changed_since_load(id)) {
                to_save.push(id);
            }
        }
        w.patch_u32(at, listed);

        // The objects themselves.
        w.u32(to_save.len() as u32);
        for id in to_save {
            w.u32(id.0);
            w.u8(u8::from(self.objects.is_in_root_set(id)));
            let kind = self.kind_of(id);
            let index = match self.objects.get(id) {
                Some(Object::Opaque { meta_index, .. }) => Some(usize::from(*meta_index)),
                _ => kind.and_then(|kind| self.meta.index_of(kind)),
            };
            w.u8(index.unwrap_or(0) as u8);
            self.save_object(id, &mut w)?;
        }

        // What the VM added to the program's symbols.
        let mut symbols: Vec<(&[u8], Value)> = vec![
            (LAST_PROP_OBJ, Value::Obj(self.predef.last_prop_obj)),
            (CONST_STR_OBJ, Value::Obj(self.predef.const_str_obj)),
            (CONST_LST_OBJ, Value::Obj(self.predef.const_lst_obj)),
        ];
        if self.exception_message_synthesized {
            symbols.push((
                EXCEPTION_MESSAGE,
                Value::Prop(self.predef.exception_message),
            ));
        }
        w.u32(symbols.len() as u32);
        for (name, value) in symbols {
            w.counted(name);
            w.value(value);
        }

        let size = (w.out.len() - start - 8) as u32;
        let crc = checksum(&w.out[start + 8..]);
        w.patch_u32(start, size);
        w.patch_u32(start + 4, crc);
        Ok(w.out)
    }

    /// Read one object's data into an object. `existing` is the object
    /// from the image, for an object that came from there.
    fn restore_object(
        &self,
        kind: MetaKind,
        meta_index: u16,
        existing: Option<&Object>,
        r: &mut Reader,
        fixups: &Fixups,
    ) -> VmResult<Object> {
        let bad = || VmError::new(code::BAD_SAVED_META_DATA);
        Ok(match kind {
            MetaKind::TadsObject | MetaKind::IntClassMod => {
                let changed = r.u16()?;
                let superclass_count = r.u16()?;
                let mut tads = match existing {
                    Some(Object::Tads(tads) | Object::IntClassMod(tads)) => tads.clone(),
                    _ => TadsObject::default(),
                };
                let mut superclasses = Vec::with_capacity(superclass_count);
                for _ in 0..superclass_count {
                    superclasses.push(fixups.id(ObjId(r.u32()?)));
                }
                tads.set_superclasses(superclasses);
                for _ in 0..changed {
                    let prop = r.u16()? as PropId;
                    let value = fixups.read_value(r)?;
                    tads.set(prop, value);
                }
                tads.modified = true;
                if kind == MetaKind::TadsObject {
                    Object::Tads(tads)
                } else {
                    Object::IntClassMod(tads)
                }
            }
            MetaKind::String => Object::String(r.counted()?.into()),
            MetaKind::List => {
                let count = r.u16()?;
                let mut items = Vec::with_capacity(count);
                for _ in 0..count {
                    items.push(fixups.read_value(r)?);
                }
                Object::List(items.into())
            }
            MetaKind::Vector | MetaKind::AnonFuncPtr => {
                let _allocated = r.u16()?;
                let count = r.u16()?;
                let mut items = Vec::with_capacity(count);
                for _ in 0..count {
                    items.push(fixups.read_value(r)?);
                }
                if kind == MetaKind::Vector {
                    Object::Vector(items)
                } else {
                    Object::AnonFn(items)
                }
            }
            MetaKind::IndexedIterator => {
                let collection = fixups.read_value(r)?;
                let current = r.u32()? as i32;
                let first = r.u32()? as i32;
                let last = r.u32()? as i32;
                r.u32()?;
                Object::IndexedIter {
                    collection,
                    current,
                    first,
                    last,
                }
            }
            MetaKind::LookupTable | MetaKind::WeakRefLookupTable => {
                let bucket_count = r.u16()?;
                let slot_count = r.u16()?;
                let first_free = r.u16()?;
                let default_value = if self.table_has_default(kind) {
                    fixups.read_value(r)?
                } else {
                    Value::Nil
                };
                let mut buckets = Vec::with_capacity(bucket_count);
                for _ in 0..bucket_count {
                    buckets.push(r.u16()?);
                }
                let mut slots = Vec::with_capacity(slot_count);
                for _ in 0..slot_count {
                    let key = fixups.read_value(r)?;
                    let value = fixups.read_value(r)?;
                    slots.push((key, value, r.u16()?));
                }
                Object::LookupTable(Box::new(LookupTable::from_parts(
                    buckets,
                    slots,
                    first_free,
                    default_value,
                    kind == MetaKind::WeakRefLookupTable,
                )))
            }
            MetaKind::LookupTableIterator => {
                let table = fixups.read_value(r)?;
                let slot = r.u16()? as u32;
                r.u16()?;
                Object::LookupIter {
                    table: table.as_obj().unwrap_or(ObjId::INVALID),
                    slot,
                }
            }
            MetaKind::CharacterSet => {
                let name = String::from_utf8_lossy(r.counted()?).into_owned();
                let map = crate::charset::CharMap::load(&name).map(Rc::new);
                Object::CharSet { name, map }
            }
            MetaKind::FileName => {
                let special = r.u32()? as i32;
                let path = if special == 0 {
                    String::from_utf8_lossy(r.counted()?).into_owned()
                } else {
                    String::new()
                };
                Object::FileName(Box::new(crate::file::FileNameObj {
                    path,
                    special,
                    from_ui: 0,
                }))
            }
            MetaKind::GrammarProduction => {
                let (mut production, used) =
                    crate::gram::GramProd::from_bytes(&r.data[r.pos..]).ok_or_else(bad)?;
                r.bytes(used)?;
                production.fix_ids(&|id| fixups.id(id));
                production.modified = true;
                Object::GramProd(Box::new(production))
            }
            MetaKind::StringComparator => {
                let (comparator, used) =
                    crate::strcomp::StrComp::from_bytes(&r.data[r.pos..]).ok_or_else(bad)?;
                r.bytes(used)?;
                Object::StrComp(Rc::new(comparator))
            }
            MetaKind::Dictionary => {
                // The words go in by their plain hash, and are moved for
                // the comparator once every object is back.
                let comparator = fixups.id(ObjId(r.u32()?));
                let mut dictionary = crate::dict::Dictionary::new(ObjId::INVALID);
                let plain = |text: &[u8]| text.iter().fold(0u32, |h, &b| h.wrapping_add(b.into()));
                let count = r.u32()?;
                for _ in 0..count {
                    let text = r.counted()?;
                    let entries = r.u16()?;
                    for _ in 0..entries {
                        let object = fixups.id(ObjId(r.u32()?));
                        let prop = r.u16()? as PropId;
                        if object.is_valid() {
                            dictionary.add(plain(text), text, object, prop);
                        }
                    }
                }
                dictionary.comparator = comparator;
                dictionary.modified = true;
                Object::Dict(Box::new(dictionary))
            }
            MetaKind::ByteArray => {
                let len = r.u32()? as usize;
                Object::ByteArray(r.bytes(len)?.to_vec())
            }
            MetaKind::BigNumber => {
                // The length depends on the precision, which comes first.
                let start = r.pos;
                let prec = r.u16()?;
                r.pos = start;
                let data = r.bytes(5 + prec.div_ceil(2))?;
                Object::BigNumber(Box::new(Big::from_bytes(data).ok_or_else(bad)?))
            }
            MetaKind::StringBuffer => {
                let allocated = r.u32()?;
                r.u32()?;
                let len = r.u32()?.min(allocated) as usize;
                let mut buffer = Vec::with_capacity(len);
                for _ in 0..len {
                    let unit = r.u16()? as u32;
                    buffer.push(char::from_u32(unit).unwrap_or('\u{fffd}'));
                }
                Object::StringBuffer(buffer)
            }
            MetaKind::RexPattern => Object::Pattern {
                source: fixups.read_value(r)?,
                compiled: None,
            },
            MetaKind::IntrinsicClass => {
                let len = r.u16()?;
                let expected = 2 + 2 + 4 + Value::HOLDER_SIZE;
                let meta_index = r.u16()? as u16;
                let modifier = ObjId(r.u32()?);
                let class_state = if len >= expected {
                    fixups.read_value(r)?
                } else {
                    Value::Nil
                };
                if len > expected {
                    r.bytes(len - expected)?;
                }
                Object::Class(IntrinsicClass {
                    meta_index,
                    modifier,
                    class_state,
                })
            }
            other => {
                let _ = meta_index;
                return Err(VmError::unimplemented(format!(
                    "restoring an object of intrinsic class {}",
                    other.name()
                )));
            }
        })
    }

    /// Put every object from the image back as it was loaded, and forget
    /// what depends on the objects the program created.
    fn reset_to_image(&mut self) {
        self.objects.drop_undo();
        self.objects.reset_to_image();
        // The intrinsic class objects the image defines are the ones in
        // force again.
        for index in 0..self.meta.len() {
            if let Some(entry) = self.meta.get_mut(index) {
                entry.class_obj = ObjId::INVALID;
            }
        }
        let ids: Vec<ObjId> = self.objects.ids().collect();
        for id in ids {
            if !self.objects.is_in_root_set(id) {
                continue;
            }
            if let Some(Object::Class(class)) = self.objects.get(id) {
                let index = usize::from(class.meta_index);
                if let Some(entry) = self.meta.get_mut(index) {
                    entry.class_obj = id;
                }
            }
        }
    }

    /// Replace the state of the program with a saved one.
    pub(crate) fn restore_state(&mut self, data: &[u8]) -> VmResult<()> {
        let mut r = Reader { data, pos: 0 };
        let signature = r.bytes(SIGNATURE.len());
        if signature.ok() != Some(SIGNATURE) {
            return Err(VmError::new(code::NOT_SAVED_STATE));
        }
        let size = r.u32()? as usize;
        let crc = r.u32()?;
        let body = data.get(r.pos..r.pos + size);
        if body.map(checksum) != Some(crc) {
            return Err(VmError::new(code::BAD_SAVED_STATE));
        }
        let mut timestamp = self.image_timestamp.clone();
        timestamp.resize(24, 0);
        if r.bytes(24)? != timestamp.as_slice() {
            return Err(VmError::new(code::WRONG_SAVED_STATE));
        }
        // The image's file name and the descriptive text are for whoever
        // is choosing a file, not for restoring it.
        r.counted()?;
        r.counted()?;

        // The intrinsic class table. The state is for this image, so the
        // classes are the ones already here, but the state numbers them
        // its own way, and can have some the image does not list.
        let class_count = r.u16()?;
        let mut classes: Vec<Option<usize>> = Vec::with_capacity(class_count);
        for _ in 0..class_count {
            let name = r.counted()?;
            if name.len() > 255 {
                return Err(VmError::new(code::SAVED_META_TOO_LONG));
            }
            r.u32()?;
            let method_count = r.u16()?;
            r.bytes(4 + method_count * 2)?;
            let name = String::from_utf8_lossy(name).into_owned();
            let stem = |full: &str| full.split('/').next().unwrap_or("").to_owned();
            let find = |vm: &Vm| {
                (0..vm.meta.len()).find(|&index| {
                    vm.meta
                        .get(index)
                        .is_some_and(|entry| stem(&entry.image_name) == stem(&name))
                })
            };
            let mut found = find(self);
            if found.is_none() && self.meta.add(&name, Vec::new()).is_ok() {
                found = find(self);
            }
            classes.push(found);
        }

        self.reset_to_image();

        // A new ID for each object the program had created.
        let mut fixups = Fixups {
            map: HashMap::new(),
        };
        let listed = r.u32()?;
        for _ in 0..listed {
            let old = r.u32()?;
            let transient = r.u32()? & 1 != 0;
            let new = if transient {
                None
            } else {
                Some(self.objects.alloc(Object::List(Rc::from(&[][..]))))
            };
            fixups.map.insert(old, new);
        }

        // The objects.
        let mut dictionaries = Vec::new();
        let saved = r.u32()?;
        for _ in 0..saved {
            let old = ObjId(r.u32()?);
            let from_image = r.u8()? != 0;
            let saved_index = usize::from(r.u8()?);
            let Some(meta_index) = classes.get(saved_index).copied().flatten() else {
                return Err(VmError::new(code::BAD_SAVED_META_DATA));
            };
            let meta_index = meta_index as u16;
            let id = if from_image {
                if self.objects.get(old).is_none() {
                    return Err(VmError::new(code::SAVED_OBJ_ID_INVALID));
                }
                old
            } else {
                fixups.id(old)
            };
            let Some(kind) = self.meta.get(usize::from(meta_index)).map(|e| e.kind) else {
                return Err(VmError::new(code::BAD_SAVED_META_DATA));
            };
            let existing = if from_image {
                self.objects.get(id)
            } else {
                None
            };
            let mut object = self.restore_object(kind, meta_index, existing, &mut r, &fixups)?;
            if matches!(object, Object::Dict(_)) {
                dictionaries.push(id);
            }
            if let Object::Class(class) = &mut object {
                // The class is named by its number in the saved table.
                let local = classes
                    .get(usize::from(class.meta_index))
                    .copied()
                    .flatten();
                class.meta_index = local.unwrap_or(usize::from(u16::MAX)) as u16;
                if let Some(entry) = self.meta.get_mut(usize::from(class.meta_index)) {
                    entry.class_obj = id;
                }
            }
            if id.is_valid() {
                if let Some(slot) = self.objects.get_mut(id) {
                    *slot = object;
                }
            }
        }

        // What the VM had added to the program's symbols.
        let symbols = r.u32()?;
        for _ in 0..symbols {
            let name = r.counted()?;
            let value = fixups.read_value(&mut r)?;
            match (name, value) {
                (LAST_PROP_OBJ, Value::Obj(id)) => self.predef.last_prop_obj = id,
                (CONST_STR_OBJ, Value::Obj(id)) => self.predef.const_str_obj = id,
                (CONST_LST_OBJ, Value::Obj(id)) => self.predef.const_lst_obj = id,
                (EXCEPTION_MESSAGE, Value::Prop(prop)) => self.predef.exception_message = prop,
                _ => {}
            }
        }
        self.create_class_objects();
        for id in dictionaries {
            self.dict_rehash(id)?;
        }
        self.last_match = Default::default();
        self.collect_garbage(false);
        Ok(())
    }

    /// Return the program to the state it was loaded in.
    pub(crate) fn restart_state(&mut self) -> VmResult<()> {
        self.reset_to_image();
        self.create_synthesized_objects()?;
        self.create_class_objects();
        self.last_match = Default::default();
        self.collect_garbage(false);
        self.run_static_initializers()
    }

    // ---- the functions a program calls ----

    /// `saveGame(file, metadata?)`.
    pub(crate) fn bif_save(&mut self, argc: u32) -> VmResult<()> {
        if !(1..=2).contains(&argc) {
            return Err(VmError::new(code::WRONG_NUM_OF_ARGS));
        }
        let metadata = if argc >= 2 { self.arg(1) } else { Value::Nil };
        if !matches!(metadata, Value::Nil | Value::Obj(_)) {
            return Err(VmError::new(code::BAD_TYPE_BIF));
        }
        let name = self.file_name_arg(self.arg(0), crate::file::Access::Write)?;
        let state = self.save_state(metadata)?;
        self.host
            .write_file(&name, &state)
            .map_err(|_| VmError::new(code::CREATE_FILE))?;
        self.r0 = Value::Nil;
        Ok(())
    }

    /// `restoreGame(file)`.
    pub(crate) fn bif_restore(&mut self, argc: u32) -> VmResult<()> {
        if argc != 1 {
            return Err(VmError::new(code::WRONG_NUM_OF_ARGS));
        }
        let name = self.file_name_arg(self.arg(0), crate::file::Access::Read)?;
        self.sync_files();
        let state = self.host.read_file(&name).map_err(|error| {
            VmError::new(match error {
                FileError::NotFound => code::FILE_NOT_FOUND,
                _ => code::READ_FILE,
            })
        })?;
        self.restore_state(&state)?;
        self.r0 = Value::Nil;
        Ok(())
    }

    /// `restartGame()`.
    pub(crate) fn bif_restart(&mut self, argc: u32) -> VmResult<()> {
        if argc != 0 {
            return Err(VmError::new(code::WRONG_NUM_OF_ARGS));
        }
        self.restart_state()?;
        self.r0 = Value::Nil;
        Ok(())
    }
}
