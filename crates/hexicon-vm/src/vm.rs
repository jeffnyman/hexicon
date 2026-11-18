//! The virtual machine's state and the helpers everything else builds on.

use std::ops::Deref;
use std::rc::Rc;

use hexicon_host::Host;
use hexicon_image::DebugInfo;

use crate::bif::FunctionSet;
use crate::console::{Formatter, Newline};
use crate::error::{VmError, VmResult, code};
use crate::meta::{MetaKind, MetaTable};
use crate::object::{Object, ObjectTable};
use crate::value::{INVALID_PROP, ObjId, PropId, Value};

/// What happens when a function returns.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Return {
    /// Continue the calling function at the return address.
    Normal,
    /// The function was called by the VM itself, not by byte code. Leave
    /// the interpreter loop and hand the result to the caller.
    Native,
    /// The function implements an overloaded operator. Push its result
    /// and continue at the return address.
    Operator,
    /// The function implements an overloaded operator in a compound
    /// assignment to a local. Store its result in that local of the
    /// caller and continue at the return address.
    OperatorToLocal(u16),
}

/// What the VM was doing when it called into byte code. Stack traces show
/// this in place of a byte-code frame.
#[derive(Debug, Clone)]
pub(crate) struct NativeCall {
    /// A description for error messages.
    pub name: &'static str,
    /// The intrinsic function making the call, if it is one.
    pub function: Option<(u16, u16)>,
    /// The object whose intrinsic method is making the call, or `nil`.
    pub self_value: Value,
    /// The number of that method within its intrinsic class.
    pub method_index: u16,
    /// The arguments the native code was called with, first argument first.
    pub args: Vec<Value>,
    /// Address of the instruction that invoked the native code, or zero.
    pub caller_pc: u32,
    /// How much was on the stack when the native code was called, with
    /// its arguments on top, or zero if that was not recorded. Named
    /// arguments lie under the positional ones.
    pub arg_top: usize,
}

impl NativeCall {
    /// A call with only a description.
    pub(crate) fn named(name: &'static str) -> Self {
        Self {
            name,
            function: None,
            self_value: Value::Nil,
            method_index: 0,
            args: Vec::new(),
            caller_pc: 0,
            arg_top: 0,
        }
    }
}

/// An activation of a byte-code function.
#[derive(Debug, Clone)]
pub(crate) struct Frame {
    /// Code pool address of the function's method header.
    pub func: u32,
    /// What to do when the function returns.
    pub ret: Return,
    /// Code pool address in the caller to continue at.
    pub return_pc: u32,
    /// Stack index of local variable 0. The arguments are just below:
    /// argument `i` (from 0) is at `base - 1 - i`.
    pub base: u32,
    /// Number of arguments the function was called with.
    pub argc: u32,
    /// The property being evaluated, or the invalid property.
    pub prop: PropId,
    /// The object the property was first looked up on, or `nil`.
    pub target: Value,
    /// The object the method was found in, or the invalid ID.
    pub definer: ObjId,
    /// `self`, or `nil` outside a method.
    pub self_value: Value,
    /// The function pointer or object that was invoked.
    pub invokee: Value,
    /// Present when the VM, not byte code, made the call.
    pub native: Option<Box<NativeCall>>,
}

impl Predef {
    /// The objects the VM knows by name, which are never garbage.
    pub(crate) fn objects(&self) -> [ObjId; 9] {
        [
            self.gram_alt_info,
            self.gram_alt_token_info,
            self.runtime_error,
            self.stack_info_class,
            self.const_str_obj,
            self.const_lst_obj,
            self.last_prop_obj,
            self.reflection_services,
            self.charset_unknown_exception,
        ]
    }
}

/// The method context a call starts with.
#[derive(Debug, Clone, Copy)]
pub(crate) struct MethodContext {
    pub prop: PropId,
    pub target: Value,
    pub definer: ObjId,
    pub self_value: Value,
    pub invokee: Value,
}

impl MethodContext {
    /// The context of a plain function call.
    pub(crate) fn function(invokee: Value) -> Self {
        Self {
            prop: INVALID_PROP,
            target: Value::Nil,
            definer: ObjId::INVALID,
            self_value: Value::Nil,
            invokee,
        }
    }
}

/// Objects and properties the VM itself refers to. Images name most of
/// them through exported symbols; the rest the VM creates at load time.
#[derive(Debug, Clone, Default)]
pub(crate) struct Predef {
    /// Base class of the exceptions the VM throws.
    pub runtime_error: ObjId,
    /// Property holding a VM exception's message text.
    pub exception_message: PropId,
    /// The constructor property.
    pub construct: PropId,
    /// The finalizer property.
    #[allow(dead_code)]
    pub destruct: PropId,
    /// The highest property ID the compiler assigned.
    pub last_prop: PropId,
    /// Property invoked when an undefined property is called.
    pub prop_not_defined: PropId,
    /// Class of the entries in a stack trace.
    pub stack_info_class: ObjId,
    /// Function to call to restore a saved state at startup.
    #[allow(dead_code)]
    pub main_restore: u32,
    /// The `length` property.
    pub length: PropId,
    /// A string object standing for all constant strings in method lookups.
    pub const_str_obj: ObjId,
    /// A list object standing for all constant lists in method lookups.
    pub const_lst_obj: ObjId,
    /// A one-element vector holding the last property ID allocated.
    pub last_prop_obj: ObjId,
    /// Operator overload properties.
    pub operator_add: PropId,
    pub operator_sub: PropId,
    pub operator_mul: PropId,
    pub operator_div: PropId,
    pub operator_mod: PropId,
    pub operator_xor: PropId,
    pub operator_shl: PropId,
    pub operator_ashr: PropId,
    pub operator_lshr: PropId,
    pub operator_bit_not: PropId,
    pub operator_bit_and: PropId,
    pub operator_bit_or: PropId,
    pub operator_neg: PropId,
    pub operator_idx: PropId,
    pub operator_setidx: PropId,
    /// The object and property the VM calls to name a value.
    pub reflection_services: ObjId,
    pub reflection_val_to_symbol: PropId,
    /// What the grammar parser sets on the match objects it creates,
    /// and the classes `getGrammarInfo` describes a production with.
    pub gram_first_token: PropId,
    pub gram_last_token: PropId,
    pub gram_token_list: PropId,
    pub gram_token_match_list: PropId,
    pub gram_tag: PropId,
    pub gram_misc_vocab: PropId,
    pub gram_alt_props: PropId,
    pub gram_alt_info: ObjId,
    pub gram_alt_token_info: ObjId,
    /// The methods of a comparator a program writes for a dictionary.
    pub calc_hash: PropId,
    pub match_values: PropId,
    /// The exception class for using a character set the VM lacks.
    pub charset_unknown_exception: ObjId,
    /// Where `specialsToHtml` keeps its state between calls.
    pub string_sth_flags: PropId,
    pub string_sth_tag: PropId,
    /// Property for converting an object to a string.
    pub obj_to_string: PropId,
    /// The iterator methods `for..in` loops call.
    pub iter_get_next: PropId,
    pub iter_next_avail: PropId,
}

/// The text of a string value: a constant in the image or a string object.
#[derive(Debug, Clone)]
pub(crate) enum StrRef {
    /// Bytes `start..start + len` of the constant pool.
    Const(Rc<[u8]>, u32, u32),
    /// The text of a string object.
    Obj(Rc<[u8]>),
}

impl Deref for StrRef {
    type Target = [u8];

    fn deref(&self) -> &[u8] {
        match self {
            Self::Const(pool, start, len) => &pool[*start as usize..(*start + *len) as usize],
            Self::Obj(text) => text,
        }
    }
}

/// The elements of a list value: a constant in the image or a list object.
#[derive(Debug, Clone)]
pub(crate) enum ListRef {
    /// A constant list: the pool, the offset of the first element, and the
    /// element count.
    Const(Rc<[u8]>, u32, u32),
    /// The elements of a list object.
    Obj(Rc<[Value]>),
}

impl ListRef {
    /// Number of elements.
    pub(crate) fn len(&self) -> usize {
        match self {
            Self::Const(_, _, len) => *len as usize,
            Self::Obj(items) => items.len(),
        }
    }

    /// The element at a zero-based index. Panics if out of range.
    pub(crate) fn get(&self, index: usize) -> Value {
        match self {
            Self::Const(pool, start, len) => {
                assert!(index < *len as usize);
                let at = *start as usize + index * Value::HOLDER_SIZE;
                Value::from_holder(&pool[at..at + Value::HOLDER_SIZE])
            }
            Self::Obj(items) => items[index],
        }
    }

    /// The elements, in order.
    pub(crate) fn iter(&self) -> impl Iterator<Item = Value> + '_ {
        (0..self.len()).map(|index| self.get(index))
    }

    /// The elements as a vector.
    pub(crate) fn to_vec(&self) -> Vec<Value> {
        self.iter().collect()
    }
}

/// Sizes of the structures in the code pool, which each image states.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct Layout {
    pub method_header: u32,
    pub exception_entry: u32,
    pub debug_line_entry: u32,
    pub debug_table_header: u32,
    /// Size of the header of a local symbol record in a debug table.
    pub debug_local_header: u32,
    /// Size of the header of a frame in a debug table.
    pub debug_frame_header: u32,
}

/// Stack slots available to a program, as in the reference VM's default
/// configuration. Programs can observe it: it decides how deep a runaway
/// recursion gets before it is stopped.
pub(crate) const STACK_LIMIT: usize = 4096;

/// Extra slots released while a stack overflow error is being reported.
pub(crate) const STACK_RESERVE: usize = 64;

/// Stack slots the reference VM spends on each call for bookkeeping that
/// Hexicon keeps elsewhere. They still count against the limit.
pub(crate) const FRAME_OVERHEAD: usize = 11;

/// A T3 virtual machine with a program loaded.
pub struct Vm {
    /// The code pool, as one contiguous array.
    pub(crate) code: Rc<[u8]>,
    /// The constant pool, as one contiguous array.
    pub(crate) consts: Rc<[u8]>,
    pub(crate) layout: Layout,
    pub(crate) objects: ObjectTable,
    pub(crate) meta: MetaTable,
    pub(crate) function_sets: Vec<FunctionSet>,
    pub(crate) predef: Predef,
    pub(crate) stack: Vec<Value>,
    pub(crate) frames: Vec<Frame>,
    /// The stack limit in force; raised briefly to report an overflow.
    pub(crate) stack_limit: usize,
    /// Register 0: the result of the last call.
    pub(crate) r0: Value,
    /// Address of the instruction being executed.
    pub(crate) pc: u32,
    /// The function the `say` instructions call, or `nil`.
    pub(crate) say_function: Value,
    /// The method the `say` instructions call on `self`, if it has one.
    pub(crate) say_method: PropId,
    /// The entry point's code address.
    pub(crate) entry_point: u32,
    /// Static initializers still to run: object and property.
    pub(crate) static_initializers: Vec<(ObjId, PropId)>,
    /// Debug records from the image, for stack traces.
    pub(crate) debug: DebugInfo,
    /// Whether the program is running under pre-initialization.
    pub(crate) preinit_mode: bool,
    /// The tables `t3GetGlobalSymbols` returns, once built.
    pub(crate) symbol_table: Option<ObjId>,
    pub(crate) macro_table: Option<ObjId>,
    /// Mathematical constants BigNumber has computed so far.
    pub(crate) big_cache: crate::bignum_math::Cache,
    /// The random number generator.
    pub(crate) rng: crate::rng::Rng,
    /// The image's timestamp and file name, which a saved state records
    /// so that it is only restored into the program that made it.
    pub(crate) image_timestamp: Vec<u8>,
    pub(crate) image_name: String,
    /// The VM chose the property for exception messages itself, because
    /// the image did not name one.
    pub(crate) exception_message_synthesized: bool,
    /// How deeply byte code is running inside calls made by the VM
    /// itself: 1 for the program's own code, more inside a callback.
    pub(crate) run_depth: u32,
    /// The collector is running, and must not start again.
    pub(crate) in_gc: bool,
    /// The most recent regular expression match.
    pub(crate) last_match: crate::string::LastMatch,
    /// Report each VM error on standard error as it is raised.
    pub(crate) trace_errors: bool,
    /// Formats what the program displays into lines for the host.
    pub(crate) console: Formatter,
    pub(crate) host: Box<dyn Host>,
}

impl Vm {
    // ---- the stack ----

    /// Push a value.
    #[inline]
    pub(crate) fn push(&mut self, value: Value) {
        self.stack.push(value);
    }

    /// Pop a value.
    #[inline]
    pub(crate) fn pop(&mut self) -> Value {
        self.stack.pop().unwrap_or(Value::Nil)
    }

    /// The value `depth` slots below the top of the stack.
    #[inline]
    pub(crate) fn top(&self, depth: usize) -> Value {
        self.stack[self.stack.len() - 1 - depth]
    }

    /// The slot `depth` slots below the top of the stack.
    #[inline]
    pub(crate) fn top_mut(&mut self, depth: usize) -> &mut Value {
        let index = self.stack.len() - 1 - depth;
        &mut self.stack[index]
    }

    /// Remove `count` values from the top of the stack.
    #[inline]
    pub(crate) fn discard(&mut self, count: usize) {
        let len = self.stack.len().saturating_sub(count);
        self.stack.truncate(len);
    }

    /// Argument `index` (from 0) of a native function called with its
    /// arguments on top of the stack.
    #[inline]
    pub(crate) fn arg(&self, index: usize) -> Value {
        self.top(index)
    }

    /// Stack slots in use, counted the way the reference VM counts them.
    pub(crate) fn stack_depth(&self) -> usize {
        self.stack.len() + self.frames.len() * FRAME_OVERHEAD
    }

    /// Fail with a stack overflow unless `slots` more slots are available.
    pub(crate) fn check_stack(&self, slots: usize) -> VmResult<()> {
        if self.stack_depth() + slots > self.stack_limit {
            Err(VmError::new(code::STACK_OVERFLOW))
        } else {
            Ok(())
        }
    }

    // ---- the current frame ----

    /// `self` in the current frame, or the invalid ID.
    pub(crate) fn self_obj(&self) -> ObjId {
        self.frames
            .last()
            .map_or(ObjId::INVALID, |frame| frame.self_value.obj_id())
    }

    // ---- objects ----

    /// Create a string object.
    pub(crate) fn new_string(&mut self, text: impl Into<Rc<[u8]>>) -> Value {
        Value::Obj(self.objects.alloc(Object::String(text.into())))
    }

    /// Create a string object from Rust text.
    pub(crate) fn new_str(&mut self, text: &str) -> Value {
        self.new_string(text.as_bytes())
    }

    /// Create a list object.
    pub(crate) fn new_list(&mut self, items: impl Into<Rc<[Value]>>) -> Value {
        Value::Obj(self.objects.alloc(Object::List(items.into())))
    }

    /// The intrinsic class an object belongs to.
    pub(crate) fn kind_of(&self, id: ObjId) -> Option<MetaKind> {
        Some(match self.objects.get(id)? {
            Object::Tads(_) => MetaKind::TadsObject,
            Object::IntClassMod(_) => MetaKind::IntClassMod,
            Object::String(_) => MetaKind::String,
            Object::List(_) => MetaKind::List,
            Object::Vector(_) => MetaKind::Vector,
            Object::AnonFn(_) => MetaKind::AnonFuncPtr,
            Object::IndexedIter { .. } => MetaKind::IndexedIterator,
            Object::LookupTable(table) if table.weak => MetaKind::WeakRefLookupTable,
            Object::LookupTable(_) => MetaKind::LookupTable,
            Object::LookupIter { .. } => MetaKind::LookupTableIterator,
            Object::Pattern { .. } => MetaKind::RexPattern,
            Object::StringBuffer(_) => MetaKind::StringBuffer,
            Object::BigNumber(_) => MetaKind::BigNumber,
            Object::ByteArray(_) => MetaKind::ByteArray,
            Object::StrComp(_) => MetaKind::StringComparator,
            Object::Dict(_) => MetaKind::Dictionary,
            Object::GramProd(_) => MetaKind::GrammarProduction,
            Object::CharSet { .. } => MetaKind::CharacterSet,
            Object::Class(_) => MetaKind::IntrinsicClass,
            Object::Opaque { meta_index, .. } => self.meta.get(*meta_index as usize)?.kind,
        })
    }

    // ---- strings and lists ----

    /// The constant string at a pool offset.
    pub(crate) fn const_str(&self, offset: u32) -> StrRef {
        let at = offset as usize;
        match self.consts.get(at..at + 2) {
            Some(b) => {
                let len = u32::from(u16::from_le_bytes([b[0], b[1]]));
                let available = self.consts.len() - at - 2;
                StrRef::Const(self.consts.clone(), offset + 2, len.min(available as u32))
            }
            None => StrRef::Const(self.consts.clone(), 0, 0),
        }
    }

    /// The constant list at a pool offset.
    pub(crate) fn const_list(&self, offset: u32) -> ListRef {
        let at = offset as usize;
        match self.consts.get(at..at + 2) {
            Some(b) => {
                let len = u32::from(u16::from_le_bytes([b[0], b[1]]));
                let available = (self.consts.len() - at - 2) / Value::HOLDER_SIZE;
                ListRef::Const(self.consts.clone(), offset + 2, len.min(available as u32))
            }
            None => ListRef::Const(self.consts.clone(), 0, 0),
        }
    }

    /// The text of a value, if it is a string constant or string object.
    pub(crate) fn as_str(&self, value: Value) -> Option<StrRef> {
        match value {
            Value::SString(offset) => Some(self.const_str(offset)),
            Value::Obj(id) => match self.objects.get(id)? {
                Object::String(text) => Some(StrRef::Obj(text.clone())),
                _ => None,
            },
            _ => None,
        }
    }

    /// The elements of a value, if it is a list constant or list object.
    pub(crate) fn as_list(&self, value: Value) -> Option<ListRef> {
        match value {
            Value::List(offset) => Some(self.const_list(offset)),
            Value::Obj(id) => match self.objects.get(id)? {
                Object::List(items) => Some(ListRef::Obj(items.clone())),
                _ => None,
            },
            _ => None,
        }
    }

    /// The text of a value; anything but a string is an error.
    pub(crate) fn require_str(&self, value: Value) -> VmResult<StrRef> {
        self.as_str(value)
            .ok_or_else(|| VmError::new(code::STRING_VAL_REQD))
    }

    // ---- argument helpers for native code ----

    /// An integer argument.
    pub(crate) fn int_arg(&self, value: Value) -> VmResult<i32> {
        match value {
            Value::Int(n) => Ok(n),
            _ => Err(VmError::new(code::INT_VAL_REQD)),
        }
    }

    /// A property ID argument.
    pub(crate) fn prop_arg(&self, value: Value) -> VmResult<PropId> {
        match value {
            Value::Prop(prop) => Ok(prop),
            _ => Err(VmError::new(code::PROPPTR_VAL_REQD)),
        }
    }

    /// An object argument.
    pub(crate) fn obj_arg(&self, value: Value) -> VmResult<ObjId> {
        match value {
            Value::Obj(id) => Ok(id),
            _ => Err(VmError::new(code::OBJ_VAL_REQD)),
        }
    }

    /// Display text on the console.
    pub(crate) fn print(&mut self, text: &[u8]) {
        self.console.format_text(text);
        self.send_output();
    }

    /// Hand finished output to the host.
    fn send_output(&mut self) {
        let output = self.console.take_output();
        if !output.is_empty() {
            self.host.print(&output);
        }
    }

    /// Report each VM error on standard error as it is raised, with the
    /// address of the instruction that caused it. This is a debugging aid
    /// for work on the VM itself.
    pub fn set_error_trace(&mut self, on: bool) {
        self.trace_errors = on;
    }

    /// Write out any text the console is holding for an unfinished line.
    /// Call this when the program ends.
    pub fn flush_output(&mut self) {
        self.console.flush(Newline::None);
        self.send_output();
    }
}
