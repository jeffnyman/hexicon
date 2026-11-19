//! Hexicon for web browsers: a game that a page's JavaScript runs.
//!
//! A page cannot wait for the player without freezing, so a game runs
//! until it needs something from the page and then stops, saying what it
//! needs. The page supplies it and tells the game to carry on:
//!
//! ```js
//! const game = new Game(imageBytes, "game.t3", timeZoneName);
//! let state = game.start();
//! show(game.takeOutput());
//! // state is "line", "key", "sleep", or "done". For "line":
//! game.sendLine("look");
//! state = game.resume();
//! ```
//!
//! The game's files are kept in memory. The page can read them out to
//! store them, and put them back the next time the game is loaded.

use std::cell::RefCell;
use std::collections::{BTreeMap, VecDeque};
use std::rc::Rc;

use hexicon_host::{FileError, FileInfo, Host, Input, Sandbox, Time, file_attr, file_type};
use hexicon_image::Image;
use hexicon_vm::{RunState, Vm};
use wasm_bindgen::prelude::*;

/// What a game that has stopped is waiting for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Waiting {
    Nothing,
    Line,
    Key,
    Sleep(u32),
}

/// What the game and the page pass between them.
#[derive(Debug)]
struct Shared {
    output: String,
    lines: VecDeque<String>,
    keys: VecDeque<String>,
    waiting: Waiting,
    /// A pause the game asked for is over.
    woken: bool,
    /// The screen was cleared since the page last asked.
    cleared: bool,
    status: String,
    status_right: String,
    files: BTreeMap<String, Vec<u8>>,
    /// Files written or deleted since the page last asked.
    changed: Vec<String>,
    time_zone: Option<String>,
    temp_count: u32,
}

impl Shared {
    fn note_change(&mut self, name: &str) {
        if !self.changed.iter().any(|changed| changed == name) {
            self.changed.push(name.to_owned());
        }
    }
}

/// The host a game in a browser runs on.
struct WebHost(Rc<RefCell<Shared>>);

impl Host for WebHost {
    fn print(&mut self, text: &str) {
        self.0.borrow_mut().output.push_str(text);
    }

    fn random_bytes(&mut self, buffer: &mut [u8]) {
        for byte in buffer {
            *byte = (js_sys::Math::random() * 256.0) as u8;
        }
    }

    // Files are kept in memory, by name. The page stores them.
    fn read_file(&mut self, name: &str) -> Result<Vec<u8>, FileError> {
        self.0
            .borrow()
            .files
            .get(name)
            .cloned()
            .ok_or(FileError::NotFound)
    }

    fn write_file(&mut self, name: &str, data: &[u8]) -> Result<(), FileError> {
        let mut shared = self.0.borrow_mut();
        shared.files.insert(name.to_owned(), data.to_vec());
        shared.note_change(name);
        Ok(())
    }

    fn append_file(&mut self, name: &str, data: &[u8]) -> Result<(), FileError> {
        let mut shared = self.0.borrow_mut();
        shared
            .files
            .entry(name.to_owned())
            .or_default()
            .extend_from_slice(data);
        shared.note_change(name);
        Ok(())
    }

    fn file_info(&mut self, name: &str, _follow_links: bool) -> Option<FileInfo> {
        self.0.borrow().files.get(name).map(|data| FileInfo {
            kind: file_type::FILE,
            attrs: file_attr::READ | file_attr::WRITE,
            size: data.len() as u64,
            ..FileInfo::default()
        })
    }

    fn delete_file(&mut self, name: &str) -> Result<(), FileError> {
        let mut shared = self.0.borrow_mut();
        if shared.files.remove(name).is_none() {
            return Err(FileError::NotFound);
        }
        shared.note_change(name);
        Ok(())
    }

    fn rename_file(&mut self, from: &str, to: &str) -> Result<(), FileError> {
        let mut shared = self.0.borrow_mut();
        if shared.files.contains_key(to) {
            return Err(FileError::Failed);
        }
        let data = shared.files.remove(from).ok_or(FileError::NotFound)?;
        shared.files.insert(to.to_owned(), data);
        shared.note_change(from);
        shared.note_change(to);
        Ok(())
    }

    // There are no folders: every file is listed for any of them.
    fn list_dir(&mut self, _name: &str) -> Option<Vec<String>> {
        Some(self.0.borrow().files.keys().cloned().collect())
    }

    fn sandbox(&mut self, _name: &str) -> Sandbox {
        Sandbox::Inside
    }

    fn temp_file_name(&mut self) -> Option<String> {
        let mut shared = self.0.borrow_mut();
        shared.temp_count += 1;
        Some(format!("temporary-{}", shared.temp_count))
    }

    fn special_file(&mut self, id: i32) -> Option<String> {
        match id {
            1 => Some("settings.txt".to_owned()),
            2 => Some("webprefs.txt".to_owned()),
            _ => None,
        }
    }

    fn read_line(&mut self) -> Input {
        let mut shared = self.0.borrow_mut();
        match shared.lines.pop_front() {
            Some(line) => Input::Text(line),
            None => {
                shared.waiting = Waiting::Line;
                Input::Pending
            }
        }
    }

    fn read_key(&mut self) -> Input {
        let mut shared = self.0.borrow_mut();
        match shared.keys.pop_front() {
            Some(key) => Input::Text(key),
            None => {
                shared.waiting = Waiting::Key;
                Input::Pending
            }
        }
    }

    fn clear_screen(&mut self) {
        let mut shared = self.0.borrow_mut();
        shared.output.clear();
        shared.cleared = true;
    }

    // The page does the waiting, and wakes the game when it is over.
    fn sleep(&mut self, milliseconds: u32) -> bool {
        let mut shared = self.0.borrow_mut();
        if shared.woken || milliseconds == 0 {
            shared.woken = false;
            true
        } else {
            shared.waiting = Waiting::Sleep(milliseconds);
            false
        }
    }

    fn status_line(&mut self, text: &str) {
        self.0.borrow_mut().status = text.to_owned();
    }

    fn status_right(&mut self, text: &str) {
        self.0.borrow_mut().status_right = text.to_owned();
    }

    fn time(&mut self) -> Time {
        let now = js_sys::Date::new_0();
        Time {
            unix_millis: now.get_time() as i64,
            // The browser gives the offset in minutes, counted westward.
            utc_offset_seconds: (-now.get_timezone_offset() * 60.0) as i32,
        }
    }

    fn local_time_zone(&mut self) -> Option<String> {
        self.0.borrow().time_zone.clone()
    }

    fn system_name(&self) -> String {
        "web".to_owned()
    }
}

/// A game loaded and ready to play.
#[wasm_bindgen]
pub struct Game {
    vm: Vm,
    shared: Rc<RefCell<Shared>>,
    name: String,
    started: bool,
    finished: bool,
}

#[wasm_bindgen]
impl Game {
    /// Load a game from the bytes of its image file. `name` is the file's
    /// name, which the game sees as its first argument. `time_zone` is
    /// the player's time zone by its zoneinfo name, such as
    /// `America/Chicago`, if the page knows it.
    #[wasm_bindgen(constructor)]
    pub fn new(image: &[u8], name: &str, time_zone: Option<String>) -> Result<Game, JsError> {
        Self::load(image, name, time_zone, true)
    }

    /// Load a game that makes the same random choices every time it is
    /// played, for testing.
    #[wasm_bindgen(js_name = withoutRandomness)]
    pub fn without_randomness(
        image: &[u8],
        name: &str,
        time_zone: Option<String>,
    ) -> Result<Game, JsError> {
        Self::load(image, name, time_zone, false)
    }

    fn load(
        image: &[u8],
        name: &str,
        time_zone: Option<String>,
        seed_random: bool,
    ) -> Result<Game, JsError> {
        let parsed = Image::parse(image).map_err(|error| JsError::new(&error.to_string()))?;
        let shared = Rc::new(RefCell::new(Shared {
            output: String::new(),
            lines: VecDeque::new(),
            keys: VecDeque::new(),
            waiting: Waiting::Nothing,
            woken: false,
            cleared: false,
            status: String::new(),
            status_right: String::new(),
            files: BTreeMap::new(),
            changed: Vec::new(),
            time_zone,
            temp_count: 0,
        }));
        let mut vm = Vm::new(&parsed, Box::new(WebHost(shared.clone())))
            .map_err(|error| JsError::new(&error.to_string()))?;
        vm.set_image_name(name);
        if seed_random {
            vm.seed_random();
        }
        for resource in &parsed.resources {
            let end = resource.offset + resource.size as usize;
            if let Some(contents) = image.get(resource.offset..end) {
                vm.add_resource(&resource.name, contents);
            }
        }
        Ok(Game {
            vm,
            shared,
            name: name.to_owned(),
            started: false,
            finished: false,
        })
    }

    /// Start the game, and run it until it needs something. Returns what
    /// it needs: `line` for a line of input, `key` for a keystroke,
    /// `sleep` for a pause of `sleepMillis()`, or `done` if the game is
    /// over.
    pub fn start(&mut self) -> String {
        if self.started {
            return self.resume();
        }
        self.started = true;
        let result = self.vm.start(std::slice::from_ref(&self.name));
        self.after_run(result)
    }

    /// Carry on with the game after supplying what it needed, until it
    /// needs something else. Returns the same as `start`.
    pub fn resume(&mut self) -> String {
        if self.finished || !self.started {
            return "done".to_owned();
        }
        self.shared.borrow_mut().waiting = Waiting::Nothing;
        let result = self.vm.resume();
        self.after_run(result)
    }

    fn after_run(&mut self, result: hexicon_vm::VmResult<RunState>) -> String {
        match result {
            Ok(RunState::Suspended) => {
                // What is on the line so far is the prompt.
                self.vm.flush_output();
                let waiting = self.shared.borrow().waiting;
                match waiting {
                    Waiting::Key => "key",
                    Waiting::Sleep(_) => "sleep",
                    _ => "line",
                }
                .to_owned()
            }
            Ok(RunState::Finished) => self.finish(None),
            Err(error) => {
                let message = self.vm.error_message(&error, true);
                self.finish(Some(message))
            }
        }
    }

    fn finish(&mut self, error: Option<String>) -> String {
        self.finished = true;
        if let Some(message) = error {
            let mut shared = self.shared.borrow_mut();
            shared.output.push_str(&message);
            shared.output.push('\n');
        }
        self.vm.flush_output();
        self.vm.close_files();
        "done".to_owned()
    }

    /// Give the game a line the player entered.
    #[wasm_bindgen(js_name = sendLine)]
    pub fn send_line(&mut self, line: &str) {
        self.shared.borrow_mut().lines.push_back(line.to_owned());
    }

    /// Give the game a keystroke: the character a key types, a newline
    /// for Enter, or a name in brackets such as `[up]`.
    #[wasm_bindgen(js_name = sendKey)]
    pub fn send_key(&mut self, key: &str) {
        self.shared.borrow_mut().keys.push_back(key.to_owned());
    }

    /// Tell the game that the pause it asked for is over.
    pub fn wake(&mut self) {
        self.shared.borrow_mut().woken = true;
    }

    /// How long a pause the game is waiting for, in milliseconds.
    #[wasm_bindgen(js_name = sleepMillis)]
    pub fn sleep_millis(&self) -> u32 {
        match self.shared.borrow().waiting {
            Waiting::Sleep(milliseconds) => milliseconds,
            _ => 0,
        }
    }

    /// The text the game has displayed since this was last called.
    #[wasm_bindgen(js_name = takeOutput)]
    pub fn take_output(&mut self) -> String {
        std::mem::take(&mut self.shared.borrow_mut().output)
    }

    /// Whether the game cleared the screen since this was last called.
    /// If so, what `takeOutput` returns next is all that should show.
    #[wasm_bindgen(js_name = takeCleared)]
    pub fn take_cleared(&mut self) -> bool {
        std::mem::take(&mut self.shared.borrow_mut().cleared)
    }

    /// The left part of the status line: usually the player's location.
    pub fn status(&self) -> String {
        self.shared.borrow().status.clone()
    }

    /// The right part of the status line: usually the score.
    #[wasm_bindgen(js_name = statusRight)]
    pub fn status_right(&self) -> String {
        self.shared.borrow().status_right.clone()
    }

    /// Give the game a file, such as a saved game stored earlier.
    #[wasm_bindgen(js_name = putFile)]
    pub fn put_file(&mut self, name: &str, data: &[u8]) {
        self.shared
            .borrow_mut()
            .files
            .insert(name.to_owned(), data.to_vec());
    }

    /// The contents of one of the game's files, if there is one by that
    /// name.
    #[wasm_bindgen(js_name = getFile)]
    pub fn get_file(&self, name: &str) -> Option<Vec<u8>> {
        self.shared.borrow().files.get(name).cloned()
    }

    /// The names of the files the game has written or deleted since this
    /// was last called. A name `getFile` has nothing for was deleted.
    #[wasm_bindgen(js_name = takeChangedFiles)]
    pub fn take_changed_files(&mut self) -> Vec<String> {
        std::mem::take(&mut self.shared.borrow_mut().changed)
    }
}
