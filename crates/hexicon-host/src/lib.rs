//! The interface between the Hexicon VM and the system it runs on.
//!
//! The VM has no direct access to terminals, files, clocks, or randomness.
//! Everything outside the VM goes through a [`Host`], which keeps the VM
//! portable to WebAssembly and makes runs reproducible under test.

pub mod entities;
pub mod pane;
pub mod rich;

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

/// What kind of thing a file name refers to. The values are the ones
/// programs see, and can be combined.
pub mod file_type {
    /// An ordinary file.
    pub const FILE: u32 = 0x01;
    /// A directory.
    pub const DIR: u32 = 0x02;
    /// A character device.
    pub const CHAR: u32 = 0x04;
    /// A block device.
    pub const BLOCK: u32 = 0x08;
    /// A pipe.
    pub const PIPE: u32 = 0x10;
    /// A socket.
    pub const SOCKET: u32 = 0x20;
    /// A symbolic link.
    pub const LINK: u32 = 0x40;
}

/// Attributes of a file. The values are the ones programs see.
pub mod file_attr {
    /// Not normally listed.
    pub const HIDDEN: u32 = 0x01;
    /// Belongs to the system.
    pub const SYSTEM: u32 = 0x02;
    /// Can be read.
    pub const READ: u32 = 0x04;
    /// Can be written.
    pub const WRITE: u32 = 0x08;
}

/// What the host knows about a file.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FileInfo {
    /// A combination of [`file_type`] values.
    pub kind: u32,
    /// A combination of [`file_attr`] values.
    pub attrs: u32,
    /// The size in bytes.
    pub size: u64,
    /// When the file was last read, in seconds since the start of 1970.
    pub accessed: Option<i64>,
    /// When the file was last changed.
    pub modified: Option<i64>,
    /// When the file was created.
    pub created: Option<i64>,
    /// Where a symbolic link leads.
    pub link_target: Option<String>,
}

/// Where a file is in relation to the folder a program's file access is
/// confined to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sandbox {
    /// In the folder, or in a folder within it.
    Inside,
    /// The folder itself.
    Itself,
    /// A folder the sandbox is in.
    Contains,
    /// Anywhere else.
    Outside,
}

/// What a host has when it is asked for input.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Input {
    /// A line the player entered, or a key.
    Text(String),
    /// There is no more input and never will be.
    End,
    /// There is none yet. The VM stops where it is and hands control
    /// back, and asks again when it is resumed. A host that cannot wait
    /// for the player without freezing, as in a browser, answers this
    /// way; one that can wait never needs to.
    Pending,
}

/// Which file dialog a program asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileDialog {
    /// A file to read: one that is there.
    Open,
    /// A file to write: one that may not be there yet.
    Save,
}

/// What the player did with a file dialog.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileChoice {
    /// They chose a file: its name, as the host's file functions take it.
    Chosen(String),
    /// They chose none.
    Cancelled,
    /// They have not chosen yet. As with [`Input::Pending`], the VM stops
    /// and asks again when it is resumed.
    Pending,
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

    /// What the display can do beyond plain text, if it is a rich
    /// display; `None` for a display of text, which gets its text through
    /// [`Host::print`], broken into lines that fit.
    fn rich_display(&self) -> Option<rich::RichDisplay> {
        None
    }

    /// On a rich display: what the program wrote, as it wrote it, markup
    /// and all, in order. The host checks it with a [`rich::Checker`].
    fn print_markup(&mut self, markup: &str) {
        let _ = markup;
    }

    /// On a rich display: a line the player typed, from a script, shown
    /// as the player's own words.
    fn print_input(&mut self, text: &str) {
        let _ = text;
    }

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

    /// What a name refers to, or `None` if nothing. With `follow_links`,
    /// a symbolic link is described by what it leads to.
    ///
    /// File names are in one syntax on every host: parts separated by
    /// `/`, with `.` for the folder itself and `..` for the one above. A
    /// name that does not start at a root is relative to wherever the
    /// host keeps the program's files.
    fn file_info(&mut self, name: &str, follow_links: bool) -> Option<FileInfo> {
        let _ = (name, follow_links);
        None
    }

    /// Delete a file.
    fn delete_file(&mut self, name: &str) -> Result<(), FileError> {
        let _ = name;
        Err(FileError::Unsupported)
    }

    /// Give a file a new name, which must not be in use.
    fn rename_file(&mut self, from: &str, to: &str) -> Result<(), FileError> {
        let _ = (from, to);
        Err(FileError::Unsupported)
    }

    /// Create a directory, and with `parents` any directories above it
    /// that are missing.
    fn create_dir(&mut self, name: &str, parents: bool) -> Result<(), FileError> {
        let _ = (name, parents);
        Err(FileError::Unsupported)
    }

    /// Remove a directory, which has to be empty unless `contents` says
    /// to delete what is in it too.
    fn remove_dir(&mut self, name: &str, contents: bool) -> Result<(), FileError> {
        let _ = (name, contents);
        Err(FileError::Unsupported)
    }

    /// The names in a directory, without the path to it. A host whose
    /// directories have entries for themselves and their parents includes
    /// `.` and `..`.
    fn list_dir(&mut self, name: &str) -> Option<Vec<String>> {
        let _ = name;
        None
    }

    /// The full name of a file, from a root.
    fn absolute_path(&mut self, name: &str) -> Option<String> {
        let _ = name;
        None
    }

    /// The roots of the file system: `/` on some systems, a list of
    /// drives on others.
    fn root_dirs(&mut self) -> Vec<String> {
        Vec::new()
    }

    /// Where a file is in relation to the folder the program's file
    /// access is confined to. The default confines nothing.
    fn sandbox(&mut self, name: &str) -> Sandbox {
        let _ = name;
        Sandbox::Inside
    }

    /// A name for a new temporary file, which does not exist yet.
    fn temp_file_name(&mut self) -> Option<String> {
        None
    }

    /// The name of one of the files that programs ask for by number
    /// because where they are kept is up to the system: 1 is the file of
    /// settings shared by every game that uses the standard library, and
    /// 2 the same for the web interface.
    fn special_file(&mut self, id: i32) -> Option<String> {
        let _ = id;
        None
    }

    /// Add to the end of a file, creating it if it is not there.
    fn append_file(&mut self, name: &str, data: &[u8]) -> Result<(), FileError> {
        let _ = (name, data);
        Err(FileError::Unsupported)
    }

    /// Read a line of input from the player, without the line ending.
    fn read_line(&mut self) -> Input {
        Input::End
    }

    /// Read one keystroke. A key that types a character is that character;
    /// Enter is a newline; any other key goes by its name in square
    /// brackets, such as `[up]` or `[f1]`.
    fn read_key(&mut self) -> Input {
        Input::End
    }

    /// Ask the player for a file in a dialog of the system's own, with
    /// the program's prompt as its title, or `None` for a host that has
    /// no such dialog: the VM then asks for the file's name as a line of
    /// input, after the prompt, as a console of text does.
    fn choose_file(&mut self, prompt: &str, dialog: FileDialog) -> Option<FileChoice> {
        let _ = (prompt, dialog);
        None
    }

    /// Clear the main console.
    fn clear_screen(&mut self) {}

    /// The width of the main console in columns, if the host has one of
    /// its own, such as a terminal window's. Text is broken into lines
    /// that fit. Without one the width is 80 columns.
    fn console_width(&mut self) -> Option<usize> {
        None
    }

    /// Input has just come from a script instead of from the player. A
    /// host that pauses when a screenful of text has gone by since the
    /// player last typed starts counting again from here.
    fn script_input(&mut self) {}

    /// Pause for a number of milliseconds. A host that can wait does,
    /// and returns true. One that cannot returns false, which stops the
    /// VM where it is; when the time has passed, the host resumes the VM,
    /// which asks again, and this time the answer is true.
    fn sleep(&mut self, milliseconds: u32) -> bool {
        let _ = milliseconds;
        true
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

    /// The name of the host's time zone in the zoneinfo database, such as
    /// `America/Chicago`, if the host knows it. Without one, local time
    /// is UTC shifted by the offset that [`Host::time`] reports.
    fn local_time_zone(&mut self) -> Option<String> {
        None
    }

    /// A name for the system the host runs on.
    fn system_name(&self) -> String {
        std::env::consts::OS.to_owned()
    }

    /// Whether the host can show panes. Without them, a program
    /// that asks for one is told it cannot have it, as on a plain
    /// console.
    fn has_panes(&self) -> bool {
        false
    }

    /// Create a pane inside another, or inside the main window
    /// if `parent` is `None`: its handle, or `None` if the host cannot.
    fn pane_create(
        &mut self,
        parent: Option<u32>,
        placement: pane::Placement,
        spec: pane::PaneSpec,
    ) -> Option<u32> {
        let _ = (parent, placement, spec);
        None
    }

    /// Remove a pane.
    fn pane_delete(&mut self, id: u32) {
        let _ = id;
    }

    /// Remove the text in a pane.
    fn pane_clear(&mut self, id: u32) {
        let _ = id;
    }

    /// Add text to a pane. The text is already broken into lines
    /// that fit.
    fn pane_write(&mut self, id: u32, text: &str) {
        let _ = (id, text);
    }

    /// On a rich display: add what the program wrote to a text pane, as
    /// it wrote it, markup and all, as [`Host::print_markup`] does for the
    /// main window. A text grid's text still comes through
    /// [`Host::pane_write`].
    fn pane_write_markup(&mut self, id: u32, markup: &str) {
        let _ = (id, markup);
    }

    /// Show what has been written to a pane.
    fn pane_flush(&mut self, id: u32) {
        let _ = id;
    }

    /// Change the size of a pane. See [`pane::Screen::set_size`].
    fn pane_set_size(&mut self, id: u32, size: i32, units: i32, advisory: bool) {
        let _ = (id, size, units, advisory);
    }

    /// Make a pane as big as what it holds.
    fn pane_size_to_contents(&mut self, id: u32) {
        let _ = id;
    }

    /// Move where text goes next in a text grid, counting from zero.
    fn pane_go_to(&mut self, id: u32, row: usize, column: usize) {
        let _ = (id, row, column);
    }

    /// What a program can learn about a pane.
    fn pane_info(&mut self, id: u32) -> Option<pane::PaneInfo> {
        let _ = id;
        None
    }

    /// Give a pane a background color: one of the colors the
    /// host chooses, such as [`pane::color::STATUS_BG`], or an RGB value.
    fn pane_set_screen_color(&mut self, id: u32, color: u32) {
        let _ = (id, color);
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
    temp_counter: Rc<RefCell<u32>>,
    interactive: Rc<RefCell<bool>>,
    screen: Rc<RefCell<Option<pane::Screen>>>,
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

    /// Say that more input may be queued later, so that running out is
    /// not the end of input: the program is to stop and wait.
    pub fn set_interactive(&self, interactive: bool) {
        *self.interactive.borrow_mut() = interactive;
    }

    fn next_input(&self) -> Input {
        match self.input.borrow_mut().pop_front() {
            Some(line) => Input::Text(line),
            None if *self.interactive.borrow() => Input::Pending,
            None => Input::End,
        }
    }

    /// The contents of a file the program wrote.
    pub fn file(&self, name: &str) -> Option<Vec<u8>> {
        self.files.borrow().get(name).cloned()
    }

    /// Show panes, on a screen of the given size.
    pub fn enable_panes(&self, rows: usize, columns: usize) {
        *self.screen.borrow_mut() = Some(pane::Screen::new(rows, columns, true));
    }

    /// The panes, if they are shown.
    pub fn screen(&self) -> std::cell::Ref<'_, Option<pane::Screen>> {
        self.screen.borrow()
    }

    fn with_screen<T>(&self, f: impl FnOnce(&mut pane::Screen) -> T) -> Option<T> {
        self.screen.borrow_mut().as_mut().map(f)
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

    // The files have no directories: a name is only a key.
    fn file_info(&mut self, name: &str, _follow_links: bool) -> Option<FileInfo> {
        self.files.borrow().get(name).map(|data| FileInfo {
            kind: file_type::FILE,
            attrs: file_attr::READ | file_attr::WRITE,
            size: data.len() as u64,
            ..FileInfo::default()
        })
    }

    fn delete_file(&mut self, name: &str) -> Result<(), FileError> {
        match self.files.borrow_mut().remove(name) {
            Some(_) => Ok(()),
            None => Err(FileError::NotFound),
        }
    }

    fn rename_file(&mut self, from: &str, to: &str) -> Result<(), FileError> {
        let mut files = self.files.borrow_mut();
        if files.contains_key(to) {
            return Err(FileError::Failed);
        }
        let data = files.remove(from).ok_or(FileError::NotFound)?;
        files.insert(to.to_owned(), data);
        Ok(())
    }

    fn temp_file_name(&mut self) -> Option<String> {
        let mut counter = self.temp_counter.borrow_mut();
        *counter += 1;
        Some(format!("/tmp/temp{counter}"))
    }

    fn read_line(&mut self) -> Input {
        self.next_input()
    }

    fn read_key(&mut self) -> Input {
        self.next_input()
    }

    fn has_panes(&self) -> bool {
        self.screen.borrow().is_some()
    }

    fn pane_create(
        &mut self,
        parent: Option<u32>,
        placement: pane::Placement,
        spec: pane::PaneSpec,
    ) -> Option<u32> {
        self.with_screen(|screen| screen.create(parent, placement, spec))
            .flatten()
    }

    fn pane_delete(&mut self, id: u32) {
        self.with_screen(|screen| screen.delete(id));
    }

    fn pane_clear(&mut self, id: u32) {
        self.with_screen(|screen| screen.clear(id));
    }

    fn pane_write(&mut self, id: u32, text: &str) {
        self.with_screen(|screen| screen.write(id, text));
    }

    fn pane_set_size(&mut self, id: u32, size: i32, units: i32, advisory: bool) {
        self.with_screen(|screen| screen.set_size(id, size, units, advisory));
    }

    fn pane_size_to_contents(&mut self, id: u32) {
        self.with_screen(|screen| screen.size_to_contents(id));
    }

    fn pane_go_to(&mut self, id: u32, row: usize, column: usize) {
        self.with_screen(|screen| screen.go_to(id, row, column));
    }

    fn pane_info(&mut self, id: u32) -> Option<pane::PaneInfo> {
        self.with_screen(|screen| screen.info(id)).flatten()
    }

    fn pane_set_screen_color(&mut self, id: u32, color: u32) {
        self.with_screen(|screen| screen.set_screen_color(id, color));
    }
}
