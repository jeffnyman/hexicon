//! The ByteArray built-in class: a fixed-size array of bytes that can
//! change in place.

use crate::error::{VmError, VmResult, code};
use crate::hash;
use crate::object::Object;
use crate::pack::Stream;
use crate::value::{NativeDesc, ObjId, Value};
use crate::vm::Vm;

/// The name and calling interface of each method, by method number.
const METHODS: &[(&str, NativeDesc)] = &[
    ("", NativeDesc::fixed(0)),
    ("length", NativeDesc::fixed(0)),
    ("subarray", NativeDesc::optional(1, 1)),
    ("copyFrom", NativeDesc::fixed(4)),
    ("fillValue", NativeDesc::optional(1, 2)),
    ("mapToString", NativeDesc::optional(0, 3)),
    ("readInt", NativeDesc::fixed(2)),
    ("writeInt", NativeDesc::fixed(3)),
    ("packBytes", NativeDesc::variable(2)),
    ("unpackBytes", NativeDesc::fixed(2)),
    ("sha256", NativeDesc::optional(0, 2)),
    ("digestMD5", NativeDesc::optional(0, 2)),
];

const PACK_BYTES: u16 = 8;

/// The storage of the reference VM is in pages of this size, which
/// shows in one place: see `digest_md5`.
const PAGE: usize = 32 * 1024;

/// A one-based starting index, kept within the array or just past it.
/// A negative index counts as too large, as it does when read as an
/// unsigned number.
fn start_index(index: i32, len: usize) -> usize {
    if index < 0 {
        len + 1
    } else {
        (index as usize).clamp(1, len + 1)
    }
}

/// A count of bytes from a one-based index, kept within the array.
fn span(index: usize, count: i32, len: usize) -> usize {
    let available = (len + 1).saturating_sub(index);
    if count < 0 {
        available
    } else {
        (count as usize).min(available)
    }
}

/// Bytes as text, each byte being the character with that code.
pub(crate) fn latin1_text(bytes: &[u8]) -> String {
    bytes.iter().map(|&b| char::from(b)).collect()
}

impl Vm {
    fn bytes(&self, id: ObjId) -> &[u8] {
        match self.objects.get(id) {
            Some(Object::ByteArray(bytes)) => bytes,
            _ => &[],
        }
    }

    pub(crate) fn bytes_mut(&mut self, id: ObjId) -> VmResult<&mut Vec<u8>> {
        match self.objects.get_mut(id) {
            Some(Object::ByteArray(bytes)) => Ok(bytes),
            _ => Err(VmError::new(code::BAD_TYPE_BIF)),
        }
    }

    /// The bytes of a string. With no character set, a character is the
    /// byte with its code, and none can be over 255.
    pub(crate) fn text_to_bytes(&mut self, text: &[u8], charset: Value) -> VmResult<Vec<u8>> {
        let text = String::from_utf8_lossy(text);
        match self.charset_arg(charset)? {
            Some(map) => Ok(map.encode(&text)),
            None => text
                .chars()
                .map(|c| u8::try_from(u32::from(c)).map_err(|_| VmError::new(code::NUM_OVERFLOW)))
                .collect(),
        }
    }

    /// Construct a byte array: `new ByteArray(length)`,
    /// `new ByteArray(string, charset?)`, or
    /// `new ByteArray(source, start?, length?)`.
    pub(crate) fn create_bytes(&mut self, argc: u32) -> VmResult<ObjId> {
        if argc < 1 {
            return Err(VmError::new(code::WRONG_NUM_OF_ARGS));
        }
        let first = self.arg(0);
        let bytes = if let Value::Int(len) = first {
            if argc != 1 {
                return Err(VmError::new(code::WRONG_NUM_OF_ARGS));
            }
            let len = usize::try_from(len).map_err(|_| VmError::new(code::OUT_OF_MEMORY))?;
            vec![0; len]
        } else if let Some(text) = self.as_str(first) {
            if argc > 2 {
                return Err(VmError::new(code::WRONG_NUM_OF_ARGS));
            }
            let text = text.to_vec();
            let charset = if argc >= 2 { self.arg(1) } else { Value::Nil };
            self.text_to_bytes(&text, charset)?
        } else if let Some(source) = self.as_bytes(first) {
            let source_len = source.len();
            let (start, len) = if argc == 1 {
                (1, source_len)
            } else {
                let start = self.int_arg(self.arg(1))?.max(1) as usize;
                let len = if argc >= 3 {
                    if argc > 3 {
                        return Err(VmError::new(code::WRONG_NUM_OF_ARGS));
                    }
                    usize::try_from(self.int_arg(self.arg(2))?)
                        .map_err(|_| VmError::new(code::OUT_OF_MEMORY))?
                } else {
                    (source_len + 1).saturating_sub(start)
                };
                (start, len)
            };
            // Past the end of the source, the new array is zeros.
            let source = self.as_bytes(first).map_or(&[][..], Vec::as_slice);
            let mut bytes = vec![0; len];
            let available = source.get(start - 1..).unwrap_or(&[]);
            let copied = len.min(available.len());
            bytes[..copied].copy_from_slice(&available[..copied]);
            bytes
        } else {
            return Err(VmError::new(code::BAD_TYPE_BIF));
        };
        Ok(self.new_bytes(bytes).obj_id())
    }

    /// The byte at an index.
    pub(crate) fn bytes_index(&self, id: ObjId, index: i32) -> VmResult<Value> {
        let bytes = self.bytes(id);
        if index < 1 || index as usize > bytes.len() {
            return Err(VmError::new(code::INDEX_OUT_OF_RANGE));
        }
        Ok(Value::Int(bytes[index as usize - 1].into()))
    }

    pub(crate) fn bytes_set_index(&mut self, id: ObjId, index: i32, value: i32) -> VmResult<()> {
        let bytes = self.bytes_mut(id)?;
        if index < 1 || index as usize > bytes.len() {
            return Err(VmError::new(code::INDEX_OUT_OF_RANGE));
        }
        bytes[index as usize - 1] =
            u8::try_from(value).map_err(|_| VmError::new(code::OUT_OF_RANGE))?;
        Ok(())
    }

    pub(crate) fn bytes_method(
        &mut self,
        id: ObjId,
        index: u16,
        argc: Option<&mut u32>,
    ) -> VmResult<Option<Value>> {
        let Some(&(_, desc)) = METHODS.get(index as usize).filter(|_| index != 0) else {
            return Ok(None);
        };
        self.native_method(argc, desc, |vm, argc| vm.bytes_call(id, index, argc))
            .map(Some)
    }

    /// `ByteArray.packBytes(format, ...)`, called on the class: a new
    /// byte array of the packed values.
    pub(crate) fn bytes_static_method(
        &mut self,
        index: u16,
        argc: Option<&mut u32>,
    ) -> VmResult<Option<Value>> {
        if index != PACK_BYTES {
            return Ok(None);
        }
        self.native_method(argc, NativeDesc::variable(1), |vm, argc| {
            let packed = vm.pack(0, argc as usize, Stream::growable())?;
            Ok(vm.new_bytes(packed.data))
        })
        .map(Some)
    }

    fn bytes_call(&mut self, id: ObjId, index: u16, argc: u32) -> VmResult<Value> {
        let self_value = Value::Obj(id);
        let len = self.bytes(id).len();
        Ok(match index {
            // length()
            1 => Value::Int(len as i32),
            // subarray(start, length?)
            2 => {
                let start = start_index(self.int_arg(self.arg(0))?, len);
                let count = if argc >= 2 {
                    self.int_arg(self.arg(1))?
                } else {
                    -1
                };
                let count = span(start, count, len);
                let part = self.bytes(id)[start - 1..start - 1 + count].to_vec();
                self.new_bytes(part)
            }
            // copyFrom(source, sourceStart, destStart, length)
            3 => {
                let source_id = self.obj_arg(self.arg(0))?;
                let Some(source) = self.as_bytes(self.arg(0)) else {
                    return Err(VmError::new(code::BAD_TYPE_BIF));
                };
                let source_start = start_index(self.int_arg(self.arg(1))?, source.len());
                let dest_start = start_index(self.int_arg(self.arg(2))?, len);
                let count = span(dest_start, self.int_arg(self.arg(3))?, len);
                // Past the end of the source, the copy is zeros. Copying
                // through a buffer makes an overlapping copy within one
                // array come out right.
                let mut chunk = vec![0; count];
                let available = self.bytes(source_id).get(source_start - 1..).unwrap_or(&[]);
                let copied = count.min(available.len());
                chunk[..copied].copy_from_slice(&available[..copied]);
                self.bytes_mut(id)?[dest_start - 1..dest_start - 1 + count].copy_from_slice(&chunk);
                self_value
            }
            // fillValue(value, start?, length?)
            4 => {
                let value = u8::try_from(self.int_arg(self.arg(0))?)
                    .map_err(|_| VmError::new(code::OUT_OF_RANGE))?;
                let start = if argc >= 2 {
                    start_index(self.int_arg(self.arg(1))?, len)
                } else {
                    1
                };
                let count = if argc >= 3 {
                    self.int_arg(self.arg(2))?
                } else {
                    -1
                };
                let count = span(start, count, len);
                self.bytes_mut(id)?[start - 1..start - 1 + count].fill(value);
                self_value
            }
            // mapToString(charset?, start?, length?)
            5 => {
                let charset = if argc >= 1 {
                    self.charset_arg(self.arg(0))?
                } else {
                    None
                };
                let start = if argc >= 2 {
                    start_index(self.int_arg(self.arg(1))?, len)
                } else {
                    1
                };
                let count = if argc >= 3 {
                    self.int_arg(self.arg(2))?
                } else {
                    -1
                };
                let count = span(start, count, len);
                let part = &self.bytes(id)[start - 1..start - 1 + count];
                let text = match charset {
                    Some(map) => map.decode(part),
                    None => latin1_text(part),
                };
                self.try_new_str(&text)?
            }
            // readInt(index, format)
            6 => {
                let at = self.int_arg(self.arg(0))?;
                let format = self.int_arg(self.arg(1))?;
                let size = int_size(format);
                if at < 1 || at as usize + size - 1 > len {
                    return Err(VmError::new(code::INDEX_OUT_OF_RANGE));
                }
                let at = at as usize - 1;
                let mut raw = self.bytes(id)[at..at + size].to_vec();
                if format & FORMAT_ORDER == FORMAT_BIG_ENDIAN {
                    raw.reverse();
                }
                let signed = format & FORMAT_SIGN == 0;
                Value::Int(match (size, signed) {
                    (1, true) => i32::from(raw[0] as i8),
                    (1, false) => i32::from(raw[0]),
                    (2, true) => i32::from(i16::from_le_bytes([raw[0], raw[1]])),
                    (2, false) => i32::from(u16::from_le_bytes([raw[0], raw[1]])),
                    _ => i32::from_le_bytes([raw[0], raw[1], raw[2], raw[3]]),
                })
            }
            // writeInt(index, format, value)
            7 => {
                let at = self.int_arg(self.arg(0))?;
                let format = self.int_arg(self.arg(1))?;
                let value = self.int_arg(self.arg(2))?;
                let size = int_size(format);
                if at < 1 || at as usize + size - 1 > len {
                    return Err(VmError::new(code::INDEX_OUT_OF_RANGE));
                }
                let mut raw = value.to_le_bytes()[..size].to_vec();
                if format & FORMAT_ORDER == FORMAT_BIG_ENDIAN {
                    raw.reverse();
                }
                let at = at as usize - 1;
                self.bytes_mut(id)?[at..at + size].copy_from_slice(&raw);
                Value::Nil
            }
            // packBytes(index, format, ...): the number of bytes written.
            8 => {
                let start = pack_start(self.int_arg(self.arg(0))?, len);
                let stream = Stream::fixed(self.bytes(id).to_vec(), start);
                let packed = self.pack(1, argc as usize - 1, stream)?;
                let written = packed.pos - start;
                *self.bytes_mut(id)? = packed.data;
                Value::Int(written as i32)
            }
            // unpackBytes(index, format)
            9 => {
                let start = pack_start(self.int_arg(self.arg(0))?, len);
                let stream = Stream::fixed(self.bytes(id).to_vec(), start);
                self.unpack(self.arg(1), stream)?
            }
            // sha256(start?, length?)
            10 => {
                let (start, count) = self.hash_range(argc, len)?;
                // The start can be anywhere in the array, and a length
                // that is too large takes what there is.
                let start = start.clamp(1, len as i64 + 1) as usize;
                let count = if count < 0 {
                    len + 1 - start
                } else {
                    (count as usize).min(len + 1 - start)
                };
                let digest = hash::sha256(&self.bytes(id)[start - 1..start - 1 + count]);
                self.try_new_str(&hash::hex(&digest))?
            }
            // digestMD5(start?, length?)
            11 => {
                let (start, count) = self.hash_range(argc, len)?;
                let start = start.clamp(1, len as i64 + 1) as usize;
                let count = count.clamp(0, (len + 1 - start) as i64) as usize;
                let digest = digest_md5(self.bytes(id), start, count);
                self.try_new_str(&hash::hex(&digest))?
            }
            _ => Value::Nil,
        })
    }

    /// The optional start and length arguments of the hash methods.
    fn hash_range(&self, argc: u32, len: usize) -> VmResult<(i64, i64)> {
        let start = if argc >= 1 {
            self.int_arg(self.arg(0))?
        } else {
            1
        };
        let count = if argc >= 2 {
            self.int_arg(self.arg(1))?
        } else {
            len as i32
        };
        Ok((start.into(), count.into()))
    }
}

/// The MD5 hash of part of a byte array.
///
/// The reference VM hashes a page of its storage at a time but never
/// moves on to the next page: a range that runs past the end of the page
/// it starts in hashes the first page's part over again instead. Hexicon
/// has no pages, but gives the same answer.
fn digest_md5(bytes: &[u8], start: usize, count: usize) -> [u8; 16] {
    let from = start - 1;
    let page_left = PAGE - from % PAGE;
    let mut data = Vec::with_capacity(count);
    let mut left = count;
    while left > 0 {
        let chunk = left.min(page_left);
        data.extend_from_slice(&bytes[from..from + chunk]);
        left -= chunk;
    }
    hash::md5(&data)
}

// The format argument of `readInt` and `writeInt`.
const FORMAT_SIZE: i32 = 0x000f;
const FORMAT_ORDER: i32 = 0x00f0;
const FORMAT_BIG_ENDIAN: i32 = 0x0010;
const FORMAT_SIGN: i32 = 0x0f00;

fn int_size(format: i32) -> usize {
    match format & FORMAT_SIZE {
        1 => 2,
        2 => 4,
        _ => 1,
    }
}

/// Where packing or unpacking starts, as an offset: the index is kept
/// within the array.
fn pack_start(index: i32, len: usize) -> usize {
    (index.max(1) as usize).min(len.max(1)) - 1
}
