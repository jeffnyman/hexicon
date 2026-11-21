//! The interpreter loop: one arm per instruction.
//!
//! The semantics follow the reference interpreter (`vmrun.cpp`) closely,
//! including behavior the instruction set document leaves open.

use std::cmp::Ordering;

use hexicon_bytecode::Opcode;

use crate::bignum_obj::Arith;
use crate::error::{ErrParam, VmError, VmResult, code};
use crate::meta::MetaKind;
use crate::object::Object;
use crate::value::{ObjId, PropId, Value};
use crate::vm::{Return, Vm};

/// `nil` and zero are false in a condition.
#[inline]
fn is_false(value: Value) -> bool {
    matches!(value, Value::Nil | Value::Int(0))
}

/// The values the short-circuit `||` treats as true.
#[inline]
fn is_true(value: Value) -> bool {
    match value {
        Value::True | Value::Enum(_) => true,
        Value::Int(n) => n != 0,
        _ => false,
    }
}

impl Vm {
    /// Execute instructions from `start_pc` until the function that is on
    /// top of the frame stack at entry returns.
    pub(crate) fn execute(&mut self, start_pc: u32) -> VmResult<()> {
        let code_rc = self.code.clone();
        let code: &[u8] = &code_rc;
        let mut pc = start_pc as usize;
        let mut base = self.frames.last().map_or(0, |f| f.base as usize);
        // An argument count supplied by a `varargc` prefix.
        let mut argc_override: Option<u32> = None;

        macro_rules! reload {
            () => {
                base = self.frames.last().map_or(0, |f| f.base as usize)
            };
        }
        macro_rules! byte {
            () => {{
                let v = code[pc];
                pc += 1;
                v
            }};
        }
        macro_rules! word {
            () => {{
                let v = u16::from_le_bytes([code[pc], code[pc + 1]]);
                pc += 2;
                v
            }};
        }
        macro_rules! dword {
            () => {{
                let v = u32::from_le_bytes([code[pc], code[pc + 1], code[pc + 2], code[pc + 3]]);
                pc += 4;
                v
            }};
        }
        // The target of the branch operand at `pc`.
        macro_rules! target {
            () => {
                (pc as isize + i16::from_le_bytes([code[pc], code[pc + 1]]) as isize) as usize
            };
        }
        macro_rules! branch_if {
            ($cond:expr) => {
                if $cond {
                    pc = target!();
                } else {
                    pc += 2;
                }
            };
        }
        // An 8-bit argument count, unless a `varargc` prefix supplied one.
        macro_rules! argc {
            () => {
                match argc_override.take() {
                    Some(n) => {
                        pc += 1;
                        n
                    }
                    None => u32::from(byte!()),
                }
            };
        }
        macro_rules! argc16 {
            () => {
                match argc_override.take() {
                    Some(n) => {
                        pc += 2;
                        n
                    }
                    None => u32::from(word!()),
                }
            };
        }
        macro_rules! local {
            ($n:expr) => {
                self.stack[base + $n as usize]
            };
        }
        macro_rules! param {
            ($n:expr) => {
                self.stack[base - 1 - $n as usize]
            };
        }
        // Continue at an address returned by a call or return.
        macro_rules! jump {
            ($pc:expr) => {{
                pc = $pc as usize;
                reload!();
            }};
        }
        macro_rules! ret {
            () => {
                match self.pop_frame() {
                    Some(next) => jump!(next),
                    None => return Ok(()),
                }
            };
        }
        // Evaluate a property with the same value as target and `self`.
        macro_rules! get_prop {
            ($target:expr, $prop:expr, $argc:expr) => {{
                let target: Value = $target;
                let next = self.eval_prop(
                    target,
                    $prop,
                    target,
                    $argc,
                    Return::Normal,
                    pc as u32,
                    None,
                )?;
                jump!(next);
            }};
        }
        // Apply an overloaded binary operator: the left operand is under
        // the right one on the stack.
        macro_rules! overload_binary {
            ($prop:ident, $error:ident) => {{
                let right = self.pop();
                let left = self.pop();
                self.push(right);
                let next = self.op_overload(
                    pc as u32,
                    None,
                    left,
                    self.predef.$prop,
                    1,
                    code::$error,
                    Return::Normal,
                )?;
                jump!(next);
            }};
        }
        // A binary operator defined only for integers.
        macro_rules! int_binary {
            ($prop:ident, $error:ident, |$a:ident, $b:ident| $result:expr) => {{
                if let (Value::Int($a), Value::Int($b)) = (self.top(1), self.top(0)) {
                    self.discard(1);
                    *self.top_mut(0) = Value::Int($result);
                } else {
                    overload_binary!($prop, $error);
                }
            }};
        }
        // Add to a local, for the compound assignment instructions.
        macro_rules! add_to_local {
            ($index:expr, $operand:expr, $prop:ident, $error:ident, $int_op:ident, $op:ident) => {{
                let index = $index as usize;
                let operand: Value = $operand;
                match (local!(index), operand) {
                    (Value::Int(a), Value::Int(b)) => {
                        local!(index) = self.$int_op(a, b)?;
                    }
                    (current, _) => match self.$op(current, operand)? {
                        Some(result) => local!(index) = result,
                        None => {
                            self.push(operand);
                            let next = self.op_overload(
                                pc as u32,
                                Some(index as u16),
                                current,
                                self.predef.$prop,
                                1,
                                code::$error,
                                Return::Normal,
                            )?;
                            jump!(next);
                        }
                    },
                }
            }};
        }

        loop {
            // After a `varargc` prefix, the instruction it modifies is
            // still part of the same instruction for error reporting.
            if argc_override.is_none() {
                self.pc = pc as u32;
                self.io.varargc = None;
                self.objects.at_safe_point(self.run_depth);
            }
            let Some(&byte) = code.get(pc) else {
                return Err(VmError::new(code::INVALID_OPCODE));
            };
            let Some(opcode) = Opcode::from_byte(byte) else {
                return Err(VmError::new(code::INVALID_OPCODE));
            };
            pc += 1;
            match opcode {
                // ---- pushing constants ----
                Opcode::Push0 => self.push(Value::Int(0)),
                Opcode::Push1 => self.push(Value::Int(1)),
                Opcode::PushInt8 => {
                    let n = byte!() as i8;
                    self.push(Value::Int(n.into()));
                }
                Opcode::PushInt => {
                    let n = dword!() as i32;
                    self.push(Value::Int(n));
                }
                Opcode::PushStr => {
                    let offset = dword!();
                    self.push(Value::SString(offset));
                }
                Opcode::PushLst => {
                    let offset = dword!();
                    self.push(Value::List(offset));
                }
                Opcode::PushObj => {
                    let id = dword!();
                    self.push(Value::Obj(ObjId(id)));
                }
                Opcode::PushNil => self.push(Value::Nil),
                Opcode::PushTrue => self.push(Value::True),
                Opcode::PushPropId => {
                    let prop = word!();
                    self.push(Value::Prop(prop));
                }
                Opcode::PushFnPtr => {
                    let func = dword!();
                    self.push(Value::FuncPtr(func));
                }
                Opcode::PushStrI => {
                    let len = word!() as usize;
                    let Some(text) = code.get(pc..pc + len) else {
                        return Err(VmError::new(code::INVALID_OPCODE));
                    };
                    pc += len;
                    let string = self.new_string(text);
                    self.push(string);
                }
                Opcode::PushParLst => {
                    let fixed = usize::from(byte!());
                    let argc = self.frames.last().map_or(0, |f| f.argc as usize);
                    let items: Vec<Value> = (fixed..argc).map(|i| param!(i)).collect();
                    let list = self.new_list(items);
                    self.push(list);
                }
                Opcode::MakeLstPar => self.make_list_params()?,
                Opcode::PushEnum => {
                    let n = dword!();
                    self.push(Value::Enum(n));
                }
                Opcode::PushBifPtr => {
                    let index = word!();
                    let set = word!();
                    if self.bif_desc(set, index).is_none() {
                        return Err(VmError::with(
                            code::UNAVAIL_INTRINSIC,
                            vec![
                                ErrParam::Int(index.into()),
                                ErrParam::Str(format!("#{set}")),
                            ],
                        ));
                    }
                    self.push(Value::BifPtr { set, index });
                }

                // ---- arithmetic ----
                Opcode::Neg => match self.top(0) {
                    Value::Int(n) => *self.top_mut(0) = self.int_neg(n)?,
                    operand if self.as_big(operand).is_some() => {
                        let big = self.as_big(operand).cloned().expect("checked above");
                        *self.top_mut(0) = self.big_neg(operand.obj_id(), &big);
                    }
                    operand => {
                        self.pop();
                        let next = self.op_overload(
                            pc as u32,
                            None,
                            operand,
                            self.predef.operator_neg,
                            0,
                            code::BAD_TYPE_NEG,
                            Return::Normal,
                        )?;
                        jump!(next);
                    }
                },
                Opcode::BNot => match self.top(0) {
                    Value::Int(n) => *self.top_mut(0) = Value::Int(!n),
                    operand => {
                        self.pop();
                        let next = self.op_overload(
                            pc as u32,
                            None,
                            operand,
                            self.predef.operator_bit_not,
                            0,
                            code::BAD_TYPE_BIT_NOT,
                            Return::Normal,
                        )?;
                        jump!(next);
                    }
                },
                Opcode::Add => match (self.top(1), self.top(0)) {
                    (Value::Int(a), Value::Int(b)) => {
                        self.discard(1);
                        *self.top_mut(0) = self.int_add(a, b)?;
                    }
                    (a, b) => match self.add(a, b)? {
                        Some(result) => {
                            self.discard(1);
                            *self.top_mut(0) = result;
                        }
                        None => overload_binary!(operator_add, BAD_TYPE_ADD),
                    },
                },
                Opcode::Sub => match (self.top(1), self.top(0)) {
                    (Value::Int(a), Value::Int(b)) => {
                        self.discard(1);
                        *self.top_mut(0) = self.int_sub(a, b)?;
                    }
                    (a, b) => match self.sub(a, b)? {
                        Some(result) => {
                            self.discard(1);
                            *self.top_mut(0) = result;
                        }
                        None => overload_binary!(operator_sub, BAD_TYPE_SUB),
                    },
                },
                Opcode::Mul => match (self.top(1), self.top(0)) {
                    (Value::Int(a), Value::Int(b)) => {
                        self.discard(1);
                        *self.top_mut(0) = self.int_mul(a, b)?;
                    }
                    (a, b) => match self.mul_div(Arith::Mul, a, b)? {
                        Some(result) => {
                            self.discard(1);
                            *self.top_mut(0) = result;
                        }
                        None => overload_binary!(operator_mul, BAD_TYPE_MUL),
                    },
                },
                Opcode::Div => match (self.top(1), self.top(0)) {
                    (Value::Int(a), Value::Int(b)) => {
                        let result = self.int_div(a, b)?;
                        self.discard(1);
                        *self.top_mut(0) = result;
                    }
                    (a, b) => match self.mul_div(Arith::Div, a, b)? {
                        Some(result) => {
                            self.discard(1);
                            *self.top_mut(0) = result;
                        }
                        None => overload_binary!(operator_div, BAD_TYPE_DIV),
                    },
                },
                Opcode::Mod => {
                    if let (Value::Int(a), Value::Int(b)) = (self.top(1), self.top(0)) {
                        if b == 0 {
                            return Err(VmError::new(code::DIVIDE_BY_ZERO));
                        }
                        self.discard(1);
                        *self.top_mut(0) = Value::Int(a.wrapping_rem(b));
                    } else {
                        overload_binary!(operator_mod, BAD_TYPE_MOD);
                    }
                }
                Opcode::BAnd => int_binary!(operator_bit_and, BAD_TYPE_BIT_AND, |a, b| a & b),
                Opcode::BOr => int_binary!(operator_bit_or, BAD_TYPE_BIT_OR, |a, b| a | b),
                Opcode::Shl => {
                    int_binary!(operator_shl, BAD_TYPE_SHL, |a, b| a.wrapping_shl(b as u32))
                }
                Opcode::AShr => {
                    int_binary!(operator_ashr, BAD_TYPE_ASHR, |a, b| a
                        .wrapping_shr(b as u32))
                }
                Opcode::LShr => int_binary!(operator_lshr, BAD_TYPE_LSHR, |a, b| {
                    (a as u32).wrapping_shr(b as u32) as i32
                }),
                Opcode::Xor => {
                    let right = self.pop();
                    let left = self.pop();
                    let logical = |v: Value| matches!(v, Value::Nil | Value::True);
                    let truth = |v: Value| -> VmResult<bool> {
                        match v {
                            Value::True => Ok(true),
                            Value::Nil => Ok(false),
                            Value::Int(n) => Ok(n != 0),
                            _ => Err(VmError::new(code::NO_LOG_CONV)),
                        }
                    };
                    if logical(left) || logical(right) {
                        self.push(Value::logical(truth(left)? ^ truth(right)?));
                    } else if let (Value::Int(a), Value::Int(b)) = (left, right) {
                        self.push(Value::Int(a ^ b));
                    } else {
                        self.push(right);
                        let next = self.op_overload(
                            pc as u32,
                            None,
                            left,
                            self.predef.operator_xor,
                            1,
                            code::BAD_TYPE_XOR,
                            Return::Normal,
                        )?;
                        jump!(next);
                    }
                }
                Opcode::Not => {
                    let result = match self.top(0) {
                        Value::Nil => Value::True,
                        Value::Obj(id) => Value::logical(!id.is_valid()),
                        Value::True
                        | Value::Prop(_)
                        | Value::SString(_)
                        | Value::List(_)
                        | Value::CodeOfs(_)
                        | Value::FuncPtr(_)
                        | Value::Enum(_) => Value::Nil,
                        Value::Int(n) => Value::logical(n == 0),
                        _ => return Err(VmError::new(code::NO_LOG_CONV)),
                    };
                    *self.top_mut(0) = result;
                }
                Opcode::Boolize => {
                    let result = match self.top(0) {
                        value @ (Value::Nil | Value::True) => value,
                        Value::Int(n) => Value::logical(n != 0),
                        Value::Enum(_) => Value::True,
                        _ => return Err(VmError::new(code::NO_LOG_CONV)),
                    };
                    *self.top_mut(0) = result;
                }
                Opcode::Inc => match self.top(0) {
                    // The reference interpreter does not check this one
                    // for overflow.
                    Value::Int(n) => *self.top_mut(0) = Value::Int(n.wrapping_add(1)),
                    operand => match self.add(operand, Value::Int(1))? {
                        Some(result) => *self.top_mut(0) = result,
                        None => {
                            self.pop();
                            self.push(Value::Int(1));
                            let next = self.op_overload(
                                pc as u32,
                                None,
                                operand,
                                self.predef.operator_add,
                                1,
                                code::BAD_TYPE_ADD,
                                Return::Normal,
                            )?;
                            jump!(next);
                        }
                    },
                },
                Opcode::Dec => match self.top(0) {
                    Value::Int(n) => *self.top_mut(0) = self.int_sub(n, 1)?,
                    operand => match self.sub(operand, Value::Int(1))? {
                        Some(result) => *self.top_mut(0) = result,
                        None => {
                            self.pop();
                            self.push(Value::Int(1));
                            let next = self.op_overload(
                                pc as u32,
                                None,
                                operand,
                                self.predef.operator_sub,
                                1,
                                code::BAD_TYPE_SUB,
                                Return::Normal,
                            )?;
                            jump!(next);
                        }
                    },
                },

                // ---- comparison ----
                Opcode::Eq => {
                    let equal = self.pop_equal()?;
                    self.push(Value::logical(equal));
                }
                Opcode::Ne => {
                    let equal = self.pop_equal()?;
                    self.push(Value::logical(!equal));
                }
                Opcode::Lt => {
                    let order = self.pop_compare()?;
                    self.push(Value::logical(order == Ordering::Less));
                }
                Opcode::Le => {
                    let order = self.pop_compare()?;
                    self.push(Value::logical(order != Ordering::Greater));
                }
                Opcode::Gt => {
                    let order = self.pop_compare()?;
                    self.push(Value::logical(order == Ordering::Greater));
                }
                Opcode::Ge => {
                    let order = self.pop_compare()?;
                    self.push(Value::logical(order != Ordering::Less));
                }

                // ---- returning ----
                Opcode::RetVal => {
                    self.r0 = self.pop();
                    ret!();
                }
                Opcode::RetNil => {
                    self.r0 = Value::Nil;
                    ret!();
                }
                Opcode::RetTrue => {
                    self.r0 = Value::True;
                    ret!();
                }
                Opcode::Ret => ret!(),

                // ---- calling ----
                Opcode::NamedArgPtr => {
                    // Reached on return from a call with named arguments:
                    // remove them from the stack.
                    let count = usize::from(byte!());
                    self.discard(count);
                    pc += 2;
                }
                Opcode::NamedArgTab => {
                    let table_len = usize::from(word!());
                    let count = usize::from(word!());
                    self.discard(count);
                    pc += table_len - 2;
                }
                Opcode::Call => {
                    let argc = argc!();
                    let func = dword!();
                    let next = self.call_function(func, argc, pc as u32)?;
                    jump!(next);
                }
                Opcode::PtrCall => {
                    let argc = argc!();
                    let function = self.pop();
                    match function {
                        // A property pointer is called on `self`.
                        Value::Prop(prop) if self.self_obj().is_valid() => {
                            get_prop!(Value::Obj(self.self_obj()), prop, argc);
                        }
                        _ => {
                            let next = self.call_func_ptr(
                                function,
                                argc,
                                Return::Normal,
                                pc as u32,
                                None,
                            )?;
                            jump!(next);
                        }
                    }
                }
                Opcode::VarArgc => {
                    let Value::Int(count) = self.pop() else {
                        return Err(VmError::new(code::INT_VAL_REQD));
                    };
                    argc_override = Some(count as u32);
                    self.io.varargc = Some(count);
                    continue;
                }

                // ---- properties ----
                Opcode::GetProp => {
                    let prop = word!();
                    let target = self.pop();
                    get_prop!(target, prop, 0);
                }
                Opcode::CallProp => {
                    let argc = argc!();
                    let prop = word!();
                    let target = self.pop();
                    get_prop!(target, prop, argc);
                }
                Opcode::PtrCallProp => {
                    let argc = argc!();
                    let prop = self.pop_prop()?;
                    let target = self.pop();
                    get_prop!(target, prop, argc);
                }
                Opcode::GetPropSelf => {
                    let prop = word!();
                    get_prop!(self.self_value(), prop, 0);
                }
                Opcode::CallPropSelf => {
                    let argc = argc!();
                    let prop = word!();
                    get_prop!(self.self_value(), prop, argc);
                }
                Opcode::PtrCallPropSelf => {
                    let argc = argc!();
                    let prop = self.pop_prop()?;
                    get_prop!(self.self_value(), prop, argc);
                }
                Opcode::ObjGetProp => {
                    let id = dword!();
                    let prop = word!();
                    get_prop!(Value::Obj(ObjId(id)), prop, 0);
                }
                Opcode::ObjCallProp => {
                    let argc = argc!();
                    let id = dword!();
                    let prop = word!();
                    get_prop!(Value::Obj(ObjId(id)), prop, argc);
                }
                Opcode::GetPropLcl1 => {
                    let index = byte!();
                    let prop = word!();
                    get_prop!(local!(index), prop, 0);
                }
                Opcode::CallPropLcl1 => {
                    let argc = argc!();
                    let index = byte!();
                    let prop = word!();
                    get_prop!(local!(index), prop, argc);
                }
                Opcode::GetPropR0 => {
                    let prop = word!();
                    get_prop!(self.r0, prop, 0);
                }
                Opcode::CallPropR0 => {
                    let argc = argc!();
                    let prop = word!();
                    get_prop!(self.r0, prop, argc);
                }
                Opcode::GetPropData | Opcode::PtrGetPropData => {
                    return Err(VmError::new(code::NO_DEBUGGER));
                }
                Opcode::Inherit => {
                    let argc = argc!();
                    let prop = word!();
                    let next = self.inherit_prop(prop, argc, pc as u32)?;
                    jump!(next);
                }
                Opcode::PtrInherit => {
                    let argc = argc!();
                    let prop = self.pop_prop()?;
                    let next = self.inherit_prop(prop, argc, pc as u32)?;
                    jump!(next);
                }
                Opcode::ExpInherit => {
                    let argc = argc!();
                    let prop = word!();
                    let class = Value::Obj(ObjId(dword!()));
                    let self_value = self.self_value();
                    let next = self.eval_prop(
                        class,
                        prop,
                        self_value,
                        argc,
                        Return::Normal,
                        pc as u32,
                        None,
                    )?;
                    jump!(next);
                }
                Opcode::PtrExpInherit => {
                    let argc = argc!();
                    let prop = self.pop_prop()?;
                    let class = Value::Obj(ObjId(dword!()));
                    let self_value = self.self_value();
                    let next = self.eval_prop(
                        class,
                        prop,
                        self_value,
                        argc,
                        Return::Normal,
                        pc as u32,
                        None,
                    )?;
                    jump!(next);
                }
                Opcode::Delegate => {
                    let argc = argc!();
                    let prop = word!();
                    let target = self.pop();
                    let self_value = self.self_value();
                    let next = self.eval_prop(
                        target,
                        prop,
                        self_value,
                        argc,
                        Return::Normal,
                        pc as u32,
                        None,
                    )?;
                    jump!(next);
                }
                Opcode::PtrDelegate => {
                    let argc = argc!();
                    let prop = self.pop_prop()?;
                    let target = self.pop();
                    let self_value = self.self_value();
                    let next = self.eval_prop(
                        target,
                        prop,
                        self_value,
                        argc,
                        Return::Normal,
                        pc as u32,
                        None,
                    )?;
                    jump!(next);
                }

                // ---- stack manipulation ----
                Opcode::Swap2 => {
                    let len = self.stack.len();
                    self.stack.swap(len - 1, len - 3);
                    self.stack.swap(len - 2, len - 4);
                }
                Opcode::SwapN => {
                    let a = usize::from(byte!());
                    let b = usize::from(byte!());
                    let len = self.stack.len();
                    self.stack.swap(len - 1 - a, len - 1 - b);
                }
                Opcode::GetArgN0 => self.push(param!(0)),
                Opcode::GetArgN1 => self.push(param!(1)),
                Opcode::GetArgN2 => self.push(param!(2)),
                Opcode::GetArgN3 => self.push(param!(3)),
                Opcode::GetLcl1 => {
                    let index = byte!();
                    self.push(local!(index));
                }
                Opcode::GetLcl2 => {
                    let index = word!();
                    self.push(local!(index));
                }
                Opcode::GetArg1 => {
                    let index = byte!();
                    self.push(param!(index));
                }
                Opcode::GetArg2 => {
                    let index = word!();
                    self.push(param!(index));
                }
                Opcode::PushSelf => self.push(self.self_value()),
                Opcode::GetDbLcl
                | Opcode::GetDbArg
                | Opcode::GetDbArgc
                | Opcode::SetDbLcl
                | Opcode::SetDbArg => return Err(VmError::new(code::NO_DEBUGGER)),
                Opcode::GetArgc => {
                    let argc = self.frames.last().map_or(0, |f| f.argc);
                    self.push(Value::Int(argc as i32));
                }
                Opcode::Dup => self.push(self.top(0)),
                Opcode::Disc => self.discard(1),
                Opcode::Disc1 => {
                    let count = usize::from(byte!());
                    self.discard(count);
                }
                Opcode::GetR0 => self.push(self.r0),
                Opcode::Swap => {
                    let len = self.stack.len();
                    self.stack.swap(len - 1, len - 2);
                }
                Opcode::PushCtxEle => {
                    let element = byte!();
                    let frame = self.frames.last().expect("a function is running");
                    let value = match element {
                        1 => Value::Prop(frame.prop),
                        2 => frame.target,
                        3 => Value::obj_or_nil(frame.definer),
                        4 => frame.invokee,
                        _ => return Err(VmError::new(code::INVALID_OPCODE)),
                    };
                    self.push(value);
                }
                Opcode::Dup2 => {
                    self.push(self.top(1));
                    self.push(self.top(1));
                }
                Opcode::GetSpN => {
                    let depth = usize::from(byte!());
                    self.push(self.top(depth));
                }
                Opcode::GetLclN0 => self.push(local!(0)),
                Opcode::GetLclN1 => self.push(local!(1)),
                Opcode::GetLclN2 => self.push(local!(2)),
                Opcode::GetLclN3 => self.push(local!(3)),
                Opcode::GetLclN4 => self.push(local!(4)),
                Opcode::GetLclN5 => self.push(local!(5)),
                Opcode::GetSetLcl1R0 => {
                    let index = byte!();
                    self.push(self.r0);
                    local!(index) = self.r0;
                }
                Opcode::GetSetLcl1 => {
                    let index = byte!();
                    local!(index) = self.top(0);
                }
                Opcode::DupR0 => {
                    self.push(self.r0);
                    self.push(self.r0);
                }

                // ---- branching ----
                Opcode::Switch => {
                    let value = self.top(0);
                    let count = usize::from(word!());
                    let mut matched = false;
                    for _ in 0..count {
                        let case = Value::from_holder(&code[pc..pc + Value::HOLDER_SIZE]);
                        pc += Value::HOLDER_SIZE;
                        if self.equals(value, case)? {
                            pc = target!();
                            matched = true;
                            break;
                        }
                        pc += 2;
                    }
                    self.discard(1);
                    if !matched {
                        pc = target!();
                    }
                }
                Opcode::Jmp => pc = target!(),
                Opcode::Jt => {
                    let value = self.pop();
                    branch_if!(!is_false(value));
                }
                Opcode::Jf => {
                    let value = self.pop();
                    branch_if!(is_false(value));
                }
                Opcode::Je => {
                    let equal = self.pop_equal()?;
                    branch_if!(equal);
                }
                Opcode::Jne => {
                    let equal = self.pop_equal()?;
                    branch_if!(!equal);
                }
                Opcode::Jgt => {
                    let order = self.pop_compare()?;
                    branch_if!(order == Ordering::Greater);
                }
                Opcode::Jge => {
                    let order = self.pop_compare()?;
                    branch_if!(order != Ordering::Less);
                }
                Opcode::Jlt => {
                    let order = self.pop_compare()?;
                    branch_if!(order == Ordering::Less);
                }
                Opcode::Jle => {
                    let order = self.pop_compare()?;
                    branch_if!(order != Ordering::Greater);
                }
                Opcode::Jst => {
                    // Short-circuit `||`: keep a true value and branch.
                    let value = self.top(0);
                    if is_true(value) {
                        pc = target!();
                    } else {
                        if !matches!(value, Value::Nil | Value::Int(_)) {
                            return Err(VmError::new(code::LOG_VAL_REQD));
                        }
                        self.discard(1);
                        pc += 2;
                    }
                }
                Opcode::Jsf => {
                    // Short-circuit `&&`: keep a false value and branch.
                    if is_false(self.top(0)) {
                        pc = target!();
                    } else {
                        self.discard(1);
                        pc += 2;
                    }
                }
                Opcode::LJsr => {
                    // A local subroutine call, used for `finally` blocks.
                    // The return address is an offset within the function.
                    let func = self.frames.last().map_or(0, |f| f.func as usize);
                    self.push(Value::Int((pc + 2 - func) as i32));
                    pc = target!();
                }
                Opcode::LRet => {
                    let index = u16::from_le_bytes([code[pc], code[pc + 1]]);
                    let Value::Int(offset) = local!(index) else {
                        return Err(VmError::new(code::INT_VAL_REQD));
                    };
                    let func = self.frames.last().map_or(0, |f| f.func as usize);
                    pc = func + offset as usize;
                }
                Opcode::JNil => {
                    let value = self.pop();
                    branch_if!(value.is_nil());
                }
                Opcode::JNotNil => {
                    let value = self.pop();
                    branch_if!(!value.is_nil());
                }
                Opcode::JR0T => branch_if!(!is_false(self.r0)),
                Opcode::JR0F => branch_if!(is_false(self.r0)),
                Opcode::IterNext => {
                    let index = word!();
                    match self.iter_next(local!(index))? {
                        Some(value) => {
                            self.push(value);
                            pc += 2;
                        }
                        None => pc = target!(),
                    }
                }

                // ---- output and intrinsic functions ----
                Opcode::Say => {
                    let offset = dword!();
                    self.push(Value::SString(offset));
                    let self_id = self.self_obj();
                    let next = self.say_top(self_id, Return::Normal, pc as u32)?;
                    jump!(next);
                }
                Opcode::SayVal => {
                    let self_id = self.self_obj();
                    let next = self.say_top(self_id, Return::Normal, pc as u32)?;
                    jump!(next);
                }
                Opcode::BuiltinA | Opcode::BuiltinB | Opcode::BuiltinC | Opcode::BuiltinD => {
                    let argc = argc!();
                    let index = u16::from(byte!());
                    let set = opcode as u16 - Opcode::BuiltinA as u16;
                    self.call_bif(set, index, argc)?;
                }
                Opcode::Builtin1 => {
                    let argc = argc!();
                    let index = u16::from(byte!());
                    let set = u16::from(byte!());
                    self.call_bif(set, index, argc)?;
                }
                Opcode::Builtin2 => {
                    let argc = argc!();
                    let index = word!();
                    let set = u16::from(byte!());
                    self.call_bif(set, index, argc)?;
                }
                Opcode::CallExt => return Err(VmError::new(code::CALLEXT_NOT_IMPL)),
                Opcode::Throw => {
                    let Value::Obj(exception) = self.pop() else {
                        return Err(VmError::new(code::OBJ_VAL_REQD));
                    };
                    return Err(VmError::Thrown(exception));
                }

                // ---- indexing ----
                Opcode::Index => {
                    let (container, index) = (self.top(1), self.top(0));
                    match self.index(container, index)? {
                        Some(value) => {
                            self.discard(1);
                            *self.top_mut(0) = value;
                        }
                        None => overload_binary!(operator_idx, CANNOT_INDEX_TYPE),
                    }
                }
                Opcode::IdxLcl1Int8 => {
                    let container = local!(byte!());
                    let index = Value::Int(byte!().into());
                    match self.index(container, index)? {
                        Some(value) => self.push(value),
                        None => {
                            self.push(index);
                            let next = self.op_overload(
                                pc as u32,
                                None,
                                container,
                                self.predef.operator_idx,
                                1,
                                code::CANNOT_INDEX_TYPE,
                                Return::Normal,
                            )?;
                            jump!(next);
                        }
                    }
                }
                Opcode::IdxInt8 => {
                    let container = self.top(0);
                    let index = Value::Int(byte!().into());
                    match self.index(container, index)? {
                        Some(value) => *self.top_mut(0) = value,
                        None => {
                            self.pop();
                            self.push(index);
                            let next = self.op_overload(
                                pc as u32,
                                None,
                                container,
                                self.predef.operator_idx,
                                1,
                                code::CANNOT_INDEX_TYPE,
                                Return::Normal,
                            )?;
                            jump!(next);
                        }
                    }
                }
                Opcode::SetInd => {
                    let index = self.pop();
                    let container = self.pop();
                    let value = self.pop();
                    match self.set_index(container, index, value)? {
                        Some(result) => self.push(result),
                        None => {
                            self.push(value);
                            self.push(index);
                            let next = self.op_overload(
                                pc as u32,
                                None,
                                container,
                                self.predef.operator_setidx,
                                2,
                                code::CANNOT_INDEX_TYPE,
                                Return::Normal,
                            )?;
                            jump!(next);
                        }
                    }
                }
                Opcode::SetIndLcl1I8 => {
                    let local_index = byte!();
                    let index = Value::Int(byte!().into());
                    let container = local!(local_index);
                    let value = self.pop();
                    match self.set_index(container, index, value)? {
                        Some(result) => local!(local_index) = result,
                        None => {
                            self.push(value);
                            self.push(index);
                            let next = self.op_overload(
                                pc as u32,
                                Some(local_index.into()),
                                container,
                                self.predef.operator_setidx,
                                2,
                                code::CANNOT_INDEX_TYPE,
                                Return::Normal,
                            )?;
                            jump!(next);
                        }
                    }
                }

                // ---- creating objects ----
                Opcode::New1 | Opcode::TrNew1 => {
                    let argc = argc!();
                    let class = usize::from(byte!());
                    let next = self.new_object(class, argc, opcode == Opcode::TrNew1, pc as u32)?;
                    jump!(next);
                }
                Opcode::New2 | Opcode::TrNew2 => {
                    let argc = argc16!();
                    let class = usize::from(word!());
                    let next = self.new_object(class, argc, opcode == Opcode::TrNew2, pc as u32)?;
                    jump!(next);
                }

                // ---- operating on locals ----
                Opcode::IncLcl => {
                    let index = word!();
                    add_to_local!(
                        index,
                        Value::Int(1),
                        operator_add,
                        BAD_TYPE_ADD,
                        int_add,
                        add
                    );
                }
                Opcode::DecLcl => {
                    let index = word!();
                    add_to_local!(
                        index,
                        Value::Int(1),
                        operator_sub,
                        BAD_TYPE_SUB,
                        int_sub,
                        sub
                    );
                }
                Opcode::AddILcl1 => {
                    let index = byte!();
                    let operand = Value::Int((byte!() as i8).into());
                    add_to_local!(index, operand, operator_add, BAD_TYPE_ADD, int_add, add);
                }
                Opcode::AddILcl4 => {
                    let index = word!();
                    let operand = Value::Int(dword!() as i32);
                    add_to_local!(index, operand, operator_add, BAD_TYPE_ADD, int_add, add);
                }
                Opcode::AddToLcl => {
                    let index = word!();
                    let operand = self.pop();
                    add_to_local!(index, operand, operator_add, BAD_TYPE_ADD, int_add, add);
                }
                Opcode::SubFromLcl => {
                    let index = word!();
                    let operand = self.pop();
                    add_to_local!(index, operand, operator_sub, BAD_TYPE_SUB, int_sub, sub);
                }
                Opcode::ZeroLcl1 => {
                    let index = byte!();
                    local!(index) = Value::Int(0);
                }
                Opcode::ZeroLcl2 => {
                    let index = word!();
                    local!(index) = Value::Int(0);
                }
                Opcode::NilLcl1 => {
                    let index = byte!();
                    local!(index) = Value::Nil;
                }
                Opcode::NilLcl2 => {
                    let index = word!();
                    local!(index) = Value::Nil;
                }
                Opcode::OneLcl1 => {
                    let index = byte!();
                    local!(index) = Value::Int(1);
                }
                Opcode::OneLcl2 => {
                    let index = word!();
                    local!(index) = Value::Int(1);
                }
                Opcode::SetLcl1 => {
                    let index = byte!();
                    local!(index) = self.pop();
                }
                Opcode::SetLcl2 => {
                    let index = word!();
                    local!(index) = self.pop();
                }
                Opcode::SetArg1 => {
                    let index = byte!();
                    param!(index) = self.pop();
                }
                Opcode::SetArg2 => {
                    let index = word!();
                    param!(index) = self.pop();
                }
                Opcode::SetLcl1R0 => {
                    let index = byte!();
                    local!(index) = self.r0;
                }

                // ---- assigning to properties ----
                Opcode::SetProp => {
                    let prop = word!();
                    let object = self.pop_obj()?;
                    let value = self.pop();
                    self.set_prop(object, prop, value)?;
                }
                Opcode::PtrSetProp => {
                    let prop = self.pop_prop()?;
                    let object = self.pop_obj()?;
                    let value = self.pop();
                    self.set_prop(object, prop, value)?;
                }
                Opcode::SetPropSelf => {
                    let prop = word!();
                    let value = self.pop();
                    let object = self.self_obj();
                    self.set_prop(object, prop, value)?;
                }
                Opcode::ObjSetProp => {
                    let object = ObjId(dword!());
                    let prop = word!();
                    let value = self.pop();
                    self.set_prop(object, prop, value)?;
                }

                // ---- method context ----
                Opcode::SetSelf => {
                    let value = self.pop();
                    if let Some(frame) = self.frames.last_mut() {
                        frame.self_value = value;
                    }
                }
                Opcode::LoadCtx => {
                    let context = self.pop();
                    let items = match self.as_list(context) {
                        Some(items) if items.len() >= 4 => items,
                        _ => return Err(VmError::new(code::LIST_VAL_REQD)),
                    };
                    if let Some(frame) = self.frames.last_mut() {
                        frame.self_value = items.get(0);
                        frame.prop = match items.get(1) {
                            Value::Prop(prop) => prop,
                            _ => 0,
                        };
                        frame.target = items.get(2);
                        frame.definer = items.get(3).obj_id();
                    }
                }
                Opcode::StoreCtx => {
                    let frame = self.frames.last().expect("a function is running");
                    let items = vec![
                        frame.self_value,
                        Value::Prop(frame.prop),
                        frame.target,
                        Value::obj_or_nil(frame.definer),
                    ];
                    let context = self.new_list(items);
                    self.push(context);
                }

                Opcode::Bp => return Err(VmError::new(code::BREAKPOINT)),
                Opcode::Nop => {}
            }
        }
    }

    // ---- helpers for the loop ----

    /// `self` in the current frame, as a value.
    #[inline]
    fn self_value(&self) -> Value {
        self.frames.last().map_or(Value::Nil, |f| f.self_value)
    }

    fn pop_prop(&mut self) -> VmResult<PropId> {
        match self.pop() {
            Value::Prop(prop) => Ok(prop),
            _ => Err(VmError::new(code::PROPPTR_VAL_REQD)),
        }
    }

    fn pop_obj(&mut self) -> VmResult<ObjId> {
        match self.pop() {
            Value::Obj(id) => Ok(id),
            _ => Err(VmError::new(code::OBJ_VAL_REQD)),
        }
    }

    /// Pop two values and compare them for equality.
    #[inline]
    fn pop_equal(&mut self) -> VmResult<bool> {
        let (a, b) = (self.top(1), self.top(0));
        let equal = match (a, b) {
            (Value::Int(x), Value::Int(y)) => x == y,
            _ => self.equals(a, b)?,
        };
        self.discard(2);
        Ok(equal)
    }

    /// Pop two values and compare them for order.
    #[inline]
    fn pop_compare(&mut self) -> VmResult<Ordering> {
        let (a, b) = (self.top(1), self.top(0));
        let order = match (a, b) {
            (Value::Int(x), Value::Int(y)) => x.cmp(&y),
            _ => self.compare(a, b)?,
        };
        self.discard(2);
        Ok(order)
    }

    // Integer arithmetic. A result that does not fit in 32 bits becomes
    // a BigNumber.

    pub(crate) fn int_add(&mut self, a: i32, b: i32) -> VmResult<Value> {
        match a.checked_add(b) {
            Some(sum) => Ok(Value::Int(sum)),
            None => self.promote_int_arith(Arith::Add, a, b),
        }
    }

    pub(crate) fn int_sub(&mut self, a: i32, b: i32) -> VmResult<Value> {
        match a.checked_sub(b) {
            Some(difference) => Ok(Value::Int(difference)),
            None => self.promote_int_arith(Arith::Sub, a, b),
        }
    }

    pub(crate) fn int_mul(&mut self, a: i32, b: i32) -> VmResult<Value> {
        match a.checked_mul(b) {
            Some(product) => Ok(Value::Int(product)),
            None => self.promote_int_arith(Arith::Mul, a, b),
        }
    }

    pub(crate) fn int_div(&mut self, a: i32, b: i32) -> VmResult<Value> {
        if b == 0 {
            return Err(VmError::new(code::DIVIDE_BY_ZERO));
        }
        match a.checked_div(b) {
            Some(quotient) => Ok(Value::Int(quotient)),
            None => self.promote_int_arith(Arith::Div, a, b),
        }
    }

    pub(crate) fn int_neg(&mut self, a: i32) -> VmResult<Value> {
        match a.checked_neg() {
            Some(negated) => Ok(Value::Int(negated)),
            None => self.promote_int_arith(Arith::Sub, 0, a),
        }
    }

    /// Turn a list on the stack into individual arguments, for a call
    /// that passes a list with `...`. The argument count so far is under
    /// the list and is replaced by the new count.
    fn make_list_params(&mut self) -> VmResult<()> {
        let value = self.pop();
        let Value::Int(count) = self.pop() else {
            return Err(VmError::new(code::INT_VAL_REQD));
        };
        match self.list_like_len(value)? {
            None => {
                // Not a list: it is one more argument as it stands.
                self.push(value);
                self.push(Value::Int(count + 1));
            }
            Some(len) => {
                self.check_stack(len)?;
                for index in (1..=len).rev() {
                    let item = self.list_like_get(value, index)?;
                    self.push(item);
                }
                self.push(Value::Int(count + len as i32));
            }
        }
        Ok(())
    }

    /// Advance the iterator of a `for..in` loop. Returns the next value,
    /// or `None` when the iteration is over.
    fn iter_next(&mut self, iterator: Value) -> VmResult<Option<Value>> {
        let Value::Obj(id) = iterator else {
            return Ok(None);
        };
        match self.objects.get(id) {
            Some(Object::IndexedIter { .. }) => return self.indexed_iter_next(id),
            Some(Object::LookupIter { .. }) => return self.table_iter_next(id),
            Some(Object::Opaque { .. }) => {
                return Err(self.unimplemented_object(id, "iteration"));
            }
            Some(_) => {}
            None => return Ok(None),
        }
        // Anything else is asked through its methods.
        let (next_avail, get_next) = (self.predef.iter_next_avail, self.predef.iter_get_next);
        if next_avail == 0 || get_next == 0 {
            return Ok(None);
        }
        let native = || crate::vm::NativeCall::named("for..in");
        let more = self.call_prop(iterator, next_avail, 0, Some(native()))?;
        if is_false(more) {
            return Ok(None);
        }
        self.call_prop(iterator, get_next, 0, Some(native()))
            .map(Some)
    }

    /// Create an object of an intrinsic class, for the `new` instructions.
    /// The constructor arguments are on the stack. Returns the address to
    /// continue at; the new object ends up in register 0.
    fn new_object(
        &mut self,
        class: usize,
        argc: u32,
        transient: bool,
        return_pc: u32,
    ) -> VmResult<u32> {
        let Some(kind) = self.meta.get(class).map(|entry| entry.kind) else {
            return Err(VmError::new(code::BAD_METACLASS_INDEX));
        };
        if kind == MetaKind::TadsObject {
            let (_, next) = self.create_tads_object(argc, transient, Some(return_pc))?;
            return Ok(next);
        }
        let id = self.create_intrinsic(kind, argc)?;
        if transient {
            self.objects.set_transient(id);
        }
        self.r0 = Value::Obj(id);
        Ok(return_pc)
    }

    /// Construct an object of an intrinsic class from arguments on the
    /// stack, removing them.
    fn create_intrinsic(&mut self, kind: MetaKind, argc: u32) -> VmResult<ObjId> {
        match kind {
            MetaKind::AnonFuncPtr => {
                if argc < 1 {
                    return Err(VmError::new(code::WRONG_NUM_OF_ARGS));
                }
                let function = self.pop();
                if !matches!(function, Value::FuncPtr(_)) && !self.is_function_value(function) {
                    return Err(VmError::new(code::FUNCPTR_VAL_REQD));
                }
                let mut items = vec![function];
                for _ in 1..argc {
                    items.push(self.pop());
                }
                Ok(self.alloc_obj(Object::AnonFn(items)))
            }
            MetaKind::Vector => {
                if argc > 2 {
                    return Err(VmError::new(code::WRONG_NUM_OF_ARGS));
                }
                // The arguments are an allocation size, a source to copy
                // from or a number of nil elements, or both.
                let mut source = None;
                let mut count = 0usize;
                if argc >= 1 {
                    match self.arg(0) {
                        Value::Int(_) => {}
                        value => match self.list_like_len(value)? {
                            Some(len) => {
                                source = Some(value);
                                count = len;
                            }
                            None => return Err(VmError::new(code::BAD_TYPE_BIF)),
                        },
                    }
                }
                if argc >= 2 {
                    match self.arg(1) {
                        Value::Int(n) => {
                            if self.list_like_len(self.arg(1))?.is_none() {
                                count = n.max(0) as usize;
                            }
                        }
                        value => match self.list_like_len(value)? {
                            Some(len) => {
                                source = Some(value);
                                count = len;
                            }
                            None => return Err(VmError::new(code::BAD_TYPE_BIF)),
                        },
                    }
                }
                let mut items = Vec::with_capacity(count);
                for index in 1..=count {
                    items.push(match source {
                        Some(source) => self.list_like_get(source, index)?,
                        None => Value::Nil,
                    });
                }
                // Room for the number asked for, ten if none was, or the
                // elements copied if they are more.
                let room = if argc == 0 {
                    10
                } else if let Value::Int(n) = self.arg(0) {
                    n.max(0) as usize
                } else {
                    0
                };
                self.discard(argc as usize);
                let id = self.alloc_obj(Object::Vector(items));
                let room = room.max(self.objects.vector_alloc(id));
                self.objects.set_vector_alloc(id, room);
                Ok(id)
            }
            MetaKind::List => {
                // The elements, in order.
                let items: Vec<Value> = (0..argc as usize).map(|i| self.arg(i)).collect();
                self.discard(argc as usize);
                Ok(self.alloc_obj(Object::List(items.into())))
            }
            MetaKind::LookupTable => self.create_table(argc, false),
            MetaKind::WeakRefLookupTable => self.create_table(argc, true),
            MetaKind::StringBuffer => self.create_strbuf(argc),
            MetaKind::BigNumber => self.create_big(argc),
            MetaKind::File => Err(VmError::new(code::BAD_DYNAMIC_NEW)),
            MetaKind::ByteArray
            | MetaKind::CharacterSet
            | MetaKind::StringComparator
            | MetaKind::GrammarProduction
            | MetaKind::FileName
            | MetaKind::TemporaryFile
            | MetaKind::Date
            | MetaKind::TimeZone
            | MetaKind::Dictionary => {
                let id = match kind {
                    MetaKind::Date => self.create_date(argc)?,
                    MetaKind::TimeZone => self.create_time_zone(argc)?,
                    MetaKind::FileName => self.create_file_name(argc)?,
                    MetaKind::TemporaryFile => self.create_temp_file(argc)?,
                    MetaKind::ByteArray => self.create_bytes(argc)?,
                    MetaKind::CharacterSet => self.create_charset(argc)?,
                    MetaKind::StringComparator => self.create_strcomp(argc)?,
                    MetaKind::GrammarProduction => self.create_gram(argc)?,
                    _ => self.create_dict(argc)?,
                };
                self.discard(argc as usize);
                Ok(id)
            }
            MetaKind::RexPattern => {
                if argc != 1 {
                    return Err(VmError::new(code::WRONG_NUM_OF_ARGS));
                }
                let id = self.new_pattern(self.arg(0))?;
                self.discard(1);
                Ok(id)
            }
            MetaKind::IntrinsicClass => Err(VmError::new(code::ILLEGAL_NEW)),
            other => Err(VmError::unimplemented(format!(
                "creating an object of intrinsic class {}",
                other.name()
            ))),
        }
    }
}
