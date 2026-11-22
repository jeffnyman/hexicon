//! Writing the program back out as an image, after it has run to set
//! itself up: preinitialization.
//!
//! The reference compiler runs a program once before it writes the
//! final image (`t3make -pre`, or `t3pre` on an image), and the image it
//! writes is the program as that run left it: every object the program
//! can still reach, in the image's own format, with the strings and
//! lists among them made constants. The code and the constants the image
//! had are kept, but for the code that only ran before the program
//! started, which is gone with the list of what it was for.
//!
//! This follows the reference's rewriter (`vmimgrb.cpp`) step by step,
//! so that the image is the same, byte for byte, for the same state.

use std::collections::HashMap;

use crate::error::{ErrParam, VmError, VmResult, code};
use crate::meta::MetaKind;
use crate::object::Object;
use crate::value::{ObjId, Value};
use crate::vm::Vm;

/// An image, as the rewriter writes it.
#[derive(Debug, Clone, Default)]
pub struct RebuiltImage {
    /// The image file.
    pub bytes: Vec<u8>,
    /// Stretches of it, as offset and length, that mean nothing: the
    /// value bytes of `nil` and `true`, and the half of a property ID's
    /// four bytes that it does not use. The reference writes them as
    /// its memory had them.
    pub unspecified: Vec<(usize, usize)>,
}

/// The size of the file header: the signature, the version, the
/// compiler's own 32 bytes, and the time stamp.
const HEADER_SIZE: usize = 11 + 2 + 32 + 24;

/// The most data an object block holds before another is started.
const OBJS_BLOCK_LIMIT: usize = 64000;

/// Where the constants made of strings and lists go: new pages after
/// the image's own, each item on one page.
struct ConstMapper {
    page_size: usize,
    first_page: usize,
    base: usize,
    next_free: usize,
    rem: usize,
    /// Each object made a constant: its address in the pool.
    addrs: HashMap<ObjId, u32>,
    /// The pages, and how much of each is used.
    pages: Vec<(Vec<u8>, usize)>,
    /// For each page, the stretches that mean nothing.
    page_unspecified: Vec<Vec<(usize, usize)>>,
}

impl ConstMapper {
    fn new(page_size: usize, page_count: usize) -> Self {
        let base = page_size * page_count;
        Self {
            page_size,
            first_page: page_count,
            base,
            next_free: base,
            rem: page_size,
            addrs: HashMap::new(),
            pages: Vec::new(),
            page_unspecified: Vec::new(),
        }
    }

    /// The address of an object made a constant. The reference cannot
    /// tell an address of zero from none.
    fn addr(&self, id: ObjId) -> Option<u32> {
        self.addrs.get(&id).copied().filter(|&addr| addr != 0)
    }

    fn reserve(&mut self, id: ObjId, len: usize) {
        // What does not fit on a page stays an object.
        if len > self.page_size {
            return;
        }
        if len > self.rem {
            self.next_free += self.rem;
            self.rem = self.page_size;
        }
        self.addrs.insert(id, self.next_free as u32);
        self.next_free += len;
        self.rem -= len;
    }

    fn prepare_to_store(&mut self) {
        let used = self.next_free - self.base;
        let count = used.div_ceil(self.page_size.max(1));
        self.pages = (0..count).map(|_| (vec![0; self.page_size], 0)).collect();
        self.page_unspecified = vec![Vec::new(); count];
    }

    fn store(&mut self, id: ObjId, data: &Data) {
        let Some(addr) = self.addr(id) else {
            return;
        };
        let at = addr as usize - self.base;
        let (page, ofs) = (at / self.page_size, at % self.page_size);
        let (bytes, used) = &mut self.pages[page];
        bytes[ofs..ofs + data.bytes.len()].copy_from_slice(&data.bytes);
        *used = (*used).max(ofs + data.bytes.len());
        self.page_unspecified[page]
            .extend(data.unspecified.iter().map(|&(at, len)| (ofs + at, len)));
    }
}

/// An image file as the rewriter writes it, with the stretches that
/// mean nothing.
#[derive(Default)]
struct ImageOut {
    out: Vec<u8>,
    unspecified: Vec<(usize, usize)>,
}

impl ImageOut {
    fn u16(&mut self, value: usize) {
        self.out.extend_from_slice(&(value as u16).to_le_bytes());
    }

    fn u32(&mut self, value: u32) {
        self.out.extend_from_slice(&value.to_le_bytes());
    }

    /// A block: its type, its size, and its flags (1 for one the VM
    /// has to understand).
    fn block(&mut self, kind: &[u8; 4], data: &[u8], unspecified: &[(usize, usize)]) {
        self.out.extend_from_slice(kind);
        self.u32(data.len() as u32);
        self.u16(1);
        let at = self.out.len();
        self.unspecified
            .extend(unspecified.iter().map(|&(ofs, len)| (at + ofs, len)));
        self.out.extend_from_slice(data);
    }
}

/// Data for an object or a block, with the stretches that mean nothing.
#[derive(Default)]
struct Data {
    bytes: Vec<u8>,
    unspecified: Vec<(usize, usize)>,
}

impl Data {
    fn u8(&mut self, value: u8) {
        self.bytes.push(value);
    }

    fn u16(&mut self, value: usize) {
        self.bytes.extend_from_slice(&(value as u16).to_le_bytes());
    }

    fn u32(&mut self, value: u32) {
        self.bytes.extend_from_slice(&value.to_le_bytes());
    }

    fn raw(&mut self, bytes: &[u8]) {
        self.bytes.extend_from_slice(bytes);
    }

    /// A value as five bytes. `nil` and `true` have no value, and a
    /// property ID is two bytes of the four.
    fn value(&mut self, value: Value) {
        let at = self.bytes.len();
        match value {
            Value::Nil | Value::True | Value::Empty => self.unspecified.push((at + 1, 4)),
            Value::Prop(_) => self.unspecified.push((at + 3, 2)),
            _ => {}
        }
        self.bytes.extend_from_slice(&value.to_holder());
    }

    /// Another's data appended, with its stretches.
    fn append(&mut self, other: Data) {
        let at = self.bytes.len();
        self.unspecified.extend(
            other
                .unspecified
                .into_iter()
                .map(|(ofs, len)| (at + ofs, len)),
        );
        self.bytes.extend(other.bytes);
    }
}

/// What the rewriter needs of the image it was given: where its blocks
/// are.
struct Block<'a> {
    kind: [u8; 4],
    header: &'a [u8],
    data: &'a [u8],
    /// Where the block starts in the image.
    at: usize,
}

fn blocks(image: &[u8]) -> VmResult<Vec<Block<'_>>> {
    let bad = || VmError::new(code::NOT_AN_IMAGE_FILE);
    let mut out = Vec::new();
    let mut at = HEADER_SIZE;
    while at + 10 <= image.len() {
        let header = &image[at..at + 10];
        let size = u32::from_le_bytes([header[4], header[5], header[6], header[7]]) as usize;
        let data = image.get(at + 10..at + 10 + size).ok_or_else(bad)?;
        let kind = [header[0], header[1], header[2], header[3]];
        out.push(Block {
            kind,
            header,
            data,
            at,
        });
        at += 10 + size;
        if &kind == b"EOF " {
            break;
        }
    }
    Ok(out)
}

impl Vm {
    /// Give the program the compiler's table of global symbols and of
    /// macros, for `t3GetGlobalSymbols`, as the compiler does for a
    /// program it runs before writing the image. The order is the
    /// order of the tables the program sees.
    pub fn set_runtime_symbols(
        &mut self,
        symbols: Vec<hexicon_image::GlobalSymbol>,
        macros: Vec<hexicon_image::Macro>,
    ) {
        self.debug.global_symbols = symbols;
        self.debug.macros = macros;
    }

    /// Run the program as it is run before its image is written: with
    /// `t3GetVMPreinitMode()` true, and with the arguments given.
    pub fn run_preinit(&mut self, args: &[String]) -> VmResult<()> {
        self.preinit_mode = true;
        let result = self.run(args);
        self.preinit_mode = false;
        result
    }

    /// Write the program as it now stands as an image, given the image
    /// it was loaded from. This is what the reference does after
    /// preinitialization.
    pub fn rewrite_image(&mut self, original: &[u8]) -> VmResult<RebuiltImage> {
        self.rewrite_image_keeping(original, &[])
    }

    /// The same, given the stretches of the image it was loaded from
    /// that mean nothing: those in the parts that are copied are
    /// carried over to where the parts go.
    pub fn rewrite_image_keeping(
        &mut self,
        original: &[u8],
        unspecified: &[(usize, usize)],
    ) -> VmResult<RebuiltImage> {
        if original.len() < HEADER_SIZE {
            return Err(VmError::new(code::NOT_AN_IMAGE_FILE));
        }
        let blocks = blocks(original)?;

        // What the image's pools are like, and where the code that only
        // runs before the program starts begins.
        let mut const_pages = (0usize, 0usize);
        let mut static_code_offset = 0u32;
        for block in &blocks {
            match &block.kind {
                b"CPDF" if block.data.len() >= 10 => {
                    let pool = u16::from_le_bytes([block.data[0], block.data[1]]);
                    let count = u32::from_le_bytes(block.data[2..6].try_into().unwrap());
                    let size = u32::from_le_bytes(block.data[6..10].try_into().unwrap());
                    if pool == 2 {
                        const_pages = (count as usize, size as usize);
                    }
                }
                b"SINI" if block.data.len() >= 8 => {
                    static_code_offset = u32::from_le_bytes(block.data[4..8].try_into().unwrap());
                }
                _ => {}
            }
        }

        // A time zone is written with its offsets as of now.
        self.note_save_time();

        // Nothing is left to undo, and nothing on the stack: what is
        // still reachable is what goes in the image.
        self.objects.drop_undo();
        self.stack.clear();
        self.frames.clear();
        self.symbol_table = None;
        self.macro_table = None;
        self.collect_garbage(false);
        let ids: Vec<ObjId> = self.objects.ids().collect();
        for &id in &ids {
            if let Some(kind) = self.kind_of(id) {
                self.meta.add_if_new(kind);
            }
        }

        // The strings and lists become constants, where they fit on a
        // page; and every value that is one of them becomes the
        // constant.
        let (page_count, page_size) = const_pages;
        let mut mapper = ConstMapper::new(page_size, page_count);
        for &id in &ids {
            match self.objects.get(id) {
                Some(Object::String(text)) => mapper.reserve(id, 2 + text.len()),
                Some(Object::List(items)) => {
                    mapper.reserve(id, 2 + items.len() * Value::HOLDER_SIZE);
                }
                _ => {}
            }
        }
        mapper.prepare_to_store();
        for &id in &ids {
            self.convert_to_const(id, &mut mapper)?;
        }

        // The new image: the header, and the blocks of the old one but
        // for the objects, the classes, and the static initializers,
        // which are written anew; and the code that ran before the
        // program started, which is gone.
        let mut out = ImageOut::default();
        // What means nothing in a part that is copied as it is.
        let copy_unspecified = |out: &mut ImageOut, from: usize, len: usize| {
            let to = out.out.len();
            for &(ofs, size) in unspecified {
                if ofs >= from && ofs + size <= from + len {
                    out.unspecified.push((ofs - from + to, size));
                }
            }
        };
        copy_unspecified(&mut out, 0, HEADER_SIZE);
        out.out.extend_from_slice(&original[..HEADER_SIZE]);
        let mut code_page_size = 0usize;
        let mut xor_mask = 0xEFu8;
        for block in &blocks {
            match &block.kind {
                b"OBJS" | b"MCLD" | b"SINI" => {}
                b"CPDF" => {
                    let mut data = block.data.to_vec();
                    let pool = u16::from_le_bytes([data[0], data[1]]);
                    let size = u32::from_le_bytes(data[6..10].try_into().unwrap());
                    if pool == 2 {
                        let count = page_count + mapper.pages.len();
                        data[2..6].copy_from_slice(&(count as u32).to_le_bytes());
                    }
                    if pool == 1 && static_code_offset != 0 {
                        let count = static_code_offset / size;
                        data[2..6].copy_from_slice(&count.to_le_bytes());
                        code_page_size = size as usize;
                    }
                    out.out.extend_from_slice(block.header);
                    out.out.extend_from_slice(&data);
                }
                b"CPPG" => {
                    let pool = u16::from_le_bytes([block.data[0], block.data[1]]);
                    let index = u32::from_le_bytes(block.data[2..6].try_into().unwrap()) as usize;
                    if pool == 2 {
                        xor_mask = block.data[6];
                    }
                    let gone = pool == 1
                        && static_code_offset != 0
                        && index * code_page_size >= static_code_offset as usize;
                    if !gone {
                        copy_unspecified(&mut out, block.at, 10 + block.data.len());
                        out.out.extend_from_slice(block.header);
                        out.out.extend_from_slice(block.data);
                    }
                }
                b"EOF " => {
                    self.write_meta_table(&mut out);
                    self.write_objects(&ids, &mapper, &mut out)?;
                    for (index, (bytes, used)) in mapper.pages.iter().enumerate() {
                        let mut data = Vec::with_capacity(7 + used);
                        data.extend_from_slice(&2u16.to_le_bytes());
                        data.extend_from_slice(&((mapper.first_page + index) as u32).to_le_bytes());
                        data.push(xor_mask);
                        data.extend(bytes[..*used].iter().map(|byte| byte ^ xor_mask));
                        let unspecified: Vec<(usize, usize)> = mapper.page_unspecified[index]
                            .iter()
                            .map(|&(at, len)| (7 + at, len))
                            .collect();
                        out.block(b"CPPG", &data, &unspecified);
                    }
                    out.out.extend_from_slice(block.header);
                    out.out.extend_from_slice(block.data);
                }
                _ => {
                    copy_unspecified(&mut out, block.at, 10 + block.data.len());
                    out.out.extend_from_slice(block.header);
                    out.out.extend_from_slice(block.data);
                }
            }
        }
        Ok(RebuiltImage {
            bytes: out.out,
            unspecified: out.unspecified,
        })
    }

    /// Make the values in an object that are strings and lists made
    /// constants into the constants; and store a string or a list made
    /// one.
    fn convert_to_const(&mut self, id: ObjId, mapper: &mut ConstMapper) -> VmResult<()> {
        let to_const = |value: Value, objects: &crate::object::ObjectTable| -> Value {
            let Value::Obj(target) = value else {
                return value;
            };
            let Some(addr) = mapper.addr(target) else {
                return value;
            };
            match objects.get(target) {
                Some(Object::String(_)) => Value::SString(addr),
                Some(Object::List(_)) => Value::List(addr),
                _ => value,
            }
        };
        // The new values, worked out first, as the table is borrowed.
        let mut store: Option<Data> = None;
        let replacement: Option<Object> = match self.objects.get(id) {
            Some(Object::Tads(tads) | Object::IntClassMod(tads)) => {
                let changes: Vec<(u16, Value)> = tads
                    .props()
                    .iter()
                    .filter(|entry| entry.modified && !matches!(entry.value, Value::Empty))
                    .filter_map(|entry| {
                        let new = to_const(entry.value, &self.objects);
                        (new != entry.value).then_some((entry.prop, new))
                    })
                    .collect();
                if !changes.is_empty() {
                    let mut object = self.objects.get(id).cloned().expect("the object");
                    if let Object::Tads(tads) | Object::IntClassMod(tads) = &mut object {
                        for (prop, value) in changes {
                            tads.set_value_in_place(prop, value);
                        }
                    }
                    Some(object)
                } else {
                    None
                }
            }
            Some(Object::List(items)) => {
                let items: Vec<Value> = items.iter().map(|&v| to_const(v, &self.objects)).collect();
                let mut data = Data::default();
                data.u16(items.len());
                for &item in &items {
                    data.value(item);
                }
                store = Some(data);
                Some(Object::List(items.into()))
            }
            Some(Object::String(text)) => {
                let mut data = Data::default();
                data.u16(text.len());
                data.raw(text);
                store = Some(data);
                None
            }
            Some(Object::Vector(items)) => Some(Object::Vector(
                items.iter().map(|&v| to_const(v, &self.objects)).collect(),
            )),
            Some(Object::AnonFn(items)) => Some(Object::AnonFn(
                items.iter().map(|&v| to_const(v, &self.objects)).collect(),
            )),
            Some(Object::LookupTable(table)) => {
                let mut table = table.clone();
                table.map_values(|value| to_const(value, &self.objects));
                Some(Object::LookupTable(table))
            }
            Some(Object::IndexedIter {
                collection,
                current,
                first,
                last,
            }) => Some(Object::IndexedIter {
                collection: to_const(*collection, &self.objects),
                current: *current,
                first: *first,
                last: *last,
            }),
            Some(Object::Pattern { source, compiled }) => Some(Object::Pattern {
                source: to_const(*source, &self.objects),
                compiled: compiled.clone(),
            }),
            Some(Object::Dict(dictionary)) => {
                // A dictionary's objects cannot be constants.
                let fail = |text: &[u8]| {
                    VmError::with(
                        code::DICT_NO_CONST,
                        vec![ErrParam::Str(String::from_utf8_lossy(text).into_owned())],
                    )
                };
                if mapper.addr(dictionary.comparator).is_some() {
                    return Err(fail(b"<comparator>"));
                }
                if let Some(word) = dictionary.words().find(|word| {
                    word.entries
                        .iter()
                        .any(|&(obj, _)| mapper.addr(obj).is_some())
                }) {
                    return Err(fail(&word.text));
                }
                None
            }
            _ => None,
        };
        if let Some(object) = replacement {
            self.objects.replace(id, object);
        }
        if let Some(data) = store {
            mapper.store(id, &data);
        }
        Ok(())
    }

    /// The table of intrinsic classes: each by the name the image gave
    /// it, with the properties the image gave its methods.
    fn write_meta_table(&self, out: &mut ImageOut) {
        let mut data = Data::default();
        data.u16(self.meta.len());
        for index in 0..self.meta.len() {
            let Some(entry) = self.meta.get(index) else {
                continue;
            };
            let name = entry.image_name.as_bytes();
            let size = 2 + 1 + name.len() + 2 + 2 + 2 * entry.method_props.len();
            data.u16(size);
            data.u8(name.len() as u8);
            data.raw(name);
            data.u16(entry.method_props.len());
            data.u16(2);
            for &prop in &entry.method_props {
                data.u16(prop.into());
            }
        }
        out.block(b"MCLD", &data.bytes, &[]);
    }

    /// The objects, a class at a time in the order of the table of
    /// classes, those that are saved before those that are not; and in
    /// a class, in the order of their IDs. A block holds what fits in
    /// 64,000 bytes, unless an object is larger than 65,535 bytes, which
    /// starts blocks with a larger size for each object.
    fn write_objects(
        &self,
        ids: &[ObjId],
        mapper: &ConstMapper,
        out: &mut ImageOut,
    ) -> VmResult<()> {
        let kinds: Vec<Option<MetaKind>> = ids.iter().map(|&id| self.kind_of(id)).collect();
        for index in 0..self.meta.len() {
            let Some(entry) = self.meta.get(index) else {
                continue;
            };
            for transient in [false, true] {
                let mut block = Data::default();
                let mut count = 0usize;
                let mut large = false;
                let finish =
                    |block: &mut Data, count: &mut usize, large: bool, out: &mut ImageOut| {
                        if *count == 0 {
                            return;
                        }
                        let mut head = Data::default();
                        head.u16(*count);
                        head.u16(index);
                        head.u16(usize::from(large) | usize::from(transient) << 1);
                        head.append(std::mem::take(block));
                        out.block(b"OBJS", &head.bytes, &head.unspecified);
                        *count = 0;
                    };
                for (&id, kind) in ids.iter().zip(&kinds) {
                    if *kind != Some(entry.kind)
                        || self.meta.index_of(entry.kind) != Some(index)
                        || self.objects.is_transient(id) != transient
                        || mapper.addr(id).is_some()
                    {
                        continue;
                    }
                    let data = self.image_data(id)?;
                    if data.bytes.is_empty() {
                        continue;
                    }
                    let size = data.bytes.len();
                    if size > 65535 && !large {
                        finish(&mut block, &mut count, large, out);
                        large = true;
                    }
                    if block.bytes.len() + size + 6 > OBJS_BLOCK_LIMIT && count != 0 && !large {
                        finish(&mut block, &mut count, large, out);
                    }
                    block.u32(id.0);
                    if large {
                        block.u32(size as u32);
                    } else {
                        block.u16(size);
                    }
                    block.append(data);
                    count += 1;
                }
                finish(&mut block, &mut count, large, out);
            }
        }
        Ok(())
    }

    /// An object's data as an image holds it.
    fn image_data(&self, id: ObjId) -> VmResult<Data> {
        let mut d = Data::default();
        let Some(object) = self.objects.get(id) else {
            return Ok(d);
        };
        match object {
            Object::Tads(tads) | Object::IntClassMod(tads) => {
                // The superclasses, then the properties with values, in
                // the order of their IDs.
                let mut props: Vec<(u16, Value)> = tads
                    .props()
                    .iter()
                    .filter(|entry| !matches!(entry.value, Value::Empty))
                    .map(|entry| (entry.prop, entry.value))
                    .collect();
                props.sort_by_key(|(prop, _)| *prop);
                d.u16(tads.superclasses.len());
                d.u16(props.len());
                d.u16(tads.load_flags.into());
                for superclass in &tads.superclasses {
                    d.u32(superclass.0);
                }
                for (prop, value) in props {
                    d.u16(prop.into());
                    d.value(value);
                }
            }
            Object::String(text) => {
                d.u16(text.len());
                d.raw(text);
            }
            Object::List(items) => {
                d.u16(items.len());
                for &item in items.iter() {
                    d.value(item);
                }
            }
            Object::Vector(items) | Object::AnonFn(items) => {
                d.u16(self.objects.vector_alloc(id).max(items.len()));
                d.u16(items.len());
                for &item in items {
                    d.value(item);
                }
            }
            Object::IndexedIter {
                collection,
                current,
                first,
                last,
            } => {
                d.value(*collection);
                d.u32(*current as u32);
                d.u32(*first as u32);
                d.u32(*last as u32);
                d.u32(0);
            }
            Object::LookupTable(table) => {
                let (buckets, slots, first_free) = table.save_parts();
                d.u16(buckets.len());
                d.u16(slots.len());
                d.u16(first_free);
                for bucket in buckets {
                    d.u16(bucket);
                }
                for (key, value, next) in slots {
                    d.value(key);
                    d.value(value);
                    d.u16(next);
                }
                d.value(table.default_value);
            }
            Object::LookupIter { table, slot } => {
                d.value(Value::Obj(*table));
                d.u16(*slot as usize);
                d.u16(0);
            }
            Object::CharSet { name, .. } => {
                d.u16(name.len());
                d.raw(name.as_bytes());
            }
            Object::FileName(name) => {
                d.u32(name.special as u32);
                if name.special == 0 {
                    d.u16(name.path.len());
                    d.raw(name.path.as_bytes());
                }
            }
            Object::Date { dayno, daytime } => {
                d.u32(*dayno as u32);
                d.u32(*daytime as u32);
            }
            Object::TimeZone(zone) => {
                let (gmtofs, save, abbr, name) = self.zone_saved_form(zone);
                d.u32(gmtofs as u32);
                d.u32(save as u32);
                for text in [name, abbr] {
                    let text = &text.as_bytes()[..text.len().min(255)];
                    d.u8(text.len() as u8);
                    d.raw(text);
                }
            }
            Object::File(file) => {
                d.u32(file.charset.0);
                d.u8(file.mode);
                d.u8(file.access);
                d.u32(u32::from(file.out_of_sync));
            }
            // A temporary file has nothing to keep, and is left out.
            Object::TempFile(_) => {}
            Object::StrComp(comparator) => d.raw(&comparator.to_bytes()),
            Object::GramProd(production) => d.raw(&production.to_bytes()),
            Object::Dict(dictionary) => {
                d.u32(dictionary.comparator.0);
                let words: Vec<_> = dictionary.words().collect();
                d.u16(words.len());
                for word in words {
                    let text = &word.text[..word.text.len().min(255)];
                    d.u8(text.len() as u8);
                    d.raw(&text.iter().map(|byte| byte ^ 0xBD).collect::<Vec<u8>>());
                    d.u16(word.entries.len());
                    for &(obj, prop) in &word.entries {
                        d.u32(obj.0);
                        d.u16(prop.into());
                    }
                }
            }
            Object::ByteArray(bytes) => {
                d.u32(bytes.len() as u32);
                d.raw(bytes);
            }
            Object::BigNumber(big) => {
                // The fifth byte has the sign and the kind of number in
                // its low bits; the reference never sets the high ones.
                d.unspecified.push((d.bytes.len() + 4, 1));
                d.raw(&big.to_bytes());
            }
            // The length of the code, the number of objects it refers
            // to, the method context, the source, the code, and where in
            // it each object's number is.
            Object::DynFunc(func) => {
                d.u16(func.code.len());
                d.u16(func.obj_refs.len());
                d.value(func.method_ctx);
                d.value(func.source);
                d.raw(&func.code);
                for &at in &func.obj_refs {
                    d.u16(usize::from(at));
                }
            }
            Object::StringBuffer(buffer) => {
                // The room, the step it grows by, the length, and the
                // characters, each in 16 bits, as the reference keeps them.
                d.u32(buffer.room);
                d.u32(buffer.step);
                d.u32(buffer.len() as u32);
                for &c in buffer.iter() {
                    d.u16(u32::from(c) as usize & 0xffff);
                }
            }
            Object::Pattern { source, .. } => d.value(*source),
            Object::Class(class) => {
                d.u16(2 + 2 + 4 + Value::HOLDER_SIZE);
                d.u16(class.meta_index.into());
                d.u32(class.modifier.0);
                d.value(class.class_state);
            }
            Object::Opaque { data, .. } => d.raw(data),
        }
        Ok(d)
    }
}
