//! The `sprintf` function: text built from a format string and values.

use crate::bignum::{self, Big};
use crate::error::{VmError, VmResult, code};
use crate::value::Value;
use crate::vm::Vm;

/// The options of one format item.
struct Options {
    /// What to show in front of a number that is not negative: `+`, a
    /// space, or nothing.
    sign: Option<char>,
    /// The character to put between groups of three digits.
    group: Option<char>,
    width: i32,
    precision: i32,
    left_align: bool,
    pad: char,
    /// The `#` flag: show a radix prefix, or always show a decimal point.
    pound: bool,
}

/// The format string, read a byte or a character at a time.
struct Reader<'a> {
    text: &'a [u8],
    at: usize,
}

impl Reader<'_> {
    fn more(&self) -> bool {
        self.at < self.text.len()
    }

    fn peek(&self) -> u8 {
        self.text.get(self.at).copied().unwrap_or(0)
    }

    fn next(&mut self) -> u8 {
        let byte = self.peek();
        if self.more() {
            self.at += 1;
        }
        byte
    }

    fn skip_if(&mut self, byte: u8) -> bool {
        let found = self.more() && self.peek() == byte;
        if found {
            self.at += 1;
        }
        found
    }

    /// Read a whole character.
    fn next_char(&mut self) -> char {
        let rest = &self.text[self.at..];
        let len = match rest.first() {
            None => return '\0',
            Some(&b) if b < 0x80 => 1,
            Some(&b) if b < 0xe0 => 2,
            Some(&b) if b < 0xf0 => 3,
            Some(_) => 4,
        }
        .min(rest.len());
        self.at += len;
        String::from_utf8_lossy(&rest[..len])
            .chars()
            .next()
            .unwrap_or('\0')
    }

    fn number(&mut self) -> i32 {
        let mut acc: i32 = 0;
        while self.peek().is_ascii_digit() {
            acc = acc
                .wrapping_mul(10)
                .wrapping_add(i32::from(self.next() - b'0'));
        }
        acc
    }
}

fn pad(out: &mut String, with: char, count: i32) {
    out.extend(std::iter::repeat_n(with, count.max(0) as usize));
}

/// Write text in a field of the width the options ask for.
fn padded(out: &mut String, text: &str, options: &Options) {
    let chars = text.chars().count() as i32;
    let fill = (options.width - chars).max(0);
    if !options.left_align {
        pad(out, options.pad, fill);
    }
    out.push_str(text);
    if options.left_align {
        pad(out, options.pad, fill);
    }
}

/// Write the digits of an integer, with sign, prefix, grouping, and
/// padding.
fn integer(out: &mut String, digits: &str, prefix: Option<&str>, options: &Options, caps: bool) {
    let text = if digits.is_empty() { "0" } else { digits };
    let negative = text.starts_with('-');
    let zero = text.starts_with('0');
    let len = text.len() as i32;
    let digit_count = len - i32::from(negative);
    let prefix = prefix.filter(|_| options.pound && !zero);

    let mut shown = len;
    if options.sign.is_some() && !negative {
        shown += 1;
    }
    if options.group.is_some() {
        shown += (digit_count - 1) / 3;
    }
    if let Some(prefix) = prefix {
        shown += prefix.len() as i32;
    }
    if digit_count < options.precision {
        shown += options.precision - digit_count;
    }
    let fill = (options.width - shown).max(0);

    if !options.left_align {
        pad(out, options.pad, fill);
    }
    match options.sign {
        Some(sign) if !negative => out.push(sign),
        _ => {}
    }
    if negative {
        out.push('-');
    }
    if let Some(prefix) = prefix {
        out.push_str(prefix);
    }
    pad(out, '0', options.precision - digit_count);
    let body = &text[usize::from(negative)..];
    for (index, c) in body.chars().enumerate() {
        let left = body.len() - index;
        if let Some(group) = options.group
            && index != 0
            && left % 3 == 0
        {
            out.push(group);
        }
        out.push(if caps {
            c.to_ascii_uppercase()
        } else {
            c.to_ascii_lowercase()
        });
    }
    if options.left_align {
        pad(out, options.pad, fill);
    }
}

// The reference VM has no entry for forty, so 40 is "xxxx". Programs can
// see that, so it stays.
const ROMAN: [(&str, i32); 12] = [
    ("m", 1000),
    ("cm", 900),
    ("d", 500),
    ("cd", 400),
    ("c", 100),
    ("xc", 90),
    ("l", 50),
    ("x", 10),
    ("ix", 9),
    ("v", 5),
    ("iv", 4),
    ("i", 1),
];

impl Vm {
    /// `sprintf(format, ...)`.
    pub(crate) fn sprintf(&mut self, argc: u32) -> VmResult<Value> {
        if argc < 1 {
            return Err(VmError::new(code::WRONG_NUM_OF_ARGS));
        }
        let format = self.require_str(self.arg(0))?.to_vec();
        let value_count = argc - 1;
        let mut reader = Reader {
            text: &format,
            at: 0,
        };
        let mut out: Vec<u8> = Vec::with_capacity(format.len() * 3 / 2);
        let mut next_arg: u32 = 1;
        while reader.more() {
            let byte = reader.next();
            if byte != b'%' {
                out.push(byte);
                continue;
            }
            let start = reader.at - 1;
            let mut options = Options {
                sign: None,
                group: None,
                width: -1,
                precision: -1,
                left_align: false,
                pad: ' ',
                pound: false,
            };
            let mut explicit_arg: i32 = -1;
            loop {
                match reader.peek() {
                    b'[' if reader.more() => {
                        reader.at += 1;
                        explicit_arg = reader.number();
                        reader.skip_if(b']');
                    }
                    b'+' => {
                        options.sign = Some('+');
                        reader.at += 1;
                    }
                    b' ' => {
                        options.sign = Some(' ');
                        reader.at += 1;
                    }
                    b',' => {
                        options.group = Some(',');
                        reader.at += 1;
                    }
                    b'-' => {
                        options.left_align = true;
                        reader.at += 1;
                    }
                    b'_' => {
                        reader.at += 1;
                        options.pad = reader.next_char();
                    }
                    b'#' => {
                        options.pound = true;
                        reader.at += 1;
                    }
                    _ => break,
                }
            }
            if reader.skip_if(b'0') && !options.left_align {
                options.pad = '0';
            }
            if reader.peek().is_ascii_digit() {
                options.width = reader.number();
            }
            if reader.skip_if(b'.') {
                options.precision = reader.number();
            }
            let spec = reader.next();
            let arg_index = if explicit_arg > 0 {
                explicit_arg as u32
            } else {
                next_arg
            };
            let value = if arg_index <= value_count {
                self.arg(arg_index as usize)
            } else {
                Value::Nil
            };
            let mut text = String::new();
            let mut used_arg = true;
            match spec {
                b'%' => {
                    text.push('%');
                    used_arg = false;
                }
                b'b' => self.format_int(&mut text, value, 2, None, &options, true, false)?,
                b'c' => match self.as_str(value) {
                    Some(string) => text.push(
                        String::from_utf8_lossy(&string)
                            .chars()
                            .next()
                            .unwrap_or('\0'),
                    ),
                    None => {
                        let code_point = self.cast_to_int(value)?;
                        if !(0..=65535).contains(&code_point) {
                            return Err(VmError::new(code::NUM_OVERFLOW));
                        }
                        text.push(char::from_u32(code_point as u32).unwrap_or('\u{fffd}'));
                    }
                },
                b'd' => self.format_int(&mut text, value, 10, None, &options, false, false)?,
                b'r' | b'R' => {
                    let caps = spec == b'R';
                    let mut n = self.cast_to_int(value)?;
                    if (1..=4999).contains(&n) {
                        for (numeral, worth) in ROMAN {
                            while worth <= n {
                                if caps {
                                    text.push_str(&numeral.to_ascii_uppercase());
                                } else {
                                    text.push_str(numeral);
                                }
                                n -= worth;
                            }
                        }
                    } else {
                        integer(&mut text, &n.to_string(), None, &options, false);
                    }
                }
                b'u' => self.format_int(&mut text, value, 10, None, &options, true, false)?,
                b'e' | b'E' | b'f' | b'g' | b'G' => {
                    self.format_float(&mut text, value, spec, &options)?;
                }
                b'o' => self.format_int(&mut text, value, 8, Some("0"), &options, true, false)?,
                b's' => {
                    let string = self.cast_to_text(value)?;
                    let string = String::from_utf8_lossy(&string);
                    let shown: String = if options.precision >= 0 {
                        string.chars().take(options.precision as usize).collect()
                    } else {
                        string.into_owned()
                    };
                    padded(&mut text, &shown, &options);
                }
                b'x' => self.format_int(&mut text, value, 16, Some("0x"), &options, true, false)?,
                b'X' => self.format_int(&mut text, value, 16, Some("0X"), &options, true, true)?,
                _ => {
                    // Not a format item after all: keep the text as it is.
                    out.extend_from_slice(&format[start..reader.at]);
                    used_arg = false;
                }
            }
            out.extend_from_slice(text.as_bytes());
            if used_arg && explicit_arg < 0 {
                next_arg += 1;
            }
        }
        self.try_new_string(out)
    }

    #[allow(clippy::too_many_arguments)]
    fn format_int(
        &mut self,
        out: &mut String,
        value: Value,
        radix: u32,
        prefix: Option<&str>,
        options: &Options,
        unsigned: bool,
        caps: bool,
    ) -> VmResult<()> {
        let digits = match self.cast_to_num(value)? {
            Value::Int(n) => crate::bif::int_to_radix(n, radix, !unsigned),
            // A BigNumber is rounded to a whole number.
            number => match self.as_big(number) {
                Some(big) => big.to_radix_text(radix),
                None => return Err(VmError::new(code::NO_NUM_CONV)),
            },
        };
        integer(out, &digits, prefix, options, caps);
        Ok(())
    }

    fn format_float(
        &mut self,
        out: &mut String,
        value: Value,
        spec: u8,
        options: &Options,
    ) -> VmResult<()> {
        let mut frac_digits = if options.precision >= 0 {
            options.precision
        } else {
            6
        };
        let mut max_digits = -1;
        let mut flags = bignum::FORMAT_LEADING_ZERO;
        match options.sign {
            Some('+') => flags |= bignum::FORMAT_SIGN,
            Some(' ') => flags |= bignum::FORMAT_POS_SPACE,
            _ => {}
        }
        let general = spec == b'g' || spec == b'G';
        if options.pound {
            flags |= bignum::FORMAT_POINT;
            if general {
                flags |= bignum::FORMAT_TRAILING_ZEROS;
            }
        }
        if spec == b'E' || spec == b'G' {
            flags |= bignum::FORMAT_EXP_CAP;
        }
        if spec == b'e' || spec == b'E' {
            flags |= bignum::FORMAT_EXP | bignum::FORMAT_EXP_SIGN;
        }
        if general {
            // The precision counts significant digits, and the shorter
            // of the plain and scientific forms is used.
            flags |= bignum::FORMAT_COMPACT | bignum::FORMAT_EXP_SIGN | bignum::FORMAT_MAXSIG;
            max_digits = frac_digits;
            frac_digits = -1;
        }
        let number = match self.cast_to_num(value)? {
            Value::Int(n) => {
                // Zeros after the point are not significant in an
                // integer, so its precision is its number of digits.
                let big = Big::from_int(n.into(), 20);
                let digits = big.exp.max(1) as usize;
                big.with_prec(digits, false)
            }
            number => match self.as_big(number) {
                Some(big) => big.clone(),
                None => return Ok(()),
            },
        };
        let text = number.format(max_digits, -1, frac_digits, 3, flags, None);
        padded(out, &text, options);
        Ok(())
    }
}
