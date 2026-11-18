//! `String.specialsToHtml` and `String.specialsToText`: turning the
//! output conventions of the console formatter into standard HTML or
//! plain text.
//!
//! Text meant for the console uses control characters for things HTML
//! has no word for: a newline that does nothing at the start of a line,
//! a blank line, a space that is never collapsed, and flags to
//! capitalize the next letter. These methods spell those out, for a
//! program that sends its output to a browser or a file instead.
//!
//! A program converts its output a piece at a time, and a tag or a run
//! of newlines can be split between pieces, so the state between calls
//! is kept in an object the caller passes in.

use crate::error::{VmError, VmResult, code};
use crate::value::{INVALID_PROP, Value};
use crate::vm::Vm;

// The control characters of console text.
const BLANK_LINE: char = '\u{0b}';
const CAPS: char = '\u{0f}';
const NO_CAPS: char = '\u{0e}';
const QUOTED_SPACE: char = '\u{15}';

/// What carries over from one call to the next.
#[derive(Debug, Default)]
struct State {
    /// Text has been written on the current line.
    in_line: bool,
    caps: bool,
    nocaps: bool,
    /// A space or a quoted space is waiting to be written.
    space: bool,
    qspace: bool,
    in_tag: bool,
    in_entity: bool,
    /// Inside a `<q>`, so the next one is an inner quote.
    qlevel: bool,
    /// The quote that an attribute value in the current tag opened with.
    attr_quote: Option<char>,
    /// The column, for tab stops.
    col: i32,
    /// The text of the tag or entity so far.
    tag: String,
}

impl State {
    fn from_flags(flags: i32) -> Self {
        Self {
            in_line: flags & 0x0001 != 0,
            caps: flags & 0x0002 != 0,
            nocaps: flags & 0x0004 != 0,
            in_tag: flags & 0x0008 != 0,
            attr_quote: if flags & 0x0010 != 0 {
                Some('"')
            } else if flags & 0x0020 != 0 {
                Some('\'')
            } else {
                None
            },
            space: flags & 0x0040 != 0,
            qspace: flags & 0x0080 != 0,
            qlevel: flags & 0x0100 != 0,
            in_entity: flags & 0x0200 != 0,
            col: (flags & 0x3000) >> 12,
            tag: String::new(),
        }
    }

    fn flags(&self) -> i32 {
        i32::from(self.in_line)
            | i32::from(self.caps) << 1
            | i32::from(self.nocaps) << 2
            | i32::from(self.in_tag) << 3
            | match self.attr_quote {
                Some('"') => 0x0010,
                Some(_) => 0x0020,
                None => 0,
            }
            | i32::from(self.space) << 6
            | i32::from(self.qspace) << 7
            | i32::from(self.qlevel) << 8
            | i32::from(self.in_entity) << 9
            | (self.col & 3) << 12
    }
}

/// The name of a tag, and whether it is an end tag.
fn tag_name(tag: &str) -> (&str, bool) {
    let rest = tag.trim_start();
    let (rest, is_end) = match rest.strip_prefix('/') {
        Some(rest) => (rest.trim_start(), true),
        None => (rest, false),
    };
    let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
    (&rest[..end], is_end)
}

/// The value of an attribute of a tag, without its quotes.
fn attr_value(tag: &str, wanted: &str) -> Option<String> {
    let (name, _) = tag_name(tag);
    let after_name = tag.find(name).map_or(tag.len(), |at| at + name.len());
    let mut rest = &tag[after_name..];
    loop {
        rest = rest.trim_start();
        if rest.is_empty() {
            return None;
        }
        let name_end = rest
            .find(|c: char| c == '=' || c.is_whitespace())
            .unwrap_or(rest.len());
        let name = &rest[..name_end];
        // A value runs to the next space outside quotes. An attribute
        // without a value stands for its own name.
        let (value, next) = match rest[name_end..].strip_prefix('=') {
            Some(after) => {
                let mut quote = None;
                let mut end = after.len();
                for (at, c) in after.char_indices() {
                    if c.is_whitespace() && quote.is_none() {
                        end = at;
                        break;
                    }
                    if quote.is_none() && (c == '"' || c == '\'') {
                        quote = Some(c);
                    } else if quote == Some(c) {
                        quote = None;
                    }
                }
                (&after[..end], &after[end..])
            }
            None => (name, &rest[name_end..]),
        };
        if name.eq_ignore_ascii_case(wanted) {
            let mut quote = None;
            let mut out = String::new();
            for c in value.chars() {
                if quote.is_none() && (c == '"' || c == '\'') {
                    quote = Some(c);
                } else if quote == Some(c) {
                    quote = None;
                } else {
                    out.push(c);
                }
            }
            return Some(out);
        }
        rest = next;
    }
}

/// The number at the start of some text, as C's `atoi` reads it.
fn leading_int(text: &str) -> i32 {
    let text = text.trim_start();
    let (negative, digits) = match text.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, text.strip_prefix('+').unwrap_or(text)),
    };
    let value = digits
        .chars()
        .map_while(|c| c.to_digit(10))
        .fold(0i32, |acc, d| acc.wrapping_mul(10).wrapping_add(d as i32));
    if negative {
        value.wrapping_neg()
    } else {
        value
    }
}

/// Convert one piece of console text.
fn convert(text: &str, html: bool, st: &mut State) -> String {
    let mut out = String::with_capacity(text.len() * 5 / 4);
    let line_break = if html { "<br>" } else { "\n" };
    for ch in text.chars() {
        if st.in_tag {
            match ch {
                '\n' | BLANK_LINE | QUOTED_SPACE | '\t' => st.tag.push(' '),
                CAPS | NO_CAPS => {}
                '"' | '\'' => {
                    if st.attr_quote.is_none() {
                        st.attr_quote = Some(ch);
                    } else if st.attr_quote == Some(ch) {
                        st.attr_quote = None;
                    }
                    st.tag.push(ch);
                }
                '>' if st.attr_quote.is_some() => st.tag.push(ch),
                '>' => {
                    st.in_tag = false;
                    let tag = std::mem::take(&mut st.tag);
                    end_tag(&tag, html, st, &mut out);
                }
                _ => st.tag.push(ch),
            }
            continue;
        }

        if st.in_entity {
            // An entity ends at its semicolon, at ten characters, or,
            // for a numbered one, at anything that is not a digit.
            if ch == ';' || st.tag.len() >= 10 || (st.tag.starts_with('#') && !ch.is_ascii_digit())
            {
                let name = std::mem::take(&mut st.tag);
                if let Some(number) = name.strip_prefix('#') {
                    out.extend(char::from_u32(leading_int(number) as u32));
                    if ch != ';' {
                        out.push(ch);
                    }
                } else {
                    match name.to_ascii_lowercase().as_str() {
                        "nbsp" => out.push(' '),
                        "gt" => out.push('>'),
                        "lt" => out.push('<'),
                        "amp" => out.push('&'),
                        "ldquo" | "rdquo" | "quot" => out.push('"'),
                        "lsquo" | "rsquo" => out.push('\''),
                        _ => out.push_str(&name),
                    }
                }
                st.in_entity = false;
            } else {
                st.tag.push(ch);
            }
            continue;
        }

        // A space is held back until the next character shows whether it
        // is followed by a quoted space, which absorbs it.
        if st.space {
            if ch != QUOTED_SPACE {
                out.push(' ');
                st.col += 1;
            }
            st.space = false;
        } else if st.qspace {
            if ch == ' ' {
                continue;
            }
            out.push_str(if html && ch == QUOTED_SPACE {
                "&nbsp;"
            } else {
                " "
            });
            st.col += 1;
            st.qspace = false;
        }

        match ch {
            '\n' => {
                if st.in_line {
                    out.push_str(line_break);
                }
                st.in_line = false;
                st.col = 0;
                st.caps = false;
                st.nocaps = false;
            }
            BLANK_LINE => {
                out.push_str(line_break);
                if st.in_line {
                    out.push_str(line_break);
                }
                st.in_line = false;
                st.col = 0;
                st.caps = false;
                st.nocaps = false;
            }
            CAPS => {
                st.caps = true;
                st.nocaps = false;
            }
            NO_CAPS => {
                st.caps = false;
                st.nocaps = true;
            }
            QUOTED_SPACE => st.qspace = true,
            ' ' => st.space = true,
            '\t' => {
                if html {
                    // Tab stops are every four columns.
                    st.col = ((st.col + 1) & 3) - 1;
                    while st.col + 1 < 3 {
                        out.push_str("&nbsp;");
                        st.col += 1;
                    }
                    out.push(' ');
                    st.col += 1;
                } else {
                    out.push('\t');
                }
            }
            '<' => st.in_tag = true,
            '&' if !html => st.in_entity = true,
            '&' => out.push('&'),
            _ => {
                if st.caps {
                    out.extend(ch.to_uppercase());
                    st.caps = false;
                } else if st.nocaps {
                    out.extend(ch.to_lowercase());
                    st.nocaps = false;
                } else {
                    out.push(ch);
                }
                st.in_line = true;
                st.col += 1;
            }
        }
    }
    out
}

/// Handle a complete tag.
fn end_tag(tag: &str, html: bool, st: &mut State, out: &mut String) {
    let (name, is_end) = tag_name(tag);
    let name = name.to_ascii_lowercase();
    let mut keep = true;
    match name.as_str() {
        "q" => {
            // Quotes alternate between double and single as they nest.
            if is_end {
                st.qlevel = !st.qlevel;
            }
            let index = usize::from(st.qlevel) * 2 + usize::from(is_end);
            out.push_str(if html {
                ["&ldquo;", "&rdquo;", "&lsquo;", "&rsquo;"][index]
            } else {
                ["\"", "\"", "'", "'"][index]
            });
            st.col += 1;
            if !is_end {
                st.qlevel = !st.qlevel;
            }
            keep = false;
        }
        "br" => {
            // `height=N` asks for N blank lines, on top of ending the
            // current line if there is one.
            if let Some(height) = attr_value(tag, "height") {
                let mut lines = leading_int(&height);
                if st.in_line {
                    lines += 1;
                }
                for _ in 0..lines.max(0) {
                    out.push_str(if html { "<br>" } else { "\n" });
                }
                keep = false;
            } else if !html {
                out.push('\n');
            }
            st.in_line = false;
            st.col = 0;
        }
        "p" => {
            if !html {
                out.push('\n');
                if st.in_line {
                    out.push('\n');
                }
            }
            st.in_line = false;
            st.col = 0;
        }
        "div" | "center" | "table" | "td" | "th" | "caption" => {
            st.in_line = false;
            st.col = 0;
            if !html {
                out.push('\n');
            }
        }
        _ => {}
    }
    if html && keep {
        out.push('<');
        out.push_str(tag);
        out.push('>');
    }
}

impl Vm {
    /// `specialsToHtml(state?)` or `specialsToText(state?)`.
    pub(crate) fn specials_to(&mut self, text: &[u8], argc: u32, html: bool) -> VmResult<Value> {
        let state_obj = match if argc >= 1 { self.arg(0) } else { Value::Nil } {
            Value::Nil => None,
            Value::Obj(id) => Some(id),
            _ => return Err(VmError::new(code::BAD_TYPE_BIF)),
        };
        let flags_prop = self.predef.string_sth_flags;
        let tag_prop = self.predef.string_sth_tag;

        let mut state = State::default();
        if let Some(id) = state_obj {
            let object = Value::Obj(id);
            if flags_prop != INVALID_PROP {
                if let Value::Int(flags) = self.call_prop(object, flags_prop, 0, None)? {
                    state = State::from_flags(flags);
                }
            }
            if tag_prop != INVALID_PROP {
                let saved = self.call_prop(object, tag_prop, 0, None)?;
                if let Some(tag) = self.as_str(saved) {
                    state.tag = String::from_utf8_lossy(&tag).into_owned();
                }
            }
        }

        let converted = convert(&String::from_utf8_lossy(text), html, &mut state);

        if let Some(id) = state_obj {
            if flags_prop != INVALID_PROP {
                self.set_prop(id, flags_prop, Value::Int(state.flags()))?;
            }
            if tag_prop != INVALID_PROP {
                let tag = if state.in_tag || state.in_entity {
                    self.new_str(&state.tag)
                } else {
                    Value::Nil
                };
                self.set_prop(id, tag_prop, tag)?;
            }
        }
        Ok(self.new_str(&converted))
    }
}
