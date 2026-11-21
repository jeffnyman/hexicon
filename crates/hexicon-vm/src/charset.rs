//! Character sets: converting between text and the bytes of an encoding,
//! and the CharacterSet intrinsic class that names one.
//!
//! UTF-8 and UTF-16 are built in. Every other set is a table from the
//! reference VM's collection, embedded in the VM (see
//! `tools/gen-charmaps.py`). A table does more than list the characters
//! a set has: it also gives a stand-in for many it lacks, so that `à`
//! becomes `a` in ASCII instead of a question mark.

use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use crate::error::{VmError, VmResult, code};
use crate::object::Object;
use crate::value::{NativeDesc, ObjId, Value};
use crate::vm::Vm;

/// The embedded tables: for each, a name length, the name, the table's
/// length, and the table.
static TABLES: &[u8] = include_bytes!("../data/charmaps.bin");

/// A mapping between text and bytes.
#[derive(Debug)]
pub(crate) enum CharMap {
    Utf8,
    Utf16 { big_endian: bool },
    Table(Box<Table>),
}

/// A character set defined by a table.
#[derive(Debug)]
pub(crate) struct Table {
    /// The bytes for each character the table knows, including the
    /// stand-ins for characters the set does not really have.
    to_bytes: HashMap<u16, Vec<u8>>,
    /// The bytes for any other character.
    unknown: Vec<u8>,
    /// The character for each byte.
    to_char: [u16; 256],
    /// Characters the console would spell out with several others, such
    /// as `(c)` for the copyright sign. They do not survive a round trip.
    expanded: HashSet<u16>,
}

const REPLACEMENT: u16 = 0xfffd;

fn u16_at(data: &[u8], at: usize) -> Option<u16> {
    let b = data.get(at..at + 2)?;
    Some(u16::from_le_bytes([b[0], b[1]]))
}

fn u32_at(data: &[u8], at: usize) -> Option<usize> {
    let b = data.get(at..at + 4)?;
    Some(u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as usize)
}

impl Table {
    /// Read a table in the reference's compiled format.
    fn parse(data: &[u8]) -> Option<Self> {
        // First, pairs of a character and its byte.
        let to_bytes_at = u32_at(data, 0)?;
        let pair_count = usize::from(u16_at(data, 4)?);
        let mut to_char = [REPLACEMENT; 256];
        for index in 0..pair_count {
            let at = 6 + index * 4;
            let character = u16_at(data, at)?;
            let local = u16_at(data, at + 2)?;
            if let Some(slot) = to_char.get_mut(usize::from(local)) {
                *slot = character;
            }
        }

        // Then each character and the bytes to write for it. The first
        // entry is what to write for a character that has no entry.
        let entry_count = usize::from(u16_at(data, to_bytes_at)?);
        let mut at = to_bytes_at + 6;
        let mut to_bytes = HashMap::with_capacity(entry_count);
        let mut unknown = vec![b'?'];
        for index in 0..entry_count {
            let character = u16_at(data, at)?;
            let len = usize::from(*data.get(at + 2)?);
            let bytes = data.get(at + 3..at + 3 + len)?.to_vec();
            at += 3 + len;
            if index == 0 {
                unknown = bytes;
            } else {
                to_bytes.insert(character, bytes);
            }
        }

        // Last, the characters with expansions.
        let mut expanded = HashSet::new();
        if data.get(at..at + 4).is_some_and(|mark| mark != b"$EOF") {
            let count = usize::from(u16_at(data, at)?);
            at += 6;
            for _ in 0..count {
                let character = u16_at(data, at)?;
                let len = usize::from(*data.get(at + 2)?);
                at += 3 + len * 2;
                expanded.insert(character);
            }
        }
        Some(Self {
            to_bytes,
            unknown,
            to_char,
            expanded,
        })
    }

    fn bytes_of(&self, c: char) -> &[u8] {
        u16::try_from(u32::from(c))
            .ok()
            .and_then(|code| self.to_bytes.get(&code))
            .unwrap_or(&self.unknown)
    }
}

/// Find an embedded table by its file name.
fn embedded(name: &str) -> Option<&'static [u8]> {
    let mut rest = TABLES;
    while let Some((&name_len, after)) = rest.split_first() {
        let (entry_name, after) = after.split_at_checked(usize::from(name_len))?;
        let len = u32_at(after, 0)?;
        let (data, after) = after.get(4..)?.split_at_checked(len)?;
        if entry_name == name.as_bytes() {
            return Some(data);
        }
        rest = after;
    }
    None
}

/// Find the table for a character set name, trying the usual other
/// spellings: `latin-2` and `iso-8859-2` are `iso2`, and `windows-1252`
/// and plain `1252` are `cp1252`.
fn find_table(name: &str) -> Option<&'static [u8]> {
    if let Some(table) = embedded(name) {
        return Some(table);
    }
    let prefix = name.trim_end_matches(|c: char| c.is_ascii_digit());
    let digits = &name[prefix.len()..];
    let number: u32 = digits.parse().unwrap_or(0);
    // The reference compares the two lengths here, which leaves out a
    // name like `l1`. That one is found as a name for Latin-1 later.
    if prefix.len() == digits.len() || number == 0 {
        return None;
    }
    const ISO: [&str; 10] = [
        "latin",
        "latin-",
        "iso",
        "iso-",
        "8859-",
        "iso8859-",
        "iso-8859-",
        "iso_8859-",
        "iso_8859_",
        "l",
    ];
    const CODE_PAGE: [&str; 7] = ["", "win", "win-", "windows", "windows-", "dos", "dos-"];
    if ISO.contains(&prefix) {
        if let Some(table) = embedded(&format!("iso{number}")) {
            return Some(table);
        }
    }
    if CODE_PAGE.contains(&prefix) {
        return embedded(&format!("cp{number}"));
    }
    None
}

/// Text in a named character set, as a string; `None` if the name is not
/// one of a character set that is known. A byte that the set has no
/// character for becomes U+FFFD.
///
/// This is for the compiler, which reads source files in whatever
/// character set they say they are in, and uses the same tables as
/// programs do.
pub fn decode_text(charset: &str, bytes: &[u8]) -> Option<String> {
    let map = CharMap::load(charset)?;
    Some(match map {
        // Source text keeps what is there; it is not a program's string.
        CharMap::Utf8 => String::from_utf8_lossy(bytes).into_owned(),
        other => other.decode(bytes),
    })
}

impl CharMap {
    /// The mapping for a character set name, if the name is known.
    pub(crate) fn load(name: &str) -> Option<Self> {
        let name = name.to_ascii_lowercase();
        let name = name.as_str();
        if matches!(name, "utf-8" | "utf8") {
            return Some(Self::Utf8);
        }
        const UTF16_LE: [&str; 8] = [
            "utf-16le",
            "utf16le",
            "utf_16le",
            "unicodel",
            "unicode-l",
            "unicode-le",
            "ucs-2le",
            "ucs2le",
        ];
        const UTF16_BE: [&str; 8] = [
            "utf-16be",
            "utf16be",
            "utf_16be",
            "unicodeb",
            "unicode-b",
            "unicode-be",
            "ucs-2be",
            "ucs2be",
        ];
        if UTF16_LE.contains(&name) {
            return Some(Self::Utf16 { big_endian: false });
        }
        if UTF16_BE.contains(&name) {
            return Some(Self::Utf16 { big_endian: true });
        }
        const ASCII: [&str; 7] = [
            "us-ascii",
            "asc7dflt",
            "ascii",
            "iso646-us",
            "iso-ir-6",
            "cp367",
            "us",
        ];
        const LATIN1: [&str; 13] = [
            "iso-8859-1",
            "iso_8859-1",
            "iso_8859_1",
            "iso8859-1",
            "iso8859_1",
            "8859-1",
            "8859_1",
            "iso-ir-100",
            "latin1",
            "latin-1",
            "l1",
            "iso1",
            "cp819",
        ];
        let table = find_table(name).or_else(|| {
            // For these names with no table of their own, the reference
            // has two maps built in, which the embedded tables include.
            if ASCII.contains(&name) {
                embedded("#ascii")
            } else if LATIN1.contains(&name) {
                embedded("#latin1")
            } else {
                None
            }
        })?;
        Table::parse(table).map(|table| Self::Table(Box::new(table)))
    }

    /// The bytes of text in this character set.
    pub(crate) fn encode(&self, text: &str) -> Vec<u8> {
        match self {
            Self::Utf8 => text.as_bytes().to_vec(),
            Self::Utf16 { big_endian } => text
                .chars()
                .flat_map(|c| {
                    let unit = u32::from(c) as u16;
                    if *big_endian {
                        unit.to_be_bytes()
                    } else {
                        unit.to_le_bytes()
                    }
                })
                .collect(),
            Self::Table(table) => text
                .chars()
                .flat_map(|c| table.bytes_of(c).iter().copied())
                .collect(),
        }
    }

    /// The text that bytes in this character set stand for.
    pub(crate) fn decode(&self, bytes: &[u8]) -> String {
        let from_unit = |unit: u16| char::from_u32(unit.into()).unwrap_or('\u{fffd}');
        match self {
            // Anything that is not UTF-8 becomes a question mark, and an
            // unfinished character at the end is left off.
            Self::Utf8 => {
                let mut end = bytes.len();
                for back in 1..=bytes.len().min(3) {
                    let byte = bytes[bytes.len() - back];
                    if byte & 0xc0 != 0x80 {
                        let need = match byte {
                            0xc0..=0xdf => 2,
                            0xe0..=0xff => 3,
                            _ => 1,
                        };
                        if need > back {
                            end = bytes.len() - back;
                        }
                        break;
                    }
                }
                String::from_utf8_lossy(&bytes[..end]).replace('\u{fffd}', "?")
            }
            Self::Utf16 { big_endian } => bytes
                .chunks_exact(2)
                .map(|pair| {
                    from_unit(if *big_endian {
                        u16::from_be_bytes([pair[0], pair[1]])
                    } else {
                        u16::from_le_bytes([pair[0], pair[1]])
                    })
                })
                .collect(),
            Self::Table(table) => bytes
                .iter()
                .map(|&byte| from_unit(table.to_char[usize::from(byte)]))
                .collect(),
        }
    }

    /// Whether the set has the character, or a stand-in for it.
    ///
    /// The reference VM answers no for every character in UTF-16, which
    /// has them all: its UTF-16 mapping never says otherwise. A program
    /// can see the answer, so it is the same here.
    fn is_mappable(&self, c: char) -> bool {
        match self {
            Self::Table(table) => {
                u16::try_from(u32::from(c)).is_ok_and(|code| table.to_bytes.contains_key(&code))
            }
            Self::Utf8 => true,
            Self::Utf16 { .. } => false,
        }
    }

    /// Whether the character comes back unchanged from being converted
    /// to bytes and back.
    fn is_round_trip(&self, c: char) -> bool {
        if !self.is_mappable(c) {
            return false;
        }
        if let Self::Table(table) = self {
            if u16::try_from(u32::from(c)).is_ok_and(|code| table.expanded.contains(&code)) {
                return false;
            }
        }
        let mut text = [0; 4];
        let text = c.encode_utf8(&mut text);
        self.decode(&self.encode(text)) == *text
    }
}

/// The name and calling interface of each method, by method number.
const METHODS: &[(&str, NativeDesc)] = &[
    ("", NativeDesc::fixed(0)),
    ("getName", NativeDesc::fixed(0)),
    ("isMappingKnown", NativeDesc::fixed(0)),
    ("isMappable", NativeDesc::fixed(1)),
    ("isRoundTripMappable", NativeDesc::fixed(1)),
];

impl Vm {
    /// Make a CharacterSet object for a name. The name need not be one
    /// the VM knows: that only matters when the set is used.
    pub(crate) fn new_charset(&mut self, name: &[u8]) -> ObjId {
        let name = String::from_utf8_lossy(name).into_owned();
        let map = CharMap::load(&name).map(Rc::new);
        self.alloc_obj(Object::CharSet { name, map })
    }

    /// `new CharacterSet(name)`.
    pub(crate) fn create_charset(&mut self, argc: u32) -> VmResult<ObjId> {
        if argc != 1 {
            return Err(VmError::new(code::WRONG_NUM_OF_ARGS));
        }
        let Some(name) = self.as_str(self.arg(0)) else {
            return Err(VmError::new(code::BAD_TYPE_BIF));
        };
        let name = name.to_vec();
        Ok(self.new_charset(&name))
    }

    /// The mapping an argument asks for: a CharacterSet, the name of
    /// one, or nil for none. Asking for a set the VM does not know throws
    /// the program's exception for that.
    pub(crate) fn charset_arg(&mut self, value: Value) -> VmResult<Option<Rc<CharMap>>> {
        let id = match value {
            Value::Nil => return Ok(None),
            Value::Obj(id) if matches!(self.objects.get(id), Some(Object::CharSet { .. })) => id,
            other => match self.as_str(other) {
                Some(name) => {
                    let name = name.to_vec();
                    self.new_charset(&name)
                }
                None => return Err(VmError::new(code::BAD_TYPE_BIF)),
            },
        };
        self.charset_map(id).map(Some)
    }

    pub(crate) fn charset_map(&mut self, id: ObjId) -> VmResult<Rc<CharMap>> {
        if let Some(Object::CharSet { map: Some(map), .. }) = self.objects.get(id) {
            return Ok(map.clone());
        }
        let class = self.predef.charset_unknown_exception;
        Err(self.throw_class(class, "unknown character set"))
    }

    /// Whether two character sets have the same name, in any case.
    pub(crate) fn charset_equals(&self, id: ObjId, other: Value) -> bool {
        let name_of = |id: ObjId| match self.objects.get(id) {
            Some(Object::CharSet { name, .. }) => Some(name.as_str()),
            _ => None,
        };
        match (name_of(id), other) {
            (Some(name), Value::Obj(other)) => {
                other == id || name_of(other).is_some_and(|other| other.eq_ignore_ascii_case(name))
            }
            _ => false,
        }
    }

    pub(crate) fn charset_method(
        &mut self,
        id: ObjId,
        index: u16,
        argc: Option<&mut u32>,
    ) -> VmResult<Option<Value>> {
        let Some(&(_, desc)) = METHODS.get(index as usize).filter(|_| index != 0) else {
            return Ok(None);
        };
        self.native_method(argc, desc, |vm, _| vm.charset_call(id, index))
            .map(Some)
    }

    fn charset_call(&mut self, id: ObjId, index: u16) -> VmResult<Value> {
        let Some(Object::CharSet { name, map }) = self.objects.get(id) else {
            return Ok(Value::Nil);
        };
        let known = map.is_some();
        Ok(match index {
            // getName()
            1 => {
                let name = name.clone();
                self.new_str(&name)
            }
            // isMappingKnown()
            2 => Value::logical(known),
            // isMappable(val) and isRoundTripMappable(val): of a
            // character code, or of every character of a string.
            _ => {
                let map = self.charset_map(id)?;
                let test = |c: char| {
                    if index == 3 {
                        map.is_mappable(c)
                    } else {
                        map.is_round_trip(c)
                    }
                };
                let arg = self.arg(0);
                if let Some(text) = self.as_str(arg) {
                    Value::logical(String::from_utf8_lossy(&text).chars().all(test))
                } else if let Value::Int(code_point) = arg {
                    let c = u16::try_from(code_point)
                        .ok()
                        .and_then(|code| char::from_u32(code.into()));
                    Value::logical(c.is_some_and(test))
                } else {
                    Value::Nil
                }
            }
        })
    }
}
