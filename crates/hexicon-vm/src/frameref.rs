//! References to stack frames: `StackFrameDesc`, which the program sees,
//! and `StackFrameRef`, which it refers to.
//!
//! A `StackFrameRef` stands for one call of a function. While the call
//! runs, its variables are read and written on the stack; when the call
//! returns, they are copied into the object, which keeps them from then
//! on. A frame has one at most, made when a stack trace first asks for
//! it. A `StackFrameDesc` is a place in that call: the scope of the code
//! there, which says which variables have which names.

use crate::error::{VmError, VmResult, code};
use crate::lookup::LookupTable;
use crate::object::Object;
use crate::value::{INVALID_PROP, NativeDesc, ObjId, PropId, Value};
use crate::vm::Vm;

/// A call of a function, while it runs and after.
#[derive(Debug, Clone)]
pub(crate) struct FrameRef {
    /// The frame of the call, by its place on the frame stack, while it
    /// runs.
    pub frame: Option<usize>,
    /// The function: a function pointer, or the DynamicFunc for code
    /// compiled while the program runs.
    pub entry: Value,
    /// How many local variables the function has.
    pub nlocals: usize,
    /// How many arguments the call was given.
    pub nparams: usize,
    /// The method context of the call: `self`, the object the method was
    /// found in, the object it was looked up on, and the property.
    pub self_value: Value,
    pub definer: ObjId,
    pub target: ObjId,
    pub prop: PropId,
    /// What was called.
    pub invokee: Value,
    /// After the call returns, its locals and then its arguments.
    pub vars: Vec<Value>,
}

/// A place in a call: the call, the scope of the code there (0 if the
/// function records none), and the offset of the code.
#[derive(Debug, Clone, Copy)]
pub(crate) struct FrameDesc {
    pub fref: ObjId,
    pub scope: u16,
    pub offset: u16,
}

/// A variable a scope gives a name.
#[derive(Debug, Clone)]
pub(crate) struct LocalSym {
    /// The name.
    pub name: Vec<u8>,
    /// The name's place in the constant pool, if it is kept there.
    pub pool: Option<u32>,
    /// The number of the local or the argument.
    pub number: usize,
    /// Whether it is an argument.
    pub is_param: bool,
    /// For a variable shared with an anonymous function, its index in the
    /// context object the local holds.
    pub ctx: Option<usize>,
}

/// The name and calling interface of each method, by method number.
const METHODS: &[(&str, NativeDesc)] = &[
    ("", NativeDesc::fixed(0)),
    ("isActive", NativeDesc::fixed(0)),
    ("getVars", NativeDesc::fixed(0)),
    ("getSelf", NativeDesc::fixed(0)),
    ("getDefiningObj", NativeDesc::fixed(0)),
    ("getTargetObj", NativeDesc::fixed(0)),
    ("getTargetProp", NativeDesc::fixed(0)),
    ("getInvokee", NativeDesc::fixed(0)),
];

impl FrameRef {
    /// The size of the object in an image or a saved game.
    pub(crate) fn data_len(data: &[u8]) -> Option<usize> {
        let nlocals = usize::from(u16::from_le_bytes([*data.first()?, *data.get(1)?]));
        let nparams = usize::from(u16::from_le_bytes([*data.get(2)?, *data.get(3)?]));
        Some(4 + 3 * Value::HOLDER_SIZE + 10 + (nlocals + nparams) * Value::HOLDER_SIZE)
    }

    /// The object as an image or a saved game has it: the number of
    /// locals and of arguments, the function, `self`, the object the
    /// method was found in, the object it was looked up on, the property,
    /// what was called, and the variables. The values are read with
    /// `value`; object numbers with `obj`. A call read so has returned.
    pub(crate) fn read(
        data: &[u8],
        value: &mut dyn FnMut(&[u8]) -> Option<Value>,
        obj: &dyn Fn(ObjId) -> ObjId,
    ) -> Option<Self> {
        let u16_at = |at: usize| -> Option<usize> {
            let b = data.get(at..at + 2)?;
            Some(usize::from(u16::from_le_bytes([b[0], b[1]])))
        };
        let u32_at = |at: usize| -> Option<u32> {
            let b = data.get(at..at + 4)?;
            Some(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        };
        let holder = Value::HOLDER_SIZE;
        let nlocals = u16_at(0)?;
        let nparams = u16_at(2)?;
        let mut at = 4;
        let entry = value(data.get(at..at + holder)?)?;
        at += holder;
        let self_value = value(data.get(at..at + holder)?)?;
        at += holder;
        let fix = |id: u32| {
            let id = ObjId(id);
            if id.is_valid() { obj(id) } else { id }
        };
        let definer = fix(u32_at(at)?);
        let target = fix(u32_at(at + 4)?);
        let prop = u16_at(at + 8)? as PropId;
        at += 10;
        let invokee = value(data.get(at..at + holder)?)?;
        at += holder;
        let mut vars = Vec::with_capacity(nlocals + nparams);
        for _ in 0..nlocals + nparams {
            vars.push(value(data.get(at..at + holder)?)?);
            at += holder;
        }
        Some(Self {
            frame: None,
            entry,
            nlocals,
            nparams,
            self_value,
            definer,
            target,
            prop,
            invokee,
            vars,
        })
    }
}

impl FrameDesc {
    /// The object as an image or a saved game has it: the call, the
    /// scope, and the offset.
    pub(crate) fn read(data: &[u8]) -> Option<Self> {
        let b = data.get(..8)?;
        Some(Self {
            fref: ObjId(u32::from_le_bytes([b[0], b[1], b[2], b[3]])),
            scope: u16::from_le_bytes([b[4], b[5]]),
            offset: u16::from_le_bytes([b[6], b[7]]),
        })
    }
}

impl Vm {
    /// The address of a function's code from a value that calls it: a
    /// function pointer, or a DynamicFunc.
    pub(crate) fn entry_address(&self, entry: Value) -> Option<u32> {
        match entry {
            Value::FuncPtr(addr) | Value::CodeOfs(addr) => Some(addr),
            Value::Obj(id) => match self.objects.get(id) {
                Some(Object::DynFunc(func)) => Some(func.addr),
                _ => None,
            },
            _ => None,
        }
    }

    /// The value that stands for the function whose code is at an
    /// address: a function pointer, or for code compiled while the
    /// program runs, its DynamicFunc.
    pub(crate) fn function_value(&self, addr: u32) -> Value {
        if (addr as usize) < self.image_code_len {
            return Value::FuncPtr(addr);
        }
        let owner = self.objects.ids().find(
            |&id| matches!(self.objects.get(id), Some(Object::DynFunc(func)) if func.addr == addr),
        );
        owner.map_or(Value::Nil, Value::Obj)
    }

    /// The `StackFrameRef` of a frame, made the first time it is asked
    /// for.
    pub(crate) fn frame_ref_of(&mut self, frame: usize) -> ObjId {
        if let Some(id) = self.frames[frame].frame_ref {
            return id;
        }
        let f = &self.frames[frame];
        let nlocals = self
            .function_header(f.func)
            .map_or(0, |h| h.locals as usize);
        let fref = FrameRef {
            frame: Some(frame),
            entry: self.function_value(f.func),
            nlocals,
            nparams: f.argc as usize,
            self_value: f.self_value,
            definer: f.definer,
            target: f.target.obj_id(),
            prop: f.prop,
            invokee: f.invokee,
            vars: Vec::new(),
        };
        let id = self.alloc_obj(Object::FrameRef(Box::new(fref)));
        self.frames[frame].frame_ref = Some(id);
        id
    }

    /// Copy a returning frame's variables into its `StackFrameRef`.
    pub(crate) fn snapshot_frame(&mut self, id: ObjId) {
        let vars = self.frame_vars(id);
        if let Some(Object::FrameRef(fref)) = self.objects.get_mut(id)
            && let Some(vars) = vars
        {
            fref.vars = vars;
            fref.frame = None;
        }
    }

    /// The variables of a call that is running: its locals, then its
    /// arguments.
    pub(crate) fn frame_vars(&self, id: ObjId) -> Option<Vec<Value>> {
        let Some(Object::FrameRef(fref)) = self.objects.get(id) else {
            return None;
        };
        let frame = &self.frames[fref.frame?];
        let base = frame.base as usize;
        let mut vars = Vec::with_capacity(fref.nlocals + fref.nparams);
        for n in 0..fref.nlocals {
            vars.push(self.stack.get(base + n).copied().unwrap_or(Value::Nil));
        }
        for n in 0..fref.nparams {
            let slot = base.checked_sub(1 + n);
            vars.push(
                slot.and_then(|s| self.stack.get(s))
                    .copied()
                    .unwrap_or(Value::Nil),
            );
        }
        Some(vars)
    }

    /// The stack slot, or the place in the copy, of variable `n` of a
    /// call: its locals first, then its arguments.
    fn frame_var_slot(&self, fref: &FrameRef, n: usize) -> Option<usize> {
        let frame = &self.frames[fref.frame?];
        let base = frame.base as usize;
        if n < fref.nlocals {
            Some(base + n)
        } else {
            base.checked_sub(1 + n - fref.nlocals)
        }
    }

    /// Variable `n` of a call.
    fn frame_var(&self, id: ObjId, n: usize) -> Value {
        let Some(Object::FrameRef(fref)) = self.objects.get(id) else {
            return Value::Nil;
        };
        if fref.frame.is_some() {
            self.frame_var_slot(fref, n)
                .and_then(|slot| self.stack.get(slot))
                .copied()
                .unwrap_or(Value::Nil)
        } else {
            fref.vars.get(n).copied().unwrap_or(Value::Nil)
        }
    }

    /// Set variable `n` of a call.
    fn set_frame_var(&mut self, id: ObjId, n: usize, value: Value) {
        let slot = match self.objects.get(id) {
            Some(Object::FrameRef(fref)) if fref.frame.is_some() => self.frame_var_slot(fref, n),
            _ => None,
        };
        match slot {
            Some(slot) => {
                if let Some(place) = self.stack.get_mut(slot) {
                    *place = value;
                }
            }
            None => {
                if let Some(Object::FrameRef(fref)) = self.objects.get_mut(id)
                    && let Some(place) = fref.vars.get_mut(n)
                {
                    *place = value;
                }
            }
        }
    }

    /// `fref[n]`: variable `n` of a call, its locals first and then its
    /// arguments.
    pub(crate) fn frame_ref_index(&mut self, id: ObjId, index: Value) -> VmResult<Value> {
        let n = self.frame_ref_var_number(id, index)?;
        Ok(self.frame_var(id, n))
    }

    /// `fref[n] = value`.
    pub(crate) fn frame_ref_set_index(
        &mut self,
        id: ObjId,
        index: Value,
        value: Value,
    ) -> VmResult<Value> {
        let n = self.frame_ref_var_number(id, index)?;
        self.set_frame_var(id, n, value);
        Ok(Value::Obj(id))
    }

    fn frame_ref_var_number(&self, id: ObjId, index: Value) -> VmResult<usize> {
        let Some(Object::FrameRef(fref)) = self.objects.get(id) else {
            return Err(VmError::new(code::INDEX_OUT_OF_RANGE));
        };
        match index {
            Value::Int(n) if n >= 0 && (n as usize) < fref.nlocals + fref.nparams => Ok(n as usize),
            _ => Err(VmError::new(code::INDEX_OUT_OF_RANGE)),
        }
    }

    /// The number a variable has among a call's variables: its locals,
    /// then its arguments.
    pub(crate) fn frame_var_number(&self, fref: ObjId, sym: &LocalSym) -> usize {
        let nlocals = match self.objects.get(fref) {
            Some(Object::FrameRef(f)) => f.nlocals,
            _ => 0,
        };
        sym.number + if sym.is_param { nlocals } else { 0 }
    }

    /// The value of a named variable of a call.
    fn frame_sym_value(&mut self, fref: ObjId, sym: &LocalSym) -> VmResult<Value> {
        let stored = self.frame_var(fref, self.frame_var_number(fref, sym));
        match sym.ctx {
            Some(index) => self.list_like_get(stored, index),
            None => Ok(stored),
        }
    }

    /// Set a named variable of a call.
    fn set_frame_sym_value(&mut self, fref: ObjId, sym: &LocalSym, value: Value) -> VmResult<()> {
        let n = self.frame_var_number(fref, sym);
        match sym.ctx {
            Some(index) => {
                let holder = self.frame_var(fref, n);
                if !matches!(holder, Value::Obj(_)) {
                    return Err(VmError::new(code::CANNOT_INDEX_TYPE));
                }
                self.set_index(holder, Value::Int(index as i32), value)?;
            }
            None => self.set_frame_var(fref, n, value),
        }
        Ok(())
    }

    /// The `FrameDesc` of an object.
    pub(crate) fn frame_desc(&self, id: ObjId) -> Option<FrameDesc> {
        match self.objects.get(id) {
            Some(Object::FrameDesc(desc)) => Some(*desc),
            _ => None,
        }
    }

    /// The variables in scope at a place in a call, innermost first.
    pub(crate) fn frame_desc_syms(&self, desc: FrameDesc) -> Vec<LocalSym> {
        let entry = match self.objects.get(desc.fref) {
            Some(Object::FrameRef(fref)) => fref.entry,
            _ => return Vec::new(),
        };
        match self.entry_address(entry) {
            Some(func) => self.scope_syms(func, desc.scope),
            None => Vec::new(),
        }
    }

    /// Find a variable by name at a place in a call.
    pub(crate) fn frame_desc_find(&self, desc: FrameDesc, name: &[u8]) -> Option<LocalSym> {
        if desc.scope == 0 {
            return None;
        }
        self.frame_desc_syms(desc)
            .into_iter()
            .find(|sym| sym.name == name)
    }

    /// `desc[name]`: the value of a variable by name.
    pub(crate) fn frame_desc_index(&mut self, id: ObjId, index: Value) -> VmResult<Value> {
        let sym = self.frame_desc_lookup(id, index)?;
        let desc = self.frame_desc(id).expect("a frame descriptor");
        self.frame_sym_value(desc.fref, &sym)
    }

    /// `desc[name] = value`.
    pub(crate) fn frame_desc_set_index(
        &mut self,
        id: ObjId,
        index: Value,
        value: Value,
    ) -> VmResult<Value> {
        let sym = self.frame_desc_lookup(id, index)?;
        let desc = self.frame_desc(id).expect("a frame descriptor");
        self.set_frame_sym_value(desc.fref, &sym, value)?;
        Ok(Value::Obj(id))
    }

    fn frame_desc_lookup(&self, id: ObjId, index: Value) -> VmResult<LocalSym> {
        let desc = self.frame_desc(id).expect("a frame descriptor");
        let Some(name) = self.as_str(index) else {
            return Err(VmError::new(code::INDEX_OUT_OF_RANGE));
        };
        self.frame_desc_find(desc, &name)
            .ok_or_else(|| VmError::new(code::INDEX_OUT_OF_RANGE))
    }

    /// The method context of a call as `loadctx` takes it.
    pub(crate) fn frame_ref_loadctx(&mut self, fref: ObjId) -> Value {
        let (self_value, prop, target, definer) = self.frame_ref_context(fref);
        let self_value = match self_value {
            Value::Obj(_) => self_value,
            _ => Value::Nil,
        };
        self.new_list(vec![
            self_value,
            Value::Prop(prop),
            Value::obj_or_nil(target),
            Value::obj_or_nil(definer),
        ])
    }

    /// `self`, the property, the object the method was looked up on, and
    /// the one it was found in, of a call.
    fn frame_ref_context(&self, fref: ObjId) -> (Value, PropId, ObjId, ObjId) {
        let Some(Object::FrameRef(f)) = self.objects.get(fref) else {
            return (Value::Nil, INVALID_PROP, ObjId::INVALID, ObjId::INVALID);
        };
        match f.frame {
            Some(index) => {
                let frame = &self.frames[index];
                (
                    frame.self_value,
                    frame.prop,
                    frame.target.obj_id(),
                    frame.definer,
                )
            }
            None => (f.self_value, f.prop, f.target, f.definer),
        }
    }

    /// `self` in a call.
    pub(crate) fn frame_ref_self(&self, fref: ObjId) -> Value {
        match self.frame_ref_context(fref).0 {
            Value::Obj(id) if !id.is_valid() => Value::Nil,
            other => other,
        }
    }

    /// What was called, in a call.
    fn frame_ref_invokee(&self, fref: ObjId) -> Value {
        let Some(Object::FrameRef(f)) = self.objects.get(fref) else {
            return Value::Nil;
        };
        match f.frame {
            Some(index) => self.frames[index].invokee,
            None => f.invokee,
        }
    }

    /// A table of the variables in scope at a place in a call, by name.
    fn frame_desc_vars(&mut self, id: ObjId) -> VmResult<Value> {
        let table = self.new_table(LookupTable::new(32, 64, false));
        let result = Value::Obj(table);
        let desc = self.frame_desc(id).expect("a frame descriptor");
        if desc.scope == 0 {
            return Ok(result);
        }
        self.push(result);
        let syms = self.frame_desc_syms(desc);
        let filled = (|| {
            for sym in &syms {
                let key = match sym.pool {
                    Some(at) => Value::SString(at),
                    None => self.new_string(sym.name.clone()),
                };
                self.push(key);
                let present = self.table_get(table, key)?.is_some();
                if !present {
                    let value = self.frame_sym_value(desc.fref, sym)?;
                    self.table_set(table, key, value)?;
                }
                self.discard(1);
            }
            Ok(())
        })();
        self.discard(1);
        filled.map(|()| result)
    }

    /// Evaluate a method of a `StackFrameDesc`.
    pub(crate) fn frame_desc_method(
        &mut self,
        id: ObjId,
        index: u16,
        argc: Option<&mut u32>,
    ) -> VmResult<Option<Value>> {
        let Some(&(_, desc)) = METHODS.get(index as usize).filter(|_| index != 0) else {
            return Ok(None);
        };
        self.native_method(argc, desc, |vm, _| {
            let fref = vm.frame_desc(id).expect("a frame descriptor").fref;
            Ok(match index {
                1 => Value::logical(matches!(
                    vm.objects.get(fref),
                    Some(Object::FrameRef(f)) if f.frame.is_some()
                )),
                2 => vm.frame_desc_vars(id)?,
                3 => vm.frame_ref_self(fref),
                4 => Value::obj_or_nil(vm.frame_ref_context(fref).3),
                5 => Value::obj_or_nil(vm.frame_ref_context(fref).2),
                6 => match vm.frame_ref_context(fref).1 {
                    INVALID_PROP => Value::Nil,
                    prop => Value::Prop(prop),
                },
                _ => vm.frame_ref_invokee(fref),
            })
        })
        .map(Some)
    }

    // ---- scopes ----

    /// The innermost scope of a function's code that covers an offset,
    /// or 0 if the function records none.
    pub(crate) fn scope_at(&self, func: u32, offset: u32) -> u16 {
        let Some(scopes) = self.scope_table(func) else {
            return 0;
        };
        let code = &self.code;
        let offset = offset as usize;
        let mut best = 0;
        for index in 1..=scopes.count {
            let frame = scopes.frame_at(code, index);
            let (start, end) = if scopes.header >= 8 {
                (u16_at(code, frame + 4), u16_at(code, frame + 6))
            } else {
                (0, 0)
            };
            if offset < start || offset > end {
                continue;
            }
            let mut nested = best == 0;
            let mut parent = u16_at(code, frame);
            let mut steps = 0;
            while !nested && parent != 0 && steps <= scopes.count {
                nested = parent == best;
                parent = u16_at(code, scopes.frame_at(code, parent));
                steps += 1;
            }
            if nested {
                best = index;
            }
        }
        best as u16
    }

    /// The variables of a scope and each one around it, innermost first.
    pub(crate) fn scope_syms(&self, func: u32, scope: u16) -> Vec<LocalSym> {
        let mut syms = Vec::new();
        let Some(scopes) = self.scope_table(func) else {
            return syms;
        };
        let code = &self.code;
        let local_header = self.layout.debug_local_header as usize;
        let mut best = usize::from(scope);
        let mut steps = 0;
        while best != 0 && best <= scopes.count && steps <= scopes.count {
            let frame = scopes.frame_at(code, best);
            let symbol_count = u16_at(code, frame + 2);
            let mut at = frame + scopes.header;
            for _ in 0..symbol_count {
                let number = u16_at(code, at);
                let flags = u16_at(code, at + 2);
                let context_index = u16_at(code, at + 4);
                let name_at = at + local_header;
                let (name, pool) = if flags & 4 == 0 {
                    let len = u16_at(code, name_at);
                    at = name_at + 2 + len;
                    (
                        code.get(name_at + 2..name_at + 2 + len)
                            .unwrap_or(&[])
                            .to_vec(),
                        None,
                    )
                } else {
                    let pointer = code
                        .get(name_at..name_at + 4)
                        .map_or(0, |b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]));
                    at = name_at + 4;
                    (self.const_str(pointer).to_vec(), Some(pointer))
                };
                let in_context = flags & 2 != 0;
                syms.push(LocalSym {
                    name,
                    pool,
                    number,
                    is_param: flags & 1 != 0 && !in_context,
                    ctx: in_context.then_some(context_index),
                });
            }
            best = u16_at(code, frame);
            steps += 1;
        }
        syms
    }

    /// Where a function's scopes are recorded.
    fn scope_table(&self, func: u32) -> Option<ScopeTable> {
        let header = self.function_header(func).ok()?;
        if header.debug_records == 0 {
            return None;
        }
        let debug = (func + header.debug_records) as usize;
        let layout = self.layout;
        let table_header = layout.debug_table_header as usize;
        let line_count = u16_at(&self.code, debug + table_header);
        let frames = debug + table_header + 2 + line_count * layout.debug_line_entry as usize + 2;
        Some(ScopeTable {
            frames,
            count: u16_at(&self.code, frames),
            header: layout.debug_frame_header as usize,
        })
    }
}

/// A function's table of scopes.
struct ScopeTable {
    /// Where the table is.
    frames: usize,
    /// How many scopes there are.
    count: usize,
    /// The size of a scope's header.
    header: usize,
}

impl ScopeTable {
    /// Where scope `index` (from 1) is.
    fn frame_at(&self, code: &[u8], index: usize) -> usize {
        let entry = self.frames + 2 + 2 * (index - 1);
        entry + u16_at(code, entry)
    }
}

fn u16_at(code: &[u8], at: usize) -> usize {
    code.get(at..at + 2)
        .map_or(0, |b| usize::from(u16::from_le_bytes([b[0], b[1]])))
}
