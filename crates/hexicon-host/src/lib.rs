//! The interface between the Hexicon VM and the system it runs on.
//!
//! The VM has no direct access to terminals, files, clocks, or randomness.
//! Everything outside the VM goes through a [`Host`], which keeps the VM
//! portable to WebAssembly and makes runs reproducible under test.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

/// Why a file could not be read or written.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileError {
    /// There is no file by that name.
    NotFound,
    /// The host does not let the program use that file.
    Denied,
    /// The host has no files.
    Unsupported,
    /// Anything else, such as a full disk.
    Failed,
}

/// The services a VM needs from the system it runs on.
pub trait Host {
    /// Display text on the main console.
    fn print(&mut self, text: &str);

    /// Fill a buffer with unpredictable bytes, for seeding the random
    /// number generator. A host that leaves the buffer alone gives every
    /// run the same seed, which is what tests want.
    fn random_bytes(&mut self, buffer: &mut [u8]) {
        let _ = buffer;
    }

    /// The whole contents of a file. Names are the host's to interpret:
    /// a path on a system with a file system, a key in a browser's
    /// storage.
    fn read_file(&mut self, name: &str) -> Result<Vec<u8>, FileError> {
        let _ = name;
        Err(FileError::Unsupported)
    }

    /// Create a file with the given contents, replacing any there was.
    fn write_file(&mut self, name: &str, data: &[u8]) -> Result<(), FileError> {
        let _ = (name, data);
        Err(FileError::Unsupported)
    }
}

/// A host that collects console output in memory.
///
/// Clones share one buffer, so a caller can keep a clone, hand the other to
/// a VM, and read the output afterward.
#[derive(Debug, Default, Clone)]
pub struct BufferHost {
    output: Rc<RefCell<String>>,
    files: Rc<RefCell<HashMap<String, Vec<u8>>>>,
}

impl BufferHost {
    /// A host with an empty buffer.
    pub fn new() -> Self {
        Self::default()
    }

    /// Everything displayed so far.
    pub fn output(&self) -> String {
        self.output.borrow().clone()
    }
}

impl Host for BufferHost {
    fn print(&mut self, text: &str) {
        self.output.borrow_mut().push_str(text);
    }

    // Files are kept in memory, shared between clones like the output.
    fn read_file(&mut self, name: &str) -> Result<Vec<u8>, FileError> {
        self.files
            .borrow()
            .get(name)
            .cloned()
            .ok_or(FileError::NotFound)
    }

    fn write_file(&mut self, name: &str, data: &[u8]) -> Result<(), FileError> {
        self.files
            .borrow_mut()
            .insert(name.to_owned(), data.to_vec());
        Ok(())
    }
}
