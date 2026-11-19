//! Constant expressions in `#if`, `#elif`, and `#line`.
//!
//! The reference hands these to the compiler's own expression parser and
//! takes the constant that comes out. Until Hexicon's parser exists, this
//! evaluates the expressions that preprocessor conditions are made of:
//! integers, strings, `true` and `nil`, and the operators on them. By the
//! time it sees an expression, macros are expanded and `defined(X)` has
//! become 1 or 0.

use crate::lex::{self, EmbedCtx, LexCtx, Tok, Token};

/// The value of a preprocessor expression.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PpValue {
    Nil,
    True,
    Int(i32),
    Str(Vec<u8>),
}

impl PpValue {
    /// Whether the value counts as true in an `#if`: anything but `nil`
    /// and zero.
    pub(crate) fn truth(&self) -> bool {
        !matches!(self, PpValue::Nil | PpValue::Int(0))
    }

    fn of_bool(value: bool) -> Self {
        if value { PpValue::True } else { PpValue::Nil }
    }
}

const NOT_CONST: &str = "preprocessor expression is not a constant value";

struct Parser<'a> {
    text: &'a [u8],
    pos: usize,
    tok: Token,
    ec: EmbedCtx,
    notes: Vec<lex::LexNote>,
}

impl Parser<'_> {
    fn advance(&mut self) {
        let mut cx = LexCtx {
            ec: Some(&mut self.ec),
            expanding: false,
            in_pp_expr: true,
            notes: &mut self.notes,
        };
        self.tok = lex::next_on_line(self.text, &mut self.pos, &mut cx);
    }

    fn accept(&mut self, typ: Tok) -> bool {
        if self.tok.typ == typ {
            self.advance();
            true
        } else {
            false
        }
    }

    fn conditional(&mut self) -> Result<PpValue, String> {
        let condition = self.binary(0)?;
        if !self.accept(Tok::Question) {
            return Ok(condition);
        }
        let yes = self.conditional()?;
        if !self.accept(Tok::Colon) {
            return Err("missing colon in conditional expression".to_owned());
        }
        let no = self.conditional()?;
        Ok(if condition.truth() { yes } else { no })
    }

    /// Binary operators, loosest binding first.
    fn binary(&mut self, level: usize) -> Result<PpValue, String> {
        const LEVELS: [&[Tok]; 10] = [
            &[Tok::OrOr],
            &[Tok::AndAnd],
            &[Tok::Or],
            &[Tok::Xor],
            &[Tok::And],
            &[Tok::EqEq, Tok::Ne],
            &[Tok::Lt, Tok::Le, Tok::Gt, Tok::Ge],
            &[Tok::Shl, Tok::AShr, Tok::LShr],
            &[Tok::Plus, Tok::Minus],
            &[Tok::Times, Tok::Div, Tok::Mod],
        ];
        if level == LEVELS.len() {
            return self.unary();
        }
        let mut left = self.binary(level + 1)?;
        while LEVELS[level].contains(&self.tok.typ) {
            let op = self.tok.typ;
            self.advance();
            let right = self.binary(level + 1)?;
            left = apply(op, left, right)?;
        }
        Ok(left)
    }

    fn unary(&mut self) -> Result<PpValue, String> {
        let op = self.tok.typ;
        match op {
            Tok::Not => {
                self.advance();
                Ok(PpValue::of_bool(!self.unary()?.truth()))
            }
            Tok::Minus | Tok::Plus | Tok::BNot => {
                self.advance();
                match self.unary()? {
                    PpValue::Int(value) => Ok(PpValue::Int(match op {
                        Tok::Minus => value.wrapping_neg(),
                        Tok::BNot => !value,
                        _ => value,
                    })),
                    _ => Err("invalid type for a numeric operator".to_owned()),
                }
            }
            _ => self.primary(),
        }
    }

    fn primary(&mut self) -> Result<PpValue, String> {
        let tok = self.tok;
        match tok.typ {
            Tok::Int => {
                self.advance();
                Ok(PpValue::Int(tok.int_val as i32))
            }
            Tok::SStr | Tok::DStr => {
                self.advance();
                let text = tok.text(self.text);
                let inner = if text.len() >= 2 {
                    &text[1..text.len() - 1]
                } else {
                    &text[..0]
                };
                Ok(PpValue::Str(translate_escapes(inner)))
            }
            Tok::LPar => {
                self.advance();
                let value = self.conditional()?;
                if !self.accept(Tok::RPar) {
                    return Err("missing right parenthesis in expression".to_owned());
                }
                Ok(value)
            }
            Tok::Sym => {
                let name = tok.text(self.text).to_vec();
                self.advance();
                match name.as_slice() {
                    b"true" => Ok(PpValue::True),
                    b"nil" => Ok(PpValue::Nil),
                    // Any other name is something only a running program
                    // has a value for.
                    _ => Err(NOT_CONST.to_owned()),
                }
            }
            Tok::Eof => Err("expression expected in preprocessor directive".to_owned()),
            _ => Err(NOT_CONST.to_owned()),
        }
    }
}

fn apply(op: Tok, left: PpValue, right: PpValue) -> Result<PpValue, String> {
    use PpValue::{Int, Str};
    Ok(match (op, &left, &right) {
        (Tok::OrOr, ..) => PpValue::of_bool(left.truth() || right.truth()),
        (Tok::AndAnd, ..) => PpValue::of_bool(left.truth() && right.truth()),
        (Tok::EqEq, ..) => PpValue::of_bool(left == right),
        (Tok::Ne, ..) => PpValue::of_bool(left != right),
        (Tok::Lt, Int(a), Int(b)) => PpValue::of_bool(a < b),
        (Tok::Le, Int(a), Int(b)) => PpValue::of_bool(a <= b),
        (Tok::Gt, Int(a), Int(b)) => PpValue::of_bool(a > b),
        (Tok::Ge, Int(a), Int(b)) => PpValue::of_bool(a >= b),
        (Tok::Lt, Str(a), Str(b)) => PpValue::of_bool(a < b),
        (Tok::Le, Str(a), Str(b)) => PpValue::of_bool(a <= b),
        (Tok::Gt, Str(a), Str(b)) => PpValue::of_bool(a > b),
        (Tok::Ge, Str(a), Str(b)) => PpValue::of_bool(a >= b),
        (Tok::Or, Int(a), Int(b)) => Int(a | b),
        (Tok::Xor, Int(a), Int(b)) => Int(a ^ b),
        (Tok::And, Int(a), Int(b)) => Int(a & b),
        (Tok::Shl, Int(a), Int(b)) => Int(a.wrapping_shl(*b as u32)),
        (Tok::AShr, Int(a), Int(b)) => Int(a.wrapping_shr(*b as u32)),
        (Tok::LShr, Int(a), Int(b)) => Int(((*a as u32).wrapping_shr(*b as u32)) as i32),
        (Tok::Plus, Int(a), Int(b)) => Int(a.wrapping_add(*b)),
        (Tok::Minus, Int(a), Int(b)) => Int(a.wrapping_sub(*b)),
        (Tok::Times, Int(a), Int(b)) => Int(a.wrapping_mul(*b)),
        (Tok::Div | Tok::Mod, Int(_), Int(0)) => {
            return Err("division by zero in constant expression".to_owned());
        }
        (Tok::Div, Int(a), Int(b)) => Int(a.wrapping_div(*b)),
        (Tok::Mod, Int(a), Int(b)) => Int(a.wrapping_rem(*b)),
        (Tok::Plus, Str(a), Str(b)) => Str([a.as_slice(), b.as_slice()].concat()),
        _ => return Err("invalid types for operator in preprocessor expression".to_owned()),
    })
}

fn parser(text: &[u8]) -> Parser<'_> {
    let mut parser = Parser {
        text,
        pos: 0,
        tok: Token::new(Tok::Eof),
        ec: EmbedCtx::default(),
        notes: Vec::new(),
    };
    parser.advance();
    parser
}

/// The value of the expression that is the whole of `text`.
pub(crate) fn evaluate(text: &[u8]) -> Result<PpValue, String> {
    let mut parser = parser(text);
    let value = parser.conditional()?;
    if parser.tok.typ != Tok::Eof {
        return Err("extra characters at end of line".to_owned());
    }
    Ok(value)
}

/// The line number and file name of a `#line` directive.
pub(crate) fn line_directive(text: &[u8]) -> Result<(u32, String), String> {
    let mut parser = parser(text);
    let PpValue::Int(line) = parser.conditional()? else {
        return Err("integer value required for #line".to_owned());
    };
    let PpValue::Str(name) = parser.conditional()? else {
        return Err("string value required for #line".to_owned());
    };
    if parser.tok.typ != Tok::Eof {
        return Err("extra characters at end of line".to_owned());
    }
    Ok((line as u32, String::from_utf8_lossy(&name).into_owned()))
}

/// A string's text with its escape sequences replaced by the characters
/// they stand for, as UTF-8.
fn translate_escapes(text: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(text.len());
    let push_char = |out: &mut Vec<u8>, code: u32| {
        let c = char::from_u32(code).unwrap_or('?');
        out.extend_from_slice(c.encode_utf8(&mut [0; 4]).as_bytes());
    };
    let mut p = 0;
    while p < text.len() {
        if text[p] != b'\\' {
            out.push(text[p]);
            p += 1;
            continue;
        }
        p += 1;
        let cur = lex::at(text, p);
        // A number in a given base, of up to so many digits.
        let number = |p: &mut usize, base: u32, most: usize, limit: u32| {
            let mut acc = 0u32;
            for _ in 0..most {
                let Some(digit) = char::from(lex::at(text, *p)).to_digit(base) else {
                    break;
                };
                if acc * base + digit > limit {
                    break;
                }
                acc = acc * base + digit;
                *p += 1;
            }
            acc
        };
        match cur {
            b'u' => {
                p += 1;
                let code = number(&mut p, 16, 4, u32::MAX);
                push_char(&mut out, code);
                continue;
            }
            b'x' => {
                p += 1;
                let code = number(&mut p, 16, 2, u32::MAX);
                push_char(&mut out, code);
                continue;
            }
            b'0'..=b'7' => {
                let code = number(&mut p, 8, 3, 255);
                push_char(&mut out, code);
                continue;
            }
            b'^' => out.push(0x0f),
            b'v' => out.push(0x0e),
            b'b' => out.push(0x0b),
            b' ' => out.push(0x15),
            b'n' => out.push(b'\n'),
            b'r' => out.push(b'\r'),
            b't' => out.push(b'\t'),
            b'"' | b'\'' | b'<' | b'>' | b'\\' => out.push(cur),
            0 => break,
            other => {
                out.push(b'\\');
                out.push(other);
            }
        }
        p += 1;
    }
    out
}
