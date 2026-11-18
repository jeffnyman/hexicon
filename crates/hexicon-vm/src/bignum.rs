//! Decimal floating-point arithmetic for the BigNumber intrinsic class.
//!
//! A number is a sign, a string of decimal digits, and a decimal exponent:
//! its value is `0.d1d2d3... × 10^exp`. The number of digits, the
//! precision, is fixed when the number is created, and the result of an
//! operation has the larger of its operands' precisions.
//!
//! Programs see every digit of every result, so the arithmetic here is a
//! port of the reference VM's (`vmbignumlib.cpp`), operation by operation:
//! the same alignment, the same handling of digits that do not fit, and
//! the same rounding, which is to the nearest value with ties going to
//! the even digit.

use std::cmp::Ordering;

/// What kind of value a number holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// An ordinary number.
    Num,
    /// Not a number.
    NaN,
    /// Infinity, positive or negative.
    Inf,
}

/// A decimal floating-point number.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Big {
    /// The digits, most significant first. The length is the precision.
    pub digs: Vec<u8>,
    /// The decimal exponent: the value is `0.digs × 10^exp`.
    pub exp: i32,
    /// Whether the number is negative.
    pub neg: bool,
    /// Whether this is a number, an infinity, or not a number.
    pub kind: Kind,
    /// Whether the number is zero. A zero can be negative.
    pub zero: bool,
}

// Formatting flags, for `formatString`.
pub const FORMAT_SIGN: u32 = 0x0001;
pub const FORMAT_EXP: u32 = 0x0002;
pub const FORMAT_EXP_SIGN: u32 = 0x0004;
pub const FORMAT_LEADING_ZERO: u32 = 0x0008;
pub const FORMAT_POINT: u32 = 0x0010;
/// The reference VM gives these two the same value, so asking for one
/// asks for both.
pub const FORMAT_EXP_CAP: u32 = 0x0020;
pub const FORMAT_COMMAS: u32 = 0x0020;
pub const FORMAT_POS_SPACE: u32 = 0x0040;
pub const FORMAT_EUROSTYLE: u32 = 0x0080;
pub const FORMAT_COMPACT: u32 = 0x0100;
pub const FORMAT_MAXSIG: u32 = 0x0200;
pub const FORMAT_TRAILING_ZEROS: u32 = 0x0400;

impl Big {
    /// A zero with a given precision.
    pub fn zero(prec: usize) -> Self {
        Self {
            digs: vec![0; prec],
            exp: 1,
            neg: false,
            kind: Kind::Num,
            zero: true,
        }
    }

    /// The precision: the number of digits stored.
    pub fn prec(&self) -> usize {
        self.digs.len()
    }

    /// Digit `i`, or zero past the end.
    fn dig(&self, i: i32) -> i32 {
        usize::try_from(i)
            .ok()
            .and_then(|i| self.digs.get(i))
            .map_or(0, |&d| i32::from(d))
    }

    fn set_dig(&mut self, i: i32, d: i32) {
        if let Some(slot) = usize::try_from(i).ok().and_then(|i| self.digs.get_mut(i)) {
            *slot = d as u8;
        }
    }

    fn set_zero(&mut self) {
        self.exp = 1;
        self.zero = true;
        self.neg = false;
        self.kind = Kind::Num;
        self.digs.fill(0);
    }

    /// Whether this is anything but an ordinary number.
    pub fn is_nan(&self) -> bool {
        self.kind != Kind::Num
    }

    /// An integer, to a given precision.
    pub fn from_int(value: i64, prec: usize) -> Self {
        let mut n = Self::zero(prec);
        n.set_uint(value.unsigned_abs());
        if value < 0 {
            n.neg = true;
        }
        n
    }

    fn set_uint(&mut self, mut value: u64) {
        self.neg = false;
        self.kind = Kind::Num;
        self.zero = false;
        self.digs.fill(0);
        let mut exp = 0;
        while value != 0 {
            self.shift_right(1);
            self.set_dig(0, (value % 10) as i32);
            value /= 10;
            exp += 1;
        }
        self.exp = exp;
        self.normalize();
    }

    /// The number as stored in an image or a saved state: precision,
    /// exponent, flags, and the digits packed two to a byte.
    pub fn from_bytes(data: &[u8]) -> Option<Self> {
        let prec = usize::from(u16::from_le_bytes([*data.first()?, *data.get(1)?]));
        let exp = i32::from(i16::from_le_bytes([*data.get(2)?, *data.get(3)?]));
        let flags = *data.get(4)?;
        let mut digs = Vec::with_capacity(prec);
        for i in 0..prec {
            let pair = *data.get(5 + i / 2)?;
            digs.push(if i % 2 == 0 { pair >> 4 } else { pair & 0x0f });
        }
        Some(Self {
            digs,
            exp,
            neg: flags & 0x01 != 0,
            kind: match flags & 0x06 {
                0x00 => Kind::Num,
                0x04 => Kind::Inf,
                _ => Kind::NaN,
            },
            zero: flags & 0x08 != 0,
        })
    }

    // ---- shifting and rounding ----

    fn shift_left(&mut self, shift: usize) {
        let prec = self.digs.len();
        if shift == 0 {
            return;
        }
        if shift >= prec {
            self.digs.fill(0);
        } else {
            self.digs.copy_within(shift.., 0);
            self.digs[prec - shift..].fill(0);
        }
    }

    fn shift_right(&mut self, shift: usize) {
        let prec = self.digs.len();
        if shift >= prec {
            self.digs.fill(0);
        } else {
            self.digs.copy_within(..prec - shift, shift);
            self.digs[..shift].fill(0);
        }
    }

    /// Put the first non-zero digit first, adjusting the exponent.
    pub fn normalize(&mut self) {
        if self.is_nan() {
            return;
        }
        match self.digs.iter().position(|&d| d != 0) {
            Some(first) if self.exp - first as i32 >= -32768 => {
                self.zero = false;
                self.shift_left(first);
                self.exp -= first as i32;
            }
            _ => self.set_zero(),
        }
    }

    /// Whether any digit from `from` on is non-zero.
    fn any_digits_from(&self, from: i32) -> bool {
        let from = from.max(0) as usize;
        self.digs
            .get(from..)
            .is_some_and(|rest| rest.iter().any(|&d| d != 0))
    }

    /// Whether to round up when keeping only the first `digits` digits:
    /// to nearest, with a tie going to the even digit.
    pub fn round_dir(&self, digits: i32) -> bool {
        let prec = self.digs.len() as i32;
        if digits >= prec {
            return false;
        }
        let digits = digits.max(0);
        match self.dig(digits).cmp(&5) {
            Ordering::Greater => true,
            Ordering::Less => false,
            Ordering::Equal => {
                self.any_digits_from(digits + 1) || (digits > 0 && self.dig(digits - 1) & 1 != 0)
            }
        }
    }

    /// Add one to the last of the first `keep` digits.
    fn round_up_abs(&mut self, keep: i32) {
        let mut carry = true;
        let mut idx = keep.min(self.digs.len() as i32);
        while idx != 0 {
            idx -= 1;
            let d = self.dig(idx);
            if d < 9 {
                self.set_dig(idx, d + 1);
                carry = false;
                break;
            }
            self.set_dig(idx, 0);
        }
        if carry {
            self.shift_right(1);
            self.exp += 1;
            self.set_dig(0, 1);
        }
        self.zero = false;
    }

    /// Round for digits that an operation could not keep: the first of
    /// them, and whether any after it was non-zero.
    fn round_for_dropped(&mut self, trail_dig: i32, trail_val: i32) {
        let last = self.dig(self.digs.len() as i32 - 1);
        if trail_dig > 5 || (trail_dig == 5 && (trail_val > 0 || last & 1 != 0)) {
            self.round_up_abs(self.digs.len() as i32);
        }
    }

    /// Keep only the first `digits` digits, rounding.
    pub fn round_to(&mut self, digits: i32) {
        let prec = self.digs.len() as i32;
        if digits < prec {
            let up = self.round_dir(digits);
            for i in digits.max(0)..prec {
                self.set_dig(i, 0);
            }
            if up {
                self.round_up_abs(digits);
            }
        }
    }

    /// Copy a value into this number's precision, optionally rounding if
    /// it has more digits than fit.
    pub fn copy_from(&mut self, src: &Big, round: bool) {
        let prec = self.digs.len();
        self.exp = src.exp;
        self.neg = src.neg;
        self.kind = src.kind;
        self.zero = src.zero;
        if prec > src.digs.len() {
            self.digs[..src.digs.len()].copy_from_slice(&src.digs);
            self.digs[src.digs.len()..].fill(0);
        } else {
            self.digs.copy_from_slice(&src.digs[..prec]);
            if round && src.round_dir(prec as i32) {
                self.round_up_abs(prec as i32);
            }
        }
    }

    /// A copy of this value at another precision.
    pub fn with_prec(&self, prec: usize, round: bool) -> Self {
        let mut n = Self::zero(prec);
        n.copy_from(self, round);
        n
    }

    // ---- comparison ----

    /// Compare magnitudes.
    pub fn compare_abs(&self, other: &Big) -> Ordering {
        match (self.zero, other.zero) {
            (true, true) => return Ordering::Equal,
            (true, false) => return Ordering::Less,
            (false, true) => return Ordering::Greater,
            _ => {}
        }
        if self.exp != other.exp {
            return self.exp.cmp(&other.exp);
        }
        let common = self.digs.len().min(other.digs.len());
        match self.digs[..common].cmp(&other.digs[..common]) {
            Ordering::Equal => {}
            unequal => return unequal,
        }
        // Equal as far as both go: the longer one is larger if it has
        // anything further.
        let (longer, when_more) = if self.digs.len() > other.digs.len() {
            (self, Ordering::Greater)
        } else {
            (other, Ordering::Less)
        };
        if longer.digs[common..].iter().any(|&d| d != 0) {
            when_more
        } else {
            Ordering::Equal
        }
    }

    /// Compare values. `None` if either is not a number.
    pub fn compare(&self, other: &Big) -> Option<Ordering> {
        if self.is_nan() || other.is_nan() {
            return None;
        }
        if self.neg != other.neg {
            return Some(if self.neg {
                Ordering::Less
            } else {
                Ordering::Greater
            });
        }
        let order = self.compare_abs(other);
        Some(if self.neg { order.reverse() } else { order })
    }

    /// Whether two values are exactly equal, whatever their precisions.
    pub fn equals_exact(&self, other: &Big) -> bool {
        if self.is_nan() || other.is_nan() || self.neg != other.neg || self.exp != other.exp {
            return false;
        }
        let common = self.digs.len().min(other.digs.len());
        self.digs[..common] == other.digs[..common]
            && self.digs[common..].iter().all(|&d| d == 0)
            && other.digs[common..].iter().all(|&d| d == 0)
    }

    /// Whether two values are equal once the more precise one is rounded
    /// to the precision of the other.
    pub fn equals_rounded(&self, other: &Big) -> bool {
        match self.digs.len().cmp(&other.digs.len()) {
            Ordering::Equal => self.equals_exact(other),
            Ordering::Greater => other.equals_exact(&self.with_prec(other.digs.len(), true)),
            Ordering::Less => self.equals_exact(&other.with_prec(self.digs.len(), true)),
        }
    }

    // ---- arithmetic ----

    /// The result an operation on two values starts from: a zero of the
    /// larger precision, or, if either operand is not an ordinary number,
    /// the answer.
    fn start(a: &Big, b: &Big) -> Result<Big, Big> {
        let prec = if a.is_nan() || b.is_nan() {
            1
        } else {
            a.digs.len().max(b.digs.len())
        };
        let mut result = Big::zero(prec);
        for operand in [a, b] {
            if operand.kind != Kind::Num {
                result.kind = operand.kind;
                result.neg = operand.neg;
                result.zero = false;
                return Err(result);
            }
        }
        Ok(result)
    }

    /// `a + b`.
    pub fn add(a: &Big, b: &Big) -> Big {
        match Self::start(a, b) {
            Ok(mut result) => {
                result.sum_into(a, b);
                result
            }
            Err(special) => special,
        }
    }

    /// `a - b`.
    pub fn sub(a: &Big, b: &Big) -> Big {
        match Self::start(a, b) {
            Ok(mut result) => {
                result.diff_into(a, b);
                result
            }
            Err(special) => special,
        }
    }

    /// `a * b`.
    pub fn mul(a: &Big, b: &Big) -> Big {
        match Self::start(a, b) {
            Ok(mut result) => {
                result.prod_into(a, b);
                result
            }
            Err(special) => special,
        }
    }

    /// `a / b`. `None` for division by zero.
    pub fn div(a: &Big, b: &Big) -> Option<Big> {
        match Self::start(a, b) {
            Ok(mut result) => {
                result.quotient_into(None, a, b)?;
                Some(result)
            }
            Err(special) => Some(special),
        }
    }

    /// The whole-number quotient of `a / b` and the remainder. `None` for
    /// division by zero.
    pub fn div_rem(a: &Big, b: &Big) -> Option<(Big, Big)> {
        let quotient = Self::start(a, b);
        let remainder = Self::start(a, b);
        match (quotient, remainder) {
            (Ok(mut quotient), Ok(mut remainder)) => {
                quotient.quotient_into(Some(&mut remainder), a, b)?;
                Some((quotient, remainder))
            }
            (Err(q), Err(r)) | (Ok(q), Err(r)) | (Err(q), Ok(r)) => Some((q, r)),
        }
    }

    pub(crate) fn sum_into(&mut self, a: &Big, b: &Big) {
        if a.neg == b.neg {
            self.abs_sum_into(a, b);
            self.neg = a.neg;
        } else if a.neg {
            self.abs_diff_into(b, a);
        } else {
            self.abs_diff_into(a, b);
        }
    }

    fn diff_into(&mut self, a: &Big, b: &Big) {
        if a.neg == b.neg {
            self.abs_diff_into(a, b);
            if a.neg {
                self.neg = !self.neg;
            }
        } else {
            self.abs_sum_into(a, b);
            if !self.zero {
                self.neg = a.neg;
            }
        }
    }

    fn abs_sum_into(&mut self, a: &Big, b: &Big) {
        if a.zero {
            self.copy_from(b, true);
            return;
        }
        if b.zero {
            self.copy_from(a, true);
            return;
        }
        let (exp1, exp2) = (a.exp, b.exp);
        let (prec1, prec2) = (a.digs.len() as i32, b.digs.len() as i32);
        let new_prec = self.digs.len() as i32;
        let new_exp = exp1.max(exp2);
        self.exp = new_exp;
        self.kind = Kind::Num;
        // Each number covers a range of decimal positions.
        let (hi1, lo1) = (exp1 - 1, exp1 - prec1);
        let (hi2, lo2) = (exp2 - 1, exp2 - prec2);
        let (hi3, lo3) = (new_exp - 1, new_exp - new_prec);
        // The digit just below what fits decides the rounding.
        let (mut trail_dig, mut trail_val) = (0, 0);
        if (lo1..=hi1).contains(&(lo3 - 1)) {
            trail_dig = a.dig(exp1 - (lo3 - 1) - 1);
            trail_val = i32::from(a.any_digits_from(exp1 - (lo3 - 1)));
        } else if (lo2..=hi2).contains(&(lo3 - 1)) {
            trail_dig = b.dig(exp2 - (lo3 - 1) - 1);
            trail_val = i32::from(b.any_digits_from(exp2 - (lo3 - 1)));
        }
        let mut carry = 0;
        for pos in lo3..=hi3 {
            let mut acc = carry;
            if pos >= lo1 && pos <= hi1 {
                acc += a.dig(exp1 - pos - 1);
            }
            if pos >= lo2 && pos <= hi2 {
                acc += b.dig(exp2 - pos - 1);
            }
            carry = i32::from(acc > 9);
            if acc > 9 {
                acc -= 10;
            }
            self.set_dig(new_exp - pos - 1, acc);
        }
        if carry != 0 {
            trail_val |= trail_dig;
            trail_dig = self.dig(new_prec - 1);
            self.shift_right(1);
            self.exp += 1;
            self.set_dig(0, 1);
        }
        self.neg = false;
        self.zero = false;
        self.round_for_dropped(trail_dig, trail_val);
        self.normalize();
    }

    pub(crate) fn abs_diff_into(&mut self, a: &Big, b: &Big) {
        if b.zero {
            self.copy_from(a, true);
            return;
        }
        if a.zero {
            self.copy_from(b, true);
            self.neg = true;
            return;
        }
        let (mut a, mut b) = (a, b);
        let mut result_neg = false;
        if a.compare_abs(b) == Ordering::Less {
            result_neg = true;
            std::mem::swap(&mut a, &mut b);
        }
        let new_prec = self.digs.len() as i32;
        self.exp = a.exp.max(b.exp);
        self.kind = Kind::Num;
        let (hi1, lo1) = (a.exp - 1, a.exp - a.digs.len() as i32);
        let (hi2, lo2) = (b.exp - 1, b.exp - b.digs.len() as i32);
        let (hi3, lo3) = (self.exp - 1, self.exp - new_prec);
        // If the smaller number has digits below what fits, borrow for
        // them when they amount to more than half a unit.
        let mut borrow = 0;
        if !(lo1..=hi1).contains(&(lo3 - 1)) && (lo2..=hi2).contains(&(lo3 - 1)) {
            let idx = b.exp - (lo3 - 1) - 1;
            if b.dig(idx) >= 6 || (b.dig(idx) == 5 && b.any_digits_from(idx + 1)) {
                borrow = 1;
            }
        }
        for pos in lo3..=hi3 {
            let mut acc = 0;
            if pos >= lo1 && pos <= hi1 {
                acc = a.dig(a.exp - pos - 1);
            }
            if pos >= lo2 && pos <= hi2 {
                acc -= b.dig(b.exp - pos - 1);
            }
            acc -= borrow;
            borrow = i32::from(acc < 0);
            if acc < 0 {
                acc += 10;
            }
            self.set_dig(self.exp - pos - 1, acc);
        }
        self.neg = result_neg;
        self.zero = false;
        self.normalize();
    }

    pub(crate) fn prod_into(&mut self, a: &Big, b: &Big) {
        let (prec1, prec2) = (a.digs.len() as i32, b.digs.len() as i32);
        let new_prec = self.digs.len() as i32;
        self.digs.fill(0);
        self.kind = Kind::Num;
        let mut out_exp = a.exp + b.exp - prec2;
        let (mut trail_dig, mut trail_val) = (0, 0);
        // Long multiplication, one digit of `b` at a time from the
        // right, shifting the partial result along as it goes.
        let mut idx2 = prec2;
        while idx2 != 0 {
            idx2 -= 1;
            let mut carry = 0;
            let mut out_idx = prec1;
            let digit = b.dig(idx2);
            if digit != 0 {
                let mut idx1 = prec1;
                while idx1 != 0 {
                    idx1 -= 1;
                    out_idx -= 1;
                    let value = a.dig(idx1) * digit + carry + self.dig(out_idx);
                    carry = value / 10;
                    self.set_dig(out_idx, value % 10);
                }
            }
            if idx2 != 0 || carry != 0 {
                trail_val |= trail_dig;
                trail_dig = self.dig(new_prec - 1);
                self.shift_right(1);
                out_exp += 1;
                self.set_dig(0, carry);
            }
        }
        self.exp = out_exp;
        self.neg = a.neg != b.neg;
        self.zero = false;
        self.round_for_dropped(trail_dig, trail_val);
        self.normalize();
    }

    /// Long division. With `remainder`, stop at the whole-number part of
    /// the quotient and report what is left over.
    pub(crate) fn quotient_into(
        &mut self,
        mut remainder: Option<&mut Big>,
        a: &Big,
        b: &Big,
    ) -> Option<()> {
        if b.zero {
            return None;
        }
        if a.zero {
            self.set_zero();
            if let Some(remainder) = remainder {
                remainder.set_zero();
            }
            return Some(());
        }
        let quo_prec = self.digs.len();
        let dvs_prec = b.digs.len();
        let rem_prec = (a.digs.len().max(dvs_prec) + 2).max(3);
        let mut rem = Big::zero(rem_prec);
        let mut rem2 = Big::zero(rem_prec);
        let mut dvs2 = Big::zero(rem_prec);
        let mut dvs = Big::zero(dvs_prec);
        rem.copy_from(a, true);
        dvs.copy_from(b, true);
        rem.neg = false;
        dvs.neg = false;
        self.digs.fill(0);
        self.kind = Kind::Num;
        self.zero = false;

        let mut lead_dig_set = false;
        let mut quo_exp = a.exp - b.exp + 1;
        // Line the divisor up under the dividend.
        dvs.exp = a.exp;
        let mut zero_remainder = false;

        if remainder.is_some() && quo_exp <= 0 {
            // The quotient has no whole part.
            if let Some(remainder) = remainder.as_deref_mut() {
                remainder.copy_from(&rem, true);
                remainder.neg = a.neg;
            }
            self.set_zero();
            return Some(());
        }

        let mut quo_idx = 0usize;
        loop {
            // Estimate this digit of the quotient from the leading
            // digits, low enough never to overshoot.
            let mut acc = 0;
            let mut rem_approx = rem.dig(0) * 100 + rem.dig(1) * 10 + rem.dig(2);
            let mut dvs_approx = dvs.dig(0) * 100 + dvs.dig(1) * 10 + dvs.dig(2) + 1;
            let exp_diff = rem.exp - dvs.exp;
            if exp_diff > 0 {
                for _ in 0..exp_diff.min(6) {
                    rem_approx *= 10;
                }
            } else if exp_diff <= -3 {
                dvs_approx = 1000;
            } else {
                for _ in exp_diff..0 {
                    dvs_approx *= 10;
                }
            }
            let dig_approx = rem_approx / dvs_approx;
            if dig_approx > 2 {
                acc = dig_approx;
                dvs2.copy_from(&dvs, false);
                dvs2.mul_by_long(dig_approx as u64);
                rem2.abs_diff_into(&rem, &dvs2);
                if rem2.zero {
                    zero_remainder = true;
                } else {
                    std::mem::swap(&mut rem, &mut rem2);
                }
            }
            // Finish the digit by subtracting until what is left is
            // smaller than the divisor.
            if !zero_remainder {
                loop {
                    match rem.compare_abs(&dvs) {
                        Ordering::Less => break,
                        Ordering::Equal => {
                            zero_remainder = true;
                            acc += 1;
                            break;
                        }
                        Ordering::Greater => {
                            rem2.abs_diff_into(&rem, &dvs);
                            std::mem::swap(&mut rem, &mut rem2);
                            acc += 1;
                        }
                    }
                }
            }
            if quo_idx < quo_prec {
                self.set_dig(quo_idx as i32, acc);
            } else if quo_idx == quo_prec {
                // One digit more than fits: it decides the rounding.
                self.exp = quo_exp;
                self.round_for_dropped(acc, i32::from(!zero_remainder));
                break;
            }
            if acc != 0 {
                lead_dig_set = true;
            }
            if lead_dig_set {
                quo_idx += 1;
            } else {
                quo_exp -= 1;
            }
            if zero_remainder
                || (remainder.is_some() && (quo_idx as i32 == quo_exp || quo_idx == quo_prec))
            {
                for idx in quo_idx..quo_prec {
                    self.set_dig(idx as i32, 0);
                }
                self.exp = quo_exp;
                break;
            }
            dvs.exp -= 1;
        }

        if let Some(remainder) = remainder {
            if zero_remainder {
                remainder.set_zero();
            } else {
                remainder.copy_from(&rem, true);
                remainder.neg = a.neg;
                remainder.normalize();
            }
        }
        self.neg = a.neg != b.neg;
        self.normalize();
        Some(())
    }

    /// Multiply in place by a small integer.
    pub fn mul_by_long(&mut self, value: u64) {
        let prec = self.digs.len() as i32;
        let mut carry: u64 = 0;
        for idx in (0..prec).rev() {
            let product = value * self.dig(idx) as u64 + carry;
            self.set_dig(idx, (product % 10) as i32);
            carry = product / 10;
        }
        let (mut dropped_dig, mut dropped_val) = (0, 0);
        while carry != 0 {
            dropped_val |= dropped_dig;
            dropped_dig = self.dig(prec - 1);
            self.shift_right(1);
            self.exp += 1;
            self.set_dig(0, (carry % 10) as i32);
            carry /= 10;
        }
        self.round_for_dropped(dropped_dig, dropped_val);
        self.normalize();
    }

    /// Divide in place by a small integer, rounding the result.
    pub fn div_by_long(&mut self, value: u64) {
        let prec = self.digs.len();
        let mut exp = self.exp;
        let mut rem: u64 = 0;
        let mut significant = false;
        let (mut in_idx, mut out_idx) = (0usize, 0usize);
        while out_idx < prec {
            rem = rem * 10 + self.dig(in_idx as i32) as u64;
            let quo = rem / value;
            rem %= value;
            if quo != 0 {
                significant = true;
            }
            if significant {
                self.set_dig(out_idx as i32, quo as i32);
                out_idx += 1;
            } else {
                exp -= 1;
                self.exp = exp;
            }
            if rem == 0 && in_idx >= prec {
                if !significant {
                    self.set_zero();
                    out_idx = prec;
                }
                break;
            }
            in_idx += 1;
        }
        for idx in out_idx..prec {
            self.set_dig(idx as i32, 0);
        }
        let last = self.dig(prec as i32 - 1);
        if rem * 2 > value || (rem * 2 == value && last & 1 != 0) {
            self.round_up_abs(prec as i32);
        }
        self.normalize();
    }

    /// Divide in place by a small integer, returning the remainder of the
    /// whole-number part.
    pub fn div_by_long_rem(&mut self, value: u64) -> u64 {
        let prec = self.digs.len();
        let mut exp = self.exp;
        if exp <= 0 {
            self.set_zero();
            return 0;
        }
        let mut rem: u64 = 0;
        let mut significant = false;
        let (mut in_idx, mut out_idx) = (0usize, 0usize);
        while out_idx < prec {
            rem = rem * 10 + self.dig(in_idx as i32) as u64;
            let quo = rem / value;
            rem %= value;
            if quo != 0 {
                significant = true;
            }
            if significant {
                self.set_dig(out_idx as i32, quo as i32);
                out_idx += 1;
                if out_idx as i32 == exp {
                    break;
                }
            } else {
                exp -= 1;
                self.exp = exp;
                if exp == 0 {
                    break;
                }
            }
            if rem == 0 && in_idx >= prec {
                if !significant {
                    self.set_zero();
                    out_idx = prec;
                }
                break;
            }
            in_idx += 1;
        }
        for idx in out_idx..prec {
            self.set_dig(idx as i32, 0);
        }
        self.normalize();
        rem
    }

    // ---- parts of a number ----

    /// Whether the number has no fractional part.
    pub fn is_frac_zero(&self) -> bool {
        !self.any_digits_from(self.exp.max(0))
    }

    /// Keep only the fractional part.
    pub fn keep_frac(&mut self) {
        for idx in 0..self.exp.min(self.digs.len() as i32) {
            self.set_dig(idx, 0);
        }
        self.normalize();
    }

    /// Keep only the whole part.
    pub fn keep_whole(&mut self) {
        if self.exp <= 0 {
            self.set_zero();
        } else {
            for idx in self.exp..self.digs.len() as i32 {
                self.set_dig(idx, 0);
            }
            self.normalize();
        }
    }

    /// Add one to the magnitude of the whole part.
    pub fn increment_abs(&mut self) {
        let mut idx = self.exp.max(0);
        if idx > self.digs.len() as i32 {
            return;
        }
        let mut carry = true;
        while idx != 0 {
            idx -= 1;
            let d = self.dig(idx);
            if d == 9 {
                self.set_dig(idx, 0);
            } else {
                self.set_dig(idx, d + 1);
                carry = false;
                break;
            }
        }
        if carry {
            while self.exp < 0 {
                self.shift_right(1);
                self.exp += 1;
            }
            self.shift_right(1);
            self.exp += 1;
            self.set_dig(0, 1);
        }
        self.zero = false;
    }

    /// The value as a 32-bit integer, rounded. The flag reports overflow.
    pub fn to_i32(&self) -> (i32, bool) {
        let mut overflow = false;
        let magnitude = self.to_u32_magnitude(&mut overflow);
        if self.neg {
            overflow |= magnitude > 2_147_483_648;
            ((magnitude as i64).wrapping_neg() as i32, overflow)
        } else {
            overflow |= magnitude > 2_147_483_647;
            (magnitude as i32, overflow)
        }
    }

    fn to_u32_magnitude(&self, overflow: &mut bool) -> u32 {
        if self.exp < 0 || self.zero {
            return 0;
        }
        let round_inc = u32::from(self.round_dir(self.exp));
        let stop = (self.exp.max(0) as usize).min(self.digs.len());
        let mut acc: u32 = 0;
        for &d in &self.digs[..stop] {
            *overflow |= acc > u32::MAX / 10;
            acc = acc.wrapping_mul(10);
            *overflow |= acc > u32::MAX - u32::from(d);
            acc = acc.wrapping_add(u32::from(d));
        }
        // A number with more whole digits than it stores is too large.
        if self.exp as usize > self.digs.len() {
            *overflow = true;
        }
        *overflow |= acc > u32::MAX - round_inc;
        acc.wrapping_add(round_inc)
    }

    // ---- text ----

    /// The number of digits needed to hold a decimal number as written.
    pub fn precision_from_str(text: &str) -> usize {
        let mut chars = text.trim_start().chars().peekable();
        if matches!(chars.peek(), Some('-' | '+')) {
            chars.next();
        }
        while chars.next_if(|c| c.is_whitespace()).is_some() {}
        let (mut prec, mut digits, mut significant, mut point) = (0, 0, false, false);
        for c in chars {
            if c.is_ascii_digit() {
                if c != '0' {
                    significant = true;
                }
                if significant {
                    prec += 1;
                }
                digits += 1;
            } else if c == '.' && !point {
                point = true;
            } else {
                break;
            }
        }
        if prec == 0 { digits } else { prec }
    }

    /// Parse a decimal number, with an optional exponent, to a precision.
    /// Parse a whole number written in a radix other than 10. The
    /// precision is what the digits need.
    pub fn parse_radix(text: &str, radix: u32) -> Self {
        let rest = text.trim_start();
        let (neg, rest) = match rest.strip_prefix('-') {
            Some(rest) => (true, rest),
            None => (false, rest.strip_prefix('+').unwrap_or(rest)),
        };
        let digits: Vec<u32> = rest
            .trim_start()
            .chars()
            .map_while(|c| c.to_digit(36).filter(|&d| d < radix))
            .skip_while(|&d| d == 0)
            .collect();
        let prec = (digits.len() as f64 * f64::from(radix).log10()).ceil() as usize;
        let mut n = Self::zero(prec.max(2));
        for digit in digits {
            if !n.zero {
                n.mul_by_long(u64::from(radix));
            }
            n = Self::add(&n, &Self::from_int(digit.into(), 2));
        }
        let mut n = n.with_prec(prec.max(1), false);
        n.neg = neg && !n.zero;
        n
    }

    pub fn parse(text: &str, prec: usize) -> Self {
        let mut n = Self::zero(prec);
        n.zero = false;
        let mut chars = text.trim_start().chars().peekable();
        match chars.peek() {
            Some('+') => {
                chars.next();
            }
            Some('-') => {
                n.neg = true;
                chars.next();
            }
            _ => {}
        }
        while chars.next_if(|c| c.is_whitespace()).is_some() {}
        let mut exp = 0i32;
        let (mut significant, mut point) = (false, false);
        let mut idx = 0;
        while let Some(c) = chars.next() {
            if let Some(d) = c.to_digit(10) {
                if d != 0 {
                    significant = true;
                }
                if significant {
                    if idx < prec {
                        n.digs[idx] = d as u8;
                        idx += 1;
                    }
                    if !point {
                        exp += 1;
                    }
                } else if point {
                    exp -= 1;
                }
            } else if c == '.' && !point {
                point = true;
            } else if c == 'e' || c == 'E' {
                let mut exp_neg = false;
                match chars.peek() {
                    Some('+') => {
                        chars.next();
                    }
                    Some('-') => {
                        exp_neg = true;
                        chars.next();
                    }
                    _ => {}
                }
                let mut acc = 0i32;
                while let Some(d) = chars.peek().and_then(|c| c.to_digit(10)) {
                    acc = acc.saturating_mul(10).saturating_add(d as i32);
                    chars.next();
                }
                exp = exp.saturating_add(if exp_neg { -acc } else { acc });
                break;
            } else {
                break;
            }
        }
        n.exp = exp;
        let neg = n.neg;
        n.normalize();
        // A negative zero keeps its sign.
        if n.zero {
            n.neg = neg;
        }
        n
    }

    /// Format the number. The parameters are those of the `formatString`
    /// method: a limit on digits, a minimum number of places before the
    /// point, a number of digits after it, a minimum number of exponent
    /// digits (each -1 for no constraint), flags, and text to pad with
    /// on the left in place of spaces.
    pub fn format(
        &self,
        max_digits: i32,
        whole_places: i32,
        frac_digits: i32,
        exp_digits: i32,
        flags: u32,
        lead_fill: Option<&str>,
    ) -> String {
        match self.kind {
            Kind::Num => {}
            Kind::NaN => return "1.#NAN".to_owned(),
            Kind::Inf => return if self.neg { "-1.#INF" } else { "1.#INF" }.to_owned(),
        }
        let always_sign_exp = flags & FORMAT_EXP_SIGN != 0;
        let mut always_exp = flags & FORMAT_EXP != 0;
        let lead_zero = flags & FORMAT_LEADING_ZERO != 0;
        let always_show_pt = flags & FORMAT_POINT != 0;
        let exp_caps = flags & FORMAT_EXP_CAP != 0;
        let commas = flags & FORMAT_COMMAS != 0;
        let euro = flags & FORMAT_EUROSTYLE != 0;
        let max_sig = flags & FORMAT_MAXSIG != 0;
        let trailing_zeros = flags & FORMAT_TRAILING_ZEROS != 0;
        let (point_char, comma_char) = if euro { (',', '.') } else { ('.', ',') };
        let plus_sign = if flags & FORMAT_POS_SPACE != 0 {
            Some(' ')
        } else if flags & FORMAT_SIGN != 0 {
            Some('+')
        } else {
            None
        };

        let exp = self.exp;
        if max_digits >= 0 && exp > max_digits {
            always_exp = true;
        }
        if max_digits >= 0
            && exp < 0
            && (-exp > max_digits || (frac_digits != -1 && -exp > frac_digits))
        {
            always_exp = true;
        }
        if flags & FORMAT_COMPACT != 0 && (exp < -3 || (max_digits >= 0 && exp - 1 > max_digits)) {
            always_exp = true;
        }

        // Work on a copy rounded to the digits that can be shown.
        let prec = if max_digits < 0 {
            self.digs.len()
        } else {
            (max_digits as usize).min(self.digs.len())
        };
        let mut num = self.with_prec(prec, true);
        let prec = prec as i32;

        'start_over: loop {
            let exp = num.exp;
            let mut out: Vec<char> = Vec::new();
            let (mut all_out, mut sig_out) = (0, 0);
            let dig_before_pt = if always_exp {
                1
            } else if exp > 0 {
                exp
            } else {
                0
            };
            if max_digits >= 0 && dig_before_pt > max_digits && !always_exp {
                always_exp = true;
                continue 'start_over;
            }
            let mut dig_after_pt = if max_digits >= 0 {
                max_digits - dig_before_pt
            } else if frac_digits >= 0 {
                frac_digits
            } else if prec > dig_before_pt {
                prec - dig_before_pt
            } else {
                0
            };

            // Padding on the left, to line the number up.
            if !always_exp && dig_before_pt < whole_places {
                let mut count = whole_places - dig_before_pt;
                if dig_before_pt == 0 && lead_zero {
                    count -= 1;
                }
                if commas {
                    for idx in dig_before_pt..whole_places {
                        if idx % 3 == 0 {
                            count += 1;
                        }
                    }
                }
                let fill: Vec<char> = lead_fill.unwrap_or(" ").chars().collect();
                for n in 0..count.max(0) as usize {
                    out.push(fill[n % fill.len()]);
                }
            }
            let digits_start = out.len();

            if num.neg {
                out.push('-');
            } else if let Some(sign) = plus_sign {
                out.push(sign);
            }
            if dig_before_pt == 0 && lead_zero {
                out.push('0');
                if max_digits >= 0 && !max_sig {
                    dig_after_pt -= 1;
                }
                all_out += 1;
            }
            if max_digits >= 0
                && !always_exp
                && frac_digits != -1
                && dig_after_pt < frac_digits
                && dig_before_pt > 1
            {
                always_exp = true;
                continue 'start_over;
            }

            // The digits before the point.
            let mut idx = 0;
            while idx < dig_before_pt && idx < prec {
                if idx != 0 && commas && !always_exp && (dig_before_pt - idx) % 3 == 0 {
                    out.push(comma_char);
                }
                let d = num.dig(idx);
                out.push((b'0' + d as u8) as char);
                all_out += 1;
                if d != 0 || sig_out != 0 {
                    sig_out += 1;
                }
                idx += 1;
            }
            while idx < dig_before_pt {
                out.push('0');
                all_out += 1;
                if sig_out != 0 {
                    sig_out += 1;
                }
                idx += 1;
            }

            let mut show_pt = always_show_pt || frac_digits != 0;
            if show_pt {
                let mut last_non_zero = out.len();
                out.push(point_char);
                if always_show_pt {
                    last_non_zero = out.len();
                }
                let frac_lim = if frac_digits == -1 {
                    dig_after_pt
                } else {
                    frac_digits.min(dig_after_pt)
                };
                let mut frac_len = 0;
                // Zeros between the point and the first digit.
                if idx == 0 && exp < 0 {
                    let mut count = exp;
                    while count != 0 && frac_len < frac_lim {
                        out.push('0');
                        if !max_sig {
                            frac_len += 1;
                        }
                        all_out += 1;
                        count += 1;
                    }
                }
                while idx < prec && frac_len < frac_lim {
                    let d = num.dig(idx);
                    out.push((b'0' + d as u8) as char);
                    if d != 0 {
                        last_non_zero = out.len();
                    }
                    all_out += 1;
                    if d != 0 || sig_out != 0 {
                        sig_out += 1;
                    }
                    idx += 1;
                    frac_len += 1;
                }
                if trailing_zeros && max_digits >= 0 {
                    let mut shown = if max_sig { sig_out } else { all_out };
                    while shown < max_digits {
                        out.push('0');
                        shown += 1;
                    }
                }
                if frac_digits != -1 {
                    while frac_len < frac_lim {
                        out.push('0');
                        frac_len += 1;
                    }
                } else if !trailing_zeros {
                    // Drop trailing zeros, and the point if nothing
                    // follows it.
                    if last_non_zero < out.len() && out[last_non_zero] == point_char {
                        show_pt = false;
                    }
                    if out.len() > last_non_zero && num.round_dir(idx) {
                        last_non_zero = out.len();
                    }
                    out.truncate(last_non_zero);
                    if !out[digits_start..].iter().any(char::is_ascii_digit) {
                        if out.last() == Some(&point_char) && out.len() > digits_start {
                            let last = out.len() - 1;
                            out[last] = '0';
                            out.push(point_char);
                        } else {
                            out.push('0');
                        }
                    }
                }
            }

            // If the digits left off round up, carry into what is shown.
            if num.round_dir(idx) {
                let mut need_carry = true;
                let mut found_pt = false;
                let mut dig_count = 0;
                let mut at = out.len();
                while at > digits_start {
                    at -= 1;
                    let c = out[at];
                    if c.is_ascii_digit() {
                        dig_count += 1;
                        if c == '9' {
                            out[at] = '0';
                            if show_pt && !found_pt && frac_digits == -1 {
                                out.pop();
                                dig_count -= 1;
                            }
                        } else {
                            out[at] = ((c as u8) + 1) as char;
                            need_carry = false;
                            break;
                        }
                    } else if c == point_char {
                        found_pt = true;
                    }
                }
                if need_carry {
                    // Every digit shown was a 9: round the number itself
                    // and format it again.
                    num.set_dig(idx, 0);
                    num.round_up_abs(idx);
                    if max_digits >= 0 && dig_count + 1 > max_digits {
                        always_exp = true;
                    }
                    continue 'start_over;
                }
            }

            if always_exp {
                out.push(if exp_caps { 'E' } else { 'e' });
                let mut disp_exp = exp - 1;
                if disp_exp < 0 {
                    out.push('-');
                    disp_exp = -disp_exp;
                } else if always_sign_exp {
                    out.push('+');
                }
                if disp_exp == 0 && exp_digits == -1 {
                    out.push('0');
                } else {
                    let digits = if disp_exp == 0 {
                        String::new()
                    } else {
                        disp_exp.to_string()
                    };
                    let padding = (exp_digits - digits.len() as i32).max(0) as usize;
                    out.extend(std::iter::repeat_n('0', padding));
                    out.extend(digits.chars());
                }
            }
            return out.into_iter().collect();
        }
    }

    /// The number as it converts to a string by default.
    pub fn to_text(&self) -> String {
        self.format(100, -1, -1, -1, 0, None)
    }

    /// The whole part of the number in a radix other than 10.
    pub fn to_radix_text(&self, radix: u32) -> String {
        if self.zero {
            return "0".to_owned();
        }
        match self.kind {
            Kind::Inf => return if self.neg { "-1.#INF" } else { "1.#INF" }.to_owned(),
            Kind::NaN => return "1.#NAN".to_owned(),
            Kind::Num => {}
        }
        let mut whole = self.clone();
        whole.round_to(whole.exp);
        whole.normalize();
        let mut digits = Vec::new();
        while !whole.zero {
            let rem = whole.div_by_long_rem(u64::from(radix));
            digits.push(
                char::from_digit(rem as u32, radix)
                    .unwrap_or('0')
                    .to_ascii_uppercase(),
            );
        }
        if digits.is_empty() {
            digits.push('0');
        }
        if self.neg {
            digits.push('-');
        }
        digits.iter().rev().collect()
    }
}

// ---- binary forms ----

/// Negate a 64-bit integer stored low byte first.
fn twos_complement(bytes: &mut [u8; 8]) {
    let mut carry = true;
    for byte in bytes.iter_mut() {
        *byte = (!*byte).wrapping_add(u8::from(carry));
        carry &= *byte == 0;
    }
}

impl Big {
    /// The value as an unsigned 32-bit integer, rounded. The flag reports
    /// overflow, which includes any negative number.
    pub fn to_u32(&self) -> (u32, bool) {
        let mut overflow = self.neg;
        let magnitude = self.to_u32_magnitude(&mut overflow);
        (magnitude, overflow)
    }

    /// An unsigned integer as a BigNumber.
    pub fn from_u64(value: u64, prec: usize) -> Self {
        let mut n = Self::zero(prec);
        n.set_uint(value);
        n
    }

    /// The whole part as a 64-bit integer, low byte first: two's
    /// complement if `signed`, and otherwise unsigned. The flag reports
    /// a value that does not fit.
    pub fn to_p8(&self, signed: bool) -> ([u8; 8], bool) {
        let mut overflow = self.exp > 30;
        let rounded = self.with_prec(20, true);
        let mut magnitude: u128 = 0;
        if !rounded.zero {
            for idx in 0..rounded.exp.clamp(0, 60) {
                magnitude = magnitude
                    .wrapping_mul(10)
                    .wrapping_add(rounded.dig(idx) as u128);
            }
        }
        overflow |= magnitude >> 64 != 0;
        let mut bytes = (magnitude as u64).to_le_bytes();
        if signed {
            if self.neg {
                twos_complement(&mut bytes);
                overflow |= bytes[7] & 0x80 == 0;
            } else {
                overflow |= bytes[7] & 0x80 != 0;
            }
        } else if self.neg {
            overflow = true;
            twos_complement(&mut bytes);
        }
        (bytes, overflow)
    }

    /// A 64-bit integer, stored low byte first, as a BigNumber.
    pub fn from_p8(mut bytes: [u8; 8], signed: bool) -> Self {
        let neg = signed && bytes[7] & 0x80 != 0;
        if neg {
            twos_complement(&mut bytes);
        }
        let mut n = Self::from_u64(u64::from_le_bytes(bytes), 20);
        n.neg = neg;
        n
    }

    /// The whole part in the BER compressed integer format: seven bits
    /// to a byte, most significant first, with the high bit set on every
    /// byte but the last. The flag reports a negative number or one too
    /// long for the 256 bytes allowed.
    pub fn to_ber(&self) -> (Vec<u8>, bool) {
        let mut overflow = self.neg;
        let mut rest = self.clone();
        let mut out = Vec::new();
        while !rest.zero && out.len() < 256 {
            out.push(rest.div_by_long_rem(128) as u8);
        }
        overflow |= !rest.zero;
        if out.is_empty() {
            out.push(0);
        }
        out.reverse();
        let last = out.len() - 1;
        for byte in &mut out[..last] {
            *byte |= 0x80;
        }
        (out, overflow)
    }

    /// A BER compressed integer, given as its seven-bit groups, as a
    /// BigNumber.
    pub fn from_ber(groups: &[u8]) -> Self {
        // The precision comes from the number of bits, with the same
        // rounded logarithm of 2 the reference uses.
        #[allow(clippy::approx_constant)]
        let prec = ((groups.len() * 7) as f64 * 0.30103 + 0.5) as usize;
        let mut total = Self::zero(prec.max(1));
        for &group in groups {
            if !total.zero {
                total.mul_by_long(128);
            }
            let digit = Self::from_u64(u64::from(group & 0x7f), 3);
            let mut sum = Self::zero(prec.max(1));
            sum.sum_into(&total, &digit);
            total = sum;
        }
        total
    }
}
