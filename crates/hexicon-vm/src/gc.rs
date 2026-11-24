//! Garbage collection: finding the objects nothing can reach any more,
//! running their finalizers, and releasing them.
//!
//! The object table does the marking and sweeping. This module supplies
//! the roots, which only the VM knows, and runs the finalizers.
//!
//! When the collector runs matters to a program, because a released
//! object's ID is the next one handed out, and a program can see IDs: in
//! the order `firstObj` and `nextObj` go through objects, for one. The
//! reference VM collects before creating an object once 10,000 have been
//! created since the last collection, or more than 6 MB allocated for
//! them (see [`crate::heap`]), and Hexicon collects at the same moments.

use crate::object::Object;
use crate::program_object;
use crate::value::{INVALID_PROP, ObjId, Value};
use crate::vm::{NativeCall, Vm};

impl Vm {
    /// Every value the VM itself holds.
    fn gc_roots(&self) -> Vec<Value> {
        let mut roots: Vec<Value> = self.stack.clone();
        for frame in &self.frames {
            roots.push(frame.target);
            roots.push(Value::Obj(frame.definer));
            roots.push(frame.self_value);
            roots.push(frame.invokee);
            if let Some(id) = frame.frame_ref {
                roots.push(Value::Obj(id));
            }
            if let Some(call) = &frame.native {
                roots.push(call.self_value);
                roots.extend(call.args.iter().copied());
            }
        }
        roots.extend(self.gc_global_roots());
        roots
    }

    /// The values the VM holds outside the stack: what stays reachable
    /// whatever code is running.
    pub(crate) fn gc_global_roots(&self) -> Vec<Value> {
        let mut roots: Vec<Value> = Vec::new();
        roots.push(self.r0);
        roots.push(self.say_function);
        roots.push(self.last_match.subject);
        roots.extend(self.symbol_table.map(Value::Obj));
        roots.extend(self.macro_table.map(Value::Obj));
        roots.extend(self.predef.objects().into_iter().map(Value::Obj));
        // The objects that stand for the built-in classes.
        for index in 0..self.meta.len() {
            if let Some(entry) = self.meta.get(index) {
                roots.push(Value::Obj(entry.class_obj));
            }
        }
        roots
    }

    /// Whether an object has a finalizer: a `finalize` method.
    fn has_finalizer(&self, id: ObjId) -> bool {
        let destruct = self.predef.destruct;
        destruct != INVALID_PROP
            && matches!(self.objects.get(id), Some(Object::Program(_)))
            && program_object::find_prop(&self.objects, id, destruct).is_some_and(|(value, _)| {
                matches!(
                    value,
                    Value::CodeOfs(_)
                        | Value::NativeCode(_)
                        | Value::ObjX(_)
                        | Value::BifPtrX { .. }
                )
            })
    }

    /// Collect garbage. `automatic` says the collector is running on its
    /// own, before the creation of an object that the reference VM would
    /// not count toward the next collection.
    pub(crate) fn collect_garbage(&mut self, automatic: bool) {
        self.collect_garbage_keeping(automatic, Vec::new());
    }

    /// Collect garbage, keeping what these values refer to as well: the
    /// references of an object about to be created, which is not in the
    /// table yet.
    pub(crate) fn collect_garbage_keeping(&mut self, automatic: bool, keep: Vec<Value>) {
        prof_scope!(crate::prof::GC);
        if self.in_gc {
            return;
        }
        self.in_gc = true;
        if crate::object::trace_alloc() {
            let why = if automatic { "at" } else { "asked at" };
            eprintln!("GC {why} {:#x} stack {}", self.pc, self.stack.len());
        }
        let mut roots = self.gc_roots();
        roots.extend(keep);
        let candidates = self.objects.collect_mark(&roots, automatic);
        let with_finalizers: Vec<ObjId> = candidates
            .iter()
            .copied()
            .filter(|&id| self.has_finalizer(id))
            .collect();
        let queue = self.objects.collect_sweep(&candidates, &with_finalizers);
        if automatic {
            self.objects.skip_next_alloc_count();
        }
        self.release_files();

        // Run the finalizers. One that fails is no one's business but
        // its own, and a finalizer changes nothing in the code that
        // happened to be running when the collector started.
        let (r0, pc) = (self.r0, self.pc);
        for id in queue {
            let native = NativeCall::named("ProgramObject.finalize");
            let _ = self.call_prop(Value::Obj(id), self.predef.destruct, 0, Some(native));
        }
        self.r0 = r0;
        self.pc = pc;
        self.in_gc = false;
    }
}
