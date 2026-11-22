//! Reader for T3 VM program image files (`.t3`).
//!
//! An image file holds a compiled program: its byte code, constant data,
//! static objects, and the lists of intrinsic classes and functions it
//! depends on. This crate parses a file into an [`Image`] and checks its
//! structure. It does not interpret object data or byte code.
//!
//! The format is described in the T3 VM technical documentation, "T3 VM
//! Image File Format". Where that document and the reference interpreter
//! disagree, this crate follows the interpreter.

mod error;
mod reader;

pub use error::{BlockKind, ImageError};
use reader::Reader;

/// The bytes every image file starts with.
pub const SIGNATURE: &[u8; 11] = b"T3-image\r\n\x1a";

/// The newest image format version this crate reads.
pub const MAX_FORMAT_VERSION: u16 = 2;

const HEADER_SIZE: usize = 69;
const BLOCK_HEADER_SIZE: usize = 10;
const FLAG_MANDATORY: u16 = 0x0001;

/// Pool identifier of the byte-code pool.
pub const POOL_CODE: u16 = 1;
/// Pool identifier of the constant data pool.
pub const POOL_CONSTANT: u16 = 2;

/// A parsed program image.
#[derive(Debug, Clone)]
pub struct Image {
    /// The file header.
    pub header: Header,
    /// Every block in the file, in file order, including the end marker.
    pub blocks: Vec<BlockInfo>,
    /// The program entry point and global structure sizes.
    pub entry_point: EntryPoint,
    /// The byte-code pool.
    pub code_pool: Pool,
    /// The constant data pool.
    pub constant_pool: Pool,
    /// Intrinsic classes the program depends on, indexed as the program refers to them.
    pub metaclasses: Vec<MetaclassDependency>,
    /// Intrinsic function sets the program depends on, indexed as the program refers to them.
    pub function_sets: Vec<String>,
    /// Static object blocks, in file order.
    pub object_blocks: Vec<ObjectBlock>,
    /// Named values the program exports to the VM.
    pub symbols: Vec<Symbol>,
    /// Static initializers, if the image has not been through pre-initialization.
    pub static_initializers: Option<StaticInitializers>,
    /// Resources embedded in the image.
    pub resources: Vec<Resource>,
    /// Links to resources kept in local files (debug builds only).
    pub resource_links: Vec<ResourceLink>,
    /// Information present only in images compiled for debugging.
    pub debug: DebugInfo,
}

/// The fixed header at the start of an image file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Header {
    /// Image format version.
    pub version: u16,
    /// Four bytes reserved for build tools; the reference compiler stores a
    /// hash of the build configuration here.
    pub build_configuration: [u8; 4],
    /// Creation time, in the form `Sun Aug 01 17:05:20 1999`. Saved games
    /// record this to check they are restored into the same image.
    pub timestamp: String,
}

/// The location and header of one block in the file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BlockInfo {
    /// The block type.
    pub kind: BlockKind,
    /// File offset of the block's header.
    pub offset: usize,
    /// Size of the block's data, not counting the header.
    pub size: u32,
    /// The block's flags.
    pub flags: u16,
}

impl BlockInfo {
    /// Whether a reader that does not know this block type must reject the file.
    pub fn is_mandatory(&self) -> bool {
        self.flags & FLAG_MANDATORY != 0
    }
}

/// The program entry point and the sizes of structures that are uniform
/// across the image.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EntryPoint {
    /// Code pool offset of the function to run after loading.
    pub code_offset: u32,
    /// Size of every method header.
    pub method_header_size: u16,
    /// Size of every exception table entry.
    pub exception_entry_size: u16,
    /// Size of every debugger line table entry.
    pub debug_line_entry_size: u16,
    /// Size of every debug table header.
    pub debug_table_header_size: u16,
    /// Size of every local symbol record header in debug tables.
    pub debug_local_header_size: u16,
    /// Version of the debug record format.
    pub debug_version: u16,
    /// Size of every debug frame header. Images older than format version 2
    /// do not store this; it is then 4.
    pub debug_frame_header_size: u16,
}

/// One of the two paged pools of read-only program data.
#[derive(Debug, Clone, Default)]
pub struct Pool {
    page_size: u32,
    pages: Vec<Option<Box<[u8]>>>,
}

impl Pool {
    /// Size of each page in bytes.
    pub fn page_size(&self) -> u32 {
        self.page_size
    }

    /// Number of pages in the pool, whether or not the image supplies them.
    pub fn page_count(&self) -> usize {
        self.pages.len()
    }

    /// Number of pages the image supplies data for.
    pub fn loaded_page_count(&self) -> usize {
        self.pages.iter().filter(|p| p.is_some()).count()
    }

    /// Total bytes of page data in the pool.
    pub fn data_len(&self) -> usize {
        self.pages.iter().flatten().map(|p| p.len()).sum()
    }

    /// The data of one page. A page may be shorter than the page size.
    pub fn page(&self, index: usize) -> Option<&[u8]> {
        self.pages.get(index)?.as_deref()
    }

    /// The data from a pool address to the end of its page. Items in a pool
    /// never span pages, so this always covers a whole item.
    pub fn at(&self, address: u32) -> Option<&[u8]> {
        if self.page_size == 0 {
            return None;
        }
        let page = self.page((address / self.page_size) as usize)?;
        page.get((address % self.page_size) as usize..)
    }
}

/// An intrinsic class the program depends on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MetaclassDependency {
    /// The class identifier with its version, such as `tads-object/030005`.
    pub name: String,
    /// The property the program uses for each of the class's methods, in
    /// the order the class defines them.
    pub properties: Vec<u16>,
}

impl MetaclassDependency {
    /// The identifier without its version suffix.
    pub fn base_name(&self) -> &str {
        self.name
            .split_once('/')
            .map_or(&self.name, |(base, _)| base)
    }

    /// The version suffix, if the identifier has one.
    pub fn version(&self) -> Option<&str> {
        self.name.split_once('/').map(|(_, version)| version)
    }
}

/// A block of static objects that share one intrinsic class.
#[derive(Debug, Clone)]
pub struct ObjectBlock {
    /// Index of the objects' class in [`Image::metaclasses`].
    pub metaclass_index: u16,
    /// Whether the objects are transient, that is, left out of saved games.
    pub transient: bool,
    /// The objects defined in the block.
    pub objects: Vec<StaticObject>,
}

/// A static object: an ID and data for its intrinsic class to interpret.
#[derive(Debug, Clone)]
pub struct StaticObject {
    /// The object ID assigned by the compiler.
    pub id: u32,
    /// The class-specific data.
    pub data: Box<[u8]>,
}

/// A value stored in the portable five-byte "data holder" form.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Value {
    /// `nil`.
    Nil,
    /// `true`.
    True,
    /// An object reference.
    Object(u32),
    /// A property ID.
    Property(u16),
    /// A 32-bit integer.
    Int(i32),
    /// A constant pool offset of a string.
    String(u32),
    /// A constant pool offset of a self-printing string.
    SelfPrintingString(u32),
    /// A constant pool offset of a list.
    List(u32),
    /// A code pool offset.
    CodeOffset(u32),
    /// A code pool offset of a function.
    FunctionPointer(u32),
    /// The "empty" marker.
    Empty,
    /// An enumerator.
    Enum(u32),
    /// An intrinsic function.
    IntrinsicFunction {
        /// Index into [`Image::function_sets`].
        set: u16,
        /// Index of the function within its set.
        index: u16,
    },
    /// A type this crate does not interpret.
    Other {
        /// The type code.
        type_code: u8,
        /// The four value bytes.
        raw: [u8; 4],
    },
}

impl Value {
    /// Size of a data holder in bytes.
    pub const ENCODED_SIZE: usize = 5;

    /// Decode a data holder.
    pub fn decode(bytes: [u8; 5]) -> Self {
        let raw = [bytes[1], bytes[2], bytes[3], bytes[4]];
        let word = u32::from_le_bytes(raw);
        let low = u16::from_le_bytes([raw[0], raw[1]]);
        let high = u16::from_le_bytes([raw[2], raw[3]]);
        match bytes[0] {
            1 => Self::Nil,
            2 => Self::True,
            5 | 17 => Self::Object(word),
            6 => Self::Property(low),
            7 => Self::Int(word as i32),
            8 => Self::String(word),
            9 => Self::SelfPrintingString(word),
            10 => Self::List(word),
            11 => Self::CodeOffset(word),
            12 => Self::FunctionPointer(word),
            13 => Self::Empty,
            15 => Self::Enum(word),
            16 | 18 => Self::IntrinsicFunction {
                set: high,
                index: low,
            },
            type_code => Self::Other { type_code, raw },
        }
    }
}

/// A named value the program exports to the VM, such as the property to
/// call for an object's constructor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Symbol {
    /// The symbol name.
    pub name: String,
    /// The symbol's value.
    pub value: Value,
}

/// Property evaluations to run once before the program starts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StaticInitializers {
    /// Code at or above this code pool offset is used only by static
    /// initializers and can be discarded afterward.
    pub static_code_offset: u32,
    /// The initializers, in the order they must run.
    pub initializers: Vec<StaticInitializer>,
}

/// One static initializer: evaluate a property of an object.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StaticInitializer {
    /// The object.
    pub object: u32,
    /// The property to evaluate.
    pub property: u16,
}

/// A resource embedded in the image, such as a picture or sound.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resource {
    /// The resource name.
    pub name: String,
    /// File offset of the resource's data.
    pub offset: usize,
    /// Size of the resource's data in bytes.
    pub size: u32,
}

/// A resource name mapped to a local file that holds its data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceLink {
    /// The resource name.
    pub name: String,
    /// The local file name.
    pub file_name: String,
}

/// Information compilers include only in debug builds.
#[derive(Debug, Clone, Default)]
pub struct DebugInfo {
    /// Source files the program was compiled from.
    pub source_files: Vec<SourceFile>,
    /// Top-level symbols of the program.
    pub global_symbols: Vec<GlobalSymbol>,
    /// Code pool addresses of every method header, in ascending order.
    pub method_headers: Vec<u32>,
    /// Preprocessor macros defined in the program.
    pub macros: Vec<Macro>,
}

/// A source file the program was compiled from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceFile {
    /// Index of the first record for the same file; a file can be listed
    /// more than once.
    pub master_index: u16,
    /// The file name as the compiler recorded it.
    pub name: String,
    /// Source lines and the code compiled from them, in ascending line order.
    pub lines: Vec<SourceLine>,
}

/// The code pool address of the code compiled from one source line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SourceLine {
    /// Line number, starting at 1.
    pub line: u32,
    /// Code pool address.
    pub code_address: u32,
}

/// A top-level symbol of the program.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GlobalSymbol {
    /// The symbol name.
    pub name: String,
    /// What the symbol names.
    pub kind: GlobalSymbolKind,
}

/// What a global symbol names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GlobalSymbolKind {
    /// A function.
    Function {
        /// Code pool offset of the function.
        code_offset: u32,
        /// Number of required parameters.
        argc: u16,
        /// Whether the function accepts any number of further arguments.
        varargs: bool,
        /// Whether the function returns a value.
        has_return: bool,
        /// Number of optional parameters after the required ones.
        optional_argc: u16,
    },
    /// An object.
    Object {
        /// The object ID.
        id: u32,
        /// The object that modifies this one, or 0.
        modifier: u32,
    },
    /// A property.
    Property {
        /// The property ID.
        id: u16,
        /// Whether the property is a dictionary property.
        dictionary: bool,
    },
    /// An intrinsic function.
    IntrinsicFunction {
        /// Index of the function within its set.
        index: u16,
        /// Index into [`Image::function_sets`].
        set: u16,
        /// Whether the function returns a value.
        has_return: bool,
        /// Fewest arguments accepted.
        min_argc: u16,
        /// Most arguments accepted, when not `varargs`.
        max_argc: u16,
        /// Whether the function accepts an unlimited number of arguments.
        varargs: bool,
    },
    /// An intrinsic class.
    IntrinsicClass {
        /// Index into [`Image::metaclasses`].
        metaclass_index: u16,
        /// The object that represents the class.
        class_object: u32,
    },
    /// An enumerator.
    Enumerator {
        /// The enumerator's internal ID.
        id: u32,
        /// Whether the enumerator is an "enum token".
        token: bool,
    },
    /// A symbol type this crate does not interpret.
    Other {
        /// The type code.
        type_code: u16,
        /// The type-specific data.
        data: Box<[u8]>,
    },
}

/// A preprocessor macro.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Macro {
    /// The macro name.
    pub name: String,
    /// Whether the macro takes arguments.
    pub function_like: bool,
    /// Whether the last parameter stands for any number of arguments.
    pub varargs: bool,
    /// The parameter names.
    pub parameters: Vec<String>,
    /// The expansion text.
    pub expansion: String,
}

impl Image {
    /// Parse an image file.
    ///
    /// `data` may have other content after the image's end-of-file block,
    /// as happens when an image is embedded in a larger file.
    pub fn parse(data: &[u8]) -> Result<Self, ImageError> {
        Loader::default().load(data)
    }

    /// The class of the objects in a static object block.
    pub fn metaclass_of(&self, block: &ObjectBlock) -> &MetaclassDependency {
        // The index was checked when the block was read.
        &self.metaclasses[block.metaclass_index as usize]
    }

    /// Total number of static objects.
    pub fn object_count(&self) -> usize {
        self.object_blocks.iter().map(|b| b.objects.len()).sum()
    }

    /// Look up an exported symbol by name.
    pub fn symbol(&self, name: &str) -> Option<Value> {
        self.symbols
            .iter()
            .find(|s| s.name == name)
            .map(|s| s.value)
    }
}

/// Read only the header of an image file.
pub fn parse_header(data: &[u8]) -> Result<Header, ImageError> {
    if data.len() < SIGNATURE.len() || &data[..SIGNATURE.len()] != SIGNATURE {
        return Err(ImageError::NotAnImage);
    }
    if data.len() < HEADER_SIZE {
        return Err(ImageError::TruncatedFile { offset: data.len() });
    }
    let version = u16::from_le_bytes([data[11], data[12]]);
    if version > MAX_FORMAT_VERSION {
        return Err(ImageError::UnsupportedVersion(version));
    }
    Ok(Header {
        version,
        build_configuration: [data[41], data[42], data[43], data[44]],
        timestamp: String::from_utf8_lossy(&data[45..69]).into_owned(),
    })
}

#[derive(Default)]
struct Loader {
    blocks: Vec<BlockInfo>,
    entry_point: Option<EntryPoint>,
    code_pool: Option<Pool>,
    constant_pool: Option<Pool>,
    metaclasses: Option<Vec<MetaclassDependency>>,
    function_sets: Option<Vec<String>>,
    object_blocks: Vec<ObjectBlock>,
    symbols: Vec<Symbol>,
    static_initializers: Option<StaticInitializers>,
    resources: Vec<Resource>,
    resource_links: Vec<ResourceLink>,
    debug: DebugInfo,
}

impl Loader {
    fn load(mut self, data: &[u8]) -> Result<Image, ImageError> {
        let header = parse_header(data)?;

        let mut offset = HEADER_SIZE;
        loop {
            if offset == data.len() {
                return Err(ImageError::MissingEof);
            }
            let Some(head) = data.get(offset..offset + BLOCK_HEADER_SIZE) else {
                return Err(ImageError::TruncatedFile { offset: data.len() });
            };
            let info = BlockInfo {
                kind: BlockKind([head[0], head[1], head[2], head[3]]),
                offset,
                size: u32::from_le_bytes([head[4], head[5], head[6], head[7]]),
                flags: u16::from_le_bytes([head[8], head[9]]),
            };
            self.blocks.push(info);
            if info.kind == BlockKind::EOF {
                break;
            }

            let start = offset + BLOCK_HEADER_SIZE;
            let Some(body) = start
                .checked_add(info.size as usize)
                .and_then(|end| data.get(start..end))
            else {
                return Err(ImageError::TruncatedFile { offset: data.len() });
            };
            self.block(&info, Reader::new(body, info.kind, offset), start)?;
            offset = start + body.len();
        }

        Ok(Image {
            header,
            blocks: self.blocks,
            entry_point: self
                .entry_point
                .ok_or(ImageError::MissingBlock(BlockKind::ENTP))?,
            code_pool: self.code_pool.unwrap_or_default(),
            constant_pool: self.constant_pool.unwrap_or_default(),
            metaclasses: self
                .metaclasses
                .ok_or(ImageError::MissingBlock(BlockKind::MCLD))?,
            function_sets: self
                .function_sets
                .ok_or(ImageError::MissingBlock(BlockKind::FNSD))?,
            object_blocks: self.object_blocks,
            symbols: self.symbols,
            static_initializers: self.static_initializers,
            resources: self.resources,
            resource_links: self.resource_links,
            debug: self.debug,
        })
    }

    /// Read one block. `data_offset` is the file offset of the block's data.
    fn block(
        &mut self,
        info: &BlockInfo,
        r: Reader<'_>,
        data_offset: usize,
    ) -> Result<(), ImageError> {
        match info.kind {
            BlockKind::ENTP => self.entry_point(r),
            BlockKind::CPDF => self.pool_definition(r),
            BlockKind::CPPG => self.pool_page(r),
            BlockKind::MCLD => self.metaclass_dependencies(r),
            BlockKind::FNSD => self.function_set_dependencies(r),
            BlockKind::OBJS => self.static_objects(r),
            BlockKind::SYMD => self.symbolic_names(r),
            BlockKind::SINI => self.static_initializers(r),
            BlockKind::MRES => self.resources(r, data_offset),
            BlockKind::MREL => self.resource_links(r),
            BlockKind::SRCF => self.source_files(r),
            BlockKind::GSYM => self.global_symbols(r),
            BlockKind::MHLS => self.method_headers(r),
            BlockKind::MACR => self.macros(r),
            kind if info.is_mandatory() => Err(ImageError::UnknownMandatoryBlock {
                kind,
                block_offset: info.offset,
            }),
            _ => Ok(()),
        }
    }

    fn entry_point(&mut self, mut r: Reader<'_>) -> Result<(), ImageError> {
        if self.entry_point.is_some() {
            return Err(ImageError::DuplicateBlock(BlockKind::ENTP));
        }
        self.entry_point = Some(EntryPoint {
            code_offset: r.u32()?,
            method_header_size: r.u16()?,
            exception_entry_size: r.u16()?,
            debug_line_entry_size: r.u16()?,
            debug_table_header_size: r.u16()?,
            debug_local_header_size: r.u16()?,
            debug_version: r.u16()?,
            debug_frame_header_size: if r.remaining() >= 2 { r.u16()? } else { 4 },
        });
        Ok(())
    }

    fn pool_slot(&mut self, id: u16) -> Result<&mut Option<Pool>, ImageError> {
        match id {
            POOL_CODE => Ok(&mut self.code_pool),
            POOL_CONSTANT => Ok(&mut self.constant_pool),
            _ => Err(ImageError::UnknownPool(id)),
        }
    }

    fn pool_definition(&mut self, mut r: Reader<'_>) -> Result<(), ImageError> {
        let id = r.u16()?;
        let page_count = r.u32()?;
        let page_size = r.u32()?;
        let slot = self.pool_slot(id)?;
        if slot.is_some() {
            return Err(ImageError::DuplicatePool(id));
        }
        // Pages are filled in as their blocks arrive, but each one the
        // definition claims takes a slot here. A program has thousands at
        // most; a million pages of the usual size would be 2 GB.
        if page_count > 1 << 20 {
            return Err(r.malformed("unreasonable page count"));
        }
        *slot = Some(Pool {
            page_size,
            pages: vec![None; page_count as usize],
        });
        Ok(())
    }

    fn pool_page(&mut self, mut r: Reader<'_>) -> Result<(), ImageError> {
        let id = r.u16()?;
        let index = r.u32()?;
        let mask = r.u8()?;
        let data = r.rest();
        let pool = self
            .pool_slot(id)?
            .as_mut()
            .ok_or(ImageError::PageBeforePoolDefinition(id))?;
        let slot = pool
            .pages
            .get_mut(index as usize)
            .ok_or(ImageError::PageIndexOutOfRange { pool: id, index })?;
        if slot.is_some() {
            return Err(ImageError::DuplicatePage { pool: id, index });
        }
        if data.len() > pool.page_size as usize {
            return Err(ImageError::PageTooLarge { pool: id, index });
        }
        *slot = Some(data.iter().map(|b| b ^ mask).collect());
        Ok(())
    }

    fn metaclass_dependencies(&mut self, mut r: Reader<'_>) -> Result<(), ImageError> {
        if self.metaclasses.is_some() {
            return Err(ImageError::DuplicateBlock(BlockKind::MCLD));
        }
        let count = r.u16()?;
        let mut list = Vec::with_capacity(count as usize);
        for _ in 0..count {
            let entry_start = r.position();
            let next_offset = r.u16()? as usize;
            let name_len = r.u8()? as usize;
            let name = r.string(name_len)?;
            let property_count = r.u16()?;
            let record_size = r.u16()? as usize;
            if record_size < 2 {
                return Err(r.malformed("metaclass property record is too small"));
            }
            let mut properties = Vec::with_capacity(property_count as usize);
            for _ in 0..property_count {
                let record = r.bytes(record_size)?;
                properties.push(u16::from_le_bytes([record[0], record[1]]));
            }
            // Each entry gives the offset of the next, so entries can grow
            // new fields that older readers skip.
            if entry_start + next_offset < r.position() {
                return Err(r.malformed("metaclass entry is larger than its stated size"));
            }
            r.seek(entry_start + next_offset)?;
            list.push(MetaclassDependency { name, properties });
        }
        self.metaclasses = Some(list);
        Ok(())
    }

    fn function_set_dependencies(&mut self, mut r: Reader<'_>) -> Result<(), ImageError> {
        if self.function_sets.is_some() {
            return Err(ImageError::DuplicateBlock(BlockKind::FNSD));
        }
        let count = r.u16()?;
        let mut list = Vec::with_capacity(count as usize);
        for _ in 0..count {
            let len = r.u8()? as usize;
            list.push(r.string(len)?);
        }
        self.function_sets = Some(list);
        Ok(())
    }

    fn static_objects(&mut self, mut r: Reader<'_>) -> Result<(), ImageError> {
        let count = r.u16()?;
        let metaclass_index = r.u16()?;
        let flags = r.u16()?;
        let large = flags & 0x0001 != 0;
        let transient = flags & 0x0002 != 0;

        let metaclasses = self
            .metaclasses
            .as_ref()
            .ok_or(ImageError::ObjectsBeforeMetaclasses)?;
        if metaclass_index as usize >= metaclasses.len() {
            return Err(ImageError::MetaclassIndexOutOfRange(metaclass_index));
        }

        let mut objects = Vec::with_capacity(count as usize);
        for _ in 0..count {
            let id = r.u32()?;
            let size = if large {
                r.u32()? as usize
            } else {
                r.u16()? as usize
            };
            objects.push(StaticObject {
                id,
                data: r.bytes(size)?.into(),
            });
        }
        self.object_blocks.push(ObjectBlock {
            metaclass_index,
            transient,
            objects,
        });
        Ok(())
    }

    fn symbolic_names(&mut self, mut r: Reader<'_>) -> Result<(), ImageError> {
        let count = r.u16()?;
        self.symbols.reserve(count as usize);
        for _ in 0..count {
            let holder = r.bytes(Value::ENCODED_SIZE)?;
            let value = Value::decode([holder[0], holder[1], holder[2], holder[3], holder[4]]);
            let len = r.u8()? as usize;
            self.symbols.push(Symbol {
                name: r.string(len)?,
                value,
            });
        }
        Ok(())
    }

    fn static_initializers(&mut self, mut r: Reader<'_>) -> Result<(), ImageError> {
        if self.static_initializers.is_some() {
            return Err(ImageError::DuplicateBlock(BlockKind::SINI));
        }
        let header_size = r.u32()? as usize;
        let static_code_offset = r.u32()?;
        let count = r.u32()? as usize;
        if header_size < 12 {
            return Err(r.malformed("static initializer header is too small"));
        }
        r.seek(header_size)?;
        if count > r.remaining() / 6 {
            return Err(r.truncated());
        }
        let mut initializers = Vec::with_capacity(count);
        for _ in 0..count {
            initializers.push(StaticInitializer {
                object: r.u32()?,
                property: r.u16()?,
            });
        }
        self.static_initializers = Some(StaticInitializers {
            static_code_offset,
            initializers,
        });
        Ok(())
    }

    fn resources(&mut self, mut r: Reader<'_>, data_offset: usize) -> Result<(), ImageError> {
        let block_size = r.remaining();
        let count = r.u16()?;
        for _ in 0..count {
            let offset = r.u32()? as usize;
            let size = r.u32()?;
            let len = r.u8()? as usize;
            // Names are stored with every byte inverted.
            let name: Vec<u8> = r.bytes(len)?.iter().map(|b| b ^ 0xff).collect();
            if offset
                .checked_add(size as usize)
                .is_none_or(|end| end > block_size)
            {
                return Err(r.malformed("resource data lies outside its block"));
            }
            self.resources.push(Resource {
                name: String::from_utf8_lossy(&name).into_owned(),
                offset: data_offset + offset,
                size,
            });
        }
        Ok(())
    }

    fn resource_links(&mut self, mut r: Reader<'_>) -> Result<(), ImageError> {
        let count = r.u16()?;
        for _ in 0..count {
            let len = r.u8()? as usize;
            let name = r.string(len)?;
            let len = r.u8()? as usize;
            let file_name = r.string(len)?;
            self.resource_links.push(ResourceLink { name, file_name });
        }
        Ok(())
    }

    fn source_files(&mut self, mut r: Reader<'_>) -> Result<(), ImageError> {
        let count = r.u16()?;
        let line_record_size = r.u16()? as usize;
        for _ in 0..count {
            let entry_start = r.position();
            let entry_size = r.u32()? as usize;
            let master_index = r.u16()?;
            let len = r.u16()? as usize;
            let name = r.string(len)?;
            let line_count = r.u32()? as usize;
            let mut lines = Vec::new();
            if line_count != 0 {
                if line_record_size < 8 {
                    return Err(r.malformed("source line record is too small"));
                }
                if line_count > r.remaining() / line_record_size {
                    return Err(r.truncated());
                }
                lines.reserve(line_count);
                for _ in 0..line_count {
                    let record = r.bytes(line_record_size)?;
                    lines.push(SourceLine {
                        line: u32::from_le_bytes([record[0], record[1], record[2], record[3]]),
                        code_address: u32::from_le_bytes([
                            record[4], record[5], record[6], record[7],
                        ]),
                    });
                }
            }
            if entry_start + entry_size < r.position() {
                return Err(r.malformed("source file entry is larger than its stated size"));
            }
            r.seek(entry_start + entry_size)?;
            self.debug.source_files.push(SourceFile {
                master_index,
                name,
                lines,
            });
        }
        Ok(())
    }

    fn global_symbols(&mut self, mut r: Reader<'_>) -> Result<(), ImageError> {
        let count = r.u32()? as usize;
        if count > r.remaining() / 6 {
            return Err(r.truncated());
        }
        self.debug.global_symbols.reserve(count);
        for _ in 0..count {
            let name_len = r.u16()? as usize;
            let extra_len = r.u16()? as usize;
            let type_code = r.u16()?;
            let name = r.string(name_len)?;
            let extra = r.bytes(extra_len)?;
            let kind = global_symbol_kind(type_code, extra);
            self.debug.global_symbols.push(GlobalSymbol { name, kind });
        }
        Ok(())
    }

    fn method_headers(&mut self, mut r: Reader<'_>) -> Result<(), ImageError> {
        let count = r.u32()? as usize;
        if count > r.remaining() / 4 {
            return Err(r.truncated());
        }
        self.debug.method_headers.reserve(count);
        for _ in 0..count {
            self.debug.method_headers.push(r.u32()?);
        }
        Ok(())
    }

    fn macros(&mut self, mut r: Reader<'_>) -> Result<(), ImageError> {
        let count = r.u32()? as usize;
        if count > r.remaining() / 10 {
            return Err(r.truncated());
        }
        self.debug.macros.reserve(count);
        for _ in 0..count {
            let len = r.u16()? as usize;
            let name = r.string(len)?;
            let flags = r.u16()?;
            let parameter_count = r.u16()?;
            let mut parameters = Vec::with_capacity(parameter_count as usize);
            for _ in 0..parameter_count {
                let len = r.u16()? as usize;
                parameters.push(r.string(len)?);
            }
            let len = r.u32()? as usize;
            let expansion = r.string(len)?;
            self.debug.macros.push(Macro {
                name,
                function_like: flags & 0x0001 != 0,
                varargs: flags & 0x0002 != 0,
                parameters,
                expansion,
            });
        }
        Ok(())
    }
}

/// Interpret a global symbol's type-specific data. Fields a compiler left
/// out read as zero, and extra bytes are ignored, as the format allows the
/// records to grow.
fn global_symbol_kind(type_code: u16, extra: &[u8]) -> GlobalSymbolKind {
    let byte = |at: usize| extra.get(at).copied().unwrap_or(0);
    let u16_at = |at: usize| u16::from_le_bytes([byte(at), byte(at + 1)]);
    let u32_at =
        |at: usize| u32::from_le_bytes([byte(at), byte(at + 1), byte(at + 2), byte(at + 3)]);
    match type_code {
        1 => GlobalSymbolKind::Function {
            code_offset: u32_at(0),
            argc: u16_at(4),
            varargs: byte(6) != 0,
            has_return: byte(7) != 0,
            optional_argc: u16_at(8),
        },
        2 => GlobalSymbolKind::Object {
            id: u32_at(0),
            modifier: u32_at(4),
        },
        3 => GlobalSymbolKind::Property {
            id: u16_at(0),
            dictionary: byte(2) & 0x01 != 0,
        },
        6 => GlobalSymbolKind::IntrinsicFunction {
            index: u16_at(0),
            set: u16_at(2),
            has_return: byte(4) != 0,
            min_argc: u16_at(5),
            max_argc: u16_at(7),
            varargs: byte(9) != 0,
        },
        9 => GlobalSymbolKind::IntrinsicClass {
            metaclass_index: u16_at(0),
            class_object: u32_at(2),
        },
        10 => GlobalSymbolKind::Enumerator {
            id: u32_at(0),
            token: byte(4) & 0x01 != 0,
        },
        _ => GlobalSymbolKind::Other {
            type_code,
            data: extra.into(),
        },
    }
}

#[cfg(test)]
mod tests;
