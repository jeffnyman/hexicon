//! A cursor over the contents of one block.

use crate::error::{BlockKind, ImageError};

/// Reads little-endian fields from a block's data, reporting any overrun as
/// an error against that block.
pub(crate) struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
    kind: BlockKind,
    block_offset: usize,
}

impl<'a> Reader<'a> {
    pub(crate) fn new(data: &'a [u8], kind: BlockKind, block_offset: usize) -> Self {
        Self {
            data,
            pos: 0,
            kind,
            block_offset,
        }
    }

    /// Current position, as an offset from the start of the block's data.
    pub(crate) fn position(&self) -> usize {
        self.pos
    }

    pub(crate) fn remaining(&self) -> usize {
        self.data.len() - self.pos
    }

    pub(crate) fn truncated(&self) -> ImageError {
        ImageError::TruncatedBlock {
            kind: self.kind,
            block_offset: self.block_offset,
        }
    }

    pub(crate) fn malformed(&self, reason: &'static str) -> ImageError {
        ImageError::Malformed {
            kind: self.kind,
            block_offset: self.block_offset,
            reason,
        }
    }

    pub(crate) fn bytes(&mut self, len: usize) -> Result<&'a [u8], ImageError> {
        if len > self.remaining() {
            return Err(self.truncated());
        }
        let slice = &self.data[self.pos..self.pos + len];
        self.pos += len;
        Ok(slice)
    }

    /// The rest of the block.
    pub(crate) fn rest(&mut self) -> &'a [u8] {
        let slice = &self.data[self.pos..];
        self.pos = self.data.len();
        slice
    }

    /// Move to an absolute position within the block's data.
    pub(crate) fn seek(&mut self, pos: usize) -> Result<(), ImageError> {
        if pos > self.data.len() {
            return Err(self.truncated());
        }
        self.pos = pos;
        Ok(())
    }

    pub(crate) fn u8(&mut self) -> Result<u8, ImageError> {
        Ok(self.bytes(1)?[0])
    }

    pub(crate) fn u16(&mut self) -> Result<u16, ImageError> {
        let b = self.bytes(2)?;
        Ok(u16::from_le_bytes([b[0], b[1]]))
    }

    pub(crate) fn u32(&mut self) -> Result<u32, ImageError> {
        let b = self.bytes(4)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    /// A string of `len` bytes. Names in image files are ASCII or UTF-8;
    /// anything else is replaced rather than rejected.
    pub(crate) fn string(&mut self, len: usize) -> Result<String, ImageError> {
        Ok(String::from_utf8_lossy(self.bytes(len)?).into_owned())
    }
}
