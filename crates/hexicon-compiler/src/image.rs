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

/// The metaclasses every image depends on, whether it uses them or not.
const METACLASSES: [&str; 9] = [
    "tads-object/000000",
    "list/000000",
    "dictionary2/030000",
    "grammar-production/030000",
    "vector/000000",
    "anon-func-ptr/000000",
    "int-class-mod/030000",
    "lookuptable/030003",
    "intrinsic-class/000000",
];

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
}

/// Write the image. `timestamp` is the 24 characters that record when
/// the program was compiled, and `tool_data` four bytes for the
/// compiler's own use.
pub(crate) fn write_image(
    prog: &Program,
    mut generated: Generated,
    timestamp: &[u8; 24],
    tool_data: &[u8; 4],
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
    let static_anchors = [Anchor { ofs: 0 }];
    let static_addrs = code_layout.place(&static_anchors, STATIC_INIT_STUB.len());
    code_layout.finish();

    // Fill in the addresses, in the code and in the constants.
    let Generated {
        code,
        consts,
        fixups,
        const_fixups,
        func_anchor,
        body_anchor,
        ..
    } = &mut generated;
    for (stream, fixups) in [(code, &*fixups), (consts, &*const_fixups)] {
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
    writer.out.extend_from_slice(b"T3-image\r\n\x1a");
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
    symd.extend_from_slice(&((EXPORTED_PROPS.len() + 1) as u16).to_le_bytes());
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

    let mut mcld = Vec::new();
    mcld.extend_from_slice(&(METACLASSES.len() as u16).to_le_bytes());
    for name in METACLASSES {
        // Each entry: its own size, the name, and an empty list of
        // properties whose entries would be two bytes each.
        mcld.extend_from_slice(&((2 + 1 + name.len() + 2 + 2) as u16).to_le_bytes());
        mcld.push(name.len() as u8);
        mcld.extend_from_slice(name.as_bytes());
        mcld.extend_from_slice(&0u16.to_le_bytes());
        mcld.extend_from_slice(&2u16.to_le_bytes());
    }
    writer.block(b"MCLD", true, &mcld);

    let mut code_items = items(&generated.code, &generated.code_anchors, &code_addrs);
    code_items.push((static_addrs[0], &STATIC_INIT_STUB));
    writer.pool(1, &code_layout, &code_items, 0);
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

    // The objects, all in one block. (The reference means to start a
    // new block when one is full, but never counts what it has put in.)
    if !generated.objs.is_empty() {
        let mut objs = Vec::new();
        objs.extend_from_slice(&(generated.objs.len() as u16).to_le_bytes());
        // They are of the first metaclass, and the block has no flags.
        objs.extend_from_slice(&[0; 4]);
        let mut gen_objs = std::mem::take(&mut generated.objs);
        for obj in &mut gen_objs {
            obj.props.sort_by_key(|prop| prop.id);
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

    // Where the code that runs before the program starts is, and the
    // (empty) list of properties with values to work out then.
    let mut sini = Vec::new();
    sini.extend_from_slice(&12u32.to_le_bytes());
    sini.extend_from_slice(&((first_static_page * code_layout.page_size) as u32).to_le_bytes());
    sini.extend_from_slice(&0u32.to_le_bytes());
    writer.block(b"SINI", true, &sini);

    writer.block(b"EOF ", true, &[]);
    Ok(Image {
        bytes: writer.out,
        unspecified,
    })
}
