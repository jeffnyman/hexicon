//! Writing the program image: the file the virtual machine loads.
//!
//! The generated code and constants are divided into pages, addresses
//! that could not be known while generating are filled in, and the
//! whole is written out as the image's blocks, in the reference
//! compiler's order.

use crate::emit::{Anchor, Generated, Target};
use crate::parse::{Global, Program};

/// The smallest a pool's pages can be.
const MIN_PAGE_SIZE: usize = 2048;
/// The mask the constant pool's pages are stored under.
const CONST_XOR_MASK: u8 = 0xDF;
/// The function the compiler always generates for setting up
/// multi-methods when the program starts. With none to set up, it only
/// returns.
const STATIC_INIT_STUB: [u8; 11] = [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x51];

/// The properties every image exports by name, with their numbers in a
/// program that defines no properties of its own.
const EXPORTED_PROPS: [(&str, u16); 20] = [
    ("Constructor", 1),
    ("Destructor", 2),
    ("GrammarProd.grammarInfo", 3),
    ("GrammarProd.grammarTag", 4),
    ("GrammarProd.miscVocab", 5),
    ("operator +", 11),
    ("operator -", 12),
    ("operator *", 13),
    ("operator /", 14),
    ("operator %", 15),
    ("operator ^", 16),
    ("operator <<", 17),
    ("operator >>", 18),
    ("operator >>>", 19),
    ("operator ~", 20),
    ("operator |", 21),
    ("operator &", 22),
    ("operator negate", 23),
    ("operator []", 24),
    ("operator []=", 25),
];

/// Where the items of a stream go in a pool.
struct Layout {
    page_size: usize,
    page_cnt: usize,
    free_ofs: usize,
    rem: usize,
}

impl Layout {
    /// Start a pool whose pages must hold an item of `max_len` bytes.
    fn new(max_len: usize) -> Self {
        let mut page_size = MIN_PAGE_SIZE;
        while page_size < max_len {
            page_size <<= 1;
        }
        Self {
            page_size,
            page_cnt: 1,
            free_ofs: 0,
            rem: page_size,
        }
    }

    /// Carry on with another stream, which starts on a new page.
    fn start_new_stream(&mut self) {
        self.rem = 0;
        self.free_ofs = self.page_size * self.page_cnt;
    }

    /// Give each item its address. An item that does not fit in what is
    /// left of a page starts the next page.
    fn place(&mut self, anchors: &[Anchor], stream_len: usize) -> Vec<usize> {
        let mut addrs = Vec::with_capacity(anchors.len());
        for (index, anchor) in anchors.iter().enumerate() {
            let end = anchors.get(index + 1).map_or(stream_len, |next| next.ofs);
            let len = end - anchor.ofs;
            if len > self.rem {
                self.free_ofs += self.rem;
                self.page_cnt += 1;
                self.rem = self.page_size;
            }
            addrs.push(self.free_ofs);
            self.free_ofs += len;
            self.rem -= len;
        }
        addrs
    }

    fn finish(&mut self) {
        if self.free_ofs == 0 {
            self.page_cnt = 0;
        }
    }
}

struct ImageWriter {
    out: Vec<u8>,
    /// Where in the file each item of the pool being written starts:
    /// the item's address, and its offset in the file.
    item_ofs: Vec<(usize, usize)>,
}

impl ImageWriter {
    fn block(&mut self, kind: &[u8; 4], mandatory: bool, data: &[u8]) {
        self.out.extend_from_slice(kind);
        self.out
            .extend_from_slice(&(data.len() as u32).to_le_bytes());
        self.out
            .extend_from_slice(&u16::from(mandatory).to_le_bytes());
        self.out.extend_from_slice(data);
    }

    /// Write a pool: its definition, then its pages. `items` are the
    /// pool's contents as (address, bytes), in order of address.
    fn pool(&mut self, id: u16, layout: &Layout, items: &[(usize, &[u8])], xor_mask: u8) {
        let mut def = Vec::new();
        def.extend_from_slice(&id.to_le_bytes());
        def.extend_from_slice(&(layout.page_cnt as u32).to_le_bytes());
        def.extend_from_slice(&(layout.page_size as u32).to_le_bytes());
        self.block(b"CPDF", true, &def);

        for page in 0..layout.page_cnt {
            let start = page * layout.page_size;
            let mut data = Vec::new();
            for (addr, bytes) in items {
                if (start..start + layout.page_size).contains(addr) {
                    // Items on a page follow one another without gaps.
                    debug_assert_eq!(addr - start, data.len());
                    // The page's data comes after the block's header and
                    // the page's own.
                    self.item_ofs
                        .push((*addr, self.out.len() + 10 + 7 + data.len()));
                    data.extend(bytes.iter().map(|byte| byte ^ xor_mask));
                }
            }
            let mut block = Vec::new();
            block.extend_from_slice(&id.to_le_bytes());
            block.extend_from_slice(&(page as u32).to_le_bytes());
            block.push(xor_mask);
            block.extend_from_slice(&data);
            self.block(b"CPPG", true, &block);
        }
    }
}

/// The pieces of a stream between its anchors.
fn items<'s>(stream: &'s [u8], anchors: &[Anchor], addrs: &[usize]) -> Vec<(usize, &'s [u8])> {
    anchors
        .iter()
        .enumerate()
        .map(|(index, anchor)| {
            let end = anchors.get(index + 1).map_or(stream.len(), |next| next.ofs);
            (addrs[index], &stream[anchor.ofs..end])
        })
        .collect()
}

/// The address of a function's code, if the function is defined.
fn func_anchor_addr(generated: &Generated, code_addrs: &[usize], func: usize) -> Option<usize> {
    generated
        .func_anchor
        .get(&func)
        .map(|anchor| code_addrs[*anchor])
}

/// An image, and the parts of it that mean nothing.
pub(crate) struct Image {
    pub bytes: Vec<u8>,
    /// Stretches of the image, as offset and length, whose contents are
    /// not the program's: the time stamp, the compiler's own bytes, and
    /// fields that are written but never read. The reference compiler
    /// leaves some of them holding whatever was in its memory.
    pub unspecified: Vec<(usize, usize)>,
    /// Where each function went in the code: its address, by its index
    /// in the program's functions.
    pub func_addrs: std::collections::HashMap<usize, u32>,
}

/// What an image compiled for debugging has besides the program.
pub(crate) struct DebugRecords {
    /// The source files of all the modules, in order, a file that two
    /// modules read once for each.
    pub files: Vec<String>,
    /// The number each value of an enumeration has in the image.
    pub enum_ids: Vec<u32>,
    /// The macros, as the `MACR` block has them, and the stretches of
    /// it that are the date and time of the compilation.
    pub macros: Vec<u8>,
    pub macros_unspecified: Vec<(usize, usize)>,
}

/// Write the image. `timestamp` is the 24 characters that record when
/// the program was compiled, and `tool_data` four bytes for the
/// compiler's own use. A program compiled for debugging has `debug`.
pub(crate) fn write_image(
    prog: &Program,
    mut generated: Generated,
    timestamp: &[u8; 24],
    tool_data: &[u8; 4],
    debug: Option<&DebugRecords>,
) -> Result<Image, String> {
    // Where the constants go. Pages are sized for twice the largest
    // item, unless that is large already.
    let mut max_item = (generated.max_str_len + 2).max(generated.max_list_cnt * 5 + 2);
    if max_item < 16 * 1024 {
        max_item <<= 1;
    }
    let mut const_layout = Layout::new(max_item);
    let const_addrs = const_layout.place(&generated.const_anchors, generated.consts.len());
    const_layout.finish();

    // Where the code goes: the program's functions, then, starting on a
    // page of its own, the code that runs before the program starts.
    let mut code_layout = Layout::new(generated.max_bytecode_len.max(STATIC_INIT_STUB.len()));
    let code_addrs = code_layout.place(&generated.code_anchors, generated.code.len());
    let first_static_page = code_layout.page_cnt;
    code_layout.start_new_stream();
    // That is the code for static properties, and after it the
    // function that sets up multi-methods.
    if !generated.has_mm_init {
        let stub_ofs = generated.static_code.len();
        generated.static_code.extend_from_slice(&STATIC_INIT_STUB);
        generated.static_anchors.push(Anchor { ofs: stub_ofs });
    }
    let static_addrs = code_layout.place(&generated.static_anchors, generated.static_code.len());
    code_layout.finish();

    // Fill in the addresses, in the code and in the constants.
    let Generated {
        code,
        static_code,
        consts,
        fixups,
        static_fixups,
        const_fixups,
        func_anchor,
        body_anchor,
        static_body_anchor,
        ..
    } = &mut generated;
    for (stream, fixups) in [
        (code, &*fixups),
        (static_code, &*static_fixups),
        (consts, &*const_fixups),
    ] {
        for fixup in fixups {
            let addr = match &fixup.target {
                Target::Func(func) => match func_anchor.get(func) {
                    Some(anchor) => code_addrs[*anchor],
                    None => {
                        return Err(format!(
                            "function \"{}\" is declared but not defined",
                            String::from_utf8_lossy(&prog.funcs[*func].name)
                        ));
                    }
                },
                Target::Const(anchor) => const_addrs[*anchor],
                Target::Body(body) => code_addrs[body_anchor[body]],
                Target::StaticBody(body) => static_addrs[static_body_anchor[body]],
                Target::Anchor(anchor) => code_addrs[*anchor],
                Target::FuncBase(func) => {
                    return Err(format!(
                        "function \"{}\" is modified but not defined",
                        String::from_utf8_lossy(&prog.funcs[*func].name)
                    ));
                }
            };
            stream[fixup.ofs..fixup.ofs + 4].copy_from_slice(&(addr as u32).to_le_bytes());
        }
    }

    // The program starts at `_main`.
    let main_addr = match prog.globals.get(b"_main".as_slice()) {
        Some(Global::Func(func)) => match generated.func_anchor.get(func) {
            Some(anchor) => code_addrs[*anchor],
            None => return Err("function _main() is not defined".to_owned()),
        },
        Some(_) => return Err("_main is not a function".to_owned()),
        None => return Err("function _main() is not defined".to_owned()),
    };

    let mut unspecified = Vec::new();
    let mut writer = ImageWriter {
        out: Vec::new(),
        item_ofs: Vec::new(),
    };
    writer.out.extend_from_slice(b"HX-image\r\n\x1a");
    writer.out.extend_from_slice(&1u16.to_le_bytes());
    writer.out.extend_from_slice(&[0; 28]);
    unspecified.push((writer.out.len(), tool_data.len() + timestamp.len()));
    writer.out.extend_from_slice(tool_data);
    writer.out.extend_from_slice(timestamp);

    // The entry point, and the sizes of the records in a function's
    // tables.
    let mut entp = Vec::new();
    entp.extend_from_slice(&(main_addr as u32).to_le_bytes());
    for size in [10u16, 10, 10, 0, 6, 2, 8] {
        entp.extend_from_slice(&size.to_le_bytes());
    }
    writer.block(b"ENTP", true, &entp);

    // The symbols the virtual machine looks up by name.
    let mut symd = Vec::new();
    let count = generated.exports.len() + EXPORTED_PROPS.len() + 1;
    symd.extend_from_slice(&(count as u16).to_le_bytes());
    // What the program exports comes first.
    for export in &generated.exports {
        let value = match export.func {
            Some(func) => match func_anchor_addr(&generated, &code_addrs, func) {
                Some(addr) => addr as u32,
                None => {
                    return Err(format!(
                        "function \"{}\" is declared but not defined",
                        String::from_utf8_lossy(&prog.funcs[func].name)
                    ));
                }
            },
            None => export.value,
        };
        symd.push(export.kind);
        if export.kind == 6 {
            // A property number is two bytes of the four.
            unspecified.push((writer.out.len() + 10 + symd.len() + 2, 2));
        }
        symd.extend_from_slice(&value.to_le_bytes());
        symd.push(export.name.len() as u8);
        symd.extend_from_slice(&export.name);
    }
    let props = std::iter::once(("LastProp", generated.next_prop_id)).chain(EXPORTED_PROPS);
    for (name, prop) in props {
        symd.push(6);
        symd.extend_from_slice(&prop.to_le_bytes());
        // A value has room for four bytes; a property number is two.
        unspecified.push((writer.out.len() + 10 + symd.len(), 2));
        symd.extend_from_slice(&[0, 0]);
        symd.push(name.len() as u8);
        symd.extend_from_slice(name.as_bytes());
    }
    writer.block(b"SYMD", false, &symd);

    // The sets of built-in functions the program uses.
    let mut fnsd = Vec::new();
    fnsd.extend_from_slice(&(prog.fnsets.len() as u16).to_le_bytes());
    for name in &prog.fnsets {
        fnsd.push(name.len() as u8);
        fnsd.extend_from_slice(name);
    }
    writer.block(b"FNSD", true, &fnsd);

    // The built-in classes the image depends on. The class of the
    // objects that stand for built-in classes is one, and goes last
    // unless the program has declared it.
    const INTRINSIC_CLASS: &[u8] = b"intrinsic-class";
    let mut metas = std::mem::take(&mut generated.metas);
    // The classes of the objects that constants are made into, if
    // there are any and the program has not declared them.
    let mut meta_for = |name: &[u8], wanted: bool| -> u16 {
        if !wanted {
            return 0;
        }
        let found = metas.iter().position(|(full, ..)| {
            full.strip_prefix(name)
                .is_some_and(|rest| rest.first() == Some(&b'/'))
        });
        let index = found.unwrap_or_else(|| {
            metas.push(([name, b"/000000"].concat(), Vec::new(), None));
            metas.len() - 1
        });
        index as u16
    };
    let bignum_meta = meta_for(b"bignumber", !generated.bignums.is_empty());
    let rexpat_meta = meta_for(b"regex-pattern", !generated.rexpats.is_empty());
    let is_intrinsic_class = |name: &[u8]| {
        name.strip_prefix(INTRINSIC_CLASS)
            .is_some_and(|rest| rest.first() == Some(&b'/'))
    };
    let intrinsic_class = match metas.iter().position(|(name, ..)| is_intrinsic_class(name)) {
        Some(index) => index,
        None => {
            metas.push((b"intrinsic-class/000000".to_vec(), Vec::new(), None));
            metas.len() - 1
        }
    };
    let mut mcld = Vec::new();
    mcld.extend_from_slice(&(metas.len() as u16).to_le_bytes());
    for (name, props, _) in &metas {
        // Each entry: its own size, the name, and the class's
        // properties, two bytes each.
        let size = 2 + 1 + name.len() + 2 + 2 + 2 * props.len();
        mcld.extend_from_slice(&(size as u16).to_le_bytes());
        mcld.push(name.len() as u8);
        mcld.extend_from_slice(name);
        mcld.extend_from_slice(&(props.len() as u16).to_le_bytes());
        mcld.extend_from_slice(&2u16.to_le_bytes());
        for prop in props {
            mcld.extend_from_slice(&prop.to_le_bytes());
        }
    }
    writer.block(b"MCLD", true, &mcld);

    let mut code_items = items(&generated.code, &generated.code_anchors, &code_addrs);
    code_items.extend(items(
        &generated.static_code,
        &generated.static_anchors,
        &static_addrs,
    ));
    writer.pool(1, &code_layout, &code_items, 0);
    for ofs in &generated.static_unspecified {
        let index = generated
            .static_anchors
            .partition_point(|anchor| anchor.ofs <= *ofs)
            - 1;
        let (_, file_ofs) = writer.item_ofs[generated.code_anchors.len() + index];
        unspecified.push((file_ofs + ofs - generated.static_anchors[index].ofs, 2));
    }
    // The code's meaningless fields, now that it is known where in the
    // file each function went.
    for ofs in &generated.unspecified {
        let index = generated
            .code_anchors
            .partition_point(|anchor| anchor.ofs <= *ofs)
            - 1;
        let (_, file_ofs) = writer.item_ofs[index];
        unspecified.push((file_ofs + ofs - generated.code_anchors[index].ofs, 2));
    }
    let const_items = items(&generated.consts, &generated.const_anchors, &const_addrs);
    let first_const_item = writer.item_ofs.len();
    writer.pool(2, &const_layout, &const_items, CONST_XOR_MASK);
    for ofs in &generated.const_unspecified {
        let index = generated
            .const_anchors
            .partition_point(|anchor| anchor.ofs <= *ofs)
            - 1;
        let (_, file_ofs) = writer.item_ofs[first_const_item + index];
        unspecified.push((file_ofs + ofs - generated.const_anchors[index].ofs, 4));
    }
    for ofs in &generated.const_unspecified2 {
        let index = generated
            .const_anchors
            .partition_point(|anchor| anchor.ofs <= *ofs)
            - 1;
        let (_, file_ofs) = writer.item_ofs[first_const_item + index];
        unspecified.push((file_ofs + ofs - generated.const_anchors[index].ofs, 2));
    }

    // The objects, all in one block. (The reference means to start a
    // new block when one is full, but never counts what it has put in.)
    // After them, in a block of their own, come the objects that hold
    // what the program adds to built-in classes, which are of the
    // seventh of the metaclasses every image has.
    // Of each kind, the transient ones are in a block after the rest,
    // which says that is what they are.
    let mut kinds: [Vec<crate::emit::GenObj>; 4] = Default::default();
    for (index, obj) in std::mem::take(&mut generated.objs).into_iter().enumerate() {
        // An object that another source file's `replace` took the
        // place of is not in the image.
        if generated.dead_objs.contains(&index) {
            continue;
        }
        kinds[2 * usize::from(obj.icmod) + usize::from(obj.transient)].push(obj);
    }
    for (kind, mut gen_objs) in kinds.into_iter().enumerate() {
        if gen_objs.is_empty() {
            continue;
        }
        let metaclass: u16 = if kind >= 2 { 6 } else { 0 };
        let flags: u16 = if kind % 2 == 1 { 2 } else { 0 };
        let mut objs = Vec::new();
        objs.extend_from_slice(&(gen_objs.len() as u16).to_le_bytes());
        objs.extend_from_slice(&metaclass.to_le_bytes());
        objs.extend_from_slice(&flags.to_le_bytes());
        for obj in &mut gen_objs {
            crt_qsort(&mut obj.props, |a, b| a.id.cmp(&b.id));
            let size = 6 + 4 * obj.superclasses.len() + 7 * obj.props.len();
            objs.extend_from_slice(&obj.id.to_le_bytes());
            objs.extend_from_slice(&(size as u16).to_le_bytes());
            objs.extend_from_slice(&(obj.superclasses.len() as u16).to_le_bytes());
            objs.extend_from_slice(&(obj.props.len() as u16).to_le_bytes());
            objs.extend_from_slice(&u16::from(obj.is_class).to_le_bytes());
            for superclass in &obj.superclasses {
                objs.extend_from_slice(&superclass.to_le_bytes());
            }
            for prop in &obj.props {
                objs.extend_from_slice(&prop.id.to_le_bytes());
                objs.push(prop.kind);
                let value = match &prop.target {
                    None => prop.value,
                    Some(Target::Const(anchor)) => const_addrs[*anchor] as u32,
                    Some(Target::Body(body)) => code_addrs[generated.body_anchor[body]] as u32,
                    Some(Target::Anchor(anchor)) => code_addrs[*anchor] as u32,
                    Some(Target::FuncBase(_)) => 0,
                    Some(Target::StaticBody(body)) => {
                        static_addrs[generated.static_body_anchor[body]] as u32
                    }
                    Some(Target::Func(func)) => {
                        match func_anchor_addr(&generated, &code_addrs, *func) {
                            Some(addr) => addr as u32,
                            None => {
                                return Err(format!(
                                    "function \"{}\" is declared but not defined",
                                    String::from_utf8_lossy(&prog.funcs[*func].name)
                                ));
                            }
                        }
                    }
                };
                if prop.kind == 6 {
                    // A property's number is two bytes of the four.
                    unspecified.push((writer.out.len() + 10 + objs.len() + 2, 2));
                }
                if prop.kind <= 2 {
                    // Nil and true have no value, and the reference
                    // leaves the room for one as its memory had it.
                    unspecified.push((writer.out.len() + 10 + objs.len(), 4));
                }
                objs.extend_from_slice(&value.to_le_bytes());
            }
        }
        writer.block(b"OBJS", true, &objs);
    }

    // The dictionaries, all in one block, which says its objects' sizes
    // are four bytes: each is its number, the size of what follows,
    // four bytes for the object that will compare words when the
    // program runs, which is nothing yet, the count of words, and the
    // words. A word is its length, its text with every byte's bits
    // changed about, the count of its uses, and each use as an object
    // and a property.
    if !generated.dicts.is_empty() {
        let mut objs = Vec::new();
        objs.extend_from_slice(&(generated.dicts.len() as u16).to_le_bytes());
        objs.extend_from_slice(&2u16.to_le_bytes());
        objs.extend_from_slice(&1u16.to_le_bytes());
        for (id, words) in &generated.dicts {
            let mut data = vec![0; 4];
            data.extend_from_slice(&(words.len() as u16).to_le_bytes());
            for (text, items) in words {
                let text = &text[..text.len().min(255)];
                data.push(text.len() as u8);
                data.extend(text.iter().map(|byte| byte ^ 0xBD));
                data.extend_from_slice(&(items.len() as u16).to_le_bytes());
                for (obj, prop) in items {
                    data.extend_from_slice(&obj.to_le_bytes());
                    data.extend_from_slice(&prop.to_le_bytes());
                }
            }
            objs.extend_from_slice(&id.to_le_bytes());
            objs.extend_from_slice(&(data.len() as u32).to_le_bytes());
            objs.extend_from_slice(&data);
        }
        writer.block(b"OBJS", true, &objs);
    }

    // The grammar productions, in a block like the dictionaries'.
    if !generated.gramprods.is_empty() {
        let mut objs = Vec::new();
        objs.extend_from_slice(&(generated.gramprods.len() as u16).to_le_bytes());
        objs.extend_from_slice(&3u16.to_le_bytes());
        objs.extend_from_slice(&1u16.to_le_bytes());
        for (id, data) in &generated.gramprods {
            objs.extend_from_slice(&id.to_le_bytes());
            objs.extend_from_slice(&(data.len() as u32).to_le_bytes());
            objs.extend_from_slice(data);
        }
        writer.block(b"OBJS", true, &objs);
    }

    // The objects that constants were made into: the numbers that are
    // not integers, and then the regular expressions. Each is its
    // number, the length of its data, and the data, and a block holds
    // no more than 64,000 bytes of them.
    let rexpats: Vec<(u32, Vec<u8>)> = generated
        .rexpats
        .iter()
        .map(|(id, target)| {
            let addr = match target {
                Target::Const(anchor) => const_addrs[*anchor] as u32,
                _ => 0,
            };
            // A string in the constants.
            let mut data = vec![8];
            data.extend_from_slice(&addr.to_le_bytes());
            (*id, data)
        })
        .collect();
    for (metaclass, items) in [(bignum_meta, &generated.bignums), (rexpat_meta, &rexpats)] {
        let mut rest = &items[..];
        while !rest.is_empty() {
            let mut size = 0;
            let mut count = 0;
            for (_, data) in rest {
                if count != 0 && size + 6 + data.len() > 64000 {
                    break;
                }
                size += 6 + data.len();
                count += 1;
            }
            let mut objs = Vec::new();
            objs.extend_from_slice(&(count as u16).to_le_bytes());
            objs.extend_from_slice(&metaclass.to_le_bytes());
            objs.extend_from_slice(&0u16.to_le_bytes());
            for (id, data) in &rest[..count] {
                objs.extend_from_slice(&id.to_le_bytes());
                objs.extend_from_slice(&(data.len() as u16).to_le_bytes());
                if metaclass == bignum_meta && !generated.bignums.is_empty() {
                    // A number's fifth byte has its sign and its kind
                    // in the low bits. The reference never sets the
                    // high ones, which are as its memory had them.
                    unspecified.push((writer.out.len() + 10 + objs.len() + 4, 1));
                }
                objs.extend_from_slice(data);
            }
            writer.block(b"OBJS", true, &objs);
            rest = &rest[count..];
        }
    }

    // The objects that stand for the built-in classes the program
    // declared: each says which class it is, and has room for an
    // object that adds to the class.
    let class_objs: Vec<(u32, usize)> = metas
        .iter()
        .enumerate()
        .filter_map(|(index, (_, _, obj))| obj.map(|obj| (obj, index)))
        .collect();
    if !class_objs.is_empty() {
        let mut objs = Vec::new();
        objs.extend_from_slice(&(class_objs.len() as u16).to_le_bytes());
        objs.extend_from_slice(&(intrinsic_class as u16).to_le_bytes());
        objs.extend_from_slice(&0u16.to_le_bytes());
        for (obj, meta) in class_objs {
            objs.extend_from_slice(&obj.to_le_bytes());
            objs.extend_from_slice(&8u16.to_le_bytes());
            objs.extend_from_slice(&8u16.to_le_bytes());
            objs.extend_from_slice(&(meta as u16).to_le_bytes());
            let mod_obj = generated.meta_mod_objs.get(meta).copied().unwrap_or(0);
            objs.extend_from_slice(&mod_obj.to_le_bytes());
        }
        writer.block(b"OBJS", true, &objs);
    }

    // Where the code that runs before the program starts is, and the
    // (empty) list of properties with values to work out then.
    let mut sini = Vec::new();
    sini.extend_from_slice(&12u32.to_le_bytes());
    sini.extend_from_slice(&((first_static_page * code_layout.page_size) as u32).to_le_bytes());
    sini.extend_from_slice(&(generated.static_inits.len() as u32).to_le_bytes());
    for (obj, prop) in &generated.static_inits {
        sini.extend_from_slice(&obj.to_le_bytes());
        sini.extend_from_slice(&prop.to_le_bytes());
    }
    writer.block(b"SINI", true, &sini);

    if let Some(debug) = debug {
        // The functions in the order the reference made them, each with
        // its address and its code: those a `replace` dropped are where
        // they were, at address zero.
        let mut methods: Vec<(usize, &[u8])> = Vec::new();
        let mut dropped = generated.dropped_code.iter().peekable();
        for (index, (anchor, &addr)) in generated.code_anchors.iter().zip(&code_addrs).enumerate() {
            while let Some((_, code)) = dropped.next_if(|(before, _)| *before <= index) {
                methods.push((0, code));
            }
            methods.push((addr, &generated.code[anchor.ofs..]));
        }
        methods.extend(dropped.map(|(_, code)| (0, code.as_slice())));
        let srcf = source_files_block(debug, &methods);
        writer.block(b"SRCF", false, &srcf);
        let func_addr = |func: usize| func_anchor_addr(&generated, &code_addrs, func);
        writer.block(
            b"GSYM",
            false,
            &global_symbols_block(prog, &debug.enum_ids, &func_addr),
        );
        // The address of each function in the program's code, not that
        // which runs before it starts.
        let mut mhls = Vec::new();
        mhls.extend_from_slice(&(methods.len() as u32).to_le_bytes());
        for &(addr, _) in &methods {
            mhls.extend_from_slice(&(addr as u32).to_le_bytes());
        }
        writer.block(b"MHLS", false, &mhls);
        let at = writer.out.len() + 10;
        unspecified.extend(
            debug
                .macros_unspecified
                .iter()
                .map(|&(ofs, len)| (at + ofs, len)),
        );
        writer.block(b"MACR", false, &debug.macros);
    }

    writer.block(b"EOF ", true, &[]);
    let func_addrs = generated
        .func_anchor
        .iter()
        .map(|(&func, &anchor)| (func, code_addrs[anchor] as u32))
        .collect();
    Ok(Image {
        bytes: writer.out,
        unspecified,
        func_addrs,
    })
}

/// The source files, each with the address of the code of each of its
/// lines that has any. The lines are read from the functions' tables,
/// in the order of the functions; a line's first address is the one
/// kept. A file that an earlier module read too has its lines with
/// that one, and says which it is.
fn source_files_block(debug: &DebugRecords, methods: &[(usize, &[u8])]) -> Vec<u8> {
    let orig: Vec<usize> = (0..debug.files.len())
        .map(|index| {
            debug.files[..index]
                .iter()
                .position(|name| *name == debug.files[index])
                .unwrap_or(index)
        })
        .collect();
    let mut lines: Vec<std::collections::BTreeMap<u32, u32>> =
        vec![std::collections::BTreeMap::new(); debug.files.len()];
    for &(addr, code) in methods {
        let u16_at = |at: usize| usize::from(u16::from_le_bytes([code[at], code[at + 1]]));
        let table = u16_at(8);
        if table == 0 {
            continue;
        }
        for rec in 0..u16_at(table) {
            let at = table + 2 + 10 * rec;
            let ofs = u16_at(at);
            let file = u16_at(at + 2);
            let line = u32::from_le_bytes(code[at + 4..at + 8].try_into().expect("four bytes"));
            let Some(&file) = orig.get(file) else {
                continue;
            };
            // An address of zero is no address, to the reference.
            let entry = lines[file].entry(line).or_insert(0);
            if *entry == 0 {
                *entry = (addr + ofs) as u32;
            }
        }
    }
    let mut block = Vec::new();
    block.extend_from_slice(&(debug.files.len() as u16).to_le_bytes());
    block.extend_from_slice(&8u16.to_le_bytes());
    for (index, name) in debug.files.iter().enumerate() {
        let start = block.len();
        block.extend_from_slice(&0u32.to_le_bytes());
        block.extend_from_slice(&(orig[index] as u16).to_le_bytes());
        block.extend_from_slice(&(name.len() as u16).to_le_bytes());
        block.extend_from_slice(name.as_bytes());
        let lines: Vec<(u32, u32)> = lines[index]
            .iter()
            .filter(|(_, addr)| **addr != 0)
            .map(|(&line, &addr)| (line, addr))
            .collect();
        block.extend_from_slice(&(lines.len() as u32).to_le_bytes());
        for (line, addr) in lines {
            block.extend_from_slice(&line.to_le_bytes());
            block.extend_from_slice(&addr.to_le_bytes());
        }
        let size = (block.len() - start) as u32;
        block[start..start + 4].copy_from_slice(&size.to_le_bytes());
    }
    block
}

/// The program's global names, in the order of the reference's table,
/// each with its kind and what the debugger needs to know of it. Names
/// that start with a period are the compiler's own, and left out.
fn global_symbols_block(
    prog: &Program,
    enum_ids: &[u32],
    func_addr: &dyn Fn(usize) -> Option<usize>,
) -> Vec<u8> {
    // Each definition a `modify` set aside knows the object that set it
    // aside: the reference's symbol for it, which a `replace` after it
    // takes over, so that it is the number of the definition that
    // replaced it if one did.
    let def_id = |def: &crate::parse::ObjDef| def.mod_id.or(def.sym.map(|sym| prog.objs[sym].id));
    let mut modifying: std::collections::HashMap<u32, u32> = std::collections::HashMap::new();
    for (index, def) in prog.objdefs.iter().enumerate() {
        let Some(base) = def.mod_base else {
            continue;
        };
        let mut last = def;
        let mut later = prog.objdefs[index + 1..]
            .iter()
            .filter(|other| other.sym == def.sym);
        while last.replaced {
            match later.next() {
                Some(next) => last = next,
                None => break,
            }
        }
        if let Some(id) = def_id(last) {
            modifying.insert(base, id);
        }
    }
    // Each object that holds what a `modify` adds to a built-in class
    // knows the one made for the next, in whichever file it is. (Each
    // file's chain starts with an empty object of its own, which has no
    // name.)
    let mut last_mod: std::collections::HashMap<usize, u32> = std::collections::HashMap::new();
    for def in &prog.objdefs {
        if let (Some(meta), Some(id), Some(_)) = (def.meta, def.mod_id, def.mod_base) {
            if let Some(prev) = last_mod.insert(meta, id) {
                modifying.insert(prev, id);
            }
        }
    }
    let mut count = 0u32;
    let mut entries = Vec::new();
    for (name, global) in prog.globals.in_table_order() {
        if name.first() == Some(&b'.') {
            continue;
        }
        let mut data = Vec::new();
        let kind: u16 = match global {
            Global::Func(func) => {
                let f = &prog.funcs[func];
                let addr = func_addr(func).unwrap_or(0) as u32;
                data.extend_from_slice(&addr.to_le_bytes());
                data.extend_from_slice(&(f.argc as u16).to_le_bytes());
                data.push(u8::from(f.varargs));
                data.push(u8::from(f.has_retval));
                data.extend_from_slice(&(f.opt_argc as u16).to_le_bytes());
                1
            }
            Global::Obj(obj) => {
                let id = prog.objs[obj].id;
                data.extend_from_slice(&id.to_le_bytes());
                data.extend_from_slice(&modifying.get(&id).copied().unwrap_or(0).to_le_bytes());
                2
            }
            Global::Prop(prop) => {
                data.extend_from_slice(&prog.props[prop].id.to_le_bytes());
                // Whether it is a dictionary property, which the
                // reference does not keep in its object files, so that
                // after linking no property is one.
                data.push(0);
                3
            }
            Global::Bif(bif) => {
                let b = &prog.bifs[bif];
                data.extend_from_slice(&(b.index as u16).to_le_bytes());
                data.extend_from_slice(&(b.set as u16).to_le_bytes());
                // A built-in function is taken to return a value.
                data.push(1);
                data.extend_from_slice(&(b.min_argc as u16).to_le_bytes());
                data.extend_from_slice(&(b.max_argc as u16).to_le_bytes());
                data.push(u8::from(b.varargs));
                6
            }
            Global::Meta(meta) => {
                let m = &prog.metas[meta];
                data.extend_from_slice(&(m.meta as u16).to_le_bytes());
                data.extend_from_slice(&m.obj_id.to_le_bytes());
                9
            }
            Global::Enum(sym) => {
                data.extend_from_slice(&enum_ids.get(sym).copied().unwrap_or(0).to_le_bytes());
                data.push(u8::from(prog.enums[sym].is_token));
                10
            }
            // Only code compiled while the program runs has these.
            Global::DynLocal(_) => continue,
        };
        entries.extend_from_slice(&(name.len() as u16).to_le_bytes());
        entries.extend_from_slice(&(data.len() as u16).to_le_bytes());
        entries.extend_from_slice(&kind.to_le_bytes());
        entries.extend_from_slice(name);
        entries.extend_from_slice(&data);
        count += 1;
    }
    let mut block = count.to_le_bytes().to_vec();
    block.extend_from_slice(&entries);
    block
}

/// Sort as the C library's `qsort` that the reference is built with
/// (Microsoft's) does. It is not stable, and an object can have two
/// entries for one property, when a vocabulary property is defined before
/// a nested object and so gets a placeholder as well: the order the two
/// end up in is the order this leaves them in. Up to eight items are put
/// in order by finding the greatest, the first of equals, and swapping it
/// to the end; more are split around the middle of three.
pub(crate) fn crt_qsort<T>(items: &mut [T], cmp: impl Fn(&T, &T) -> std::cmp::Ordering) {
    use std::cmp::Ordering::{Equal, Greater};
    const CUTOFF: usize = 8;
    if items.len() < 2 {
        return;
    }
    let short_sort = |items: &mut [T], lo: usize, mut hi: usize| {
        while hi > lo {
            let mut max = lo;
            for p in lo + 1..=hi {
                if cmp(&items[p], &items[max]) == Greater {
                    max = p;
                }
            }
            items.swap(max, hi);
            hi -= 1;
        }
    };
    let mut stack = Vec::new();
    let (mut lo, mut hi) = (0usize, items.len() - 1);
    loop {
        let size = hi - lo + 1;
        if size <= CUTOFF {
            short_sort(items, lo, hi);
        } else {
            let mut mid = lo + size / 2;
            if cmp(&items[lo], &items[mid]) == Greater {
                items.swap(lo, mid);
            }
            if cmp(&items[lo], &items[hi]) == Greater {
                items.swap(lo, hi);
            }
            if cmp(&items[mid], &items[hi]) == Greater {
                items.swap(mid, hi);
            }
            // Positions run from lo - 1 to hi + 1, as the C pointers do.
            let mut loguy = lo as isize;
            let mut higuy = hi as isize + 1;
            let at = |i: isize| i as usize;
            loop {
                if (mid as isize) > loguy {
                    loop {
                        loguy += 1;
                        if !(loguy < mid as isize && cmp(&items[at(loguy)], &items[mid]) != Greater)
                        {
                            break;
                        }
                    }
                }
                if (mid as isize) <= loguy {
                    loop {
                        loguy += 1;
                        if !(loguy <= hi as isize && cmp(&items[at(loguy)], &items[mid]) != Greater)
                        {
                            break;
                        }
                    }
                }
                loop {
                    higuy -= 1;
                    if !(higuy > mid as isize && cmp(&items[at(higuy)], &items[mid]) == Greater) {
                        break;
                    }
                }
                if higuy < loguy {
                    break;
                }
                items.swap(at(loguy), at(higuy));
                if mid as isize == higuy {
                    mid = at(loguy);
                }
            }
            higuy += 1;
            if (mid as isize) < higuy {
                loop {
                    higuy -= 1;
                    if !(higuy > mid as isize && cmp(&items[at(higuy)], &items[mid]) == Equal) {
                        break;
                    }
                }
            }
            if (mid as isize) >= higuy {
                loop {
                    higuy -= 1;
                    if !(higuy > lo as isize && cmp(&items[at(higuy)], &items[mid]) == Equal) {
                        break;
                    }
                }
            }
            // The smaller part waits; the larger is sorted now.
            if higuy - lo as isize >= hi as isize - loguy {
                if (lo as isize) < higuy {
                    stack.push((lo, at(higuy)));
                }
                if loguy < hi as isize {
                    lo = at(loguy);
                    continue;
                }
            } else {
                if loguy < hi as isize {
                    stack.push((at(loguy), hi));
                }
                if (lo as isize) < higuy {
                    hi = at(higuy);
                    continue;
                }
            }
        }
        match stack.pop() {
            Some((l, h)) => (lo, hi) = (l, h),
            None => return,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::crt_qsort;

    #[test]
    fn the_first_of_equal_greatest_items_goes_last() {
        let mut items = [(2, 'a'), (1, 'b'), (2, 'c')];
        crt_qsort(&mut items, |x, y| x.0.cmp(&y.0));
        assert_eq!(items, [(1, 'b'), (2, 'c'), (2, 'a')]);
    }

    #[test]
    fn long_tables_are_sorted() {
        let mut seed = 12345u32;
        for len in 0..200 {
            let mut items: Vec<u32> = (0..len)
                .map(|_| {
                    seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12345);
                    (seed >> 16) % 50
                })
                .collect();
            let mut expected = items.clone();
            expected.sort_unstable();
            crt_qsort(&mut items, |a, b| a.cmp(b));
            assert_eq!(items, expected, "length {len}");
        }
    }
}
