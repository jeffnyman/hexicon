//! Methods of the collection and iterator intrinsic classes.

use crate::error::{VmError, VmResult, code};
use crate::object::Object;
use crate::value::{NativeDesc, ObjId, Value};
use crate::vm::Vm;

impl Vm {
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
                if let Value::Obj(id) = self_value
                    && matches!(vm.objects.get(id), Some(Object::LookupTable(_)))
                {
                    return vm.table_iterator(id, index == 2);
                }
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
                Ok(Value::Obj(vm.alloc_obj(Object::IndexedIter {
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
        if matches!(self.objects.get(id), Some(Object::LookupIter { .. })) {
            return match index {
                1..=5 => self
                    .native_method(argc, NativeDesc::fixed(0), |vm, _| {
                        vm.table_iter_method(id, index)
                    })
                    .map(Some),
                _ => Ok(None),
            };
        }
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
}
