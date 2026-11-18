//! Operators: equality, ordering, arithmetic, indexing, and conversion
//! to text.

use std::cmp::Ordering;

use crate::error::{VmError, VmResult, code};
use crate::object::Object;
use crate::value::{INVALID_PROP, ObjId, Value};
use crate::vm::{NativeCall, Return, Vm};

impl Vm {
    // ---- equality and ordering ----

    /// Whether two values are equal, as the `==` operator sees them.
    pub(crate) fn equals(&mut self, a: Value, b: Value) -> VmResult<bool> {
        // An object on either side decides for both.
        if let (Value::Obj(id), false) = (b, matches!(a, Value::Obj(_))) {
            return self.obj_equals(id, a);
        }
        Ok(match a {
            Value::Nil | Value::True => a == b,
            Value::Obj(id) => return self.obj_equals(id, b),
            Value::Prop(_) | Value::Int(_) | Value::Enum(_) | Value::ObjX(_) => a == b,
            Value::BifPtr { set, index } | Value::BifPtrX { set, index } => {
                b == Value::BifPtr { set, index }
            }
            Value::SString(_) => match (self.as_str(a), self.as_str(b)) {
                (Some(x), Some(y)) => *x == *y,
                _ => false,
            },
            Value::List(_) => return self.list_equals(a, b),
            Value::CodeOfs(_) | Value::FuncPtr(_) => a == b,
            Value::Empty | Value::DString(_) | Value::NativeCode(_) => false,
        })
    }

    fn obj_equals(&mut self, id: ObjId, other: Value) -> VmResult<bool> {
        match self.objects.get(id) {
            Some(Object::String(text)) => {
                let text = text.clone();
                Ok(self.as_str(other).is_some_and(|other| *other == *text))
            }
            Some(Object::List(_)) => self.list_equals(Value::Obj(id), other),
            // Everything else is equal only to itself.
            _ => Ok(other == Value::Obj(id)),
        }
    }

    /// Whether a list equals another value: a list-like value of the same
    /// length with equal elements.
    fn list_equals(&mut self, list: Value, other: Value) -> VmResult<bool> {
        let Some(items) = self.as_list(list) else {
            return Ok(false);
        };
        let Some(len) = self.list_like_len(other)? else {
            return Ok(false);
        };
        if len != items.len() {
            return Ok(false);
        }
        for index in 0..len {
            let theirs = self.list_like_get(other, index + 1)?;
            if !self.equals(items.get(index), theirs)? {
                return Ok(false);
            }
        }
        Ok(true)
    }

    /// Compare two values for the ordering operators.
    pub(crate) fn compare(&mut self, a: Value, b: Value) -> VmResult<Ordering> {
        match (a, b) {
            (Value::Int(x), Value::Int(y)) => Ok(x.cmp(&y)),
            (Value::Obj(id), _) => self.obj_compare(id, b),
            (Value::SString(_), _) => match (self.as_str(a), self.as_str(b)) {
                (Some(x), Some(y)) => Ok(x.cmp(&y)),
                _ => Err(VmError::new(code::INVALID_COMPARISON)),
            },
            (_, Value::Obj(id)) => Ok(self.obj_compare(id, a)?.reverse()),
            _ => Err(VmError::new(code::INVALID_COMPARISON)),
        }
    }

    fn obj_compare(&mut self, id: ObjId, other: Value) -> VmResult<Ordering> {
        match self.objects.get(id) {
            Some(Object::String(text)) => {
                let text = text.clone();
                match self.as_str(other) {
                    Some(other) => Ok((*text).cmp(&*other)),
                    None => Err(VmError::new(code::INVALID_COMPARISON)),
                }
            }
            _ => Err(VmError::new(code::INVALID_COMPARISON)),
        }
    }

    // ---- arithmetic ----

    /// Add two values when the left one is not an integer paired with an
    /// integer. Returns `None` if the left operand has no built-in
    /// addition, leaving it to an overloaded operator.
    pub(crate) fn add(&mut self, a: Value, b: Value) -> VmResult<Option<Value>> {
        match a {
            Value::SString(_) => self.concat(a, b).map(Some),
            Value::List(_) => self.list_append(a, b).map(Some),
            Value::Obj(id) => match self.objects.get(id) {
                Some(Object::String(_)) => self.concat(a, b).map(Some),
                Some(Object::List(_)) => self.list_append(a, b).map(Some),
                Some(Object::Vector(items)) => {
                    let mut items = items.clone();
                    match self.list_like_len(b)? {
                        Some(len) => {
                            for index in 1..=len {
                                items.push(self.list_like_get(b, index)?);
                            }
                        }
                        None => items.push(b),
                    }
                    Ok(Some(Value::Obj(self.objects.alloc(Object::Vector(items)))))
                }
                _ => Ok(None),
            },
            Value::Int(_) => Err(VmError::new(code::NUM_VAL_REQD)),
            _ => Ok(None),
        }
    }

    /// Subtract, under the same terms as [`Vm::add`].
    pub(crate) fn sub(&mut self, a: Value, b: Value) -> VmResult<Option<Value>> {
        match a {
            Value::List(_) => self.list_remove(a, b).map(Some),
            Value::Obj(id) => match self.objects.get(id) {
                Some(Object::List(_)) => self.list_remove(a, b).map(Some),
                Some(Object::Vector(items)) => {
                    let items = items.clone();
                    let kept = self.without(&items, b)?;
                    Ok(Some(Value::Obj(self.objects.alloc(Object::Vector(kept)))))
                }
                _ => Ok(None),
            },
            Value::Int(_) => Err(VmError::new(code::NUM_VAL_REQD)),
            _ => Ok(None),
        }
    }

    /// Multiply or divide values that are not both integers. Nothing but
    /// numbers has built-in multiplication or division.
    pub(crate) fn mul_div(&mut self, a: Value) -> VmResult<Option<Value>> {
        match a {
            Value::Int(_) => Err(VmError::new(code::NUM_VAL_REQD)),
            _ => Ok(None),
        }
    }

    /// Concatenate a string with the text of another value.
    fn concat(&mut self, string: Value, other: Value) -> VmResult<Value> {
        let left = self.require_str(string)?;
        let (right_value, right) = self.text_of(other)?;
        if right.is_empty() || other.is_nil() {
            return Ok(string);
        }
        if left.is_empty() && self.as_str(other).is_some() {
            return Ok(right_value);
        }
        let mut text = Vec::with_capacity(left.len() + right.len());
        text.extend_from_slice(&left);
        text.extend_from_slice(&right);
        Ok(self.new_string(text))
    }

    /// A list with another value appended. A list-like value contributes
    /// its elements.
    fn list_append(&mut self, list: Value, other: Value) -> VmResult<Value> {
        let mut items = match self.as_list(list) {
            Some(items) => items.to_vec(),
            None => return Err(VmError::new(code::LIST_VAL_REQD)),
        };
        match self.list_like_len(other)? {
            Some(len) => {
                for index in 1..=len {
                    items.push(self.list_like_get(other, index)?);
                }
            }
            None => items.push(other),
        }
        Ok(self.new_list(items))
    }

    /// A list without the elements equal to another value, or to any
    /// element of a list-like value.
    fn list_remove(&mut self, list: Value, other: Value) -> VmResult<Value> {
        let items = match self.as_list(list) {
            Some(items) => items.to_vec(),
            None => return Err(VmError::new(code::LIST_VAL_REQD)),
        };
        let kept = self.without(&items, other)?;
        Ok(self.new_list(kept))
    }

    fn without(&mut self, items: &[Value], other: Value) -> VmResult<Vec<Value>> {
        let remove: Vec<Value> = match self.list_like_len(other)? {
            Some(len) => {
                let mut values = Vec::with_capacity(len);
                for index in 1..=len {
                    values.push(self.list_like_get(other, index)?);
                }
                values
            }
            None => vec![other],
        };
        let mut kept = Vec::with_capacity(items.len());
        for &item in items {
            let mut keep = true;
            for &value in &remove {
                if self.equals(value, item)? {
                    keep = false;
                    break;
                }
            }
            if keep {
                kept.push(item);
            }
        }
        Ok(kept)
    }

    // ---- indexing ----

    /// Convert an index operand to an integer.
    fn index_arg(&self, index: Value) -> VmResult<i32> {
        match index {
            Value::Int(n) => Ok(n),
            _ => Err(VmError::new(code::NUM_VAL_REQD)),
        }
    }

    /// Index a value with the built-in rules. Returns `None` if the value
    /// has no built-in indexing, leaving it to an overloaded operator.
    pub(crate) fn index(&mut self, container: Value, index: Value) -> VmResult<Option<Value>> {
        if let Some(items) = self.as_list(container) {
            let n = self.index_arg(index)?;
            if n < 1 || n as usize > items.len() {
                return Err(VmError::new(code::INDEX_OUT_OF_RANGE));
            }
            return Ok(Some(items.get(n as usize - 1)));
        }
        let Value::Obj(id) = container else {
            return Ok(None);
        };
        match self.objects.get(id) {
            Some(Object::Vector(items) | Object::AnonFn(items)) => {
                let n = self.index_arg(index)?;
                if n < 1 || n as usize > items.len() {
                    return Err(VmError::new(code::INDEX_OUT_OF_RANGE));
                }
                Ok(Some(items[n as usize - 1]))
            }
            Some(Object::Opaque { .. }) => Err(self.unimplemented_object(id, "indexing")),
            _ => Ok(None),
        }
    }

    /// Assign to an element of a value with the built-in rules. Returns
    /// the container to store back (a new list, since lists never change
    /// in place), or `None` to leave it to an overloaded operator.
    pub(crate) fn set_index(
        &mut self,
        container: Value,
        index: Value,
        value: Value,
    ) -> VmResult<Option<Value>> {
        if let Some(items) = self.as_list(container) {
            let n = self.index_arg(index)?;
            if n < 1 || n as usize > items.len() {
                return Err(VmError::new(code::INDEX_OUT_OF_RANGE));
            }
            let mut items = items.to_vec();
            items[n as usize - 1] = value;
            return Ok(Some(self.new_list(items)));
        }
        let Value::Obj(id) = container else {
            return Ok(None);
        };
        match self.objects.get_mut(id) {
            Some(Object::Vector(items) | Object::AnonFn(items)) => {
                let n = match index {
                    Value::Int(n) => n,
                    _ => return Err(VmError::new(code::NUM_VAL_REQD)),
                };
                if n < 1 {
                    return Err(VmError::new(code::INDEX_OUT_OF_RANGE));
                }
                // Assigning past the end extends the vector.
                if n as usize > items.len() {
                    items.resize(n as usize, Value::Nil);
                }
                items[n as usize - 1] = value;
                Ok(Some(container))
            }
            Some(Object::Opaque { .. }) => Err(self.unimplemented_object(id, "indexing")),
            _ => Ok(None),
        }
    }

    /// An error for an operation on an object of an intrinsic class that
    /// is not implemented yet.
    pub(crate) fn unimplemented_object(&self, id: ObjId, what: &str) -> VmError {
        let class = self.kind_of(id).map_or("unknown", |kind| kind.name());
        VmError::unimplemented(format!("{what} for intrinsic class {class}"))
    }

    // ---- list-like values ----

    /// The length of a value that can be treated as a list: a list, a
    /// vector, or an object that overloads indexing and has a `length`
    /// method. `None` for anything else.
    pub(crate) fn list_like_len(&mut self, value: Value) -> VmResult<Option<usize>> {
        if let Some(items) = self.as_list(value) {
            return Ok(Some(items.len()));
        }
        let Value::Obj(id) = value else {
            return Ok(None);
        };
        match self.objects.get(id) {
            Some(Object::Vector(items) | Object::AnonFn(items)) => return Ok(Some(items.len())),
            Some(Object::Tads(_)) => {}
            _ => return Ok(None),
        }
        let (operator_idx, length) = (self.predef.operator_idx, self.predef.length);
        if operator_idx == INVALID_PROP || length == INVALID_PROP {
            return Ok(None);
        }
        if self.obj_get_prop(id, operator_idx, None)?.is_none() {
            return Ok(None);
        }
        match self.prop_interface(id, length)? {
            Some((0, _, _)) => {}
            _ => return Ok(None),
        }
        let native = NativeCall::named("Object.ll_length");
        match self.call_prop(value, length, 0, Some(native))? {
            Value::Int(n) if n >= 0 => Ok(Some(n as usize)),
            _ => Ok(None),
        }
    }

    /// The element at a one-based index of a list-like value.
    pub(crate) fn list_like_get(&mut self, value: Value, index: usize) -> VmResult<Value> {
        let index_value = Value::Int(index as i32);
        if let Some(found) = self.index(value, index_value)? {
            return Ok(found);
        }
        match value {
            Value::Obj(_) => {
                self.push(index_value);
                self.op_overload(
                    0,
                    None,
                    value,
                    self.predef.operator_idx,
                    1,
                    code::CANNOT_INDEX_TYPE,
                    Return::Native,
                )?;
                Ok(self.r0)
            }
            _ => Ok(Value::Nil),
        }
    }

    // ---- conversion to text ----

    /// The text of a value, as string concatenation and `toString` use
    /// it. Returns a string value along with the text; for a value that
    /// is already a string, the value itself.
    pub(crate) fn text_of(&mut self, value: Value) -> VmResult<(Value, std::rc::Rc<[u8]>)> {
        let text: String = match value {
            Value::SString(_) => {
                let text = self.require_str(value)?;
                return Ok((value, text.to_vec().into()));
            }
            Value::Obj(id) => return self.obj_to_text(id),
            Value::Int(n) => n.to_string(),
            Value::Nil => "nil".to_owned(),
            Value::True => "true".to_owned(),
            Value::List(_) => return self.list_to_text(value),
            Value::Prop(prop) => return self.reflect_to_text(value, format!("property#{prop}")),
            Value::FuncPtr(func) => {
                return self.reflect_to_text(value, format!("function#{func:x}"));
            }
            Value::Enum(n) => return self.reflect_to_text(value, format!("enum#{}", n as i32)),
            Value::BifPtr { set, index } => {
                return self.reflect_to_text(value, format!("builtin#{set}.{index}"));
            }
            _ => return Err(VmError::new(code::NO_STR_CONV)),
        };
        let text: std::rc::Rc<[u8]> = text.into_bytes().into();
        Ok((self.new_string(text.clone()), text))
    }

    fn obj_to_text(&mut self, id: ObjId) -> VmResult<(Value, std::rc::Rc<[u8]>)> {
        let value = Value::Obj(id);
        match self.objects.get(id) {
            Some(Object::String(text)) => return Ok((value, text.clone())),
            Some(Object::List(_)) => return self.list_to_text(value),
            Some(Object::Opaque { .. }) => {
                return Err(self.unimplemented_object(id, "conversion to text"));
            }
            _ => {}
        }
        // A program can define how its objects convert to text.
        if self.predef.obj_to_string != INVALID_PROP {
            let result = self.call_prop(value, self.predef.obj_to_string, 0, None)?;
            if let Some(text) = self.as_str(result) {
                return Ok((result, text.to_vec().into()));
            }
        }
        self.reflect_to_text(value, format!("object#{}", id.0))
    }

    /// Name a value through the program's reflection services, if it has
    /// them, and otherwise with a generic description.
    fn reflect_to_text(
        &mut self,
        value: Value,
        fallback: String,
    ) -> VmResult<(Value, std::rc::Rc<[u8]>)> {
        let services = self.predef.reflection_services;
        let prop = self.predef.reflection_val_to_symbol;
        if services.is_valid() && prop != INVALID_PROP {
            self.push(value);
            let result = self.call_prop(Value::Obj(services), prop, 1, None)?;
            if let Some(text) = self.as_str(result) {
                return Ok((result, text.to_vec().into()));
            }
        }
        let text: std::rc::Rc<[u8]> = fallback.into_bytes().into();
        Ok((self.new_string(text.clone()), text))
    }

    /// The text of a list: its elements' text, separated by commas.
    fn list_to_text(&mut self, list: Value) -> VmResult<(Value, std::rc::Rc<[u8]>)> {
        let len = self.list_like_len(list)?.unwrap_or(0);
        let mut text = Vec::new();
        for index in 1..=len {
            if index > 1 {
                text.push(b',');
            }
            let item = self.list_like_get(list, index)?;
            let (_, item_text) = self.text_of(item)?;
            text.extend_from_slice(&item_text);
        }
        let text: std::rc::Rc<[u8]> = text.into();
        Ok((self.new_string(text.clone()), text))
    }
}
