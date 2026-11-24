//! Finding the next token in a line of source text.
//!
//! The scanner works on bytes. Source text is UTF-8, and everything the
//! scanner looks for is ASCII, so a byte at a time finds the same tokens
//! as a character at a time would: no byte of a longer character is ever
//! an ASCII one.
//!
//! A line that the preprocessor is working on can also hold marker bytes
//! that never appear in source text (they are below a tab, and such
//! characters in source are turned into spaces as a line is read). The
//! markers stand for a macro's parameters and record what has already
//! been expanded.

/// In a macro's stored expansion: a parameter. The next byte is the
/// parameter's number, counting from one.
pub(crate) const FORMAL_FLAG: u8 = 0x01;
/// Before a symbol: it has been through macro expansion already and is
/// not to be expanded again.
pub(crate) const FULLY_EXPANDED_FLAG: u8 = 0x02;
/// After a macro's expansion: the expansion of the macro whose number
/// follows, in [`MACRO_ID_LEN`] bytes, ends here.
pub(crate) const MACRO_EXP_END: u8 = 0x03;
/// The end of a preprocessor directive's line. Nothing is read past it.
pub(crate) const END_PP_LINE: u8 = 0x04;
/// In a macro's stored expansion: `#foreach` on the variable arguments.
pub(crate) const FOREACH_FLAG: u8 = 0x05;
/// In a macro's stored expansion: `#argcount`.
pub(crate) const ARGCOUNT_FLAG: u8 = 0x06;
/// In a macro's stored expansion: `#ifempty`.
pub(crate) const IFEMPTY_FLAG: u8 = 0x07;
/// In a macro's stored expansion: `#ifnempty`.
pub(crate) const IFNEMPTY_FLAG: u8 = 0x08;

/// How many bytes after [`MACRO_EXP_END`] hold the macro's number.
pub(crate) const MACRO_ID_LEN: usize = 4;

/// The longest a symbol can usefully be; the rest is ignored, with a
/// warning.
pub(crate) const SYM_MAX_LEN: usize = 80;

/// The bytes that record the end of a macro's expansion. Each byte of
/// the number has its top bit set, so that none is mistaken for a marker,
/// for the end of the text, or for an ASCII character.
pub(crate) fn macro_end_marker(id: usize) -> [u8; 1 + MACRO_ID_LEN] {
    let mut bytes = [MACRO_EXP_END; 1 + MACRO_ID_LEN];
    for (index, byte) in bytes[1..].iter_mut().enumerate() {
        *byte = 0x80 | ((id >> (7 * index)) & 0x7f) as u8;
    }
    bytes
}

/// The macro number recorded after a [`MACRO_EXP_END`] at `pos`.
pub(crate) fn macro_end_id(buf: &[u8], pos: usize) -> usize {
    (0..MACRO_ID_LEN)
        .map(|index| usize::from(at(buf, pos + 1 + index) & 0x7f) << (7 * index))
        .sum()
}

/// The byte at a position, or zero past the end, as text that ends in a
/// zero byte would read.
pub(crate) fn at(buf: &[u8], pos: usize) -> u8 {
    buf.get(pos).copied().unwrap_or(0)
}

pub(crate) fn is_space(c: u8) -> bool {
    matches!(c, b' ' | b'\t' | b'\n' | 0x0b | 0x0c | b'\r')
}

pub(crate) fn is_digit(c: u8) -> bool {
    c.is_ascii_digit()
}

fn is_odigit(c: u8) -> bool {
    (b'0'..=b'7').contains(&c)
}

fn is_xdigit(c: u8) -> bool {
    c.is_ascii_hexdigit()
}

pub(crate) fn is_syminit(c: u8) -> bool {
    c == b'_' || c.is_ascii_alphabetic()
}

pub(crate) fn is_sym(c: u8) -> bool {
    c == b'_' || c.is_ascii_alphanumeric()
}

/// The kinds of token. They are in the order the reference compiler has
/// them in, so that a kind's number here is its number there, which is
/// what lets the two tokenizers' output be compared.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(missing_docs)]
pub enum Tok {
    Invalid,
    /// Nothing: the reader should go on to the next token.
    NullTok,
    /// The end of the line, or of the source.
    Eof,
    MacroFormal,
    MacroForeach,
    MacroArgCount,
    MacroIfEmpty,
    MacroIfNEmpty,
    Sym,
    Int,
    SStr,
    SStrStart,
    SStrMid,
    SStrEnd,
    DStr,
    DStrStart,
    DStrMid,
    DStrEnd,
    ReStr,
    LPar,
    RPar,
    Comma,
    Dot,
    LBrace,
    RBrace,
    LBrack,
    RBrack,
    Eq,
    EqEq,
    /// `:=`, which the language no longer has.
    Asi,
    Plus,
    Minus,
    Times,
    Div,
    Mod,
    Gt,
    Lt,
    Ge,
    Le,
    Ne,
    Arrow,
    Colon,
    Sem,
    And,
    AndAnd,
    Or,
    OrOr,
    Xor,
    Shl,
    AShr,
    LShr,
    Inc,
    Dec,
    PlusEq,
    MinEq,
    TimesEq,
    DivEq,
    ModEq,
    AndEq,
    OrEq,
    XorEq,
    ShlEq,
    AShrEq,
    LShrEq,
    Not,
    BNot,
    Pound,
    PoundPound,
    PoundAt,
    Ellipsis,
    Question,
    QQ,
    ColonColon,
    Float,
    /// An integer too big to be one, to be made a floating-point number.
    BigInt,
    At,
    DotDot,
    /// The format in `<<%fmt expr>>`.
    FmtSpec,

    // Keywords.
    SelfKw,
    Inherited,
    ArgCount,
    If,
    Else,
    For,
    While,
    Do,
    Switch,
    Case,
    Default,
    Goto,
    Break,
    Continue,
    Function,
    Return,
    Local,
    Object,
    Nil,
    True,
    Pass,
    External,
    Extern,
    FormatString,
    Class,
    Replace,
    Modify,
    New,
    Delete,
    Throw,
    Try,
    Catch,
    Finally,
    Builtin,
    Dictionary,
    Grammar,
    Enum,
    Template,
    Static,
    Foreach,
    Export,
    Delegated,
    TargetProp,
    PropertySet,
    TargetObj,
    DefiningObj,
    Transient,
    Replaced,
    Property,
    Operator,
    Method,
    Invokee,
}

/// The keyword a symbol is, if it is one.
pub(crate) fn keyword(text: &[u8]) -> Option<Tok> {
    Some(match text {
        b"self" => Tok::SelfKw,
        b"targetprop" => Tok::TargetProp,
        b"targetobj" => Tok::TargetObj,
        b"definingobj" => Tok::DefiningObj,
        b"inherited" => Tok::Inherited,
        b"delegated" => Tok::Delegated,
        b"argcount" => Tok::ArgCount,
        b"if" => Tok::If,
        b"else" => Tok::Else,
        b"for" => Tok::For,
        b"while" => Tok::While,
        b"do" => Tok::Do,
        b"switch" => Tok::Switch,
        b"case" => Tok::Case,
        b"default" => Tok::Default,
        b"goto" => Tok::Goto,
        b"break" => Tok::Break,
        b"continue" => Tok::Continue,
        b"function" => Tok::Function,
        b"return" => Tok::Return,
        b"local" => Tok::Local,
        b"object" => Tok::Object,
        b"nil" => Tok::Nil,
        b"true" => Tok::True,
        b"pass" => Tok::Pass,
        b"external" => Tok::External,
        b"extern" => Tok::Extern,
        b"formatstring" => Tok::FormatString,
        b"class" => Tok::Class,
        b"replace" => Tok::Replace,
        b"modify" => Tok::Modify,
        b"new" => Tok::New,
        b"throw" => Tok::Throw,
        b"try" => Tok::Try,
        b"catch" => Tok::Catch,
        b"finally" => Tok::Finally,
        b"builtin" => Tok::Builtin,
        b"dictionary" => Tok::Dictionary,
        b"grammar" => Tok::Grammar,
        b"enum" => Tok::Enum,
        b"template" => Tok::Template,
        b"static" => Tok::Static,
        b"foreach" => Tok::Foreach,
        b"export" => Tok::Export,
        b"propertyset" => Tok::PropertySet,
        b"transient" => Tok::Transient,
        b"replaced" => Tok::Replaced,
        b"property" => Tok::Property,
        b"operator" => Tok::Operator,
        b"method" => Tok::Method,
        b"invokee" => Tok::Invokee,
        _ => return None,
    })
}

/// A token: its kind and where its text is in the buffer it came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Token {
    pub typ: Tok,
    pub pos: usize,
    pub len: usize,
    /// The symbol was marked as already expanded.
    pub fully_expanded: bool,
    /// The value of an integer token.
    pub int_val: u32,
}

impl Token {
    pub(crate) fn new(typ: Tok) -> Self {
        Self {
            typ,
            pos: 0,
            len: 0,
            fully_expanded: false,
            int_val: 0,
        }
    }

    pub(crate) fn end(&self) -> usize {
        self.pos + self.len
    }

    pub(crate) fn text<'a>(&self, buf: &'a [u8]) -> &'a [u8] {
        let end = self.end().min(buf.len());
        &buf[self.pos.min(end)..end]
    }
}

/// How deep embedded expressions can nest: a string in an expression in a
/// string, and so on.
const EMBED_DEPTH: usize = 10;

#[derive(Debug, Clone, Copy, Default)]
struct EmbedLevel {
    /// The quote of the string the expression is in.
    quote: u8,
    triple: bool,
    /// How many brackets are open within the expression.
    parens: i32,
}

/// Where the scanner is in strings with embedded expressions: inside
/// `<<` and `>>`, the text is tokens, and the `>>` goes back to the
/// string.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct EmbedCtx {
    level: usize,
    stack: [EmbedLevel; EMBED_DEPTH],
}

impl EmbedCtx {
    /// The level in use, if within reach.
    fn current(&mut self) -> Option<&mut EmbedLevel> {
        match self.level {
            0 => None,
            level if level <= EMBED_DEPTH => Some(&mut self.stack[level - 1]),
            // Deeper than the stack: the reference goes on using the
            // last level it had room for.
            _ => Some(&mut self.stack[EMBED_DEPTH - 1]),
        }
    }

    fn peek(&self) -> Option<&EmbedLevel> {
        match self.level {
            0 => None,
            level => Some(&self.stack[level.min(EMBED_DEPTH) - 1]),
        }
    }

    /// Enter an expression embedded in a string. Returns false if that
    /// is deeper than expressions can nest.
    pub(crate) fn start_expr(&mut self, quote: u8, triple: bool) -> bool {
        let fits = self.level < EMBED_DEPTH;
        if fits {
            self.stack[self.level] = EmbedLevel {
                quote,
                triple,
                parens: 0,
            };
        }
        self.level += 1;
        fits
    }

    pub(crate) fn end_expr(&mut self) {
        if self.level > 0 {
            self.level -= 1;
        }
    }

    pub(crate) fn in_expr(&self) -> bool {
        self.level != 0
    }

    pub(crate) fn parens(&self) -> i32 {
        self.peek().map_or(0, |level| level.parens)
    }

    pub(crate) fn add_parens(&mut self, change: i32) {
        if let Some(level) = self.current() {
            level.parens = (level.parens + change).max(0);
        }
    }

    pub(crate) fn quote(&self) -> u8 {
        self.peek().map_or(0, |level| level.quote)
    }

    pub(crate) fn triple(&self) -> bool {
        self.peek().is_some_and(|level| level.triple)
    }
}

/// Something the scanner noticed that the caller may want to report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum LexNote {
    /// A number that starts with zero, so is octal, has an 8 or a 9.
    DecimalInOctal(Vec<u8>),
    /// A symbol has a character that is not ASCII.
    NonAsciiSymbol(Vec<u8>),
    /// A symbol is longer than symbols can be.
    SymbolTruncated(Vec<u8>),
    /// Expressions embedded in strings are nested too deeply.
    EmbeddingTooDeep,
    /// A string in a preprocessor expression has no end.
    UntermString,
}

/// What the scanner needs besides the text.
pub(crate) struct LexCtx<'a> {
    /// Embedded expressions are followed, if this is given.
    pub ec: Option<&'a mut EmbedCtx>,
    /// The text is being scanned for macro expansion, not to be compiled,
    /// so problems with it are not reported yet.
    pub expanding: bool,
    /// The text is a preprocessor expression.
    pub in_pp_expr: bool,
    pub notes: &'a mut Vec<LexNote>,
}

/// How many of a quote character there are in a row.
pub(crate) fn count_quotes(buf: &[u8], pos: usize, quote: u8) -> usize {
    let mut count = 0;
    while at(buf, pos + count) == quote {
        count += 1;
    }
    count
}

/// Skip spaces and the markers that end macro expansions.
pub(crate) fn skip_ws_and_markers(buf: &[u8], pos: &mut usize) {
    loop {
        let c = at(buf, *pos);
        if c == MACRO_EXP_END {
            *pos += 1 + MACRO_ID_LEN;
        } else if is_space(c) {
            *pos += 1;
        } else {
            return;
        }
    }
}

/// Skip one character of a string, or a whole escape sequence.
pub(crate) fn skip_escape(buf: &[u8], pos: &mut usize) {
    if at(buf, *pos) != b'\\' {
        *pos += 1;
        return;
    }
    *pos += 1;
    match at(buf, *pos) {
        0 => {}
        // An octal character code: up to three digits, and no more than
        // fit in a byte.
        b'0'..=b'7' => {
            let mut acc: u32 = 0;
            for _ in 0..3 {
                let c = at(buf, *pos);
                if !is_odigit(c) || 8 * acc + u32::from(c - b'0') > 255 {
                    break;
                }
                acc = 8 * acc + u32::from(c - b'0');
                *pos += 1;
            }
        }
        // A hexadecimal character code: two digits after `x`, four after
        // `u`.
        code @ (b'x' | b'u') => {
            *pos += 1;
            let most = if code == b'x' { 2 } else { 4 };
            let mut count = 0;
            while count < most && is_xdigit(at(buf, *pos)) {
                *pos += 1;
                count += 1;
            }
        }
        _ => *pos += 1,
    }
}

/// Skip the format after `<<%`: flags, width, precision, and the letter
/// that says what kind of value it is.
pub(crate) fn scan_sprintf_spec(buf: &[u8], pos: &mut usize) {
    *pos += 1;
    loop {
        match at(buf, *pos) {
            b'[' => {
                *pos += 1;
                while is_digit(at(buf, *pos)) {
                    *pos += 1;
                }
                if at(buf, *pos) == b']' {
                    *pos += 1;
                }
            }
            b'_' => {
                *pos += 1;
                if at(buf, *pos) != 0 {
                    skip_escape(buf, pos);
                }
            }
            b'-' | b'+' | b' ' | b',' | b'#' => *pos += 1,
            _ => break,
        }
    }
    while is_digit(at(buf, *pos)) {
        *pos += 1;
    }
    if at(buf, *pos) == b'.' {
        *pos += 1;
        while is_digit(at(buf, *pos)) {
            *pos += 1;
        }
    }
    if at(buf, *pos) != 0 {
        skip_escape(buf, pos);
    }
}

/// Scan a string, or the part of one up to an embedded expression, or
/// the part after one.
fn tokenize_string(buf: &[u8], pos: &mut usize, cx: &mut LexCtx<'_>) -> Token {
    let start = *pos;
    let regex = at(buf, *pos) == b'R';
    if regex {
        *pos += 1;
    }
    let mut quote = at(buf, *pos);
    *pos += 1;
    let mut triple = false;
    if (quote == b'"' || quote == b'\'') && count_quotes(buf, *pos, quote) >= 2 {
        triple = true;
        *pos += 2;
    }

    let mut typ;
    let mut allow_embedding;
    match quote {
        b'\'' => {
            typ = Tok::SStr;
            allow_embedding = cx.ec.is_some();
        }
        b'"' => {
            typ = Tok::DStr;
            allow_embedding = cx.ec.is_some();
        }
        b'>' => {
            // The end of an embedded expression: the string it was in
            // carries on.
            match cx.ec.as_deref_mut() {
                Some(ec) => {
                    typ = if !ec.in_expr() {
                        Tok::Invalid
                    } else if ec.quote() == b'"' {
                        Tok::DStrEnd
                    } else {
                        Tok::SStrEnd
                    };
                    quote = ec.quote();
                    triple = ec.triple();
                    allow_embedding = true;
                    ec.end_expr();
                }
                None => {
                    typ = Tok::Invalid;
                    quote = b'"';
                    allow_embedding = false;
                }
            }
            *pos += 1;
        }
        _ => {
            typ = Tok::Invalid;
            quote = b'"';
            allow_embedding = false;
        }
    }
    if regex {
        allow_embedding = false;
        typ = Tok::ReStr;
    }

    loop {
        let cur = at(buf, *pos);
        if cur == b'\\' {
            *pos += 1;
            // In a triple-quoted string, a backslash escapes a whole run
            // of quotes.
            if triple && at(buf, *pos) == quote {
                while at(buf, *pos) == quote {
                    *pos += 1;
                }
                continue;
            }
        } else if cur == b'<' && allow_embedding && at(buf, *pos + 1) == b'<' {
            typ = match typ {
                Tok::DStr => Tok::DStrStart,
                Tok::DStrEnd => Tok::DStrMid,
                Tok::SStr => Tok::SStrStart,
                Tok::SStrEnd => Tok::SStrMid,
                other => other,
            };
            if let Some(ec) = cx.ec.as_deref_mut() {
                if !ec.start_expr(quote, triple) && !cx.expanding {
                    cx.notes.push(LexNote::EmbeddingTooDeep);
                }
            }
            *pos += 2;
            break;
        } else if cur == quote {
            if triple {
                let count = count_quotes(buf, *pos, quote);
                if count >= 3 {
                    *pos += count;
                } else {
                    *pos += count;
                    continue;
                }
            } else {
                *pos += 1;
            }
            break;
        } else if cur == 0 {
            if cx.in_pp_expr {
                cx.notes.push(LexNote::UntermString);
            }
            break;
        }
        *pos += 1;
    }

    Token {
        typ,
        pos: start,
        len: *pos - start,
        fully_expanded: false,
        int_val: 0,
    }
}

/// Get the next token on the line, starting at `pos`, and leave `pos`
/// after it.
pub(crate) fn next_on_line(buf: &[u8], pos: &mut usize, cx: &mut LexCtx<'_>) -> Token {
    skip_ws_and_markers(buf, pos);
    let mut start = *pos;
    let mut cur = at(buf, *pos);
    let mut tok = Token::new(Tok::Eof);
    tok.pos = start;
    if cur == 0 {
        return tok;
    }
    *pos += 1;

    let typ;
    let mut symbol = false;
    match cur {
        FORMAL_FLAG => {
            typ = Tok::MacroFormal;
            *pos += 1;
        }
        FOREACH_FLAG => typ = Tok::MacroForeach,
        IFEMPTY_FLAG => typ = Tok::MacroIfEmpty,
        IFNEMPTY_FLAG => typ = Tok::MacroIfNEmpty,
        ARGCOUNT_FLAG => typ = Tok::MacroArgCount,
        FULLY_EXPANDED_FLAG => {
            tok.fully_expanded = true;
            start = *pos;
            cur = at(buf, *pos);
            *pos += 1;
            symbol = true;
            typ = Tok::Sym;
        }
        END_PP_LINE => {
            // Stay on the marker, so that every later read is the end of
            // the line too.
            *pos = start;
            typ = Tok::Eof;
        }
        b'0'..=b'9' => {
            let (number_typ, value) = scan_number(buf, pos, start, cur, cx);
            typ = number_typ;
            tok.int_val = value;
        }
        b'"' | b'\'' => {
            *pos = start;
            return tokenize_string(buf, pos, cx);
        }
        b'(' => {
            typ = Tok::LPar;
            add_parens(cx, 1);
        }
        b')' => {
            typ = Tok::RPar;
            add_parens(cx, -1);
        }
        b',' => typ = Tok::Comma,
        b'.' => {
            if at(buf, *pos) == b'.' {
                *pos += 1;
                if at(buf, *pos) == b'.' {
                    *pos += 1;
                    typ = Tok::Ellipsis;
                } else {
                    typ = Tok::DotDot;
                }
            } else if is_digit(at(buf, *pos)) {
                typ = scan_float(buf, pos, start, Tok::Float);
            } else {
                typ = Tok::Dot;
            }
        }
        b'{' => {
            typ = Tok::LBrace;
            add_parens(cx, 1);
        }
        b'}' => {
            typ = Tok::RBrace;
            add_parens(cx, -1);
        }
        b'[' => {
            typ = Tok::LBrack;
            add_parens(cx, 1);
        }
        b']' => {
            typ = Tok::RBrack;
            add_parens(cx, -1);
        }
        b'=' => typ = two(buf, pos, &[(b'=', Tok::EqEq)], Tok::Eq),
        b':' => typ = two(buf, pos, &[(b':', Tok::ColonColon)], Tok::Colon),
        b'?' => typ = two(buf, pos, &[(b'?', Tok::QQ)], Tok::Question),
        b'+' => {
            typ = two(
                buf,
                pos,
                &[(b'+', Tok::Inc), (b'=', Tok::PlusEq)],
                Tok::Plus,
            )
        }
        b'-' => {
            typ = two(
                buf,
                pos,
                &[(b'-', Tok::Dec), (b'=', Tok::MinEq), (b'>', Tok::Arrow)],
                Tok::Minus,
            );
        }
        b'*' => typ = two(buf, pos, &[(b'=', Tok::TimesEq)], Tok::Times),
        b'/' => typ = two(buf, pos, &[(b'=', Tok::DivEq)], Tok::Div),
        b'%' => typ = two(buf, pos, &[(b'=', Tok::ModEq)], Tok::Mod),
        b'>' => {
            if at(buf, *pos) == b'=' {
                *pos += 1;
                typ = Tok::Ge;
            } else if at(buf, *pos) == b'>' {
                // The end of an embedded expression, if one is open and
                // its brackets are all closed.
                if cx
                    .ec
                    .as_deref()
                    .is_some_and(|ec| ec.in_expr() && ec.parens() == 0)
                {
                    *pos = start;
                    return tokenize_string(buf, pos, cx);
                }
                *pos += 1;
                if at(buf, *pos) == b'>' {
                    *pos += 1;
                    typ = two(buf, pos, &[(b'=', Tok::LShrEq)], Tok::LShr);
                } else {
                    typ = two(buf, pos, &[(b'=', Tok::AShrEq)], Tok::AShr);
                }
            } else {
                typ = Tok::Gt;
            }
        }
        b'<' => {
            if at(buf, *pos) == b'=' {
                *pos += 1;
                typ = Tok::Le;
            } else if at(buf, *pos) == b'<' {
                *pos += 1;
                typ = two(buf, pos, &[(b'=', Tok::ShlEq)], Tok::Shl);
            } else {
                typ = Tok::Lt;
            }
        }
        b';' => typ = Tok::Sem,
        b'&' => {
            typ = two(
                buf,
                pos,
                &[(b'&', Tok::AndAnd), (b'=', Tok::AndEq)],
                Tok::And,
            )
        }
        b'|' => typ = two(buf, pos, &[(b'|', Tok::OrOr), (b'=', Tok::OrEq)], Tok::Or),
        b'^' => typ = two(buf, pos, &[(b'=', Tok::XorEq)], Tok::Xor),
        b'!' => typ = two(buf, pos, &[(b'=', Tok::Ne)], Tok::Not),
        b'~' => typ = Tok::BNot,
        b'@' => typ = Tok::At,
        b'#' => {
            typ = two(
                buf,
                pos,
                &[(b'#', Tok::PoundPound), (b'@', Tok::PoundAt)],
                Tok::Pound,
            )
        }
        b'R' if at(buf, *pos) == b'"' || at(buf, *pos) == b'\'' => {
            *pos = start;
            return tokenize_string(buf, pos, cx);
        }
        _ => {
            if is_syminit(cur) || !cur.is_ascii() {
                symbol = true;
                typ = Tok::Sym;
            } else {
                typ = Tok::Invalid;
            }
        }
    }

    if symbol {
        let mut non_ascii = !cur.is_ascii();
        let mut too_long = false;
        loop {
            let ch = at(buf, *pos);
            if !ch.is_ascii() {
                non_ascii = true;
            } else if !is_sym(ch) {
                break;
            }
            *pos += 1;
            if *pos - start >= SYM_MAX_LEN {
                too_long = true;
            }
        }
        if non_ascii {
            cx.notes
                .push(LexNote::NonAsciiSymbol(buf[start..*pos].to_vec()));
        }
        if too_long && !cx.expanding {
            cx.notes
                .push(LexNote::SymbolTruncated(buf[start..*pos].to_vec()));
        }
    }

    tok.typ = typ;
    tok.pos = start;
    tok.len = *pos - start;
    tok
}

fn add_parens(cx: &mut LexCtx<'_>, change: i32) {
    if let Some(ec) = cx.ec.as_deref_mut() {
        ec.add_parens(change);
    }
}

/// A one-character operator, unless the next character makes it one of
/// the two-character ones given.
fn two(buf: &[u8], pos: &mut usize, longer: &[(u8, Tok)], single: Tok) -> Tok {
    let next = at(buf, *pos);
    for &(second, typ) in longer {
        if next == second {
            *pos += 1;
            return typ;
        }
    }
    single
}

/// Scan a number whose first digit, `first`, has been read.
fn scan_number(
    buf: &[u8],
    pos: &mut usize,
    start: usize,
    first: u8,
    cx: &mut LexCtx<'_>,
) -> (Tok, u32) {
    let mut acc: u32 = u32::from(first - b'0');
    let mut overflow = false;
    if first == b'0' {
        if at(buf, *pos) == b'x' || at(buf, *pos) == b'X' {
            *pos += 1;
            while is_xdigit(at(buf, *pos)) {
                if acc & 0xF000_0000 != 0 {
                    overflow = true;
                    break;
                }
                let digit = (at(buf, *pos) as char).to_digit(16).unwrap_or(0);
                acc = acc * 16 + digit;
                *pos += 1;
            }
        } else {
            while is_odigit(at(buf, *pos)) {
                if acc & 0xE000_0000 != 0 {
                    overflow = true;
                    break;
                }
                acc = 8 * acc + u32::from(at(buf, *pos) - b'0');
                *pos += 1;
            }
            if !overflow && is_digit(at(buf, *pos)) {
                *pos += 1;
                while is_digit(at(buf, *pos)) {
                    *pos += 1;
                }
                if !cx.expanding {
                    cx.notes
                        .push(LexNote::DecimalInOctal(buf[start..*pos].to_vec()));
                }
            }
        }
    } else {
        while is_digit(at(buf, *pos)) {
            if acc > 214_748_364 {
                overflow = true;
                break;
            }
            acc *= 10;
            let digit = u32::from(at(buf, *pos) - b'0');
            if acc > 2_147_483_647 - digit {
                overflow = true;
                break;
            }
            acc += digit;
            *pos += 1;
        }
    }
    if overflow {
        return (scan_float(buf, pos, start, Tok::BigInt), 0);
    }
    // A decimal point or an exponent makes it a floating-point number,
    // but two periods are a range, not a decimal point.
    let next = at(buf, *pos);
    if (next == b'.' && at(buf, *pos + 1) != b'.') || next == b'e' || next == b'E' {
        return (scan_float(buf, pos, start, Tok::Float), 0);
    }
    (Tok::Int, acc)
}

/// Scan a floating-point number from its start. `typ` says whether it
/// began as an integer that turned out too big.
fn scan_float(buf: &[u8], pos: &mut usize, start: usize, typ: Tok) -> Tok {
    let mut typ = typ;
    let mut found_point = false;
    let mut hex = false;
    *pos = start;
    if at(buf, *pos) == b'-' {
        *pos += 1;
    }
    if typ == Tok::BigInt && at(buf, *pos) == b'0' && matches!(at(buf, *pos + 1), b'x' | b'X') {
        hex = true;
        *pos += 2;
    }
    loop {
        let cur = at(buf, *pos);
        if is_digit(cur) || (hex && is_xdigit(cur)) {
            // Another digit.
        } else if !found_point && !hex && cur == b'.' {
            found_point = true;
            if typ == Tok::BigInt {
                typ = Tok::Float;
            }
        } else if cur == b'e' || cur == b'E' {
            // An exponent, if digits follow.
            let mut ahead = *pos + 1;
            if matches!(at(buf, ahead), b'-' | b'+') {
                ahead += 1;
            }
            if !is_digit(at(buf, ahead)) {
                break;
            }
            while is_digit(at(buf, ahead)) {
                ahead += 1;
            }
            *pos = ahead;
            if typ == Tok::BigInt {
                typ = Tok::Float;
            }
            break;
        } else {
            break;
        }
        *pos += 1;
    }
    typ
}
