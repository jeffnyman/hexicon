//! The tokenizer: the stream of tokens the parser reads.
//!
//! It sits on top of the preprocessor. Each line comes through macro
//! expansion, and its tokens are read off one at a time; when a line
//! runs out, the next one is fetched. Keywords are told apart from other
//! symbols here, and strings are translated: the token for a string has
//! the string's text, with escape sequences replaced by the characters
//! they stand for. A string with expressions embedded in it comes out as
//! several tokens: the text before the first `<<`, then the tokens of the
//! expression, then the text after `>>`, and so on.
//!
//! This is a child of the preprocessor's module because the two share
//! the line being read.

use super::{Preprocessor, Severity, show_name};
use crate::lex::{self, LexCtx, Tok, at, count_quotes, skip_ws_and_markers};

/// A token as the parser sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SrcToken {
    /// What kind of token it is.
    pub typ: Tok,
    /// Its text. For a string, this is the translated contents.
    pub text: Vec<u8>,
    /// The value, for an integer.
    pub int_val: u32,
    /// The file it came from: an index for [`Preprocessor::file_name`].
    pub file: usize,
    /// The line it came from.
    pub line: u32,
}

impl SrcToken {
    pub(crate) fn new(typ: Tok) -> Self {
        Self {
            typ,
            text: Vec::new(),
            int_val: 0,
            file: 0,
            line: 0,
        }
    }
}

/// How many characters of an unterminated string a message quotes.
const UNTERM_SHOWN: usize = 20;

impl Preprocessor<'_> {
    /// Read tokens for the compiler proper, instead of writing out
    /// preprocessed text: directives that are the compiler's to act on
    /// are acted on.
    pub fn set_compile_mode(&mut self) {
        self.pp_only = false;
    }

    /// The token most recently read.
    pub fn cur_token(&self) -> &SrcToken {
        &self.cur
    }

    /// Read the next token. At the end of the source, the token is
    /// [`Tok::Eof`], however often it is asked for.
    pub fn next_token(&mut self) -> Tok {
        self.prv = self.cur.clone();
        if self.gave_up {
            self.cur = SrcToken::new(Tok::Eof);
            self.cur.int_val = self.last_int_val;
            return Tok::Eof;
        }
        if let Some(token) = self.pushed.pop() {
            self.cur = token;
            return self.cur.typ;
        }
        loop {
            let typ = self.next_on_line_xlat_keep();
            if typ == Tok::NullTok {
                continue;
            }
            if typ != Tok::Eof {
                return typ;
            }
            let at_directive_end = at(self.token_buf(), self.p) == lex::END_PP_LINE;
            if at_directive_end || self.read_line_pp() {
                self.cur = SrcToken::new(Tok::Eof);
                self.cur.int_val = self.last_int_val;
                return Tok::Eof;
            }
        }
    }

    /// Put the current token back, to be read again, and make the one
    /// before it current. Only one token can be put back this way.
    pub fn unget_token(&mut self) {
        let current = std::mem::replace(&mut self.cur, self.prv.clone());
        self.pushed.push(current);
        self.prv = SrcToken::new(Tok::Invalid);
    }

    /// Put the current token back, to be read again, and make a token
    /// that was read earlier current in its place.
    pub fn unget_to(&mut self, earlier: SrcToken) {
        let current = std::mem::replace(&mut self.cur, earlier);
        self.pushed.push(current);
        self.prv = SrcToken::new(Tok::Invalid);
    }

    /// Put a token in front of what is still to be read.
    pub fn push_token(&mut self, token: SrcToken) {
        self.pushed.push(token);
    }

    /// The text that tokens are being read from: the macro-expanded line,
    /// or the line itself while a directive is being read.
    fn token_buf(&self) -> &[u8] {
        if self.tok_in_expbuf {
            &self.expbuf
        } else {
            &self.linebuf
        }
    }

    fn take_token_buf(&mut self) -> Vec<u8> {
        if self.tok_in_expbuf {
            std::mem::take(&mut self.expbuf)
        } else {
            std::mem::take(&mut self.linebuf)
        }
    }

    fn restore_token_buf(&mut self, buf: Vec<u8>) {
        if self.tok_in_expbuf {
            self.expbuf = buf;
        } else {
            self.linebuf = buf;
        }
    }

    fn make_token(&self, typ: Tok, text: Vec<u8>) -> SrcToken {
        SrcToken {
            typ,
            text,
            int_val: self.last_int_val,
            file: self.last_desc.unwrap_or(0),
            line: self.last_line,
        }
    }

    /// Read the next token on the current line into the current token.
    /// [`Tok::Eof`] is the end of the line.
    fn next_on_line_xlat_keep(&mut self) -> Tok {
        loop {
            let buf = self.take_token_buf();
            skip_ws_and_markers(&buf, &mut self.p);
            let c = at(&buf, self.p);
            let next = at(&buf, self.p + 1);
            let is_string = c == b'"'
                || c == b'\''
                || (c == b'>'
                    && self.main_ec.in_expr()
                    && self.main_ec.parens() == 0
                    && next == b'>')
                || (c == b'R' && (next == b'"' || next == b'\''));
            if is_string {
                let typ = self.xlat_string_to(&buf, true);
                self.restore_token_buf(buf);
                return typ;
            }

            let mut cx = LexCtx {
                ec: Some(&mut self.main_ec),
                expanding: false,
                in_pp_expr: false,
                notes: &mut self.notes,
            };
            let tok = lex::next_on_line(&buf, &mut self.p, &mut cx);
            let text = tok.text(&buf).to_vec();
            let mut typ = tok.typ;
            if typ == Tok::Invalid {
                // The character after one that is not valid is skipped
                // too, as in the reference.
                let width = utf8_width(at(&buf, self.p));
                if at(&buf, self.p) != 0 {
                    self.p += width;
                }
            }
            self.restore_token_buf(buf);
            self.flush_notes();
            match typ {
                Tok::Sym => {
                    if let Some(keyword) = lex::keyword(&text) {
                        typ = keyword;
                    }
                }
                Tok::Invalid => {
                    self.report(
                        Severity::Error,
                        format!("invalid character in input \"{}\"", show_name(&text)),
                    );
                    continue;
                }
                _ => {}
            }
            if typ == Tok::Int {
                self.last_int_val = tok.int_val;
            }
            self.cur = self.make_token(typ, text);
            return typ;
        }
    }

    /// Read a string, or the part of one up to an embedded expression,
    /// or the part after one, translating it into the current token.
    /// With `embed`, expressions embedded in the string are followed.
    pub(super) fn xlat_string_to(&mut self, buf: &[u8], embed: bool) -> Tok {
        let mut dst: Vec<u8> = Vec::new();
        let mut p = self.p;
        let mut qu = at(buf, p);
        let mut typ = match qu {
            b'"' => Tok::DStr,
            b'\'' => Tok::SStr,
            b'R' => Tok::ReStr,
            _ if embed => end_token(self.main_ec.in_expr(), self.main_ec.quote()),
            _ => Tok::Invalid,
        };
        if qu == b'R' {
            p += 1;
            qu = at(buf, p);
        }
        let mut triple = false;
        if (qu == b'"' || qu == b'\'') && count_quotes(buf, p, qu) >= 3 {
            p += 2;
            triple = true;
        }
        p += 1;
        if qu == b'>' && embed && self.main_ec.parens() == 0 {
            // The end of an embedded expression: the string it is in
            // carries on.
            p += 1;
            qu = self.main_ec.quote();
            triple = self.main_ec.triple();
            self.main_ec.end_expr();
        }

        loop {
            let cur = at(buf, p);
            if cur == qu {
                if triple {
                    let mut count = count_quotes(buf, p, qu);
                    if count >= 3 {
                        // Quotes before the last three are part of the
                        // string.
                        while count > 3 {
                            dst.push(qu);
                            p += 1;
                            count -= 1;
                        }
                        p += 3;
                        break;
                    }
                } else {
                    p += 1;
                    break;
                }
            }
            if cur == 0 {
                self.p = p;
                // A string with nothing in it yet, at the end of a line
                // that was cut off from text still to come, is a quote
                // that belongs with that text.
                if dst.is_empty() && qu == b'"' && !self.unsplicebuf.is_empty() {
                    self.unsplicebuf.insert(0, b'"');
                    return Tok::NullTok;
                }
                let shown: String = String::from_utf8_lossy(&dst)
                    .chars()
                    .take(UNTERM_SHOWN)
                    .collect();
                let quote = char::from(qu);
                self.report(
                    Severity::Error,
                    format!(
                        "unterminated string literal: string started with {quote}{}{quote}",
                        show_name(shown.as_bytes())
                    ),
                );
                self.cur = self.make_token(typ, dst);
                return typ;
            }
            if cur == b'\\' {
                xlat_escape(&mut dst, buf, &mut p, qu, triple);
                continue;
            } else if embed && typ != Tok::ReStr && cur == b'<' && at(buf, p + 1) == b'<' {
                typ = match typ {
                    Tok::DStr => Tok::DStrStart,
                    Tok::DStrEnd => Tok::DStrMid,
                    Tok::SStr => Tok::SStrStart,
                    Tok::SStrEnd => Tok::SStrMid,
                    other => other,
                };
                p += 2;
                // "<<%fmt" is a format for the value of the expression.
                // It becomes a token of its own, read after this one.
                if at(buf, p) == b'%' {
                    let start = p;
                    lex::scan_sprintf_spec(buf, &mut p);
                    let mut spec = Vec::new();
                    let mut q = start;
                    while q < p {
                        if at(buf, q) == b'\\' {
                            xlat_escape(&mut spec, buf, &mut q, 0, false);
                        } else {
                            spec.push(at(buf, q));
                            q += 1;
                        }
                    }
                    let token = self.make_token(Tok::FmtSpec, spec);
                    self.pushed.push(token);
                }
                if !self.main_ec.start_expr(qu, triple) {
                    self.report(
                        Severity::Error,
                        "embedded expressions in strings are nested too deeply".to_owned(),
                    );
                }
                break;
            }
            dst.push(cur);
            p += 1;
        }
        self.p = p;
        self.cur = self.make_token(typ, dst);
        typ
    }
}

/// The token for the part of a string that follows an embedded
/// expression, before it is known whether another expression follows.
fn end_token(in_expr: bool, quote: u8) -> Tok {
    if !in_expr {
        Tok::Invalid
    } else if quote == b'"' {
        Tok::DStrEnd
    } else {
        Tok::SStrEnd
    }
}

/// How many bytes the character that starts with `first` takes.
fn utf8_width(first: u8) -> usize {
    match first {
        0xc0..=0xdf => 2,
        0xe0..=0xef => 3,
        0xf0..=0xf7 => 4,
        _ => 1,
    }
}

fn push_char(dst: &mut Vec<u8>, code: u32) {
    let c = char::from_u32(code).unwrap_or('\u{fffd}');
    dst.extend_from_slice(c.encode_utf8(&mut [0; 4]).as_bytes());
}

/// Translate the escape sequence at `p`, which is at its backslash, and
/// leave `p` after it.
pub(super) fn xlat_escape(dst: &mut Vec<u8>, buf: &[u8], p: &mut usize, qu: u8, triple: bool) {
    *p += 1;
    let mut cur = at(buf, *p);
    match cur {
        // Characters the output formatter takes as instructions.
        b'^' => cur = 0x0f,
        b'v' => cur = 0x0e,
        b'b' => cur = 0x0b,
        b' ' => cur = 0x15,
        b'n' => cur = b'\n',
        b'r' => cur = b'\r',
        b't' => cur = b'\t',
        b'"' | b'\'' => {
            // In a triple-quoted string, a backslash escapes a whole run
            // of quotes.
            if triple && cur == qu {
                while at(buf, *p) == qu {
                    dst.push(cur);
                    *p += 1;
                }
                return;
            }
        }
        b'u' | b'x' => {
            let most = if cur == b'u' { 4 } else { 2 };
            *p += 1;
            let mut acc = 0u32;
            for _ in 0..most {
                let Some(digit) = char::from(at(buf, *p)).to_digit(16) else {
                    break;
                };
                acc = 16 * acc + digit;
                *p += 1;
            }
            push_char(dst, acc);
            return;
        }
        b'0'..=b'7' => {
            let mut acc = 0u32;
            for _ in 0..3 {
                let c = at(buf, *p);
                if !(b'0'..=b'7').contains(&c) || 8 * acc + u32::from(c - b'0') > 255 {
                    break;
                }
                acc = 8 * acc + u32::from(c - b'0');
                *p += 1;
            }
            push_char(dst, acc);
            return;
        }
        b'<' | b'>' | b'\\' => {}
        0 => return,
        _ => {
            // Not an escape the language has: the backslash stays.
            dst.push(b'\\');
        }
    }
    if cur.is_ascii() {
        dst.push(cur);
        *p += 1;
    } else {
        // A character of several bytes after the backslash: all of it.
        let width = utf8_width(cur);
        for _ in 0..width {
            if at(buf, *p) != 0 {
                dst.push(at(buf, *p));
                *p += 1;
            }
        }
    }
}

/// What the parser needs besides tokens: where it is, and somewhere to
/// report problems.
impl Preprocessor<'_> {
    /// The file and line of the token most recently read.
    pub(crate) fn last_pos(&self) -> (usize, u32) {
        (self.last_desc.unwrap_or(0), self.last_line)
    }

    /// Report an error at the current position.
    pub(crate) fn log_error(&mut self, message: impl Into<String>) {
        self.error(message);
    }

    /// Report a warning at the current position.
    pub(crate) fn log_warning(&mut self, message: impl Into<String>) {
        self.warning(message);
    }

    /// Report a problem at a given position.
    pub(crate) fn log_at(&mut self, file: usize, line: u32, severity: Severity, message: String) {
        let file = self.file_name(file).to_owned();
        self.push_diagnostic(super::Diagnostic {
            file,
            line,
            severity,
            message,
        });
    }

    /// The current token as a message shows it.
    pub(crate) fn cur_token_shown(&self) -> String {
        let text = String::from_utf8_lossy(&self.cur.text);
        let quoted = |before: &str, after: &str| {
            // The first twenty characters, with control characters
            // written out, and a mark if there are more.
            let mut shown = String::from(before);
            let mut chars = text.chars();
            for c in chars.by_ref().take(20) {
                let code = u32::from(c);
                if code >= 32 {
                    shown.push(c);
                    continue;
                }
                shown.push('\\');
                match code {
                    10 => shown.push('n'),
                    13 => shown.push('r'),
                    0x0f => shown.push('^'),
                    0x0e => shown.push('v'),
                    0x0b => shown.push('b'),
                    0x15 => shown.push(' '),
                    9 => shown.push('t'),
                    _ => {
                        // The reference means to give the code in
                        // hexadecimal, but adds the digit zero to what
                        // is already a digit.
                        shown.push('x');
                        for shift in [12, 8, 4, 0] {
                            let nibble = (code >> shift) & 0xf;
                            let digit = if nibble < 10 {
                                u32::from(b'0') + nibble
                            } else {
                                u32::from(b'A') + nibble - 10
                            };
                            shown.push(char::from_u32(u32::from(b'0') + digit).unwrap_or('?'));
                        }
                    }
                }
            }
            if chars.next().is_some() {
                shown.push_str("...");
            }
            shown.push_str(after);
            shown
        };
        let shown = match self.cur.typ {
            Tok::Eof => "<End Of File>".to_owned(),
            Tok::SStr => quoted("'", "'"),
            Tok::DStr => quoted("\"", "\""),
            Tok::DStrStart => quoted("\"", "<<"),
            Tok::DStrMid => quoted(">>", "<<"),
            Tok::DStrEnd => quoted(">>", "\""),
            Tok::ReStr => quoted("R'", "'"),
            _ => text.into_owned(),
        };
        show_name(shown.as_bytes())
    }
}
