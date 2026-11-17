//! Finding the functions in an image.
//!
//! An image has no table of its functions unless it was compiled for
//! debugging. Otherwise functions are found by following references: the
//! entry point, exported symbols, method values in static objects, function
//! pointers in constant lists, and calls from functions already found.

use std::collections::{BTreeMap, BTreeSet};

use hexicon_image::{GlobalSymbolKind, Image, Value};

use crate::decode::{Operand, Payload};
use crate::function::{DisassembleError, Extent, Function, disassemble};

/// The functions found in an image.
#[derive(Debug, Clone, Default)]
pub struct Functions {
    /// Each function found, by code pool address. An address that was
    /// referenced as a function but could not be disassembled maps to the
    /// error.
    pub by_address: BTreeMap<u32, Result<Function, DisassembleError>>,
    /// Functions nothing refers to. They were found because they sit
    /// directly after another function, in bytes that are not part of it.
    pub unreferenced: BTreeSet<u32>,
    /// Referenced addresses that lie beyond the code the image supplies.
    /// Pre-initialization discards static initializer code, which leaves
    /// such references behind in the properties that code initialized.
    pub dangling: BTreeSet<u32>,
}

impl Functions {
    /// The functions that disassembled, in address order.
    pub fn functions(&self) -> impl Iterator<Item = &Function> {
        self.by_address.values().filter_map(|f| f.as_ref().ok())
    }

    /// The addresses that failed to disassemble, with their errors.
    pub fn errors(&self) -> impl Iterator<Item = (u32, &DisassembleError)> {
        self.by_address
            .iter()
            .filter_map(|(address, f)| f.as_ref().err().map(|e| (*address, e)))
    }
}

/// Find and disassemble every function reachable from an image's roots.
pub fn discover(image: &Image) -> Functions {
    discover_from(image, true)
}

/// Like [`discover`], but ignoring the function lists that debug builds
/// carry, so the result is what would be found in a release build of the
/// same program. Comparing the two measures how complete discovery is.
pub fn discover_without_debug_info(image: &Image) -> Functions {
    discover_from(image, false)
}

fn discover_from(image: &Image, use_debug_info: bool) -> Functions {
    let mut finder = Finder {
        image,
        pending: Vec::new(),
        seen_lists: BTreeSet::new(),
        found: BTreeMap::new(),
        speculative: BTreeSet::new(),
        rejected: BTreeSet::new(),
        unreferenced: BTreeSet::new(),
        dangling: BTreeSet::new(),
    };
    finder.roots(use_debug_info);
    finder.run();

    // Code nothing refers to can still be found if it starts a page, since
    // the first function in a page starts at the page's first byte. The
    // static initializer code, which begins a page of its own, is the usual
    // case.
    for index in 0..image.code_pool.page_count() {
        let address = index as u32 * image.code_pool.page_size();
        let has_code = image.code_pool.page(index).is_some_and(|p| !p.is_empty());
        if has_code && !finder.found.contains_key(&address) {
            finder.speculative.insert(address);
            finder.pending.push(address);
        }
    }
    finder.run();

    // With every start known, each function can be bounded by the next,
    // which brings unreachable code into view. That code can refer to
    // further functions, so repeat until nothing new turns up.
    let page_size = image.code_pool.page_size().max(1);
    loop {
        let starts: Vec<u32> = finder.found.keys().copied().collect();
        for (index, &address) in starts.iter().enumerate() {
            // The bound is the next function in the same page, or failing
            // that the end of the page's data.
            let page_start = address - address % page_size;
            let page_len = image
                .code_pool
                .page((address / page_size) as usize)
                .map_or(0, |page| page.len() as u32);
            let limit = starts
                .get(index + 1)
                .copied()
                .filter(|next| next / page_size == address / page_size)
                .unwrap_or(page_start + page_len);
            let bounded = disassemble(image, address, Some(limit));
            if let Ok(function) = &bounded {
                finder.references(function);
                // Bytes after a function that are not more of its code are
                // most likely a function nothing refers to.
                // The next function usually starts where this one's
                // reachable code ends. If that fails, this function may end
                // in unreachable code, and the next starts after that.
                let tail = function.tail_address();
                let candidates = match function.extent {
                    Extent::Gap { decodable_end, .. } => {
                        [Some(tail), Some(address + decodable_end)]
                    }
                    Extent::Table if tail < limit => [Some(tail), None],
                    _ => [None, None],
                };
                let untried = candidates.into_iter().flatten().find(|candidate| {
                    !finder.found.contains_key(candidate) && !finder.rejected.contains(candidate)
                });
                if let Some(candidate) = untried {
                    finder.speculative.insert(candidate);
                    finder.pending.push(candidate);
                }
            }
            finder.found.insert(address, bounded);
        }
        let found = &finder.found;
        finder
            .pending
            .retain(|address| !found.contains_key(address));
        if finder.pending.is_empty() {
            break;
        }
        finder.run();
    }

    Functions {
        by_address: finder.found,
        unreferenced: finder.unreferenced,
        dangling: finder.dangling,
    }
}

struct Finder<'a> {
    image: &'a Image,
    pending: Vec<u32>,
    seen_lists: BTreeSet<u32>,
    found: BTreeMap<u32, Result<Function, DisassembleError>>,
    /// Addresses being tried as functions without a reference to them.
    speculative: BTreeSet<u32>,
    /// Guesses that did not disassemble.
    rejected: BTreeSet<u32>,
    unreferenced: BTreeSet<u32>,
    dangling: BTreeSet<u32>,
}

impl Finder<'_> {
    fn roots(&mut self, use_debug_info: bool) {
        let image = self.image;
        self.pending.push(image.entry_point.code_offset);
        for symbol in &image.symbols {
            self.value(symbol.value);
        }
        if use_debug_info {
            self.pending.extend(&image.debug.method_headers);
            for symbol in &image.debug.global_symbols {
                if let GlobalSymbolKind::Function { code_offset, .. } = symbol.kind {
                    self.pending.push(code_offset);
                }
            }
        }
        for block in &image.object_blocks {
            let layout = match image.metaclass_of(block).base_name() {
                "tads-object" | "int-class-mod" => ObjectLayout::Properties,
                "vector" | "anon-func-ptr" => ObjectLayout::Vector,
                "lookuptable" | "weakreflookuptable" => ObjectLayout::LookupTable,
                _ => continue,
            };
            for object in &block.objects {
                // Object data this crate cannot follow is skipped; it only
                // means fewer roots.
                let _ = self.object(layout, &object.data);
            }
        }
    }

    fn run(&mut self) {
        while let Some(address) = self.pending.pop() {
            if self.found.contains_key(&address) {
                continue;
            }
            let result = disassemble(self.image, address, None);
            let speculative = self.speculative.remove(&address);
            match &result {
                Ok(function) => {
                    self.references(function);
                    if speculative {
                        self.unreferenced.insert(address);
                    }
                }
                // A guess that did not work out is not a finding, but it
                // must not be tried again.
                Err(_) if speculative => {
                    self.rejected.insert(address);
                    continue;
                }
                Err(DisassembleError::BadAddress) if self.is_beyond_code(address) => {
                    self.dangling.insert(address);
                    continue;
                }
                Err(_) => {}
            }
            self.found.insert(address, result);
        }
    }

    /// Whether an address is past the last byte of code in the image.
    fn is_beyond_code(&self, address: u32) -> bool {
        let pool = &self.image.code_pool;
        let page_size = pool.page_size().max(1);
        let index = (address / page_size) as usize;
        match pool.page(index) {
            Some(page) => (address % page_size) as usize >= page.len(),
            None => true,
        }
    }

    /// Queue everything a function's code refers to.
    fn references(&mut self, function: &Function) {
        for instruction in &function.instructions {
            for operand in &instruction.operands {
                match *operand {
                    Operand::Function(address) => self.pending.push(address),
                    Operand::List(offset) => self.list(offset),
                    _ => {}
                }
            }
            if let Payload::Switch { cases, .. } = &instruction.payload {
                for (value, _) in cases {
                    self.value(*value);
                }
            }
        }
    }

    fn value(&mut self, value: Value) {
        match value {
            Value::CodeOffset(address) | Value::FunctionPointer(address) => {
                self.pending.push(address);
            }
            Value::List(offset) => self.list(offset),
            _ => {}
        }
    }

    /// Follow a constant list: a count followed by that many values.
    fn list(&mut self, offset: u32) {
        if !self.seen_lists.insert(offset) {
            return;
        }
        let Some(data) = self.image.constant_pool.at(offset) else {
            return;
        };
        let Some(count) = data.get(..2).map(|b| u16::from_le_bytes([b[0], b[1]])) else {
            return;
        };
        let values: Vec<Value> = data[2..]
            .chunks_exact(Value::ENCODED_SIZE)
            .take(count as usize)
            .map(holder)
            .collect();
        for value in values {
            self.value(value);
        }
    }

    fn object(&mut self, layout: ObjectLayout, data: &[u8]) -> Option<()> {
        let u16_at = |at: usize| Some(u16::from_le_bytes([*data.get(at)?, *data.get(at + 1)?]));
        match layout {
            ObjectLayout::Properties => {
                // Superclass count, property count, flags, the superclass
                // IDs, then each property's ID and value.
                let superclasses = u16_at(0)? as usize;
                let properties = u16_at(2)? as usize;
                let table = data.get(6 + superclasses * 4..)?;
                for entry in table.chunks_exact(2 + Value::ENCODED_SIZE).take(properties) {
                    self.value(holder(&entry[2..]));
                }
            }
            ObjectLayout::Vector => {
                // Allocated count, used count, then the used elements.
                let used = u16_at(2)? as usize;
                for element in data.get(4..)?.chunks_exact(Value::ENCODED_SIZE).take(used) {
                    self.value(holder(element));
                }
            }
            ObjectLayout::LookupTable => {
                // Bucket count, entry count, first free index, the bucket
                // indexes, then each entry's key, value, and next index.
                let buckets = u16_at(0)? as usize;
                let entries = u16_at(2)? as usize;
                let table = data.get(6 + buckets * 2..)?;
                for entry in table
                    .chunks_exact(Value::ENCODED_SIZE * 2 + 2)
                    .take(entries)
                {
                    self.value(holder(&entry[..Value::ENCODED_SIZE]));
                    self.value(holder(&entry[Value::ENCODED_SIZE..]));
                }
            }
        }
        Some(())
    }
}

#[derive(Clone, Copy)]
enum ObjectLayout {
    Properties,
    Vector,
    LookupTable,
}

fn holder(bytes: &[u8]) -> Value {
    Value::decode([bytes[0], bytes[1], bytes[2], bytes[3], bytes[4]])
}
