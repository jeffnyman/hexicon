//! The string intrinsic class, the regular-expression pattern class, and
//! the searching and replacing they share with the `rex` functions.
//!
//! Strings are UTF-8. Positions that programs see count characters from
//! 1; a negative position counts back from the end.

use std::rc::Rc;

use crate::error::{VmError, VmResult, code};
use crate::object::Object;
use crate::regex::{self, GROUP_COUNT, Groups, Pattern};
use crate::value::{NativeDesc, ObjId, Value};
use crate::vm::{NativeCall, StrRef, Vm};

/// The name and calling interface of each string method, by method number.
const METHODS: &[(&str, NativeDesc)] = &[
    ("", NativeDesc::fixed(0)),
    ("length", NativeDesc::fixed(0)),
    ("substr", NativeDesc::optional(1, 1)),
    ("toUpper", NativeDesc::fixed(0)),
    ("toLower", NativeDesc::fixed(0)),
    ("find", NativeDesc::optional(1, 1)),
    ("toUnicode", NativeDesc::optional(0, 1)),
    ("htmlify", NativeDesc::optional(0, 1)),
    ("startsWith", NativeDesc::fixed(1)),
    ("endsWith", NativeDesc::fixed(1)),
    ("mapToByteArray", NativeDesc::optional(0, 1)),
    ("findReplace", NativeDesc::optional(2, 3)),
    ("splice", NativeDesc::optional(2, 1)),
    ("split", NativeDesc::optional(0, 2)),
    ("specialsToHtml", NativeDesc::optional(0, 1)),
    ("specialsToText", NativeDesc::optional(0, 1)),
    ("urlEncode", NativeDesc::fixed(0)),
    ("urlDecode", NativeDesc::fixed(0)),
    ("sha256", NativeDesc::fixed(0)),
    ("digestMD5", NativeDesc::fixed(0)),
    ("packBytes", NativeDesc::variable(1)),
    ("unpackBytes", NativeDesc::fixed(1)),
    ("toTitleCase", NativeDesc::fixed(0)),
    ("toFoldedCase", NativeDesc::fixed(0)),
    ("compareTo", NativeDesc::fixed(1)),
    ("compareIgnoreCase", NativeDesc::fixed(1)),
    ("findLast", NativeDesc::optional(1, 1)),
    ("findAll", NativeDesc::optional(1, 1)),
    ("match", NativeDesc::optional(1, 1)),
];

// `htmlify` flags.
const HTMLIFY_KEEP_SPACES: i32 = 0x0001;
const HTMLIFY_KEEP_NEWLINES: i32 = 0x0002;
const HTMLIFY_KEEP_TABS: i32 = 0x0004;

// Replacement flags, for `findReplace` and `rexReplace`.
const REPLACE_ALL: i32 = 0x0001;
const REPLACE_NOCASE: i32 = 0x0002;
const REPLACE_FOLLOW_CASE: i32 = 0x0004;
const REPLACE_SERIAL: i32 = 0x0008;
const REPLACE_ONCE: i32 = 0x0010;

/// The result of the most recent regular expression match, which
/// `rexGroup` reports on.
#[derive(Debug, Clone)]
pub(crate) struct LastMatch {
    /// The string that was searched.
    pub subject: Value,
    /// The whole match, as byte offsets.
    pub whole: Option<(usize, usize)>,
    pub groups: Groups,
    pub group_count: usize,
}

impl Default for LastMatch {
    fn default() -> Self {
        Self {
            subject: Value::Nil,
            whole: None,
            groups: [None; GROUP_COUNT],
            group_count: 0,
        }
    }
}

// ---- UTF-8 helpers ----

/// Byte offsets of the character boundaries of UTF-8 text, including the
/// end. Continuation bytes are the ones of the form `10xxxxxx`.
pub(crate) fn char_starts(text: &[u8]) -> Vec<usize> {
    let mut starts: Vec<usize> = text
        .iter()
        .enumerate()
        .filter(|(_, b)| **b & 0xc0 != 0x80)
        .map(|(i, _)| i)
        .collect();
    starts.push(text.len());
    starts
}

/// Number of characters in UTF-8 text.
pub(crate) fn char_count(text: &[u8]) -> usize {
    text.iter().filter(|b| **b & 0xc0 != 0x80).count()
}

/// The characters of UTF-8 text.
fn chars(text: &[u8]) -> Vec<char> {
    String::from_utf8_lossy(text).chars().collect()
}

/// The byte offset of the start of the character after the one at `pos`.
fn next_char(text: &[u8], pos: usize) -> usize {
    let mut at = pos + 1;
    while at < text.len() && text[at] & 0xc0 == 0x80 {
        at += 1;
    }
    at.min(text.len())
}

/// Whether `text` starts with `prefix`, ignoring case. Returns the number
/// of bytes of `text` matched.
fn fold_prefix(prefix: &[u8], text: &[u8]) -> Option<usize> {
    let mut consumed = 0;
    let text_str = String::from_utf8_lossy(text);
    let mut text_chars = text_str.chars();
    for wanted in String::from_utf8_lossy(prefix).chars() {
        let have = text_chars.next()?;
        if regex::fold(have) != regex::fold(wanted) {
            return None;
        }
        consumed += have.len_utf8();
    }
    Some(consumed.min(text.len()))
}

/// Convert a one-based character position that may count from the end
/// (if negative) to a zero-based character index.
fn index_from(position: i32, count: usize) -> i32 {
    position + if position < 0 { count as i32 } else { -1 }
}

/// Something to search for: literal text or a compiled pattern.
#[derive(Clone)]
enum Target {
    Text(Vec<u8>),
    Pattern(Rc<Pattern>),
}

impl Vm {
    // ---- patterns ----

    /// Create a pattern object from the string a program wrote.
    pub(crate) fn new_pattern(&mut self, source: Value) -> VmResult<ObjId> {
        let text = self.require_str(source)?;
        let compiled = Rc::new(Pattern::compile(&text));
        Ok(self.alloc_obj(Object::Pattern {
            source,
            compiled: Some(compiled),
        }))
    }

    /// The compiled form of a pattern object. A pattern loaded from an
    /// image is compiled the first time it is used.
    pub(crate) fn pattern_of(&mut self, id: ObjId) -> Option<Rc<Pattern>> {
        let source = match self.objects.get(id)? {
            Object::Pattern {
                compiled: Some(pattern),
                ..
            } => return Some(pattern.clone()),
            Object::Pattern { source, .. } => *source,
            _ => return None,
        };
        let text = self.as_str(source)?;
        let pattern = Rc::new(Pattern::compile(&text));
        if let Some(Object::Pattern { compiled, .. }) = self.objects.get_mut(id) {
            *compiled = Some(pattern.clone());
        }
        Some(pattern)
    }

    /// The pattern a value holds, if it is a pattern object.
    fn as_pattern(&mut self, value: Value) -> Option<Rc<Pattern>> {
        match value {
            Value::Obj(id) => self.pattern_of(id),
            _ => None,
        }
    }

    /// Call a method of the pattern class.
    pub(crate) fn pattern_method(
        &mut self,
        id: ObjId,
        index: u16,
        argc: Option<&mut u32>,
    ) -> VmResult<Option<Value>> {
        match index {
            // getPatternString()
            1 => self
                .native_method(argc, NativeDesc::fixed(0), |vm, _| {
                    Ok(match vm.objects.get(id) {
                        Some(Object::Pattern { source, .. }) => *source,
                        _ => Value::Nil,
                    })
                })
                .map(Some),
            _ => Ok(None),
        }
    }

    /// A search target from an argument that may be a string or a
    /// pattern object. A string is literal text unless `compile` is set.
    fn target_arg(&mut self, value: Value, compile: bool) -> VmResult<Target> {
        if let Some(text) = self.as_str(value) {
            return Ok(if compile {
                Target::Pattern(Rc::new(Pattern::compile(&text)))
            } else {
                Target::Text(text.to_vec())
            });
        }
        match self.as_pattern(value) {
            Some(pattern) => Ok(Target::Pattern(pattern)),
            None => Err(VmError::new(code::BAD_TYPE_BIF)),
        }
    }

    /// Record a pattern match for `rexGroup`.
    fn record_match(
        &mut self,
        subject: Value,
        pattern: &Pattern,
        whole: Option<(usize, usize)>,
        groups: Groups,
    ) {
        self.last_match = LastMatch {
            subject,
            whole,
            groups: if whole.is_some() {
                groups
            } else {
                [None; GROUP_COUNT]
            },
            group_count: pattern.group_count,
        };
    }

    /// Search `text` forward from byte offset `from`. Returns the offset
    /// and length of the match.
    fn find_forward(
        &mut self,
        subject: Value,
        text: &[u8],
        from: usize,
        target: &Target,
        case_sensitive: bool,
    ) -> Option<(usize, usize)> {
        match target {
            Target::Text(wanted) => {
                let mut pos = from;
                while text.len() - pos >= wanted.len() {
                    if case_sensitive {
                        if text[pos..].starts_with(wanted) {
                            return Some((pos, wanted.len()));
                        }
                    } else if let Some(len) = fold_prefix(wanted, &text[pos..]) {
                        return Some((pos, len));
                    }
                    if pos >= text.len() {
                        break;
                    }
                    pos = next_char(text, pos);
                }
                None
            }
            Target::Pattern(pattern) => {
                let mut groups = [None; GROUP_COUNT];
                let found = pattern.search(text, from, case_sensitive, &mut groups);
                let whole = found.map(|(start, len)| (start, start + len));
                self.record_match(subject, pattern, whole, groups);
                found
            }
        }
    }

    /// Search `text` backward for a match ending at or before `from`.
    fn find_backward(
        &mut self,
        subject: Value,
        text: &[u8],
        from: usize,
        target: &Target,
    ) -> Option<(usize, usize)> {
        match target {
            Target::Text(wanted) => {
                let mut pos = from.checked_sub(wanted.len())?;
                loop {
                    if text[pos..].starts_with(wanted) && pos + wanted.len() <= from {
                        return Some((pos, wanted.len()));
                    }
                    if pos == 0 {
                        return None;
                    }
                    pos -= 1;
                }
            }
            Target::Pattern(pattern) => {
                let mut groups = [None; GROUP_COUNT];
                let found = pattern.search_back(text, from, true, &mut groups);
                let whole = found.map(|(start, len)| (start, start + len));
                self.record_match(subject, pattern, whole, groups);
                found
            }
        }
    }

    // ---- the string class ----

    /// Call a method of the string class on a string constant or object.
    pub(crate) fn string_method(
        &mut self,
        self_value: Value,
        index: u16,
        argc: Option<&mut u32>,
    ) -> VmResult<Option<Value>> {
        let Some(&(_, desc)) = METHODS.get(index as usize).filter(|_| index != 0) else {
            return Ok(None);
        };
        self.native_method(argc, desc, |vm, argc| {
            vm.string_call(self_value, index, argc)
        })
        .map(Some)
    }

    /// `String.packBytes(format, ...)`, called on the class: the packed
    /// bytes as a string, a character for each byte.
    pub(crate) fn string_static_method(
        &mut self,
        index: u16,
        argc: Option<&mut u32>,
    ) -> VmResult<Option<Value>> {
        if index != 20 {
            return Ok(None);
        }
        self.native_method(argc, NativeDesc::variable(1), |vm, argc| {
            let packed = vm.pack(0, argc as usize, crate::pack::Stream::growable())?;
            Ok(vm.new_str(&crate::bytearray::latin1_text(&packed.data)))
        })
        .map(Some)
    }

    fn string_call(&mut self, self_value: Value, index: u16, argc: u32) -> VmResult<Value> {
        let text: StrRef = self.require_str(self_value)?;
        Ok(match index {
            // length(): the number of characters.
            1 => Value::Int(char_count(&text) as i32),
            // substr(start, len?)
            2 => {
                let start = self.int_arg(self.arg(0))?;
                let starts = char_starts(&text);
                let count = starts.len() - 1;
                let from = if start > 0 {
                    (start as usize - 1).min(count)
                } else if start < 0 {
                    count.saturating_sub(start.unsigned_abs() as usize)
                } else {
                    0
                };
                let to = if argc >= 2 {
                    let len = self.int_arg(self.arg(1))?;
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
                self.new_string(&text[starts[from]..starts[to]])
            }
            // toUpper(), toLower(), toTitleCase(), toFoldedCase()
            3 | 4 | 22 | 23 => {
                let source = String::from_utf8_lossy(&text);
                let mut out = String::with_capacity(source.len());
                for c in source.chars() {
                    match index {
                        3 => out.extend(c.to_uppercase()),
                        22 => {
                            // Title case: a capital followed, where one
                            // letter becomes several, by small letters.
                            let mut upper = c.to_uppercase();
                            out.extend(upper.next());
                            for rest in upper {
                                out.extend(rest.to_lowercase());
                            }
                        }
                        23 => match crate::casefold::fold(c) {
                            Some(folded) => out.push_str(folded),
                            None => out.push(c),
                        },
                        _ => out.extend(c.to_lowercase()),
                    }
                }
                self.new_str(&out)
            }
            // find(target, index?) and findLast(target, index?)
            5 | 26 => self.string_find(self_value, &text, argc, index == 5)?,
            // toUnicode(index?)
            6 => {
                let all = chars(&text);
                if argc >= 1 {
                    let mut position = self.int_arg(self.arg(0))?;
                    if position < 0 {
                        position += all.len() as i32 + 1;
                    }
                    match usize::try_from(position - 1).ok().and_then(|i| all.get(i)) {
                        Some(&c) if position >= 1 => Value::Int(c as i32),
                        _ => Value::Nil,
                    }
                } else {
                    let codes: Vec<Value> = all.iter().map(|&c| Value::Int(c as i32)).collect();
                    self.new_list(codes)
                }
            }
            // htmlify(flags?)
            7 => {
                let flags = if argc >= 1 {
                    self.int_arg(self.arg(0))?
                } else {
                    0
                };
                let mut out = String::new();
                let mut previous_was_space = false;
                for c in String::from_utf8_lossy(&text).chars() {
                    let mut is_space = false;
                    match c {
                        '&' => out.push_str("&amp;"),
                        '<' => out.push_str("&lt;"),
                        '>' => out.push_str("&gt;"),
                        ' ' => {
                            is_space = true;
                            // Only a space after a space needs help to
                            // survive.
                            if previous_was_space && flags & HTMLIFY_KEEP_SPACES != 0 {
                                out.push_str("&nbsp;");
                            } else {
                                out.push(c);
                            }
                        }
                        '\t' => {
                            is_space = true;
                            if flags & HTMLIFY_KEEP_TABS != 0 {
                                out.push_str("<tab>");
                            } else {
                                out.push(c);
                            }
                        }
                        '\n' | '\u{2028}' => {
                            is_space = true;
                            if flags & HTMLIFY_KEEP_NEWLINES != 0 {
                                out.push_str("<br>");
                            } else {
                                out.push(c);
                            }
                        }
                        _ => out.push(c),
                    }
                    previous_was_space = is_space;
                }
                self.new_str(&out)
            }
            // startsWith(str) and endsWith(str)
            8 | 9 => {
                let other = self.require_str(self.arg(0))?;
                Value::logical(if index == 8 {
                    text.starts_with(&other)
                } else {
                    text.ends_with(&other)
                })
            }
            // findReplace(target, replacement, flags?, index?, limit?)
            11 => self.find_replace(self_value, argc, false)?,
            // splice(index, deleteCount, insert?)
            12 => {
                let starts = char_starts(&text);
                let count = (starts.len() - 1) as i32;
                let start = index_from(self.int_arg(self.arg(0))?, count as usize).clamp(0, count);
                let delete = self.int_arg(self.arg(1))?.clamp(0, count - start);
                let insert = if argc >= 3 && !self.arg(2).is_nil() {
                    self.text_of(self.arg(2))?.1.to_vec()
                } else {
                    Vec::new()
                };
                if delete == 0 && insert.is_empty() {
                    return Ok(self_value);
                }
                let mut out = text[..starts[start as usize]].to_vec();
                out.extend_from_slice(&insert);
                out.extend_from_slice(&text[starts[(start + delete) as usize]..]);
                self.new_string(out)
            }
            // split(delimiter?, limit?)
            13 => self.string_split(self_value, &text, argc)?,
            // urlEncode()
            16 => {
                let mut out = String::new();
                for &byte in text.iter() {
                    match byte {
                        b' ' => out.push('+'),
                        b'-' | b'_' | b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' => {
                            out.push(byte as char);
                        }
                        _ => out.push_str(&format!("%{byte:02X}")),
                    }
                }
                self.new_str(&out)
            }
            // urlDecode()
            17 => {
                let mut out = Vec::with_capacity(text.len());
                let mut at = 0;
                while at < text.len() {
                    let hex = |i: usize| text.get(i).and_then(|b| (*b as char).to_digit(16));
                    match (text[at], hex(at + 1), hex(at + 2)) {
                        (b'+', _, _) => out.push(b' '),
                        (b'%', Some(high), Some(low)) => {
                            out.push((high << 4 | low) as u8);
                            at += 2;
                        }
                        (byte, _, _) => out.push(byte),
                    }
                    at += 1;
                }
                // The result has to be valid text.
                let valid = String::from_utf8_lossy(&out).replace('\u{fffd}', "?");
                self.new_str(&valid)
            }
            // compareTo(str): by character code, then by length.
            24 => {
                let other = self.require_str(self.arg(0))?;
                let (a, b) = (chars(&text), chars(&other));
                let differ = a
                    .iter()
                    .zip(&b)
                    .find(|(x, y)| x != y)
                    .map(|(&x, &y)| x as i32 - y as i32);
                Value::Int(match differ {
                    Some(delta) => delta,
                    None => {
                        // The reference VM compares what is left in bytes.
                        let common = a.len().min(b.len());
                        let rest = |all: &[char]| -> i32 {
                            all[common..].iter().map(|c| c.len_utf8() as i32).sum()
                        };
                        rest(&a) - rest(&b)
                    }
                })
            }
            // compareIgnoreCase(str)
            25 => {
                let other = self.require_str(self.arg(0))?;
                let fold = |text: &[u8]| -> Vec<char> {
                    String::from_utf8_lossy(text)
                        .chars()
                        .flat_map(char::to_lowercase)
                        .collect()
                };
                let (a, b) = (fold(&text), fold(&other));
                let differ = a
                    .iter()
                    .zip(&b)
                    .find(|(x, y)| x != y)
                    .map(|(&x, &y)| x as i32 - y as i32);
                Value::Int(differ.unwrap_or(a.len() as i32 - b.len() as i32))
            }
            // findAll(target, func?)
            27 => self.string_find_all(self_value, &text, argc)?,
            // match(target, index?): the length of a match at a position.
            28 => {
                let target = self.target_arg(self.arg(0), false)?;
                let mut from = 0;
                if argc >= 2 && !self.arg(1).is_nil() {
                    let starts = char_starts(&text);
                    let position = index_from(self.int_arg(self.arg(1))?, starts.len() - 1);
                    match usize::try_from(position).ok().and_then(|i| starts.get(i)) {
                        Some(&offset) => from = offset,
                        None => return Ok(Value::Nil),
                    }
                }
                // The length is in bytes, as in the reference VM.
                match target {
                    Target::Text(wanted) => {
                        if text[from..].starts_with(&wanted) {
                            Value::Int(wanted.len() as i32)
                        } else {
                            Value::Nil
                        }
                    }
                    Target::Pattern(pattern) => {
                        let mut groups = [None; GROUP_COUNT];
                        let found = pattern.match_at(&text, from, text.len(), true, &mut groups);
                        let whole = found.map(|len| (from, from + len));
                        self.record_match(self_value, &pattern, whole, groups);
                        match found {
                            Some(len) => Value::Int(len as i32),
                            None => Value::Nil,
                        }
                    }
                }
            }
            // specialsToHtml(state?) and specialsToText(state?)
            14 | 15 => self.specials_to(&text, argc, index == 14)?,
            // mapToByteArray(charset?)
            10 => {
                let charset = if argc >= 1 { self.arg(0) } else { Value::Nil };
                let bytes = self.text_to_bytes(&text, charset)?;
                self.new_bytes(bytes)
            }
            // sha256() and digestMD5(): of the text as UTF-8.
            18 => self.new_str(&crate::hash::hex(&crate::hash::sha256(&text))),
            19 => self.new_str(&crate::hash::hex(&crate::hash::md5(&text))),
            // unpackBytes(format): the characters are taken as bytes.
            21 => {
                let stream = crate::pack::Stream::from_text(&text);
                self.unpack(self.arg(0), stream)?
            }
            _ => {
                return Err(VmError::unimplemented(format!(
                    "String.{}",
                    METHODS[index as usize].0
                )));
            }
        })
    }

    /// `find` and `findLast`: the character position of a match, or nil.
    fn string_find(
        &mut self,
        subject: Value,
        text: &[u8],
        argc: u32,
        forward: bool,
    ) -> VmResult<Value> {
        let target = self.target_arg(self.arg(0), false)?;
        let starts = char_starts(text);
        let count = starts.len() - 1;
        let start = if argc >= 2 && !self.arg(1).is_nil() {
            let position = self.int_arg(self.arg(1))?;
            let position = if position == 0 && !forward {
                count as i32
            } else {
                index_from(position, count)
            };
            match usize::try_from(position).ok().filter(|&p| p <= count) {
                Some(position) => position,
                None => return Ok(Value::Nil),
            }
        } else if forward {
            0
        } else {
            count
        };
        let found = if forward {
            self.find_forward(subject, text, starts[start], &target, true)
        } else {
            self.find_backward(subject, text, starts[start], &target)
        };
        Ok(match found {
            Some((offset, _)) => Value::Int(char_count(&text[..offset]) as i32 + 1),
            None => Value::Nil,
        })
    }

    /// `findAll`: every match, or the result of a callback for each.
    fn string_find_all(&mut self, subject: Value, text: &[u8], argc: u32) -> VmResult<Value> {
        let target = self.target_arg(self.arg(0), false)?;
        let callback = if argc >= 2 && !self.arg(1).is_nil() {
            let function = self.arg(1);
            if !self.is_function_value(function) {
                return Err(VmError::new(code::BAD_TYPE_BIF));
            }
            let (min, opt, varargs) = self.value_interface(function).unwrap_or((0, 0, true));
            Some((function, min, if varargs { -1 } else { min + opt }))
        } else {
            None
        };
        let native = self.native_call("String.findAll", subject, 27, argc);
        let mut results = Vec::new();
        let mut pos = 0;
        while pos < text.len() {
            let Some((start, len)) = self.find_forward(subject, text, pos, &target, true) else {
                break;
            };
            let matched = self.new_string(&text[start..start + len]);
            let element = match callback {
                None => matched,
                Some((function, min, max)) => {
                    // The callback gets the match, its position, and as
                    // many groups as it has parameters for.
                    let group_count = self.last_match.group_count as i32;
                    let wanted = if max < 0 { group_count } else { max - 2 }.max(min - 2);
                    let mut args = Vec::new();
                    if !(0..1).contains(&max) {
                        args.push(matched);
                    }
                    if !(0..2).contains(&max) {
                        args.push(Value::Int(char_count(&text[..start]) as i32 + 1));
                    }
                    for group in 0..wanted.max(0) as usize {
                        let range = (group < self.last_match.group_count)
                            .then(|| self.last_match.groups.get(group).copied().flatten())
                            .flatten();
                        args.push(match range {
                            Some((a, b)) => self.new_string(&text[a..b]),
                            None => Value::Nil,
                        });
                    }
                    self.call_value(function, &args, native.clone())?
                }
            };
            results.push(element);
            pos = start + len;
            if len == 0 && pos < text.len() {
                pos = next_char(text, pos);
            }
        }
        Ok(self.new_list(results))
    }

    /// `split`: cut a string at a delimiter, or into pieces of a length.
    fn string_split(&mut self, subject: Value, text: &[u8], argc: u32) -> VmResult<Value> {
        let delimiter = if argc >= 1 { self.arg(0) } else { Value::Nil };
        let (piece_len, target) = match delimiter {
            Value::Nil => (1, None),
            Value::Int(n) if n <= 0 => return Err(VmError::new(code::BAD_VAL_BIF)),
            Value::Int(n) => (n as usize, None),
            other => (0, Some(self.target_arg(other, false)?)),
        };
        let limit = if argc >= 2 && !self.arg(1).is_nil() {
            let limit = self.int_arg(self.arg(1))?;
            if limit < 1 {
                return Err(VmError::new(code::BAD_VAL_BIF));
            }
            Some(limit as usize)
        } else {
            None
        };
        // The list is made first, and the pieces after it, as in the
        // reference, so that each is given the same object number. The
        // reference makes it as long as the limit, or as the number of
        // pieces of a given length, counting the text's bytes, or ten,
        // and lengthens it one element at a time after that.
        let list = self.alloc_obj(Object::List(Rc::from([])));
        let mut room = match (limit, &target) {
            (Some(limit), _) => limit,
            (None, None) => text.len().div_ceil(piece_len),
            (None, Some(_)) => 10,
        };
        crate::heap::adjust(5 * room as i64, "list");
        let mut cell = crate::heap::cell_for(2 + 5 * room as i64);
        let mut pieces = Vec::new();
        let mut make_room = |count: usize| {
            if count > room {
                room = count;
                crate::heap::realloc(&mut cell, 2 + 5 * room as i64, "list");
            }
        };
        let mut pos = 0;
        while pos < text.len() && limit.is_none_or(|limit| pieces.len() + 1 < limit) {
            let (cut, skip) = match &target {
                None => {
                    let mut cut = pos;
                    for _ in 0..piece_len {
                        cut = next_char(text, cut);
                    }
                    if cut >= text.len() {
                        break;
                    }
                    (cut, 0)
                }
                Some(target) => match self.find_forward(subject, text, pos, target, true) {
                    Some((start, len)) => (start, len),
                    None => break,
                },
            };
            make_room(pieces.len() + 1);
            pieces.push(self.new_string(&text[pos..cut]));
            pos = cut + skip;
            // An empty delimiter match consumes a character, so the
            // split makes progress.
            if target.is_some() && skip == 0 && pos < text.len() {
                pos = next_char(text, pos);
            }
        }
        if pos < text.len() {
            make_room(pieces.len() + 1);
            pieces.push(self.new_string(&text[pos..]));
        }
        self.objects.replace(list, Object::List(pieces.into()));
        Ok(Value::Obj(list))
    }

    // ---- replacing ----

    /// Start a string that is built up a piece at a time, as the
    /// reference does when it replaces text: with room for three times
    /// the subject's bytes, to grow by what each piece needs and 512 more.
    fn start_built_string(&mut self, subject_len: usize) -> BuiltString {
        let id = self.alloc_obj(Object::String(Rc::from([])));
        let room = subject_len * 3;
        crate::heap::adjust(room as i64, "string");
        BuiltString {
            id,
            room,
            cell: crate::heap::cell_for(room as i64 + 2),
        }
    }

    /// Finish a built string with its text. The reference gives back
    /// room it did not use if that is 256 bytes or more.
    fn finish_built_string(&mut self, mut built: BuiltString, text: Vec<u8>) -> Value {
        built.make_room(text.len(), 0);
        if built.room - text.len() >= 256 {
            crate::heap::realloc(&mut built.cell, text.len() as i64 + 2, "string");
        }
        self.objects.replace(built.id, Object::String(text.into()));
        Value::Obj(built.id)
    }

    /// `String.findReplace` and `rexReplace`. The arguments are on the
    /// stack; for `rexReplace` the subject is the second of them and
    /// string targets are patterns.
    pub(crate) fn find_replace(
        &mut self,
        subject: Value,
        argc: u32,
        is_rex: bool,
    ) -> VmResult<Value> {
        struct Rule {
            target: Option<Target>,
            replacement: Replacement,
            /// The cached next match: its offset and length.
            found: Option<Option<(usize, usize)>>,
            groups: Groups,
            group_count: usize,
        }
        enum Replacement {
            Text(Vec<u8>),
            Function(Value, i32),
        }

        let argc = argc as usize;
        let new_ofs = if is_rex { 2 } else { 1 };
        let arg = |vm: &Vm, index: usize| (index < argc).then(|| vm.arg(index));
        let mut flags = REPLACE_ALL;
        if let Some(value) = arg(self, new_ofs + 1).filter(|v| !v.is_nil()) {
            flags = self.int_arg(value)?;
        }
        if flags == 0 {
            flags = REPLACE_ONCE;
        } else if flags & REPLACE_ONCE == 0 {
            flags |= REPLACE_ALL;
        }
        let mut limit: i32 = if flags & REPLACE_ONCE != 0 { 1 } else { -1 };
        if let Some(value) = arg(self, new_ofs + 3) {
            match value {
                Value::Nil => limit = -1,
                Value::Int(n) if n < 0 => return Err(VmError::new(code::BAD_VAL_BIF)),
                Value::Int(0) => return Ok(subject),
                Value::Int(n) => limit = n,
                _ => return Err(VmError::new(code::INT_VAL_REQD)),
            }
        }
        let case_sensitive = flags & REPLACE_NOCASE == 0;

        // The targets and their replacements; each may be a list.
        let targets = self.arg(0);
        let target_list: Vec<Value> = match self.list_like_len(targets)? {
            Some(0) => return Ok(subject),
            Some(len) => {
                let mut items = Vec::with_capacity(len);
                for index in 1..=len {
                    items.push(self.list_like_get(targets, index)?);
                }
                items
            }
            None => vec![targets],
        };
        let replacements = self.arg(new_ofs);
        let replacement_list: Option<Vec<Value>> = match self.list_like_len(replacements)? {
            Some(len) => {
                let mut items = Vec::with_capacity(len);
                for index in 1..=len {
                    items.push(self.list_like_get(replacements, index)?);
                }
                Some(items)
            }
            None => None,
        };
        let mut rules = Vec::with_capacity(target_list.len());
        for (index, &target) in target_list.iter().enumerate() {
            let replacement = match &replacement_list {
                Some(list) => list.get(index).copied().unwrap_or(Value::Nil),
                None => replacements,
            };
            let target = if self.as_str(target).is_some() || self.as_pattern(target).is_some() {
                Some(self.target_arg(target, is_rex)?)
            } else {
                return Err(VmError::new(code::BAD_TYPE_BIF));
            };
            let replacement = if replacement.is_nil() {
                Replacement::Text(Vec::new())
            } else if let Some(text) = self.as_str(replacement) {
                Replacement::Text(text.to_vec())
            } else if self.is_function_value(replacement) {
                let (min, opt, varargs) = self.value_interface(replacement).unwrap_or((0, 0, true));
                Replacement::Function(replacement, if varargs { -1 } else { min + opt })
            } else {
                return Err(VmError::new(code::BAD_TYPE_BIF));
            };
            rules.push(Rule {
                target,
                replacement,
                found: None,
                groups: [None; GROUP_COUNT],
                group_count: 0,
            });
        }
        let native = if is_rex {
            let mut native = NativeCall::named("rexReplace");
            native.function = self.function_set_index("tads-gen").map(|set| (set, 12));
            native.args = (0..argc).map(|i| self.arg(i)).collect();
            native.caller_pc = self.pc;
            native.arg_top = self.stack.len();
            native
        } else {
            self.native_call("String.findReplace", subject, 11, argc as u32)
        };

        let mut start_char = 1;
        if let Some(value) = arg(self, new_ofs + 2).filter(|v| !v.is_nil()) {
            start_char = self.int_arg(value)?;
        }

        let mut subject = subject;
        let mut serial = 0;
        loop {
            let text = self.require_str(subject)?.to_vec();
            let starts = char_starts(&text);
            let first = index_from(start_char, starts.len() - 1).max(0) as usize;
            let skip = starts[first.min(starts.len() - 1)];
            for rule in &mut rules {
                rule.found = None;
            }
            let mut out: Option<Vec<u8>> = None;
            let mut built: Option<BuiltString> = None;
            let mut pos = skip;
            while pos < text.len() {
                // Find the next match: of the current rule in serial
                // mode, otherwise of whichever rule matches first.
                let mut best: Option<usize> = None;
                let candidates: Vec<usize> = if flags & REPLACE_SERIAL != 0 {
                    if serial >= rules.len() {
                        break;
                    }
                    rules[serial].found = None;
                    vec![serial]
                } else {
                    (0..rules.len()).collect()
                };
                for index in candidates {
                    if rules[index].found.is_none() {
                        let target = rules[index].target.clone();
                        let found = match &target {
                            Some(target) => {
                                self.find_forward(subject, &text, pos, target, case_sensitive)
                            }
                            None => None,
                        };
                        rules[index].found = Some(found);
                        if matches!(target, Some(Target::Pattern(_))) {
                            rules[index].groups = self.last_match.groups;
                            rules[index].group_count = self.last_match.group_count;
                        }
                    }
                    if let Some(Some((offset, _))) = rules[index].found
                        && best.is_none_or(|b| {
                            offset < rules[b].found.flatten().map_or(usize::MAX, |f| f.0)
                        })
                    {
                        best = Some(index);
                    }
                }
                let Some(best) = best else { break };
                let (match_pos, match_len) = rules[best].found.flatten().unwrap_or((pos, 0));
                let matched = &text[match_pos..match_pos + match_len];
                let is_pattern = matches!(rules[best].target, Some(Target::Pattern(_)));
                if is_pattern {
                    self.last_match.groups = rules[best].groups;
                    self.last_match.group_count = rules[best].group_count;
                    self.last_match.subject = subject;
                }
                if limit > 0 {
                    limit -= 1;
                }
                if out.is_none() {
                    built = Some(self.start_built_string(text.len()));
                }
                let built = built.as_mut().expect("made with the first match");
                let out = out.get_or_insert_with(|| text[..skip].to_vec());
                built.make_room(out.len(), match_pos - pos);
                out.extend_from_slice(&text[pos..match_pos]);

                match &rules[best].replacement {
                    Replacement::Text(replacement) => {
                        if !is_pattern && flags & REPLACE_FOLLOW_CASE == 0 {
                            built.make_room(out.len(), replacement.len());
                            out.extend_from_slice(replacement);
                        } else {
                            // Follow the case of the matched text, and
                            // expand %-sequences for a pattern.
                            let matched_chars = chars(matched);
                            let has_upper = matched_chars.iter().any(|c| c.is_uppercase());
                            let has_lower = matched_chars.iter().any(|c| c.is_lowercase());
                            let follow = flags & REPLACE_FOLLOW_CASE != 0;
                            let mut letters = 0;
                            let source = chars(replacement);
                            let mut at = 0;
                            let mut buffer = String::new();
                            while at < source.len() {
                                let c = source[at];
                                if !is_pattern || c != '%' || at + 1 == source.len() {
                                    // The reference makes room for each
                                    // character as it copies it.
                                    built.make_room(out.len() + buffer.len(), 3);
                                    if follow && c.is_lowercase() && has_upper && !has_lower {
                                        buffer.extend(c.to_uppercase());
                                    } else if follow && c.is_lowercase() && has_upper && has_lower {
                                        if letters == 0 {
                                            buffer.extend(c.to_uppercase());
                                        } else {
                                            buffer.push(c);
                                        }
                                        letters += 1;
                                    } else {
                                        buffer.push(c);
                                    }
                                } else {
                                    at += 1;
                                    out.extend_from_slice(buffer.as_bytes());
                                    buffer.clear();
                                    match source[at] {
                                        digit @ '1'..='9' => {
                                            let group = digit as usize - '1' as usize;
                                            if group < rules[best].group_count
                                                && let Some((a, b)) = rules[best].groups[group]
                                            {
                                                built.make_room(out.len(), b - a);
                                                out.extend_from_slice(&text[a..b]);
                                            }
                                        }
                                        '*' => {
                                            built.make_room(out.len(), matched.len());
                                            out.extend_from_slice(matched);
                                        }
                                        '%' => {
                                            built.make_room(out.len(), 1);
                                            out.push(b'%');
                                        }
                                        other => {
                                            built.make_room(out.len(), 4);
                                            buffer.push('%');
                                            buffer.push(other);
                                        }
                                    }
                                }
                                at += 1;
                            }
                            out.extend_from_slice(buffer.as_bytes());
                        }
                    }
                    Replacement::Function(function, max_args) => {
                        // The callback gets the match, its position, and
                        // the original string, as far as it has
                        // parameters for them.
                        let (function, max_args) = (*function, *max_args);
                        let matched_value = self.new_string(matched);
                        let position = Value::Int(char_count(&text[..match_pos]) as i32 + 1);
                        let all = [matched_value, position, subject];
                        let count = if max_args < 0 { 3 } else { max_args.min(3) } as usize;
                        let result = self.call_value(function, &all[..count], native.clone())?;
                        if !result.is_nil() {
                            let replacement = self.require_str(result)?;
                            built.make_room(out.len(), replacement.len());
                            out.extend_from_slice(&replacement);
                        }
                    }
                }

                pos = match_pos + match_len;
                // After an empty match, copy a character and move on.
                if match_len == 0 && pos < text.len() {
                    let next = next_char(&text, pos);
                    built.make_room(out.len(), 3);
                    out.extend_from_slice(&text[pos..next]);
                    pos = next;
                }
                for rule in &mut rules {
                    if let Some(Some((offset, _))) = rule.found
                        && offset < pos
                    {
                        rule.found = None;
                    }
                }
                if limit == 0 {
                    break;
                }
            }
            if let (Some(mut out), Some(built)) = (out, built) {
                out.extend_from_slice(&text[pos.min(text.len())..]);
                subject = self.finish_built_string(built, out);
            }
            // In serial mode, each rule in turn is applied to the result
            // of the one before.
            serial += 1;
            if flags & REPLACE_SERIAL == 0 || limit == 0 || serial >= rules.len() {
                break;
            }
        }
        Ok(subject)
    }

    // ---- the rex functions ----

    /// `rexMatch(pat, str, index?)`: the length in characters of a match
    /// at a position, or nil.
    pub(crate) fn rex_match(&mut self, argc: u32) -> VmResult<Value> {
        let subject = self.arg(1);
        let position = if argc >= 3 {
            match self.arg(2) {
                Value::Int(n) => n,
                _ => return Err(VmError::new(code::BAD_TYPE_BIF)),
            }
        } else {
            1
        };
        let Target::Pattern(pattern) = self.target_arg(self.arg(0), true)? else {
            return Err(VmError::new(code::BAD_TYPE_BIF));
        };
        let text = self.require_str(subject)?;
        let starts = char_starts(&text);
        let start = index_from(position, starts.len() - 1).clamp(0, starts.len() as i32 - 1);
        let from = starts[start as usize];
        let mut groups = [None; GROUP_COUNT];
        let found = pattern.match_at(&text, from, text.len(), true, &mut groups);
        self.record_match(
            subject,
            &pattern,
            found.map(|len| (from, from + len)),
            groups,
        );
        Ok(match found {
            Some(len) => Value::Int(char_count(&text[from..from + len]) as i32),
            None => Value::Nil,
        })
    }

    /// `rexSearch` and `rexSearchLast(pat, str, index?)`: the position,
    /// length, and text of a match, as a list, or nil.
    pub(crate) fn rex_search(&mut self, argc: u32, forward: bool) -> VmResult<Value> {
        let subject = self.arg(1);
        let position = if argc >= 3 {
            match self.arg(2) {
                Value::Int(n) => n,
                _ => return Err(VmError::new(code::BAD_TYPE_BIF)),
            }
        } else if forward {
            1
        } else {
            0
        };
        let target = self.target_arg(self.arg(0), true)?;
        let text = self.require_str(subject)?.to_vec();
        let starts = char_starts(&text);
        let count = starts.len() - 1;
        let start = if position == 0 && !forward {
            count as i32
        } else {
            index_from(position, count)
        }
        .clamp(0, count as i32);
        let from = starts[start as usize];
        let found = if forward {
            self.find_forward(subject, &text, from, &target, true)
        } else {
            self.find_backward(subject, &text, from, &target)
        };
        Ok(match found {
            Some((offset, len)) => {
                let matched = self.new_string(&text[offset..offset + len]);
                self.new_list(vec![
                    Value::Int(char_count(&text[..offset]) as i32 + 1),
                    Value::Int(char_count(&text[offset..offset + len]) as i32),
                    matched,
                ])
            }
            None => Value::Nil,
        })
    }

    /// `rexGroup(n)`: the position, length, and text of a group of the
    /// last match, as a list; group 0 is the whole match.
    pub(crate) fn rex_group(&mut self) -> VmResult<Value> {
        let number = self.int_arg(self.arg(0))?;
        let range = if number == 0 {
            self.last_match.whole
        } else {
            if number < 1 || number as usize > GROUP_COUNT {
                return Err(VmError::new(code::BAD_VAL_BIF));
            }
            let group = number as usize - 1;
            if group >= self.last_match.group_count {
                return Ok(Value::Nil);
            }
            self.last_match.groups[group]
        };
        let (Some((start, end)), Some(text)) = (range, self.as_str(self.last_match.subject)) else {
            return Ok(Value::Nil);
        };
        if end > text.len() || start > end {
            return Ok(Value::Nil);
        }
        let matched = self.new_string(&text[start..end]);
        Ok(self.new_list(vec![
            Value::Int(char_count(&text[..start]) as i32 + 1),
            Value::Int(char_count(&text[start..end]) as i32),
            matched,
        ]))
    }
}

/// A string being built up a piece at a time, and the room the reference
/// would have allocated for it so far.
struct BuiltString {
    id: ObjId,
    /// The bytes there is room for.
    room: usize,
    /// The reference's heap cell. See [`crate::heap::realloc`].
    cell: u16,
}

impl BuiltString {
    /// Make room for `need` more bytes after the first `len`, growing by
    /// 512 more than that when there is not enough.
    fn make_room(&mut self, len: usize, need: usize) {
        if len + need > self.room {
            self.room = len + need + 512;
            crate::heap::realloc(&mut self.cell, self.room as i64 + 2, "string");
        }
    }
}
