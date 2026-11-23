//! What a terminal window adds to plain standard input and output: its
//! width, single keystrokes, a pause when a screenful of text has gone by
//! without the player having had a chance to read it, and banner windows,
//! such as a status line, kept in place at the top and bottom while the
//! main window's text scrolls between them.
//!
//! None of this applies when input or output is a file or a pipe, which is
//! how tests run.

use std::io::{IsTerminal as _, Write};

use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use crossterm::style::{Attribute, SetAttribute};
use crossterm::{cursor, execute, queue, terminal};
use hexicon_host::banner::{BannerInfo, BannerSpec, Placement, Rect, Screen, color};

/// What shows while output is paused.
const MORE_PROMPT: &str = "[More]";

/// Where a terminal's output goes, and what can be asked of it: the real
/// terminal, or a stand-in under test.
pub(crate) struct Device {
    out: Box<dyn Write>,
    /// The window's size, as lines and columns.
    size: Box<dyn Fn() -> (usize, usize)>,
    /// Where the cursor is, as column and line.
    cursor: Box<dyn FnMut() -> Option<(usize, usize)>>,
}

impl Device {
    /// The terminal that standard output is.
    fn real() -> Self {
        Self {
            out: Box::new(std::io::stdout()),
            size: Box::new(size),
            cursor: Box::new(|| {
                cursor::position()
                    .ok()
                    .map(|(x, y)| (usize::from(x), usize::from(y)))
            }),
        }
    }
}

/// A terminal that the program's console is.
pub(crate) struct Terminal {
    device: Device,
    /// Pause when a screenful has gone by.
    paging: bool,
    /// Lines displayed since the player last typed something.
    lines: usize,
    /// The banners, and the main window's place among them.
    screen: Screen,
    /// Whether the terminal understands the control sequences banners
    /// need. Without them there are none.
    banners: bool,
    /// The main window's area as last set on the terminal, if banners
    /// have ever made it less than the whole window.
    region: Option<Rect>,
    /// The old-style status line's two parts.
    status: (String, String),
}

/// A key the player pressed.
enum Key {
    /// A key, by the name programs know it by.
    Named(String),
    /// Input has ended.
    End,
}

impl Terminal {
    /// The terminal, if standard input and output are both one.
    pub(crate) fn detect(paging: bool) -> Option<Self> {
        if !(std::io::stdin().is_terminal() && std::io::stdout().is_terminal()) {
            return None;
        }
        Some(Self::new(
            Device::real(),
            paging,
            supports_control_sequences(),
        ))
    }

    fn new(device: Device, paging: bool, banners: bool) -> Self {
        let (rows, columns) = (device.size)();
        Self {
            device,
            paging,
            lines: 0,
            // The main window's text cannot be kept to part of the width.
            screen: Screen::new(rows, columns, false),
            banners,
            region: None,
            status: (String::new(), String::new()),
        }
    }

    /// The width of the main window in columns, which is the whole width
    /// of the terminal's.
    pub(crate) fn width(&mut self) -> Option<usize> {
        self.sync_size();
        Some(self.screen.main().width)
    }

    /// How many lines can go by before output pauses: the main window's
    /// height, less a line for the player's last command and one for the
    /// prompt.
    fn page_length(&self) -> Option<usize> {
        if !self.paging {
            return None;
        }
        Some(self.screen.main().height.saturating_sub(2).max(1))
    }

    /// The player has seen everything displayed so far.
    pub(crate) fn start_page(&mut self) {
        self.lines = 0;
    }

    /// Display text, pausing whenever a screenful has gone by.
    pub(crate) fn print(&mut self, text: &str) {
        for piece in text.split_inclusive('\n') {
            if let Some(length) = self.page_length() {
                if self.lines >= length {
                    let out = &mut self.device.out;
                    let _ = out.write_all(MORE_PROMPT.as_bytes());
                    let _ = out.flush();
                    let key = read_key(self.region.is_some());
                    // Rub the prompt out.
                    let blank = " ".repeat(MORE_PROMPT.len());
                    let _ = write!(out, "\r{blank}\r");
                    self.lines = match key {
                        // Enter shows one more line, and anything else
                        // the next screenful.
                        Key::Named(name) if name == "\n" => length - 1,
                        Key::Named(_) => 0,
                        Key::End => {
                            self.paging = false;
                            0
                        }
                    };
                }
            }
            let out = &mut self.device.out;
            let _ = out.write_all(piece.as_bytes());
            if piece.ends_with('\n') {
                self.lines += 1;
            }
        }
        let _ = self.device.out.flush();
    }

    /// Wait for the player to press a key: its name, or `None` when there
    /// is no more input.
    pub(crate) fn read_key(&mut self) -> Option<String> {
        let _ = self.device.out.flush();
        self.start_page();
        match read_key(self.region.is_some()) {
            Key::Named(name) => Some(name),
            Key::End => None,
        }
    }

    /// Clear the main window.
    pub(crate) fn clear(&mut self) {
        let _ = queue!(
            self.device.out,
            terminal::Clear(terminal::ClearType::All),
            cursor::MoveTo(0, 0)
        );
        if self.region.is_some() {
            let main = self.screen.main();
            let _ = queue!(self.device.out, cursor::MoveTo(0, main.y as u16));
            self.draw_all();
        }
        let _ = self.device.out.flush();
        self.start_page();
    }

    // ---- banners ----

    /// Whether banners can be shown.
    pub(crate) fn has_banners(&self) -> bool {
        self.banners
    }

    /// Change the banners, then bring the screen up to date.
    pub(crate) fn banners<T>(&mut self, change: impl FnOnce(&mut Screen) -> T) -> T {
        let result = change(&mut self.screen);
        self.refresh();
        result
    }

    /// Change what banners hold, to be drawn when the program flushes
    /// them or next waits for input, so that a banner rewritten in pieces
    /// is not drawn piece by piece.
    pub(crate) fn banners_later(&mut self, change: impl FnOnce(&mut Screen)) {
        change(&mut self.screen);
    }

    /// Draw whatever banners have changed.
    pub(crate) fn flush_banners(&mut self) {
        self.refresh();
    }

    /// Create a banner.
    pub(crate) fn create_banner(
        &mut self,
        parent: Option<u32>,
        placement: Placement,
        spec: BannerSpec,
    ) -> Option<u32> {
        if !self.banners {
            return None;
        }
        self.sync_size();
        self.banners(|screen| screen.create(parent, placement, spec))
    }

    /// What a program can learn about a banner.
    pub(crate) fn banner_info(&mut self, id: u32) -> Option<BannerInfo> {
        self.sync_size();
        self.screen.info(id)
    }

    /// Show the old-style status line: the left part, then the right.
    pub(crate) fn set_status(&mut self, left: Option<&str>, right: Option<&str>) {
        if !self.banners {
            return;
        }
        if let Some(left) = left {
            self.status.0 = left.to_owned();
        }
        if let Some(right) = right {
            self.status.1 = right.to_owned();
        }
        let (left, right) = self.status.clone();
        self.banners(|screen| screen.set_status(&left, &right));
    }

    /// Follow the terminal window to a new size.
    fn sync_size(&mut self) {
        let (rows, columns) = (self.device.size)();
        self.screen.resize(rows, columns);
        self.refresh();
    }

    /// Draw what has changed: everything, if the layout has.
    fn refresh(&mut self) {
        if self.screen.take_layout_changed() {
            self.screen.take_changed();
            self.apply_layout();
            self.draw_all();
        } else {
            for id in self.screen.take_changed() {
                self.draw(id);
            }
        }
        let _ = self.device.out.flush();
    }

    /// Fit the main window into what the banners leave of the screen.
    fn apply_layout(&mut self) {
        let (rows, columns) = self.screen.size();
        let mut main = self.screen.main();
        if main.height == 0 {
            // Banners have taken everything, but the player still needs a
            // line to type on: the bottom one, over whatever is there.
            main = Rect {
                y: rows.saturating_sub(1),
                height: 1,
                ..main
            };
        }
        let whole = Rect {
            x: 0,
            y: 0,
            width: columns,
            height: rows,
        };
        if self.region.is_none() && main == whole {
            return;
        }
        let old = self.region.unwrap_or(whole);
        let _ = self.device.out.flush();
        let (column, row) =
            (self.device.cursor)().unwrap_or((0, old.y + old.height.saturating_sub(1)));
        let bottom = main.y + main.height - 1;
        let mut row = row;
        let out = &mut self.device.out;
        // The text the player was reading stays in view: if the cursor is
        // now below the main window, everything above it moves up.
        if row > bottom {
            let lines = row - bottom;
            let _ = write!(out, "\x1b[{};{}r", old.y + 1, old.y + old.height);
            let _ = queue!(out, terminal::ScrollUp(lines as u16));
            row = bottom;
        }
        row = row.max(main.y);
        // Lines a banner has given back to the main window are cleared of
        // what it showed.
        for line in main.y..=bottom {
            if line < old.y || line >= old.y + old.height {
                let _ = queue!(
                    out,
                    cursor::MoveTo(0, line as u16),
                    terminal::Clear(terminal::ClearType::CurrentLine)
                );
            }
        }
        let _ = write!(out, "\x1b[{};{}r", main.y + 1, bottom + 1);
        let _ = queue!(out, cursor::MoveTo(column as u16, row as u16));
        self.region = Some(main);
    }

    fn draw_all(&mut self) {
        for (id, _) in self.screen.banners() {
            self.draw(id);
        }
    }

    /// Draw a banner where it is, leaving the cursor where it was.
    fn draw(&mut self, id: u32) {
        let Some(rect) = self.screen.rect(id) else {
            return;
        };
        if rect.width == 0 || rect.height == 0 {
            return;
        }
        let reverse = self.screen.screen_color(id) == Some(color::STATUS_BG);
        let out = &mut self.device.out;
        let _ = queue!(out, cursor::SavePosition);
        for (i, line) in self.screen.view(id).iter().enumerate() {
            let _ = queue!(out, cursor::MoveTo(rect.x as u16, (rect.y + i) as u16));
            if reverse {
                let _ = queue!(out, SetAttribute(Attribute::Reverse));
            }
            let _ = out.write_all(line.as_bytes());
            if reverse {
                let _ = queue!(out, SetAttribute(Attribute::Reset));
            }
        }
        let _ = queue!(out, cursor::RestorePosition);
    }

    /// Give the terminal back as it was, with the cursor below everything,
    /// for whatever runs next in it.
    fn restore(&mut self) {
        if self.region.take().is_some() {
            let (rows, _) = self.screen.size();
            let out = &mut self.device.out;
            let _ = write!(out, "\x1b[r");
            let _ = queue!(out, cursor::MoveTo(0, rows.saturating_sub(1) as u16));
            let _ = writeln!(out);
            let _ = out.flush();
        }
    }
}

impl Drop for Terminal {
    fn drop(&mut self) {
        self.restore();
    }
}

/// The terminal window's size, as lines and columns.
fn size() -> (usize, usize) {
    terminal::size().map_or((24, 80), |(columns, rows)| {
        (usize::from(rows), usize::from(columns))
    })
}

/// Whether the terminal acts on the control sequences that keep part of
/// the screen still. Windows consoles do once they are asked to.
fn supports_control_sequences() -> bool {
    #[cfg(windows)]
    {
        crossterm::ansi_support::supports_ansi()
    }
    #[cfg(not(windows))]
    {
        std::env::var("TERM").map_or(true, |term| term != "dumb")
    }
}

/// Wait for a key. Keys arrive one at a time only while the terminal is
/// in raw mode, which also stops it from showing them and from acting on
/// Ctrl+C, so that mode lasts no longer than the wait.
///
/// Ctrl+C stops the program, giving the terminal back as it was if banners
/// have kept part of it still.
fn read_key(banners: bool) -> Key {
    if terminal::enable_raw_mode().is_err() {
        return Key::End;
    }
    let key = loop {
        match crossterm::event::read() {
            Ok(Event::Key(event)) if event.kind != KeyEventKind::Release => {
                if interrupts(&event) {
                    let _ = terminal::disable_raw_mode();
                    if banners {
                        let (rows, _) = size();
                        print!("\x1b[r");
                        let _ = execute!(
                            std::io::stdout(),
                            cursor::MoveTo(0, rows.saturating_sub(1) as u16)
                        );
                    }
                    println!();
                    // The exit status of a program stopped by Ctrl+C.
                    std::process::exit(130);
                }
                if let Some(name) = key_name(&event) {
                    break Key::Named(name);
                }
            }
            Ok(_) => {}
            Err(_) => break Key::End,
        }
    };
    let _ = terminal::disable_raw_mode();
    key
}

/// Whether a key is Ctrl+C, which stops the program.
fn interrupts(event: &KeyEvent) -> bool {
    event.modifiers.contains(KeyModifiers::CONTROL)
        && matches!(event.code, KeyCode::Char('c' | 'C'))
}

/// The name programs know a key by: the character it types, a newline
/// for Enter, or a name in square brackets. Keys with no name, such as
/// Shift on its own, are `None`.
fn key_name(event: &KeyEvent) -> Option<String> {
    let control = event.modifiers.contains(KeyModifiers::CONTROL);
    let alt = event.modifiers.contains(KeyModifiers::ALT);
    let name = match event.code {
        // AltGr, which types characters on many keyboards, arrives as
        // Ctrl and Alt together, and is not a command.
        KeyCode::Char(c) if control && alt => c.to_string(),
        KeyCode::Char(c) if control && c.is_ascii_alphabetic() => {
            format!("[ctrl-{}]", c.to_ascii_lowercase())
        }
        KeyCode::Char(c) if alt && c.is_ascii_alphanumeric() => {
            format!("[alt-{}]", c.to_ascii_lowercase())
        }
        KeyCode::Char(c) => c.to_string(),
        KeyCode::Enter => "\n".to_owned(),
        KeyCode::Tab => "\t".to_owned(),
        KeyCode::Esc => "[esc]".to_owned(),
        KeyCode::Backspace => "[bksp]".to_owned(),
        KeyCode::Delete => "[del]".to_owned(),
        KeyCode::Up => "[up]".to_owned(),
        KeyCode::Down => "[down]".to_owned(),
        KeyCode::Left => "[left]".to_owned(),
        KeyCode::Right => "[right]".to_owned(),
        KeyCode::Home => "[home]".to_owned(),
        KeyCode::End => "[end]".to_owned(),
        KeyCode::PageUp => "[page up]".to_owned(),
        KeyCode::PageDown => "[page down]".to_owned(),
        KeyCode::Insert => "[insert]".to_owned(),
        KeyCode::F(number) => format!("[f{number}]"),
        _ => return None,
    };
    Some(name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::rc::Rc;

    use hexicon_host::banner::{align, kind, units};

    /// Output collected for a stand-in terminal.
    #[derive(Clone, Default)]
    struct Shared(Rc<RefCell<Vec<u8>>>);

    impl Write for Shared {
        fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
            self.0.borrow_mut().extend_from_slice(data);
            Ok(data.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    /// Just enough of a terminal to show what the control sequences the
    /// terminal sends do: cursor movement, scroll regions, scrolling,
    /// clearing, and saving and restoring the cursor.
    struct Vt {
        grid: Vec<Vec<char>>,
        x: usize,
        y: usize,
        top: usize,
        bottom: usize,
        saved: (usize, usize),
    }

    impl Vt {
        fn new(rows: usize, columns: usize) -> Self {
            Self {
                grid: vec![vec![' '; columns]; rows],
                x: 0,
                y: 0,
                top: 0,
                bottom: rows - 1,
                saved: (0, 0),
            }
        }

        fn scroll_up(&mut self, lines: usize) {
            for _ in 0..lines {
                self.grid.remove(self.top);
                let columns = self.grid[0].len();
                self.grid.insert(self.bottom, vec![' '; columns]);
            }
        }

        fn run(&mut self, bytes: &[u8]) {
            let text = String::from_utf8_lossy(bytes);
            let mut chars = text.chars().peekable();
            while let Some(c) = chars.next() {
                match c {
                    '\x1b' => match chars.next() {
                        Some('7') => self.saved = (self.x, self.y),
                        Some('8') => (self.x, self.y) = self.saved,
                        Some('[') => {
                            let mut params = String::new();
                            let mut end = ' ';
                            for c in chars.by_ref() {
                                if c.is_ascii_digit() || c == ';' {
                                    params.push(c);
                                } else {
                                    end = c;
                                    break;
                                }
                            }
                            let n: Vec<usize> =
                                params.split(';').map(|p| p.parse().unwrap_or(0)).collect();
                            let arg = |i: usize, default: usize| match n.get(i) {
                                Some(&0) | None => default,
                                Some(&v) => v,
                            };
                            match end {
                                'H' => {
                                    self.y = arg(0, 1) - 1;
                                    self.x = arg(1, 1) - 1;
                                }
                                'r' => {
                                    self.top = arg(0, 1) - 1;
                                    self.bottom = arg(1, self.grid.len()) - 1;
                                    (self.x, self.y) = (0, 0);
                                }
                                'S' => self.scroll_up(arg(0, 1)),
                                'K' => self.grid[self.y].fill(' '),
                                'J' => self.grid.iter_mut().for_each(|line| line.fill(' ')),
                                _ => {}
                            }
                        }
                        _ => {}
                    },
                    '\r' => self.x = 0,
                    // A Windows console, like most, returns to the start
                    // of the line at a newline.
                    '\n' => {
                        self.x = 0;
                        if self.y == self.bottom {
                            self.scroll_up(1);
                        } else if self.y + 1 < self.grid.len() {
                            self.y += 1;
                        }
                    }
                    c => {
                        if let Some(cell) = self.grid[self.y].get_mut(self.x) {
                            *cell = c;
                        }
                        self.x += 1;
                    }
                }
            }
        }

        fn lines(&self) -> Vec<String> {
            self.grid
                .iter()
                .map(|line| line.iter().collect::<String>().trim_end().to_owned())
                .collect()
        }
    }

    fn stand_in(rows: usize, columns: usize) -> (Terminal, Shared) {
        let out = Shared::default();
        let seen = out.clone();
        let device = Device {
            out: Box::new(out.clone()),
            size: Box::new(move || (rows, columns)),
            cursor: Box::new(move || {
                let mut vt = Vt::new(rows, columns);
                vt.run(&seen.0.borrow());
                Some((vt.x, vt.y))
            }),
        };
        (Terminal::new(device, false, true), out)
    }

    fn screen_of(out: &Shared, rows: usize, columns: usize) -> Vec<String> {
        let mut vt = Vt::new(rows, columns);
        vt.run(&out.0.borrow());
        vt.lines()
    }

    #[test]
    fn a_status_banner_stays_while_text_scrolls_below_it() {
        let (mut terminal, out) = stand_in(6, 20);
        for n in 1..=6 {
            terminal.print(&format!("line {n}\n"));
        }
        let spec = BannerSpec {
            kind: kind::TEXT,
            align: align::TOP,
            size: 0,
            units: units::CELLS,
            style: 0,
        };
        let id = terminal
            .create_banner(None, Placement::First, spec)
            .unwrap();
        terminal.banners(|screen| {
            screen.write(id, "Kitchen\n");
            screen.size_to_contents(id);
        });
        assert_eq!(
            screen_of(&out, 6, 20),
            ["Kitchen", "line 3", "line 4", "line 5", "line 6", ""]
        );
        terminal.print("line 7\nline 8\n");
        assert_eq!(
            screen_of(&out, 6, 20),
            ["Kitchen", "line 5", "line 6", "line 7", "line 8", ""]
        );
        // The banner's new text replaces the old.
        terminal.banners_later(|screen| {
            screen.clear(id);
            screen.write(id, "Hall\n");
        });
        assert_eq!(
            screen_of(&out, 6, 20)[0],
            "Kitchen",
            "not drawn until flushed"
        );
        terminal.flush_banners();
        assert_eq!(screen_of(&out, 6, 20)[0], "Hall");

        // Deleted, it gives its line back.
        terminal.banners(|screen| screen.delete(id));
        assert_eq!(screen_of(&out, 6, 20)[0], "");
        terminal.print("line 9\n");
        drop(terminal);
        let lines = screen_of(&out, 6, 20);
        assert!(lines.contains(&"line 9".to_owned()), "{lines:?}");
    }

    #[test]
    fn a_bottom_banner_pushes_the_text_up() {
        let (mut terminal, out) = stand_in(5, 20);
        for n in 1..=5 {
            terminal.print(&format!("line {n}\n"));
        }
        let spec = BannerSpec {
            kind: kind::TEXT,
            align: align::BOTTOM,
            size: 1,
            units: units::CELLS,
            style: 0,
        };
        let id = terminal.create_banner(None, Placement::Last, spec).unwrap();
        terminal.banners(|screen| screen.write(id, "Exits: north"));
        // The cursor was on the last line, under the new banner, so the
        // text moved up to keep it in view.
        assert_eq!(
            screen_of(&out, 5, 20),
            ["line 3", "line 4", "line 5", "", "Exits: north"]
        );
    }

    #[test]
    fn the_old_status_line_is_shown_at_the_top() {
        let (mut terminal, out) = stand_in(4, 20);
        terminal.print("Welcome.\n");
        terminal.set_status(Some("Cellar"), None);
        terminal.set_status(None, Some("3/10"));
        assert_eq!(screen_of(&out, 4, 20)[0], " Cellar        3/10");
    }

    fn name(code: KeyCode, modifiers: KeyModifiers) -> Option<String> {
        key_name(&KeyEvent::new(code, modifiers))
    }

    #[test]
    fn keys_go_by_the_names_programs_know() {
        let none = KeyModifiers::NONE;
        assert_eq!(name(KeyCode::Char('a'), none).as_deref(), Some("a"));
        assert_eq!(
            name(KeyCode::Char('A'), KeyModifiers::SHIFT).as_deref(),
            Some("A")
        );
        assert_eq!(name(KeyCode::Enter, none).as_deref(), Some("\n"));
        assert_eq!(name(KeyCode::PageUp, none).as_deref(), Some("[page up]"));
        assert_eq!(name(KeyCode::F(5), none).as_deref(), Some("[f5]"));
        assert_eq!(
            name(KeyCode::Char('x'), KeyModifiers::CONTROL).as_deref(),
            Some("[ctrl-x]")
        );
        assert_eq!(
            name(KeyCode::Char('x'), KeyModifiers::ALT).as_deref(),
            Some("[alt-x]")
        );
        assert_eq!(
            name(
                KeyCode::Char('@'),
                KeyModifiers::CONTROL | KeyModifiers::ALT
            )
            .as_deref(),
            Some("@")
        );
        assert_eq!(name(KeyCode::CapsLock, none), None);
    }

    #[test]
    fn only_control_c_interrupts() {
        let event = |code, modifiers| KeyEvent::new(code, modifiers);
        assert!(interrupts(&event(
            KeyCode::Char('c'),
            KeyModifiers::CONTROL
        )));
        assert!(!interrupts(&event(KeyCode::Char('c'), KeyModifiers::NONE)));
        assert!(!interrupts(&event(
            KeyCode::Char('x'),
            KeyModifiers::CONTROL
        )));
    }
}
