//! Looking at the call stack: stack traces, source positions, and named
//! arguments.

use hexicon_bytecode::{Opcode, decode};

use crate::error::{ErrParam, VmError, VmResult, code};
use crate::lookup::LookupTable;
use crate::value::Value;
use crate::vm::{NativeCall, Return, Vm};

/// `t3GetStackTrace` flag: include local variables.
const GST_LOCALS: i32 = 0x0001;
/// `t3GetStackTrace` flag: include frame references.
const GST_FREFS: i32 = 0x0002;

/// One level of the call stack as a stack trace reports it.
struct Level {
    /// Index of the frame this level belongs to.
    frame: usize,
    /// The function running at this level.
    func: u32,
    /// Offset of the current instruction within the function. Zero means
    /// this level is native code that called into byte code.
    offset: u32,
    /// For a native level, what the native code was.
    native: Option<Box<NativeCall>>,
}

impl Vm {
    /// The address just past the instruction at `pc`, skipping a
    /// `varargc` prefix.
    fn instruction_end(&self, pc: u32) -> u32 {
        let mut at = pc;
        if self.code.get(at as usize) == Some(&(Opcode::VarArgc as u8)) {
            at += 1;
        }
        match decode(&self.code, at) {
            Ok(instruction) => at + instruction.len,
            Err(_) => at + 1,
        }
    }

    /// The offset in the caller at which frame `index` returns: zero if
    /// native code made the call.
    fn return_offset(&self, index: usize) -> u32 {
        let frame = &self.frames[index];
        if frame.ret == Return::Native || index == 0 {
            0
        } else {
            frame.return_pc.wrapping_sub(self.frames[index - 1].func)
        }
    }

    /// The levels of the call stack, innermost first. Native code that
    /// called into byte code gets a level of its own.
    fn stack_levels(&self) -> Vec<Level> {
        let mut levels = Vec::new();
        let Some(top) = self.frames.len().checked_sub(1) else {
            return levels;
        };
        let mut frame = top;
        let mut func = self.frames[top].func;
        let mut offset = self.pc.wrapping_sub(func);
        let mut native: Option<Box<NativeCall>> = None;
        loop {
            levels.push(Level {
                frame,
                func,
                offset,
                native: native.clone(),
            });
            if offset == 0 {
                // A native level: next comes the byte code that invoked
                // the native code, in the same frame.
                offset = match native.take() {
                    Some(call) if call.caller_pc != 0 => {
                        self.instruction_end(call.caller_pc).wrapping_sub(func)
                    }
                    _ => self.layout.method_header,
                };
            } else {
                offset = self.return_offset(frame);
                native = self.frames[frame].native.clone();
                if frame == 0 {
                    break;
                }
                frame -= 1;
                func = self.frames[frame].func;
            }
        }
        levels
    }

    /// `t3GetStackTrace(level?, flags?)`: describe the call stack as a
    /// list of `T3StackInfo` objects, or one level of it as one object.
    pub(crate) fn t3_get_stack_trace(&mut self, argc: u32) -> VmResult<()> {
        if argc > 2 {
            return Err(VmError::new(code::WRONG_NUM_OF_ARGS));
        }
        let class = self.predef.stack_info_class;
        if !class.is_valid() {
            self.r0 = Value::Nil;
            return Ok(());
        }
        // The more arguments the program's T3StackInfo constructor takes,
        // the more detail it is given.
        let (mut want_named_args, mut want_locals, mut want_frefs) = (false, false, false);
        if let Some((min, opt, varargs)) = self.prop_interface(class, self.predef.construct)? {
            want_named_args = min + opt >= 7 || varargs;
            want_locals = min + opt >= 8 || varargs;
            want_frefs = min + opt >= 9 || varargs;
        }
        let mut single_level = None;
        if argc >= 1 && !self.arg(0).is_nil() {
            let level = self.int_arg(self.arg(0))?;
            if level <= 0 {
                return Err(VmError::new(code::BAD_VAL_BIF));
            }
            single_level = Some(level as usize - 1);
        }
        let flags = if argc >= 2 {
            self.int_arg(self.arg(1))?
        } else {
            0
        };

        let levels = self.stack_levels();
        let mut entries = Vec::new();
        for (number, level) in levels.iter().enumerate() {
            if single_level.is_some_and(|wanted| wanted != number) {
                continue;
            }
            let entry = self.stack_info(level, flags, want_named_args, want_locals, want_frefs)?;
            if single_level.is_some() {
                self.r0 = entry;
                return Ok(());
            }
            entries.push(entry);
            // Keep the entries reachable while the rest are built.
            self.push(entry);
        }
        self.discard(entries.len());
        self.r0 = match single_level {
            Some(_) => Value::Nil,
            None => self.new_list(entries),
        };
        Ok(())
    }

    /// Build the `T3StackInfo` object for one level of the stack.
    fn stack_info(
        &mut self,
        level: &Level,
        flags: i32,
        want_named_args: bool,
        want_locals: bool,
        want_frefs: bool,
    ) -> VmResult<Value> {
        let frame = &self.frames[level.frame];
        let mut info_func = Value::Nil;
        let mut info_obj = Value::Nil;
        let mut info_prop = Value::Nil;
        let mut info_self = Value::Nil;
        let mut info_args = Value::Nil;
        let mut info_source = Value::Nil;
        let mut frame_args: Vec<Value> = Vec::new();

        if level.offset == 0 {
            // Native code. Describe the intrinsic function or method, if
            // the VM recorded which it was.
            if let Some(call) = &level.native {
                if let Some((set, index)) = call.function {
                    info_func = Value::BifPtr { set, index };
                } else if !call.self_value.is_nil() {
                    info_self = call.self_value;
                    let kind = match call.self_value {
                        Value::Obj(id) => self.kind_of(id),
                        Value::List(_) => Some(crate::meta::MetaKind::List),
                        Value::SString(_) => Some(crate::meta::MetaKind::String),
                        _ => None,
                    };
                    info_obj = call.self_value;
                    if let Some(entry) = kind.and_then(|kind| self.meta.entry(kind)) {
                        info_obj = Value::obj_or_nil(entry.class_obj);
                        info_prop = Value::Prop(entry.method_prop(call.method_index));
                    }
                }
                frame_args = call.args.clone();
            }
        } else if !frame.definer.is_valid() {
            info_func = self.function_value(level.func);
        } else {
            info_obj = Value::Obj(frame.definer);
            info_prop = Value::Prop(frame.prop);
            info_self = frame.self_value;
        }
        if level.offset != 0 {
            let base = frame.base as usize;
            frame_args = (0..frame.argc as usize)
                .map(|index| self.stack[base - 1 - index])
                .collect();
        }
        let has_details = level.offset != 0 || level.native.is_some();
        let wants_frame_ref =
            level.offset != 0 && level.native.is_none() && flags & GST_FREFS != 0 && want_frefs;
        let wants_locals =
            level.offset != 0 && level.native.is_none() && flags & GST_LOCALS != 0 && want_locals;
        let named_args = if want_named_args && flags & GST_LOCALS != 0 {
            self.named_args_of_frame(level.frame)
        } else {
            None
        };

        let mut pushed = 0;
        if has_details {
            info_args = self.new_list(frame_args);
            self.push(info_args);
            pushed += 1;
            info_source = self.source_info(level.func, level.offset);
            self.push(info_source);
            pushed += 1;
        }

        // A reference to the frame, at the place the level is at.
        let info_frame_ref = if wants_frame_ref {
            let fref = self.frame_ref_of(level.frame);
            let desc = crate::frameref::FrameDesc {
                fref,
                scope: self.scope_at(level.func, level.offset),
                offset: level.offset as u16,
            };
            let desc = Value::Obj(self.alloc_obj(crate::object::Object::FrameDesc(desc)));
            self.push(desc);
            pushed += 1;
            desc
        } else {
            Value::Nil
        };

        // Constructor arguments, last first: func, obj, prop, self, args,
        // source location, then the optional locals, named arguments, and
        // frame reference.
        let info_locals = if wants_locals {
            let locals = self.stack_locals(level)?;
            self.push(locals);
            pushed += 1;
            locals
        } else {
            Value::Nil
        };
        let info_named_args = match named_args {
            Some(args) => {
                // A table of each name and its value.
                let buckets = match args.len() {
                    0..=8 => 8,
                    9..=32 => 32,
                    _ => 64,
                };
                let table = self.new_table(LookupTable::new(buckets, args.len(), false));
                self.push(Value::Obj(table));
                pushed += 1;
                for (name, slot) in args.into_iter().rev() {
                    let key = self.new_string(name);
                    let value = self.stack[slot];
                    self.table_set(table, key, value)?;
                }
                Value::Obj(table)
            }
            None => Value::Nil,
        };

        let mut info_argc = 6;
        if want_frefs {
            self.push(info_frame_ref);
            info_argc += 1;
        }
        if want_named_args {
            self.push(info_named_args);
            info_argc += 1;
        }
        if want_locals {
            self.push(info_locals);
            info_argc += 1;
        }
        self.push(info_source);
        self.push(info_args);
        self.push(info_self);
        self.push(info_prop);
        self.push(info_obj);
        self.push(info_func);
        let class = self.predef.stack_info_class;
        self.create_instance(class, info_argc)?;
        let entry = self.r0;
        self.discard(pushed);
        Ok(entry)
    }

    /// The local variables in scope at a level of the stack, as a table
    /// of each name and its value. Only a program compiled for debugging
    /// records the names; for any other, the table is empty.
    fn stack_locals(&mut self, level: &Level) -> VmResult<Value> {
        let table = self.new_table(LookupTable::new(32, 64, false));
        let result = Value::Obj(table);
        self.push(result);
        let filled = self.fill_stack_locals(table, level);
        self.discard(1);
        filled.map(|()| result)
    }

    fn fill_stack_locals(&mut self, table: crate::value::ObjId, level: &Level) -> VmResult<()> {
        // The variables of the innermost scope at the code position and
        // each one around it. An inner variable hides an outer one of
        // the same name.
        let scope = self.scope_at(level.func, level.offset);
        let base = self.frames[level.frame].base as usize;
        for sym in self.scope_syms(level.func, scope) {
            let name = match sym.pool {
                Some(at) => Value::SString(at),
                None => self.new_string(sym.name),
            };
            if self.table_get(table, name)?.is_some() {
                continue;
            }
            let slot = if sym.is_param {
                base.checked_sub(1 + sym.number)
            } else {
                Some(base + sym.number)
            };
            let stored = slot
                .and_then(|slot| self.stack.get(slot))
                .copied()
                .unwrap_or(Value::Nil);
            // A variable shared with an anonymous function lives in a
            // context object, which the local refers to.
            let value = match sym.ctx {
                Some(index) => self.list_like_get(stored, index)?,
                None => stored,
            };
            self.table_set(table, name, value)?;
        }
        Ok(())
    }

    /// The source file and line for an offset within a function, as a
    /// two-element list, or `nil` if the image does not record it.
    fn source_info(&mut self, func: u32, offset: u32) -> Value {
        let Some((source, line)) = self.source_line(func, offset) else {
            return Value::Nil;
        };
        let Some(file) = self.debug.source_files.get(source as usize) else {
            return Value::Nil;
        };
        let name = file.name.clone();
        let name = self.new_str(&name);
        self.push(name);
        let list = self.new_list(vec![name, Value::Int(line as i32)]);
        self.discard(1);
        list
    }

    /// Find the line record covering an offset within a function. Returns
    /// the source file index and line number.
    fn source_line(&self, func: u32, offset: u32) -> Option<(u16, u32)> {
        let header = self.function_header(func).ok()?;
        if header.debug_records == 0 {
            return None;
        }
        let table = (func + header.debug_records + self.layout.debug_table_header) as usize;
        let u16_at = |at: usize| -> Option<u32> {
            let b = self.code.get(at..at + 2)?;
            Some(u32::from(u16::from_le_bytes([b[0], b[1]])))
        };
        let count = u16_at(table)? as usize;
        if count == 0 {
            return None;
        }
        let entry_size = self.layout.debug_line_entry as usize;
        let entry = |index: usize| table + 2 + index * entry_size;
        // Each record covers from its own start to the next one's; the
        // last runs to the end of the code.
        let (mut lo, mut hi) = (0isize, count as isize - 1);
        while lo <= hi {
            let cur = lo + (hi - lo) / 2;
            let at = entry(cur as usize);
            let start = u16_at(at)?;
            let end = if cur as usize == count - 1 {
                header.debug_records
            } else {
                u16_at(entry(cur as usize + 1))?
            };
            if offset >= end {
                lo = if cur == lo { cur + 1 } else { cur };
            } else if offset < start {
                hi = if cur == hi { hi - 1 } else { cur };
            } else {
                let source = u16_at(at + 2)? as u16;
                let b = self.code.get(at + 4..at + 8)?;
                let line = u32::from_le_bytes([b[0], b[1], b[2], b[3]]);
                return Some((source, line));
            }
        }
        None
    }

    // ---- named arguments ----

    /// The named arguments a frame was called with: each name and the
    /// stack index of its value. Callers put named arguments on the stack
    /// under the positional ones, and follow the call instruction with a
    /// table of the names.
    fn named_args_of_frame(&self, index: usize) -> Option<Vec<(Vec<u8>, usize)>> {
        let frame = &self.frames[index];
        // A frame called from native code, such as a callback, sees the
        // named arguments of the call to that native code.
        let (return_pc, arg_top, argc) = if frame.ret == Return::Native {
            let call = frame.native.as_ref()?;
            if call.caller_pc == 0 || call.arg_top == 0 {
                return None;
            }
            (
                self.instruction_end(call.caller_pc),
                call.arg_top,
                call.args.len(),
            )
        } else if index == 0 {
            return None;
        } else {
            (frame.return_pc, frame.base as usize, frame.argc as usize)
        };
        let code = &self.code;
        let u16_at = |at: usize| -> Option<usize> {
            let b = code.get(at..at + 2)?;
            Some(usize::from(u16::from_le_bytes([b[0], b[1]])))
        };
        let mut at = return_pc as usize;
        if code.get(at) == Some(&(Opcode::NamedArgPtr as u8)) {
            // The table is elsewhere in the function; follow the pointer.
            at += 2 + u16_at(at + 2)?;
        }
        if code.get(at) != Some(&(Opcode::NamedArgTab as u8)) {
            return None;
        }
        // Skip the opcode and the table's byte length.
        let table = at + 3;
        let count = u16_at(table)?;
        let names = table + 2;
        let first_value = arg_top.checked_sub(argc)?.checked_sub(count)?;
        let mut args = Vec::with_capacity(count);
        for n in 0..count {
            let start = names + u16_at(names + n * 2)?;
            let end = names + u16_at(names + n * 2 + 2)?;
            args.push((code.get(start..end)?.to_vec(), first_value + n));
        }
        Some(args)
    }

    /// `t3GetNamedArg(name, default?)`: the value of a named argument
    /// passed to any function on the call stack.
    pub(crate) fn t3_get_named_arg(&mut self, argc: u32) -> VmResult<()> {
        if !(1..=2).contains(&argc) {
            return Err(VmError::new(code::WRONG_NUM_OF_ARGS));
        }
        let name = self.require_str(self.arg(0))?;
        for index in (0..self.frames.len()).rev() {
            let Some(args) = self.named_args_of_frame(index) else {
                continue;
            };
            for (arg_name, slot) in args {
                if arg_name == *name {
                    self.r0 = self.stack[slot];
                    return Ok(());
                }
            }
        }
        if argc >= 2 {
            self.r0 = self.arg(1);
            Ok(())
        } else {
            Err(VmError::with(
                code::MISSING_NAMED_ARG,
                vec![ErrParam::Str(String::from_utf8_lossy(&name).into_owned())],
            ))
        }
    }

    /// `t3GetNamedArgList()`: the names of all named arguments in effect.
    pub(crate) fn t3_get_named_arg_list(&mut self, argc: u32) -> VmResult<()> {
        if argc != 0 {
            return Err(VmError::new(code::WRONG_NUM_OF_ARGS));
        }
        let mut names: Vec<Vec<u8>> = Vec::new();
        for index in (0..self.frames.len()).rev() {
            let Some(args) = self.named_args_of_frame(index) else {
                continue;
            };
            for (name, _) in args.into_iter().rev() {
                if !names.contains(&name) {
                    names.push(name);
                }
            }
        }
        let mut items = Vec::with_capacity(names.len());
        for name in names {
            let string = self.new_string(name);
            self.push(string);
            items.push(string);
        }
        self.discard(items.len());
        self.r0 = self.new_list(items);
        Ok(())
    }
}
