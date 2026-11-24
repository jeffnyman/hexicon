//! The symbol tables a program compiled for debugging can look itself up
//! in: `globalSymbols`.

use hexicon_image::GlobalSymbolKind;

use crate::map::Map;
use crate::object::Object;
use crate::value::{ObjId, Value};
use crate::vm::Vm;

impl Vm {
    /// Build the tables of global symbols and macros from the image's
    /// debug records, if it has them. Each maps a name to a value.
    pub(crate) fn build_symbol_tables(&mut self) {
        if !self.debug.global_symbols.is_empty() && self.symbol_table.is_none() {
            let symbols = self.debug.global_symbols.clone();
            let id = self.new_table(Map::new(256, symbols.len(), false));
            for symbol in &symbols {
                let value = match symbol.kind {
                    GlobalSymbolKind::Function { code_offset, .. } => Value::FuncPtr(code_offset),
                    GlobalSymbolKind::Object { id, .. } => {
                        // The objects that hold a program's additions to
                        // built-in classes are not for programs to see.
                        if matches!(self.objects.get(ObjId(id)), Some(Object::IntClassMod(_))) {
                            continue;
                        }
                        Value::obj_or_nil(ObjId(id))
                    }
                    GlobalSymbolKind::Property { id, .. } => Value::Prop(id),
                    GlobalSymbolKind::BuiltinFunction { index, set, .. } => {
                        Value::BifPtr { set, index }
                    }
                    GlobalSymbolKind::BuiltinClass { class_object, .. } => {
                        Value::obj_or_nil(ObjId(class_object))
                    }
                    GlobalSymbolKind::Enumerator { id, .. } => Value::Enum(id),
                    GlobalSymbolKind::Other { .. } => continue,
                };
                let name = self.new_str(&symbol.name);
                // The table was just created, so this cannot fail.
                let _ = self.table_set(id, name, value);
            }
            self.symbol_table = Some(id);
        }
        if !self.debug.macros.is_empty() && self.macro_table.is_none() {
            let macros = self.debug.macros.clone();
            let id = self.new_table(Map::new(256, macros.len(), false));
            for definition in &macros {
                // Each macro is its expansion, its parameter names, and
                // its flags. The reference makes the name, then the list,
                // then what goes in it, in order, so the objects are made
                // in that order here too.
                let name = self.new_str(&definition.name);
                let entry = self.new_list(vec![Value::Nil; 3]);
                let expansion = self.new_str(&definition.expansion);
                let parameters = self.new_list(vec![Value::Nil; definition.parameters.len()]);
                let mut names = Vec::with_capacity(definition.parameters.len());
                for parameter in &definition.parameters {
                    names.push(self.new_str(parameter));
                }
                let flags =
                    i32::from(definition.function_like) | i32::from(definition.varargs) << 1;
                if let (Value::Obj(entry), Value::Obj(parameters)) = (entry, parameters) {
                    self.objects.replace(parameters, Object::List(names.into()));
                    let items = vec![expansion, Value::Obj(parameters), Value::Int(flags)];
                    self.objects.replace(entry, Object::List(items.into()));
                }
                let _ = self.table_set(id, name, entry);
            }
            self.macro_table = Some(id);
        }
    }
}
