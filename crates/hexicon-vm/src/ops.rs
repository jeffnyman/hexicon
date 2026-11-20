//! Operators: equality, ordering, arithmetic, indexing, and conversion
//! to text.

use std::cmp::Ordering;

use crate::bignum_obj::Arith;
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

    /// The text of a string or a string buffer.
    fn text_or_buffer(&self, value: Value) -> Option<Vec<u8>> {
        if let Some(text) = self.as_str(value) {
            return Some(text.to_vec());
        }
        match value {
            Value::Obj(id) => self.strbuf_text(id),
            _ => None,
        }
    }

    fn obj_equals(&mut self, id: ObjId, other: Value) -> VmResult<bool> {
        match self.objects.get(id) {
            Some(Object::String(text)) => {
                let text = text.clone();
                Ok(match self.text_or_buffer(other) {
                    Some(other) => *other == *text,
                    None => false,
                })
            }
            Some(Object::BigNumber(big)) => {
                let equal = self.big_equals(big, other);
                self.big_operand(other);
                Ok(equal)
            }
            Some(Object::CharSet { .. }) => Ok(self.charset_equals(id, other)),
            Some(Object::FileName(_)) => Ok(self.file_name_equals(id, other)),
            // Dates are equal if they are the same moment.
            Some(Object::Date { .. }) => Ok(self.as_date(Value::Obj(id)) == self.as_date(other)),
            // Byte arrays are equal if they hold the same bytes.
            Some(Object::ByteArray(bytes)) => {
                Ok(other == Value::Obj(id)
                    || self.as_bytes(other).is_some_and(|other| other == bytes))
            }
            // A string buffer compares by its text, and only with text.
            Some(Object::StringBuffer(_)) => {
                let text = self.strbuf_text(id).unwrap_or_default();
                match self.text_or_buffer(other) {
                    Some(other) => Ok(other == text),
                    None => Err(VmError::new(code::INVALID_COMPARISON)),
                }
            }
            Some(Object::List(_)) => self.list_equals(Value::Obj(id), other),
            // A vector equals any list-like value with equal elements.
            Some(Object::Vector(items)) => {
                if other == Value::Obj(id) {
                    return Ok(true);
                }
                let items = items.clone();
                let Some(len) = self.list_like_len(other)? else {
                    return Ok(false);
                };
                if len != items.len() {
                    return Ok(false);
                }
                for (index, &item) in items.iter().enumerate() {
                    let theirs = self.list_like_get(other, index + 1)?;
                    if !self.equals(item, theirs)? {
                        return Ok(false);
                    }
                }
                Ok(true)
            }
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
            // A string compares with a string or a string buffer.
            (Value::SString(_), _) => match (self.as_str(a), self.text_or_buffer(b)) {
                (Some(x), Some(y)) => Ok((*x).cmp(y.as_slice())),
                _ => Err(VmError::new(code::INVALID_COMPARISON)),
            },
            (_, Value::Obj(id)) => Ok(self.obj_compare(id, a)?.reverse()),
            _ => Err(VmError::new(code::INVALID_COMPARISON)),
        }
    }

    fn obj_compare(&mut self, id: ObjId, other: Value) -> VmResult<Ordering> {
        match self.objects.get(id) {
            Some(Object::BigNumber(big)) => {
                let order = self.big_compare(big, other);
                self.big_operand(other);
                order
            }
            Some(Object::String(_) | Object::StringBuffer(_)) => {
                let text = self.text_or_buffer(Value::Obj(id)).unwrap_or_default();
                match self.text_or_buffer(other) {
                    Some(other) => Ok(text.cmp(&other)),
                    None => Err(VmError::new(code::INVALID_COMPARISON)),
                }
            }
            // A date compares with a date: the earlier is the lesser.
            Some(Object::Date { dayno, daytime }) => match self.as_date(other) {
                Some(other) => Ok((*dayno, *daytime).cmp(&other)),
                None => Err(VmError::new(code::INVALID_COMPARISON)),
            },
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
                Some(Object::BigNumber(big)) => {
                    let big = big.clone();
                    self.big_arith(Arith::Add, &big, b).map(Some)
                }
                Some(Object::String(_)) => self.concat(a, b).map(Some),
                Some(Object::FileName(_)) => self.file_name_add(id, b).map(Some),
                Some(Object::Date { .. }) => self.date_add(id, b).map(Some),
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
            Value::Int(n) => self.int_with_big(Arith::Add, n, b).map(Some),
            _ => Ok(None),
        }
    }

    /// Arithmetic with an integer on the left and something other than
    /// an integer on the right, which has to be a BigNumber.
    fn int_with_big(&mut self, op: Arith, a: i32, b: Value) -> VmResult<Value> {
        if self.as_big(b).is_none() {
            return Err(VmError::new(code::NUM_VAL_REQD));
        }
        let a = crate::bignum::Big::from_int(a.into(), 10);
        self.new_big(a.clone());
        self.big_arith(op, &a, b)
    }

    /// Subtract, under the same terms as [`Vm::add`].
    pub(crate) fn sub(&mut self, a: Value, b: Value) -> VmResult<Option<Value>> {
        match a {
            Value::List(_) => self.list_remove(a, b).map(Some),
            Value::Obj(id) => match self.objects.get(id) {
                Some(Object::BigNumber(big)) => {
                    let big = big.clone();
                    self.big_arith(Arith::Sub, &big, b).map(Some)
                }
                Some(Object::Date { .. }) => self.date_sub(id, b).map(Some),
                Some(Object::List(_)) => self.list_remove(a, b).map(Some),
                Some(Object::Vector(items)) => {
                    let items = items.clone();
                    let kept = self.without(&items, b)?;
                    Ok(Some(Value::Obj(self.objects.alloc(Object::Vector(kept)))))
                }
                _ => Ok(None),
            },
            Value::Int(n) => self.int_with_big(Arith::Sub, n, b).map(Some),
            _ => Ok(None),
        }
    }

    /// Multiply or divide values that are not both integers. Nothing but
    /// numbers has built-in multiplication or division.
    pub(crate) fn mul_div(&mut self, op: Arith, a: Value, b: Value) -> VmResult<Option<Value>> {
        match a {
            Value::Int(n) => self.int_with_big(op, n, b).map(Some),
            _ => match self.as_big(a).cloned() {
                Some(big) => self.big_arith(op, &big, b).map(Some),
                None => Ok(None),
            },
        }
    }

    /// Concatenate a string with the text of another value.
    fn concat(&mut self, string: Value, other: Value) -> VmResult<Value> {
        let left = self.require_str(string)?;
        let (_, right) = self.text_raw(other)?;
        if right.is_empty() || other.is_nil() {
            return Ok(string);
        }
        if left.is_empty() && self.as_str(other).is_some() {
            return Ok(other);
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
            other => match self.as_big(other) {
                Some(big) => Self::big_to_int(big),
                None => Err(VmError::new(code::NUM_VAL_REQD)),
            },
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
            Some(Object::LookupTable(_)) => self.table_index(id, index).map(Some),
            Some(Object::StringBuffer(_)) => self.strbuf_index(id, index).map(Some),
            Some(Object::ByteArray(_)) => {
                let n = self.index_arg(index)?;
                self.bytes_index(id, n).map(Some)
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
            Some(Object::LookupTable(_)) => {
                self.table_set(id, index, value)?;
                Ok(Some(container))
            }
            Some(Object::StringBuffer(_)) => {
                self.strbuf_set_index(id, index, value)?;
                Ok(Some(container))
            }
            Some(Object::ByteArray(_)) => {
                let n = self.index_arg(index)?;
                let byte = self.index_arg(value)?;
                self.bytes_set_index(id, n, byte)?;
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
            // An anonymous function is stored like a vector but is not a list.
            Some(Object::Vector(items)) => return Ok(Some(items.len())),
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

    // ---- conversion to text ----
    //
    // There are three conversions, as in the reference VM, and they
    // differ in which string objects they create along the way. That
    // matters: a program can see how many objects have been created, in
    // the IDs of the ones it creates next.

    /// The text of a value, for when only the text is wanted.
    pub(crate) fn text_bytes(&mut self, value: Value) -> VmResult<std::rc::Rc<[u8]>> {
        Ok(self.text_raw(value)?.1)
    }

    /// The text of a value as an explicit conversion gives it, which is
    /// also how the right side of a string concatenation is converted.
    /// Along with the text comes the string value that holds it, if there
    /// is one: a number or a truth value is converted without creating a
    /// string.
    pub(crate) fn text_raw(
        &mut self,
        value: Value,
    ) -> VmResult<(Option<Value>, std::rc::Rc<[u8]>)> {
        let text: &[u8] = match value {
            Value::Int(n) => return Ok((None, n.to_string().into_bytes().into())),
            Value::Nil => b"nil",
            Value::True => b"true",
            _ => {
                let (string, text) = self.text_made(value)?;
                return Ok((Some(string), text));
            }
        };
        Ok((None, text.into()))
    }

    /// The text of a value, as `toString` returns it: always with a
    /// string value, created if need be.
    pub(crate) fn text_of(&mut self, value: Value) -> VmResult<(Value, std::rc::Rc<[u8]>)> {
        match self.text_raw(value)? {
            (Some(string), text) => Ok((string, text)),
            (None, text) => Ok((self.new_string(text.clone()), text)),
        }
    }

    /// The text of a value where a string is simply expected, as when a
    /// value is displayed or passed to `concat`. A number becomes a new
    /// string, and `nil` an empty one. A list becomes its elements joined
    /// with commas, in one new string.
    pub(crate) fn text_cast(&mut self, value: Value) -> VmResult<(Value, std::rc::Rc<[u8]>)> {
        match value {
            Value::Nil => {
                let text: std::rc::Rc<[u8]> = Vec::new().into();
                Ok((self.new_string(text.clone()), text))
            }
            Value::Int(_) | Value::True => self.text_of(value),
            _ if self.list_like_len(value)?.is_some() => self.join_text(value, b","),
            _ => self.text_of(value),
        }
    }

    /// The elements of a list as text with a separator between, in one
    /// new string. Strings, integers, and `nil` go in as they are; any
    /// other element is first converted as [`Vm::text_cast`] does it.
    pub(crate) fn join_text(
        &mut self,
        list: Value,
        separator: &[u8],
    ) -> VmResult<(Value, std::rc::Rc<[u8]>)> {
        let len = self.list_like_len(list)?.unwrap_or(0);
        // The reference creates the result before it converts anything.
        let result = self.new_string(Vec::new());
        self.push(result);
        let mut text = Vec::new();
        let mut failed = None;
        for index in 1..=len {
            if index > 1 {
                text.extend_from_slice(separator);
            }
            let piece = match self.list_like_get(list, index) {
                Ok(Value::Nil) => Ok(Vec::new().into()),
                Ok(item @ Value::Int(_)) => self.text_bytes(item),
                Ok(item) if self.as_str(item).is_some() => self.text_bytes(item),
                Ok(item) => self.text_cast(item).map(|(_, text)| text),
                Err(error) => Err(error),
            };
            match piece {
                Ok(piece) => text.extend_from_slice(&piece),
                Err(error) => {
                    failed = Some(error);
                    break;
                }
            }
        }
        self.discard(1);
        if let Some(error) = failed {
            return Err(error);
        }
        let text: std::rc::Rc<[u8]> = text.into();
        if let Some(Object::String(stored)) = self.objects.get_mut(result.obj_id()) {
            *stored = text.clone();
        }
        Ok((result, text))
    }

    /// The text of a value that is not a number or a truth value, with
    /// the string value that holds it.
    fn text_made(&mut self, value: Value) -> VmResult<(Value, std::rc::Rc<[u8]>)> {
        match value {
            Value::SString(_) => {
                let text = self.require_str(value)?;
                Ok((value, text.to_vec().into()))
            }
            Value::Obj(id) => self.obj_to_text(id),
            Value::List(_) => self.list_to_text(value),
            Value::Prop(prop) => self.reflect_to_text(value, format!("property#{prop}")),
            Value::FuncPtr(func) => self.reflect_to_text(value, format!("function#{func:x}")),
            Value::Enum(n) => self.reflect_to_text(value, format!("enum#{}", n as i32)),
            Value::BifPtr { set, index } => {
                self.reflect_to_text(value, format!("builtin#{set}.{index}"))
            }
            _ => Err(VmError::new(code::NO_STR_CONV)),
        }
    }

    fn obj_to_text(&mut self, id: ObjId) -> VmResult<(Value, std::rc::Rc<[u8]>)> {
        let value = Value::Obj(id);
        match self.objects.get(id) {
            Some(Object::String(text)) => return Ok((value, text.clone())),
            Some(Object::List(_) | Object::Vector(_)) => return self.list_to_text(value),
            Some(Object::BigNumber(big)) => {
                let text: std::rc::Rc<[u8]> = big.to_text().into_bytes().into();
                return Ok((self.new_string(text.clone()), text));
            }
            // A byte array converts a byte to the character with its code.
            Some(Object::ByteArray(bytes)) => {
                let text: std::rc::Rc<[u8]> =
                    crate::bytearray::latin1_text(bytes).into_bytes().into();
                return Ok((self.new_string(text.clone()), text));
            }
            Some(Object::StringBuffer(_)) => {
                let text: std::rc::Rc<[u8]> = self.strbuf_text(id).unwrap_or_default().into();
                return Ok((self.new_string(text.clone()), text));
            }
            // A date converts to the date and time in the local zone, and
            // a time zone to its name.
            Some(Object::Date { .. }) => {
                let text: std::rc::Rc<[u8]> = self.date_text(id).into_bytes().into();
                return Ok((self.new_string(text.clone()), text));
            }
            Some(Object::TimeZone(_)) => {
                let name = self.zone_name(id).unwrap_or_default();
                let text: std::rc::Rc<[u8]> = name.into_bytes().into();
                return Ok((self.new_string(text.clone()), text));
            }
            // A file name converts to the name.
            Some(Object::FileName(name)) => {
                let text: std::rc::Rc<[u8]> = name.path.clone().into_bytes().into();
                return Ok((self.new_string(text.clone()), text));
            }
            // A pattern converts to the text it was compiled from.
            Some(Object::Pattern { source, .. }) => {
                let source = *source;
                return self.text_of(source);
            }
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
        // The reference counts the byte that ends the text it formats as
        // part of the string, so the string has a null character at its
        // end. A program can see that, in the string's length for one.
        let mut bytes = fallback.into_bytes();
        bytes.push(0);
        let text: std::rc::Rc<[u8]> = bytes.into();
        Ok((self.new_string(text.clone()), text))
    }

    /// The text of a list as an explicit conversion gives it: each
    /// element converted as `toString` does it, joined with commas. The
    /// reference builds a list of the converted elements on the way, and
    /// so does this.
    fn list_to_text(&mut self, list: Value) -> VmResult<(Value, std::rc::Rc<[u8]>)> {
        let len = self.list_like_len(list)?.unwrap_or(0);
        let converted = self.new_list(Vec::new());
        self.push(converted);
        let mut items = Vec::with_capacity(len);
        let mut failed = None;
        for index in 1..=len {
            let item = match self.list_like_get(list, index) {
                Ok(item) => self.text_raw(item),
                Err(error) => Err(error),
            };
            match item {
                // An element that is not already a string object becomes
                // one.
                Ok((Some(string @ Value::Obj(_)), _)) => items.push(string),
                Ok((_, text)) => {
                    let string = self.new_string(text);
                    items.push(string);
                }
                Err(error) => {
                    failed = Some(error);
                    break;
                }
            }
            if let Some(Object::List(stored)) = self.objects.get_mut(converted.obj_id()) {
                *stored = items.clone().into();
            }
        }
        let joined = match failed {
            Some(error) => Err(error),
            None => self.join_text(converted, b","),
        };
        self.discard(1);
        joined
    }
}
