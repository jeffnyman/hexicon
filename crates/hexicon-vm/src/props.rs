//! Property lookup and assignment for every kind of object, and the
//! methods all objects share.

use crate::error::{VmError, VmResult, code};
use crate::meta::MetaKind;
use crate::object::Object;
use crate::tads::{self, TadsObject};
use crate::value::{INVALID_PROP, NativeDesc, ObjId, PropId, Value, type_code};
use crate::vm::{NativeCall, Return, Vm};

/// `propDefined` flag: defined at all.
const PROPDEF_ANY: i32 = 1;
/// `propDefined` flag: defined by the object itself.
const PROPDEF_DIRECTLY: i32 = 2;
/// `propDefined` flag: inherited from a superclass.
const PROPDEF_INHERITS: i32 = 3;
/// `propDefined` flag: return the defining object.
const PROPDEF_GET_CLASS: i32 = 4;

/// A property found by a lookup: its value and the object defining it.
pub(crate) type Found = Option<(Value, ObjId)>;

impl Vm {
    /// Run a method the VM implements, under the convention all such
    /// methods share.
    ///
    /// A lookup that does not want the method called (`argc` is `None`)
    /// gets the method's interface back instead. Otherwise the argument
    /// count is checked, the method runs with its arguments on top of the
    /// stack, and the arguments are removed afterward.
    pub(crate) fn native_method(
        &mut self,
        argc: Option<&mut u32>,
        desc: NativeDesc,
        method: impl FnOnce(&mut Vm, u32) -> VmResult<Value>,
    ) -> VmResult<Value> {
        let Some(argc) = argc else {
            return Ok(Value::NativeCode(desc));
        };
        let count = *argc;
        if !desc.accepts(count) {
            return Err(VmError::new(code::WRONG_NUM_OF_ARGS));
        }
        *argc = 0;
        let base = self.stack.len() - count as usize;
        let result = method(self, count)?;
        self.stack.truncate(base);
        Ok(result)
    }

    /// Look up a property on an object. See [`Vm::native_method`] for the
    /// meaning of `argc`.
    pub(crate) fn obj_get_prop(
        &mut self,
        id: ObjId,
        prop: PropId,
        mut argc: Option<&mut u32>,
    ) -> VmResult<Found> {
        let kind = match self.objects.get(id) {
            None => return Err(VmError::new(code::NIL_DEREF)),
            Some(Object::Tads(_)) => MetaKind::TadsObject,
            Some(Object::IntClassMod(_)) => return Ok(tads::find_prop(&self.objects, id, prop)),
            Some(Object::Class(_)) => return self.class_get_prop(id, prop, argc),
            Some(_) => self.kind_of(id).unwrap_or(MetaKind::Root),
        };
        if kind == MetaKind::TadsObject
            && let Some(found) = tads::find_prop(&self.objects, id, prop)
        {
            return Ok(Some(found));
        }
        if let Some(found) =
            self.intrinsic_get_prop(kind, Value::Obj(id), prop, argc.as_deref_mut())?
        {
            return Ok(Some(found));
        }
        self.root_get_prop(id, prop, argc)
    }

    /// Look for a method among those of an intrinsic class and the
    /// intrinsic classes it derives from, short of the root class.
    fn intrinsic_get_prop(
        &mut self,
        kind: MetaKind,
        self_value: Value,
        prop: PropId,
        mut argc: Option<&mut u32>,
    ) -> VmResult<Found> {
        let mut kind = kind;
        while kind != MetaKind::Root {
            let index = self.meta.method_index(kind, prop);
            if index != 0
                && let Some(value) =
                    self.intrinsic_method(kind, self_value, index, argc.as_deref_mut())?
            {
                return Ok(Some((value, self.meta.class_obj(kind))));
            }
            match kind.parent() {
                Some(parent) => kind = parent,
                None => break,
            }
        }
        Ok(None)
    }

    /// Call method `index` of intrinsic class `kind` on a value. Returns
    /// `None` if the class has no such method.
    fn intrinsic_method(
        &mut self,
        kind: MetaKind,
        self_value: Value,
        index: u16,
        argc: Option<&mut u32>,
    ) -> VmResult<Option<Value>> {
        match kind {
            MetaKind::TadsObject => self.tads_method(self_value.obj_id(), index, argc),
            MetaKind::String => self.string_method(self_value, index, argc),
            MetaKind::List => self.list_method(self_value, index, argc),
            MetaKind::Vector => self.vector_method(self_value.obj_id(), index, argc),
            MetaKind::Collection => self.collection_method(self_value, index, argc),
            MetaKind::Iterator => self.iterator_method(self_value.obj_id(), index, argc),
            // These add no methods of their own to the classes they
            // derive from.
            MetaKind::AnonFuncPtr
            | MetaKind::IndexedIterator
            | MetaKind::LookupTableIterator
            | MetaKind::WeakRefLookupTable => Ok(None),
            MetaKind::LookupTable => self.table_method(self_value.obj_id(), index, argc),
            MetaKind::RexPattern => self.pattern_method(self_value.obj_id(), index, argc),
            MetaKind::StringBuffer => self.strbuf_method(self_value.obj_id(), index, argc),
            MetaKind::BigNumber => self.big_method(self_value.obj_id(), index, argc),
            MetaKind::ByteArray => self.bytes_method(self_value.obj_id(), index, argc),
            MetaKind::CharacterSet => self.charset_method(self_value.obj_id(), index, argc),
            MetaKind::StringComparator => self.strcomp_method(self_value.obj_id(), index, argc),
            MetaKind::Dictionary => self.dict_method(self_value.obj_id(), index, argc),
            MetaKind::GrammarProduction => self.gram_method(self_value.obj_id(), index, argc),
            other => Err(VmError::unimplemented(format!(
                "method {index} of intrinsic class {}",
                other.name()
            ))),
        }
    }

    /// Look up a property on a constant string or list.
    pub(crate) fn const_get_prop(
        &mut self,
        constant: Value,
        prop: PropId,
        mut argc: Option<&mut u32>,
    ) -> VmResult<Found> {
        let (kind, stand_in) = match constant {
            Value::SString(_) => (MetaKind::String, self.predef.const_str_obj),
            _ => (MetaKind::List, self.predef.const_lst_obj),
        };
        if let Some(found) = self.intrinsic_get_prop(kind, constant, prop, argc.as_deref_mut())? {
            return Ok(Some(found));
        }
        // Everything else is the same for every string (or list), so a
        // stand-in object answers for the constant.
        self.root_get_prop(stand_in, prop, argc)
    }

    /// Look up one of the methods every object has, then the methods the
    /// program adds to intrinsic classes with `modify`.
    fn root_get_prop(
        &mut self,
        id: ObjId,
        prop: PropId,
        argc: Option<&mut u32>,
    ) -> VmResult<Found> {
        let index = self.meta.method_index(MetaKind::Root, prop);
        if index == 0 {
            return self.find_modifier_prop(id, prop, id, ObjId::INVALID, argc);
        }
        let mut source = self.meta.class_obj(MetaKind::Root);
        let value = self.root_method(id, index, argc, &mut source)?;
        Ok(value.map(|value| (value, source)))
    }

    /// Search the program's modifications to an object's intrinsic class,
    /// and to the classes that one derives from.
    fn find_modifier_prop(
        &mut self,
        id: ObjId,
        prop: PropId,
        _target: ObjId,
        definer: ObjId,
        _argc: Option<&mut u32>,
    ) -> VmResult<Found> {
        let mut definer = definer;
        let mut found_definer = false;
        let mut kind = self.kind_of(id);
        while let Some(k) = kind {
            let Some(class_obj) = self.meta.entry(k).map(|entry| entry.class_obj) else {
                break;
            };
            if !class_obj.is_valid() {
                break;
            }
            let modifier = match self.objects.get(class_obj) {
                Some(Object::Class(class)) => class.modifier,
                _ => ObjId::INVALID,
            };
            let mut skip = false;
            if modifier.is_valid() && definer.is_valid() && !found_definer {
                if modifier == definer || self.is_instance_of(modifier, definer) {
                    found_definer = true;
                } else {
                    skip = true;
                }
            }
            if !skip
                && modifier.is_valid()
                && let Some(found) =
                    tads::find_inherited_prop(&self.objects, modifier, definer, prop)
            {
                return Ok(Some(found));
            }
            kind = k.parent();
            if found_definer {
                definer = ObjId::INVALID;
            }
        }
        Ok(None)
    }

    /// Look up the definition of a property that the current method's
    /// definition overrides, for `inherited`.
    pub(crate) fn obj_inh_prop(
        &mut self,
        id: ObjId,
        prop: PropId,
        target: ObjId,
        definer: ObjId,
        mut argc: Option<&mut u32>,
    ) -> VmResult<Found> {
        let mut definer = definer;
        match self.objects.get(id) {
            None => return Err(VmError::new(code::NIL_DEREF)),
            Some(Object::IntClassMod(_)) => {
                return Ok(tads::find_inherited_prop(
                    &self.objects,
                    target,
                    definer,
                    prop,
                ));
            }
            Some(Object::Tads(_)) => {
                let outside_program = matches!(
                    self.objects.get(definer),
                    Some(Object::IntClassMod(_) | Object::Class(_))
                );
                if !outside_program {
                    if let Some(found) =
                        tads::find_inherited_prop(&self.objects, target, definer, prop)
                    {
                        return Ok(Some(found));
                    }
                    let index = self.meta.method_index(MetaKind::TadsObject, prop);
                    if index != 0
                        && let Some(value) = self.tads_method(id, index, argc.as_deref_mut())?
                    {
                        return Ok(Some((value, self.meta.class_obj(MetaKind::TadsObject))));
                    }
                    definer = ObjId::INVALID;
                }
            }
            Some(_) => {}
        }
        let index = self.meta.method_index(MetaKind::Root, prop);
        if index != 0 {
            let mut source = self.meta.class_obj(MetaKind::Root);
            let value = self.root_method(id, index, argc, &mut source)?;
            return Ok(value.map(|value| (value, source)));
        }
        self.find_modifier_prop(id, prop, target, definer, argc)
    }

    /// Look up a property on an intrinsic class object: the class's static
    /// methods, then the program's modifications to it.
    fn class_get_prop(
        &mut self,
        id: ObjId,
        prop: PropId,
        mut argc: Option<&mut u32>,
    ) -> VmResult<Found> {
        let kind = self.class_kind(id);
        if let Some(kind) = kind
            && let Some(value) = self.static_method(kind, prop, argc.as_deref_mut())?
        {
            return Ok(Some((value, id)));
        }
        // The modifier of this class, then of each class it derives from.
        let mut class_obj = id;
        let mut class_kind = kind;
        while class_obj.is_valid() {
            if let Some(Object::Class(class)) = self.objects.get(class_obj) {
                let modifier = class.modifier;
                if modifier.is_valid()
                    && let Some(found) = tads::find_prop(&self.objects, modifier, prop)
                {
                    return Ok(Some(found));
                }
            }
            class_kind = class_kind.and_then(MetaKind::parent);
            class_obj = class_kind.map_or(ObjId::INVALID, |k| self.meta.class_obj(k));
        }
        self.root_get_prop(id, prop, argc)
    }

    /// The intrinsic class that an intrinsic class object represents.
    pub(crate) fn class_kind(&self, id: ObjId) -> Option<MetaKind> {
        match self.objects.get(id)? {
            Object::Class(class) => Some(self.meta.get(class.meta_index as usize)?.kind),
            _ => None,
        }
    }

    /// Call a static method: one invoked on an intrinsic class itself.
    fn static_method(
        &mut self,
        kind: MetaKind,
        prop: PropId,
        argc: Option<&mut u32>,
    ) -> VmResult<Option<Value>> {
        let index = self.meta.method_index(kind, prop);
        match (kind, index) {
            // TadsObject.createInstance() and createTransientInstance()
            (MetaKind::TadsObject, 1 | 3) => self
                .native_method(argc, NativeDesc::fixed(0), |vm, _| {
                    let (id, _) = vm.create_tads_object(0, index == 3, None)?;
                    Ok(Value::Obj(id))
                })
                .map(Some),
            // TadsObject.createInstanceOf() and createTransientInstanceOf()
            (MetaKind::TadsObject, 4 | 5) => self
                .native_method(argc, NativeDesc::variable(0), |vm, argc| {
                    vm.create_tads_object_multi(argc, index == 5)
                        .map(Value::Obj)
                })
                .map(Some),
            (MetaKind::List, index) if index != 0 => self.list_static_method(index, argc),
            (MetaKind::Vector, index) if index != 0 => self.vector_static_method(index, argc),
            (MetaKind::BigNumber, index) if index != 0 => self.big_static_method(index, argc),
            (MetaKind::ByteArray, index) if index != 0 => self.bytes_static_method(index, argc),
            (MetaKind::String, index) if index != 0 => self.string_static_method(index, argc),
            // IntrinsicClass.isIntrinsicClass(val)
            (MetaKind::IntrinsicClass, 1) => self
                .native_method(argc, NativeDesc::fixed(1), |vm, _| {
                    let is_class = match vm.arg(0) {
                        Value::Obj(id) => matches!(vm.objects.get(id), Some(Object::Class(_))),
                        _ => false,
                    };
                    Ok(Value::logical(is_class))
                })
                .map(Some),
            // A method of a class that is not implemented yet might be a
            // static one. Say so rather than quietly finding nothing.
            (other, index)
                if index != 0
                    && argc.is_some()
                    && !matches!(
                        other,
                        MetaKind::Root
                            | MetaKind::TadsObject
                            | MetaKind::IntClassMod
                            | MetaKind::IntrinsicClass
                            | MetaKind::String
                            | MetaKind::List
                            | MetaKind::Vector
                            | MetaKind::Collection
                            | MetaKind::Iterator
                            | MetaKind::IndexedIterator
                            | MetaKind::AnonFuncPtr
                            | MetaKind::LookupTable
                            | MetaKind::WeakRefLookupTable
                            | MetaKind::LookupTableIterator
                            | MetaKind::RexPattern
                            | MetaKind::StringBuffer
                            | MetaKind::BigNumber
                            | MetaKind::ByteArray
                            | MetaKind::CharacterSet
                            | MetaKind::StringComparator
                            | MetaKind::Dictionary
                            | MetaKind::GrammarProduction
                    ) =>
            {
                Err(VmError::unimplemented(format!(
                    "method {index} of intrinsic class {}",
                    other.name()
                )))
            }
            _ => Ok(None),
        }
    }

    // ---- assignment ----

    /// Set a property of an object.
    pub(crate) fn set_prop(&mut self, id: ObjId, prop: PropId, value: Value) -> VmResult<()> {
        match self.objects.get_mut(id) {
            None => Err(VmError::new(code::NIL_DEREF)),
            Some(Object::Tads(object) | Object::IntClassMod(object)) => {
                object.set(prop, value);
                Ok(())
            }
            Some(Object::Class(class)) => {
                // Setting a property of an intrinsic class sets it on the
                // program's modifier object for the class.
                let modifier = class.modifier;
                if modifier.is_valid() {
                    self.set_prop(modifier, prop, value)
                } else {
                    Err(VmError::new(code::INVALID_SETPROP))
                }
            }
            Some(_) => Err(VmError::new(code::INVALID_SETPROP)),
        }
    }

    // ---- class relationships ----

    /// Whether an object is an instance of a class, directly or through
    /// inheritance.
    pub(crate) fn is_instance_of(&self, id: ObjId, class: ObjId) -> bool {
        match self.objects.get(id) {
            None => false,
            Some(Object::Tads(_) | Object::IntClassMod(_)) => {
                tads::inherits_from(&self.objects, id, class)
                    || self.is_intrinsic_instance(id, class)
            }
            Some(Object::Class(_)) => {
                // An intrinsic class is an instance of the classes it
                // derives from.
                let mut kind = self.class_kind(id).and_then(MetaKind::parent);
                while let Some(k) = kind {
                    if self.meta.class_obj(k) == class && class.is_valid() {
                        return true;
                    }
                    kind = k.parent();
                }
                false
            }
            Some(_) => self.is_intrinsic_instance(id, class),
        }
    }

    /// Whether `class` is the intrinsic class of an object, or one that
    /// class derives from.
    fn is_intrinsic_instance(&self, id: ObjId, class: ObjId) -> bool {
        if !matches!(self.objects.get(class), Some(Object::Class(_))) {
            return false;
        }
        let Some(kind) = self.kind_of(id) else {
            return false;
        };
        let class_obj = self.meta.class_obj(kind);
        class_obj == class || (class_obj.is_valid() && self.is_instance_of(class_obj, class))
    }

    /// An object's direct superclasses, as programs see them.
    pub(crate) fn superclasses_of(&self, id: ObjId) -> Vec<ObjId> {
        match self.objects.get(id) {
            Some(Object::Tads(object) | Object::IntClassMod(object))
                if !object.superclasses.is_empty() =>
            {
                object.superclasses.clone()
            }
            Some(Object::Class(_)) => match self.class_kind(id).and_then(MetaKind::parent) {
                Some(parent) => vec![self.meta.class_obj(parent)],
                None => Vec::new(),
            },
            // Anything else has one superclass: its intrinsic class.
            Some(_) => vec![
                self.kind_of(id)
                    .map_or(ObjId::INVALID, |kind| self.meta.class_obj(kind)),
            ],
            None => Vec::new(),
        }
    }

    /// Whether an object is a class.
    fn is_class_object(&self, id: ObjId) -> bool {
        match self.objects.get(id) {
            Some(Object::Tads(object) | Object::IntClassMod(object)) => object.is_class(),
            Some(Object::Class(_)) => true,
            _ => false,
        }
    }

    /// The properties an object defines directly, as `getPropList` reports
    /// them.
    fn prop_list(&self, id: ObjId) -> Vec<Value> {
        let own = |object: &TadsObject| -> Vec<Value> {
            object
                .props()
                .iter()
                .map(|entry| Value::Prop(entry.prop))
                .collect()
        };
        match self.objects.get(id) {
            Some(Object::Tads(object)) => own(object),
            Some(Object::IntClassMod(object)) => {
                // A modifier's list includes the modifiers it replaced.
                let mut props = own(object);
                if let Some(&base) = object.superclasses.first()
                    && matches!(self.objects.get(base), Some(Object::IntClassMod(_)))
                {
                    props.extend(self.prop_list(base));
                }
                props
            }
            Some(Object::Class(class)) => {
                let mut props: Vec<Value> = self
                    .meta
                    .get(class.meta_index as usize)
                    .map(|entry| {
                        entry
                            .method_props
                            .iter()
                            .filter(|&&prop| prop != INVALID_PROP)
                            .map(|&prop| Value::Prop(prop))
                            .collect()
                    })
                    .unwrap_or_default();
                if class.modifier.is_valid() {
                    props.extend(self.prop_list(class.modifier));
                }
                props
            }
            _ => Vec::new(),
        }
    }

    /// The calling interface of a property: required arguments, optional
    /// arguments, and whether more are accepted. `None` if the property is
    /// not defined.
    pub(crate) fn prop_interface(
        &mut self,
        id: ObjId,
        prop: PropId,
    ) -> VmResult<Option<(i32, i32, bool)>> {
        let Some((value, _)) = self.obj_get_prop(id, prop, None)? else {
            return Ok(None);
        };
        Ok(Some(self.value_interface(value).unwrap_or((0, 0, false))))
    }

    /// The calling interface of a value that can be invoked.
    pub(crate) fn value_interface(&self, value: Value) -> Option<(i32, i32, bool)> {
        let func = match value {
            Value::CodeOfs(func) | Value::FuncPtr(func) => func,
            Value::Obj(id) | Value::ObjX(id) => self.invoker(id)?,
            Value::BifPtr { set, index } | Value::BifPtrX { set, index } => {
                let desc = self.bif_desc(set, index)?;
                return Some((desc.min_args.into(), desc.opt_args.into(), desc.varargs));
            }
            Value::NativeCode(desc) => {
                return Some((desc.min_args.into(), desc.opt_args.into(), desc.varargs));
            }
            _ => return None,
        };
        let header = self.function_header(func).ok()?;
        Some((
            header.min_args as i32,
            header.opt_args as i32,
            header.varargs,
        ))
    }

    // ---- the methods every object has ----

    /// Call one of the methods of the root intrinsic class, `Object`.
    fn root_method(
        &mut self,
        id: ObjId,
        index: u16,
        argc: Option<&mut u32>,
        source: &mut ObjId,
    ) -> VmResult<Option<Value>> {
        let _ = source;
        let value = match index {
            // ofKind(cls)
            1 => self.native_method(argc, NativeDesc::fixed(1), |vm, _| {
                let class = vm.obj_arg(vm.arg(0))?;
                Ok(Value::logical(class == id || vm.is_instance_of(id, class)))
            })?,
            // getSuperclassList()
            2 => self.native_method(argc, NativeDesc::fixed(0), |vm, _| {
                let list: Vec<Value> = vm
                    .superclasses_of(id)
                    .into_iter()
                    .map(Value::obj_or_nil)
                    .collect();
                Ok(vm.new_list(list))
            })?,
            // propDefined(prop, flags?)
            3 => self.native_method(argc, NativeDesc::optional(1, 1), |vm, argc| {
                let prop = vm.prop_arg(vm.arg(0))?;
                let flags = if argc >= 2 {
                    vm.int_arg(vm.arg(1))?
                } else {
                    PROPDEF_ANY
                };
                let found = vm.obj_get_prop(id, prop, None)?;
                let source = found.map(|(_, mut source)| {
                    // A method added with `modify` is reported as defined
                    // by the intrinsic class it was added to.
                    if matches!(vm.objects.get(source), Some(Object::IntClassMod(_))) {
                        source = vm.class_for_modifier(id, source);
                    }
                    source
                });
                Ok(match flags {
                    PROPDEF_ANY => Value::logical(source.is_some()),
                    PROPDEF_DIRECTLY => Value::logical(source == Some(id)),
                    PROPDEF_INHERITS => Value::logical(source.is_some_and(|s| s != id)),
                    PROPDEF_GET_CLASS => match source {
                        Some(source) if source.is_valid() => Value::Obj(source),
                        Some(_) => Value::Obj(id),
                        None => Value::Nil,
                    },
                    _ => return Err(VmError::new(code::BAD_VAL_BIF)),
                })
            })?,
            // propType(prop)
            4 => self.native_method(argc, NativeDesc::fixed(1), |vm, _| {
                let prop = vm.prop_arg(vm.arg(0))?;
                let Some((value, _)) = vm.obj_get_prop(id, prop, None)? else {
                    return Ok(Value::Nil);
                };
                let code = match value {
                    Value::ObjX(inner) => match vm.objects.get(inner) {
                        Some(Object::AnonFn(_)) => type_code::CODEOFS,
                        Some(Object::String(_)) => type_code::DSTRING,
                        _ => type_code::OBJ,
                    },
                    Value::BifPtrX { .. } => type_code::BIFPTR,
                    other => vm.datatype(other),
                };
                Ok(Value::Int(code))
            })?,
            // getPropList()
            5 => self.native_method(argc, NativeDesc::fixed(0), |vm, _| {
                let props = vm.prop_list(id);
                Ok(vm.new_list(props))
            })?,
            // getPropParams(prop)
            6 => self.native_method(argc, NativeDesc::fixed(1), |vm, _| {
                let prop = vm.prop_arg(vm.arg(0))?;
                let (min, opt, varargs) = vm.prop_interface(id, prop)?.unwrap_or((0, 0, false));
                Ok(vm.new_list(vec![
                    Value::Int(min),
                    Value::Int(opt),
                    Value::logical(varargs),
                ]))
            })?,
            // isClass()
            7 => self.native_method(argc, NativeDesc::fixed(0), |vm, _| {
                Ok(Value::logical(vm.is_class_object(id)))
            })?,
            // propInherited(prop, origTargetObj, definingObj, flags?)
            8 => self.native_method(argc, NativeDesc::optional(3, 1), |vm, argc| {
                let prop = vm.prop_arg(vm.arg(0))?;
                let target = vm.obj_arg(vm.arg(1))?;
                let definer = vm.obj_arg(vm.arg(2))?;
                let flags = if argc >= 4 {
                    vm.int_arg(vm.arg(3))?
                } else {
                    PROPDEF_ANY
                };
                let found = vm.obj_inh_prop(id, prop, target, definer, None)?;
                Ok(match flags {
                    PROPDEF_ANY => Value::logical(found.is_some()),
                    PROPDEF_GET_CLASS => match found {
                        Some((_, source)) if source.is_valid() => Value::Obj(source),
                        Some(_) => Value::Obj(id),
                        None => Value::Nil,
                    },
                    _ => return Err(VmError::new(code::BAD_VAL_BIF)),
                })
            })?,
            // isTransient()
            9 => self.native_method(argc, NativeDesc::fixed(0), |vm, _| {
                Ok(Value::logical(vm.objects.is_transient(id)))
            })?,
            _ => return Ok(None),
        };
        Ok(Some(value))
    }

    /// The intrinsic class, among those an object belongs to, whose
    /// modifier chain includes a given modifier object.
    fn class_for_modifier(&self, id: ObjId, modifier: ObjId) -> ObjId {
        let mut kind = self.kind_of(id);
        while let Some(k) = kind {
            let class_obj = self.meta.class_obj(k);
            if let Some(Object::Class(class)) = self.objects.get(class_obj)
                && class.modifier.is_valid()
                && (class.modifier == modifier || self.is_instance_of(class.modifier, modifier))
            {
                return class_obj;
            }
            kind = k.parent();
        }
        ObjId::INVALID
    }

    /// The type code of a value as `dataType()` reports it: string and
    /// list objects report as strings and lists.
    pub(crate) fn datatype(&self, value: Value) -> i32 {
        match value {
            Value::Obj(id) => match self.objects.get(id) {
                Some(Object::String(_)) => type_code::SSTRING,
                Some(Object::List(_)) => type_code::LIST,
                _ => type_code::OBJ,
            },
            other => other.type_code(),
        }
    }

    // ---- program-defined objects ----

    /// Create an instance of a program-defined class from native code and
    /// run its constructor. The constructor arguments are on the stack.
    /// The new object is left in register 0.
    pub(crate) fn create_instance(&mut self, class: ObjId, argc: u32) -> VmResult<()> {
        self.push(Value::Obj(class));
        let (id, _) = self.create_tads_object(argc + 1, false, None)?;
        // The constructor leaves the object in register 0 by returning
        // `self`; an object with no constructor needs it set.
        let _ = id;
        Ok(())
    }

    /// Create a program-defined object. The superclass, if `argc` is not
    /// zero, is on top of the stack, with the constructor arguments under
    /// it.
    ///
    /// With a `return_pc`, the constructor's frame is pushed and the
    /// address of its first instruction returned for the interpreter loop
    /// to continue at. Without one, the constructor runs to completion.
    pub(crate) fn create_tads_object(
        &mut self,
        argc: u32,
        transient: bool,
        return_pc: Option<u32>,
    ) -> VmResult<(ObjId, u32)> {
        let mut argc = argc;
        let mut superclasses = Vec::new();
        if argc != 0 {
            match self.pop() {
                Value::Nil => {}
                Value::Obj(class) => {
                    if self.objects.is_transient(class) {
                        return Err(VmError::new(code::BAD_DYNAMIC_NEW));
                    }
                    superclasses.push(class);
                }
                _ => return Err(VmError::new(code::OBJ_VAL_REQD_SC)),
            }
            argc -= 1;
        }
        let id = self
            .objects
            .alloc(Object::Tads(TadsObject::new(superclasses)));
        if transient {
            self.objects.set_transient(id);
        }
        let construct = self.predef.construct;
        let next_pc = match self.obj_get_prop(id, construct, None)? {
            Some((_, source)) => {
                let (ret, pc, native) = match return_pc {
                    Some(pc) => (Return::Normal, pc, None),
                    None => {
                        let mut native = NativeCall::named("TadsObject.construct");
                        native.caller_pc = self.pc;
                        (Return::Native, 0, Some(Box::new(native)))
                    }
                };
                self.eval_prop(
                    Value::Obj(source),
                    construct,
                    Value::Obj(id),
                    argc,
                    ret,
                    pc,
                    native,
                )?
            }
            None => {
                if argc != 0 {
                    return Err(VmError::new(code::WRONG_NUM_OF_ARGS));
                }
                self.r0 = Value::Obj(id);
                return_pc.unwrap_or(0)
            }
        };
        Ok((id, next_pc))
    }

    /// Create an object with several superclasses, for `createInstanceOf`.
    /// Each argument is a class, or a list of a class followed by the
    /// arguments for that class's constructor.
    fn create_tads_object_multi(&mut self, argc: u32, transient: bool) -> VmResult<ObjId> {
        let args: Vec<Value> = (0..argc as usize).map(|i| self.arg(i)).collect();
        let mut superclasses = Vec::new();
        for &arg in &args {
            let class = match self.as_list(arg) {
                Some(list) if list.len() >= 1 => list.get(0),
                Some(_) => return Err(VmError::new(code::INDEX_OUT_OF_RANGE)),
                None => arg,
            };
            let Value::Obj(class) = class else {
                return Err(VmError::new(code::BAD_TYPE_BIF));
            };
            if !matches!(self.objects.get(class), Some(Object::Tads(_))) {
                return Err(VmError::new(code::BAD_TYPE_BIF));
            }
            if self.objects.is_transient(class) {
                return Err(VmError::new(code::BAD_DYNAMIC_NEW));
            }
            superclasses.push(class);
        }
        let id = self
            .objects
            .alloc(Object::Tads(TadsObject::new(superclasses.clone())));
        if transient {
            self.objects.set_transient(id);
        }
        // Keep the new object reachable while the constructors run.
        self.push(Value::Obj(id));
        for (&arg, &class) in args.iter().zip(&superclasses) {
            let Some(list) = self.as_list(arg) else {
                continue;
            };
            self.check_stack(list.len())?;
            for index in (1..list.len()).rev() {
                self.push(list.get(index));
            }
            let mut native = NativeCall::named("TadsObject.contructMulti");
            native.caller_pc = 0;
            self.eval_prop(
                Value::Obj(class),
                self.predef.construct,
                Value::Obj(id),
                list.len() as u32 - 1,
                Return::Native,
                0,
                Some(Box::new(native)),
            )?;
        }
        self.pop();
        Ok(id)
    }

    /// Call one of the methods of the `TadsObject` intrinsic class.
    fn tads_method(
        &mut self,
        id: ObjId,
        index: u16,
        argc: Option<&mut u32>,
    ) -> VmResult<Option<Value>> {
        let value = match index {
            // createInstance(...) and createTransientInstance(...)
            1 | 3 => self.native_method(argc, NativeDesc::variable(0), |vm, argc| {
                // The constructor consumes the arguments.
                vm.push(Value::Obj(id));
                let (new, _) = vm.create_tads_object(argc + 1, index == 3, None)?;
                Ok(Value::Obj(new))
            })?,
            // createClone()
            2 => self.native_method(argc, NativeDesc::fixed(0), |vm, _| {
                let Some(original) = vm.objects.tads(id) else {
                    return Err(VmError::new(code::OBJ_VAL_REQD));
                };
                let mut clone = TadsObject::new(original.superclasses.clone());
                for entry in original.props() {
                    clone.set(entry.prop, entry.value);
                }
                Ok(Value::Obj(vm.objects.alloc(Object::Tads(clone))))
            })?,
            // createInstanceOf(...) and createTransientInstanceOf(...)
            4 | 5 => self.native_method(argc, NativeDesc::variable(0), |vm, argc| {
                vm.create_tads_object_multi(argc, index == 5)
                    .map(Value::Obj)
            })?,
            // setSuperclassList(lst)
            6 => self.native_method(argc, NativeDesc::fixed(1), |vm, _| {
                vm.set_superclass_list(id, vm.arg(0))?;
                Ok(Value::Nil)
            })?,
            // getMethod(prop)
            7 => self.native_method(argc, NativeDesc::fixed(1), |vm, _| {
                let prop = vm.prop_arg(vm.arg(0))?;
                Ok(match tads::find_prop(&vm.objects, id, prop) {
                    Some((Value::CodeOfs(func), _)) => Value::FuncPtr(func),
                    Some((Value::DString(offset), _)) => Value::SString(offset),
                    Some((Value::ObjX(object), _)) => Value::Obj(object),
                    Some((Value::BifPtrX { set, index }, _)) => Value::BifPtr { set, index },
                    _ => Value::Nil,
                })
            })?,
            // setMethod(prop, method)
            8 => self.native_method(argc, NativeDesc::fixed(2), |vm, _| {
                let prop = vm.prop_arg(vm.arg(0))?;
                let method = match vm.arg(1) {
                    Value::FuncPtr(func) => Value::CodeOfs(func),
                    Value::Obj(object)
                        if matches!(vm.objects.get(object), Some(Object::String(_)))
                            || vm.invoker(object).is_some() =>
                    {
                        Value::ObjX(object)
                    }
                    Value::BifPtr { set, index } => Value::BifPtrX { set, index },
                    Value::SString(offset) => Value::DString(offset),
                    _ => return Err(VmError::new(code::BAD_TYPE_BIF)),
                };
                vm.set_prop(id, prop, method)?;
                Ok(Value::Nil)
            })?,
            _ => return Ok(None),
        };
        Ok(Some(value))
    }

    /// Replace an object's superclass list.
    fn set_superclass_list(&mut self, id: ObjId, list: Value) -> VmResult<()> {
        let Some(count) = self.list_like_len(list)? else {
            return Err(VmError::new(code::BAD_TYPE_BIF));
        };
        if count < 1 {
            return Err(VmError::new(code::BAD_VAL_BIF));
        }
        let mut superclasses = Vec::new();
        let first = self.list_like_get(list, 1)?;
        // A list of just `TadsObject` means no superclasses at all.
        let root_only = count == 1
            && first == Value::Obj(self.meta.class_obj(MetaKind::TadsObject))
            && first.obj_id().is_valid();
        if !root_only {
            for index in 1..=count {
                let Value::Obj(class) = self.list_like_get(list, index)? else {
                    return Err(VmError::new(code::BAD_VAL_BIF));
                };
                if !matches!(self.objects.get(class), Some(Object::Tads(_)))
                    || self.is_instance_of(class, id)
                {
                    return Err(VmError::new(code::BAD_VAL_BIF));
                }
                superclasses.push(class);
            }
        }
        match self.objects.tads_mut(id) {
            Some(object) => {
                object.modified = true;
                object.set_superclasses(superclasses);
            }
            None => return Err(VmError::new(code::OBJ_VAL_REQD)),
        }
        // Any object that inherits from this one may have a search order
        // that is now out of date.
        for other in self.objects.ids() {
            if let Some(object) = self.objects.tads(other)
                && object.search_path_contains(id)
            {
                object.invalidate_search_path();
            }
        }
        Ok(())
    }
}
