//! A sampling profiler, built only with the `profile` feature.
//!
//! The VM marks what it is doing in a shared slot: a tag for the kind
//! of work, and the opcode the interpreter is on. A thread reads the
//! slot every few microseconds and counts what it finds, so each count
//! is time spent in the innermost work marked. [`report`] prints the
//! counts to standard error.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering::Relaxed};
use std::sync::{Mutex, Once};
use std::time::{Duration, Instant};

use crate::meta::MetaKind;

/// The kind of work being done.
pub(crate) static TAG: AtomicU32 = AtomicU32::new(0);
/// The opcode the interpreter is on.
pub(crate) static OP: AtomicU32 = AtomicU32::new(0);
static STOP: AtomicBool = AtomicBool::new(false);
static STARTED: Once = Once::new();
static COUNTS: Mutex<Option<HashMap<(u32, u32), u64>>> = Mutex::new(None);

/// Nothing marked: the host, loading, or the VM outside these.
#[allow(dead_code)]
pub(crate) const OTHER: u32 = 0;
/// The interpreter, running byte code.
pub(crate) const INTERP: u32 = 1;
/// Looking up a property of an object, through its superclasses.
pub(crate) const GET_PROP: u32 = 2;
/// The garbage collector.
pub(crate) const GC: u32 = 3;
/// Making a new object.
pub(crate) const ALLOC: u32 = 4;
/// Formatting text for the console.
pub(crate) const CONSOLE: u32 = 5;
/// Starting a function: its frame.
pub(crate) const CALL: u32 = 6;
/// Leaving a function.
pub(crate) const RETURN: u32 = 7;
/// Setting a property.
pub(crate) const SET_PROP: u32 = 8;
/// An undo savepoint, or taking one back.
pub(crate) const UNDO: u32 = 9;
/// The collector: listing the roots.
pub(crate) const GC_ROOTS: u32 = 10;
/// The collector: marking what is reachable.
pub(crate) const GC_MARK: u32 = 11;
/// The collector: freeing what is not.
pub(crate) const GC_SWEEP: u32 = 12;
/// The collector: weak tables, dictionaries, and saved states.
pub(crate) const GC_WEAK: u32 = 13;
/// A built-in function: this plus the set's number times 256 plus the
/// function's.
pub(crate) const BIF: u32 = 0x1_0000;
/// A method of an intrinsic class: this plus the class's place among
/// the kinds times 256 plus the method's number.
pub(crate) const NATIVE: u32 = 0x2_0000;

const NAMES: [&str; 14] = [
    "(other)",
    "interpreter",
    "property lookup",
    "garbage collection",
    "object allocation",
    "console formatting",
    "function entry",
    "function return",
    "property assignment",
    "undo",
    "gc: roots",
    "gc: mark",
    "gc: sweep",
    "gc: weak references and dictionaries",
];

/// Restores the tag that was there when it was made.
pub(crate) struct Guard(u32);

impl Drop for Guard {
    fn drop(&mut self) {
        TAG.store(self.0, Relaxed);
    }
}

/// Mark work of a kind until the guard goes.
#[inline]
pub(crate) fn scope(tag: u32) -> Guard {
    STARTED.call_once(start);
    Guard(TAG.swap(tag, Relaxed))
}

/// Start the sampling thread, which spins: the system's sleep is too
/// coarse.
fn start() {
    *COUNTS.lock().expect("counts") = Some(HashMap::new());
    std::thread::spawn(|| {
        let period = Duration::from_micros(20);
        let mut next = Instant::now();
        let mut local: HashMap<(u32, u32), u64> = HashMap::new();
        let mut taken: u64 = 0;
        while !STOP.load(Relaxed) {
            let now = Instant::now();
            if now < next {
                std::hint::spin_loop();
                continue;
            }
            next = now + period;
            let tag = TAG.load(Relaxed);
            let op = if tag == INTERP {
                OP.load(Relaxed)
            } else {
                u32::MAX
            };
            *local.entry((tag, op)).or_default() += 1;
            taken += 1;
            if taken % 65536 == 0 {
                let mut counts = COUNTS.lock().expect("counts");
                let counts = counts.get_or_insert_with(HashMap::new);
                for (key, n) in local.drain() {
                    *counts.entry(key).or_default() += n;
                }
            }
        }
        let mut counts = COUNTS.lock().expect("counts");
        let counts = counts.get_or_insert_with(HashMap::new);
        for (key, n) in local.drain() {
            *counts.entry(key).or_default() += n;
        }
    });
}

fn tag_name(tag: u32) -> String {
    if tag >= NATIVE {
        let n = tag - NATIVE;
        let kind = MetaKind::ALL
            .get((n >> 8) as usize)
            .map_or("?", |kind| kind.name());
        format!("method {kind} #{}", n & 0xff)
    } else if tag >= BIF {
        let n = tag - BIF;
        format!("built-in function set {} #{}", n >> 8, n & 0xff)
    } else {
        NAMES
            .get(tag as usize)
            .map_or_else(|| format!("tag {tag}"), |name| (*name).to_owned())
    }
}

/// Stop sampling and print what was found: the work by kind, and the
/// interpreter's time by opcode.
pub fn report() {
    STOP.store(true, Relaxed);
    std::thread::sleep(Duration::from_millis(20));
    let counts = COUNTS.lock().expect("counts").take().unwrap_or_default();
    let total: u64 = counts.values().sum();
    if total == 0 {
        return;
    }
    let mut by_tag: HashMap<u32, u64> = HashMap::new();
    let mut by_op: HashMap<u32, u64> = HashMap::new();
    for (&(tag, op), &n) in &counts {
        *by_tag.entry(tag).or_default() += n;
        if tag == INTERP {
            *by_op.entry(op).or_default() += n;
        }
    }
    let pct = |n: u64| 100.0 * n as f64 / total as f64;
    eprintln!("profile: {total} samples");
    let mut tags: Vec<_> = by_tag.into_iter().collect();
    tags.sort_by_key(|entry| std::cmp::Reverse(entry.1));
    for (tag, n) in tags.iter().take(40) {
        eprintln!("{:6.2}%  {}", pct(*n), tag_name(*tag));
    }
    eprintln!("interpreter by opcode:");
    let mut ops: Vec<_> = by_op.into_iter().collect();
    ops.sort_by_key(|entry| std::cmp::Reverse(entry.1));
    for (op, n) in ops.iter().take(30) {
        let name = hexicon_bytecode::Opcode::from_byte(*op as u8)
            .map_or_else(|| format!("0x{op:02x}"), |op| format!("{op:?}"));
        eprintln!("{:6.2}%  {name}", pct(*n));
    }
}
