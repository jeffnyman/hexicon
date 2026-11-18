//! Methods of the string, list, and vector intrinsic classes.
//!
//! Only the handful that the system library needs to start a program and
//! report an error are here so far. The rest report themselves as not
//! implemented.

use crate::error::{VmError, VmResult, code};
use crate::object::Object;
use crate::value::{NativeDesc, ObjId, Value};
use crate::vm::Vm;

/// Method names of the string class, by method number.
const STRING_METHODS: &[&str] = &[
    "",
    "length",
    "substr",
    "toUpper",
    "toLower",
    "find",
    "toUnicode",
    "htmlify",
    "startsWith",
    "endsWith",
    "mapToByteArray",
    "findReplace",
    "splice",
    "split",
    "specialsToHtml",
    "specialsToText",
    "urlEncode",
    "urlDecode",
    "sha256",
    "digestMD5",
    "packBytes",
    "unpackBytes",
    "toTitleCase",
    "toFoldedCase",
    "compareTo",
    "compareIgnoreCase",
    "findLast",
    "findAll",
    "match",
];

/// Method names of the list class, by method number.
const LIST_METHODS: &[&str] = &[
    "",
    "subset",
    "mapAll",
    "length",
    "sublist",
    "intersect",
    "indexOf",
    "car",
    "cdr",
    "indexWhich",
    "forEach",
    "valWhich",
    "lastIndexOf",
    "lastIndexWhich",
    "lastValWhich",
    "countOf",
    "countWhich",
    "getUnique",
    "appendUnique",
    "append",
    "sort",
    "prepend",
    "insertAt",
    "removeElementAt",
    "removeRange",
    "forEachAssoc",
    "generate",
    "splice",
    "join",
    "indexOfMin",
    "minVal",
    "indexOfMax",
    "maxVal",
];

fn unimplemented(class: &str, names: &[&str], index: u16) -> VmError {
    match names.get(index as usize) {
        Some(name) => VmError::unimplemented(format!("{class}.{name}")),
        None => VmError::unimplemented(format!("{class} method {index}")),
    }
}

/// Byte offsets of the character boundaries of UTF-8 text, including the
/// end. Continuation bytes are the ones of the form `10xxxxxx`.
fn char_starts(text: &[u8]) -> Vec<usize> {
    let mut starts: Vec<usize> = text
        .iter()
        .enumerate()
        .filter(|(_, b)| **b & 0xc0 != 0x80)
        .map(|(i, _)| i)
        .collect();
    starts.push(text.len());
    starts
}

impl Vm {
    /// Call a method of the string class on a string constant or object.
    pub(crate) fn string_method(
        &mut self,
        self_value: Value,
        index: u16,
        argc: Option<&mut u32>,
    ) -> VmResult<Option<Value>> {
        let value = match index {
            0 => return Ok(None),
            // length(): the number of characters.
            1 => self.native_method(argc, NativeDesc::fixed(0), |vm, _| {
                let text = vm.require_str(self_value)?;
                Ok(Value::Int(char_starts(&text).len() as i32 - 1))
            })?,
            // substr(start, len?)
            2 => self.native_method(argc, NativeDesc::optional(1, 1), |vm, argc| {
                let text = vm.require_str(self_value)?;
                let start = vm.int_arg(vm.arg(0))?;
                let starts = char_starts(&text);
                let count = starts.len() - 1;
                // A positive start counts from the beginning, a negative
                // one from the end.
                let from = if start > 0 {
                    (start as usize - 1).min(count)
                } else if start < 0 {
                    count.saturating_sub(start.unsigned_abs() as usize)
                } else {
                    0
                };
                let to = if argc >= 2 {
                    let len = vm.int_arg(vm.arg(1))?;
                    if len >= 0 {
                        (from + len as usize).min(count)
                    } else {
                        // A negative length leaves that many characters
                        // off the end.
                        count.saturating_sub(len.unsigned_abs() as usize).max(from)
                    }
                } else {
                    count
                };
                Ok(vm.new_string(&text[starts[from]..starts[to]]))
            })?,
            _ => {
                if argc.is_none() {
                    // A lookup that only asks whether the method exists.
                    return Ok(Some(Value::NativeCode(NativeDesc::variable(0))));
                }
                return Err(unimplemented("String", STRING_METHODS, index));
            }
        };
        Ok(Some(value))
    }

    /// Call a method of the list class on a list constant or object.
    pub(crate) fn list_method(
        &mut self,
        self_value: Value,
        index: u16,
        argc: Option<&mut u32>,
    ) -> VmResult<Option<Value>> {
        let items = |vm: &Vm| {
            vm.as_list(self_value)
                .ok_or_else(|| VmError::new(code::LIST_VAL_REQD))
        };
        let value = match index {
            0 => return Ok(None),
            // length()
            3 => self.native_method(argc, NativeDesc::fixed(0), |vm, _| {
                Ok(Value::Int(items(vm)?.len() as i32))
            })?,
            // sublist(start, len?)
            4 => self.native_method(argc, NativeDesc::optional(1, 1), |vm, argc| {
                let list = items(vm)?;
                let old_count = list.len() as i32;
                let mut start = vm.int_arg(vm.arg(0))?;
                if start < 0 {
                    start += old_count + 1;
                }
                let len = if argc >= 2 {
                    vm.int_arg(vm.arg(1))?
                } else {
                    old_count
                };
                let in_range = start >= 1 && start <= old_count;
                let mut new_count = if in_range { old_count - (start - 1) } else { 0 };
                if len < 0 {
                    new_count += len;
                } else if len < new_count {
                    new_count = len;
                }
                let new_count = new_count.max(0) as usize;
                let first = if in_range { start as usize - 1 } else { 0 };
                let slice: Vec<Value> = (first..first + new_count).map(|i| list.get(i)).collect();
                Ok(vm.new_list(slice))
            })?,
            // indexOf(val)
            6 => self.native_method(argc, NativeDesc::fixed(1), |vm, _| {
                let list = items(vm)?;
                let wanted = vm.arg(0);
                for index in 0..list.len() {
                    if vm.equals(list.get(index), wanted)? {
                        return Ok(Value::Int(index as i32 + 1));
                    }
                }
                Ok(Value::Nil)
            })?,
            // car(): the first element.
            7 => self.native_method(argc, NativeDesc::fixed(0), |vm, _| {
                let list = items(vm)?;
                Ok(if list.len() == 0 {
                    Value::Nil
                } else {
                    list.get(0)
                })
            })?,
            // cdr(): everything after the first element.
            8 => self.native_method(argc, NativeDesc::fixed(0), |vm, _| {
                let list = items(vm)?;
                if list.len() == 0 {
                    return Ok(Value::Nil);
                }
                let rest: Vec<Value> = list.iter().skip(1).collect();
                Ok(vm.new_list(rest))
            })?,
            _ => {
                if argc.is_none() {
                    return Ok(Some(Value::NativeCode(NativeDesc::variable(0))));
                }
                return Err(unimplemented("List", LIST_METHODS, index));
            }
        };
        Ok(Some(value))
    }

    /// Call a method of the collection class, which lists and vectors
    /// derive from.
    pub(crate) fn collection_method(
        &mut self,
        self_value: Value,
        index: u16,
        argc: Option<&mut u32>,
    ) -> VmResult<Option<Value>> {
        let value = match index {
            // createIterator() and createLiveIterator(). They differ only
            // for a vector, which can change during the iteration: the
            // first iterates over a copy.
            1 | 2 => self.native_method(argc, NativeDesc::fixed(0), |vm, _| {
                let mut collection = self_value;
                if index == 1
                    && let Value::Obj(id) = self_value
                    && let Some(Object::Vector(items)) = vm.objects.get(id)
                {
                    let copy = items.clone();
                    collection = vm.new_list(copy);
                }
                let Some(last) = vm.list_like_len(collection)? else {
                    return Err(VmError::new(code::CANNOT_INDEX_TYPE));
                };
                Ok(Value::Obj(vm.objects.alloc(Object::IndexedIter {
                    collection,
                    current: 0,
                    first: 1,
                    last: last as i32,
                })))
            })?,
            _ => return Ok(None),
        };
        Ok(Some(value))
    }

    /// Advance an indexed iterator. Returns the next element, or `None`
    /// at the end.
    pub(crate) fn indexed_iter_next(&mut self, id: ObjId) -> VmResult<Option<Value>> {
        let Some(&Object::IndexedIter {
            collection,
            current,
            last,
            ..
        }) = self.objects.get(id)
        else {
            return Ok(None);
        };
        let next = current + 1;
        if next > last {
            return Ok(None);
        }
        let value = self.list_like_get(collection, next as usize)?;
        if let Some(Object::IndexedIter { current, .. }) = self.objects.get_mut(id) {
            *current = next;
        }
        Ok(Some(value))
    }

    /// Call a method of the iterator class.
    pub(crate) fn iterator_method(
        &mut self,
        id: ObjId,
        index: u16,
        argc: Option<&mut u32>,
    ) -> VmResult<Option<Value>> {
        let Some(&Object::IndexedIter {
            collection,
            current,
            first,
            last,
        }) = self.objects.get(id)
        else {
            return match index {
                0 => Ok(None),
                _ => Err(self.unimplemented_object(id, "iteration")),
            };
        };
        let value = match index {
            // getNext()
            1 => self.native_method(argc, NativeDesc::fixed(0), |vm, _| {
                vm.indexed_iter_next(id)?
                    .ok_or_else(|| VmError::new(code::OUT_OF_RANGE))
            })?,
            // isNextAvailable()
            2 => self.native_method(argc, NativeDesc::fixed(0), |_, _| {
                Ok(Value::logical(current < last))
            })?,
            // resetIterator()
            3 => self.native_method(argc, NativeDesc::fixed(0), |vm, _| {
                if let Some(Object::IndexedIter { current, .. }) = vm.objects.get_mut(id) {
                    *current = first - 1;
                }
                Ok(Value::Nil)
            })?,
            // getCurKey() and getCurVal()
            4 | 5 => self.native_method(argc, NativeDesc::fixed(0), |vm, _| {
                if current < 1 || current > last {
                    return Err(VmError::new(code::OUT_OF_RANGE));
                }
                if index == 4 {
                    Ok(Value::Int(current))
                } else {
                    vm.list_like_get(collection, current as usize)
                }
            })?,
            _ => return Ok(None),
        };
        Ok(Some(value))
    }

    /// Call a method of the vector class.
    pub(crate) fn vector_method(
        &mut self,
        id: ObjId,
        index: u16,
        argc: Option<&mut u32>,
    ) -> VmResult<Option<Value>> {
        let _ = id;
        match index {
            0 => Ok(None),
            _ if argc.is_none() => Ok(Some(Value::NativeCode(NativeDesc::variable(0)))),
            _ => {
                let class = match self.objects.get(id) {
                    Some(Object::AnonFn(_)) => "AnonFuncPtr",
                    _ => "Vector",
                };
                Err(VmError::unimplemented(format!("{class} method {index}")))
            }
        }
    }
}
