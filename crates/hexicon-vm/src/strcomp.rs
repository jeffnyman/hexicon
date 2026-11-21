//! The StringComparator intrinsic class: the rules a dictionary uses to
//! decide whether a word a player typed matches a word it knows.
//!
//! A comparator can ignore case, accept a word cut short after some
//! number of characters, and treat one character in a dictionary word as
//! equivalent to a sequence of characters in the input, such as `ß` to
//! `ss`. A match is reported as a set of flags that say how exact it was.

use std::collections::HashMap;
use std::rc::Rc;

use crate::casefold;
use crate::error::{VmError, VmResult, code};
use crate::object::Object;
use crate::value::{NativeDesc, ObjId, Value};
use crate::vm::Vm;

/// The strings match.
const MATCH: u32 = 0x0001;
/// They match only when case is ignored.
const CASE_FOLD: u32 = 0x0002;
/// The input is a truncated form of the dictionary word.
const TRUNCATED: u32 = 0x0004;

/// What a character of a dictionary word can stand for in the input.
#[derive(Debug, Clone)]
pub(crate) struct Equivalent {
    /// The characters in the input.
    chars: Vec<char>,
    /// Flags to add to the result when the input character is uppercase,
    /// and when it is not.
    upper_flags: u32,
    lower_flags: u32,
}

/// A string comparator. It never changes once made.
#[derive(Debug, Clone, Default)]
pub(crate) struct StrComp {
    /// How many characters of input are enough to match a longer word,
    /// or zero if a word has to be given in full.
    pub truncation: usize,
    pub case_sensitive: bool,
    equivalents: HashMap<char, Equivalent>,
}

impl StrComp {
    /// The bytes the reference allocates for a comparator: a fixed part
    /// with a table of the first characters, an entry for each
    /// equivalence, and its characters in 16 bits.
    pub(crate) fn heap_bytes(&self) -> i64 {
        let chars: usize = self.equivalents.values().map(|e| e.chars.len()).sum();
        (2064 + 24 * self.equivalents.len() + 2 * chars) as i64
    }
}

/// A string read a character at a time in its case-folded form, where
/// one character can become several.
struct Folded<'a> {
    text: &'a [char],
    pos: usize,
    /// What is left of the current character's folded form.
    pending: std::str::Chars<'static>,
}

impl<'a> Folded<'a> {
    fn new(text: &'a [char]) -> Self {
        Self {
            text,
            pos: 0,
            pending: "".chars(),
        }
    }

    /// The next folded character, or NUL past the end.
    fn next(&mut self) -> char {
        if let Some(c) = self.pending.next() {
            return c;
        }
        let Some(&c) = self.text.get(self.pos) else {
            return '\0';
        };
        self.pos += 1;
        match casefold::fold(c) {
            Some(folded) => {
                self.pending = folded.chars();
                self.pending.next().unwrap_or('\0')
            }
            None => c,
        }
    }

    /// Whether the last character read was the end of a source
    /// character's folded form.
    fn at_boundary(&self) -> bool {
        self.pending.as_str().is_empty()
    }
}

/// Compare the fewest characters at the start of two strings that take
/// both to a character boundary, ignoring case. Returns how many
/// characters of each matched, or `None` if they differ.
fn fold_min(a: &[char], b: &[char]) -> Option<(usize, usize)> {
    if a.is_empty() || b.is_empty() {
        return None;
    }
    let (mut fa, mut fb) = (Folded::new(a), Folded::new(b));
    loop {
        if fa.next() != fb.next() {
            return None;
        }
        if fa.at_boundary() && fb.at_boundary() {
            return Some((fa.pos, fb.pos));
        }
    }
}

impl StrComp {
    /// Read a comparator from its image or saved data: the truncation
    /// length, flags, the number of equivalents and their total length,
    /// then each equivalent.
    pub(crate) fn from_bytes(data: &[u8]) -> Option<(Self, usize)> {
        let u16_at = |at: usize| Some(u16::from_le_bytes([*data.get(at)?, *data.get(at + 1)?]));
        let u32_at = |at: usize| {
            let b = data.get(at..at + 4)?;
            Some(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        };
        let unit = |at: usize| Some(char::from_u32(u16_at(at)?.into()).unwrap_or('\u{fffd}'));
        let mut comparator = Self {
            truncation: u16_at(0)?.into(),
            case_sensitive: u16_at(2)? & 1 != 0,
            equivalents: HashMap::new(),
        };
        let count = u16_at(4)?;
        let mut pos = 8;
        for _ in 0..count {
            let reference = unit(pos)?;
            let len = usize::from(*data.get(pos + 2)?);
            let upper_flags = u32_at(pos + 3)?;
            let lower_flags = u32_at(pos + 7)?;
            pos += 11;
            let mut chars = Vec::with_capacity(len);
            for _ in 0..len {
                chars.push(unit(pos)?);
                pos += 2;
            }
            comparator.equivalents.insert(
                reference,
                Equivalent {
                    chars,
                    upper_flags,
                    lower_flags,
                },
            );
        }
        Some((comparator, pos))
    }

    /// The comparator as [`StrComp::from_bytes`] reads it, with the
    /// equivalents in the order of their characters.
    pub(crate) fn to_bytes(&self) -> Vec<u8> {
        let mut references: Vec<&char> = self.equivalents.keys().collect();
        references.sort();
        let total: usize = self.equivalents.values().map(|e| e.chars.len()).sum();
        let mut out = Vec::new();
        out.extend_from_slice(&(self.truncation as u16).to_le_bytes());
        out.extend_from_slice(&u16::from(self.case_sensitive).to_le_bytes());
        out.extend_from_slice(&(references.len() as u16).to_le_bytes());
        out.extend_from_slice(&(total as u16).to_le_bytes());
        for reference in references {
            let equivalent = &self.equivalents[reference];
            out.extend_from_slice(&(u32::from(*reference) as u16).to_le_bytes());
            out.push(equivalent.chars.len() as u8);
            out.extend_from_slice(&equivalent.upper_flags.to_le_bytes());
            out.extend_from_slice(&equivalent.lower_flags.to_le_bytes());
            for &c in &equivalent.chars {
                out.extend_from_slice(&(u32::from(c) as u16).to_le_bytes());
            }
        }
        out
    }

    /// The hash of a string: the sum of its characters, after
    /// equivalents are expanded and, unless case matters, case is
    /// folded. Only as many characters count as the truncation length
    /// allows, so that a truncated word hashes like the whole one.
    pub(crate) fn hash(&self, text: &[u8]) -> u32 {
        let mut hash: u32 = 0;
        let mut left = self.truncation;
        // Returns false once enough characters have been counted.
        let mut add = |c: char| {
            hash = (hash + u32::from(c)) & 0xffff;
            if self.truncation != 0 {
                left -= 1;
                return left != 0;
            }
            true
        };
        let mut add_folded = |c: char| match casefold::fold(c).filter(|_| !self.case_sensitive) {
            Some(folded) => folded.chars().all(&mut add),
            None => add(c),
        };
        for c in String::from_utf8_lossy(text).chars() {
            let more = match self.equivalents.get(&c) {
                Some(equivalent) => equivalent.chars.iter().all(|&c| add_folded(c)),
                None => add_folded(c),
            };
            if !more {
                break;
            }
        }
        hash
    }

    /// Whether what the player typed matches a dictionary word: zero if
    /// not, and otherwise flags saying how.
    pub(crate) fn match_strings(&self, value: &[u8], reference: &[u8]) -> u32 {
        let value: Vec<char> = String::from_utf8_lossy(value).chars().collect();
        let reference: Vec<char> = String::from_utf8_lossy(reference).chars().collect();
        let fold_case = !self.case_sensitive;
        let (mut vi, mut ri) = (0, 0);
        let mut result = 0;
        while vi < value.len() && ri < reference.len() {
            let value_char = value[vi];
            let reference_char = reference[ri];
            if reference_char == value_char {
                vi += 1;
                ri += 1;
                continue;
            }
            if fold_case {
                if let Some((a, b)) = fold_min(&value[vi..], &reference[ri..]) {
                    vi += a;
                    ri += b;
                    result |= CASE_FOLD;
                    continue;
                }
            }
            // The dictionary word's character might stand for several in
            // the input.
            let Some(equivalent) = self.equivalents.get(&reference_char) else {
                return 0;
            };
            result |= if value_char.is_uppercase() {
                equivalent.upper_flags
            } else {
                equivalent.lower_flags
            };
            let chars = &equivalent.chars;
            let mut ei = 0;
            while vi < value.len() && ei < chars.len() {
                // The reference VM goes on comparing with the input
                // character it started from, so only the first of the
                // equivalent's characters can match this way. The rest
                // match by the case-folding comparison, which reports
                // them as differing in case. Matching results depend on
                // it, so it is the same here.
                if chars[ei] == value_char {
                    vi += 1;
                    ei += 1;
                    continue;
                }
                if fold_case {
                    if let Some((a, b)) = fold_min(&value[vi..], &chars[ei..]) {
                        vi += a;
                        ei += b;
                        result |= CASE_FOLD;
                        continue;
                    }
                }
                return 0;
            }
            if ei < chars.len() {
                return 0;
            }
            ri += 1;
        }
        if ri == reference.len() && vi == value.len() {
            result | MATCH
        } else if vi < value.len() {
            0
        } else if self.truncation != 0 && value.len() >= self.truncation {
            // The input ran out first, but was long enough.
            result | MATCH | TRUNCATED
        } else {
            0
        }
    }

    /// How many characters at the start of some input match one
    /// character of a dictionary word, or zero if they do not.
    pub(crate) fn match_chars(&self, value: &[char], reference: char) -> usize {
        let Some(&value_char) = value.first() else {
            return 0;
        };
        if reference == value_char {
            return 1;
        }
        let fold_case = !self.case_sensitive;
        if fold_case {
            if let Some((a, _)) = fold_min(value, &[reference]) {
                return a;
            }
        }
        let Some(equivalent) = self.equivalents.get(&reference) else {
            return 0;
        };
        let chars = &equivalent.chars;
        let (mut vi, mut ei) = (0, 0);
        while vi < value.len() && ei < chars.len() {
            if chars[ei] == value_char {
                vi += 1;
                ei += 1;
                continue;
            }
            if fold_case {
                if let Some((a, b)) = fold_min(&value[vi..], &chars[ei..]) {
                    vi += a;
                    ei += b;
                    continue;
                }
            }
            return 0;
        }
        vi
    }
}

/// The name and calling interface of each method, by method number.
const METHODS: &[(&str, NativeDesc)] = &[
    ("", NativeDesc::fixed(0)),
    ("calcHash", NativeDesc::fixed(1)),
    ("matchValues", NativeDesc::fixed(2)),
];

impl Vm {
    /// The comparator an object is, if it is one.
    pub(crate) fn as_strcomp(&self, id: ObjId) -> Option<Rc<StrComp>> {
        match self.objects.get(id)? {
            Object::StrComp(comparator) => Some(comparator.clone()),
            _ => None,
        }
    }

    /// `new StringComparator(truncationLength, caseSensitive, mappings)`.
    /// Each mapping is a list: the dictionary character, the input
    /// characters it stands for, and the flags for an uppercase and a
    /// lowercase match.
    pub(crate) fn create_strcomp(&mut self, argc: u32) -> VmResult<ObjId> {
        if argc != 3 {
            return Err(VmError::new(code::WRONG_NUM_OF_ARGS));
        }
        let truncation = match self.arg(0) {
            Value::Nil => 0,
            other => self.int_arg(other)?.max(0) as usize,
        };
        let case_sensitive = match self.arg(1) {
            Value::True => true,
            Value::Nil => false,
            _ => return Err(VmError::new(code::BAD_TYPE_BIF)),
        };
        let mut comparator = StrComp {
            truncation,
            case_sensitive,
            equivalents: HashMap::new(),
        };
        let mappings = self.arg(2);
        if mappings != Value::Nil {
            let Some(count) = self.list_like_len(mappings)? else {
                return Err(VmError::new(code::BAD_TYPE_BIF));
            };
            for index in 1..=count {
                let mapping = self.list_like_get(mappings, index)?;
                if self.list_like_len(mapping)?.is_none() {
                    return Err(VmError::new(code::BAD_TYPE_BIF));
                }
                let reference = self.list_like_get(mapping, 1)?;
                let value = self.list_like_get(mapping, 2)?;
                let (Some(reference), Some(value)) = (self.as_str(reference), self.as_str(value))
                else {
                    return Err(VmError::new(code::BAD_TYPE_BIF));
                };
                let reference = String::from_utf8_lossy(&reference).chars().next();
                let chars: Vec<char> = String::from_utf8_lossy(&value).chars().collect();
                let (Some(reference), false) = (reference, chars.is_empty()) else {
                    return Err(VmError::new(code::BAD_VAL_BIF));
                };
                let mut flags = [0u32; 2];
                for (slot, at) in flags.iter_mut().zip([3, 4]) {
                    match self.list_like_get(mapping, at)? {
                        Value::Int(n) => *slot = n as u32,
                        _ => return Err(VmError::new(code::BAD_TYPE_BIF)),
                    }
                }
                comparator.equivalents.insert(
                    reference,
                    Equivalent {
                        chars,
                        upper_flags: flags[0],
                        lower_flags: flags[1],
                    },
                );
            }
        }
        Ok(self.alloc_obj(Object::StrComp(Rc::new(comparator))))
    }

    pub(crate) fn strcomp_method(
        &mut self,
        id: ObjId,
        index: u16,
        argc: Option<&mut u32>,
    ) -> VmResult<Option<Value>> {
        let Some(&(_, desc)) = METHODS.get(index as usize).filter(|_| index != 0) else {
            return Ok(None);
        };
        self.native_method(argc, desc, |vm, _| {
            let Some(comparator) = vm.as_strcomp(id) else {
                return Ok(Value::Nil);
            };
            let text = |vm: &Vm, value: Value| {
                vm.as_str(value)
                    .map(|text| text.to_vec())
                    .ok_or_else(|| VmError::new(code::BAD_TYPE_BIF))
            };
            Ok(if index == 1 {
                // calcHash(str)
                Value::Int(comparator.hash(&text(vm, vm.arg(0))?) as i32)
            } else {
                // matchValues(value, reference)
                let value = text(vm, vm.arg(0))?;
                let reference = text(vm, vm.arg(1))?;
                Value::Int(comparator.match_strings(&value, &reference) as i32)
            })
        })
        .map(Some)
    }
}
