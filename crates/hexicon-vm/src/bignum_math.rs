//! The mathematical functions of BigNumber: roots, logarithms,
//! exponentials, and trigonometry.
//!
//! Each follows the reference VM's method (`vmbignum.cpp`): the same
//! series, the same working precision of three extra digits, the same
//! stopping tests. The last digit of a result depends on all of that, so
//! a cleverer algorithm would give different answers.

use std::cmp::Ordering;

use crate::bignum::Big;

/// Why a function has no result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MathError {
    /// The argument is outside the function's domain.
    OutOfRange,
    /// The result is too large to represent.
    Overflow,
    /// The function divides by zero.
    DivideByZero,
}

type MathResult<T> = Result<T, MathError>;

/// Constants computed so far. Each is kept at the highest precision that
/// has been asked for.
#[derive(Debug, Clone, Default)]
pub struct Cache {
    pi: Option<Big>,
    ln10: Option<Big>,
    e: Option<Big>,
}

fn one() -> Big {
    Big::from_int(1, 1)
}

/// A register of a given precision holding a value, unrounded.
fn reg(prec: usize, value: &Big) -> Big {
    value.with_prec(prec, false)
}

fn negate(n: &mut Big) {
    if !n.zero {
        n.neg = !n.neg;
    }
}

/// Divide, at the precision of `a` and `b` whichever is larger, into a
/// register of precision `prec`.
fn quotient(prec: usize, a: &Big, b: &Big) -> MathResult<Big> {
    let mut result = Big::zero(prec);
    result
        .quotient_into(None, a, b)
        .ok_or(MathError::DivideByZero)?;
    Ok(result)
}

fn product(prec: usize, a: &Big, b: &Big) -> Big {
    let mut result = Big::zero(prec);
    result.prod_into(a, b);
    result
}

fn sum(prec: usize, a: &Big, b: &Big) -> Big {
    let mut result = Big::zero(prec);
    result.sum_into(a, b);
    result
}

fn abs_diff(prec: usize, a: &Big, b: &Big) -> Big {
    let mut result = Big::zero(prec);
    result.abs_diff_into(a, b);
    result
}

/// Whether a series term is too small to change a sum.
fn negligible(total: &Big, term: &Big, prec: usize) -> bool {
    term.zero || total.exp - term.exp > prec as i32
}

impl Cache {
    fn cached(slot: &Option<Big>, prec: usize) -> Option<Big> {
        slot.as_ref().filter(|value| value.prec() >= prec).cloned()
    }

    /// Pi, to at least `prec` digits.
    pub fn pi(&mut self, prec: usize) -> MathResult<Big> {
        let prec = (prec + 7) & !7;
        if let Some(value) = Self::cached(&self.pi, prec) {
            return Ok(value);
        }
        // Pi is four times the arcsine of the square root of a half.
        let work = prec + 3;
        let mut half = reg(work, &one());
        half.div_by_long(2);
        let root = sqrt_into(work, &half)?;
        let mut value = asin_series(root);
        value.mul_by_long(4);
        let value = value.with_prec(prec, true);
        self.pi = Some(value.clone());
        Ok(value)
    }

    /// The natural logarithm of 10, to at least `prec` digits.
    pub fn ln10(&mut self, prec: usize) -> MathResult<Big> {
        let prec = (prec + 7) & !7;
        if let Some(value) = Self::cached(&self.ln10, prec) {
            return Ok(value);
        }
        // Twice the logarithm of the square root of 10, which is small
        // enough for the series to converge quickly.
        let work = prec + 3;
        let ten = reg(work, &Big::from_int(10, 2));
        let root = sqrt_into(work, &ten)?;
        let mut value = ln_series(root)?;
        value.mul_by_long(2);
        let value = value.with_prec(prec, true);
        self.ln10 = Some(value.clone());
        Ok(value)
    }

    /// The base of the natural logarithms, to at least `prec` digits.
    pub fn e(&mut self, prec: usize) -> MathResult<Big> {
        let prec = (prec + 7) & !7;
        if let Some(value) = Self::cached(&self.e, prec) {
            return Ok(value);
        }
        let value = exp_into(prec, &one(), self)?;
        self.e = Some(value.clone());
        Ok(value)
    }
}

/// The square root of `src`, to `prec` digits, by Newton's method.
pub fn sqrt_into(prec: usize, src: &Big) -> MathResult<Big> {
    if src.neg {
        return Err(MathError::OutOfRange);
    }
    let work = prec + 3;
    let mut guess = src.with_prec(work, true);
    // Start from a number with half the exponent.
    if guess.exp & 1 != 0 {
        guess.exp = (guess.exp + 1) / 2;
        guess.div_by_long(2);
    } else {
        guess.exp /= 2;
        guess.mul_by_long(2);
    }
    loop {
        if guess.zero {
            break;
        }
        let ratio = quotient(work, src, &guess)?;
        let mut next = sum(work, &guess, &ratio);
        next.div_by_long(2);
        // Done when one more step changes nothing that will be kept.
        if guess.neg == next.neg
            && guess.exp == next.exp
            && guess.digs[..prec + 1] == next.digs[..prec + 1]
        {
            break;
        }
        guess = next;
    }
    Ok(guess.with_prec(prec, true))
}

/// The series for the natural logarithm of a number near one:
/// `2 * (y + y^3/3 + y^5/5 + ...)` with `y = (x-1)/(x+1)`.
fn ln_series(mut x: Big) -> MathResult<Big> {
    let work = x.prec();
    let numerator = abs_diff(work, &x, &one());
    x.increment_abs();
    let mut power = quotient(work, &numerator, &x)?;
    let square = product(work, &power, &power);
    let mut total = reg(work, &power);
    let mut n: u64 = 1;
    loop {
        let next = product(work, &power, &square);
        power = reg(work, &next);
        n += 2;
        let mut term = next;
        term.div_by_long(n);
        if negligible(&total, &term, work) {
            break;
        }
        total = sum(work, &total, &term);
    }
    total.mul_by_long(2);
    Ok(total)
}

/// The natural logarithm of `src`, to `prec` digits.
pub fn ln_into(prec: usize, src: &Big, cache: &mut Cache) -> MathResult<Big> {
    let ln10 = cache.ln10(prec + 3)?;
    if src.zero {
        return Err(MathError::OutOfRange);
    }
    let work = prec + 3;
    // Split off the power of ten, leaving a number from 0.2 to 2.
    let mut mantissa = reg(work, src);
    let mut src_exp = mantissa.exp;
    mantissa.exp = 0;
    if mantissa.digs[0] == 1 {
        mantissa.exp = 1;
        src_exp -= 1;
    }
    let mut result = ln_series(mantissa)?;
    if src_exp != 0 {
        let mut scaled = ln10.with_prec(work, true);
        if src_exp < 0 {
            scaled.neg = true;
            src_exp = -src_exp;
        }
        scaled.mul_by_long(src_exp as u64);
        result = sum(work, &result, &scaled);
    }
    Ok(result.with_prec(prec, true))
}

/// `e` raised to the power `src`, to `prec` digits.
pub fn exp_into(prec: usize, src: &Big, cache: &mut Cache) -> MathResult<Big> {
    let ln10 = cache.ln10(prec + 3)?;
    let work = prec + 3;
    // Divide by ln 10: the whole part of that is the decimal exponent of
    // the result.
    let mut scaled = quotient(work, src, &ln10)?;
    let whole_digits = scaled.exp.max(0) as usize;
    let mut new_exp: i64 = 0;
    for idx in 0..whole_digits {
        let digit = scaled.digs.get(idx).copied().unwrap_or(0);
        new_exp = new_exp * 10 + i64::from(digit);
        if new_exp > if scaled.neg { 32769 } else { 32766 } {
            return Err(MathError::Overflow);
        }
        if let Some(slot) = scaled.digs.get_mut(idx) {
            *slot = 0;
        }
    }
    if scaled.neg {
        new_exp = -new_exp;
    }
    scaled.normalize();
    // What is left is the exponent of the fractional part. Halve it
    // until it is small, and square the result as many times afterward.
    let mut x = product(work, &scaled, &ln10);
    let mut half = reg(work, &one());
    half.div_by_long(2);
    let mut twos = 0;
    while x.compare_abs(&half) == Ordering::Greater {
        x.div_by_long(2);
        twos += 1;
    }
    // The Taylor series: 1 + x + x^2/2! + x^3/3! + ...
    let mut total = sum(work, &reg(work, &one()), &x);
    let mut power = product(work, &x, &x);
    let mut factorial = reg(work, &one());
    factorial.mul_by_long(2);
    let mut n: u64 = 2;
    let mut n_fact: u64 = 2;
    let mut use_int_fact = true;
    loop {
        let term = if use_int_fact {
            let mut term = reg(work, &power);
            term.div_by_long(n_fact);
            // Switch to the BigNumber factorial before the integer one
            // overflows a 32-bit long.
            if n_fact > 0x7fff_ffff / 10 / (n + 1) {
                use_int_fact = false;
            } else {
                n_fact *= n + 1;
            }
            term
        } else {
            quotient(work, &power, &factorial)?
        };
        if negligible(&total, &term, work) {
            break;
        }
        total = sum(work, &total, &term);
        n += 1;
        factorial.mul_by_long(n);
        power = product(work, &power, &x);
    }
    for _ in 0..twos {
        total = product(work, &total, &total);
    }
    let mut scale = reg(work, &one());
    scale.exp = (new_exp + 1) as i32;
    let result = product(work, &total, &scale);
    Ok(result.with_prec(prec, true))
}

/// The arcsine series: `x + (1/2)(x^3/3) + (1·3/2·4)(x^5/5) + ...`.
fn asin_series(x: Big) -> Big {
    let work = x.prec();
    let mut total = x;
    let mut power = reg(work, &total);
    let square = product(work, &total, &total);
    let mut n: u64 = 1;
    loop {
        n += 2;
        let mut coefficient = reg(work, &one());
        let mut i = 3;
        while i < n {
            coefficient.mul_by_long(i);
            i += 2;
        }
        let mut i = 2;
        while i < n {
            coefficient.div_by_long(i);
            i += 2;
        }
        coefficient.div_by_long(n);
        power = product(work, &power, &square);
        let term = product(work, &power, &coefficient);
        if negligible(&total, &term, work) {
            break;
        }
        total = sum(work, &total, &term);
    }
    total
}

/// The arcsine or arccosine of `src`, to `prec` digits.
pub fn asin_acos_into(prec: usize, src: &Big, is_acos: bool, cache: &mut Cache) -> MathResult<Big> {
    let pi = cache.pi(prec + 3)?;
    let work = prec + 3;
    let mut x = reg(work, src);
    if x.compare_abs(&reg(work, &one())) == Ordering::Greater {
        return Err(MathError::OutOfRange);
    }
    // The series is slow near one. Past 1/sqrt(2), use
    // asin(x) = pi/2 - asin(sqrt(1 - x^2)).
    let one_over_sqrt2 = Big {
        digs: vec![7, 0, 7, 1, 0, 6, 7, 8, 1, 4],
        exp: 0,
        neg: false,
        kind: crate::bignum::Kind::Num,
        zero: false,
    };
    let mut use_sqrt = false;
    let mut sqrt_neg = false;
    if x.compare_abs(&one_over_sqrt2.with_prec(work, true)) == Ordering::Greater {
        use_sqrt = true;
        sqrt_neg = x.neg;
        let mut square = product(work, &x, &x);
        square.neg = true;
        let rest = sum(work, &reg(work, &one()), &square);
        x = sqrt_into(work, &rest)?;
    }
    let mut result = asin_series(x);
    let half_pi = |pi: &Big| {
        let mut half = pi.with_prec(work, true);
        half.div_by_long(2);
        half
    };
    if use_sqrt {
        negate(&mut result);
        result = sum(work, &half_pi(&pi), &result);
        if sqrt_neg {
            negate(&mut result);
        }
    }
    if is_acos {
        negate(&mut result);
        result = sum(work, &half_pi(&pi), &result);
    }
    Ok(result.with_prec(prec, true))
}

/// The arctangent of `src`, to its own precision.
pub fn atan(src: &Big, cache: &mut Cache) -> MathResult<Big> {
    let prec = src.prec();
    let pi = cache.pi(prec + 3)?;
    let work = prec + 3;
    if src.exp < -1 || src.exp > 2 {
        // Very small or large: a series in x or in 1/x.
        let (mut total, mut power, mut term_neg) = if src.exp < 0 {
            let mut zero = Big::zero(work);
            zero.exp = 1;
            (zero, reg(work, src), false)
        } else {
            let mut half_pi = pi.with_prec(work, true);
            half_pi.div_by_long(2);
            half_pi.neg = src.neg;
            (half_pi, quotient(work, &one(), src)?, true)
        };
        let square = product(work, &power, &power);
        let mut n: u64 = 1;
        loop {
            let mut term = reg(work, &power);
            term.div_by_long(n);
            if term_neg {
                term.neg = !term.neg;
            }
            if n != 1 && negligible(&total, &term, work) {
                break;
            }
            total = sum(work, &total, &term);
            n += 2;
            term_neg = !term_neg;
            power = product(work, &power, &square);
        }
        Ok(total.with_prec(prec, true))
    } else {
        // atan(x) = acos(1 / sqrt(x^2 + 1)), with the sign of x.
        let mut square = product(work, src, src);
        square.increment_abs();
        let root = sqrt_into(work, &square)?;
        let inverse = quotient(work, &one(), &root)?;
        let mut result = asin_acos_into(prec, &inverse, true, cache)?;
        if src.neg {
            negate(&mut result);
        }
        Ok(result)
    }
}

/// The sine series for a small angle.
fn sin_series(prec: usize, x: Big) -> MathResult<Big> {
    let work = x.prec();
    let mut power = x;
    let mut factorial = reg(work, &one());
    let square = product(work, &power, &power);
    let mut total = reg(work, &power);
    let mut n: u64 = 1;
    let mut neg_term = false;
    loop {
        power = product(work, &power, &square);
        factorial.mul_by_long((n + 1) * (n + 2));
        n += 2;
        neg_term = !neg_term;
        let mut term = quotient(work, &power, &factorial)?;
        if term.zero || total.exp - term.exp > term.prec() as i32 {
            break;
        }
        if neg_term {
            term.neg = !term.neg;
        }
        total = sum(work, &total, &term);
    }
    Ok(total.with_prec(prec, true))
}

/// The cosine series for a small angle.
fn cos_series(prec: usize, x: Big) -> MathResult<Big> {
    let work = x.prec();
    let mut factorial = reg(work, &one());
    factorial.digs[0] = 2;
    let square = product(work, &x, &x);
    let mut power = reg(work, &square);
    let mut total = reg(work, &one());
    let mut n: u64 = 2;
    let mut neg_term = true;
    loop {
        let mut term = quotient(work, &power, &factorial)?;
        if term.zero || total.exp - term.exp > term.prec() as i32 {
            break;
        }
        if neg_term {
            term.neg = !term.neg;
        }
        total = sum(work, &total, &term);
        power = product(work, &power, &square);
        factorial.mul_by_long((n + 1) * (n + 2));
        n += 2;
        neg_term = !neg_term;
    }
    Ok(total.with_prec(prec, true))
}

/// Which trigonometric function to compute.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Trig {
    Sin,
    Cos,
    Tan,
}

/// The sine, cosine, or tangent of `src`, to its own precision.
pub fn trig(function: Trig, src: &Big, cache: &mut Cache) -> MathResult<Big> {
    let prec = src.prec();
    let pi = cache.pi(prec + 3)?;
    let work = prec + 3;
    let pi_reg = pi.with_prec(work, true);
    let mut x = reg(work, src);
    let mut neg_result = function != Trig::Cos && x.neg;
    x.neg = false;

    // Bring the angle into one turn, then into a half and a quarter,
    // using the symmetries of the function.
    let mut two_pi = pi_reg.clone();
    two_pi.mul_by_long(2);
    while x.compare_abs(&two_pi) == Ordering::Greater {
        let mut whole = Big::zero(work);
        let mut rest = Big::zero(work);
        whole
            .quotient_into(Some(&mut rest), &x, &two_pi)
            .ok_or(MathError::DivideByZero)?;
        x = rest;
    }
    if x.compare_abs(&pi_reg) == Ordering::Greater {
        if function != Trig::Tan {
            neg_result = !neg_result;
        }
        x = abs_diff(work, &x, &pi_reg);
    }
    let mut half_pi = pi_reg.clone();
    half_pi.div_by_long(2);
    if x.compare_abs(&half_pi) == Ordering::Greater {
        if function != Trig::Tan {
            neg_result = !neg_result;
        }
        x = abs_diff(work, &x, &pi_reg);
    }
    if function != Trig::Cos && x.neg {
        neg_result = !neg_result;
    }
    x.neg = false;

    // Past an eighth of a turn, use the other function of the
    // complementary angle, whose series converges faster.
    let mut quarter_pi = pi_reg.clone();
    quarter_pi.div_by_long(4);
    let mut complement = false;
    if x.compare_abs(&quarter_pi) == Ordering::Greater {
        complement = true;
        if function == Trig::Cos {
            neg_result = !neg_result;
        }
        let was_neg = x.neg;
        x = abs_diff(work, &x, &half_pi);
        if function != Trig::Sin && was_neg {
            neg_result = !neg_result;
        }
    }

    let mut result = match function {
        Trig::Sin if complement => cos_series(prec, x)?,
        Trig::Sin => sin_series(prec, x)?,
        Trig::Cos if complement => sin_series(prec, x)?,
        Trig::Cos => cos_series(prec, x)?,
        Trig::Tan => {
            let sine = sin_series(work, x.clone())?;
            let cosine = cos_series(work, x)?;
            let mut result = if complement {
                quotient(prec, &cosine, &sine)?
            } else {
                quotient(prec, &sine, &cosine)?
            };
            result.neg = neg_result;
            result.normalize();
            return Ok(result);
        }
    };
    if neg_result {
        result.neg = !result.neg;
    }
    result.normalize();
    Ok(result)
}

/// Convert between degrees and radians, to the number's own precision.
pub fn convert_angle(src: &Big, to_radians: bool, cache: &mut Cache) -> MathResult<Big> {
    let prec = src.prec();
    let pi = cache.pi(prec + 2)?;
    let mut factor = pi.with_prec(prec + 2, true);
    factor.div_by_long(180);
    if to_radians {
        Ok(product(prec, src, &factor))
    } else {
        quotient(prec, src, &factor)
    }
}

/// The base-10 logarithm of `src`, to its own precision.
pub fn log10(src: &Big, cache: &mut Cache) -> MathResult<Big> {
    let prec = src.prec();
    let work = prec + 3;
    let ln10 = cache.ln10(work)?;
    let ln = ln_into(work, src, cache)?;
    let result = quotient(work, &ln, &ln10.with_prec(work, true))?;
    Ok(result.with_prec(prec, true))
}

/// `base` raised to `power`, to the larger of their precisions.
pub fn pow(base: &Big, power: &Big, cache: &mut Cache) -> MathResult<Big> {
    let prec = base.prec().max(power.prec());
    if base.zero {
        if power.zero {
            return Err(MathError::OutOfRange);
        }
        if power.neg {
            return Err(MathError::DivideByZero);
        }
        let mut zero = Big::zero(prec);
        zero.exp = 1;
        return Ok(zero);
    }
    let work = prec + 3;
    let mut result_neg = false;
    let ln = if base.neg {
        // A negative number has only whole powers. The result is
        // negative for an odd one.
        if !power.is_frac_zero() {
            return Err(MathError::OutOfRange);
        }
        let units = if power.exp <= 0 || power.exp as usize > power.prec() {
            0
        } else {
            power.digs[power.exp as usize - 1]
        };
        result_neg = units & 1 != 0;
        let mut positive = reg(work, base);
        positive.neg = false;
        ln_into(work, &positive, cache)?
    } else {
        ln_into(work, base, cache)?
    };
    let exponent = product(work, power, &ln);
    let mut result = exp_into(work, &exponent, cache)?;
    if result_neg {
        negate(&mut result);
    }
    Ok(result.with_prec(prec, true))
}

/// Which hyperbolic function to compute.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hyperbolic {
    Sinh,
    Cosh,
    Tanh,
}

/// A hyperbolic function of `src`, to its own precision.
pub fn hyperbolic(function: Hyperbolic, src: &Big, cache: &mut Cache) -> MathResult<Big> {
    let prec = src.prec();
    let work = prec + 3;
    let grow = exp_into(work, src, cache)?;
    let mut shrink = quotient(work, &reg(work, &one()), &grow)?;
    let result = match function {
        Hyperbolic::Tanh => {
            let denominator = sum(work, &grow, &shrink);
            negate(&mut shrink);
            let numerator = sum(work, &grow, &shrink);
            quotient(work, &numerator, &denominator)?
        }
        _ => {
            if function == Hyperbolic::Sinh {
                negate(&mut shrink);
            }
            let mut total = sum(work, &grow, &shrink);
            total.div_by_long(2);
            total
        }
    };
    Ok(result.with_prec(prec, true))
}
