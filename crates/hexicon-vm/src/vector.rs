//! The vector intrinsic class: a list that changes in place.
//!
//! Most methods that would return a new list for a list modify the vector
//! and return it.

use std::cmp::Ordering;

use crate::error::{VmError, VmResult, code};
use crate::list::{is_true, seq_range};
use crate::meta::MetaKind;
use crate::object::Object;
use crate::value::{NativeDesc, ObjId, Value};
use crate::vm::Vm;

/// The name and calling interface of each vector method, by method number.
const METHODS: &[(&str, NativeDesc)] = &[
    ("", NativeDesc::fixed(0)),
    ("toList", NativeDesc::optional(0, 2)),
    ("length", NativeDesc::fixed(0)),
    ("copyFrom", NativeDesc::fixed(4)),
    ("fillValue", NativeDesc::optional(1, 2)),
    ("subset", NativeDesc::fixed(1)),
    ("applyAll", NativeDesc::fixed(1)),
    ("indexWhich", NativeDesc::fixed(1)),
    ("forEach", NativeDesc::fixed(1)),
    ("forEachAssoc", NativeDesc::fixed(1)),
    ("mapAll", NativeDesc::fixed(1)),
    ("indexOf", NativeDesc::fixed(1)),
    ("valWhich", NativeDesc::fixed(1)),
    ("lastIndexOf", NativeDesc::fixed(1)),
    ("lastIndexWhich", NativeDesc::fixed(1)),
    ("lastValWhich", NativeDesc::fixed(1)),
    ("countOf", NativeDesc::fixed(1)),
    ("countWhich", NativeDesc::fixed(1)),
    ("getUnique", NativeDesc::fixed(0)),
    ("appendUnique", NativeDesc::fixed(1)),
    ("sort", NativeDesc::optional(0, 2)),
    ("setLength", NativeDesc::fixed(1)),
    ("insertAt", NativeDesc::variable(2)),
    ("removeElementAt", NativeDesc::fixed(1)),
    ("removeRange", NativeDesc::fixed(2)),
    ("append", NativeDesc::fixed(1)),
    ("prepend", NativeDesc::fixed(1)),
    ("appendAll", NativeDesc::fixed(1)),
    ("removeElement", NativeDesc::fixed(1)),
    ("splice", NativeDesc::variable(2)),
    ("join", NativeDesc::optional(0, 1)),
    // `generate` is a static method; on an instance it is undefined.
    ("generate", NativeDesc::fixed(2)),
    ("indexOfMin", NativeDesc::optional(0, 1)),
    ("minVal", NativeDesc::optional(0, 1)),
    ("indexOfMax", NativeDesc::optional(0, 1)),
    ("maxVal", NativeDesc::optional(0, 1)),
];

const GENERATE: u16 = 31;

/// The most elements a vector can hold.
const MAX_ELEMENTS: i32 = 65535;

/// The names stack traces show for the methods that call back.
fn trace_name(index: u16) -> &'static str {
    match index {
        5 => "Vector.subset",
        6 => "Vector.applyAll",
        7 | 12 => "Vector.indexWhich",
        8 => "Vector.forEach",
        9 => "Vector.forEachAssoc",
        10 => "Vector.mapAll",
        14 | 15 => "Vector.lastIndexWhich",
        17 => "Vector.countWhich",
        20 => "Vector.sort",
        32 => "Vector.indexOfMin",
        33 => "Vector.minVal",
        34 => "Vector.indexOfMax",
        35 => "Vector.maxVal",
        _ => "Vector",
    }
}

impl Vm {
    /// The elements of a vector.
    fn vec_items(&self, id: ObjId) -> VmResult<&Vec<Value>> {
        match self.objects.get(id) {
            Some(Object::Vector(items) | Object::AnonFn(items)) => Ok(items),
            _ => Err(VmError::new(code::BAD_TYPE_BIF)),
        }
    }

    /// The elements of a vector, for modification.
    fn vec_items_mut(&mut self, id: ObjId) -> VmResult<&mut Vec<Value>> {
        match self.objects.get_mut(id) {
            Some(Object::Vector(items) | Object::AnonFn(items)) => Ok(items),
            _ => Err(VmError::new(code::BAD_TYPE_BIF)),
        }
    }

    fn new_vector(&mut self, items: Vec<Value>) -> Value {
        Value::Obj(self.alloc_obj(Object::Vector(items)))
    }

    /// Call a method of the vector class.
    pub(crate) fn vector_method(
        &mut self,
        id: ObjId,
        index: u16,
        argc: Option<&mut u32>,
    ) -> VmResult<Option<Value>> {
        if index == 0 || index == GENERATE {
            return Ok(None);
        }
        let Some(&(_, desc)) = METHODS.get(index as usize) else {
            return Ok(None);
        };
        self.native_method(argc, desc, |vm, argc| vm.vector_call(id, index, argc))
            .map(Some)
    }

    /// The static methods of the vector class.
    pub(crate) fn vector_static_method(
        &mut self,
        index: u16,
        argc: Option<&mut u32>,
    ) -> VmResult<Option<Value>> {
        if index != GENERATE {
            return Ok(None);
        }
        // Vector.generate(func, n)
        self.native_method(argc, NativeDesc::fixed(2), |vm, argc| {
            let class = Value::obj_or_nil(vm.meta.class_obj(MetaKind::Vector));
            let native = vm.native_call("Vector.generate", class, GENERATE, argc);
            let items = vm.generate(vm.arg(0), vm.arg(1), &native)?;
            Ok(vm.new_vector(items))
        })
        .map(Some)
    }

    fn vector_call(&mut self, id: ObjId, index: u16, argc: u32) -> VmResult<Value> {
        let result = self.vector_call_inner(id, index, argc);
        // A method that made the vector longer may have given it more
        // room.
        self.objects.note_vector(id);
        result
    }

    /// A new vector of these elements, with room for so many.
    fn new_vector_with_room(&mut self, items: Vec<Value>, room: usize) -> Value {
        let value = self.new_vector(items);
        let room = room.max(self.objects.vector_alloc(value.obj_id()));
        self.objects.set_vector_alloc(value.obj_id(), room);
        value
    }

    fn vector_call_inner(&mut self, id: ObjId, index: u16, argc: u32) -> VmResult<Value> {
        let self_value = Value::Obj(id);
        let len = self.vec_items(id)?.len();
        // Methods that report under another method's name in stack traces
        // also report that method's number.
        let trace_index = match index {
            12 => 7,
            15 => 14,
            other => other,
        };
        let native = self.native_call(trace_name(index), self_value, trace_index, argc);
        // A one-based index argument that counts from the end if negative.
        let from_end = |n: i32, len: usize| if n < 0 { n + len as i32 + 1 } else { n };
        Ok(match index {
            // toList(start?, count?)
            1 => {
                let start = if argc >= 1 {
                    from_end(self.int_arg(self.arg(0))?, len)
                } else {
                    1
                };
                let start = (start.max(1) - 1) as usize;
                let wanted = if argc >= 2 {
                    self.int_arg(self.arg(1))?.max(0) as usize
                } else {
                    len
                };
                let items = self.vec_items(id)?;
                let slice: Vec<Value> = items.iter().skip(start).take(wanted).copied().collect();
                self.new_list(slice)
            }
            // length()
            2 => Value::Int(len as i32),
            // copyFrom(source, sourceStart, destStart, count)
            3 => {
                let source = self.arg(0);
                let Some(source_len) = self.list_like_len(source)? else {
                    return Err(VmError::new(code::BAD_TYPE_BIF));
                };
                let source_start = from_end(self.int_arg(self.arg(1))?, source_len).max(1) - 1;
                let dest_start = from_end(self.int_arg(self.arg(2))?, len).max(1) - 1;
                let mut count = self.int_arg(self.arg(3))?;
                if source_start >= source_len as i32 {
                    count = 0;
                } else if source_start + count >= source_len as i32 {
                    count = source_len as i32 - source_start;
                }
                for offset in 0..count.max(0) {
                    let value = self.list_like_get(source, (source_start + offset + 1) as usize)?;
                    let items = self.vec_items_mut(id)?;
                    let at = (dest_start + offset) as usize;
                    if at >= items.len() {
                        items.resize(at + 1, Value::Nil);
                    }
                    items[at] = value;
                }
                self_value
            }
            // fillValue(value, start?, count?)
            4 => {
                let value = self.arg(0);
                let start = if argc >= 2 {
                    from_end(self.int_arg(self.arg(1))?, len)
                } else {
                    1
                };
                let start = (start.max(1) - 1) as usize;
                let end = if argc >= 3 {
                    start as i32 + self.int_arg(self.arg(2))?
                } else {
                    len as i32
                };
                let end = end.max(0) as usize;
                let items = self.vec_items_mut(id)?;
                if end > items.len() {
                    items.resize(end, Value::Nil);
                }
                for item in items.iter_mut().take(end).skip(start) {
                    *item = value;
                }
                self_value
            }
            // subset(func)
            5 => {
                let items = self.vec_items(id)?.clone();
                let kept = self.seq_subset(&items, self.arg(0), &native)?;
                self.new_vector_with_room(kept, items.len())
            }
            // applyAll(func): replace each element with the function's
            // result for it.
            6 => {
                let function = self.arg(0);
                let mut position = 0;
                while let Some(&item) = self.vec_items(id)?.get(position) {
                    let result = self.call_value(function, &[item], native.clone())?;
                    if let Some(slot) = self.vec_items_mut(id)?.get_mut(position) {
                        *slot = result;
                    }
                    position += 1;
                }
                self_value
            }
            // indexWhich, valWhich, lastIndexWhich, lastValWhich
            7 | 12 | 14 | 15 => {
                let forward = index == 7 || index == 12;
                let items = self.vec_items(id)?.clone();
                match self.seq_index_which(&items, self.arg(0), forward, &native)? {
                    Some(found) if index == 7 || index == 14 => Value::Int(found as i32 + 1),
                    Some(found) => self
                        .vec_items(id)?
                        .get(found)
                        .copied()
                        .unwrap_or(Value::Nil),
                    None => Value::Nil,
                }
            }
            // forEach(func) and forEachAssoc(func). The vector can change
            // while this runs, so each element is fetched as it is reached.
            8 | 9 => {
                let function = self.arg(0);
                let mut position = 0;
                while let Some(&item) = self.vec_items(id)?.get(position) {
                    if index == 9 {
                        let key = Value::Int(position as i32 + 1);
                        self.call_value(function, &[key, item], native.clone())?;
                    } else {
                        self.call_value(function, &[item], native.clone())?;
                    }
                    position += 1;
                }
                Value::Nil
            }
            // mapAll(func)
            10 => {
                let function = self.arg(0);
                let mut mapped = Vec::with_capacity(len);
                let mut position = 0;
                while let Some(&item) = self.vec_items(id)?.get(position) {
                    mapped.push(self.call_value(function, &[item], native.clone())?);
                    position += 1;
                }
                // With the room the vector has.
                let room = self.objects.vector_alloc(id);
                self.new_vector_with_room(mapped, room)
            }
            // indexOf(val) and lastIndexOf(val)
            11 | 13 => {
                let items = self.vec_items(id)?.clone();
                match self.seq_index_of(&items, self.arg(0), index == 11)? {
                    Some(found) => Value::Int(found as i32 + 1),
                    None => Value::Nil,
                }
            }
            // countOf(val)
            16 => {
                let items = self.vec_items(id)?.clone();
                let wanted = self.arg(0);
                let mut count = 0;
                for item in items {
                    if self.equals(item, wanted)? {
                        count += 1;
                    }
                }
                Value::Int(count)
            }
            // countWhich(func)
            17 => {
                let items = self.vec_items(id)?.clone();
                let function = self.arg(0);
                let mut count = 0;
                for item in items {
                    if is_true(self.call_value(function, &[item], native.clone())?) {
                        count += 1;
                    }
                }
                Value::Int(count)
            }
            // getUnique()
            18 => {
                let items = self.vec_items(id)?.clone();
                let unique = self.seq_unique(&items)?;
                self.new_vector_with_room(unique, items.len())
            }
            // appendUnique(val)
            19 => {
                let other = self.arg(0);
                let mut items = self.vec_items(id)?.clone();
                match self.list_like_len(other)? {
                    Some(other_len) => {
                        for position in 1..=other_len {
                            items.push(self.list_like_get(other, position)?);
                        }
                    }
                    None => items.push(other),
                }
                // The vector is made long enough for all of them before
                // what is already there is taken out.
                self.objects.note_vector_len(id, items.len());
                let unique = self.seq_unique(&items)?;
                *self.vec_items_mut(id)? = unique;
                self_value
            }
            // sort(descending?, compareFunc?)
            20 => {
                let descending = argc >= 1 && !self.arg(0).is_nil();
                let compare = if argc >= 2 { self.arg(1) } else { Value::Nil };
                let mut items = self.vec_items(id)?.clone();
                self.seq_sort(&mut items, compare, descending, &native)?;
                *self.vec_items_mut(id)? = items;
                self_value
            }
            // setLength(len)
            21 => {
                let new_len = self.int_arg(self.arg(0))?;
                if !(0..MAX_ELEMENTS).contains(&new_len) {
                    return Err(VmError::new(code::BAD_VAL_BIF));
                }
                self.vec_items_mut(id)?.resize(new_len as usize, Value::Nil);
                self_value
            }
            // insertAt(index, val...)
            22 => {
                let mut at = self.int_arg(self.arg(0))?;
                if at <= 0 {
                    at += len as i32 + 1;
                }
                if at < 1 || at > len as i32 + 1 {
                    return Err(VmError::new(code::INDEX_OUT_OF_RANGE));
                }
                let at = at as usize - 1;
                let new: Vec<Value> = (1..argc as usize).map(|i| self.arg(i)).collect();
                self.vec_items_mut(id)?.splice(at..at, new);
                self_value
            }
            // removeElementAt(index)
            23 => {
                let at = from_end(self.int_arg(self.arg(0))?, len);
                if at < 1 || at > len as i32 {
                    return Err(VmError::new(code::INDEX_OUT_OF_RANGE));
                }
                self.vec_items_mut(id)?.remove(at as usize - 1);
                self_value
            }
            // removeRange(start, end)
            24 => {
                let (start, end) =
                    seq_range(self.int_arg(self.arg(0))?, self.int_arg(self.arg(1))?, len)?;
                self.vec_items_mut(id)?.drain(start..=end);
                self_value
            }
            // append(val)
            25 => {
                let value = self.arg(0);
                self.vec_items_mut(id)?.push(value);
                self_value
            }
            // prepend(val)
            26 => {
                let value = self.arg(0);
                self.vec_items_mut(id)?.insert(0, value);
                self_value
            }
            // appendAll(val)
            27 => {
                let other = self.arg(0);
                let other_len = self.list_like_len(other)?.unwrap_or(0);
                let mut new = Vec::with_capacity(other_len);
                for position in 1..=other_len {
                    new.push(self.list_like_get(other, position)?);
                }
                self.vec_items_mut(id)?.extend(new);
                self_value
            }
            // removeElement(val)
            28 => {
                let wanted = self.arg(0);
                let items = self.vec_items(id)?.clone();
                let mut kept = Vec::with_capacity(items.len());
                for item in items {
                    if !self.equals(item, wanted)? {
                        kept.push(item);
                    }
                }
                *self.vec_items_mut(id)? = kept;
                self_value
            }
            // splice(index, deleteCount, val...)
            29 => {
                let mut start = self.int_arg(self.arg(0))?;
                if start <= 0 {
                    start += len as i32 + 1;
                }
                start -= 1;
                if start < 0 || start > len as i32 {
                    return Err(VmError::new(code::INDEX_OUT_OF_RANGE));
                }
                let delete = self.int_arg(self.arg(1))?.clamp(0, len as i32 - start);
                let new: Vec<Value> = (2..argc as usize).map(|i| self.arg(i)).collect();
                let (start, delete) = (start as usize, delete as usize);
                self.vec_items_mut(id)?.splice(start..start + delete, new);
                self_value
            }
            // join(separator?)
            30 => {
                let separator = match argc >= 1 && !self.arg(0).is_nil() {
                    true => match self.as_str(self.arg(0)) {
                        Some(text) => text.to_vec(),
                        None => return Err(VmError::new(code::BAD_TYPE_BIF)),
                    },
                    false => Vec::new(),
                };
                let items = self.vec_items(id)?.clone();
                let text = self.seq_join(&items, &separator)?;
                self.new_string(text)
            }
            // indexOfMin, minVal, indexOfMax, maxVal
            32..=35 => {
                let callback = if argc >= 1 { Some(self.arg(0)) } else { None };
                let wanted = if index <= 33 {
                    Ordering::Less
                } else {
                    Ordering::Greater
                };
                let items = self.vec_items(id)?.clone();
                let (position, value) = self.seq_minmax(&items, callback, wanted, &native)?;
                if index == 32 || index == 34 {
                    Value::Int(position as i32 + 1)
                } else {
                    value
                }
            }
            _ => {
                return Err(VmError::unimplemented(format!(
                    "Vector.{}",
                    METHODS[index as usize].0
                )));
            }
        })
    }
}
