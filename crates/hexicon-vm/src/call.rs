//! Calls, returns, exceptions, and property evaluation.

use hexicon_bytecode::Opcode;

use crate::error::{ErrParam, RuntimeError, VmError, VmResult, code};
use crate::object::Object;
use crate::program_object;
use crate::value::{INVALID_PROP, ObjId, PropId, Value};
use crate::vm::{Frame, MethodContext, NativeCall, Return, STACK_RESERVE, Vm};

/// A method header: the fixed record at the start of every function.
#[derive(Debug, Clone, Copy)]
pub(crate) struct FunctionHeader {
    pub min_args: u32,
    pub opt_args: u32,
    pub varargs: bool,
    pub locals: u32,
    pub stack_depth: u32,
    pub exception_table: u32,
    pub debug_records: u32,
}

impl FunctionHeader {
    /// Whether a call with `argc` arguments is valid.
    pub(crate) fn accepts(&self, argc: u32) -> bool {
        (argc >= self.min_args && argc <= self.min_args + self.opt_args)
            || (self.varargs && argc > self.min_args)
    }
}

impl Vm {
    /// Read the method header at a code address.
    pub(crate) fn function_header(&self, func: u32) -> VmResult<FunctionHeader> {
        let at = func as usize;
        let Some(b) = self.code.get(at..at + 10) else {
            return Err(VmError::new(code::CODEPTR_VAL_REQD));
        };
        let u16_at = |i: usize| u32::from(u16::from_le_bytes([b[i], b[i + 1]]));
        Ok(FunctionHeader {
            min_args: u32::from(b[0] & 0x7f),
            varargs: b[0] & 0x80 != 0,
            opt_args: u32::from(b[1]),
            locals: u16_at(2),
            stack_depth: u16_at(4),
            exception_table: u16_at(6),
            debug_records: u16_at(8),
        })
    }

    /// Enter a function whose arguments are on the stack. Returns the
    /// address of its first instruction.
    pub(crate) fn push_frame(
        &mut self,
        func: u32,
        argc: u32,
        context: MethodContext,
        ret: Return,
        return_pc: u32,
        native: Option<Box<NativeCall>>,
    ) -> VmResult<u32> {
        prof_scope!(crate::prof::CALL);
        self.r0 = Value::Nil;
        let header = self.function_header(func)?;
        // Only damaged code calls with more arguments than there are.
        if argc as usize > self.stack.len() {
            return Err(VmError::new(code::STACK_OUT_OF_BOUNDS));
        }
        self.check_stack(header.stack_depth as usize + crate::vm::FRAME_OVERHEAD)?;
        if !header.accepts(argc) {
            return Err(match &native {
                Some(native) => VmError::with(
                    code::WRONG_NUM_OF_ARGS_CALLING,
                    vec![ErrParam::Str(native.name.to_owned())],
                ),
                None => {
                    let takes = if header.varargs {
                        format!("at least {}", header.min_args)
                    } else if header.opt_args > 0 {
                        format!(
                            "{} to {}",
                            header.min_args,
                            header.min_args + header.opt_args
                        )
                    } else {
                        header.min_args.to_string()
                    };
                    let which = match self.definition_place(func) {
                        Some(place) => format!("the function at {place}"),
                        None => "the function".to_owned(),
                    };
                    VmError::with_detail(
                        code::WRONG_NUM_OF_ARGS,
                        format!("{which} takes {takes}, and was given {argc}"),
                    )
                }
            });
        }
        let base = self.stack.len() as u32;
        self.stack
            .resize(self.stack.len() + header.locals as usize, Value::Nil);
        self.frames.push(Frame {
            func,
            ret,
            return_pc,
            base,
            argc,
            prop: context.prop,
            target: context.target,
            definer: context.definer,
            self_value: context.self_value,
            invokee: context.invokee,
            native,
            frame_ref: None,
        });
        Ok(func + self.layout.method_header)
    }

    /// Leave the current function. Returns the address to continue at, or
    /// `None` if the function was called by the VM itself.
    pub(crate) fn pop_frame(&mut self) -> Option<u32> {
        prof_scope!(crate::prof::RETURN);
        // A reference to the call keeps its variables from now on.
        if let Some(id) = self.frames.last()?.frame_ref {
            self.snapshot_frame(id);
        }
        let frame = self.frames.pop()?;
        self.stack.truncate((frame.base - frame.argc) as usize);
        match frame.ret {
            Return::Native => return None,
            Return::Normal => {}
            Return::Operator => self.push(self.r0),
            Return::OperatorToLocal(local) => {
                if let Some(caller) = self.frames.last() {
                    let index = caller.base as usize + local as usize;
                    if let Some(slot) = self.stack.get_mut(index) {
                        *slot = self.r0;
                    }
                }
            }
        }
        // Calls are nearly always followed by an instruction to fetch the
        // result, so do that as part of the return.
        let mut pc = frame.return_pc;
        if self.code.get(pc as usize) == Some(&(Opcode::GetR0 as u8)) {
            self.push(self.r0);
            pc += 1;
        }
        Some(pc)
    }

    /// Enter a function and, if the VM itself is the caller, run it to
    /// completion. Returns the address at which the current interpreter
    /// loop continues.
    fn invoke_code(
        &mut self,
        func: u32,
        argc: u32,
        context: MethodContext,
        ret: Return,
        return_pc: u32,
        native: Option<Box<NativeCall>>,
    ) -> VmResult<u32> {
        let pc = self.push_frame(func, argc, context, ret, return_pc, native)?;
        if ret == Return::Native {
            self.run_from(pc)?;
            Ok(return_pc)
        } else {
            Ok(pc)
        }
    }

    /// Call a function by code address, as the `call` instruction does.
    pub(crate) fn call_function(&mut self, func: u32, argc: u32, return_pc: u32) -> VmResult<u32> {
        self.push_frame(
            func,
            argc,
            MethodContext::function(Value::FuncPtr(func)),
            Return::Normal,
            return_pc,
            None,
        )
    }

    /// The function an object runs when it is called like a function.
    pub(crate) fn invoker(&self, id: ObjId) -> Option<u32> {
        let mut id = id;
        // An anonymous function can wrap another; follow the chain, but
        // not forever.
        for _ in 0..64 {
            match self.objects.get(id)? {
                Object::AnonFn(items) => match items.first()? {
                    Value::FuncPtr(func) => return Some(*func),
                    Value::Obj(inner) => id = *inner,
                    _ => return None,
                },
                Object::CompiledCode(func) => return Some(func.addr),
                _ => return None,
            }
        }
        None
    }

    /// Whether a value can be called like a function.
    pub(crate) fn is_function_value(&self, value: Value) -> bool {
        match value {
            Value::FuncPtr(_) | Value::BifPtr { .. } => true,
            Value::Obj(id) => self.invoker(id).is_some(),
            _ => false,
        }
    }

    /// Call a function pointer value with a plain function's context.
    pub(crate) fn call_func_ptr(
        &mut self,
        function: Value,
        argc: u32,
        ret: Return,
        return_pc: u32,
        native: Option<Box<NativeCall>>,
    ) -> VmResult<u32> {
        let mut context = MethodContext::function(function);
        if let Value::Obj(_) = function {
            context.self_value = function;
        }
        self.call_func_ptr_in(function, argc, context, ret, return_pc, native)
    }

    /// Call a function pointer value with a given method context.
    fn call_func_ptr_in(
        &mut self,
        function: Value,
        argc: u32,
        context: MethodContext,
        ret: Return,
        return_pc: u32,
        native: Option<Box<NativeCall>>,
    ) -> VmResult<u32> {
        match function {
            Value::Obj(id) => match self.invoker(id) {
                Some(func) => self.invoke_code(func, argc, context, ret, return_pc, native),
                None => Err(VmError::new(code::FUNCPTR_VAL_REQD)),
            },
            Value::FuncPtr(func) => self.invoke_code(func, argc, context, ret, return_pc, native),
            Value::BifPtr { set, index } => {
                let result = self.call_bif(set, index, argc);
                // An instruction that is to run again needs the function
                // pointer it took off the stack.
                if matches!(result, Err(VmError::Suspend)) {
                    self.push(function);
                }
                result?;
                Ok(return_pc)
            }
            _ => Err(VmError::new(code::FUNCPTR_VAL_REQD)),
        }
    }

    // ---- calls made by the VM itself ----

    /// Call a function value from native code and return its result.
    pub(crate) fn call_value(
        &mut self,
        function: Value,
        args: &[Value],
        native: NativeCall,
    ) -> VmResult<Value> {
        self.check_stack(args.len())?;
        for &arg in args.iter().rev() {
            self.push(arg);
        }
        self.call_func_ptr(
            function,
            args.len() as u32,
            Return::Native,
            0,
            Some(Box::new(native)),
        )?;
        Ok(self.r0)
    }

    /// Evaluate a property from native code and return its value. The
    /// arguments, if any, are already on the stack.
    pub(crate) fn call_prop(
        &mut self,
        target: Value,
        prop: PropId,
        argc: u32,
        native: Option<NativeCall>,
    ) -> VmResult<Value> {
        self.r0 = Value::Nil;
        if prop == INVALID_PROP {
            return Ok(Value::Nil);
        }
        self.eval_prop(
            target,
            prop,
            target,
            argc,
            Return::Native,
            0,
            native.map(Box::new),
        )?;
        Ok(self.r0)
    }

    // ---- property evaluation ----

    /// Look up a property on a value and evaluate it: call it if it is
    /// code, otherwise leave its value in register 0.
    ///
    /// `target` is where the lookup starts and `self_value` is what the
    /// method sees as `self`; they differ for `delegated` and for explicit
    /// `inherited` calls.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn eval_prop(
        &mut self,
        target: Value,
        prop: PropId,
        self_value: Value,
        argc: u32,
        ret: Return,
        return_pc: u32,
        native: Option<Box<NativeCall>>,
    ) -> VmResult<u32> {
        let mut argc = argc;
        let mut prop = prop;
        let mut target = target;
        let mut self_value = self_value;
        let mut found = self.lookup_for_call(&mut target, prop, &mut argc, &mut self_value)?;
        if found.is_none() && self.predef.prop_not_defined != INVALID_PROP {
            let fallback = self.predef.prop_not_defined;
            found = self.lookup_for_call(&mut target, fallback, &mut argc, &mut self_value)?;
            match found {
                Some((value, _)) if value.is_invocable_prop() => {
                    self.push(Value::Prop(prop));
                    argc += 1;
                    prop = fallback;
                }
                _ => {}
            }
        }
        self.eval_prop_value(
            found, self_value, prop, target, argc, ret, return_pc, native,
        )
    }

    /// Look up a property for evaluation. Methods the VM implements run
    /// during the lookup, consuming their arguments. If a constant string
    /// or list turns out to need a real `self`, one is created and
    /// `target` and `self_value` are updated.
    fn lookup_for_call(
        &mut self,
        target: &mut Value,
        prop: PropId,
        argc: &mut u32,
        self_value: &mut Value,
    ) -> VmResult<Option<(Value, ObjId)>> {
        match *target {
            Value::Obj(id) => {
                let found = self.obj_get_prop(id, prop, Some(argc))?;
                if !matches!(self_value, Value::Obj(_)) {
                    return Err(VmError::new(code::OBJ_VAL_REQD));
                }
                Ok(found)
            }
            Value::SString(_) | Value::List(_) => {
                if *self_value != *target {
                    return Err(VmError::new(code::OBJ_VAL_REQD));
                }
                let found = self.const_get_prop(*target, prop, Some(argc))?;
                if let Some((value, _)) = found
                    && value.is_invocable_prop()
                {
                    let object = self.const_to_object(*target);
                    *target = object;
                    *self_value = object;
                }
                Ok(found)
            }
            Value::Nil => Err(VmError::new(code::NIL_DEREF)),
            _ => Err(VmError::new(code::OBJ_VAL_REQD)),
        }
    }

    /// Make a string or list object with the contents of a constant.
    pub(crate) fn const_to_object(&mut self, constant: Value) -> Value {
        match constant {
            Value::SString(offset) => {
                let text = self.const_str(offset);
                self.new_string(&*text)
            }
            Value::List(offset) => {
                let items = self.const_list(offset).to_vec();
                self.new_list(items)
            }
            other => other,
        }
    }

    /// Evaluate the result of a property lookup.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn eval_prop_value(
        &mut self,
        found: Option<(Value, ObjId)>,
        self_value: Value,
        prop: PropId,
        target: Value,
        argc: u32,
        ret: Return,
        return_pc: u32,
        native: Option<Box<NativeCall>>,
    ) -> VmResult<u32> {
        let Some((value, definer)) = found else {
            // An undefined property evaluates to nil.
            self.discard(argc as usize);
            self.r0 = Value::Nil;
            return Ok(return_pc);
        };
        match value {
            Value::CodeOfs(func) => {
                let context = MethodContext {
                    prop,
                    target,
                    definer,
                    self_value,
                    invokee: Value::FuncPtr(func),
                };
                self.invoke_code(func, argc, context, ret, return_pc, native)
            }
            Value::DString(offset) => {
                if argc != 0 {
                    return Err(VmError::new(code::WRONG_NUM_OF_ARGS));
                }
                self.push(Value::SString(offset));
                self.say_top(self_value.obj_id(), ret, return_pc)
            }
            Value::ObjX(id) => {
                if self.invoker(id).is_some() {
                    let context = MethodContext {
                        prop,
                        target,
                        definer,
                        self_value,
                        invokee: Value::Obj(id),
                    };
                    self.call_func_ptr_in(Value::Obj(id), argc, context, ret, return_pc, native)
                } else if matches!(self.objects.get(id), Some(Object::String(_))) {
                    self.push(Value::Obj(id));
                    self.say_top(self_value.obj_id(), ret, return_pc)
                } else {
                    Err(VmError::new(code::BAD_TYPE_CALL))
                }
            }
            Value::BifPtrX { set, index } => {
                self.call_bif(set, index, argc)?;
                Ok(return_pc)
            }
            data => {
                if argc != 0 {
                    return Err(VmError::new(code::WRONG_NUM_OF_ARGS));
                }
                self.r0 = data;
                Ok(return_pc)
            }
        }
    }

    /// Evaluate the property a method inherits: the definition that the
    /// current method's own definition overrides.
    pub(crate) fn inherit_prop(
        &mut self,
        prop: PropId,
        argc: u32,
        return_pc: u32,
    ) -> VmResult<u32> {
        let (definer, target, self_value) = match self.frames.last() {
            Some(frame) => (frame.definer, frame.target, frame.self_value),
            None => (ObjId::INVALID, Value::Nil, Value::Nil),
        };
        let self_id = match self_value {
            Value::Obj(id) => id,
            _ => return Err(VmError::new(code::NIL_DEREF)),
        };
        let mut argc = argc;
        let mut prop = prop;
        let target_id = target.obj_id();
        let mut found = self.obj_inh_prop(self_id, prop, target_id, definer, Some(&mut argc))?;
        if found.is_none() && self.predef.prop_not_defined != INVALID_PROP {
            let fallback = self.predef.prop_not_defined;
            found = self.obj_inh_prop(self_id, fallback, target_id, definer, Some(&mut argc))?;
            match found {
                Some((value, _)) if value.is_invocable_prop() => {
                    self.push(Value::Prop(prop));
                    argc += 1;
                    prop = fallback;
                }
                _ => {}
            }
        }
        self.eval_prop_value(
            found,
            self_value,
            prop,
            target,
            argc,
            Return::Normal,
            return_pc,
            None,
        )
    }

    /// Display the string on top of the stack through the program's
    /// output method or function, as the `say` instructions do.
    pub(crate) fn say_top(&mut self, self_id: ObjId, ret: Return, return_pc: u32) -> VmResult<u32> {
        if self.say_method != INVALID_PROP && self_id.is_valid() {
            let method = self.say_method;
            if let Some((value, definer)) = self.obj_get_prop(self_id, method, None)?
                && value.is_invocable_prop()
            {
                return self.eval_prop_value(
                    Some((value, definer)),
                    Value::Obj(self_id),
                    method,
                    Value::Obj(self_id),
                    1,
                    ret,
                    return_pc,
                    None,
                );
            }
        }
        if self.say_function.is_nil() {
            return Err(VmError::new(code::SAY_IS_NOT_DEFINED));
        }
        self.call_func_ptr(self.say_function, 1, ret, return_pc, None)
    }

    /// Apply an overloaded operator: evaluate the operator's property on
    /// the left operand, with the other operands on the stack as
    /// arguments. Fails with `error` if the operand does not define the
    /// operator.
    ///
    /// The result goes on the stack, or into local `assign_to` of the
    /// calling function for the compound assignment instructions. When
    /// the VM itself is the caller (`ret` is `Native`), the result is left
    /// in register 0 instead.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn op_overload(
        &mut self,
        return_pc: u32,
        assign_to: Option<u16>,
        operand: Value,
        prop: PropId,
        argc: u32,
        error: u32,
        ret: Return,
    ) -> VmResult<u32> {
        if prop == INVALID_PROP
            || !matches!(operand, Value::Obj(_) | Value::SString(_) | Value::List(_))
        {
            return Err(VmError::new(error));
        }
        let mut argc = argc;
        let mut target = operand;
        let mut self_value = operand;
        let Some(found) = self.lookup_for_call(&mut target, prop, &mut argc, &mut self_value)?
        else {
            return Err(VmError::new(error));
        };
        let native = ret == Return::Native;
        let is_call = matches!(
            found.0,
            Value::CodeOfs(_) | Value::DString(_) | Value::ObjX(_)
        );
        let ret = match (native, is_call, assign_to) {
            (true, _, _) => Return::Native,
            (false, false, _) => Return::Normal,
            (false, true, None) => Return::Operator,
            (false, true, Some(local)) => Return::OperatorToLocal(local),
        };
        let pc = self.eval_prop_value(
            Some(found),
            self_value,
            prop,
            target,
            argc,
            ret,
            return_pc,
            None,
        )?;
        if !is_call && !native {
            match assign_to {
                None => self.push(self.r0),
                Some(local) => self.set_local(local as usize, self.r0),
            }
        }
        Ok(pc)
    }

    /// Set a local variable of the current function.
    pub(crate) fn set_local(&mut self, local: usize, value: Value) {
        if let Some(frame) = self.frames.last() {
            let index = frame.base as usize + local;
            if let Some(slot) = self.stack.get_mut(index) {
                *slot = value;
            }
        }
    }

    // ---- exceptions ----

    /// Find a handler for an exception, unwinding the stack as far as
    /// needed. Returns the handler's address, or `None` if the exception
    /// leaves the byte code this interpreter loop is running.
    pub(crate) fn unwind(&mut self, exception: ObjId) -> Option<u32> {
        let mut pc = self.pc;
        loop {
            let func = self.frames.last()?.func;
            if let Some(handler) = self.find_handler(func, pc, exception) {
                self.push(Value::Obj(exception));
                return Some(handler);
            }
            pc = self.pop_frame()?;
        }
    }

    /// Search a function's exception table for a handler that covers an
    /// address and accepts an exception.
    fn find_handler(&self, func: u32, pc: u32, exception: ObjId) -> Option<u32> {
        let header = self.function_header(func).ok()?;
        if header.exception_table == 0 {
            return None;
        }
        let offset = pc.checked_sub(func)?;
        let table = (func + header.exception_table) as usize;
        let count = {
            let b = self.code.get(table..table + 2)?;
            u16::from_le_bytes([b[0], b[1]]) as usize
        };
        let entry_size = self.layout.exception_entry as usize;
        for index in 0..count {
            let at = table + 2 + index * entry_size;
            let e = self.code.get(at..at + 10)?;
            let start = u32::from(u16::from_le_bytes([e[0], e[1]]));
            let end = u32::from(u16::from_le_bytes([e[2], e[3]]));
            let class = ObjId(u32::from_le_bytes([e[4], e[5], e[6], e[7]]));
            let handler = u32::from(u16::from_le_bytes([e[8], e[9]]));
            if offset >= start
                && offset <= end
                && (!class.is_valid()
                    || exception == class
                    || self.is_instance_of(exception, class))
            {
                return Some(func + handler);
            }
        }
        None
    }

    /// Turn a VM error into the exception object a program can catch: an
    /// instance of the program's `RuntimeError` class.
    pub(crate) fn runtime_exception(&mut self, error: RuntimeError) -> VmResult<ObjId> {
        if self.trace_errors {
            let func = self.frames.last().map_or(0, |f| f.func);
            eprintln!(
                "[vm error {}: {} at {:#x} in function {:#x}]",
                error.code, error, self.pc, func
            );
        }
        // Reporting a stack overflow needs stack; release the reserve.
        let overflow = error.code == code::STACK_OVERFLOW;
        if overflow {
            self.stack_limit = crate::vm::STACK_LIMIT + STACK_RESERVE;
        }
        let result = self.build_runtime_exception(&error);
        if overflow {
            self.stack_limit = crate::vm::STACK_LIMIT;
        }
        result
    }

    fn build_runtime_exception(&mut self, error: &RuntimeError) -> VmResult<ObjId> {
        let exception = if self.predef.runtime_error.is_valid() {
            // The constructor gets the error number and then the message
            // parameters. The reference VM reads each parameter's type
            // from the slot before it, so a parameter only arrives intact
            // if it has the type of its predecessor; the first is read as
            // an integer.
            for index in (0..error.params.len()).rev() {
                let value = match (
                    index.checked_sub(1).map(|i| &error.params[i]),
                    &error.params[index],
                ) {
                    (None | Some(ErrParam::Int(_)), ErrParam::Int(n)) => Value::Int(*n),
                    (Some(ErrParam::Str(_)), ErrParam::Str(text)) => self.new_str(text),
                    (None, ErrParam::Str(_)) => Value::Int(0),
                    _ => Value::Nil,
                };
                self.push(value);
            }
            self.push(Value::Int(error.code as i32));
            let class = self.predef.runtime_error;
            self.create_instance(class, error.params.len() as u32 + 1)?;
            match self.r0 {
                Value::Obj(id) => id,
                _ => return Err(VmError::new(code::EXCEPTION_OBJ_REQD)),
            }
        } else {
            self.alloc_obj(Object::Program(program_object::ProgramObject::with_room(
                Vec::new(),
                1,
            )))
        };
        if self.predef.exception_message != INVALID_PROP {
            let message = match code::message(error.code) {
                Some(template) => {
                    let mut text = crate::error::format_message(template, &error.params);
                    // The reference VM formats into a 256-byte buffer.
                    if text.len() > 255 {
                        let mut end = 255;
                        while !text.is_char_boundary(end) {
                            end -= 1;
                        }
                        text.truncate(end);
                    }
                    // More about this occurrence, such as which function
                    // was called with the wrong number of arguments.
                    if let Some(detail) = &error.detail {
                        text.push_str(": ");
                        text.push_str(detail);
                    }
                    text
                }
                None => String::new(),
            };
            let message = self.new_str(&message);
            let prop = self.predef.exception_message;
            self.set_prop(exception, prop, message)?;
        }
        Ok(exception)
    }

    /// Run the function on top of the frame stack until it returns.
    ///
    /// Errors raised along the way become exceptions and are offered to
    /// the handlers of the functions this loop is running. One that none
    /// of them catches is returned to the caller.
    pub(crate) fn run_from(&mut self, start_pc: u32) -> VmResult<()> {
        let base_depth = self.frames.len() - 1;
        self.run_at(start_pc, base_depth)
    }

    /// Run from an address until the function `base_depth` frames up
    /// returns.
    pub(crate) fn run_at(&mut self, start_pc: u32, base_depth: usize) -> VmResult<()> {
        let depth = self.run_depth;
        if depth > 0 {
            self.objects.enter_nested(depth);
        }
        self.run_depth += 1;
        let result = self.run_nested(start_pc, base_depth);
        self.run_depth -= 1;
        if depth > 0 {
            self.objects.leave_nested(depth, self.r0);
        }
        result
    }

    fn run_nested(&mut self, start_pc: u32, base_depth: usize) -> VmResult<()> {
        let saved_pc = self.pc;
        let mut pc = start_pc;
        let result = loop {
            let error = match self.execute(pc) {
                Ok(()) => break Ok(()),
                Err(error) => error,
            };
            let exception = match error {
                VmError::Thrown(exception) => exception,
                VmError::Runtime(error) => match self.runtime_exception(error) {
                    Ok(exception) => exception,
                    Err(error) => break Err(error),
                },
                VmError::Suspend => {
                    // The program stops here, with its frames in place,
                    // to run the same instruction again when it resumes.
                    if let Some(count) = self.io.varargc.take() {
                        self.push(Value::Int(count));
                    }
                    self.io.suspended = Some((self.pc, base_depth));
                    self.pc = saved_pc;
                    return Err(VmError::Suspend);
                }
                fatal => break Err(fatal),
            };
            match self.unwind(exception) {
                Some(handler) => pc = handler,
                None => break Err(VmError::Thrown(exception)),
            }
        };
        if result.is_err() {
            // A fatal error leaves frames behind; drop them so the caller
            // sees the stack as it was.
            while self.frames.len() > base_depth {
                self.pop_frame();
            }
        }
        self.pc = saved_pc;
        result
    }
}
