//! Working out constant expressions when the program is compiled.
//!
//! These are the reference compiler's rules, operator by operator. They
//! are applied twice: as each operator is parsed, and again once the
//! whole program has been read and names have been looked up, when only
//! some operators are tried again.

use crate::ast::{BinOp, ConstVal, Expr, UnOp, Value};

/// What came of trying to fold an operator.
pub(crate) enum Folded {
    /// The operator is gone; this is what is left.
    Yes(Expr),
    /// The operator stays, with these operands.
    No(Expr, Expr),
}

/// The operator as messages name it.
fn op_text(op: BinOp) -> &'static str {
    match op {
        BinOp::Comma => ",",
        BinOp::Or => "||",
        BinOp::And => "&&",
        BinOp::BOr => "|",
        BinOp::BAnd => "&",
        BinOp::BXor => "^",
        BinOp::Eq => "==",
        BinOp::Ne => "!=",
        BinOp::Gt => ">",
        BinOp::Ge => ">=",
        BinOp::Lt => "<",
        BinOp::Le => "<=",
        BinOp::Shl => "<<",
        BinOp::AShr => ">>",
        BinOp::LShr => ">>>",
        BinOp::Mul => "*",
        BinOp::Div => "/",
        BinOp::Mod => "%",
        BinOp::Sub => "-",
        BinOp::Add => "+",
    }
}

fn both_const(left: &Expr, right: &Expr) -> bool {
    left.is_const() && right.is_const()
}

fn into_const(expr: Expr) -> ConstVal {
    match expr {
        Expr::Const(value) => value,
        _ => unreachable!("checked to be a constant"),
    }
}

/// Make the result of a folded `&&` or `||` a truth value.
fn boolized(expr: Expr) -> Expr {
    match expr {
        Expr::Const(mut value) => {
            let truth = value.is_true();
            value.set_bool(truth);
            Expr::Const(value)
        }
        other => Expr::Unary(UnOp::Boolize, Box::new(other)),
    }
}

/// Try to fold a two-operand operator. Problems go to `errors`.
pub(crate) fn binary(op: BinOp, left: Expr, right: Expr, errors: &mut Vec<String>) -> Folded {
    match op {
        BinOp::Comma => {
            // The left side still has to be evaluated unless it is a
            // constant too.
            if both_const(&left, &right) {
                Folded::Yes(right)
            } else {
                Folded::No(left, right)
            }
        }
        BinOp::Or => match left.as_const().map(ConstVal::is_true) {
            Some(true) => Folded::Yes(boolized(left)),
            Some(false) => Folded::Yes(boolized(right)),
            None => Folded::No(left, right),
        },
        BinOp::And => match left.as_const().map(ConstVal::is_true) {
            Some(true) => Folded::Yes(boolized(right)),
            Some(false) => Folded::Yes(boolized(left)),
            None => Folded::No(left, right),
        },
        BinOp::Gt | BinOp::Ge | BinOp::Lt | BinOp::Le => relation(op, left, right, errors),
        BinOp::Eq | BinOp::Ne => {
            if !both_const(&left, &right) {
                return Folded::No(left, right);
            }
            let equal = into_const(right).value == left.as_const().expect("constant").value;
            let mut result = into_const(left);
            result.set_bool(equal == (op == BinOp::Eq));
            Folded::Yes(Expr::Const(result))
        }
        BinOp::BOr
        | BinOp::BAnd
        | BinOp::BXor
        | BinOp::Shl
        | BinOp::AShr
        | BinOp::LShr
        | BinOp::Mul
        | BinOp::Div
        | BinOp::Mod => arithmetic(op, left, right, errors),
        BinOp::Sub => {
            if !both_const(&left, &right) {
                return Folded::No(left, right);
            }
            if is_int(&left) && is_int(&right) {
                arithmetic(op, left, right, errors)
            } else {
                errors.push("incompatible constant types for two-operand '-' operator".into());
                Folded::No(left, right)
            }
        }
        BinOp::Add => add(left, right, errors),
    }
}

fn is_int(expr: &Expr) -> bool {
    matches!(
        expr.as_const(),
        Some(ConstVal {
            value: Value::Int(_),
            ..
        })
    )
}

fn is_str(expr: &Expr) -> bool {
    matches!(
        expr.as_const(),
        Some(ConstVal {
            value: Value::SStr(_),
            ..
        })
    )
}

fn relation(op: BinOp, left: Expr, right: Expr, errors: &mut Vec<String>) -> Folded {
    use std::cmp::Ordering;
    if !both_const(&left, &right) {
        return Folded::No(left, right);
    }
    let order = match (
        &left.as_const().expect("constant").value,
        &right.as_const().expect("constant").value,
    ) {
        (Value::Int(a), Value::Int(b)) => a.cmp(b),
        (Value::SStr(a), Value::SStr(b)) => c_string(a).cmp(c_string(b)),
        _ => {
            errors.push("incompatible types for comparison in constant expression".into());
            return Folded::No(left, right);
        }
    };
    let truth = match op {
        BinOp::Gt => order == Ordering::Greater,
        BinOp::Ge => order != Ordering::Less,
        BinOp::Lt => order == Ordering::Less,
        _ => order != Ordering::Greater,
    };
    let mut result = into_const(left);
    result.set_bool(truth);
    Folded::Yes(Expr::Const(result))
}

/// A string as C's `strcmp` sees it: up to the first zero byte.
fn c_string(text: &[u8]) -> &[u8] {
    let end = text
        .iter()
        .position(|&byte| byte == 0)
        .unwrap_or(text.len());
    &text[..end]
}

fn arithmetic(op: BinOp, left: Expr, right: Expr, errors: &mut Vec<String>) -> Folded {
    if !both_const(&left, &right) {
        return Folded::No(left, right);
    }
    let (a, b) = match (
        &left.as_const().expect("constant").value,
        &right.as_const().expect("constant").value,
    ) {
        (Value::Int(a), Value::Int(b)) => (*a, *b),
        _ => {
            errors.push(format!(
                "binary '{}' operator requires numeric value in constant expression",
                op_text(op)
            ));
            return Folded::No(left, right);
        }
    };
    // Shifts take the count modulo 32, as the processor the reference
    // runs on does.
    let result = match op {
        BinOp::BOr => a | b,
        BinOp::BAnd => a & b,
        BinOp::BXor => a ^ b,
        BinOp::Shl => a.wrapping_shl(b as u32),
        BinOp::AShr => a.wrapping_shr(b as u32),
        BinOp::LShr => (a as u32).wrapping_shr(b as u32) as i32,
        // The reference built for Windows multiplies in 32 bits and so
        // never sees a product overflow; built elsewhere, it turns the
        // product into a floating-point number, as it does everywhere
        // with a sum or a difference that overflows.
        BinOp::Mul => a.wrapping_mul(b),
        BinOp::Add | BinOp::Sub => {
            let exact = if op == BinOp::Add {
                a.checked_add(b)
            } else {
                a.checked_sub(b)
            };
            let Some(exact) = exact else {
                errors.push(
                    "a number too large for an integer is not supported by this compiler yet"
                        .into(),
                );
                return Folded::No(left, right);
            };
            exact
        }
        BinOp::Div | BinOp::Mod => {
            if b == 0 {
                errors.push("divide by zero in constant expression".into());
                1
            } else if op == BinOp::Div {
                a.wrapping_div(b)
            } else {
                a.wrapping_rem(b)
            }
        }
        _ => unreachable!("not an arithmetic operator"),
    };
    let mut value = into_const(left);
    value.value = Value::Int(result);
    Folded::Yes(Expr::Const(value))
}

/// The text a constant turns into when it is added to a string, if it
/// has one.
fn as_text(value: &Value) -> Option<Vec<u8>> {
    Some(match value {
        Value::Nil => b"nil".to_vec(),
        Value::True => b"true".to_vec(),
        Value::Int(int) => int.to_string().into_bytes(),
        Value::SStr(text) => text.clone(),
        Value::FuncPtr(_) => return None,
    })
}

fn add(left: Expr, right: Expr, errors: &mut Vec<String>) -> Folded {
    if !both_const(&left, &right) {
        return Folded::No(left, right);
    }
    if is_int(&left) && is_int(&right) {
        return arithmetic(BinOp::Add, left, right, errors);
    }
    if is_str(&left) || is_str(&right) {
        let right_value = &right.as_const().expect("constant").value;
        let (Some(mut text), Some(right_text)) = (
            as_text(&left.as_const().expect("constant").value),
            as_text(right_value),
        ) else {
            return Folded::No(left, right);
        };
        // Adding nil adds nothing.
        if *right_value != Value::Nil {
            text.extend_from_slice(&right_text);
        }
        let mut value = into_const(left);
        value.value = Value::SStr(text);
        return Folded::Yes(Expr::Const(value));
    }
    errors.push("incompatible constant types for two-operand '+' operator".into());
    Folded::No(left, right)
}

/// `!` of a constant.
pub(crate) fn not(sub: Expr) -> Result<Expr, Expr> {
    match sub {
        Expr::Const(mut value) => {
            let truth = value.is_true();
            value.set_bool(!truth);
            Ok(Expr::Const(value))
        }
        other => Err(other),
    }
}
