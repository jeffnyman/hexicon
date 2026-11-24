//! Byte packing: converting values to and from binary data according to
//! a format string, for `packBytes` and `unpackBytes`.
//!
//! A format is a list of items. Each is a type letter, such as `l` for a
//! 32-bit integer or `a` for text, followed by modifiers: a repeat count
//! or `*`, `<` or `>` for byte order, and a few others. Items can be
//! grouped in parentheses, or in square brackets where a group
//! corresponds to a list, and `count/item` stores a count before a
//! repeated item.
//!
//! This follows the reference VM step for step, since the
//! exact handling of counts, padding, and running out of arguments is
//! what programs depend on.

use crate::decimal::Big;
use crate::decimal_math;
use crate::error::{ErrParam, VmError, VmResult, code};
use crate::object::Object;
use crate::value::Value;
use crate::vm::Vm;

/// No repeat count was given.
const ITER_NONE: i32 = -1;
/// The repeat count is `*`: as many as there are.
const ITER_STAR: i32 = -2;

/// Bytes being read or written, with a position.
pub(crate) struct Stream {
    pub data: Vec<u8>,
    pub pos: usize,
    /// Whether writing past the end extends the data. A byte array has a
    /// fixed size.
    growable: bool,
    /// Positions that cannot be read: characters of a string source that
    /// do not fit in a byte.
    wide: Vec<usize>,
}

impl Stream {
    /// An empty stream that grows as it is written.
    pub fn growable() -> Self {
        Self {
            data: Vec::new(),
            pos: 0,
            growable: true,
            wide: Vec::new(),
        }
    }

    /// A stream over data that grows as it is written past its end.
    pub fn growing(data: Vec<u8>, pos: usize) -> Self {
        let pos = pos.min(data.len());
        Self {
            data,
            pos,
            growable: true,
            wide: Vec::new(),
        }
    }

    /// A stream over data of a fixed size.
    pub fn fixed(data: Vec<u8>, pos: usize) -> Self {
        let pos = pos.min(data.len());
        Self {
            data,
            pos,
            growable: false,
            wide: Vec::new(),
        }
    }

    /// A stream over the characters of a string, each of which has to
    /// fit in a byte by the time it is read.
    pub fn from_text(text: &[u8]) -> Self {
        let mut data = Vec::new();
        let mut wide = Vec::new();
        for c in String::from_utf8_lossy(text).chars() {
            match u8::try_from(u32::from(c)) {
                Ok(byte) => data.push(byte),
                Err(_) => {
                    wide.push(data.len());
                    data.push(b'?');
                }
            }
        }
        Self {
            data,
            pos: 0,
            growable: false,
            wide,
        }
    }

    fn size(&self) -> i64 {
        self.data.len() as i64
    }

    fn at(&self) -> i64 {
        self.pos as i64
    }

    fn at_end(&self) -> bool {
        self.pos >= self.data.len()
    }

    fn seek(&mut self, to: i64) {
        self.pos = to.clamp(0, self.size()) as usize;
    }

    fn skip(&mut self, by: i64) {
        self.seek(self.at() + by);
    }

    fn read(&mut self, len: usize) -> VmResult<Vec<u8>> {
        let end = self.pos + len;
        if end > self.data.len() {
            return Err(VmError::new(code::READ_FILE));
        }
        if self.wide.iter().any(|&at| at >= self.pos && at < end) {
            return Err(VmError::new(code::NUM_OVERFLOW));
        }
        let bytes = self.data[self.pos..end].to_vec();
        self.pos = end;
        Ok(bytes)
    }

    /// Write bytes at the position. False means they did not all fit.
    fn write(&mut self, bytes: &[u8]) -> bool {
        let room = if self.growable {
            bytes.len()
        } else {
            bytes.len().min(self.data.len() - self.pos)
        };
        let end = self.pos + room;
        if end > self.data.len() {
            self.data.resize(end, 0);
        }
        self.data[self.pos..end].copy_from_slice(&bytes[..room]);
        self.pos = end;
        room == bytes.len()
    }

    fn write_checked(&mut self, bytes: &[u8]) -> VmResult<()> {
        if self.write(bytes) {
            Ok(())
        } else {
            Err(VmError::new(code::WRITE_FILE))
        }
    }

    /// Write `count` bytes of padding, alternating between two values.
    fn pad(&mut self, first: u8, second: u8, count: i64) -> VmResult<()> {
        let bytes: Vec<u8> = (0..count.max(0))
            .map(|i| if i % 2 == 0 { first } else { second })
            .collect();
        self.write_checked(&bytes)
    }
}

/// A position in the format string.
#[derive(Debug, Clone, Copy, Default)]
struct Pos {
    at: usize,
    end: usize,
}

impl Pos {
    /// The position as error messages count it, from one.
    fn index(&self) -> i32 {
        self.at as i32 + 1
    }
}

/// One item of a format: a type and its modifiers.
#[derive(Debug, Clone)]
struct Type {
    code: char,
    /// The text of a literal item.
    lit: Pos,
    big_endian: bool,
    count: i32,
    /// `!` on a type whose count is in characters: count bytes instead.
    count_in_bytes: bool,
    /// A count and a `*` together: up to that many.
    up_to_count: bool,
    /// `!`: align, or measure from the start of everything.
    bang: bool,
    /// `~`: unpack as an integer where one is big enough.
    tilde: bool,
    /// `%`: quietly truncate a value that does not fit.
    pct: bool,
    /// `?` on `@`: unpack the position.
    qu: bool,
    null_term: bool,
    /// Where the item is in the format, for error messages.
    fmtidx: i32,
}

impl Type {
    fn in_group(group: &Group) -> Self {
        Self {
            code: '\0',
            lit: Pos::default(),
            big_endian: group.big_endian,
            count: ITER_NONE,
            count_in_bytes: false,
            up_to_count: false,
            bang: false,
            tilde: group.tilde,
            pct: group.pct,
            qu: false,
            null_term: false,
            fmtidx: 0,
        }
    }

    fn has_count(&self) -> bool {
        self.count >= 0
    }

    fn get_count(&self) -> i32 {
        if self.has_count() { self.count } else { 1 }
    }
}

/// A parenthesized group being worked through.
#[derive(Debug, Clone)]
struct Group {
    close: char,
    big_endian: bool,
    tilde: bool,
    pct: bool,
    /// Where the group's items start in the format.
    start: Pos,
    /// Where the current iteration started in the data.
    stream_ofs: i64,
    /// Where the outermost group started in the data.
    root_ofs: i64,
    cur_iter: i32,
    num_iters: i32,
    up_to_iters: bool,
}

/// Where the values to pack come from.
enum Args {
    /// Arguments on the stack, from `cur` up to but not including `end`.
    Stack { cur: usize, end: usize },
    /// The elements of a list. A value that is not a list stands for a
    /// list of itself, marked by a negative length.
    List { list: Value, idx: i32, len: i32 },
    /// One value.
    Single { value: Value, used: bool },
}

impl Args {
    fn list(vm: &mut Vm, list: Value) -> VmResult<Self> {
        let len = match list_len(vm, list)? {
            Some(len) => len as i32,
            None => -1,
        };
        Ok(Self::List { list, idx: 1, len })
    }

    fn single(value: Value) -> Self {
        Self::Single { value, used: false }
    }

    fn remaining(&self) -> i32 {
        match *self {
            Self::Stack { cur, end } => end.saturating_sub(cur) as i32,
            Self::List { idx, len, .. } => ((if len >= 0 { len } else { 1 }) - idx + 1).max(0),
            Self::Single { used, .. } => i32::from(!used),
        }
    }

    fn more(&self) -> bool {
        self.remaining() > 0
    }

    fn get(&self, vm: &mut Vm) -> VmResult<Value> {
        Ok(match *self {
            Self::Stack { cur, end } if cur < end => vm.arg(cur),
            Self::List { list, idx: 1, len } if len < 0 => list,
            Self::List { list, idx, len } if idx <= len => vm.list_like_get(list, idx as usize)?,
            Self::Single { value, used: false } => value,
            _ => Value::Nil,
        })
    }

    fn next(&mut self) {
        match self {
            Self::Stack { cur, .. } => *cur += 1,
            Self::List { idx, .. } => *idx += 1,
            Self::Single { used, .. } => *used = true,
        }
    }

    fn drain(&mut self) {
        while self.more() {
            self.next();
        }
    }
}

/// The length of a value that packs as a list. A byte array is data, not
/// a list, even though it can be indexed.
fn list_len(vm: &mut Vm, value: Value) -> VmResult<Option<usize>> {
    if vm.as_bytes(value).is_some() {
        return Ok(None);
    }
    vm.list_like_len(value)
}

fn parse_error(index: i32) -> VmError {
    VmError::with(code::PACK_PARSE, vec![ErrParam::Int(index)])
}

fn is_valid_type(c: char) -> bool {
    "aAbuUwWhHcCsSlLqQkfdxX@\"{".contains(c)
}

/// Types whose count is a length, not a number of repetitions.
fn is_varying_length(c: char) -> bool {
    "aAbuUwWhH".contains(c)
}

fn is_repeatable(c: char) -> bool {
    !"aAbuUwWhHxX@".contains(c)
}

fn type_consumes_arg(c: char) -> bool {
    !"xX@\"{".contains(c)
}

fn padding_char(c: char) -> u8 {
    if "AUW".contains(c) { b' ' } else { 0 }
}

fn is_space(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\n' | '\r' | '\x0b' | '\x0c')
}

/// How the characters of a text item are stored.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Conv {
    Latin1,
    Byte,
    Utf8,
    Ucs2 { big_endian: bool },
    Hex { high_first: bool },
}

impl Conv {
    fn for_type(t: &Type) -> Option<Self> {
        Some(match t.code {
            'a' | 'A' => Self::Latin1,
            'b' => Self::Byte,
            'u' | 'U' => Self::Utf8,
            'w' | 'W' => Self::Ucs2 {
                big_endian: t.big_endian,
            },
            'h' => Self::Hex { high_first: false },
            'H' => Self::Hex { high_first: true },
            _ => return None,
        })
    }

    fn count_to_bytes(self, count: i32) -> i32 {
        match self {
            Self::Ucs2 { .. } => count * 2,
            Self::Hex { .. } => (count + 1) / 2,
            _ => count,
        }
    }

    fn bytes_to_count(self, bytes: i64) -> i64 {
        match self {
            Self::Ucs2 { .. } => (bytes + 1) / 2,
            Self::Hex { .. } => bytes * 2,
            _ => bytes,
        }
    }

    fn byte_count(self, t: &Type) -> i32 {
        if t.count_in_bytes {
            t.get_count()
        } else {
            self.count_to_bytes(t.get_count())
        }
    }
}

/// The 32-bit integer BER encoding. The reference keeps only the top bit
/// of the highest group, so a value loses bits 28 to 30; packed data has
/// to match, so this does too.
fn encode_ber(value: u32) -> Vec<u8> {
    if value == 0 {
        return vec![0];
    }
    let mut out = Vec::new();
    let mut mask = 0x08;
    for shift in [28, 21, 14, 7, 0] {
        let group = ((value >> shift) & mask) as u8;
        if group != 0 || !out.is_empty() {
            out.push(group);
        }
        mask = 0x7f;
    }
    // A value with only the lost bits set leaves nothing, which the
    // reference does not survive. Write it as zero.
    if out.is_empty() {
        return vec![0];
    }
    let last = out.len() - 1;
    for byte in &mut out[..last] {
        *byte |= 0x80;
    }
    out
}

fn decode_ber(groups: &[u8]) -> i32 {
    let mut value: i32 = 0;
    let mut shift = 0;
    for &group in groups.iter().rev() {
        value = value.wrapping_add(i32::from(group).wrapping_shl(shift));
        shift += 7;
    }
    value
}

/// The state of one pack or unpack call.
struct Packer<'a> {
    vm: &'a mut Vm,
    fmt: Vec<char>,
    ds: Stream,
}

impl Packer<'_> {
    // ---- reading the format ----

    fn raw(&self, p: &Pos) -> char {
        if p.at < p.end { self.fmt[p.at] } else { '\0' }
    }

    /// The next character that is not a space.
    fn getch(&self, p: &mut Pos) -> char {
        while p.at < p.end && is_space(self.fmt[p.at]) {
            p.at += 1;
        }
        self.raw(p)
    }

    fn more(&self, p: &mut Pos) -> bool {
        self.getch(p);
        p.at < p.end
    }

    fn inc(p: &mut Pos) {
        if p.at < p.end {
            p.at += 1;
        }
    }

    fn nextch(&self, p: &mut Pos) -> char {
        let c = self.getch(p);
        Self::inc(p);
        c
    }

    fn parse_int(&self, p: &mut Pos) -> VmResult<i32> {
        let start = p.index();
        let mut acc: i32 = 0;
        while let Some(digit) = self.getch(p).to_digit(10) {
            acc = acc.wrapping_mul(10).wrapping_add(digit as i32);
            if acc < 0 {
                return Err(parse_error(start));
            }
            Self::inc(p);
        }
        Ok(acc)
    }

    fn parse_type(&self, p: &mut Pos, group: &Group) -> VmResult<Type> {
        let c = self.nextch(p);
        if !is_valid_type(c) {
            return Err(parse_error(p.index() - 1));
        }
        let mut t = Type::in_group(group);
        t.fmtidx = p.index() - 1;
        t.code = c;
        if c == '"' {
            // A literal runs to the closing quote. A doubled quote is a
            // quote in the text.
            t.lit = *p;
            loop {
                if !self.more(p) {
                    return Err(parse_error(p.index()));
                }
                if self.getch(p) == '"' {
                    let end = p.at;
                    Self::inc(p);
                    if self.getch(p) != '"' {
                        t.lit.end = end;
                        break;
                    }
                }
                Self::inc(p);
            }
        } else if c == '{' {
            // Literal bytes in hexadecimal.
            t.lit = *p;
            loop {
                if !self.more(p) {
                    return Err(parse_error(p.index()));
                }
                let c = self.getch(p);
                if c == '}' {
                    t.lit.end = p.at;
                    Self::inc(p);
                    break;
                } else if !c.is_ascii_hexdigit() {
                    return Err(parse_error(p.index()));
                }
                Self::inc(p);
            }
        }
        self.parse_mods(p, &mut t)?;
        Ok(t)
    }

    fn parse_mods(&self, p: &mut Pos, t: &mut Type) -> VmResult<()> {
        while self.more(p) {
            let c = self.getch(p);
            match c {
                '<' => {
                    t.big_endian = false;
                    Self::inc(p);
                }
                '>' => {
                    t.big_endian = true;
                    Self::inc(p);
                }
                '0' => {
                    if t.code != '\0' && !"auwhH".contains(t.code) {
                        return Err(parse_error(p.index()));
                    }
                    t.null_term = true;
                    Self::inc(p);
                }
                '1'..='9' => {
                    if t.count == ITER_STAR {
                        t.up_to_count = true;
                    }
                    let count_index = p.index();
                    t.count = self.parse_int(p)?;
                    let min = i32::from(t.code != '@');
                    if t.count < min {
                        return Err(parse_error(count_index));
                    }
                }
                ':' => {
                    // The count is the size of another type. In the
                    // reference, every text type and `k` falls through to
                    // the two-byte case.
                    Self::inc(p);
                    t.count = match self.nextch(p) {
                        'a' | 'A' | 'b' | 'u' | 'U' | 'h' | 'H' | 'k' | 'w' | 'W' => 2,
                        'c' | 'C' => 1,
                        's' | 'S' => 2,
                        'l' | 'L' | 'f' => 4,
                        'q' | 'Q' | 'd' => 8,
                        _ => return Err(parse_error(p.index())),
                    };
                }
                '*' => {
                    if t.count >= 0 {
                        t.up_to_count = true;
                    } else {
                        t.count = if matches!(t.code, 'x' | 'X' | '@') {
                            0
                        } else {
                            ITER_STAR
                        };
                    }
                    Self::inc(p);
                }
                '!' => {
                    if "wWhH".contains(t.code) {
                        t.count_in_bytes = true;
                    } else {
                        t.bang = true;
                    }
                    Self::inc(p);
                }
                '~' => {
                    t.tilde = true;
                    Self::inc(p);
                }
                '?' => {
                    t.qu = true;
                    Self::inc(p);
                }
                '%' => {
                    t.pct = true;
                    Self::inc(p);
                }
                _ => break,
            }
        }
        Ok(())
    }

    /// Skip to the parenthesis that closes a group.
    fn skip_group(&self, p: &mut Pos, close: char) -> VmResult<()> {
        let mut depth = 1;
        Self::inc(p);
        while self.more(p) {
            let c = self.getch(p);
            match c {
                '(' | '[' | '{' => depth += 1,
                ')' | ']' | '}' => depth -= 1,
                _ => {}
            }
            if c == close && depth == 0 {
                break;
            }
            Self::inc(p);
        }
        if !self.more(p) {
            return Err(parse_error(p.index()));
        }
        Ok(())
    }

    /// Read the modifiers that follow a group, which apply to everything
    /// in it, without moving past the group.
    fn parse_group_mods(&self, p: &Pos, gt: &mut Type) -> VmResult<()> {
        let mut p = *p;
        let close = match self.getch(&mut p) {
            '(' => ')',
            '[' => ']',
            '{' => '}',
            _ => '\0',
        };
        Self::inc(&mut p);
        self.skip_group(&mut p, close)?;
        Self::inc(&mut p);
        self.parse_mods(&mut p, gt)
    }

    fn child_group(&self, parent: &Group, open: char, start: Pos, t: &Type) -> Group {
        Group {
            close: match open {
                '(' => ')',
                '[' => ']',
                '{' => '}',
                _ => '\0',
            },
            big_endian: t.big_endian,
            tilde: t.tilde,
            pct: t.pct,
            start,
            stream_ofs: self.ds.at(),
            root_ofs: parent.root_ofs,
            cur_iter: 1,
            num_iters: t.count,
            up_to_iters: t.up_to_count,
        }
    }

    fn root_group(&self, start: Pos) -> Group {
        Group {
            close: '\0',
            big_endian: false,
            tilde: false,
            pct: false,
            start,
            stream_ofs: self.ds.at(),
            root_ofs: self.ds.at(),
            cur_iter: 1,
            num_iters: 1,
            up_to_iters: false,
        }
    }

    /// How far to move to reach a multiple of the item's count, measured
    /// from the start of the data.
    fn alignment_padding(&self, t: &Type, group: &Group, forward: bool) -> i64 {
        let c = i64::from(t.count);
        if c <= 1 {
            return 0;
        }
        let over = (self.ds.at() - group.root_ofs) % c;
        if over == 0 {
            0
        } else if forward {
            c - over
        } else {
            -over
        }
    }

    // ---- packing ----

    fn pack_group(
        &mut self,
        p: &mut Pos,
        args_main: &mut Args,
        group: &mut Group,
        list_per_iter: bool,
    ) -> VmResult<()> {
        // In a `[...]!` group, each iteration takes its values from the
        // next argument, which is a list.
        let first = args_main.get(self.vm)?;
        let mut list_args = Args::list(self.vm, first)?;
        let more = args_main.more();
        if list_per_iter {
            args_main.next();
        }
        if (group.num_iters == ITER_STAR || group.up_to_iters) && !more {
            *p = group.start;
            return self.skip_group(p, group.close);
        }
        while self.more(p) {
            let c = self.getch(p);
            if c == '(' || c == '[' {
                let args = if list_per_iter {
                    &mut list_args
                } else {
                    &mut *args_main
                };
                self.pack_subgroup(p, args, group, None)?;
            } else if c == group.close {
                let more = if list_per_iter {
                    let more = args_main.more();
                    let next = args_main.get(self.vm)?;
                    list_args = Args::list(self.vm, next)?;
                    args_main.next();
                    more
                } else {
                    args_main.more()
                };
                let done = if group.num_iters == ITER_STAR {
                    !more
                } else {
                    group.cur_iter >= group.num_iters
                };
                if done || (group.up_to_iters && !more) {
                    if group.up_to_iters {
                        args_main.drain();
                    }
                    return Ok(());
                }
                group.cur_iter += 1;
                group.stream_ofs = self.ds.at();
                *p = group.start;
            } else {
                let t = self.parse_type(p, group)?;
                let args = if list_per_iter {
                    &mut list_args
                } else {
                    &mut *args_main
                };
                if self.getch(p) == '/' {
                    // A count, then what is counted.
                    Self::inc(p);
                    let c = self.getch(p);
                    if c == '(' || c == '[' {
                        self.pack_subgroup(p, args, group, Some(&t))?;
                    } else {
                        let counted = self.parse_type(p, group)?;
                        self.pack_item(group, args, &counted, Some(&t))?;
                    }
                } else {
                    self.pack_item(group, args, &t, None)?;
                }
            }
        }
        Ok(())
    }

    fn pack_subgroup(
        &mut self,
        p: &mut Pos,
        args: &mut Args,
        group: &Group,
        prefix_count: Option<&Type>,
    ) -> VmResult<()> {
        let mut gt = Type::in_group(group);
        self.parse_group_mods(p, &mut gt)?;
        let paren = self.getch(p);
        Self::inc(p);
        let mut child = self.child_group(group, paren, *p, &gt);

        // A `[...]` group packs the elements of one list argument, or
        // with `!`, of a list argument for each iteration.
        let arg = args.get(self.vm)?;
        let mut list_args = Args::list(self.vm, arg)?;
        let mut list_per_iter = false;
        let mut use_list = false;
        if paren == '[' {
            if gt.bang {
                list_per_iter = true;
            } else {
                use_list = true;
                args.next();
            }
        }

        // Leave room for the count, to fill in once it is known.
        let prefix_pos = self.ds.at();
        if let Some(count_type) = prefix_count {
            self.pack_one_item(group, &mut Args::single(Value::Int(0)), count_type, None)?;
            child.num_iters = ITER_STAR;
        }

        let child_args = if use_list { &mut list_args } else { args };
        self.pack_group(p, child_args, &mut child, list_per_iter)?;
        if self.nextch(p) != child.close {
            return Err(parse_error(p.index()));
        }

        if let Some(count_type) = prefix_count {
            let end = self.ds.at();
            self.ds.seek(prefix_pos);
            if count_type.code == 'k'
                || (is_varying_length(count_type.code) && count_type.count == ITER_STAR)
            {
                return Err(VmError::with(
                    code::PACK_ARG_MISMATCH,
                    vec![ErrParam::Int(count_type.fmtidx)],
                ));
            }
            let count = Value::Int(child.cur_iter);
            self.pack_one_item(group, &mut Args::single(count), count_type, None)?;
            self.ds.seek(end);
        }
        self.parse_mods(p, &mut gt)
    }

    fn pack_item(
        &mut self,
        group: &Group,
        args: &mut Args,
        t: &Type,
        prefix_count: Option<&Type>,
    ) -> VmResult<()> {
        let value = args.get(self.vm)?;
        let count = if prefix_count.is_some() {
            ITER_STAR
        } else {
            t.count
        };
        if let Some(len) = list_len(self.vm, value)? {
            // A list argument supplies the values for a repeated item.
            let len = len as i32;
            let mut list = Args::list(self.vm, value)?;
            if type_consumes_arg(t.code) {
                args.next();
            }
            if let Some(count_type) = prefix_count {
                let mut count_arg = Args::single(Value::Int(len));
                self.pack_one_item(group, &mut count_arg, count_type, None)?;
            }
            let mut iters = match count {
                ITER_STAR => len,
                count if count < 1 => 1,
                count => count,
            };
            if t.up_to_count && iters > len {
                iters = len;
            }
            for _ in 0..iters {
                self.pack_one_item(group, &mut list, t, None)?;
            }
        } else if is_repeatable(t.code) && (count == ITER_STAR || count > 1) {
            if let Some(count_type) = prefix_count {
                let mut count_arg = Args::single(Value::Int(args.remaining()));
                self.pack_one_item(group, &mut count_arg, count_type, None)?;
            }
            let mut n = 0;
            while if count == ITER_STAR {
                args.more()
            } else {
                n < count
            } {
                if t.up_to_count && !args.more() {
                    break;
                }
                self.pack_one_item(group, args, t, None)?;
                n += 1;
            }
            if t.up_to_count {
                args.drain();
            }
        } else {
            self.pack_one_item(group, args, t, prefix_count)?;
        }
        Ok(())
    }

    /// An integer argument within a range. With `%`, a value outside it
    /// is truncated instead of being an error.
    fn int_value(
        &mut self,
        value: Value,
        min: i64,
        max: i64,
        pct: bool,
        mask: u32,
    ) -> VmResult<i64> {
        let n: i32 = match value {
            Value::Nil => 0,
            Value::Int(n) => n,
            other => match self.vm.as_big(other) {
                Some(big) => {
                    let (n, overflow) = if min < 0 {
                        big.to_i32()
                    } else {
                        let (n, overflow) = big.to_u32();
                        (n as i32, overflow)
                    };
                    if overflow && !pct {
                        return Err(VmError::new(code::NUM_OVERFLOW));
                    }
                    n
                }
                None => self.vm.cast_to_int(other)?,
            },
        };
        if pct {
            Ok(i64::from(n as u32 & mask))
        } else if i64::from(n) < min || i64::from(n) > max {
            Err(VmError::new(code::NUM_OVERFLOW))
        } else {
            Ok(i64::from(n))
        }
    }

    /// A value as a Decimal: an integer, a number written as text, or
    /// a Decimal.
    fn big_value(&mut self, value: Value) -> VmResult<Big> {
        if let Value::Int(n) = value {
            return Ok(Big::from_int(n.into(), 10));
        }
        if let Some(text) = self.vm.as_str(value) {
            let text = String::from_utf8_lossy(&text).into_owned();
            return Ok(Big::parse(&text, Big::precision_from_str(&text)));
        }
        self.vm
            .as_big(value)
            .cloned()
            .ok_or_else(|| VmError::new(code::NO_BIGNUM_CONV))
    }

    fn write_num(&mut self, mut bytes: Vec<u8>, t: &Type) -> VmResult<()> {
        if t.big_endian {
            bytes.reverse();
        }
        self.ds.write_checked(&bytes)
    }

    fn pack_one_item(
        &mut self,
        group: &Group,
        args: &mut Args,
        t: &Type,
        prefix_count: Option<&Type>,
    ) -> VmResult<()> {
        // The items that take no argument.
        match t.code {
            'x' => {
                let count = if t.bang {
                    self.alignment_padding(t, group, true)
                } else {
                    t.get_count().into()
                };
                return self.ds.pad(0, 0, count);
            }
            'X' => {
                let back = if t.bang {
                    self.alignment_padding(t, group, false)
                } else {
                    -i64::from(t.get_count())
                };
                self.ds.skip(back);
                return Ok(());
            }
            '"' => {
                let mut bytes = Vec::new();
                let mut p = t.lit;
                while p.at < p.end {
                    let c = self.raw(&p);
                    if c == '"' {
                        Self::inc(&mut p);
                    }
                    let Ok(byte) = u8::try_from(u32::from(c)) else {
                        return Err(VmError::new(code::NUM_OVERFLOW));
                    };
                    bytes.push(byte);
                    Self::inc(&mut p);
                }
                self.ds.write(&bytes);
                return Ok(());
            }
            '{' => {
                let mut bytes = Vec::new();
                let mut p = t.lit;
                while self.more(&mut p) {
                    let high = self.nextch(&mut p).to_digit(16).unwrap_or(0);
                    let low = self.nextch(&mut p).to_digit(16).unwrap_or(0);
                    bytes.push(((high << 4) + low) as u8);
                }
                self.ds.write(&bytes);
                return Ok(());
            }
            '@' => {
                if !t.qu {
                    let base = if t.bang {
                        group.root_ofs
                    } else {
                        group.stream_ofs
                    };
                    let distance = base + i64::from(t.get_count()) - self.ds.at();
                    if distance > 0 {
                        self.ds.pad(0, 0, distance)?;
                    } else {
                        self.ds.skip(distance);
                    }
                }
                return Ok(());
            }
            _ => {}
        }

        let value = args.get(self.vm)?;
        args.next();

        if let Some(conv) = Conv::for_type(t) {
            return self.pack_text(group, value, t, conv, prefix_count);
        }

        let pct = t.pct;
        match t.code {
            'c' => {
                let n = self.int_value(value, -128, 127, pct, 0xff)?;
                self.write_num(vec![n as u8], t)
            }
            'C' => {
                let n = self.int_value(value, 0, 255, pct, 0xff)?;
                self.write_num(vec![n as u8], t)
            }
            's' => {
                let n = self.int_value(value, -32768, 32767, pct, 0xffff)?;
                self.write_num((n as u16).to_le_bytes().to_vec(), t)
            }
            'S' => {
                let n = self.int_value(value, 0, 65535, pct, 0xffff)?;
                self.write_num((n as u16).to_le_bytes().to_vec(), t)
            }
            'l' => {
                let n = self.int_value(value, i32::MIN.into(), i32::MAX.into(), pct, u32::MAX)?;
                self.write_num((n as u32).to_le_bytes().to_vec(), t)
            }
            'L' => {
                // An unsigned value past the integer range has to come as
                // a Decimal or as text.
                let n = if self.vm.as_big(value).is_some() || self.vm.as_str(value).is_some() {
                    let (n, overflow) = self.big_value(value)?.to_u32();
                    if overflow && !pct {
                        return Err(VmError::new(code::NUM_OVERFLOW));
                    }
                    n
                } else {
                    self.int_value(value, 0, i32::MAX.into(), pct, u32::MAX)? as u32
                };
                self.write_num(n.to_le_bytes().to_vec(), t)
            }
            'q' | 'Q' => {
                let signed = t.code == 'q';
                let bytes = match value {
                    Value::Int(n) if signed => i64::from(n).to_le_bytes(),
                    Value::Int(n) => {
                        if n < 0 && !pct {
                            return Err(VmError::new(code::NUM_OVERFLOW));
                        }
                        u64::from(n as u32).to_le_bytes()
                    }
                    Value::Nil => [0; 8],
                    other => {
                        let (bytes, overflow) = self.big_value(other)?.to_p8(signed);
                        if overflow && !pct {
                            return Err(VmError::new(code::NUM_OVERFLOW));
                        }
                        bytes
                    }
                };
                self.write_num(bytes.to_vec(), t)
            }
            'k' => {
                let bytes = match value {
                    Value::Int(n) => {
                        if n < 0 && !pct {
                            return Err(VmError::new(code::NUM_OVERFLOW));
                        }
                        encode_ber(if n < 0 {
                            n.wrapping_neg() as u32
                        } else {
                            n as u32
                        })
                    }
                    Value::Nil => vec![0],
                    other => {
                        let (bytes, overflow) = self.big_value(other)?.to_ber();
                        if overflow && !pct {
                            return Err(VmError::new(code::NUM_OVERFLOW));
                        }
                        bytes
                    }
                };
                self.ds.write_checked(&bytes)
            }
            'f' | 'd' => {
                let bits = if t.code == 'f' { 32 } else { 64 };
                let bytes = if value == Value::Nil {
                    vec![0; bits / 8]
                } else {
                    let number = self.big_value(value)?;
                    let (bytes, overflow) =
                        decimal_math::to_ieee754(&number, bits, &mut self.vm.big_cache)
                            .map_err(|_| VmError::new(code::OUT_OF_RANGE))?;
                    if overflow && !pct {
                        return Err(VmError::new(code::NUM_OVERFLOW));
                    }
                    bytes
                };
                self.write_num(bytes, t)
            }
            _ => Ok(()),
        }
    }

    /// Pack a text item: a string, or a byte array taken as characters.
    fn pack_text(
        &mut self,
        group: &Group,
        value: Value,
        t: &Type,
        conv: Conv,
        prefix_count: Option<&Type>,
    ) -> VmResult<()> {
        let (chars, from_bytes): (Vec<char>, bool) = match self.vm.as_bytes(value) {
            Some(bytes) => (bytes.iter().map(|&b| char::from(b)).collect(), true),
            None => {
                let text = self.vm.cast_to_text(value)?;
                (String::from_utf8_lossy(&text).chars().collect(), false)
            }
        };

        if let Some(count_type) = prefix_count {
            // The count is the length in the units the type counts in.
            let len = chars.len() as i32;
            let array_len = match t.code {
                'a' | 'A' | 'b' => len,
                'w' | 'W' => len * if t.count_in_bytes { 2 } else { 1 },
                'h' | 'H' if t.count_in_bytes => (len + 1) / 2,
                'h' | 'H' => len,
                // UTF-8 counts bytes.
                _ if from_bytes => chars
                    .iter()
                    .map(|&c| if (c as u32) < 128 { 1 } else { 2 })
                    .sum(),
                _ => chars.iter().map(|c| c.len_utf8() as i32).sum(),
            };
            let mut count_arg = Args::single(Value::Int(array_len));
            self.pack_one_item(group, &mut count_arg, count_type, None)?;
        }

        // How many bytes the field holds, if it has a fixed size.
        let field = if prefix_count.is_some()
            || t.count == ITER_STAR
            || (t.count == ITER_NONE && t.null_term)
        {
            ITER_STAR
        } else {
            conv.byte_count(t)
        };

        let mut out: Vec<u8> = Vec::new();
        let mut total = 0;
        let mut half: u8 = 0;
        let mut taken = 0;
        for (index, &c) in chars.iter().enumerate() {
            taken = index + 1;
            let before = out.len();
            let code_point = c as u32;
            match conv {
                Conv::Latin1 => out.push(match u8::try_from(code_point) {
                    Ok(byte) => byte,
                    Err(_) if t.pct => code_point as u8,
                    Err(_) => b'?',
                }),
                Conv::Byte => out.push(match u8::try_from(code_point) {
                    Ok(byte) => byte,
                    Err(_) if t.pct => code_point as u8,
                    Err(_) => return Err(VmError::new(code::NUM_OVERFLOW)),
                }),
                Conv::Utf8 => out.extend_from_slice(c.encode_utf8(&mut [0; 4]).as_bytes()),
                Conv::Ucs2 { big_endian } => {
                    let unit = code_point as u16;
                    out.extend_from_slice(&if big_endian {
                        unit.to_be_bytes()
                    } else {
                        unit.to_le_bytes()
                    });
                }
                Conv::Hex { high_first } => {
                    // Two digits make a byte. Anything else counts as 0.
                    let digit = c.to_digit(16).unwrap_or(0) as u8;
                    let first = index % 2 == 0;
                    let nibble = if first == high_first {
                        digit << 4
                    } else {
                        digit & 0x0f
                    };
                    if first {
                        half = nibble;
                    } else {
                        out.push(half | nibble);
                    }
                }
            }
            total += (out.len() - before) as i32;
            if field > 0 && total > field {
                // This character does not fit: leave it out.
                total -= (out.len() - before) as i32;
                out.truncate(before);
                break;
            }
        }
        if matches!(conv, Conv::Hex { .. }) && taken % 2 == 1 {
            // An odd digit at the end makes a byte with a zero.
            out.push(half);
        }
        self.ds.write_checked(&out)?;

        // Fill out a fixed field.
        if total < field && !t.up_to_count {
            let pad = padding_char(t.code);
            let count = i64::from(conv.byte_count(t) - total);
            match conv {
                Conv::Ucs2 { big_endian: false } => self.ds.pad(pad, 0, count)?,
                Conv::Ucs2 { big_endian: true } => self.ds.pad(0, pad, count)?,
                _ => self.ds.pad(pad, pad, count)?,
            }
        }
        if t.null_term {
            let terminator: &[u8] = if matches!(conv, Conv::Ucs2 { .. }) {
                &[0, 0]
            } else {
                &[0]
            };
            self.ds.write_checked(terminator)?;
        }
        Ok(())
    }

    // ---- unpacking ----

    fn unpack_group(
        &mut self,
        p: &mut Pos,
        out: &mut Vec<Value>,
        group: &mut Group,
        list_per_iter: bool,
    ) -> VmResult<()> {
        if (group.num_iters == ITER_STAR || group.up_to_iters) && self.ds.at_end() {
            *p = group.start;
            return self.skip_group(p, group.close);
        }
        // In a `[...]!` group, each iteration makes a list of its own.
        let mut iteration: Vec<Value> = Vec::new();
        while self.more(p) {
            let c = self.getch(p);
            let target = if list_per_iter {
                &mut iteration
            } else {
                &mut *out
            };
            if c == '(' || c == '[' {
                self.unpack_subgroup(p, target, group, None)?;
            } else if c == group.close {
                group.cur_iter += 1;
                if list_per_iter {
                    let list = self.vm.new_list(std::mem::take(&mut iteration));
                    out.push(list);
                }
                let done = if group.num_iters == ITER_STAR {
                    self.ds.at_end()
                } else {
                    group.cur_iter > group.num_iters
                };
                if done || (group.up_to_iters && self.ds.at_end()) {
                    return Ok(());
                }
                *p = group.start;
                group.stream_ofs = self.ds.at();
            } else if c == '/' {
                // The value just unpacked is the count for what follows.
                let Some(count) = target.pop() else {
                    return Err(parse_error(p.index()));
                };
                let count = self.vm.cast_to_int(count)?;
                Self::inc(p);
                let c = self.getch(p);
                if c == '(' || c == '[' {
                    self.unpack_subgroup(p, target, group, Some(count))?;
                } else {
                    self.unpack_iter_item(p, target, group, Some(count))?;
                }
            } else {
                self.unpack_iter_item(p, target, group, None)?;
            }
        }
        if list_per_iter {
            let list = self.vm.new_list(iteration);
            out.push(list);
        }
        Ok(())
    }

    fn unpack_subgroup(
        &mut self,
        p: &mut Pos,
        out: &mut Vec<Value>,
        group: &Group,
        prefix_count: Option<i32>,
    ) -> VmResult<()> {
        let mut gt = Type::in_group(group);
        self.parse_group_mods(p, &mut gt)?;
        let paren = self.getch(p);
        Self::inc(p);
        let mut child = self.child_group(group, paren, *p, &gt);
        if let Some(count) = prefix_count {
            child.num_iters = count;
        }
        if paren == '[' && !gt.bang {
            // The whole group makes one list.
            let mut items = Vec::new();
            self.unpack_group(p, &mut items, &mut child, false)?;
            let list = self.vm.new_list(items);
            out.push(list);
        } else {
            self.unpack_group(p, out, &mut child, paren == '[')?;
        }
        if self.nextch(p) != child.close {
            return Err(parse_error(p.index()));
        }
        self.parse_mods(p, &mut gt)
    }

    fn unpack_iter_item(
        &mut self,
        p: &mut Pos,
        out: &mut Vec<Value>,
        group: &Group,
        prefix_count: Option<i32>,
    ) -> VmResult<()> {
        let mut t = self.parse_type(p, group)?;
        match t.code {
            'x' => {
                let by = if t.bang {
                    self.alignment_padding(&t, group, true)
                } else {
                    t.get_count().into()
                };
                self.ds.skip(by);
                return Ok(());
            }
            'X' => {
                let by = if t.bang {
                    self.alignment_padding(&t, group, false)
                } else {
                    -i64::from(t.get_count())
                };
                self.ds.skip(by);
                return Ok(());
            }
            '"' => {
                // Skip the literal's length.
                let mut len = 0;
                let mut lit = t.lit;
                while lit.at < lit.end {
                    if self.raw(&lit) == '"' {
                        Self::inc(&mut lit);
                    }
                    Self::inc(&mut lit);
                    len += 1;
                }
                self.ds.skip(len * i64::from(t.get_count()));
                return Ok(());
            }
            '{' => {
                let mut len = 0;
                let mut lit = t.lit;
                while self.more(&mut lit) {
                    Self::inc(&mut lit);
                    Self::inc(&mut lit);
                    len += 1;
                }
                self.ds.skip(len * i64::from(t.get_count()));
                return Ok(());
            }
            '@' => {
                if t.qu {
                    let value = self.unpack_item(&t, group)?;
                    out.push(value);
                } else {
                    let base = if t.bang {
                        group.root_ofs
                    } else {
                        group.stream_ofs
                    };
                    self.ds.skip(base + i64::from(t.get_count()) - self.ds.at());
                }
                return Ok(());
            }
            _ => {}
        }
        if let Some(count) = prefix_count {
            t.count = count;
        }
        if is_varying_length(t.code) {
            // The count is a length: one value.
            let value = self.unpack_item(&t, group)?;
            out.push(value);
        } else if t.count == ITER_STAR {
            while !self.ds.at_end() {
                let value = self.unpack_item(&t, group)?;
                out.push(value);
            }
        } else {
            for _ in 0..t.get_count() {
                if t.up_to_count && self.ds.at_end() {
                    break;
                }
                let value = self.unpack_item(&t, group)?;
                out.push(value);
            }
        }
        Ok(())
    }

    /// Read one character of a text item.
    fn read_char(&mut self, conv: Conv) -> VmResult<u32> {
        Ok(match conv {
            Conv::Utf8 => {
                let first = self.ds.read(1)?[0];
                let extra = match first {
                    0x00..=0x7f => 0,
                    0x80..=0xdf => 1,
                    0xe0..=0xef => 2,
                    _ => 3,
                };
                let mut bytes = vec![first];
                bytes.extend(self.ds.read(extra)?);
                String::from_utf8_lossy(&bytes)
                    .chars()
                    .next()
                    .map_or(0, u32::from)
            }
            Conv::Ucs2 { big_endian } => {
                let bytes = self.ds.read(2)?;
                u32::from(if big_endian {
                    u16::from_be_bytes([bytes[0], bytes[1]])
                } else {
                    u16::from_le_bytes([bytes[0], bytes[1]])
                })
            }
            _ => self.ds.read(1)?[0].into(),
        })
    }

    fn unpack_text(&mut self, t: &Type, conv: Conv) -> VmResult<Value> {
        // With a terminator and no count, read up to the terminator.
        if t.null_term && t.count <= 0 {
            let mut text = String::new();
            loop {
                let c = self.read_char(conv)?;
                if c == 0 {
                    break;
                }
                text.push(char::from_u32(c).unwrap_or('?'));
            }
            return self.vm.try_new_str(&text);
        }

        let mut count = i64::from(t.get_count());
        if t.count_in_bytes {
            count = conv.bytes_to_count(count);
        }
        if count == 0 {
            return self.vm.try_new_str("");
        }
        if t.count == ITER_STAR || t.up_to_count {
            // Take what there is.
            let available = conv.bytes_to_count(self.ds.size() - self.ds.at());
            if t.count == ITER_STAR {
                if available > i64::from(i32::MAX) {
                    return Err(VmError::new(code::NUM_OVERFLOW));
                }
                count = available;
            } else if available < count {
                count = available;
            }
        }
        let count = count.max(0) as usize;
        let pad = char::from(padding_char(t.code));

        let value = match conv {
            Conv::Byte => {
                let bytes = self.ds.read(count)?;
                self.vm.new_bytes(bytes)
            }
            Conv::Utf8 => {
                // The count is in bytes. Anything that is not UTF-8
                // becomes a question mark.
                let bytes = self.ds.read(count)?;
                let text = String::from_utf8_lossy(&bytes).replace('\u{fffd}', "?");
                self.vm.try_new_str(text.trim_end_matches(pad))?
            }
            _ if count == 0 => self.vm.try_new_str("")?,
            Conv::Latin1 => {
                let bytes = self.ds.read(count)?;
                let text: String = bytes.iter().map(|&b| char::from(b)).collect();
                self.vm.try_new_str(text.trim_end_matches(pad))?
            }
            Conv::Ucs2 { big_endian } => {
                let bytes = self.ds.read(count * 2)?;
                let text: String = bytes
                    .chunks_exact(2)
                    .map(|pair| {
                        let unit = if big_endian {
                            u16::from_be_bytes([pair[0], pair[1]])
                        } else {
                            u16::from_le_bytes([pair[0], pair[1]])
                        };
                        char::from_u32(unit.into()).unwrap_or('?')
                    })
                    .collect();
                self.vm.try_new_str(text.trim_end_matches(pad))?
            }
            Conv::Hex { high_first } => {
                let bytes = self.ds.read(count.div_ceil(2))?;
                let text: String = (0..count)
                    .map(|index| {
                        let byte = bytes[index / 2];
                        let first = index % 2 == 0;
                        let nibble = if first == high_first {
                            byte >> 4
                        } else {
                            byte & 0x0f
                        };
                        char::from_digit(nibble.into(), 16)
                            .unwrap_or('?')
                            .to_ascii_uppercase()
                    })
                    .collect();
                self.vm.try_new_str(&text)?
            }
        };

        if t.null_term {
            // Skip the terminator, and end the text at an earlier one.
            self.read_char(conv)?;
            if let Some(text) = self.vm.as_str(value) {
                if let Some(nul) = text.iter().position(|&b| b == 0) {
                    let cut = text[..nul].to_vec();
                    return self.vm.try_new_string(cut);
                }
            }
        }
        Ok(value)
    }

    fn read_num(&mut self, len: usize, t: &Type) -> VmResult<Vec<u8>> {
        let mut bytes = self.ds.read(len)?;
        if t.big_endian {
            bytes.reverse();
        }
        Ok(bytes)
    }

    fn unpack_item(&mut self, t: &Type, group: &Group) -> VmResult<Value> {
        if let Some(conv) = Conv::for_type(t) {
            return self.unpack_text(t, conv);
        }
        Ok(match t.code {
            '@' => {
                let base = if t.bang {
                    group.root_ofs
                } else {
                    group.stream_ofs
                };
                Value::Int((self.ds.at() - base) as i32)
            }
            'c' => Value::Int(i32::from(self.read_num(1, t)?[0] as i8)),
            'C' => Value::Int(i32::from(self.read_num(1, t)?[0])),
            's' => {
                let b = self.read_num(2, t)?;
                Value::Int(i32::from(i16::from_le_bytes([b[0], b[1]])))
            }
            'S' => {
                let b = self.read_num(2, t)?;
                Value::Int(i32::from(u16::from_le_bytes([b[0], b[1]])))
            }
            'l' => {
                let b = self.read_num(4, t)?;
                Value::Int(i32::from_le_bytes([b[0], b[1], b[2], b[3]]))
            }
            'L' => {
                // An unsigned value is a Decimal, unless `~` asks for
                // an integer where one will do.
                let b = self.read_num(4, t)?;
                let n = u32::from_le_bytes([b[0], b[1], b[2], b[3]]);
                match i32::try_from(n) {
                    Ok(n) if t.tilde => Value::Int(n),
                    _ => self.vm.new_big(Big::from_u64(n.into(), 10)),
                }
            }
            'q' | 'Q' => {
                let signed = t.code == 'q';
                let b = self.read_num(8, t)?;
                let bytes: [u8; 8] = b.try_into().unwrap_or([0; 8]);
                let as_signed = i64::from_le_bytes(bytes);
                let small = i32::try_from(as_signed).ok().filter(|&n| signed || n >= 0);
                match small {
                    Some(n) if t.tilde => Value::Int(n),
                    _ => self.vm.new_big(Big::from_p8(bytes, signed)),
                }
            }
            'k' => {
                let mut groups = Vec::new();
                loop {
                    if groups.len() >= 256 {
                        return Err(VmError::new(code::NUM_OVERFLOW));
                    }
                    let byte = self.ds.read(1)?[0];
                    groups.push(byte & 0x7f);
                    if byte & 0x80 == 0 {
                        break;
                    }
                }
                if groups.len() < 5 || (groups.len() == 5 && groups[0] & 0xf8 == 0) {
                    Value::Int(decode_ber(&groups))
                } else {
                    self.vm.new_big(Big::from_ber(&groups))
                }
            }
            'f' | 'd' => {
                let b = self.read_num(if t.code == 'f' { 4 } else { 8 }, t)?;
                let number = decimal_math::from_ieee754(&b, &mut self.vm.big_cache)
                    .map_err(|_| VmError::new(code::OUT_OF_RANGE))?;
                self.vm.new_big(number)
            }
            _ => Value::Nil,
        })
    }
}

impl Vm {
    /// The contents of a byte array.
    pub(crate) fn as_bytes(&self, value: Value) -> Option<&Vec<u8>> {
        match value {
            Value::Obj(id) => match self.objects.get(id)? {
                Object::ByteArray(bytes) => Some(bytes),
                _ => None,
            },
            _ => None,
        }
    }

    pub(crate) fn new_bytes(&mut self, bytes: Vec<u8>) -> Value {
        Value::Obj(self.alloc_obj(Object::ByteArray(bytes)))
    }

    /// Pack arguments into a stream. Argument `first_arg` is the format
    /// string, and the `argc - 1` after it are the values.
    pub(crate) fn pack(&mut self, first_arg: usize, argc: usize, ds: Stream) -> VmResult<Stream> {
        let (ds, result) = self.pack_stream(first_arg, argc, ds);
        result.map(|()| ds)
    }

    /// Pack arguments into a stream, and give the stream back whether or
    /// not that worked.
    pub(crate) fn pack_stream(
        &mut self,
        first_arg: usize,
        argc: usize,
        ds: Stream,
    ) -> (Stream, VmResult<()>) {
        let format = match self.require_str(self.arg(first_arg)) {
            Ok(format) => format.to_vec(),
            Err(error) => return (ds, Err(error)),
        };
        let fmt: Vec<char> = String::from_utf8_lossy(&format).chars().collect();
        let mut p = Pos {
            at: 0,
            end: fmt.len(),
        };
        let mut packer = Packer { vm: self, fmt, ds };
        let mut root = packer.root_group(p);
        let mut args = Args::Stack {
            cur: first_arg + 1,
            end: first_arg + argc.max(1),
        };
        let mut result = packer.pack_group(&mut p, &mut args, &mut root, false);
        if result.is_ok() && packer.more(&mut p) {
            result = Err(parse_error(p.index()));
        }
        if result.is_ok() && args.more() {
            result = Err(VmError::with(
                code::PACK_ARGC_MISMATCH,
                vec![ErrParam::Int(format.len() as i32 + 1)],
            ));
        }
        (packer.ds, result)
    }

    /// Unpack a stream into a list of values, according to a format.
    pub(crate) fn unpack(&mut self, format: Value, ds: Stream) -> VmResult<Value> {
        self.unpack_stream(format, ds).1
    }

    /// Unpack a stream, and give the stream back whether or not that
    /// worked.
    pub(crate) fn unpack_stream(&mut self, format: Value, ds: Stream) -> (Stream, VmResult<Value>) {
        let format = match self.require_str(format) {
            Ok(format) => format.to_vec(),
            Err(error) => return (ds, Err(error)),
        };
        let fmt: Vec<char> = String::from_utf8_lossy(&format).chars().collect();
        let mut p = Pos {
            at: 0,
            end: fmt.len(),
        };
        let mut packer = Packer { vm: self, fmt, ds };
        let mut root = packer.root_group(p);
        let mut out = Vec::new();
        let mut result = packer.unpack_group(&mut p, &mut out, &mut root, false);
        if result.is_ok() && packer.more(&mut p) {
            result = Err(parse_error(p.index()));
        }
        let ds = packer.ds;
        match result {
            Ok(_) => {
                let list = self.new_list(out);
                (ds, Ok(list))
            }
            Err(error) => (ds, Err(error)),
        }
    }
}
