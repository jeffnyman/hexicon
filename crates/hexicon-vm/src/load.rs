//! Setting up a VM from a program image, and running the program.

use std::rc::Rc;

use hexicon_host::Host;
use hexicon_image::{Image, Pool, Value as ImageValue};

use crate::bif::FunctionSet;
use crate::error::{ErrParam, VmError, VmResult, code};
use crate::map::Map;
use crate::meta::{MetaKind, MetaTable};
use crate::object::{BuiltinClass, Object, ObjectTable};
use crate::program_object::ProgramObject;
use crate::value::{INVALID_PROP, ObjId, PropId, Value};
use crate::vm::{Layout, MethodContext, NativeCall, Predef, Return, RunState, STACK_LIMIT, Vm};

/// Width of the console in columns. The reference VM's plain console is
/// 80 columns wide.
pub(crate) const CONSOLE_WIDTH: usize = 80;

/// Copy a pool into one contiguous array, so that a pool address is an
/// index. Pages the image does not supply read as zeros.
fn flatten(pool: &Pool) -> Rc<[u8]> {
    let page_size = pool.page_size() as usize;
    // Up to the last page the image supplies: a damaged definition can
    // claim millions of pages the image does not have, and an address
    // past the end is caught when it is read.
    let used = (0..pool.page_count())
        .rev()
        .find(|&index| pool.page(index).is_some())
        .map_or(0, |index| index + 1);
    let mut data = vec![0u8; page_size * used];
    for index in 0..used {
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
        MetaKind::ProgramObject => {
            Object::Program(ProgramObject::from_image(data).ok_or_else(bad)?)
        }
        MetaKind::IntClassMod => {
            Object::IntClassMod(ProgramObject::from_image(data).ok_or_else(bad)?)
        }
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
        MetaKind::Vector | MetaKind::Function => {
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
        MetaKind::Map | MetaKind::WeakMap => Object::Map(Box::new(
            Map::from_image(data, kind == MetaKind::WeakMap).ok_or_else(bad)?,
        )),
        MetaKind::Decimal => Object::Decimal(Box::new(
            crate::decimal::Big::from_bytes(data).ok_or_else(bad)?,
        )),
        MetaKind::ByteArray => {
            // A length, then the bytes. Any the image leaves out are zero.
            let len = data.get(..4).ok_or_else(bad)?;
            let len = u32::from_le_bytes([len[0], len[1], len[2], len[3]]) as usize;
            let mut bytes = data[4..].to_vec();
            bytes.resize(len, 0);
            Object::ByteArray(bytes)
        }
        MetaKind::GrammarRule => Object::GrammarRule(Box::new(
            crate::gram::GrammarRule::from_bytes(data)
                .ok_or_else(bad)?
                .0,
        )),
        MetaKind::StringComparator => Object::StrComp(std::rc::Rc::new(
            crate::strcomp::StrComp::from_bytes(data).ok_or_else(bad)?.0,
        )),
        MetaKind::Dictionary => Object::Dict(Box::new(
            crate::dict::Dictionary::from_image(data).ok_or_else(bad)?,
        )),
        MetaKind::CharacterSet => {
            let len = u16_at(0)? as usize;
            let name = String::from_utf8_lossy(data.get(2..2 + len).ok_or_else(bad)?).into_owned();
            let map = crate::charset::CharMap::load(&name).map(std::rc::Rc::new);
            Object::CharSet { name, map }
        }
        MetaKind::Regex => Object::Pattern {
            source: Value::from_holder(data.get(..Value::HOLDER_SIZE).ok_or_else(bad)?),
            compiled: None,
        },
        MetaKind::FileName => {
            // The number of a special file, or zero and then the name.
            let special = data.get(..4).ok_or_else(bad)?;
            let special = i32::from_le_bytes([special[0], special[1], special[2], special[3]]);
            let path = if special == 0 {
                let len = u16_at(4)? as usize;
                String::from_utf8_lossy(data.get(6..6 + len).ok_or_else(bad)?).into_owned()
            } else {
                String::new()
            };
            Object::FileName(Box::new(crate::file::FileNameObj {
                path,
                special,
                from_ui: 0,
            }))
        }
        MetaKind::File => {
            // The character set, the mode, the access, and flags. The
            // file itself is not open: the object is out of sync with it.
            let charset = data.get(..4).ok_or_else(bad)?;
            let charset = u32::from_le_bytes([charset[0], charset[1], charset[2], charset[3]]);
            Object::File(Box::new(crate::file::FileObj {
                charset: ObjId(charset),
                mode: *data.get(4).ok_or_else(bad)?,
                access: *data.get(5).ok_or_else(bad)?,
                out_of_sync: true,
                spec: Value::Nil,
                open: None,
            }))
        }
        MetaKind::TemporaryFile => Object::TempFile(String::new()),
        MetaKind::TimeZone => {
            // The offsets, the name, and the abbreviation, the last two
            // each after a byte with its length.
            let int = |at: usize| {
                let bytes = data.get(at..at + 4).ok_or_else(bad)?;
                Ok::<i32, VmError>(i32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
            };
            let text = |at: usize| {
                let len = usize::from(*data.get(at).ok_or_else(bad)?);
                let bytes = data.get(at + 1..at + 1 + len).ok_or_else(bad)?;
                Ok::<(String, usize), VmError>((
                    String::from_utf8_lossy(bytes).into_owned(),
                    at + 1 + len,
                ))
            };
            let (name, after) = text(8)?;
            let (abbr, _) = text(after)?;
            Object::TimeZone(std::rc::Rc::new(crate::tz::Zone::recorded(
                &name,
                int(0)?,
                int(4)?,
                &abbr,
            )))
        }
        MetaKind::Date => {
            let int = |at: usize| {
                let bytes = data.get(at..at + 4).ok_or_else(bad)?;
                Ok::<i32, VmError>(i32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
            };
            Object::Date {
                dayno: int(0)?,
                daytime: int(4)?,
            }
        }
        MetaKind::StringBuilder => {
            // The room, the step it grows by, the length, and the
            // characters in 16 bits. Loading keeps the sizes within the
            // limits, as the reference does.
            let int = |at: usize| {
                let bytes = data.get(at..at + 4).ok_or_else(bad)?;
                Ok::<u32, VmError>(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
            };
            let limit = |n: u32| n.clamp(16, crate::object::StringBuilder::MAX_ROOM);
            let room = limit(int(0)?);
            let mut buffer = crate::object::StringBuilder::new(room, limit(int(4)?));
            let len = int(8)?.min(room) as usize;
            for index in 0..len {
                let unit = u32::from(u16_at(12 + 2 * index)?);
                buffer
                    .chars
                    .push(char::from_u32(unit).unwrap_or('\u{fffd}'));
            }
            Object::StringBuilder(buffer)
        }
        MetaKind::CompiledCode => Object::CompiledCode(Box::new(
            crate::compiled_code::CompiledCode::read(data, &mut |holder| {
                Some(Value::from_holder(holder))
            })
            .ok_or_else(bad)?,
        )),
        MetaKind::StackFrameRef => Object::FrameRef(Box::new(
            crate::frameref::FrameRef::read(
                data,
                &mut |holder| Some(Value::from_holder(holder)),
                &|id| id,
            )
            .ok_or_else(bad)?,
        )),
        MetaKind::StackFrame => {
            Object::FrameDesc(crate::frameref::FrameDesc::read(data).ok_or_else(bad)?)
        }
        MetaKind::BuiltinClass => {
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
            Object::Class(BuiltinClass {
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
        // As in the reference, a pool's pages are a power of two in size,
        // and every page the pool has is in the image.
        for pool in [&image.code_pool, &image.constant_pool] {
            let size = pool.page_size();
            if size != 0 && !size.is_power_of_two() {
                return Err(VmError::new(code::BAD_POOL_PAGE_SIZE));
            }
            if pool.loaded_page_count() != pool.page_count() {
                return Err(VmError::new(code::LOAD_UNDEF_PAGE));
            }
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

        crate::heap::reset();
        let mut objects = ObjectTable::new();
        // Object IDs are a few times as many as the objects at most, in
        // any image a compiler writes. A damaged one could name an ID in
        // the billions, which would take as much memory to hold.
        let count: usize = image.object_blocks.iter().map(|b| b.objects.len()).sum();
        let id_limit = 16 * count as u64 + 65_536;
        for block in &image.object_blocks {
            let Some(kind) = meta.get(block.metaclass_index as usize).map(|e| e.kind) else {
                return Err(VmError::new(code::BAD_METACLASS_INDEX));
            };
            for object in &block.objects {
                if u64::from(object.id) >= id_limit {
                    return Err(VmError::new(code::OUT_OF_MEMORY));
                }
                let id = ObjId(object.id);
                let loaded = load_object(kind, block.metaclass_index, &object.data)?;
                // A built-in class object tells the VM which object
                // stands for the class.
                if let Object::Class(class) = &loaded
                    && let Some(entry) = meta.get_mut(class.meta_index as usize)
                {
                    entry.class_obj = id;
                }
                if !objects.alloc_with_id(id, loaded, block.transient) {
                    return Err(VmError::new(code::OBJ_IN_USE));
                }
                // A vector has the room the image says, or at least
                // room for its elements.
                if matches!(kind, MetaKind::Vector | MetaKind::Function) {
                    if let Some(room) = object.data.get(..2) {
                        let room = usize::from(u16::from_le_bytes([room[0], room[1]]));
                        let room = room.max(objects.vector_alloc(id));
                        objects.set_vector_alloc(id, room);
                    }
                }
            }
        }

        let entry = &image.entry_point;
        let mut vm = Self {
            code: flatten(&image.code_pool),
            image_code_len: 0,
            consts: flatten(&image.constant_pool),
            layout: Layout {
                method_header: entry.method_header_size.into(),
                exception_entry: entry.exception_entry_size.into(),
                debug_line_entry: entry.debug_line_entry_size.into(),
                debug_table_header: entry.debug_table_header_size.into(),
                debug_local_header: entry.debug_local_header_size.into(),
                debug_frame_header: entry.debug_frame_header_size.into(),
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
            last_match: Default::default(),
            pattern_cache: Default::default(),
            rng: crate::rng::Rng::new(),
            image_timestamp: image.header.timestamp.as_bytes().to_vec(),
            image_name: String::new(),
            exception_message_synthesized: false,
            run_depth: 0,
            in_gc: false,
            nesting: 0,
            big_cache: Default::default(),
            symbol_table: None,
            macro_table: None,
            trace_errors: false,
            console: crate::console::Formatter::new(CONSOLE_WIDTH),
            io: crate::io::IoState::default(),
            host,
        };
        vm.image_code_len = vm.code.len();
        vm.link(image)?;
        // A dictionary's words are stored by its comparator's hash, which
        // can only be worked out once the comparator is loaded.
        let dictionaries: Vec<ObjId> = vm
            .objects
            .ids()
            .filter(|&id| matches!(vm.objects.get(id), Some(Object::Dict(d)) if d.comparator.is_valid()))
            .collect();
        for id in dictionaries {
            vm.dict_rehash(id)?;
        }
        vm.resolve_zones();
        // From here on, a change to an object from the image is a change
        // the program made.
        vm.objects.track_image_state();
        Ok(vm)
    }

    /// Record the name of the image file, which saved states carry so
    /// that a saved state can lead back to its program.
    pub fn set_image_name(&mut self, name: &str) {
        self.image_name = name.to_owned();
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
            stack_info_class: object_symbol(image, "StackInfo")?,
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
            calc_hash: prop_symbol(image, "IfcComparator.calcHash")?,
            gram_first_token: prop_symbol(image, "GrammarRule.firstTokenIndex")?,
            gram_last_token: prop_symbol(image, "GrammarRule.lastTokenIndex")?,
            gram_token_list: prop_symbol(image, "GrammarRule.tokenList")?,
            gram_token_match_list: prop_symbol(image, "GrammarRule.tokenMatchList")?,
            gram_tag: prop_symbol(image, "GrammarRule.grammarTag")?,
            gram_misc_vocab: prop_symbol(image, "GrammarRule.miscVocab")?,
            gram_alt_props: prop_symbol(image, "GrammarRule.altProps")?,
            gram_alt_info: object_symbol(image, "GrammarRule.GrammarAltInfo")?,
            gram_alt_token_info: object_symbol(image, "GrammarRule.GrammarAltTokInfo")?,
            match_values: prop_symbol(image, "IfcComparator.matchValues")?,
            charset_unknown_exception: object_symbol(
                image,
                "CharacterSet.UnknownCharSetException",
            )?,
            file_not_found_exc: object_symbol(image, "File.FileNotFoundException")?,
            file_creation_exc: object_symbol(image, "File.FileCreationException")?,
            file_open_exc: object_symbol(image, "File.FileOpenException")?,
            file_io_exc: object_symbol(image, "File.FileIOException")?,
            file_sync_exc: object_symbol(image, "File.FileSyncException")?,
            file_closed_exc: object_symbol(image, "File.FileClosedException")?,
            file_mode_exc: object_symbol(image, "File.FileModeException")?,
            file_safety_exc: object_symbol(image, "File.FileSafetyException")?,
            compiler_exc: object_symbol(image, "CompiledCode.CompilerException")?,
            file_info: object_symbol(image, "File.FileInfo")?,
            filespec_get_filename: prop_symbol(image, "FileSpec.getFilename")?,
            filespec_close_file: prop_symbol(image, "FileSpec.closeFile")?,
            string_sth_flags: prop_symbol(image, "String.specialsToHtml.flags")?,
            string_sth_tag: prop_symbol(image, "String.specialsToHtml.tag")?,
            iter_get_next: INVALID_PROP,
            iter_next_avail: INVALID_PROP,
        };

        self.create_synthesized_objects()?;
        // Functions compiled while the program ran before its image was
        // written go with the code.
        self.place_dynamic_code();
        self.create_class_objects();

        if let Some(iterator) = self.meta.entry(MetaKind::Iterator) {
            self.predef.iter_get_next = iterator.method_prop(1);
            self.predef.iter_next_avail = iterator.method_prop(2);
        }
        Ok(())
    }

    /// Create the objects the VM needs that the image does not supply.
    /// They are created in the same order as in the reference VM, so
    /// that objects the program creates later get the same IDs.
    pub(crate) fn create_synthesized_objects(&mut self) -> VmResult<()> {
        let last_prop = self.predef.last_prop;
        self.predef.last_prop_obj = self.alloc_obj(Object::Vector(vec![Value::Prop(last_prop)]));
        if self.predef.exception_message == INVALID_PROP || self.exception_message_synthesized {
            self.predef.exception_message = self.alloc_prop_for_vm()?;
            self.exception_message_synthesized = true;
        }
        self.predef.const_str_obj = self.alloc_obj(Object::String(Rc::from(&[][..])));
        self.predef.const_lst_obj = self.alloc_obj(Object::List(Rc::from(&[][..])));
        Ok(())
    }

    /// Give every built-in class that has none an object to represent
    /// it.
    pub(crate) fn create_class_objects(&mut self) {
        for index in 0..self.meta.len() {
            if self
                .meta
                .get(index)
                .is_some_and(|e| !e.class_obj.is_valid())
            {
                let id = self.objects.alloc(Object::Class(BuiltinClass {
                    meta_index: index as u16,
                    modifier: ObjId::INVALID,
                    class_state: Value::Nil,
                }));
                if let Some(entry) = self.meta.get_mut(index) {
                    entry.class_obj = id;
                }
            }
        }
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

    /// Run the program to its end. The program sees `args` as the
    /// arguments of its main function.
    ///
    /// This is for a host that can wait for the player. With one that
    /// cannot, use [`Vm::start`] and [`Vm::resume`].
    pub fn run(&mut self, args: &[String]) -> VmResult<()> {
        let mut state = self.start(args)?;
        while state == RunState::Suspended {
            state = self.resume()?;
        }
        Ok(())
    }

    /// Start the program, and run it until it ends or has to wait for
    /// the host. The program sees `args` as the arguments of its main
    /// function.
    pub fn start(&mut self, args: &[String]) -> VmResult<RunState> {
        self.sync_console_width();
        self.build_symbol_tables();
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
        let base_depth = self.frames.len() - 1;
        self.io.resumable = true;
        self.drive(pc, base_depth)
    }

    /// Carry on with a program that stopped to wait for the host, until
    /// it ends or has to wait again.
    pub fn resume(&mut self) -> VmResult<RunState> {
        match self.io.suspended.take() {
            Some((pc, base_depth)) => self.drive(pc, base_depth),
            None => Ok(RunState::Finished),
        }
    }

    fn drive(&mut self, pc: u32, base_depth: usize) -> VmResult<RunState> {
        match self.run_at(pc, base_depth) {
            Err(VmError::Suspend) => Ok(RunState::Suspended),
            result => {
                self.io.resumable = false;
                result.map(|()| RunState::Finished)
            }
        }
    }

    /// Run the initializers of `static` properties. An image that has been
    /// pre-initialized has none left.
    pub(crate) fn run_static_initializers(&mut self) -> VmResult<()> {
        let initializers = self.static_initializers.clone();
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
            VmError::Suspend => "the program is waiting for input".to_owned(),
            VmError::Unimplemented(what) => format!("not implemented in Hexicon: {what}"),
        }
    }
}
