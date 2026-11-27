//! Hexicon for web browsers: a game that a page's JavaScript runs.
//!
//! A page cannot wait for the player without freezing, so a game runs
//! until it needs something from the page and then stops, saying what it
//! needs. The page supplies it and tells the game to carry on:
//!
//! ```js
//! const game = new Game(imageBytes, "game.hxg", timeZoneName);
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

use hexicon_host::pane::{self, PaneInfo, PaneSpec, Placement, Screen};
use hexicon_host::rich::{self, Document, Event, Item, RichDisplay};
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
    /// How many columns of text the page has room for, if it has said.
    width: Option<usize>,
    /// The panes, laid out on a screen as big as the page.
    screen: Screen,
    /// On a rich display, what it can do and the player's choices, and the
    /// document the game's markup makes, the main window's and the panes'.
    rich: Option<RichDisplay>,
    document: Document,
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

    fn rich_display(&self) -> Option<RichDisplay> {
        self.0.borrow().rich
    }

    fn print_markup(&mut self, markup: &str) {
        self.0.borrow_mut().document.write(markup);
    }

    fn print_input(&mut self, text: &str) {
        self.0.borrow_mut().document.input(text);
    }

    fn pane_write_markup(&mut self, id: u32, markup: &str) {
        self.0.borrow_mut().document.pane_write(id, markup);
    }

    fn pane_flush(&mut self, _id: u32) {
        let mut shared = self.0.borrow_mut();
        if shared.rich.is_some() {
            shared.document.pane_flush();
        }
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

    fn console_width(&mut self) -> Option<usize> {
        self.0.borrow().width
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

    fn has_panes(&self) -> bool {
        true
    }

    fn pane_create(
        &mut self,
        parent: Option<u32>,
        placement: Placement,
        spec: PaneSpec,
    ) -> Option<u32> {
        let mut shared = self.0.borrow_mut();
        let id = shared.screen.create(parent, placement, spec)?;
        if shared.rich.is_some() {
            shared.document.pane_created(id);
        }
        Some(id)
    }

    fn pane_delete(&mut self, id: u32) {
        let mut shared = self.0.borrow_mut();
        shared.screen.delete(id);
        if shared.rich.is_some() {
            shared.document.pane_deleted(id);
        }
    }

    fn pane_clear(&mut self, id: u32) {
        let mut shared = self.0.borrow_mut();
        shared.screen.clear(id);
        if shared.rich.is_some() {
            shared.document.pane_cleared(id);
        }
    }

    fn pane_write(&mut self, id: u32, text: &str) {
        self.0.borrow_mut().screen.write(id, text);
    }

    fn pane_set_size(&mut self, id: u32, size: i32, units: i32, advisory: bool) {
        self.0
            .borrow_mut()
            .screen
            .set_size(id, size, units, advisory);
    }

    fn pane_size_to_contents(&mut self, id: u32) {
        self.0.borrow_mut().screen.size_to_contents(id);
    }

    fn pane_go_to(&mut self, id: u32, row: usize, column: usize) {
        self.0.borrow_mut().screen.go_to(id, row, column);
    }

    fn pane_info(&mut self, id: u32) -> Option<PaneInfo> {
        self.0.borrow().screen.info(id)
    }

    fn pane_set_screen_color(&mut self, id: u32, color: u32) {
        self.0.borrow_mut().screen.set_screen_color(id, color);
    }
}

/// A string as JSON writes it.
fn json_string(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c if u32::from(c) < 0x20 => out.push_str(&format!("\\u{:04x}", u32::from(c))),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// A game loaded and ready to play.
#[wasm_bindgen]
pub struct Game {
    vm: Vm,
    shared: Rc<RefCell<Shared>>,
    name: String,
    started: bool,
    finished: bool,
    /// The image, for the resources in it.
    image: Vec<u8>,
    /// Each resource's name, and where it is in the image, in order.
    resources: Vec<(String, std::ops::Range<usize>)>,
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
            width: None,
            screen: Screen::new(24, 81, false),
            rich: None,
            document: Document::new(),
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
        let resources = parsed
            .resources
            .iter()
            .map(|resource| {
                let end = resource.offset + resource.size as usize;
                (resource.name.clone(), resource.offset..end)
            })
            .collect();
        Ok(Game {
            vm,
            shared,
            name: name.to_owned(),
            started: false,
            finished: false,
            image: image.to_vec(),
            resources,
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
            if shared.rich.is_some() {
                // Shown as it is, not as markup.
                let text = message.replace('&', "&amp;").replace('<', "&lt;");
                shared.document.write(&text);
                shared.document.write("\n");
            } else {
                shared.output.push_str(&message);
                shared.output.push('\n');
            }
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

    /// Say how many columns of text the page has room for. The game's
    /// text is broken into lines no longer than that. The page calls
    /// this before starting the game and whenever its size changes; a new
    /// width applies to text displayed after the game next waits for
    /// input. Without it, lines are up to 79 characters.
    #[wasm_bindgen(js_name = setWidth)]
    pub fn set_width(&mut self, columns: u32) {
        // The VM keeps the last column of its width free, in panes as
        // in the main window.
        let mut shared = self.shared.borrow_mut();
        shared.width = Some(columns as usize + 1);
        let (rows, _) = shared.screen.size();
        shared.screen.resize(rows, columns as usize + 1);
    }

    /// Say how many lines of text the page has room for, panes and the
    /// main window together. Panes take their lines from these.
    #[wasm_bindgen(js_name = setHeight)]
    pub fn set_height(&mut self, rows: u32) {
        let mut shared = self.shared.borrow_mut();
        let (_, columns) = shared.screen.size();
        shared.screen.resize(rows as usize, columns);
    }

    /// The panes and what they show, as JSON:
    /// `{"main": {"y", "height"}, "panes": [{"x", "y", "width",
    /// "height", "status", "lines"}]}`, in lines and columns of the
    /// screen. `status` is whether the pane has the status line's
    /// colors.
    pub fn panes(&self) -> String {
        let shared = self.shared.borrow();
        let screen = &shared.screen;
        let main = screen.main();
        let mut out = format!(
            "{{\"main\":{{\"y\":{},\"height\":{}}},\"panes\":[",
            main.y, main.height
        );
        for (index, (id, rect)) in screen.panes().into_iter().enumerate() {
            if index > 0 {
                out.push(',');
            }
            let status = screen.screen_color(id) == Some(pane::color::STATUS_BG);
            let lines: Vec<String> = screen
                .view(id)
                .iter()
                .map(|line| json_string(line))
                .collect();
            out.push_str(&format!(
                "{{\"id\":{id},\"x\":{},\"y\":{},\"width\":{},\"height\":{},\"status\":{},\"lines\":[{}]}}",
                rect.x,
                rect.y,
                rect.width,
                rect.height,
                status,
                lines.join(",")
            ));
        }
        out.push_str("]}");
        out
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

    // ---- the rich display ----

    /// Make the page a rich display, before the game starts: one that
    /// shows the game's markup, checked against Hexicon's profile, and
    /// gets it from `takeDocument` instead of `takeOutput`. The rest say
    /// what the player has chosen; they can be changed with
    /// `setPlayerChoices`.
    #[wasm_bindgen(js_name = setRichDisplay)]
    pub fn set_rich_display(
        &mut self,
        show_pictures: bool,
        play_sounds: bool,
        play_music: bool,
        links: bool,
    ) {
        self.shared.borrow_mut().rich = Some(RichDisplay {
            pictures: true,
            sounds: true,
            show_pictures,
            play_sounds,
            play_music,
            links,
        });
    }

    /// Change what the player has chosen. The game hears of it the next
    /// time it asks (`systemInfo`).
    #[wasm_bindgen(js_name = setPlayerChoices)]
    pub fn set_player_choices(
        &mut self,
        show_pictures: bool,
        play_sounds: bool,
        play_music: bool,
        links: bool,
    ) {
        if let Some(display) = &mut self.shared.borrow_mut().rich {
            display.show_pictures = show_pictures;
            display.play_sounds = play_sounds;
            display.play_music = play_music;
            display.links = links;
        }
    }

    /// The checked document since this was last called, as JSON: a list
    /// of items for the main window, each a list, and of the panes'.
    /// `["t", text]` is text; `["o", name, attrs]` opens an element and
    /// `["c", name]` closes the one opened last; `["v", name, attrs]` is an
    /// element that holds nothing; `["title", text]` is the window's
    /// title; and `["in", text]` a line the player typed, from a script.
    /// `attrs` is a list of `[name, value]`, with `null` for an attribute
    /// without a value. `["pane", id, items]` is what a text pane is to
    /// show next, as a list of items; and `["pane-create", id]`,
    /// `["pane-delete", id]`, and `["pane-clear", id]` say a pane was made,
    /// removed, or emptied (see `panes` for where each is).
    #[wasm_bindgen(js_name = takeDocument)]
    pub fn take_document(&mut self) -> String {
        let events = self.shared.borrow_mut().document.take();
        let list: Vec<String> = events.iter().map(event_json).collect();
        format!("[{}]", list.join(","))
    }

    /// The document since this was last called, as the profile's markup,
    /// one line to each line break, block, and line typed, and a pane's as
    /// `<pane id="1">...</pane>`: what `hexicon run --rich` writes, for
    /// tests.
    #[wasm_bindgen(js_name = takeDocumentText)]
    pub fn take_document_text(&mut self) -> String {
        self.shared.borrow_mut().document.take_text()
    }

    /// The contents of one of the game's resources: a picture, a sound,
    /// a font, a style sheet.
    pub fn resource(&self, name: &str) -> Option<Vec<u8>> {
        let (_, range) = self.resources.iter().find(|(n, _)| n == name)?;
        self.image.get(range.clone()).map(<[u8]>::to_vec)
    }

    /// The game's style sheets, checked against the profile and put under
    /// `scope` (the page's element for the game), one after another: every
    /// resource whose name ends in ".css", in order.
    #[wasm_bindgen(js_name = styleSheets)]
    pub fn style_sheets(&self, scope: &str) -> String {
        let mut out = String::new();
        for (name, range) in &self.resources {
            if !name.to_ascii_lowercase().ends_with(".css") {
                continue;
            }
            if let Some(bytes) = self.image.get(range.clone()) {
                out.push_str(&rich::check_stylesheet(
                    &String::from_utf8_lossy(bytes),
                    scope,
                ));
            }
        }
        out
    }
}

/// An event of the document as JSON.
fn event_json(event: &Event) -> String {
    match event {
        Event::Main(item) => item_json(item),
        Event::PaneCreated(id) => format!("[\"pane-create\",{id}]"),
        Event::PaneDeleted(id) => format!("[\"pane-delete\",{id}]"),
        Event::PaneCleared(id) => format!("[\"pane-clear\",{id}]"),
        Event::Pane(id, items) => {
            let list: Vec<String> = items.iter().map(item_json).collect();
            format!("[\"pane\",{id},[{}]]", list.join(","))
        }
    }
}

/// An item of the document as JSON.
fn item_json(item: &Item) -> String {
    let attrs = |attrs: &[rich::Attr]| {
        let list: Vec<String> = attrs
            .iter()
            .map(|(name, value)| {
                format!(
                    "[{},{}]",
                    json_string(name),
                    value
                        .as_deref()
                        .map_or_else(|| "null".to_owned(), json_string)
                )
            })
            .collect();
        format!("[{}]", list.join(","))
    };
    match item {
        Item::Text(text) => format!("[\"t\",{}]", json_string(text)),
        Item::Open(name, a) => format!("[\"o\",{},{}]", json_string(name), attrs(a)),
        Item::Close(name) => format!("[\"c\",{}]", json_string(name)),
        Item::Void(name, a) => format!("[\"v\",{},{}]", json_string(name), attrs(a)),
        Item::Title(text) => format!("[\"title\",{}]", json_string(text)),
        Item::Input(text) => format!("[\"in\",{}]", json_string(text)),
    }
}
