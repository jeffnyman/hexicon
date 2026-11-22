//! The DynamicFunc intrinsic class: functions compiled while the program
//! runs, from source text, against the program's own global names.
//!
//! The code of each is put after the program's own, at an address of its
//! own, so that it runs like any other function. A function compiled can
//! have functions nested in it, each a DynamicFunc of its own, which its
//! code refers to by object.

use std::rc::Rc;

use hexicon_compiler::dynamic::{
    DynContext, DynHost, DynMode, DynObjKind, DynSymbol, compile_dynamic,
};
use hexicon_compiler::pp::LinkMacro;

use crate::error::{VmError, VmResult, code};
use crate::object::Object;
use crate::value::{NativeDesc, ObjId, Value};
use crate::vm::Vm;

/// A function compiled while the program runs.
#[derive(Debug, Clone)]
pub struct DynFunc {
    /// Where its code is, among the VM's code.
    pub addr: u32,
    /// The code: its header, its instructions, and its tables.
    pub code: Rc<[u8]>,
    /// Where in the code the numbers of objects it refers to are.
    pub obj_refs: Vec<u16>,
    /// The source it was compiled from, or nil for a function nested in
    /// another's source.
    pub source: Value,
    /// The method context it runs in, from the frame it was compiled
    /// in, or nil.
    pub method_ctx: Value,
}

impl DynFunc {
    /// A function read from an image or a saved game: the length of the
    /// code, the number of objects it refers to, the method context, the
    /// source, the code, and where in it each object's number is. The
    /// values are read with `value`. Its code has no place yet: see
    /// [`Vm::place_dynamic_code`].
    pub(crate) fn read(data: &[u8], value: &mut dyn FnMut(&[u8]) -> Option<Value>) -> Option<Self> {
        let u16_at = |at: usize| -> Option<usize> {
            let bytes = data.get(at..at + 2)?;
            Some(usize::from(u16::from_le_bytes([bytes[0], bytes[1]])))
        };
        let len = u16_at(0)?;
        let refs = u16_at(2)?;
        let holder = Value::HOLDER_SIZE;
        let method_ctx = value(data.get(4..4 + holder)?)?;
        let source = value(data.get(4 + holder..4 + 2 * holder)?)?;
        let start = 4 + 2 * holder;
        let code = data.get(start..start + len)?;
        let mut obj_refs = Vec::with_capacity(refs);
        for index in 0..refs {
            obj_refs.push(u16_at(start + len + 2 * index)? as u16);
        }
        Some(Self {
            addr: UNPLACED,
            code: code.into(),
            obj_refs,
            source,
            method_ctx,
        })
    }

    /// The size of what [`DynFunc::read`] reads.
    pub(crate) fn data_len(data: &[u8]) -> Option<usize> {
        let len = usize::from(u16::from_le_bytes([*data.first()?, *data.get(1)?]));
        let refs = usize::from(u16::from_le_bytes([*data.get(2)?, *data.get(3)?]));
        Some(4 + 2 * Value::HOLDER_SIZE + len + 2 * refs)
    }

    /// A function not compiled yet: the object is made first, so that the
    /// code that refers to it can be.
    fn placeholder() -> Self {
        Self {
            addr: 0,
            code: Rc::from([]),
            obj_refs: Vec::new(),
            source: Value::Nil,
            method_ctx: Value::Nil,
        }
    }

    /// The objects its code refers to.
    pub(crate) fn object_refs(&self) -> impl Iterator<Item = ObjId> + '_ {
        self.obj_refs.iter().filter_map(|&at| {
            let at = usize::from(at);
            let bytes = self.code.get(at..at + 4)?;
            Some(ObjId(u32::from_le_bytes([
                bytes[0], bytes[1], bytes[2], bytes[3],
            ])))
        })
    }
}

/// The address a function read from an image or a saved game has until
/// its code is put with the VM's.
pub(crate) const UNPLACED: u32 = u32::MAX;

/// The name and calling interface of each method, by method number.
const METHODS: &[(&str, NativeDesc)] = &[
    ("", NativeDesc::fixed(0)),
    ("getSource", NativeDesc::fixed(0)),
];

/// What the compiler asks of the VM: objects for the functions.
struct FuncObjects<'v> {
    vm: &'v mut Vm,
}

impl DynHost for FuncObjects<'_> {
    fn new_func_ids(&mut self, count: usize) -> Vec<u32> {
        (0..count)
            .map(|_| {
                self.vm
                    .alloc_obj(Object::DynFunc(Box::new(DynFunc::placeholder())))
                    .0
            })
            .collect()
    }
}

impl Vm {
    /// `new DynamicFunc(source, globals?, locals?, macros?)`: compile the
    /// source, against the table of global names and the table of macros
    /// given, which are what `t3GetGlobalSymbols` gives.
    pub(crate) fn create_dynfunc(&mut self, argc: u32) -> VmResult<ObjId> {
        if !(1..=4).contains(&argc) {
            return Err(VmError::new(code::WRONG_NUM_OF_ARGS));
        }
        let source = self.arg(0);
        let table = |vm: &Vm, index: u32| -> VmResult<Option<ObjId>> {
            if index >= argc {
                return Ok(None);
            }
            match vm.arg(index as usize) {
                Value::Obj(id) => Ok(Some(id)),
                Value::Nil => Ok(None),
                _ => Err(VmError::new(code::BAD_TYPE_BIF)),
            }
        };
        let globals = table(self, 1)?;
        let _locals = table(self, 2)?;
        let macros = table(self, 3)?;
        let Some(text) = self.as_str(source) else {
            return Err(VmError::new(code::BAD_TYPE_BIF));
        };
        let text = text.to_vec();
        let context = self.dyn_context(globals, macros);
        let result = compile_dynamic(
            &text,
            &context,
            DynMode::Auto,
            &mut FuncObjects { vm: self },
        );
        let compiled = match result {
            Ok(compiled) => compiled,
            Err(message) => {
                self.discard(argc as usize);
                return Err(self.throw_compiler_error(&message));
            }
        };
        self.discard(argc as usize);
        let mut main = None;
        for (index, (id, func)) in compiled.funcs.into_iter().enumerate() {
            let id = ObjId(id);
            let addr = self.add_dynamic_code(&func.code);
            let dynfunc = DynFunc {
                addr,
                code: func.code.into(),
                obj_refs: func.obj_refs.iter().map(|&at| at as u16).collect(),
                source: if index == 0 { source } else { Value::Nil },
                method_ctx: Value::Nil,
            };
            self.objects.replace(id, Object::DynFunc(Box::new(dynfunc)));
            main.get_or_insert(id);
        }
        main.ok_or_else(|| VmError::new(code::BAD_TYPE_BIF))
    }

    /// The exception a compilation that failed throws: the program's
    /// `CompilerException`, with the compiler's messages.
    fn throw_compiler_error(&mut self, message: &str) -> VmError {
        let class = self.predef.compiler_exc;
        if !class.is_valid() {
            return self.throw_class(class, "code compilation failed");
        }
        let text = self.new_str(message);
        self.push(text);
        match self.create_instance(class, 1) {
            Err(error) => error,
            Ok(()) => match self.r0 {
                Value::Obj(exception) => VmError::Thrown(exception),
                _ => VmError::new(code::EXCEPTION_OBJ_REQD),
            },
        }
    }

    /// Put the code of the functions read from an image or a saved game
    /// with the VM's, each at an address of its own.
    pub(crate) fn place_dynamic_code(&mut self) {
        let ids: Vec<ObjId> = self
            .objects
            .ids()
            .filter(|&id| {
                matches!(self.objects.get(id), Some(Object::DynFunc(func)) if func.addr == UNPLACED)
            })
            .collect();
        for id in ids {
            let code = match self.objects.get(id) {
                Some(Object::DynFunc(func)) => Rc::clone(&func.code),
                _ => continue,
            };
            let addr = self.add_dynamic_code(&code);
            if let Some(Object::DynFunc(func)) = self.objects.get_mut(id) {
                func.addr = addr;
            }
        }
    }

    /// Put code after the VM's own, and say where it is.
    pub(crate) fn add_dynamic_code(&mut self, code: &[u8]) -> u32 {
        let mut all = Vec::with_capacity(self.code.len() + code.len());
        all.extend_from_slice(&self.code);
        let addr = all.len() as u32;
        all.extend_from_slice(code);
        self.code = all.into();
        addr
    }

    /// What code is compiled against: the global names in a table of
    /// them, the macros in a table of them, and the image's classes.
    fn dyn_context(&self, globals: Option<ObjId>, macros: Option<ObjId>) -> DynContext {
        let mut context = DynContext::default();
        for index in 0..self.meta.len() {
            if let Some(entry) = self.meta.get(index) {
                context
                    .metaclasses
                    .push(entry.image_name.as_bytes().to_vec());
            }
        }
        if let Some(Object::LookupTable(table)) = globals.and_then(|id| self.objects.get(id)) {
            for (key, value) in table.entries() {
                let Some(name) = self.as_str(key) else {
                    continue;
                };
                if let Some(symbol) = self.dyn_symbol(value) {
                    context.symbols.push((name.to_vec(), symbol));
                }
            }
        }
        if let Some(Object::LookupTable(table)) = macros.and_then(|id| self.objects.get(id)) {
            for (key, value) in table.entries() {
                if let (Some(name), Some(mac)) = (self.as_str(key), self.dyn_macro(value)) {
                    context.macros.push(LinkMacro {
                        name: name.to_vec(),
                        ..mac
                    });
                }
            }
        }
        context
    }

    /// What a value in the table of global names stands for, to the
    /// compiler, if it is something the compiler can use.
    fn dyn_symbol(&self, value: Value) -> Option<DynSymbol> {
        Some(match value {
            Value::Obj(id) => match self.objects.get(id)? {
                Object::Class(class) => DynSymbol::Class {
                    meta_index: class.meta_index,
                    id: id.0,
                },
                object => {
                    let kind = if self.invoker(id).is_some() {
                        DynObjKind::Invokable
                    } else {
                        match object {
                            Object::Tads(_) => DynObjKind::TadsObject,
                            Object::GramProd(_) => DynObjKind::GrammarProd,
                            Object::Dict(_) => DynObjKind::Dictionary,
                            Object::IntClassMod(_) => DynObjKind::ClassModifier,
                            _ => DynObjKind::Other,
                        }
                    };
                    DynSymbol::Object { id: id.0, kind }
                }
            },
            Value::Prop(prop) => DynSymbol::Property(prop),
            Value::FuncPtr(addr) => {
                let header = self.code.get(addr as usize..addr as usize + 2)?;
                DynSymbol::Function {
                    addr,
                    argc: usize::from(header[0] & 0x7f),
                    opt_argc: usize::from(header[1]),
                    varargs: header[0] & 0x80 != 0,
                }
            }
            Value::Enum(id) => DynSymbol::Enum(id),
            Value::BifPtr { set, index } => {
                let desc = self.bif_desc(set, index)?;
                DynSymbol::BuiltIn {
                    set,
                    index,
                    min_argc: usize::from(desc.min_args),
                    opt_argc: usize::from(desc.opt_args),
                    varargs: desc.varargs,
                }
            }
            _ => return None,
        })
    }

    /// A macro as the table of macros has it: a list of its expansion,
    /// the list of its parameters' names, and its flags.
    fn dyn_macro(&self, value: Value) -> Option<LinkMacro> {
        let items = self.list_items(value).ok()?;
        if items.len() < 3 {
            return None;
        }
        let expansion = self.as_str(items[0])?.to_vec();
        let mut params = Vec::new();
        for param in self.list_items(items[1]).ok()? {
            params.push(self.as_str(param)?.to_vec());
        }
        let Value::Int(flags) = items[2] else {
            return None;
        };
        Some(LinkMacro {
            name: Vec::new(),
            has_args: flags & 1 != 0,
            has_varargs: flags & 2 != 0,
            params,
            expansion,
        })
    }

    /// Call a method of the DynamicFunc class.
    pub(crate) fn dynfunc_method(
        &mut self,
        id: ObjId,
        index: u16,
        argc: Option<&mut u32>,
    ) -> VmResult<Option<Value>> {
        let Some(&(_, desc)) = METHODS.get(index as usize).filter(|_| index != 0) else {
            return Ok(None);
        };
        self.native_method(argc, desc, |vm, _| {
            Ok(match vm.objects.get(id) {
                Some(Object::DynFunc(func)) => func.source,
                _ => Value::Nil,
            })
        })
        .map(Some)
    }
}
