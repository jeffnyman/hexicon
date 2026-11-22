//! The list intrinsic class, and the sequence operations it shares with
//! vectors.
//!
//! A list never changes. Every method that "modifies" a list returns a new
//! one.

use std::cmp::Ordering;

use crate::error::{VmError, VmResult, code};
use crate::meta::MetaKind;
use crate::value::{NativeDesc, Value};
use crate::vm::{NativeCall, Vm};

/// Method numbers of the list class.
mod method {
    pub const SUBSET: u16 = 1;
    pub const MAP_ALL: u16 = 2;
    pub const LENGTH: u16 = 3;
    pub const SUBLIST: u16 = 4;
    pub const INTERSECT: u16 = 5;
    pub const INDEX_OF: u16 = 6;
    pub const CAR: u16 = 7;
    pub const CDR: u16 = 8;
    pub const INDEX_WHICH: u16 = 9;
    pub const FOR_EACH: u16 = 10;
    pub const VAL_WHICH: u16 = 11;
    pub const LAST_INDEX_OF: u16 = 12;
    pub const LAST_INDEX_WHICH: u16 = 13;
    pub const LAST_VAL_WHICH: u16 = 14;
    pub const COUNT_OF: u16 = 15;
    pub const COUNT_WHICH: u16 = 16;
    pub const GET_UNIQUE: u16 = 17;
    pub const APPEND_UNIQUE: u16 = 18;
    pub const APPEND: u16 = 19;
    pub const SORT: u16 = 20;
    pub const PREPEND: u16 = 21;
    pub const INSERT_AT: u16 = 22;
    pub const REMOVE_ELEMENT_AT: u16 = 23;
    pub const REMOVE_RANGE: u16 = 24;
    pub const FOR_EACH_ASSOC: u16 = 25;
    pub const GENERATE: u16 = 26;
    pub const SPLICE: u16 = 27;
    pub const JOIN: u16 = 28;
    pub const INDEX_OF_MIN: u16 = 29;
    pub const MIN_VAL: u16 = 30;
    pub const INDEX_OF_MAX: u16 = 31;
    pub const MAX_VAL: u16 = 32;
}

/// The name and calling interface of each list method, by method number.
const METHODS: &[(&str, NativeDesc)] = &[
    ("", NativeDesc::fixed(0)),
    ("subset", NativeDesc::fixed(1)),
    ("mapAll", NativeDesc::fixed(1)),
    ("length", NativeDesc::fixed(0)),
    ("sublist", NativeDesc::optional(1, 1)),
    ("intersect", NativeDesc::fixed(1)),
    ("indexOf", NativeDesc::fixed(1)),
    ("car", NativeDesc::fixed(0)),
    ("cdr", NativeDesc::fixed(0)),
    ("indexWhich", NativeDesc::fixed(1)),
    ("forEach", NativeDesc::fixed(1)),
    ("valWhich", NativeDesc::fixed(1)),
    ("lastIndexOf", NativeDesc::fixed(1)),
    ("lastIndexWhich", NativeDesc::fixed(1)),
    ("lastValWhich", NativeDesc::fixed(1)),
    ("countOf", NativeDesc::fixed(1)),
    ("countWhich", NativeDesc::fixed(1)),
    ("getUnique", NativeDesc::fixed(0)),
    ("appendUnique", NativeDesc::fixed(1)),
    ("append", NativeDesc::fixed(1)),
    ("sort", NativeDesc::optional(0, 2)),
    ("prepend", NativeDesc::fixed(1)),
    ("insertAt", NativeDesc::variable(2)),
    ("removeElementAt", NativeDesc::fixed(1)),
    ("removeRange", NativeDesc::fixed(2)),
    ("forEachAssoc", NativeDesc::fixed(1)),
    // `generate` is a static method; on an instance it is undefined.
    ("generate", NativeDesc::fixed(2)),
    ("splice", NativeDesc::variable(2)),
    ("join", NativeDesc::optional(0, 1)),
    ("indexOfMin", NativeDesc::optional(0, 1)),
    ("minVal", NativeDesc::optional(0, 1)),
    ("indexOfMax", NativeDesc::optional(0, 1)),
    ("maxVal", NativeDesc::optional(0, 1)),
];

/// Whether a callback's result counts as true.
pub(crate) fn is_true(value: Value) -> bool {
    !matches!(value, Value::Nil | Value::Int(0))
}

impl Vm {
    /// Describe a native method call for stack traces and error messages.
    /// The method's arguments are still on the stack.
    pub(crate) fn native_call(
        &self,
        name: &'static str,
        self_value: Value,
        method_index: u16,
        argc: u32,
    ) -> NativeCall {
        NativeCall {
            name,
            function: None,
            self_value,
            method_index,
            args: (0..argc as usize).map(|i| self.arg(i)).collect(),
            caller_pc: self.pc,
            arg_top: self.stack.len(),
        }
    }

    /// Call a method of the list class on a list constant or object.
    pub(crate) fn list_method(
        &mut self,
        self_value: Value,
        index: u16,
        argc: Option<&mut u32>,
    ) -> VmResult<Option<Value>> {
        if index == 0 || index == method::GENERATE {
            return Ok(None);
        }
        let Some(&(_, desc)) = METHODS.get(index as usize) else {
            return Ok(None);
        };
        self.native_method(argc, desc, |vm, argc| vm.list_call(self_value, index, argc))
            .map(Some)
    }

    /// The static methods of the list class.
    pub(crate) fn list_static_method(
        &mut self,
        index: u16,
        argc: Option<&mut u32>,
    ) -> VmResult<Option<Value>> {
        if index != method::GENERATE {
            return Ok(None);
        }
        // List.generate(func, n)
        self.native_method(argc, NativeDesc::fixed(2), |vm, argc| {
            let class = Value::obj_or_nil(vm.meta.class_obj(MetaKind::List));
            let native = vm.native_call("List.generate", class, method::GENERATE, argc);
            let items = vm.generate(vm.arg(0), vm.arg(1), &native)?;
            Ok(vm.new_list(items))
        })
        .map(Some)
    }

    /// Build a sequence by calling a function for each index from 1 to a
    /// count, for the `generate` methods.
    pub(crate) fn generate(
        &mut self,
        function: Value,
        count: Value,
        native: &NativeCall,
    ) -> VmResult<Vec<Value>> {
        let Some((min, opt, varargs)) = self.value_interface(function) else {
            return Err(VmError::new(code::BAD_TYPE_BIF));
        };
        if !self.is_function_value(function) {
            return Err(VmError::new(code::BAD_TYPE_BIF));
        }
        let Value::Int(count) = count else {
            return Err(VmError::new(code::NUM_VAL_REQD));
        };
        if count < 0 {
            return Err(VmError::new(code::BAD_VAL_BIF));
        }
        // The function gets the index if it takes an argument.
        let takes_index = varargs || min + opt >= 1;
        let mut items = Vec::with_capacity(count as usize);
        for index in 1..=count {
            let args: &[Value] = if takes_index {
                &[Value::Int(index)]
            } else {
                &[]
            };
            items.push(self.call_value(function, args, native.clone())?);
        }
        Ok(items)
    }

    pub(crate) fn list_items(&self, list: Value) -> VmResult<Vec<Value>> {
        self.as_list(list)
            .map(|items| items.to_vec())
            .ok_or_else(|| VmError::new(code::LIST_VAL_REQD))
    }

    fn list_call(&mut self, self_value: Value, index: u16, argc: u32) -> VmResult<Value> {
        let items = self.list_items(self_value)?;
        let name = METHODS[index as usize].0;
        // The description of this call that a callback's stack trace shows.
        let native = |vm: &Vm, name: &'static str| vm.native_call(name, self_value, index, argc);
        Ok(match index {
            // These make their result first, as a copy of the list, and
            // fill it in, as the reference does, so the objects the
            // callback makes are numbered after it.
            method::SUBSET => {
                let native = native(self, "List.subset");
                let result = self.new_list(items.clone());
                let kept = self.seq_subset(&items, self.arg(0), &native)?;
                self.objects
                    .replace(result.obj_id(), crate::object::Object::List(kept.into()));
                result
            }
            method::MAP_ALL => {
                let native = native(self, "List.mapAll");
                let result = self.new_list(items.clone());
                let function = self.arg(0);
                let mut mapped = Vec::with_capacity(items.len());
                for &item in &items {
                    mapped.push(self.call_value(function, &[item], native.clone())?);
                }
                self.objects
                    .replace(result.obj_id(), crate::object::Object::List(mapped.into()));
                result
            }
            method::LENGTH => Value::Int(items.len() as i32),
            method::SUBLIST => {
                let old_count = items.len() as i32;
                let mut start = self.int_arg(self.arg(0))?;
                if start < 0 {
                    start += old_count + 1;
                }
                let len = if argc >= 2 {
                    self.int_arg(self.arg(1))?
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
                self.new_list(&items[first..first + new_count])
            }
            method::INTERSECT => {
                let common = self.seq_intersect(self_value, self.arg(0))?;
                self.new_list(common)
            }
            method::INDEX_OF => match self.seq_index_of(&items, self.arg(0), true)? {
                Some(found) => Value::Int(found as i32 + 1),
                None => Value::Nil,
            },
            method::CAR => items.first().copied().unwrap_or(Value::Nil),
            method::CDR => match items.split_first() {
                Some((_, rest)) => self.new_list(rest),
                None => Value::Nil,
            },
            method::INDEX_WHICH | method::LAST_INDEX_WHICH => {
                let forward = index == method::INDEX_WHICH;
                let native = native(
                    self,
                    if forward {
                        "List.indexWhich"
                    } else {
                        "List.lastIndexWhich"
                    },
                );
                match self.seq_index_which(&items, self.arg(0), forward, &native)? {
                    Some(found) => Value::Int(found as i32 + 1),
                    None => Value::Nil,
                }
            }
            method::VAL_WHICH | method::LAST_VAL_WHICH => {
                // These report themselves as the index methods they are
                // built on.
                let forward = index == method::VAL_WHICH;
                let native = self.native_call(
                    if forward {
                        "List.indexWhich"
                    } else {
                        "List.lastIndexWhich"
                    },
                    self_value,
                    if forward {
                        method::INDEX_WHICH
                    } else {
                        method::LAST_INDEX_WHICH
                    },
                    argc,
                );
                match self.seq_index_which(&items, self.arg(0), forward, &native)? {
                    Some(found) => items[found],
                    None => Value::Nil,
                }
            }
            method::FOR_EACH | method::FOR_EACH_ASSOC => {
                let with_index = index == method::FOR_EACH_ASSOC;
                let native = native(
                    self,
                    if with_index {
                        "List.forEachAssoc"
                    } else {
                        "List.forEach"
                    },
                );
                let function = self.arg(0);
                for (position, &item) in items.iter().enumerate() {
                    if with_index {
                        let key = Value::Int(position as i32 + 1);
                        self.call_value(function, &[key, item], native.clone())?;
                    } else {
                        self.call_value(function, &[item], native.clone())?;
                    }
                }
                Value::Nil
            }
            method::LAST_INDEX_OF => match self.seq_index_of(&items, self.arg(0), false)? {
                Some(found) => Value::Int(found as i32 + 1),
                None => Value::Nil,
            },
            method::COUNT_OF => {
                let wanted = self.arg(0);
                let mut count = 0;
                for &item in &items {
                    if self.equals(item, wanted)? {
                        count += 1;
                    }
                }
                Value::Int(count)
            }
            method::COUNT_WHICH => {
                let native = native(self, "List.countWhich");
                let function = self.arg(0);
                let mut count = 0;
                for &item in &items {
                    if is_true(self.call_value(function, &[item], native.clone())?) {
                        count += 1;
                    }
                }
                Value::Int(count)
            }
            method::GET_UNIQUE => {
                let unique = self.seq_unique(&items)?;
                self.new_list(unique)
            }
            method::APPEND_UNIQUE => {
                let other = self.arg(0);
                let Some(len) = self.list_like_len(other)? else {
                    return Err(VmError::new(code::LIST_VAL_REQD));
                };
                let mut all = items;
                for position in 1..=len {
                    all.push(self.list_like_get(other, position)?);
                }
                let unique = self.seq_unique(&all)?;
                self.new_list(unique)
            }
            method::APPEND => {
                let mut items = items;
                items.push(self.arg(0));
                self.new_list(items)
            }
            method::PREPEND => {
                let mut items = items;
                items.insert(0, self.arg(0));
                self.new_list(items)
            }
            method::INSERT_AT => {
                let mut at = self.int_arg(self.arg(0))?;
                if at <= 0 {
                    at += items.len() as i32 + 1;
                }
                at -= 1;
                if at < 0 || at as usize > items.len() {
                    return Err(VmError::new(code::INDEX_OUT_OF_RANGE));
                }
                let mut items = items;
                let new: Vec<Value> = (1..argc as usize).map(|i| self.arg(i)).collect();
                items.splice(at as usize..at as usize, new);
                self.new_list(items)
            }
            method::REMOVE_ELEMENT_AT => {
                let mut at = self.int_arg(self.arg(0))?;
                if at < 0 {
                    at += items.len() as i32 + 1;
                }
                at -= 1;
                if at < 0 || at as usize >= items.len() {
                    return Err(VmError::new(code::INDEX_OUT_OF_RANGE));
                }
                let mut items = items;
                items.remove(at as usize);
                self.new_list(items)
            }
            method::REMOVE_RANGE => {
                let (start, end) = seq_range(
                    self.int_arg(self.arg(0))?,
                    self.int_arg(self.arg(1))?,
                    items.len(),
                )?;
                let mut items = items;
                items.drain(start..=end);
                self.new_list(items)
            }
            method::SORT => {
                let descending = argc >= 1 && !self.arg(0).is_nil();
                let compare = if argc >= 2 { self.arg(1) } else { Value::Nil };
                let native = native(self, "List.sort");
                let mut items = items;
                self.seq_sort(&mut items, compare, descending, &native)?;
                self.new_list(items)
            }
            method::SPLICE => {
                let count = items.len() as i32;
                let mut start = self.int_arg(self.arg(0))?;
                let delete = self.int_arg(self.arg(1))?;
                if start <= 0 {
                    start += count + 1;
                }
                let start = (start - 1).clamp(0, count);
                let delete = delete.clamp(0, count - start);
                let new: Vec<Value> = (2..argc as usize).map(|i| self.arg(i)).collect();
                if delete == 0 && new.is_empty() {
                    // Nothing changes, so the result is the same list.
                    return Ok(self_value);
                }
                let mut items = items;
                items.splice(start as usize..(start + delete) as usize, new);
                self.new_list(items)
            }
            method::JOIN => {
                let separator = match argc >= 1 && !self.arg(0).is_nil() {
                    true => match self.as_str(self.arg(0)) {
                        Some(text) => text.to_vec(),
                        None => return Err(VmError::new(code::BAD_TYPE_BIF)),
                    },
                    false => Vec::new(),
                };
                let text = self.seq_join(&items, &separator)?;
                self.new_string(text)
            }
            method::INDEX_OF_MIN | method::MIN_VAL | method::INDEX_OF_MAX | method::MAX_VAL => {
                let native = native(self, name_for_minmax(index));
                let callback = if argc >= 1 { Some(self.arg(0)) } else { None };
                let wanted = if index == method::INDEX_OF_MIN || index == method::MIN_VAL {
                    Ordering::Less
                } else {
                    Ordering::Greater
                };
                let (position, value) = self.seq_minmax(&items, callback, wanted, &native)?;
                if index == method::INDEX_OF_MIN || index == method::INDEX_OF_MAX {
                    Value::Int(position as i32 + 1)
                } else {
                    value
                }
            }
            _ => {
                return Err(VmError::unimplemented(format!("List.{name}")));
            }
        })
    }

    // ---- operations shared by lists and vectors ----

    /// The elements for which a callback returns true.
    pub(crate) fn seq_subset(
        &mut self,
        items: &[Value],
        function: Value,
        native: &NativeCall,
    ) -> VmResult<Vec<Value>> {
        let mut kept = Vec::new();
        for &item in items {
            if is_true(self.call_value(function, &[item], native.clone())?) {
                kept.push(item);
            }
        }
        Ok(kept)
    }

    /// The index of the first or last element equal to a value.
    pub(crate) fn seq_index_of(
        &mut self,
        items: &[Value],
        wanted: Value,
        forward: bool,
    ) -> VmResult<Option<usize>> {
        let mut order: Box<dyn Iterator<Item = usize>> = if forward {
            Box::new(0..items.len())
        } else {
            Box::new((0..items.len()).rev())
        };
        for index in &mut order {
            if self.equals(items[index], wanted)? {
                return Ok(Some(index));
            }
        }
        Ok(None)
    }

    /// The index of the first or last element for which a callback
    /// returns true.
    pub(crate) fn seq_index_which(
        &mut self,
        items: &[Value],
        function: Value,
        forward: bool,
        native: &NativeCall,
    ) -> VmResult<Option<usize>> {
        let mut order: Box<dyn Iterator<Item = usize>> = if forward {
            Box::new(0..items.len())
        } else {
            Box::new((0..items.len()).rev())
        };
        for index in &mut order {
            if is_true(self.call_value(function, &[items[index]], native.clone())?) {
                return Ok(Some(index));
            }
        }
        Ok(None)
    }

    /// The elements with later duplicates removed.
    pub(crate) fn seq_unique(&mut self, items: &[Value]) -> VmResult<Vec<Value>> {
        let mut unique: Vec<Value> = Vec::with_capacity(items.len());
        for &item in items {
            let mut found = false;
            for &kept in &unique {
                if self.equals(item, kept)? {
                    found = true;
                    break;
                }
            }
            if !found {
                unique.push(item);
            }
        }
        Ok(unique)
    }

    /// The elements two list-like values have in common. The result
    /// follows the order of the shorter of the two.
    pub(crate) fn seq_intersect(&mut self, a: Value, b: Value) -> VmResult<Vec<Value>> {
        let len_a = self.list_like_len(a)?.unwrap_or(0);
        let len_b = self.list_like_len(b)?.unwrap_or(0);
        let (short, short_len, long, long_len) = if len_a > len_b {
            (b, len_b, a, len_a)
        } else {
            (a, len_a, b, len_b)
        };
        let mut long_items = Vec::with_capacity(long_len);
        for index in 1..=long_len {
            long_items.push(self.list_like_get(long, index)?);
        }
        let mut common = Vec::new();
        for index in 1..=short_len {
            let item = self.list_like_get(short, index)?;
            for &other in &long_items {
                if self.equals(other, item)? {
                    common.push(item);
                    break;
                }
            }
        }
        Ok(common)
    }

    /// Join the text of the elements with a separator.
    pub(crate) fn seq_join(&mut self, items: &[Value], separator: &[u8]) -> VmResult<Vec<u8>> {
        let mut text = Vec::new();
        for (index, &item) in items.iter().enumerate() {
            if index > 0 {
                text.extend_from_slice(separator);
            }
            match item {
                Value::Nil => {}
                Value::Int(n) => text.extend_from_slice(n.to_string().as_bytes()),
                _ => match self.cast_to_text(item) {
                    Ok(piece) => text.extend_from_slice(&piece),
                    Err(VmError::Runtime(error)) if error.code == code::NO_STR_CONV => {
                        return Err(VmError::new(code::NO_STR_CONV));
                    }
                    Err(error) => return Err(error),
                },
            }
        }
        Ok(text)
    }

    /// The smallest or largest element, or the element for which a
    /// callback returns the smallest or largest value. Returns its index
    /// and the winning value.
    pub(crate) fn seq_minmax(
        &mut self,
        items: &[Value],
        callback: Option<Value>,
        wanted: Ordering,
        native: &NativeCall,
    ) -> VmResult<(usize, Value)> {
        if let Some(callback) = callback
            && !self.is_function_value(callback)
        {
            return Err(VmError::new(code::BAD_TYPE_BIF));
        }
        if items.is_empty() {
            return Err(VmError::new(code::BAD_VAL_BIF));
        }
        let mut winner = (0, Value::Nil);
        for (index, &item) in items.iter().enumerate() {
            let value = match callback {
                Some(callback) => self.call_value(callback, &[item], native.clone())?,
                None => item,
            };
            if index == 0 || self.compare(value, winner.1)? == wanted {
                winner = (index, value);
            }
        }
        Ok(winner)
    }

    /// Sort values in place.
    ///
    /// The algorithm is the reference VM's quicksort, step for step: it
    /// is not stable, so equal elements end up in an order that depends on
    /// exactly how the sort proceeds, and programs can see that order.
    pub(crate) fn seq_sort(
        &mut self,
        items: &mut [Value],
        compare: Value,
        descending: bool,
        native: &NativeCall,
    ) -> VmResult<()> {
        if items.len() > 1 {
            self.quicksort(items, 0, items.len() - 1, compare, descending, native)?;
        }
        Ok(())
    }

    fn sort_compare(
        &mut self,
        a: Value,
        b: Value,
        compare: Value,
        descending: bool,
        native: &NativeCall,
    ) -> VmResult<Ordering> {
        let order = if compare.is_nil() {
            self.compare(a, b)?
        } else {
            match self.call_value(compare, &[a, b], native.clone())? {
                Value::Int(n) => n.cmp(&0),
                _ => return Err(VmError::new(code::NUM_VAL_REQD)),
            }
        };
        Ok(if descending { order.reverse() } else { order })
    }

    fn quicksort(
        &mut self,
        items: &mut [Value],
        left: usize,
        right: usize,
        compare: Value,
        descending: bool,
        native: &NativeCall,
    ) -> VmResult<()> {
        if right <= left {
            return Ok(());
        }
        // The pivot starts as the last element and is tracked as the
        // exchanges move it. Indexes step outside the range by one, as in
        // the original, so the arithmetic wraps.
        let mut pivot = right;
        let mut i = left.wrapping_sub(1);
        let mut j = right;
        loop {
            loop {
                i = i.wrapping_add(1);
                if i == right
                    || self.sort_compare(items[i], items[pivot], compare, descending, native)?
                        != Ordering::Less
                {
                    break;
                }
            }
            loop {
                j = j.wrapping_sub(1);
                if j == left
                    || self.sort_compare(items[j], items[pivot], compare, descending, native)?
                        != Ordering::Greater
                {
                    break;
                }
            }
            items.swap(i, j);
            if pivot == i {
                pivot = j;
            } else if pivot == j {
                pivot = i;
            }
            if j <= i {
                break;
            }
        }
        items.swap(i, j);
        items.swap(i, right);
        if i > left {
            self.quicksort(items, left, i - 1, compare, descending, native)?;
        }
        if i < right {
            self.quicksort(items, i + 1, right, compare, descending, native)?;
        }
        Ok(())
    }
}

fn name_for_minmax(index: u16) -> &'static str {
    match index {
        method::INDEX_OF_MIN => "List.indexOfMin",
        method::MIN_VAL => "List.minVal",
        method::INDEX_OF_MAX => "List.indexOfMax",
        _ => "List.maxVal",
    }
}

/// Convert the one-based, possibly negative start and end indexes of a
/// range to zero-based ones, checking them against a length.
pub(crate) fn seq_range(start: i32, end: i32, len: usize) -> VmResult<(usize, usize)> {
    let len = len as i32;
    let start = if start < 0 { start + len + 1 } else { start } - 1;
    let end = if end < 0 { end + len + 1 } else { end } - 1;
    if start < 0 || start >= len || end < 0 || end >= len || end < start {
        return Err(VmError::new(code::INDEX_OUT_OF_RANGE));
    }
    Ok((start as usize, end as usize))
}
