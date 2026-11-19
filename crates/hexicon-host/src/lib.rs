//! The interface between the Hexicon VM and the system it runs on.
//!
//! The VM has no direct access to terminals, files, clocks, or randomness.
//! Everything outside the VM goes through a [`Host`], which keeps the VM
//! portable to WebAssembly and makes runs reproducible under test.

use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
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

/// The time on the host's clock.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Time {
    /// Milliseconds since the start of 1970, UTC.
    pub unix_millis: i64,
    /// How far local time is ahead of UTC, in seconds.
    pub utc_offset_seconds: i32,
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

    /// Add to the end of a file, creating it if it is not there.
    fn append_file(&mut self, name: &str, data: &[u8]) -> Result<(), FileError> {
        let _ = (name, data);
        Err(FileError::Unsupported)
    }

    /// Read a line of input from the player, without the line ending.
    /// `None` means there is no more input.
    fn read_line(&mut self) -> Option<String> {
        None
    }

    /// Read one keystroke. A key that types a character is that character;
    /// Enter is a newline; any other key goes by its T3 name in square
    /// brackets, such as `[up]` or `[f1]`. `None` means there is no more
    /// input.
    fn read_key(&mut self) -> Option<String> {
        None
    }

    /// Clear the main console.
    fn clear_screen(&mut self) {}

    /// Pause for a number of milliseconds.
    fn sleep(&mut self, milliseconds: u32) {
        let _ = milliseconds;
    }

    /// Show text in the status line: the left part, which is usually the
    /// name of the player's location.
    fn status_line(&mut self, text: &str) {
        let _ = text;
    }

    /// Show text at the right of the status line, usually the score.
    fn status_right(&mut self, text: &str) {
        let _ = text;
    }

    /// The time now. A host without a clock reports the start of 1970,
    /// which also keeps tests reproducible.
    fn time(&mut self) -> Time {
        Time::default()
    }

    /// A name for the system the host runs on.
    fn system_name(&self) -> String {
        std::env::consts::OS.to_owned()
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
    input: Rc<RefCell<VecDeque<String>>>,
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

    /// Queue a line for the program to read. A read of one key takes a
    /// whole line too, as the name of the key.
    pub fn push_input(&self, line: &str) {
        self.input.borrow_mut().push_back(line.to_owned());
    }

    /// The contents of a file the program wrote.
    pub fn file(&self, name: &str) -> Option<Vec<u8>> {
        self.files.borrow().get(name).cloned()
    }

    /// Supply a file for the program to read.
    pub fn set_file(&self, name: &str, data: &[u8]) {
        self.files
            .borrow_mut()
            .insert(name.to_owned(), data.to_vec());
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

    fn append_file(&mut self, name: &str, data: &[u8]) -> Result<(), FileError> {
        self.files
            .borrow_mut()
            .entry(name.to_owned())
            .or_default()
            .extend_from_slice(data);
        Ok(())
    }

    fn read_line(&mut self) -> Option<String> {
        self.input.borrow_mut().pop_front()
    }

    fn read_key(&mut self) -> Option<String> {
        self.input.borrow_mut().pop_front()
    }
}
