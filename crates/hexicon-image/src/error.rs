//! Errors reported while reading an image file.

use std::fmt;

/// A four-byte block type identifier, such as `ENTP` or `OBJS`.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct BlockKind(pub [u8; 4]);

impl BlockKind {
    /// End-of-file marker.
    pub const EOF: Self = Self(*b"EOF ");
    /// Entry point and global structure sizes.
    pub const ENTP: Self = Self(*b"ENTP");
    /// Static objects.
    pub const OBJS: Self = Self(*b"OBJS");
    /// Constant pool definition.
    pub const CPDF: Self = Self(*b"CPDF");
    /// Constant pool page.
    pub const CPPG: Self = Self(*b"CPPG");
    /// Embedded resources.
    pub const MRES: Self = Self(*b"MRES");
    /// Links to resources in local files.
    pub const MREL: Self = Self(*b"MREL");
    /// Metaclass dependency list.
    pub const MCLD: Self = Self(*b"MCLD");
    /// Function set dependency list.
    pub const FNSD: Self = Self(*b"FNSD");
    /// Symbolic names exported to the VM.
    pub const SYMD: Self = Self(*b"SYMD");
    /// Source file descriptors.
    pub const SRCF: Self = Self(*b"SRCF");
    /// Global symbol table.
    pub const GSYM: Self = Self(*b"GSYM");
    /// Method header list.
    pub const MHLS: Self = Self(*b"MHLS");
    /// Preprocessor macro table.
    pub const MACR: Self = Self(*b"MACR");
    /// Static initializer list.
    pub const SINI: Self = Self(*b"SINI");
}

impl fmt::Display for BlockKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for &b in &self.0 {
            if b.is_ascii_graphic() || b == b' ' {
                write!(f, "{}", b as char)?;
            } else {
                write!(f, "\\x{b:02x}")?;
            }
        }
        Ok(())
    }
}

impl fmt::Debug for BlockKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "BlockKind({self})")
    }
}

/// An error found while reading an image file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImageError {
    /// The file does not start with an image's signature.
    NotAnImage,
    /// The file uses a format version newer than this reader understands.
    UnsupportedVersion(u16),
    /// The file ended in the middle of the header or a block header.
    TruncatedFile {
        /// File offset at which more data was expected.
        offset: usize,
    },
    /// The file ended without an end-of-file block.
    MissingEof,
    /// A block's contents ended before a field was complete.
    TruncatedBlock {
        /// The block being read.
        kind: BlockKind,
        /// File offset of the block's header.
        block_offset: usize,
    },
    /// A block this reader does not know is marked as mandatory.
    UnknownMandatoryBlock {
        /// The unrecognized block type.
        kind: BlockKind,
        /// File offset of the block's header.
        block_offset: usize,
    },
    /// A block that may appear only once appears more than once.
    DuplicateBlock(BlockKind),
    /// A block every program image needs is absent.
    MissingBlock(BlockKind),
    /// A pool block names a pool other than the code or constant pool.
    UnknownPool(u16),
    /// A pool is defined more than once.
    DuplicatePool(u16),
    /// A pool page appears before the definition of its pool.
    PageBeforePoolDefinition(u16),
    /// A pool page's index is beyond the pool's page count.
    PageIndexOutOfRange {
        /// The pool identifier.
        pool: u16,
        /// The page index given in the page block.
        index: u32,
    },
    /// A pool page appears more than once.
    DuplicatePage {
        /// The pool identifier.
        pool: u16,
        /// The page index given in the page block.
        index: u32,
    },
    /// A pool page holds more data than the pool's page size.
    PageTooLarge {
        /// The pool identifier.
        pool: u16,
        /// The page index given in the page block.
        index: u32,
    },
    /// A static object block appears before the metaclass dependency list.
    ObjectsBeforeMetaclasses,
    /// A static object block names a metaclass that is not in the dependency list.
    MetaclassIndexOutOfRange(u16),
    /// A size or offset inside a block is inconsistent with the block.
    Malformed {
        /// The block being read.
        kind: BlockKind,
        /// File offset of the block's header.
        block_offset: usize,
        /// What was wrong.
        reason: &'static str,
    },
}

impl fmt::Display for ImageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotAnImage => write!(f, "not a game file"),
            Self::UnsupportedVersion(v) => write!(f, "unsupported image format version {v}"),
            Self::TruncatedFile { offset } => {
                write!(f, "file ends unexpectedly at offset {offset}")
            }
            Self::MissingEof => write!(f, "file has no end-of-file block"),
            Self::TruncatedBlock { kind, block_offset } => {
                write!(
                    f,
                    "{kind} block at offset {block_offset} is too short for its contents"
                )
            }
            Self::UnknownMandatoryBlock { kind, block_offset } => {
                write!(f, "unknown mandatory block {kind} at offset {block_offset}")
            }
            Self::DuplicateBlock(kind) => write!(f, "more than one {kind} block"),
            Self::MissingBlock(kind) => write!(f, "no {kind} block"),
            Self::UnknownPool(id) => write!(f, "unknown constant pool {id}"),
            Self::DuplicatePool(id) => write!(f, "constant pool {id} is defined more than once"),
            Self::PageBeforePoolDefinition(id) => {
                write!(
                    f,
                    "page for constant pool {id} appears before the pool's definition"
                )
            }
            Self::PageIndexOutOfRange { pool, index } => {
                write!(f, "page {index} is out of range for constant pool {pool}")
            }
            Self::DuplicatePage { pool, index } => {
                write!(
                    f,
                    "page {index} of constant pool {pool} appears more than once"
                )
            }
            Self::PageTooLarge { pool, index } => {
                write!(
                    f,
                    "page {index} of constant pool {pool} is larger than the pool's page size"
                )
            }
            Self::ObjectsBeforeMetaclasses => {
                write!(
                    f,
                    "static objects appear before the metaclass dependency list"
                )
            }
            Self::MetaclassIndexOutOfRange(i) => write!(f, "metaclass index {i} is out of range"),
            Self::Malformed {
                kind,
                block_offset,
                reason,
            } => {
                write!(
                    f,
                    "malformed {kind} block at offset {block_offset}: {reason}"
                )
            }
        }
    }
}

impl std::error::Error for ImageError {}
