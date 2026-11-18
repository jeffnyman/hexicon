//! Setting up a VM from a program image, and running the program.

use std::rc::Rc;

use hexicon_host::Host;
use hexicon_image::{Image, Pool, Value as ImageValue};

use crate::bif::FunctionSet;
use crate::error::{ErrParam, VmError, VmResult, code};
use crate::meta::{MetaKind, MetaTable};
use crate::object::{IntrinsicClass, Object, ObjectTable};
use crate::tads::TadsObject;
use crate::value::{INVALID_PROP, ObjId, PropId, Value};
use crate::vm::{Layout, MethodContext, NativeCall, Predef, Return, STACK_LIMIT, Vm};

/// Width of the console in columns. The reference VM's plain console is
/// 80 columns wide.
const CONSOLE_WIDTH: usize = 80;

/// Copy a pool into one contiguous array, so that a pool address is an
/// index. Pages the image does not supply read as zeros.
fn flatten(pool: &Pool) -> Rc<[u8]> {
    let page_size = pool.page_size() as usize;
    let mut data = vec![0u8; page_size * pool.page_count()];
    for index in 0..pool.page_count() {
        if let Some(page) = pool.page(index) {
            let len = page.len().min(page_size);
            data[index * page_size..index * page_size + len].copy_from_slice(&page[..len]);
        }
    }
    data.into()
}

fn bad_export(name: &str) -> VmError {
    VmError::with(
        code::INVAL_EXPORT_TYPE,
        vec![ErrParam::Str(name.to_owned())],
    )
}

fn object_symbol(image: &Image, name: &str) -> VmResult<ObjId> {
    match image.symbol(name) {
        None => Ok(ObjId::INVALID),
        Some(ImageValue::Object(id)) => Ok(ObjId(id)),
        Some(_) => Err(bad_export(name)),
    }
}

fn prop_symbol(image: &Image, name: &str) -> VmResult<PropId> {
    match image.symbol(name) {
        None => Ok(INVALID_PROP),
        Some(ImageValue::Property(id)) => Ok(id),
        Some(_) => Err(bad_export(name)),
    }
}

fn function_symbol(image: &Image, name: &str) -> VmResult<u32> {
    match image.symbol(name) {
        None => Ok(0),
        Some(ImageValue::FunctionPointer(address)) => Ok(address),
        Some(_) => Err(bad_export(name)),
    }
}

/// Build an object from its image data.
fn load_object(kind: MetaKind, meta_index: u16, data: &[u8]) -> VmResult<Object> {
    let bad = || VmError::new(code::INVAL_METACLASS_DATA);
    let u16_at = |at: usize| -> VmResult<u16> {
        let b = data.get(at..at + 2).ok_or_else(bad)?;
        Ok(u16::from_le_bytes([b[0], b[1]]))
    };
    Ok(match kind {
        MetaKind::TadsObject => Object::Tads(TadsObject::from_image(data).ok_or_else(bad)?),
        MetaKind::IntClassMod => Object::IntClassMod(TadsObject::from_image(data).ok_or_else(bad)?),
        MetaKind::String => {
            let len = u16_at(0)? as usize;
            Object::String(data.get(2..2 + len).ok_or_else(bad)?.into())
        }
        MetaKind::List => {
            let count = u16_at(0)? as usize;
            let items = data
                .get(2..2 + count * Value::HOLDER_SIZE)
                .ok_or_else(bad)?
                .chunks_exact(Value::HOLDER_SIZE)
                .map(Value::from_holder)
                .collect::<Vec<_>>();
            Object::List(items.into())
        }
        MetaKind::Vector | MetaKind::AnonFuncPtr => {
            // An allocation size, an element count, and the elements.
            // Elements the data does not supply are nil.
            let count = u16_at(2)? as usize;
            let mut items: Vec<Value> = data
                .get(4..)
                .unwrap_or(&[])
                .chunks_exact(Value::HOLDER_SIZE)
                .take(count)
                .map(Value::from_holder)
                .collect();
            items.resize(count, Value::Nil);
            if kind == MetaKind::Vector {
                Object::Vector(items)
            } else {
                Object::AnonFn(items)
            }
        }
        MetaKind::IntrinsicClass => {
            // A length, the index of the class in the dependency list,
            // the modifier object, and optionally the class state.
            if data.len() < 8 {
                return Err(bad());
            }
            let modifier = u32::from_le_bytes([data[4], data[5], data[6], data[7]]);
            let class_state = match data.get(8..8 + Value::HOLDER_SIZE) {
                Some(holder) => Value::from_holder(holder),
                None => Value::Nil,
            };
            Object::Class(IntrinsicClass {
                meta_index: u16_at(2)?,
                modifier: ObjId(modifier),
                class_state,
            })
        }
        _ => Object::Opaque {
            meta_index,
            data: data.into(),
        },
    })
}

impl Vm {
    /// Load a program image.
    pub fn new(image: &Image, host: Box<dyn Host>) -> VmResult<Self> {
        if image.code_pool.page_count() < 1 {
            return Err(VmError::new(code::IMAGE_NO_CODE));
        }

        let mut meta = MetaTable::new();
        for dependency in &image.metaclasses {
            meta.add(&dependency.name, dependency.properties.clone())?;
        }
        meta.add_if_new(MetaKind::Root);

        let function_sets = image
            .function_sets
            .iter()
            .map(|name| FunctionSet::from_image_name(name))
            .collect::<VmResult<Vec<_>>>()?;

        let mut objects = ObjectTable::new();
        for block in &image.object_blocks {
            let Some(kind) = meta.get(block.metaclass_index as usize).map(|e| e.kind) else {
                return Err(VmError::new(code::BAD_METACLASS_INDEX));
            };
            for object in &block.objects {
                let id = ObjId(object.id);
                let loaded = load_object(kind, block.metaclass_index, &object.data)?;
                // An intrinsic class object tells the VM which object
                // stands for the class.
                if let Object::Class(class) = &loaded
                    && let Some(entry) = meta.get_mut(class.meta_index as usize)
                {
                    entry.class_obj = id;
                }
                if !objects.alloc_with_id(id, loaded, block.transient) {
                    return Err(VmError::new(code::OBJ_IN_USE));
                }
            }
        }

        let entry = &image.entry_point;
        let mut vm = Self {
            code: flatten(&image.code_pool),
            consts: flatten(&image.constant_pool),
            layout: Layout {
                method_header: entry.method_header_size.into(),
                exception_entry: entry.exception_entry_size.into(),
                debug_line_entry: entry.debug_line_entry_size.into(),
                debug_table_header: entry.debug_table_header_size.into(),
            },
            objects,
            meta,
            function_sets,
            predef: Predef::default(),
            stack: Vec::with_capacity(STACK_LIMIT),
            frames: Vec::new(),
            stack_limit: STACK_LIMIT,
            r0: Value::Nil,
            pc: 0,
            say_function: Value::Nil,
            say_method: INVALID_PROP,
            entry_point: entry.code_offset,
            static_initializers: image
                .static_initializers
                .iter()
                .flat_map(|list| &list.initializers)
                .map(|init| (ObjId(init.object), init.property))
                .collect(),
            debug: image.debug.clone(),
            preinit_mode: false,
            trace_errors: false,
            console: crate::console::Formatter::new(CONSOLE_WIDTH),
            host,
        };
        vm.link(image)?;
        Ok(vm)
    }

    /// Resolve the symbols the VM imports from the image, and create the
    /// objects the VM needs that the image does not supply.
    fn link(&mut self, image: &Image) -> VmResult<()> {
        self.predef = Predef {
            runtime_error: object_symbol(image, "RuntimeError")?,
            exception_message: prop_symbol(image, "exceptionMessage")?,
            construct: prop_symbol(image, "Constructor")?,
            destruct: prop_symbol(image, "Destructor")?,
            last_prop: prop_symbol(image, "LastProp")?,
            prop_not_defined: prop_symbol(image, "propNotDefined")?,
            stack_info_class: object_symbol(image, "T3StackInfo")?,
            main_restore: function_symbol(image, "mainRestore")?,
            length: prop_symbol(image, "length")?,
            const_str_obj: ObjId::INVALID,
            const_lst_obj: ObjId::INVALID,
            last_prop_obj: ObjId::INVALID,
            operator_add: prop_symbol(image, "operator +")?,
            operator_sub: prop_symbol(image, "operator -")?,
            operator_mul: prop_symbol(image, "operator *")?,
            operator_div: prop_symbol(image, "operator /")?,
            operator_mod: prop_symbol(image, "operator %")?,
            operator_xor: prop_symbol(image, "operator ^")?,
            operator_shl: prop_symbol(image, "operator <<")?,
            operator_ashr: prop_symbol(image, "operator >>")?,
            operator_lshr: prop_symbol(image, "operator >>>")?,
            operator_bit_not: prop_symbol(image, "operator ~")?,
            operator_bit_and: prop_symbol(image, "operator &")?,
            operator_bit_or: prop_symbol(image, "operator |")?,
            operator_neg: prop_symbol(image, "operator negate")?,
            operator_idx: prop_symbol(image, "operator []")?,
            operator_setidx: prop_symbol(image, "operator []=")?,
            reflection_services: object_symbol(image, "reflection.reflectionServices")?,
            reflection_val_to_symbol: prop_symbol(image, "reflection.valToSymbol")?,
            obj_to_string: prop_symbol(image, "objToString")?,
            iter_get_next: INVALID_PROP,
            iter_next_avail: INVALID_PROP,
        };

        // These are created in the same order as in the reference VM, so
        // that objects the program creates later get the same IDs.
        let last_prop = self.predef.last_prop;
        self.predef.last_prop_obj = self
            .objects
            .alloc(Object::Vector(vec![Value::Prop(last_prop)]));
        if self.predef.exception_message == INVALID_PROP {
            self.predef.exception_message = self.alloc_prop_for_vm()?;
        }
        self.predef.const_str_obj = self.objects.alloc(Object::String(Rc::from(&[][..])));
        self.predef.const_lst_obj = self.objects.alloc(Object::List(Rc::from(&[][..])));

        // Every intrinsic class needs an object to represent it.
        for index in 0..self.meta.len() {
            if self
                .meta
                .get(index)
                .is_some_and(|e| !e.class_obj.is_valid())
            {
                let id = self.objects.alloc(Object::Class(IntrinsicClass {
                    meta_index: index as u16,
                    modifier: ObjId::INVALID,
                    class_state: Value::Nil,
                }));
                if let Some(entry) = self.meta.get_mut(index) {
                    entry.class_obj = id;
                }
            }
        }

        if let Some(iterator) = self.meta.entry(MetaKind::Iterator) {
            self.predef.iter_get_next = iterator.method_prop(1);
            self.predef.iter_next_avail = iterator.method_prop(2);
        }
        Ok(())
    }

    fn alloc_prop_for_vm(&mut self) -> VmResult<PropId> {
        let holder = self.predef.last_prop_obj;
        match self.objects.get_mut(holder) {
            Some(Object::Vector(items)) => match items.first() {
                Some(Value::Prop(u16::MAX)) => Err(VmError::new(code::OUT_OF_PROPIDS)),
                Some(Value::Prop(last)) => {
                    let next = last + 1;
                    items[0] = Value::Prop(next);
                    Ok(next)
                }
                _ => Err(VmError::new(code::OUT_OF_PROPIDS)),
            },
            _ => Err(VmError::new(code::OUT_OF_PROPIDS)),
        }
    }

    /// Run the program: its static initializers, if it still has any, and
    /// then its entry point, with `args` as the program's arguments.
    pub fn run(&mut self, args: &[String]) -> VmResult<()> {
        self.run_static_initializers()?;

        let mut items = Vec::with_capacity(args.len());
        for arg in args {
            let string = self.new_str(arg);
            self.push(string);
            items.push(string);
        }
        self.discard(items.len());
        let list = self.new_list(items);
        self.push(list);

        let entry = self.entry_point;
        let pc = self.push_frame(
            entry,
            1,
            MethodContext::function(Value::FuncPtr(entry)),
            Return::Native,
            0,
            Some(Box::new(NativeCall::named("Main entrypoint"))),
        )?;
        self.run_from(pc)
    }

    /// Run the initializers of `static` properties. An image that has been
    /// pre-initialized has none left.
    fn run_static_initializers(&mut self) -> VmResult<()> {
        let initializers = std::mem::take(&mut self.static_initializers);
        for (object, prop) in initializers {
            let result = self.call_prop(Value::Obj(object), prop, 0, None);
            if let Err(error) = result {
                if matches!(error, VmError::Unimplemented(_)) {
                    return Err(error);
                }
                let message = self.error_message(&error, false);
                return Err(VmError::with(
                    code::EXC_IN_STATIC_INIT,
                    vec![
                        ErrParam::Str("[object name not available]".to_owned()),
                        ErrParam::Str("[property name not available]".to_owned()),
                        ErrParam::Str(message),
                    ],
                ));
            }
        }
        Ok(())
    }

    /// The text of an exception's message, if the exception has one.
    fn exception_text(&mut self, exception: ObjId) -> Option<String> {
        let prop = self.predef.exception_message;
        if prop == INVALID_PROP || !exception.is_valid() {
            return None;
        }
        let (value, _) = self.obj_get_prop(exception, prop, None).ok()??;
        let text = self.as_str(value)?;
        Some(String::from_utf8_lossy(&text).into_owned())
    }

    /// Describe an error that ended the program, in the reference VM's
    /// words.
    pub fn error_message(&mut self, error: &VmError, unhandled_prefix: bool) -> String {
        match error {
            VmError::Thrown(exception) => match self.exception_text(*exception) {
                Some(text) if unhandled_prefix => format!("Unhandled exception: {text}"),
                Some(text) => text,
                None => "unhandled exception".to_owned(),
            },
            VmError::Runtime(error) => format!("VM Error: {error}"),
            VmError::Unimplemented(what) => format!("not implemented in Hexicon: {what}"),
        }
    }
}
