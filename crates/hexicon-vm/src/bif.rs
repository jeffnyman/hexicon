//! Intrinsic function sets: the functions the VM provides to programs.
//!
//! Each function follows the reference VM's implementation (`vmbift3.cpp`,
//! `vmbiftad.cpp`, `vmbiftio.cpp`). A function receives its arguments on
//! top of the stack, first argument uppermost, and leaves its result, if
//! it has one, in register 0. The caller removes the arguments.

use std::cmp::Ordering;

use crate::error::{ErrParam, VmError, VmResult, code};
use crate::identity;
use crate::object::Object;
use crate::value::{INVALID_PROP, NativeDesc, ObjId, Value};
use crate::vm::Vm;

/// A function set the VM knows about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FunctionSet {
    /// `t3vm`: access to the VM itself.
    T3Vm,
    /// `t3vmTEST`: hooks for the VM's own test suite.
    T3Test,
    /// `tads-gen`: general-purpose functions.
    TadsGen,
    /// `tads-io`: console input and output.
    TadsIo,
    /// `tads-io-ext`: extended console functions. Not implemented.
    TadsIoExt,
    /// `tads-net`: networking. Not implemented.
    TadsNet,
}

/// The entries of a function set: name and calling interface.
type Table = &'static [(&'static str, NativeDesc)];

const fn fixed(min: u8) -> NativeDesc {
    NativeDesc::fixed(min)
}
const fn opt(min: u8, opt: u8) -> NativeDesc {
    NativeDesc::optional(min, opt)
}
const fn var(min: u8) -> NativeDesc {
    NativeDesc::variable(min)
}

const T3VM: Table = &[
    ("t3RunGC", fixed(0)),
    ("t3SetSay", fixed(1)),
    ("t3GetVMVsn", fixed(0)),
    ("t3GetVMID", fixed(0)),
    ("t3GetVMBanner", fixed(0)),
    ("t3GetVMPreinitMode", fixed(0)),
    ("t3DebugTrace", var(1)),
    ("t3GetGlobalSymbols", opt(0, 1)),
    ("t3AllocProp", fixed(0)),
    ("t3GetStackTrace", opt(0, 2)),
    ("t3GetNamedArg", opt(1, 1)),
    ("t3GetNamedArgList", fixed(0)),
];

const T3TEST: Table = &[
    ("t3test_get_obj_id", fixed(1)),
    ("t3test_get_obj_gc_state", fixed(1)),
    ("t3test_get_charcode", fixed(1)),
];

const TADS_GEN: Table = &[
    ("dataType", fixed(1)),
    ("getArg", fixed(1)),
    ("firstObj", opt(0, 2)),
    ("nextObj", opt(1, 2)),
    ("randomize", var(0)),
    ("rand", var(0)),
    ("toString", opt(1, 2)),
    ("toInteger", opt(1, 1)),
    ("getTime", opt(0, 1)),
    ("rexMatch", opt(2, 1)),
    ("rexSearch", opt(2, 1)),
    ("rexGroup", fixed(1)),
    ("rexReplace", opt(3, 2)),
    ("savepoint", fixed(0)),
    ("undo", fixed(0)),
    ("saveGame", opt(1, 1)),
    ("restoreGame", fixed(1)),
    ("restartGame", fixed(0)),
    ("max", var(1)),
    ("min", var(1)),
    ("makeString", opt(1, 1)),
    ("getFuncParams", fixed(1)),
    ("toNumber", opt(1, 1)),
    ("sprintf", var(1)),
    ("makeList", opt(1, 1)),
    ("abs", fixed(1)),
    ("sgn", fixed(1)),
    ("concat", var(0)),
    ("rexSearchLast", opt(2, 1)),
];

const TADS_IO: Table = &[
    ("tadsSay", var(1)),
    ("setLogFile", opt(1, 1)),
    ("clearScreen", fixed(0)),
    ("morePrompt", fixed(0)),
    ("inputLine", fixed(0)),
    ("inputKey", fixed(0)),
    ("inputEvent", opt(0, 1)),
    ("inputDialog", fixed(5)),
    ("inputFile", fixed(4)),
    ("timeDelay", fixed(1)),
    ("systemInfo", var(1)),
    ("statusMode", fixed(1)),
    ("statusRight", fixed(1)),
    ("resExists", fixed(1)),
    ("setScriptFile", opt(1, 1)),
    ("getLocalCharSet", fixed(1)),
    ("flushOutput", fixed(0)),
    ("inputLineTimeout", opt(0, 1)),
    ("inputLineCancel", fixed(1)),
    ("bannerCreate", fixed(8)),
    ("bannerDelete", fixed(1)),
    ("bannerClear", fixed(1)),
    ("bannerSay", var(1)),
    ("bannerFlush", fixed(1)),
    ("bannerSizeToContents", fixed(1)),
    ("bannerGoTo", fixed(3)),
    ("bannerSetTextColor", fixed(3)),
    ("bannerSetScreenColor", fixed(2)),
    ("bannerGetInfo", fixed(1)),
    ("bannerSetSize", fixed(4)),
    ("logConsoleCreate", fixed(3)),
    ("logConsoleClose", fixed(1)),
    ("logConsoleSay", var(1)),
    ("logInputEvent", fixed(1)),
];

impl FunctionSet {
    /// The function set with a given name and version, as an image names
    /// it (`name/version`).
    pub(crate) fn from_image_name(image_name: &str) -> VmResult<Self> {
        let (name, version) = image_name.split_once('/').unwrap_or((image_name, "000000"));
        let (set, provided) = match name {
            "t3vm" => (Self::T3Vm, "010006"),
            "t3vmTEST" => (Self::T3Test, "010000"),
            "tads-gen" => (Self::TadsGen, "030008"),
            "tads-io" => (Self::TadsIo, "030007"),
            "tads-io-ext" => (Self::TadsIoExt, "030000"),
            "tads-net" => (Self::TadsNet, "030001"),
            _ => {
                return Err(VmError::with(
                    code::UNKNOWN_FUNC_SET,
                    vec![ErrParam::Str(image_name.to_owned())],
                ));
            }
        };
        if version > provided {
            return Err(VmError::with(
                code::FUNCSET_TOO_OLD,
                vec![
                    ErrParam::Str(image_name.to_owned()),
                    ErrParam::Str(provided.to_owned()),
                ],
            ));
        }
        Ok(set)
    }

    fn table(self) -> Table {
        match self {
            Self::T3Vm => T3VM,
            Self::T3Test => T3TEST,
            Self::TadsGen => TADS_GEN,
            Self::TadsIo => TADS_IO,
            Self::TadsIoExt | Self::TadsNet => &[],
        }
    }
}

// `firstObj` and `nextObj` flags.
const ENUM_INSTANCES: i32 = 1;
const ENUM_CLASSES: i32 = 2;

// `t3SetSay` arguments that clear a setting.
const SETSAY_NO_FUNC: i32 = 1;
const SETSAY_NO_METHOD: i32 = 2;

impl Vm {
    /// The calling interface of an intrinsic function, or `None` if the
    /// function does not exist.
    pub(crate) fn bif_desc(&self, set: u16, index: u16) -> Option<NativeDesc> {
        let table = self.function_sets.get(set as usize)?.table();
        table.get(index as usize).map(|(_, desc)| *desc)
    }

    /// Call an intrinsic function with `argc` arguments on the stack.
    pub(crate) fn call_bif(&mut self, set: u16, index: u16, argc: u32) -> VmResult<()> {
        let Some(&function_set) = self.function_sets.get(set as usize) else {
            return Err(VmError::with(
                code::UNAVAIL_INTRINSIC,
                vec![
                    ErrParam::Int(index.into()),
                    ErrParam::Str(format!("#{set}")),
                ],
            ));
        };
        if argc as usize > self.stack.len() {
            return Err(VmError::new(code::WRONG_NUM_OF_ARGS));
        }
        let base = self.stack.len() - argc as usize;
        match function_set {
            FunctionSet::T3Vm => self.bif_t3vm(index, argc)?,
            FunctionSet::T3Test => self.bif_t3test(index, argc)?,
            FunctionSet::TadsGen => self.bif_tads_gen(index, argc)?,
            FunctionSet::TadsIo => self.bif_tads_io(index, argc)?,
            FunctionSet::TadsIoExt | FunctionSet::TadsNet => {
                return Err(self.unimplemented_bif(function_set, index));
            }
        }
        self.stack.truncate(base);
        Ok(())
    }

    fn unimplemented_bif(&self, set: FunctionSet, index: u16) -> VmError {
        match set.table().get(index as usize) {
            Some((name, _)) => VmError::unimplemented(format!("intrinsic function {name}")),
            None => VmError::unimplemented(format!("intrinsic function {index} of {set:?}")),
        }
    }

    fn check_argc(argc: u32, expected: u32) -> VmResult<()> {
        Self::check_argc_range(argc, expected, expected)
    }

    fn check_argc_range(argc: u32, min: u32, max: u32) -> VmResult<()> {
        if argc < min || argc > max {
            Err(VmError::new(code::WRONG_NUM_OF_ARGS))
        } else {
            Ok(())
        }
    }

    // ---- t3vm ----

    fn bif_t3vm(&mut self, index: u16, argc: u32) -> VmResult<()> {
        match index {
            // t3RunGC(): garbage collection is not implemented yet, and
            // running without it changes nothing a program can see.
            0 => Self::check_argc(argc, 0)?,
            1 => self.t3_set_say(argc)?,
            2 => {
                Self::check_argc(argc, 0)?;
                self.r0 = Value::Int(identity::version_number());
            }
            3 => {
                Self::check_argc(argc, 0)?;
                self.r0 = self.new_str(identity::ID);
            }
            4 => {
                Self::check_argc(argc, 0)?;
                self.r0 = self.new_str(&identity::banner());
            }
            5 => {
                Self::check_argc(argc, 0)?;
                self.r0 = Value::Int(self.preinit_mode.into());
            }
            // t3DebugTrace(mode, ...): there is no debugger.
            6 => {
                if argc < 1 {
                    return Err(VmError::new(code::WRONG_NUM_OF_ARGS));
                }
                self.int_arg(self.arg(0))?;
                self.r0 = Value::Nil;
            }
            // t3GetGlobalSymbols(which?): the table of global symbols, or
            // of macros. Only a program compiled for debugging has them.
            7 => {
                Self::check_argc_range(argc, 0, 1)?;
                let which = if argc >= 1 {
                    self.int_arg(self.arg(0))?
                } else {
                    1
                };
                let table = match which {
                    1 => self.symbol_table,
                    2 => self.macro_table,
                    _ => None,
                };
                self.r0 = table.map_or(Value::Nil, Value::Obj);
            }
            8 => {
                Self::check_argc(argc, 0)?;
                self.r0 = Value::Prop(self.alloc_prop()?);
            }
            9 => self.t3_get_stack_trace(argc)?,
            10 => self.t3_get_named_arg(argc)?,
            11 => self.t3_get_named_arg_list(argc)?,
            _ => return Err(self.unimplemented_bif(FunctionSet::T3Vm, index)),
        }
        Ok(())
    }

    /// `t3SetSay(val)`: set the default output function or method, and
    /// return the previous setting.
    fn t3_set_say(&mut self, argc: u32) -> VmResult<()> {
        Self::check_argc(argc, 1)?;
        match self.arg(0) {
            arg @ (Value::Prop(_) | Value::Int(SETSAY_NO_METHOD)) => {
                self.r0 = match self.say_method {
                    INVALID_PROP => Value::Int(SETSAY_NO_METHOD),
                    prop => Value::Prop(prop),
                };
                self.say_method = match arg {
                    Value::Prop(prop) => prop,
                    _ => INVALID_PROP,
                };
            }
            arg @ (Value::FuncPtr(_)
            | Value::Obj(_)
            | Value::BifPtr { .. }
            | Value::Int(SETSAY_NO_FUNC)) => {
                self.r0 = match self.say_function {
                    Value::Nil => Value::Int(SETSAY_NO_FUNC),
                    function => function,
                };
                self.say_function = match arg {
                    Value::Int(_) => Value::Nil,
                    function => function,
                };
            }
            _ => return Err(VmError::new(code::BAD_TYPE_BIF)),
        }
        Ok(())
    }

    /// Allocate a property ID that the program does not use.
    fn alloc_prop(&mut self) -> VmResult<u16> {
        let holder = self.predef.last_prop_obj;
        let Some(Object::Vector(items)) = self.objects.get_mut(holder) else {
            return Err(VmError::new(code::OUT_OF_PROPIDS));
        };
        let last = match items.first() {
            Some(Value::Prop(prop)) => *prop,
            _ => 0,
        };
        if last == u16::MAX {
            return Err(VmError::new(code::OUT_OF_PROPIDS));
        }
        if items.is_empty() {
            items.push(Value::Nil);
        }
        items[0] = Value::Prop(last + 1);
        Ok(last + 1)
    }

    // ---- t3vmTEST ----

    fn bif_t3test(&mut self, index: u16, argc: u32) -> VmResult<()> {
        match index {
            0 => {
                Self::check_argc(argc, 1)?;
                let id = self.obj_arg(self.arg(0))?;
                self.r0 = Value::Int(id.0 as i32);
            }
            2 => {
                Self::check_argc(argc, 1)?;
                let text = self.require_str(self.arg(0))?;
                self.r0 = match std::str::from_utf8(&text)
                    .ok()
                    .and_then(|s| s.chars().next())
                {
                    Some(c) => Value::Int(c as i32),
                    None => Value::Nil,
                };
            }
            _ => return Err(self.unimplemented_bif(FunctionSet::T3Test, index)),
        }
        Ok(())
    }

    // ---- tads-gen ----

    fn bif_tads_gen(&mut self, index: u16, argc: u32) -> VmResult<()> {
        match index {
            // dataType(val)
            0 => {
                Self::check_argc(argc, 1)?;
                self.r0 = Value::Int(self.datatype(self.arg(0)));
            }
            // getArg(idx): an argument of the calling function.
            1 => {
                Self::check_argc(argc, 1)?;
                let index = self.int_arg(self.arg(0))?;
                let frame = self.frames.last().ok_or(VmError::new(code::BAD_VAL_BIF))?;
                if index < 1 || index as u32 > frame.argc {
                    return Err(VmError::new(code::BAD_VAL_BIF));
                }
                self.r0 = self.stack[frame.base as usize - index as usize];
            }
            // firstObj(cls?, flags?)
            2 => {
                Self::check_argc_range(argc, 0, 2)?;
                self.enum_objects(argc, 0, 1)?;
            }
            // nextObj(obj, cls?, flags?)
            3 => {
                Self::check_argc_range(argc, 1, 3)?;
                let previous = self.obj_arg(self.arg(0))?;
                self.enum_objects(argc - 1, 1, previous.0 + 1)?;
            }
            4 => self.bif_randomize(argc)?,
            5 => self.bif_rand(argc)?,
            // toString(val, radix?, isSigned?)
            6 => {
                Self::check_argc_range(argc, 1, 3)?;
                let radix = if argc >= 2 {
                    self.int_arg(self.arg(1))?
                } else {
                    10
                };
                if !(2..=36).contains(&radix) {
                    return Err(VmError::new(code::BAD_VAL_BIF));
                }
                let value = self.arg(0);
                // In any radix but 10, a number is shown unsigned unless
                // the caller asks otherwise.
                let signed = if argc >= 3 {
                    match self.arg(2) {
                        Value::True => true,
                        Value::Nil => false,
                        _ => return Err(VmError::new(code::BAD_TYPE_BIF)),
                    }
                } else {
                    radix == 10
                };
                let (string, _) = self.radix_text(value, radix as u32, signed)?;
                self.r0 = match string {
                    Value::Obj(_) => string,
                    // The result is always a string object.
                    other => self.const_to_object(other),
                };
            }
            // toInteger(val, radix?)
            7 => {
                Self::check_argc_range(argc, 1, 2)?;
                self.r0 = self.parse_integer(argc, true)?;
            }
            // sprintf(format, ...)
            23 => self.r0 = self.sprintf(argc)?,
            // toNumber(val, radix?)
            22 => {
                Self::check_argc_range(argc, 1, 2)?;
                self.r0 = self.parse_integer(argc, false)?;
            }
            // rexMatch(pat, str, index?)
            9 => {
                Self::check_argc_range(argc, 2, 3)?;
                self.r0 = self.rex_match(argc)?;
            }
            // rexSearch(pat, str, index?) and rexSearchLast(pat, str, index?)
            10 | 28 => {
                Self::check_argc_range(argc, 2, 3)?;
                self.r0 = self.rex_search(argc, index == 10)?;
            }
            // rexGroup(n)
            11 => {
                Self::check_argc(argc, 1)?;
                self.r0 = self.rex_group()?;
            }
            // rexReplace(pat, str, replacement, flags?, index?, limit?)
            12 => {
                Self::check_argc_range(argc, 3, 6)?;
                self.r0 = self.find_replace(self.arg(1), argc, true)?;
            }
            // makeString(val, repeatCount?)
            20 => {
                Self::check_argc_range(argc, 1, 2)?;
                self.r0 = self.make_string(argc)?;
            }
            // max(...) and min(...)
            18 | 19 => {
                let wanted = if index == 18 {
                    Ordering::Greater
                } else {
                    Ordering::Less
                };
                self.r0 = self.extreme(argc, wanted)?;
            }
            // getFuncParams(func)
            21 => {
                Self::check_argc(argc, 1)?;
                let Some((min, opt, varargs)) = self.value_interface(self.arg(0)) else {
                    return Err(VmError::new(code::FUNCPTR_VAL_REQD));
                };
                self.r0 = self.new_list(vec![
                    Value::Int(min),
                    Value::Int(opt),
                    Value::logical(varargs),
                ]);
            }
            // makeList(val, repeatCount?)
            24 => {
                Self::check_argc_range(argc, 1, 2)?;
                let count = if argc >= 2 {
                    self.int_arg(self.arg(1))?
                } else {
                    1
                };
                if count < 0 {
                    return Err(VmError::new(code::BAD_VAL_BIF));
                }
                self.r0 = self.new_list(vec![self.arg(0); count as usize]);
            }
            // abs(val)
            25 => {
                Self::check_argc(argc, 1)?;
                self.r0 = match self.arg(0) {
                    Value::Int(n) => Value::Int(n.wrapping_abs()),
                    other => match self.as_big(other).cloned() {
                        Some(big) if big.neg && big.kind != crate::bignum::Kind::NaN => {
                            let mut positive = big;
                            positive.neg = false;
                            self.new_big(positive)
                        }
                        Some(_) => other,
                        None => return Err(VmError::new(code::BAD_TYPE_BIF)),
                    },
                };
            }
            // sgn(val)
            26 => {
                Self::check_argc(argc, 1)?;
                self.r0 = match self.arg(0) {
                    Value::Int(n) => Value::Int(n.signum()),
                    other => match self.as_big(other) {
                        Some(big) if big.zero => Value::Int(0),
                        Some(big) if big.neg => Value::Int(-1),
                        Some(_) => Value::Int(1),
                        None => return Err(VmError::new(code::BAD_TYPE_BIF)),
                    },
                };
            }
            // concat(...)
            27 => {
                let mut text = Vec::new();
                for index in 0..argc as usize {
                    let piece = self.cast_to_text(self.arg(index))?;
                    text.extend_from_slice(&piece);
                }
                self.r0 = self.new_string(text);
            }
            _ => return Err(self.unimplemented_bif(FunctionSet::TadsGen, index)),
        }
        Ok(())
    }

    /// Find the first object at or after an ID that passes the filters of
    /// `firstObj` and `nextObj`. The filter arguments start at stack
    /// position `first_arg`.
    fn enum_objects(&mut self, argc: u32, first_arg: usize, start: u32) -> VmResult<()> {
        let mut class = ObjId::INVALID;
        let mut flags = ENUM_INSTANCES;
        match argc {
            2 => {
                class = self.obj_arg(self.arg(first_arg))?;
                flags = self.int_arg(self.arg(first_arg + 1))?;
            }
            1 => match self.arg(first_arg) {
                Value::Int(n) => flags = n,
                Value::Obj(id) => class = id,
                _ => return Err(VmError::new(code::BAD_TYPE_BIF)),
            },
            _ => {}
        }
        self.r0 = Value::Nil;
        for id in (start..self.objects.id_limit()).map(ObjId) {
            let is_class = match self.objects.get(id) {
                None | Some(Object::IntClassMod(_)) => continue,
                Some(Object::Tads(object)) => object.is_class(),
                Some(Object::Class(_)) => true,
                Some(_) => false,
            };
            let wanted = if is_class {
                ENUM_CLASSES
            } else {
                ENUM_INSTANCES
            };
            if flags & wanted == 0 {
                continue;
            }
            let matches = if class.is_valid() {
                self.is_instance_of(id, class)
            } else {
                // Without a class, skip the strings and lists that
                // programs create as temporary values.
                !matches!(
                    self.objects.get(id),
                    Some(Object::String(_) | Object::List(_))
                )
            };
            if matches {
                self.r0 = Value::Obj(id);
                break;
            }
        }
        Ok(())
    }

    /// The index of a function set in the image's dependency list.
    pub(crate) fn function_set_index(&self, name: &str) -> Option<u16> {
        let wanted = FunctionSet::from_image_name(name).ok()?;
        self.function_sets
            .iter()
            .position(|&set| set == wanted)
            .map(|index| index as u16)
    }

    /// `makeString(val, repeatCount?)`: a string from a string, a
    /// character code, or a list of character codes, repeated.
    fn make_string(&mut self, argc: u32) -> VmResult<Value> {
        let value = self.arg(0);
        let repeat = if argc >= 2 {
            self.int_arg(self.arg(1))?
        } else {
            1
        };
        if repeat < 0 {
            return Err(VmError::new(code::BAD_VAL_BIF));
        }
        let code_point = |n: i32| char::from_u32(n as u32).unwrap_or('\u{fffd}');
        let unit: Vec<u8> = if let Some(text) = self.as_str(value) {
            text.to_vec()
        } else if let Value::Int(n) = value {
            code_point(n).to_string().into_bytes()
        } else if let Some(len) = self.list_like_len(value)? {
            let mut text = String::new();
            for index in 1..=len {
                match self.list_like_get(value, index)? {
                    Value::Int(n) => text.push(code_point(n)),
                    _ => return Err(VmError::new(code::INT_VAL_REQD)),
                }
            }
            text.into_bytes()
        } else {
            return Err(VmError::new(code::BAD_TYPE_BIF));
        };
        if unit.len() * repeat as usize > 0xffff - 2 {
            return Err(VmError::new(code::BAD_VAL_BIF));
        }
        Ok(self.new_string(unit.repeat(repeat as usize)))
    }

    /// Convert a value to text, showing numbers in a radix. That reaches
    /// into lists, whose elements are converted the same way.
    fn radix_text(
        &mut self,
        value: Value,
        radix: u32,
        signed: bool,
    ) -> VmResult<(Value, std::rc::Rc<[u8]>)> {
        if radix == 10 && signed {
            return self.text_of(value);
        }
        let text: Vec<u8> = match value {
            Value::Int(n) => int_to_radix(n, radix, signed).into_bytes(),
            _ => {
                if let Some(big) = self.as_big(value) {
                    if radix == 10 || !big.is_frac_zero() {
                        return self.text_of(value);
                    }
                    big.to_radix_text(radix).into_bytes()
                } else if let Some(len) = self.list_like_len(value)? {
                    let mut joined = Vec::new();
                    for index in 1..=len {
                        let element = self.list_like_get(value, index)?;
                        if index != 1 {
                            joined.push(b',');
                        }
                        joined.extend_from_slice(&self.radix_text(element, radix, signed)?.1);
                    }
                    joined
                } else {
                    return self.text_of(value);
                }
            }
        };
        let text: std::rc::Rc<[u8]> = text.into();
        Ok((self.new_string(text.clone()), text))
    }

    /// `toInteger(val, radix?)`.
    fn parse_integer(&mut self, argc: u32, int_only: bool) -> VmResult<Value> {
        let value = self.arg(0);
        if let Some(big) = self.as_big(value) {
            return if int_only {
                Self::big_to_int(big).map(Value::Int)
            } else {
                Ok(value)
            };
        }
        match value {
            Value::Int(_) => return Ok(value),
            Value::True => return Ok(Value::Int(1)),
            Value::Nil => return Ok(Value::Int(0)),
            _ => {}
        }
        let text = self.require_str(value)?;
        let radix = if argc >= 2 {
            let radix = self.int_arg(self.arg(1))?;
            if !(2..=36).contains(&radix) {
                return Err(VmError::new(code::BAD_VAL_BIF));
            }
            radix as u32
        } else {
            10
        };
        let trimmed = text.trim_ascii();
        if trimmed == b"nil" {
            return Ok(Value::Int(0));
        }
        // The reference VM compares only the first three characters here.
        if trimmed.len() == 4 && trimmed.starts_with(b"tru") {
            return Ok(Value::Int(1));
        }
        self.parse_number(&text, radix, int_only)
    }

    /// Parse text as a number in a radix. Unless only an integer will
    /// do, a decimal number with a fraction or an exponent, or one too
    /// large for an integer, is a BigNumber.
    pub(crate) fn parse_number(
        &mut self,
        text: &[u8],
        radix: u32,
        int_only: bool,
    ) -> VmResult<Value> {
        let big = |vm: &mut Vm| {
            let source = String::from_utf8_lossy(text).into_owned();
            if radix != 10 {
                return vm.new_big(crate::bignum::Big::parse_radix(&source, radix));
            }
            let prec = crate::bignum::Big::precision_from_str(&source);
            vm.new_big(crate::bignum::Big::parse(&source, prec))
        };
        if !int_only && radix == 10 {
            let source = String::from_utf8_lossy(text);
            let digits = source
                .trim_start()
                .trim_start_matches(['-', '+'])
                .trim_start();
            let body_end = digits
                .find(|c: char| !c.is_ascii_digit())
                .unwrap_or(digits.len());
            let rest = &digits[body_end..];
            let has_exponent = rest
                .strip_prefix(['e', 'E'])
                .map(|r| r.trim_start_matches(['+', '-']))
                .is_some_and(|r| r.starts_with(|c: char| c.is_ascii_digit()));
            if rest.starts_with('.') || has_exponent {
                return Ok(big(self));
            }
        }
        match parse_int(text, radix) {
            Ok(n) => Ok(Value::Int(n)),
            Err(_) if !int_only => Ok(big(self)),
            Err(error) => Err(error),
        }
    }

    /// Convert a value to a number, as an explicit cast does: an integer
    /// or a BigNumber stays as it is, true and nil are 1 and 0, and text
    /// is parsed.
    pub(crate) fn cast_to_num(&mut self, value: Value) -> VmResult<Value> {
        match value {
            Value::Int(_) => Ok(value),
            Value::True => Ok(Value::Int(1)),
            Value::Nil => Ok(Value::Int(0)),
            _ if self.as_big(value).is_some() => Ok(value),
            _ => match self.as_str(value) {
                Some(text) => {
                    let text = text.to_vec();
                    self.parse_number(&text, 10, false)
                }
                None => Err(VmError::new(code::NO_NUM_CONV)),
            },
        }
    }

    /// Convert a value to an integer, as an explicit cast does.
    pub(crate) fn cast_to_int(&mut self, value: Value) -> VmResult<i32> {
        match value {
            Value::Int(n) => Ok(n),
            Value::True => Ok(1),
            Value::Nil => Ok(0),
            _ => {
                if let Some(big) = self.as_big(value) {
                    return Self::big_to_int(big);
                }
                match self.as_str(value) {
                    Some(text) => parse_int(&text, 10),
                    None => Err(VmError::new(code::NO_INT_CONV)),
                }
            }
        }
    }

    /// `max(...)` or `min(...)`: the extreme of the arguments, or of the
    /// elements of a single list-like argument.
    fn extreme(&mut self, argc: u32, wanted: Ordering) -> VmResult<Value> {
        if argc < 1 {
            return Err(VmError::new(code::WRONG_NUM_OF_ARGS));
        }
        let first = self.arg(0);
        let values: Vec<Value> = match (argc, self.list_like_len(first)?) {
            (1, Some(len)) => {
                if len == 0 {
                    return Err(VmError::new(code::BAD_VAL_BIF));
                }
                let mut values = Vec::with_capacity(len);
                for index in 1..=len {
                    values.push(self.list_like_get(first, index)?);
                }
                values
            }
            _ => (0..argc as usize).map(|index| self.arg(index)).collect(),
        };
        let mut best = values[0];
        for &value in &values[1..] {
            if self.compare(value, best)? == wanted {
                best = value;
            }
        }
        Ok(best)
    }

    /// The text of a value where a string is expected, as output and
    /// `concat` convert it: `nil` is the empty string.
    pub(crate) fn cast_to_text(&mut self, value: Value) -> VmResult<std::rc::Rc<[u8]>> {
        match value {
            Value::Nil => Ok(Vec::new().into()),
            Value::SString(_) | Value::Obj(_) | Value::Int(_) | Value::True | Value::List(_) => {
                Ok(self.text_of(value)?.1)
            }
            _ => Err(VmError::new(code::NO_STR_CONV)),
        }
    }

    // ---- tads-io ----

    fn bif_tads_io(&mut self, index: u16, argc: u32) -> VmResult<()> {
        match index {
            // tadsSay(...): display each argument.
            0 => {
                for index in 0..argc as usize {
                    let value = self.arg(index);
                    match value {
                        Value::Nil => {}
                        Value::SString(_) | Value::Obj(_) | Value::List(_) | Value::Int(_) => {
                            let text = self.text_of(value)?.1;
                            self.print(&text);
                        }
                        _ => return Err(VmError::new(code::BAD_TYPE_BIF)),
                    }
                }
            }
            // flushOutput(): output is not buffered yet.
            16 => Self::check_argc(argc, 0)?,
            _ => return Err(self.unimplemented_bif(FunctionSet::TadsIo, index)),
        }
        Ok(())
    }
}

/// Format an integer in a radix from 2 to 36, with capital letters for
/// the digits past 9.
pub(crate) fn int_to_radix(value: i32, radix: u32, signed: bool) -> String {
    let (negative, mut magnitude) = if value >= 0 || !signed {
        (false, value as u32)
    } else {
        (true, value.unsigned_abs())
    };
    let mut digits = Vec::new();
    loop {
        let digit = char::from_digit(magnitude % radix, radix).unwrap_or('0');
        digits.push(digit.to_ascii_uppercase());
        magnitude /= radix;
        if magnitude == 0 {
            break;
        }
    }
    if negative {
        digits.push('-');
    }
    digits.iter().rev().collect()
}

/// Parse an integer the way `toInteger` does: optional spaces, an optional
/// sign, optional spaces, then digits up to the first character that is
/// not a digit of the radix.
fn parse_int(text: &[u8], radix: u32) -> VmResult<i32> {
    let mut chars = String::from_utf8_lossy(text)
        .chars()
        .collect::<Vec<_>>()
        .into_iter()
        .peekable();
    while chars.next_if(|c| c.is_whitespace()).is_some() {}
    let negative = match chars.peek() {
        Some('-') => {
            chars.next();
            true
        }
        Some('+') => {
            chars.next();
            false
        }
        _ => false,
    };
    while chars.next_if(|c| c.is_whitespace()).is_some() {}
    // Unsigned values are allowed to fill all 32 bits in the radixes
    // normally used for bit patterns.
    let limit: u64 = if !negative && matches!(radix, 2 | 8 | 16) {
        0xffff_ffff
    } else if negative {
        0x8000_0000
    } else {
        0x7fff_ffff
    };
    let mut acc: u64 = 0;
    for c in chars {
        let Some(digit) = c.to_digit(36).filter(|&d| d < radix) else {
            break;
        };
        acc = acc * u64::from(radix) + u64::from(digit);
        if acc > limit {
            return Err(VmError::new(code::NUM_OVERFLOW));
        }
    }
    Ok(if negative && acc <= 0x7fff_ffff {
        -(acc as i64) as i32
    } else {
        acc as u32 as i32
    })
}
