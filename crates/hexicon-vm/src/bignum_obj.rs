//! The BigNumber intrinsic class: how the VM uses [`Big`] values.

use std::cmp::Ordering;

use crate::bignum::{Big, Kind};
use crate::bignum_math::{self, Hyperbolic, MathError, Trig};
use crate::error::{VmError, VmResult, code};
use crate::object::Object;
use crate::value::{NativeDesc, ObjId, Value};
use crate::vm::Vm;

/// The precision an integer gets when arithmetic mixes it with a
/// BigNumber on the right.
const INT_PREC: usize = 32;

/// The name and calling interface of each method, by method number.
const METHODS: &[(&str, NativeDesc)] = &[
    ("", NativeDesc::fixed(0)),
    ("formatString", NativeDesc::optional(0, 6)),
    ("equalRound", NativeDesc::fixed(1)),
    ("getPrecision", NativeDesc::fixed(0)),
    ("setPrecision", NativeDesc::fixed(1)),
    ("getFraction", NativeDesc::fixed(0)),
    ("getWhole", NativeDesc::fixed(0)),
    ("roundToDecimal", NativeDesc::fixed(1)),
    ("getAbs", NativeDesc::fixed(0)),
    ("getCeil", NativeDesc::fixed(0)),
    ("getFloor", NativeDesc::fixed(0)),
    ("getScale", NativeDesc::fixed(0)),
    ("scaleTen", NativeDesc::fixed(1)),
    ("negate", NativeDesc::fixed(0)),
    ("copySignFrom", NativeDesc::fixed(1)),
    ("isNegative", NativeDesc::fixed(0)),
    ("divideBy", NativeDesc::fixed(1)),
    ("sine", NativeDesc::fixed(0)),
    ("cosine", NativeDesc::fixed(0)),
    ("tangent", NativeDesc::fixed(0)),
    ("degreesToRadians", NativeDesc::fixed(0)),
    ("radiansToDegrees", NativeDesc::fixed(0)),
    ("arcsine", NativeDesc::fixed(0)),
    ("arccosine", NativeDesc::fixed(0)),
    ("arctangent", NativeDesc::fixed(0)),
    ("sqrt", NativeDesc::fixed(0)),
    ("logE", NativeDesc::fixed(0)),
    ("expE", NativeDesc::fixed(0)),
    ("log10", NativeDesc::fixed(0)),
    ("raiseToPower", NativeDesc::fixed(1)),
    ("sinh", NativeDesc::fixed(0)),
    ("cosh", NativeDesc::fixed(0)),
    ("tanh", NativeDesc::fixed(0)),
    // `getPi` and `getE` are static methods.
    ("getPi", NativeDesc::fixed(1)),
    ("getE", NativeDesc::fixed(1)),
    ("numType", NativeDesc::fixed(0)),
];

/// Turn a failed calculation into the error a program sees.
fn math<T>(result: Result<T, MathError>) -> VmResult<T> {
    result.map_err(|error| {
        VmError::new(match error {
            MathError::OutOfRange => code::OUT_OF_RANGE,
            MathError::Overflow => code::NUM_OVERFLOW,
            MathError::DivideByZero => code::DIVIDE_BY_ZERO,
        })
    })
}

/// An arithmetic operator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Arith {
    Add,
    Sub,
    Mul,
    Div,
}

impl Vm {
    /// Create a BigNumber object.
    pub(crate) fn new_big(&mut self, value: Big) -> Value {
        Value::Obj(self.alloc_obj(Object::BigNumber(Box::new(value))))
    }

    /// The number a value holds, if it is a BigNumber object.
    pub(crate) fn as_big(&self, value: Value) -> Option<&Big> {
        match value {
            Value::Obj(id) => match self.objects.get(id)? {
                Object::BigNumber(big) => Some(big),
                _ => None,
            },
            _ => None,
        }
    }

    /// The number a value holds, converting an integer.
    fn to_big(&self, value: Value) -> Option<Big> {
        match value {
            Value::Int(n) => Some(Big::from_int(n.into(), INT_PREC)),
            other => self.as_big(other).cloned(),
        }
    }

    /// Arithmetic with a BigNumber as the left operand. The right one
    /// can be a BigNumber or an integer.
    /// Note an operand of BigNumber arithmetic or comparison. The
    /// reference VM turns an integer operand into a BigNumber object
    /// before it uses it, and a program can tell that an object was
    /// created, so one is created here too.
    pub(crate) fn big_operand(&mut self, operand: Value) {
        if let Value::Int(n) = operand {
            self.new_big(Big::from_int(n.into(), INT_PREC));
        }
    }

    pub(crate) fn big_arith(&mut self, op: Arith, a: &Big, b: Value) -> VmResult<Value> {
        self.big_operand(b);
        let Some(b) = self.to_big(b) else {
            return Err(VmError::new(match op {
                Arith::Sub => code::BAD_TYPE_SUB,
                _ => code::BAD_TYPE_ADD,
            }));
        };
        let result = match op {
            Arith::Add => Big::add(a, &b),
            Arith::Sub => Big::sub(a, &b),
            Arith::Mul => Big::mul(a, &b),
            Arith::Div => match Big::div(a, &b) {
                Some(result) => result,
                None => return Err(VmError::new(code::DIVIDE_BY_ZERO)),
            },
        };
        Ok(self.new_big(result))
    }

    /// Integer arithmetic whose result does not fit in 32 bits: the
    /// result is a BigNumber instead.
    pub(crate) fn promote_int_arith(&mut self, op: Arith, a: i32, b: i32) -> VmResult<Value> {
        // Sums and quotients get ten digits, products twenty.
        let prec = if op == Arith::Mul { 20 } else { 10 };
        let (a, b) = (Big::from_int(a.into(), prec), Big::from_int(b.into(), prec));
        let result = match op {
            Arith::Add => Big::add(&a, &b),
            Arith::Sub => Big::sub(&a, &b),
            Arith::Mul => Big::mul(&a, &b),
            Arith::Div => Big::div(&a, &b).ok_or_else(|| VmError::new(code::DIVIDE_BY_ZERO))?,
        };
        Ok(self.new_big(result))
    }

    /// The negation of a BigNumber object.
    pub(crate) fn big_neg(&mut self, id: ObjId, a: &Big) -> Value {
        if a.kind == Kind::NaN || a.zero {
            return Value::Obj(id);
        }
        let mut result = a.clone();
        result.neg = !result.neg;
        self.new_big(result)
    }

    /// Whether a BigNumber equals another value.
    pub(crate) fn big_equals(&self, a: &Big, other: Value) -> bool {
        self.to_big(other).is_some_and(|b| a.equals_exact(&b))
    }

    /// Compare a BigNumber with another value.
    pub(crate) fn big_compare(&self, a: &Big, other: Value) -> VmResult<Ordering> {
        self.to_big(other)
            .and_then(|b| a.compare(&b))
            .ok_or_else(|| VmError::new(code::INVALID_COMPARISON))
    }

    /// Convert a BigNumber to an integer, rounding. A value out of range
    /// is an error.
    pub(crate) fn big_to_int(a: &Big) -> VmResult<i32> {
        match a.to_i32() {
            (value, false) => Ok(value),
            (_, true) => Err(VmError::new(code::NUM_OVERFLOW)),
        }
    }

    /// Construct a BigNumber from the arguments to `new`: a value and an
    /// optional precision.
    pub(crate) fn create_big(&mut self, argc: u32) -> VmResult<ObjId> {
        if !(1..=2).contains(&argc) {
            return Err(VmError::new(code::WRONG_NUM_OF_ARGS));
        }
        let source = self.arg(0);
        let text = self
            .as_str(source)
            .map(|t| String::from_utf8_lossy(&t).into_owned());
        let existing = self.as_big(source).cloned();
        if !matches!(source, Value::Int(_)) && text.is_none() && existing.is_none() {
            return Err(VmError::new(code::NUM_VAL_REQD));
        }
        let digits = if argc > 1 {
            match self.arg(1) {
                Value::Int(n) => n.max(0) as usize,
                _ => return Err(VmError::new(code::INT_VAL_REQD)),
            }
        } else if let Some(text) = &text {
            Big::precision_from_str(text)
        } else if let Some(existing) = &existing {
            existing.prec()
        } else {
            32
        };
        let value = match (source, text, existing) {
            (_, Some(text), _) => Big::parse(&text, digits),
            (_, _, Some(existing)) => existing.with_prec(digits, true),
            (Value::Int(n), _, _) => Big::from_int(n.into(), digits),
            _ => return Err(VmError::new(code::NUM_VAL_REQD)),
        };
        self.discard(argc as usize);
        Ok(self.alloc_obj(Object::BigNumber(Box::new(value))))
    }

    /// Call a method of the BigNumber class.
    pub(crate) fn big_method(
        &mut self,
        id: ObjId,
        index: u16,
        argc: Option<&mut u32>,
    ) -> VmResult<Option<Value>> {
        if index == 0 || index == 33 || index == 34 {
            return Ok(None);
        }
        let Some(&(_, desc)) = METHODS.get(index as usize) else {
            return Ok(None);
        };
        self.native_method(argc, desc, |vm, argc| vm.big_call(id, index, argc))
            .map(Some)
    }

    /// The static methods of the BigNumber class: `getPi(digits)` and
    /// `getE(digits)`.
    pub(crate) fn big_static_method(
        &mut self,
        index: u16,
        argc: Option<&mut u32>,
    ) -> VmResult<Option<Value>> {
        if index != 33 && index != 34 {
            return Ok(None);
        }
        self.native_method(argc, NativeDesc::fixed(1), |vm, _| {
            let digits = vm.int_arg(vm.arg(0))?.max(0) as usize;
            let constant = if index == 33 {
                vm.big_cache.pi(digits)
            } else {
                vm.big_cache.e(digits)
            };
            let value = math(constant)?.with_prec(digits, true);
            Ok(vm.new_big(value))
        })
        .map(Some)
    }

    /// An optional integer argument: nil or a missing argument gives the
    /// default.
    fn int_or(&self, argc: u32, index: u32, default: i32) -> VmResult<i32> {
        if index >= argc {
            return Ok(default);
        }
        match self.arg(index as usize) {
            Value::Nil => Ok(default),
            Value::Int(n) => Ok(n),
            _ => Err(VmError::new(code::INT_VAL_REQD)),
        }
    }

    fn big_call(&mut self, id: ObjId, index: u16, argc: u32) -> VmResult<Value> {
        let self_value = Value::Obj(id);
        let Some(a) = self.as_big(self_value).cloned() else {
            return Err(VmError::new(code::BAD_TYPE_BIF));
        };
        // Most methods of something that is not a number return it as is.
        let nan = a.kind == Kind::NaN;
        // The other operand of a two-number method.
        let other = |vm: &Vm| {
            vm.to_big(vm.arg(0))
                .ok_or_else(|| VmError::new(code::BAD_TYPE_BIF))
        };
        Ok(match index {
            // formatString(maxDigits?, flags?, wholePlaces?, fracDigits?,
            // expDigits?, leadFiller?)
            1 => {
                let max_digits = self.int_or(argc, 0, -1)?;
                let flags = self.int_or(argc, 1, 0)? as u32;
                let whole_places = self.int_or(argc, 2, -1)?;
                let frac_digits = self.int_or(argc, 3, -1)?;
                let exp_digits = self.int_or(argc, 4, -1)?;
                let fill = if argc >= 6 && !self.arg(5).is_nil() {
                    let text = self.require_str(self.arg(5))?;
                    Some(String::from_utf8_lossy(&text).into_owned()).filter(|t| !t.is_empty())
                } else {
                    None
                };
                let text = a.format(
                    max_digits,
                    whole_places,
                    frac_digits,
                    exp_digits,
                    flags,
                    fill.as_deref(),
                );
                self.try_new_str(&text)?
            }
            // equalRound(num)
            2 => match self.to_big(self.arg(0)) {
                Some(b) => Value::logical(a.equals_rounded(&b)),
                None => Value::Nil,
            },
            // getPrecision()
            3 => Value::Int(a.prec() as i32),
            // setPrecision(digits)
            4 => {
                let digits = self.int_arg(self.arg(0))?.max(0) as usize;
                if nan {
                    self_value
                } else {
                    self.new_big(a.with_prec(digits, true))
                }
            }
            // getFraction() and getWhole()
            5 | 6 if nan => self_value,
            5 => {
                let mut result = a;
                result.keep_frac();
                self.new_big(result)
            }
            6 => {
                let mut result = a;
                result.keep_whole();
                self.new_big(result)
            }
            // roundToDecimal(places)
            7 => {
                let places = self.int_arg(self.arg(0))?;
                if nan {
                    return Ok(self_value);
                }
                let mut result = a;
                let keep = places.saturating_add(result.exp);
                if keep < 0 {
                    result = Big::zero(result.prec());
                    result.exp = 1;
                } else if (keep as usize) < result.prec() {
                    result.round_to(keep);
                    result.normalize();
                }
                self.new_big(result)
            }
            // getAbs()
            8 => {
                if !a.neg || nan {
                    self_value
                } else {
                    let mut result = a;
                    result.neg = false;
                    self.new_big(result)
                }
            }
            // getCeil() and getFloor()
            9 | 10 => {
                if nan {
                    return Ok(self_value);
                }
                // Rounding goes away from zero for a positive ceiling
                // and a negative floor.
                let away = a.neg == (index == 10);
                let frac_zero = a.is_frac_zero();
                let mut result = a;
                if result.exp <= 0 {
                    let neg = result.neg;
                    if !away || frac_zero {
                        result = Big::zero(result.prec());
                        result.exp = 1;
                    } else {
                        result = Big::from_int(1, result.prec());
                        result.neg = neg;
                    }
                } else {
                    let whole = result.exp as usize;
                    if whole < result.prec() {
                        result.digs[whole..].fill(0);
                    }
                    if !frac_zero && away {
                        result.increment_abs();
                    }
                }
                self.new_big(result)
            }
            // getScale()
            11 => {
                if nan {
                    Value::Nil
                } else {
                    Value::Int(a.exp)
                }
            }
            // scaleTen(n)
            12 => {
                let scale = self.int_arg(self.arg(0))?;
                if nan {
                    return Ok(self_value);
                }
                let mut result = a;
                result.exp = result.exp.saturating_add(scale);
                self.new_big(result)
            }
            // negate()
            13 => self.big_neg(id, &a),
            // copySignFrom(num)
            14 => {
                let b = other(self)?;
                if b.kind == Kind::NaN {
                    return Ok(self.arg(0));
                }
                if nan {
                    return Ok(self_value);
                }
                let mut result = a;
                result.neg = b.neg;
                result.normalize();
                self.new_big(result)
            }
            // isNegative()
            15 => {
                if nan {
                    Value::Nil
                } else {
                    Value::logical(a.neg)
                }
            }
            // divideBy(num): the whole quotient and the remainder.
            16 => {
                let b = other(self)?;
                let Some((quotient, remainder)) = Big::div_rem(&a, &b) else {
                    return Err(VmError::new(code::DIVIDE_BY_ZERO));
                };
                let quotient = self.new_big(quotient);
                let remainder = self.new_big(remainder);
                self.new_list(vec![quotient, remainder])
            }
            // sine(), cosine(), tangent()
            17..=19 => {
                if nan {
                    return Ok(self_value);
                }
                let function = match index {
                    17 => Trig::Sin,
                    18 => Trig::Cos,
                    _ => Trig::Tan,
                };
                let result = math(bignum_math::trig(function, &a, &mut self.big_cache))?;
                self.new_big(result)
            }
            // degreesToRadians() and radiansToDegrees()
            20 | 21 => {
                if nan {
                    return Ok(self_value);
                }
                let result = math(bignum_math::convert_angle(
                    &a,
                    index == 20,
                    &mut self.big_cache,
                ))?;
                self.new_big(result)
            }
            // arcsine() and arccosine()
            22 | 23 => {
                if nan {
                    return Ok(self_value);
                }
                let result = math(bignum_math::asin_acos_into(
                    a.prec(),
                    &a,
                    index == 23,
                    &mut self.big_cache,
                ))?;
                self.new_big(result)
            }
            // arctangent()
            24 => {
                if nan {
                    return Ok(self_value);
                }
                let result = math(bignum_math::atan(&a, &mut self.big_cache))?;
                self.new_big(result)
            }
            // sqrt()
            25 => {
                if nan {
                    return Ok(self_value);
                }
                let result = math(bignum_math::sqrt_into(a.prec(), &a))?;
                self.new_big(result)
            }
            // logE()
            26 => {
                if a.zero || a.neg {
                    return Err(VmError::new(code::OUT_OF_RANGE));
                }
                if nan {
                    return Ok(self_value);
                }
                let result = math(bignum_math::ln_into(a.prec(), &a, &mut self.big_cache))?;
                self.new_big(result)
            }
            // expE()
            27 => {
                if nan {
                    return Ok(self_value);
                }
                let result = math(bignum_math::exp_into(a.prec(), &a, &mut self.big_cache))?;
                self.new_big(result)
            }
            // log10()
            28 => {
                if nan {
                    return Ok(self_value);
                }
                let result = math(bignum_math::log10(&a, &mut self.big_cache))?;
                self.new_big(result)
            }
            // raiseToPower(num)
            29 => {
                let b = other(self)?;
                if b.kind == Kind::NaN {
                    return Ok(self.arg(0));
                }
                if nan {
                    return Ok(self_value);
                }
                let result = math(bignum_math::pow(&a, &b, &mut self.big_cache))?;
                self.new_big(result)
            }
            // sinh(), cosh(), tanh()
            30..=32 => {
                if nan {
                    return Ok(self_value);
                }
                let function = match index {
                    30 => Hyperbolic::Sinh,
                    31 => Hyperbolic::Cosh,
                    _ => Hyperbolic::Tanh,
                };
                let result = math(bignum_math::hyperbolic(function, &a, &mut self.big_cache))?;
                self.new_big(result)
            }
            // numType()
            35 => Value::Int(match a.kind {
                Kind::Num if a.zero && a.neg => 0x0001 | 0x0020,
                Kind::Num if a.zero => 0x0001 | 0x0010,
                Kind::Num => 0x0001,
                Kind::Inf if a.neg => 0x0008,
                Kind::Inf => 0x0004,
                Kind::NaN => 0x0002,
            }),
            _ => {
                return Err(VmError::unimplemented(format!(
                    "BigNumber.{}",
                    METHODS[index as usize].0
                )));
            }
        })
    }
}
