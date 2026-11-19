//! What a terminal window adds to plain standard input and output: its
//! width, single keystrokes, and a pause when a screenful of text has gone
//! by without the player having had a chance to read it.
//!
//! None of this applies when input or output is a file or a pipe, which is
//! how tests run.

use std::io::{IsTerminal as _, Write as _};

use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use crossterm::{cursor, execute, terminal};

/// What shows while output is paused.
const MORE_PROMPT: &str = "[More]";

/// A terminal that the program's console is.
pub(crate) struct Terminal {
    /// Pause when a screenful has gone by.
    paging: bool,
    /// Lines displayed since the player last typed something.
    lines: usize,
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
        (std::io::stdin().is_terminal() && std::io::stdout().is_terminal())
            .then_some(Self { paging, lines: 0 })
    }

    /// The width of the window in columns.
    pub(crate) fn width(&self) -> Option<usize> {
        terminal::size().ok().map(|(columns, _)| columns.into())
    }

    /// How many lines can go by before output pauses: a screenful, less
    /// a line for the player's last command and one for the prompt.
    fn page_length(&self) -> Option<usize> {
        if !self.paging {
            return None;
        }
        let (_, rows) = terminal::size().ok()?;
        Some(usize::from(rows).saturating_sub(2).max(1))
    }

    /// The player has seen everything displayed so far.
    pub(crate) fn start_page(&mut self) {
        self.lines = 0;
    }

    /// Display text, pausing whenever a screenful has gone by.
    pub(crate) fn print(&mut self, text: &str) {
        let mut out = std::io::stdout().lock();
        for piece in text.split_inclusive('\n') {
            if let Some(length) = self.page_length() {
                if self.lines >= length {
                    let _ = out.write_all(MORE_PROMPT.as_bytes());
                    let _ = out.flush();
                    let key = read_key();
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
            let _ = out.write_all(piece.as_bytes());
            if piece.ends_with('\n') {
                self.lines += 1;
            }
        }
    }

    /// Wait for the player to press a key: its name, or `None` when there
    /// is no more input.
    pub(crate) fn read_key(&mut self) -> Option<String> {
        let _ = std::io::stdout().flush();
        self.start_page();
        match read_key() {
            Key::Named(name) => Some(name),
            Key::End => None,
        }
    }

    /// Clear the window.
    pub(crate) fn clear(&mut self) {
        let _ = execute!(
            std::io::stdout(),
            terminal::Clear(terminal::ClearType::All),
            cursor::MoveTo(0, 0)
        );
        self.start_page();
    }
}

/// Wait for a key. Keys arrive one at a time only while the terminal is
/// in raw mode, which also stops it from showing them and from acting on
/// Ctrl+C, so that mode lasts no longer than the wait.
fn read_key() -> Key {
    if terminal::enable_raw_mode().is_err() {
        return Key::End;
    }
    let key = loop {
        match crossterm::event::read() {
            Ok(Event::Key(event)) if event.kind != KeyEventKind::Release => {
                if interrupts(&event) {
                    let _ = terminal::disable_raw_mode();
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
