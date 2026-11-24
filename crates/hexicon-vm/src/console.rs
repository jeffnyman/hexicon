//! The console formatter: turns the text a program displays into lines.
//!
//! Programs write a stream of text with embedded control characters and a
//! small subset of HTML. The formatter collapses runs of spaces and blank
//! lines, applies capitalization requests, expands tabs, interprets the
//! HTML it understands, and wraps lines to the console width.
//!
//! This is a port of the reference VM's text-only formatter.
//! Transcripts must match the
//! reference byte for byte, so the logic follows the original closely,
//! quirks included. Colors and text attributes are parsed and ignored.

use std::collections::HashMap;

use crate::entities;

/// The character after which a line must not be broken.
const FLAG_NOBREAK: u8 = 0x01;
/// The character after which a line may be broken.
const FLAG_OKBREAK: u8 = 0x02;
/// The line may be broken anywhere around this character.
const FLAG_BREAK_ANY: u8 = 0x04;
/// A soft hyphen follows this character.
const FLAG_SHY: u8 = 0x08;
/// The character is a space that must not be collapsed.
const FLAG_QSPACE: u8 = 0x10;

/// How to end a line when flushing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Newline {
    /// Write what is buffered and stay on the line.
    None,
    /// Write what is buffered and start a new line.
    Line,
    /// Write what is buffered, spaces at the end included, because the
    /// player is about to type on the same line.
    Input,
}

/// Where the HTML parser is within a tag.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ParseState {
    /// Not in a tag.
    Normal,
    /// In a tag, between attributes.
    Tag,
    /// In a double-quoted string in a tag.
    DoubleQuote,
    /// In a single-quoted string in a tag.
    SingleQuote,
    /// In an attribute name.
    AttrName,
    /// In an attribute value.
    AttrValue,
}

/// What a `<br>` tag does once its attributes have been read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DeferredBreak {
    None,
    /// End the current line.
    Flush,
    /// Write a blank line.
    Blank,
}

/// Alignment of a `<tab>`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TabAlign {
    None,
    Left,
    Right,
    Center,
    Decimal,
}

/// The text being formatted, read one character at a time.
struct Source {
    chars: Vec<char>,
    pos: usize,
}

impl Source {
    /// The next character, or NUL at the end. A NUL in the text reads as
    /// a space.
    fn next(&mut self) -> char {
        match self.chars.get(self.pos) {
            Some(&c) => {
                self.pos += 1;
                if c == '\0' { ' ' } else { c }
            }
            None => '\0',
        }
    }
}

fn is_xdigit(c: char) -> bool {
    c.is_ascii_hexdigit()
}

/// Parse a decimal number the way the reference formatter does.
fn to_int(text: &str) -> i32 {
    let text = text.trim_start();
    let (negative, digits) = match text.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, text.strip_prefix('+').unwrap_or(text)),
    };
    let mut acc: i64 = 0;
    for c in digits.chars() {
        let Some(digit) = c.to_digit(10) else { break };
        acc = (acc * 10 + i64::from(digit)).min(i64::from(i32::MAX));
    }
    (if negative { -acc } else { acc }) as i32
}

/// Whether a color attribute value is one the formatter accepts. Colors
/// have no effect on a plain console, so only validity matters.
fn is_color(_value: &str) -> bool {
    true
}

/// A color as programs give it to the host: one of the colors the host
/// chooses, such as the status line's background, or `#rrggbb`.
fn color_value(value: &str) -> Option<u32> {
    let value = value.trim().to_lowercase();
    Some(match value.as_str() {
        "text" => 0x0200_0000,
        "bgcolor" => 0x0300_0000,
        "statustext" => 0x0400_0000,
        "statusbg" => 0x0500_0000,
        "input" => 0x0600_0000,
        _ => u32::from_str_radix(value.strip_prefix('#')?, 16).ok()? & 0x00ff_ffff,
    })
}

/// The width of a text grid's formatter, which no line reaches: the grid
/// itself does not wrap.
const GRID_WIDTH: usize = 1 << 16;

/// The narrowest a console can be. A host that reports less, as a window
/// dragged down to nothing would, gets this.
const MIN_WIDTH: usize = 20;

/// The console formatter.
#[derive(Debug)]
pub(crate) struct Formatter {
    /// The current line, not yet written.
    line: Vec<char>,
    /// Flags for each character of `line`.
    flags: Vec<u8>,
    /// The column the next character goes in. This can run ahead of
    /// `line.len()` when spaces at the end of a full line are dropped.
    column: usize,
    /// The console width in columns.
    width: usize,
    /// The last thing written was a newline.
    just_did_newline: bool,
    /// Capitalize the next letter.
    caps: bool,
    /// Make the next letter lower-case.
    nocaps: bool,
    /// Flags applied to each character buffered.
    cur_flags: u8,
    /// Nesting depth of `<pre>`.
    pre_level: u32,
    /// Markup is text like any other, as in a text grid.
    literal: bool,
    /// Every space counts, as in a text grid.
    obey_whitespace: bool,
    /// A background color asked for with `<body>`, not yet passed on.
    body_color: Option<u32>,

    // HTML parsing state. Text can end in the middle of a tag and resume
    // in the next call, so all of this persists.
    parse_state: ParseState,
    /// Nesting depth of tags whose contents are not displayed.
    ignore_level: i32,
    title_level: i32,
    quote_level: i32,
    deferred_break: DeferredBreak,
    allow_alt: bool,
    allow_color: bool,
    in_body: bool,
    in_tab: bool,
    in_wrap: bool,
    attr_name: String,
    attr_value: String,
    attr_quote: Option<char>,

    // Tab stops.
    tabs: HashMap<String, usize>,
    new_tab_column: Option<usize>,
    new_tab_align: TabAlign,
    new_tab_decimal: char,
    pending_tab_align: TabAlign,
    pending_tab_column: Option<usize>,
    pending_tab_start: usize,
    pending_tab_decimal: char,

    /// Finished output, waiting to be handed to the host.
    out: String,
}

impl Formatter {
    /// A formatter for a console of the given width.
    pub(crate) fn new(width: usize) -> Self {
        Self {
            line: Vec::new(),
            flags: Vec::new(),
            column: 0,
            width,
            just_did_newline: false,
            caps: false,
            nocaps: false,
            cur_flags: 0,
            pre_level: 0,
            literal: false,
            obey_whitespace: false,
            body_color: None,
            parse_state: ParseState::Normal,
            ignore_level: 0,
            title_level: 0,
            quote_level: 0,
            deferred_break: DeferredBreak::None,
            allow_alt: false,
            allow_color: false,
            in_body: false,
            in_tab: false,
            in_wrap: false,
            attr_name: String::new(),
            attr_value: String::new(),
            attr_quote: None,
            tabs: HashMap::new(),
            new_tab_column: None,
            new_tab_align: TabAlign::None,
            new_tab_decimal: '\0',
            pending_tab_align: TabAlign::None,
            pending_tab_column: None,
            pending_tab_start: 0,
            pending_tab_decimal: '\0',
            out: String::new(),
        }
    }

    /// A formatter for a text grid, as the reference makes one for a grid
    /// banner: lines are never broken, markup is shown as it is, and every
    /// space counts.
    pub(crate) fn grid() -> Self {
        let mut formatter = Self::new(GRID_WIDTH);
        formatter.literal = true;
        formatter.obey_whitespace = true;
        formatter
    }

    /// Change the console width. Text already on the current line stays
    /// there; the new width applies from the next character.
    pub(crate) fn set_width(&mut self, width: usize) {
        self.width = width.max(MIN_WIDTH);
    }

    /// Take a background color asked for with `<body>` since this was
    /// last asked.
    pub(crate) fn take_body_color(&mut self) -> Option<u32> {
        self.body_color.take()
    }

    /// Take the output produced so far.
    pub(crate) fn take_output(&mut self) -> String {
        std::mem::take(&mut self.out)
    }

    /// Add text to the output as it is, past the formatting.
    pub(crate) fn write_raw(&mut self, text: &str) {
        self.out.push_str(text);
    }

    /// The player finished a line of input, which leaves the cursor at the
    /// start of a new line.
    pub(crate) fn note_input_line(&mut self) {
        self.column = 0;
        self.just_did_newline = true;
    }

    /// Forget the unfinished line, as when the screen is cleared.
    pub(crate) fn empty_buffers(&mut self) {
        self.line.clear();
        self.flags.clear();
        self.column = 0;
        self.just_did_newline = false;
        self.pending_tab_align = TabAlign::None;
    }

    // ---- writing lines ----

    /// Write finished text.
    fn write_text(&mut self, text: &[char], newline: bool) {
        for &c in text {
            // A non-breaking space has done its job by now.
            self.out.push(if c == '\u{a0}' { ' ' } else { c });
        }
        if newline {
            self.out.push('\n');
        }
    }

    /// Write out the buffered line.
    pub(crate) fn flush(&mut self, newline: Newline) {
        self.expand_pending_tab(newline == Newline::Line);

        // Leave off spaces at the end of the line, unless they are marked
        // as not to be separated from what follows.
        let mut count = self.line.len();
        while newline != Newline::Input && count > 0 && self.line[count - 1] == ' ' {
            if self.flags[count - 1] & FLAG_NOBREAK != 0 {
                break;
            }
            count -= 1;
        }
        if newline == Newline::Line {
            self.line.truncate(count);
            self.flags.truncate(count);
        }

        // A newline right after a newline adds nothing: blank lines have
        // to be asked for explicitly.
        let write_newline = newline == Newline::Line
            && (self.column != 0 || !self.just_did_newline || self.pre_level > 0);

        if count != 0 || (self.column != 0 && !self.just_did_newline) || self.pre_level > 0 {
            let text: Vec<char> = self.line[..count].to_vec();
            self.write_text(&text, write_newline);
        }
        if newline == Newline::Line {
            self.column = 0;
        }
        self.line.drain(..count);
        self.flags.drain(..count);

        if newline == Newline::Line {
            self.just_did_newline = true;
        } else if count != 0 {
            self.just_did_newline = false;
        }
    }

    /// End the current line and leave a blank line after it.
    fn write_blank_line(&mut self) {
        self.flush(Newline::Line);
        self.write_text(&[], true);
    }

    /// Add spaces: `indent` of them, or up to the next multiple of
    /// `multiple` columns.
    fn write_tab(&mut self, indent: i32, multiple: usize) {
        if multiple != 0 {
            loop {
                if self.column >= self.width {
                    break;
                }
                self.line.push(' ');
                self.flags.push(self.cur_flags);
                self.column += 1;
                if (self.column + 1) % multiple == 0 {
                    break;
                }
            }
        } else {
            let mut remaining = indent;
            while remaining > 0 && self.column < self.width {
                self.line.push(' ');
                self.flags.push(self.cur_flags);
                self.column += 1;
                remaining -= 1;
            }
        }
    }

    /// Insert the spaces for a right-aligned, centered, or decimal tab,
    /// now that the text it positions is known.
    fn expand_pending_tab(&mut self, allow_anonymous: bool) {
        let start = self.pending_tab_start;
        let spaces: i64 = match self.pending_tab_align {
            TabAlign::None | TabAlign::Left => return,
            TabAlign::Right => match self.pending_tab_column {
                Some(column) => column as i64 - self.column as i64,
                None if allow_anonymous => self.width as i64 - 1 - self.column as i64,
                None => 0,
            },
            TabAlign::Center => {
                let text_len = self.column as i64 - start as i64;
                match self.pending_tab_column {
                    Some(column) => (column as i64 - text_len / 2) - start as i64,
                    None if allow_anonymous => ((self.width as i64 - start as i64) - text_len) / 2,
                    None => 0,
                }
            }
            TabAlign::Decimal => {
                // Find the decimal point in the text after the tab.
                let first = self
                    .line
                    .len()
                    .saturating_sub(self.column.saturating_sub(start));
                let mut column = start;
                for &c in &self.line[first..] {
                    if c == self.pending_tab_decimal {
                        break;
                    }
                    column += 1;
                }
                self.pending_tab_column.unwrap_or(0) as i64 - column as i64
            }
        };
        if spaces > 0 {
            let at = self
                .line
                .len()
                .saturating_sub(self.column.saturating_sub(start));
            for _ in 0..spaces {
                self.line.insert(at, ' ');
                self.flags.insert(at, 0);
            }
            self.column += spaces as usize;
        }
        self.pending_tab_align = TabAlign::None;
    }

    // ---- buffering characters ----

    /// Add one character of display text to the current line, wrapping
    /// the line if it is full.
    fn buffer_char(&mut self, c: char) {
        let mut c = c;
        let mut width = 1;
        let mut char_flags = self.cur_flags;
        let mut quoted_space = false;

        match c {
            // A soft hyphen: a place the line can break, with a hyphen.
            '\u{ad}' => {
                if let Some(last) = self.flags.last_mut() {
                    *last |= FLAG_SHY;
                }
                return;
            }
            // A zero-width no-break space.
            '\u{feff}' => {
                if let Some(last) = self.flags.last_mut() {
                    *last |= FLAG_NOBREAK;
                }
                return;
            }
            // Zero-width, hair, and punctuation spaces: break points that
            // take up no room.
            '\u{200b}' | '\u{200a}' | '\u{2008}' => {
                if let Some(last) = self.flags.last_mut() {
                    *last &= !FLAG_NOBREAK;
                    *last |= FLAG_OKBREAK;
                }
                return;
            }
            // A non-breaking space stays as it is until it is written.
            '\u{a0}' => {}
            // Spaces that are not to be combined with their neighbors.
            '\u{15}' | '\u{2005}' | '\u{2006}' | '\u{2007}' | '\u{2009}' => {
                quoted_space = true;
                c = ' ';
            }
            '\u{2002}' | '\u{2004}' => {
                quoted_space = true;
                width = 2;
                c = ' ';
            }
            '\u{2003}' => {
                quoted_space = true;
                width = 3;
                c = ' ';
            }
            _ => {
                if crate::regex::is_space(c) {
                    c = ' ';
                    quoted_space = self.obey_whitespace || self.pre_level > 0;
                }
            }
        }
        if quoted_space {
            char_flags |= FLAG_QSPACE;
        }

        // Apply a pending request to change the case of the next letter.
        if self.caps && c.is_alphabetic() {
            self.caps = false;
            let upper: Vec<char> = c.to_uppercase().collect();
            if upper.len() > 1 {
                // Title case: only the first letter is a capital.
                self.buffer_char(upper[0]);
                for &rest in &upper[1..] {
                    for lower in rest.to_lowercase() {
                        self.buffer_char(lower);
                    }
                }
                return;
            }
            c = upper.first().copied().unwrap_or(c);
        } else if self.nocaps && c.is_alphabetic() {
            self.nocaps = false;
            let lower: Vec<char> = c.to_lowercase().collect();
            if lower.len() > 1 {
                for &each in &lower {
                    self.buffer_char(each);
                }
                return;
            }
            c = lower.first().copied().unwrap_or(c);
        }

        if c == ' ' && self.pre_level == 0 {
            // No spaces at the start of a line, and no runs of spaces,
            // unless they are quoted.
            if self.column == 0 && !quoted_space {
                return;
            }
            if let Some(&previous) = self.line.last() {
                if !quoted_space && previous == ' ' {
                    return;
                }
                // A quoted space replaces an ordinary one before it.
                if quoted_space
                    && previous == ' '
                    && self.flags[self.line.len() - 1] & FLAG_QSPACE == 0
                {
                    self.line.pop();
                    self.flags.pop();
                    self.column -= 1;
                }
            }
        }

        if self.column + width < self.width {
            self.buffer_rendered(c, char_flags, width);
            return;
        }

        // The line is full. A space just hangs off the end.
        if c == ' ' {
            if self.line.last() != Some(&' ') {
                self.line.push(' ');
                self.flags.push(char_flags);
            }
            self.column += width;
            return;
        }

        // Find the last place the line can be broken, treating the new
        // character as tentatively on the line.
        self.line.push(c);
        self.flags.push(self.cur_flags);
        let last = self.line.len() - 1;
        let mut soft_hyphen = false;
        let mut break_at = None;
        for i in (0..=last).rev() {
            let flags = self.flags[i];
            let previous_flags = if i > 0 { self.flags[i - 1] } else { 0 };
            if previous_flags & FLAG_NOBREAK != 0 {
                continue;
            }
            if previous_flags & FLAG_OKBREAK != 0 {
                break_at = Some(i);
                break;
            }
            if flags & FLAG_BREAK_ANY != 0 || (i > 0 && previous_flags & FLAG_BREAK_ANY != 0) {
                break_at = Some(i);
                break;
            }
            if i > 0 && i < last && previous_flags & FLAG_SHY != 0 {
                soft_hyphen = true;
                break_at = Some(i);
                break;
            }
            if self.line[i] == ' '
                || (i > 0 && self.line[i - 1] == ' ')
                || (i > 0 && self.line[i - 1] == '-' && self.line[i] != '-')
            {
                break_at = Some(i);
                break;
            }
        }
        self.line.pop();
        self.flags.pop();

        match break_at {
            None => {
                // No break anywhere: the line is one long word. Write it
                // and start a new line.
                self.flush(Newline::Line);
                self.line.clear();
                self.flags.clear();
                self.column = 0;
            }
            Some(mut i) => {
                // Carry everything after the break to the next line,
                // without the spaces at the break.
                let mut next = i.min(self.line.len());
                while next < self.line.len() && self.line[next] == ' ' {
                    next += 1;
                }
                let carried: Vec<char> = self.line[next..].to_vec();
                let carried_flags: Vec<u8> = self.flags[next..].to_vec();
                i = i.min(self.line.len());
                self.line.truncate(i);
                self.flags.truncate(i);
                if soft_hyphen {
                    self.line.push('-');
                    self.flags.push(0);
                    i += 1;
                }
                while i > 0 && self.line[i - 1] == ' ' {
                    if self.flags[i - 1] & FLAG_NOBREAK != 0 {
                        break;
                    }
                    i -= 1;
                }
                self.line.truncate(i);
                self.flags.truncate(i);
                self.flush(Newline::Line);
                self.column = carried.len();
                self.line = carried;
                self.flags = carried_flags;
            }
        }
        self.buffer_rendered(c, char_flags, width);
    }

    fn buffer_rendered(&mut self, c: char, flags: u8, width: usize) {
        let flags_before = self.flags.last().map_or(0, |f| f & FLAG_NOBREAK);
        for remaining in (1..=width).rev() {
            self.line.push(c);
            // The cells of a wide space stay together.
            self.flags.push(if remaining > 1 {
                flags | flags_before
            } else {
                flags
            });
            self.column += 1;
        }
    }

    fn buffer_str(&mut self, text: &str) {
        for c in text.chars() {
            self.buffer_char(c);
        }
    }

    // ---- the text stream ----

    /// Format text a program displayed.
    pub(crate) fn format_text(&mut self, text: &[u8]) {
        prof_scope!(crate::prof::CONSOLE);
        let mut source = Source {
            chars: String::from_utf8_lossy(text).chars().collect(),
            pos: 0,
        };
        let mut c = source.next();
        while c != '\0' {
            if self.parse_state != ParseState::Normal
                || (self.ignore_level != 0 && c != '&' && c != '<')
            {
                c = self.resume_html(c, &mut source);
                continue;
            }
            match c {
                '\n' => self.flush(Newline::Line),
                '\t' => self.write_tab(0, 4),
                // "\b": a blank line.
                '\u{b}' => self.write_blank_line(),
                // "\^": capitalize the next letter.
                '\u{f}' => {
                    self.caps = true;
                    self.nocaps = false;
                }
                // "\v": make the next letter lower-case.
                '\u{e}' => {
                    self.nocaps = true;
                    self.caps = false;
                }
                '<' | '&' if !self.literal => {
                    c = self.parse_markup(c, &mut source);
                    continue;
                }
                _ => self.buffer_char(c),
            }
            c = source.next();
        }
    }

    /// Start a tag or an entity.
    fn parse_markup(&mut self, c: char, source: &mut Source) -> char {
        if c == '&' {
            let (entity, next) = self.parse_entity(source);
            self.buffer_char(entity);
            return next;
        }

        let mut c = source.next();
        let is_end_tag = c == '/' || c == '\\';
        if is_end_tag {
            c = source.next();
        }
        let mut tag = String::new();
        while c != '\0' && c != ' ' && c != '>' {
            if tag.chars().count() < 49 {
                tag.push(c);
            }
            c = source.next();
        }
        let ignoring = self.ignore_level != 0;
        match tag.to_lowercase().as_str() {
            "br" => {
                if !ignoring {
                    // What it does depends on a HEIGHT attribute, so wait
                    // for the end of the tag.
                    self.deferred_break = if self.line.is_empty() {
                        DeferredBreak::Blank
                    } else {
                        DeferredBreak::Flush
                    };
                }
            }
            // Text attributes do not show on a plain console.
            "b" | "i" | "em" | "strong" | "tt" => {}
            "p" => {
                if !ignoring {
                    self.write_blank_line();
                }
            }
            "tab" => {
                if !ignoring {
                    self.expand_pending_tab(true);
                    self.in_tab = true;
                    self.new_tab_column = None;
                    self.new_tab_align = TabAlign::None;
                    self.new_tab_decimal = '\0';
                }
            }
            "img" | "sound" => self.allow_alt = true,
            "font" => {
                if !ignoring && !is_end_tag {
                    self.allow_color = true;
                }
            }
            "body" => {
                if !ignoring && !is_end_tag {
                    self.in_body = true;
                }
            }
            "hr" => {
                if !ignoring {
                    self.flush(Newline::Line);
                    for _ in 0..self.width.saturating_sub(1) {
                        self.buffer_char('-');
                    }
                    self.write_blank_line();
                }
            }
            "q" => {
                if !ignoring {
                    if !is_end_tag {
                        self.quote_level += 1;
                    }
                    // Double quotes outermost, single quotes inside them.
                    let outer = self.quote_level & 1 == 1;
                    let quote = match (is_end_tag, outer) {
                        (false, true) => '\u{201c}',
                        (false, false) => '\u{2018}',
                        (true, true) => '\u{201d}',
                        (true, false) => '\u{2019}',
                    };
                    self.buffer_char(quote);
                    if is_end_tag {
                        self.quote_level -= 1;
                    }
                }
            }
            "title" => {
                if is_end_tag {
                    self.ignore_level -= 1;
                    self.title_level -= 1;
                } else {
                    self.ignore_level += 1;
                    self.title_level += 1;
                }
            }
            // The contents of these appear only on graphical consoles, or
            // only in a log file.
            "aboutbox" | "banner" | "log" => {
                if is_end_tag {
                    self.ignore_level -= 1;
                } else {
                    self.ignore_level += 1;
                }
            }
            "wrap" => {
                if !ignoring {
                    self.in_wrap = true;
                }
            }
            "pre" => {
                if !is_end_tag {
                    self.pre_level += 1;
                    self.write_blank_line();
                } else if self.pre_level > 0 {
                    self.pre_level -= 1;
                    self.write_blank_line();
                }
            }
            _ => {}
        }
        self.parse_state = ParseState::Tag;
        c
    }

    /// Continue reading a tag, or skip text that is not displayed.
    fn resume_html(&mut self, c: char, source: &mut Source) -> char {
        let mut c = c;
        while c != '\0' {
            if self.parse_state == ParseState::Normal {
                if self.ignore_level == 0 || c == '<' {
                    return c;
                }
                // Text inside a tag whose contents are not displayed.
                if c == '&' {
                    c = self.parse_entity(source).1;
                } else {
                    c = source.next();
                }
                continue;
            }
            match self.parse_state {
                ParseState::Normal => unreachable!("handled above"),
                ParseState::Tag => match c {
                    '>' => self.end_tag(),
                    '"' => self.parse_state = ParseState::DoubleQuote,
                    '\'' => self.parse_state = ParseState::SingleQuote,
                    _ if c.is_alphabetic() => {
                        self.parse_state = ParseState::AttrName;
                        self.attr_name.clear();
                        continue;
                    }
                    _ => {}
                },
                ParseState::DoubleQuote => {
                    if c == '"' {
                        self.parse_state = ParseState::Tag;
                    }
                }
                ParseState::SingleQuote => {
                    if c == '\'' {
                        self.parse_state = ParseState::Tag;
                    }
                }
                ParseState::AttrName => {
                    while c.is_alphabetic() || c.is_ascii_digit() {
                        if self.attr_name.chars().count() < 39 {
                            self.attr_name.push(c);
                        }
                        c = source.next();
                    }
                    if c == '\0' {
                        break;
                    }
                    while c.is_whitespace() {
                        c = source.next();
                    }
                    if c == '=' {
                        self.parse_state = ParseState::AttrValue;
                        self.attr_value.clear();
                        self.attr_quote = None;
                    } else {
                        // An attribute with no value has its name as its
                        // value.
                        self.attr_value = self.attr_name.clone();
                        self.parse_state = ParseState::Tag;
                        self.process_attribute();
                        continue;
                    }
                }
                ParseState::AttrValue => {
                    if self.attr_quote.is_none() && self.attr_value.is_empty() {
                        while c.is_whitespace() {
                            c = source.next();
                        }
                    }
                    if c == '\0' {
                        break;
                    }
                    if self.attr_value.is_empty() && (c == '"' || c == '\'') {
                        self.attr_quote = Some(c);
                        c = source.next();
                        if c == '\0' {
                            break;
                        }
                    }
                    while c != '\0' {
                        if self.attr_quote == Some(c) {
                            c = source.next();
                            self.parse_state = ParseState::Tag;
                            break;
                        }
                        if self.attr_quote.is_none() && (c.is_whitespace() || c == '>') {
                            self.parse_state = ParseState::Tag;
                            break;
                        }
                        if self.attr_value.chars().count() < 255 {
                            self.attr_value.push(c);
                        }
                        c = source.next();
                    }
                    if self.parse_state == ParseState::AttrValue {
                        // The text ran out in the middle of the value.
                        break;
                    }
                    self.process_attribute();
                    continue;
                }
            }
            c = source.next();
        }
        c
    }

    /// Finish a tag at its closing bracket.
    fn end_tag(&mut self) {
        self.parse_state = ParseState::Normal;
        match self.deferred_break {
            DeferredBreak::None => {}
            DeferredBreak::Flush => self.flush(Newline::Line),
            DeferredBreak::Blank => self.write_blank_line(),
        }
        self.deferred_break = DeferredBreak::None;

        if self.in_tab {
            let positioned = self.new_tab_column.is_some() || self.new_tab_align != TabAlign::None;
            if !positioned {
                // A bare <TAB> is an ordinary tab character.
                self.write_tab(0, 4);
            } else if self.new_tab_column.is_none()
                && self.new_tab_align != TabAlign::Right
                && self.new_tab_align != TabAlign::Center
            {
                // Only right and center alignment work without a stop.
            } else if matches!(self.new_tab_align, TabAlign::Left | TabAlign::None) {
                let column = self.new_tab_column.unwrap_or(0) as i64;
                let delta = column - self.column as i64;
                if delta > 0 {
                    self.write_tab(delta as i32, 0);
                }
            } else {
                // The spacing depends on the text that follows.
                self.pending_tab_start = self.column;
                self.pending_tab_column = self.new_tab_column;
                self.pending_tab_align = self.new_tab_align;
                self.pending_tab_decimal = match self.new_tab_decimal {
                    '\0' => '.',
                    other => other,
                };
            }
        }
        self.allow_alt = false;
        self.allow_color = false;
        self.in_body = false;
        self.in_tab = false;
        self.in_wrap = false;
    }

    /// Act on the attribute just read.
    fn process_attribute(&mut self) {
        let name = self.attr_name.to_lowercase();
        let value = self.attr_value.clone();
        if self.deferred_break != DeferredBreak::None && name == "height" {
            match to_int(&value) {
                // HEIGHT=0 ends the line without adding a blank one.
                0 => self.deferred_break = DeferredBreak::Flush,
                1 => {}
                height => {
                    for _ in 0..height.max(0) {
                        self.write_blank_line();
                    }
                }
            }
        } else if self.allow_alt && self.ignore_level == 0 && name == "alt" {
            // The text alternative to an image or sound.
            self.buffer_str(&value);
        } else if (self.allow_color && (name == "color" || name == "bgcolor"))
            || (self.in_body && (name == "text" || name == "bgcolor"))
        {
            let _ = is_color(&value);
            if self.in_body && name == "bgcolor" {
                self.body_color = color_value(&value);
            }
        } else if self.in_tab {
            match name.as_str() {
                "id" => {
                    self.tabs.insert(value.to_lowercase(), self.column);
                    self.in_tab = false;
                }
                "to" => {
                    self.new_tab_column = self.tabs.get(&value.to_lowercase()).copied();
                    if self.new_tab_column.is_none() {
                        self.in_tab = false;
                    }
                }
                "indent" | "multiple" => {
                    self.write_tab(to_int(&value), 0);
                    self.in_tab = false;
                }
                "align" => match value.to_lowercase().as_str() {
                    "left" => self.new_tab_align = TabAlign::Left,
                    "right" => self.new_tab_align = TabAlign::Right,
                    "center" => self.new_tab_align = TabAlign::Center,
                    "decimal" => self.new_tab_align = TabAlign::Decimal,
                    _ => self.in_tab = false,
                },
                "dp" => self.new_tab_decimal = value.chars().next().unwrap_or('\0'),
                _ => {}
            }
        } else if self.in_wrap {
            match name.as_str() {
                "word" => self.cur_flags &= !FLAG_BREAK_ANY,
                "char" => self.cur_flags |= FLAG_BREAK_ANY,
                _ => {}
            }
        }
    }

    /// Read an entity after its `&`. Returns the character it stands for
    /// and the next character of the text. Something that is not an
    /// entity reads as a plain `&`.
    fn parse_entity(&mut self, source: &mut Source) -> (char, char) {
        let start = source.pos;
        let mut c = source.next();
        if c == '#' {
            let mut value: u32 = 0;
            c = source.next();
            if c == 'x' || c == 'X' {
                c = source.next();
                while is_xdigit(c) {
                    value = value
                        .wrapping_mul(16)
                        .wrapping_add(c.to_digit(16).unwrap_or(0));
                    c = source.next();
                }
            } else {
                while c.is_ascii_digit() {
                    value = value
                        .wrapping_mul(10)
                        .wrapping_add(c.to_digit(10).unwrap_or(0));
                    c = source.next();
                }
            }
            if c == ';' {
                c = source.next();
            }
            return (char::from_u32(value).unwrap_or('?'), c);
        }

        let mut name = String::new();
        while c != '\0' && c.is_ascii_alphanumeric() && name.len() < 9 {
            name.push(c);
            c = source.next();
        }
        let found = entities::lookup(&name);
        if c == ';' {
            c = source.next();
        }
        match found {
            Some(entity) => (entity, c),
            None => {
                source.pos = start;
                ('&', source.next())
            }
        }
    }
}
