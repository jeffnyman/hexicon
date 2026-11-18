//! The interface between the Hexicon VM and the system it runs on.
//!
//! The VM has no direct access to terminals, files, clocks, or randomness.
//! Everything outside the VM goes through a [`Host`], which keeps the VM
//! portable to WebAssembly and makes runs reproducible under test.

use std::cell::RefCell;
use std::rc::Rc;

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
}

/// A host that collects console output in memory.
///
/// Clones share one buffer, so a caller can keep a clone, hand the other to
/// a VM, and read the output afterward.
#[derive(Debug, Default, Clone)]
pub struct BufferHost {
    output: Rc<RefCell<String>>,
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
}
