//! The string buffer intrinsic class: text that changes in place.

use crate::error::{VmError, VmResult, code};
use crate::object::{Object, StrBuf};
use crate::value::{NativeDesc, ObjId, Value};
use crate::vm::Vm;

/// The name and calling interface of each method, by method number.
const METHODS: &[(&str, NativeDesc)] = &[
    ("", NativeDesc::fixed(0)),
    ("length", NativeDesc::fixed(0)),
    ("charAt", NativeDesc::fixed(1)),
    ("append", NativeDesc::fixed(1)),
    ("insert", NativeDesc::fixed(2)),
    ("copyChars", NativeDesc::fixed(2)),
    ("deleteChars", NativeDesc::optional(1, 2)),
    ("splice", NativeDesc::fixed(3)),
    ("substr", NativeDesc::optional(1, 2)),
];

/// Replace `delete` characters at `at` with `insert`, keeping both within
/// the buffer.
fn splice(buffer: &mut Vec<char>, at: i32, delete: i32, insert: &[char]) {
    let len = buffer.len() as i32;
    let at = at.clamp(0, len);
    let delete = delete.clamp(0, len - at);
    buffer.splice(at as usize..(at + delete) as usize, insert.iter().copied());
}

impl Vm {
    /// The contents of a string buffer.
    pub(crate) fn strbuf(&self, id: ObjId) -> Option<&Vec<char>> {
        match self.objects.get(id)? {
            Object::StringBuffer(buffer) => Some(buffer),
            _ => None,
        }
    }

    fn strbuf_mut(&mut self, id: ObjId) -> VmResult<&mut StrBuf> {
        match self.objects.get_mut(id) {
            Some(Object::StringBuffer(buffer)) => Ok(buffer),
            _ => Err(VmError::new(code::BAD_TYPE_BIF)),
        }
    }

    /// Replace characters of a string buffer, growing its room as needed.
    fn strbuf_splice(&mut self, id: ObjId, at: i32, delete: i32, insert: &[char]) -> VmResult<()> {
        let buffer = self.strbuf_mut(id)?;
        splice(&mut buffer.chars, at, delete, insert);
        buffer.grow();
        Ok(())
    }

    /// The contents of a string buffer as UTF-8 text.
    pub(crate) fn strbuf_text(&self, id: ObjId) -> Option<Vec<u8>> {
        self.strbuf(id)
            .map(|buffer| buffer.iter().collect::<String>().into_bytes())
    }

    /// Construct a string buffer. The arguments, an initial size and a
    /// growth increment, only tune storage, but the reference records
    /// them in an image or a saved game, so Hexicon keeps them as it does.
    pub(crate) fn create_strbuf(&mut self, argc: u32) -> VmResult<ObjId> {
        let (room, step) = match argc {
            0 => (256, 256),
            1 => {
                let room = self.int_arg(self.arg(0))?;
                let step = match room {
                    ..256 => room,
                    256..4096 => room / 2,
                    _ => 2048,
                };
                (room, step)
            }
            2 => (self.int_arg(self.arg(0))?, self.int_arg(self.arg(1))?),
            _ => return Err(VmError::new(code::WRONG_NUM_OF_ARGS)),
        };
        self.discard(argc as usize);
        let limit = |n: i32| (n.max(16) as u32).min(StrBuf::MAX_ROOM);
        Ok(self.alloc_obj(Object::StringBuffer(StrBuf::new(limit(room), limit(step)))))
    }

    /// The characters of a value converted to text.
    fn text_chars(&mut self, value: Value) -> VmResult<Vec<char>> {
        let text = self.cast_to_text(value)?;
        Ok(String::from_utf8_lossy(&text).chars().collect())
    }

    /// A zero-based position from a one-based index argument that counts
    /// from the end if negative. It may be out of range.
    fn strbuf_pos(&self, id: ObjId, index: i32) -> i32 {
        let len = self.strbuf(id).map_or(0, |buffer| buffer.len() as i32);
        index + if index < 0 { len } else { -1 }
    }

    /// Call a method of the string buffer class.
    pub(crate) fn strbuf_method(
        &mut self,
        id: ObjId,
        index: u16,
        argc: Option<&mut u32>,
    ) -> VmResult<Option<Value>> {
        let Some(&(_, desc)) = METHODS.get(index as usize).filter(|_| index != 0) else {
            return Ok(None);
        };
        self.native_method(argc, desc, |vm, argc| vm.strbuf_call(id, index, argc))
            .map(Some)
    }

    fn strbuf_call(&mut self, id: ObjId, index: u16, argc: u32) -> VmResult<Value> {
        let self_value = Value::Obj(id);
        let len = self.strbuf(id).map_or(0, |buffer| buffer.len() as i32);
        Ok(match index {
            // length()
            1 => Value::Int(len),
            // charAt(index): the character code.
            2 => {
                let at = self.strbuf_pos(id, self.int_arg(self.arg(0))?);
                if at < 0 || at >= len {
                    return Err(VmError::new(code::INDEX_OUT_OF_RANGE));
                }
                Value::Int(self.strbuf(id).map_or(0, |b| b[at as usize] as i32))
            }
            // append(val)
            3 => {
                let text = self.text_chars(self.arg(0))?;
                self.strbuf_splice(id, len, 0, &text)?;
                self_value
            }
            // insert(index, val)
            4 => {
                let at = self.strbuf_pos(id, self.int_arg(self.arg(0))?);
                let text = self.text_chars(self.arg(1))?;
                self.strbuf_splice(id, at, 0, &text)?;
                self_value
            }
            // copyChars(index, val): overwrite with the text.
            5 => {
                let at = self.strbuf_pos(id, self.int_arg(self.arg(0))?);
                let text = self.text_chars(self.arg(1))?;
                self.strbuf_splice(id, at, text.len() as i32, &text)?;
                self_value
            }
            // deleteChars(index, count?)
            6 => {
                let at = self.strbuf_pos(id, self.int_arg(self.arg(0))?);
                let count = if argc >= 2 {
                    self.int_arg(self.arg(1))?
                } else {
                    len
                };
                self.strbuf_splice(id, at, count, &[])?;
                self_value
            }
            // splice(index, deleteCount, val)
            7 => {
                let at = self.strbuf_pos(id, self.int_arg(self.arg(0))?);
                let delete = self.int_arg(self.arg(1))?;
                let text = self.text_chars(self.arg(2))?;
                self.strbuf_splice(id, at, delete, &text)?;
                self_value
            }
            // substr(index, count?)
            8 => {
                let at = self.strbuf_pos(id, self.int_arg(self.arg(0))?);
                let count = if argc >= 2 {
                    self.int_arg(self.arg(1))?
                } else {
                    len
                };
                self.strbuf_substr(id, at, count)
            }
            _ => return Err(VmError::new(code::BAD_TYPE_BIF)),
        })
    }

    /// Part of a buffer, as a new string.
    fn strbuf_substr(&mut self, id: ObjId, at: i32, count: i32) -> Value {
        let text: String = match self.strbuf(id) {
            Some(buffer) => {
                let len = buffer.len() as i32;
                let at = at.clamp(0, len);
                let count = count.clamp(0, len - at);
                buffer[at as usize..(at + count) as usize].iter().collect()
            }
            None => String::new(),
        };
        self.new_str(&text)
    }

    /// Index a string buffer: the character at a position, as a string.
    pub(crate) fn strbuf_index(&mut self, id: ObjId, index: Value) -> VmResult<Value> {
        let Value::Int(index) = index else {
            return Err(VmError::new(code::NUM_VAL_REQD));
        };
        let at = self.strbuf_pos(id, index);
        if at < 0 || at >= self.strbuf(id).map_or(0, |b| b.len() as i32) {
            return Err(VmError::new(code::INDEX_OUT_OF_RANGE));
        }
        Ok(self.strbuf_substr(id, at, 1))
    }

    /// Assign to a position of a string buffer: a character code, or the
    /// first character of a string.
    pub(crate) fn strbuf_set_index(
        &mut self,
        id: ObjId,
        index: Value,
        value: Value,
    ) -> VmResult<()> {
        let Value::Int(index) = index else {
            return Err(VmError::new(code::NUM_VAL_REQD));
        };
        let at = self.strbuf_pos(id, index);
        if at < 0 || at >= self.strbuf(id).map_or(0, |b| b.len() as i32) {
            return Err(VmError::new(code::INDEX_OUT_OF_RANGE));
        }
        let c = match value {
            Value::Int(n) => {
                if !(0..=65535).contains(&n) {
                    return Err(VmError::new(code::BAD_VAL_BIF));
                }
                char::from_u32(n as u32).unwrap_or('\u{fffd}')
            }
            other => {
                let text = self.require_str(other)?;
                String::from_utf8_lossy(&text)
                    .chars()
                    .next()
                    .unwrap_or('\0')
            }
        };
        self.strbuf_mut(id)?.chars[at as usize] = c;
        Ok(())
    }
}
