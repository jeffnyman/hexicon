//! Console input, and the files the console reads and writes on the side:
//! input scripts, which stand in for the player; the transcript, which
//! records everything displayed; command logs, which record what the
//! player entered; and log consoles, which format text into a file.
//!
//! The functions here are the `tads-io` function set apart from `tadsSay`.

use crate::console::{Formatter, Newline};
use crate::error::{VmError, VmResult, code};
use crate::load::CONSOLE_WIDTH;
use crate::object::Object;
use crate::value::Value;
use crate::vm::Vm;

// Kinds of input event, as programs see them.
const EVT_KEY: i32 = 1;
const EVT_TIMEOUT: i32 = 2;
const EVT_HREF: i32 = 3;
const EVT_NOTIMEOUT: i32 = 4;
const EVT_EOF: i32 = 5;
const EVT_LINE: i32 = 6;
const EVT_COMMAND: i32 = 0x100;
const EVT_END_QUIET_SCRIPT: i32 = 10000;
const EVT_DIALOG: i32 = 10001;
const EVT_FILE: i32 = 10002;

/// The attribute of a script's file event that allows overwriting.
const ATTR_OVERWRITE: u32 = 1;

/// The longest line of input, in bytes, counting a terminator the
/// reference VM stores with it.
const LINE_MAX: usize = 256;

// Flags of `setScriptFile`.
const SCRIPT_QUIET: i32 = 1;
const SCRIPT_NONSTOP: i32 = 2;
const SCRIPT_EVENT: i32 = 4;
const SCRIPT_GET_STATUS: i32 = 0x7000;

/// The names scripts and event logs use for kinds of event.
const EVENT_TAGS: [(&str, i32); 9] = [
    ("key", EVT_KEY),
    ("timeout", EVT_TIMEOUT),
    ("notimeout", EVT_NOTIMEOUT),
    ("eof", EVT_EOF),
    ("line", EVT_LINE),
    ("command", EVT_COMMAND),
    ("endqs", EVT_END_QUIET_SCRIPT),
    ("dialog", EVT_DIALOG),
    ("file", EVT_FILE),
];

/// The name an event is logged under. A link event has a name for logging
/// that scripts do not recognize when they are read back.
fn event_tag(event: i32) -> &'static str {
    if event == EVT_HREF {
        return "href";
    }
    EVENT_TAGS
        .iter()
        .find(|(_, code)| *code == event)
        .map_or("", |(tag, _)| tag)
}

/// Whether a byte is white space, as C's `isspace` has it.
fn is_space(c: u8) -> bool {
    matches!(c, b' ' | b'\t' | b'\n' | 0x0b | 0x0c | b'\r')
}

/// Cut text to at most `max` bytes without splitting a character.
fn truncate(text: &mut Vec<u8>, max: usize) {
    if text.len() > max {
        let mut len = max;
        while len > 0 && text[len] & 0xc0 == 0x80 {
            len -= 1;
        }
        text.truncate(len);
    }
}

/// The year, month, and day of a count of days from the start of 1970.
pub(crate) fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let day_of_era = z.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let shifted_month = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * shifted_month + 2) / 5 + 1;
    let month = if shifted_month < 10 {
        shifted_month + 3
    } else {
        shifted_month - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

/// The count of days from the start of 1970 to a date.
pub(crate) fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let year_of_era = year.rem_euclid(400);
    let shifted_month = if month > 2 { month - 3 } else { month + 9 };
    let day_of_year = (153 * shifted_month + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

/// The integer at the start of text, as C's `atoi` reads it.
fn leading_int(text: &[u8]) -> i32 {
    let mut rest = text;
    while let [first, tail @ ..] = rest {
        if !is_space(*first) {
            break;
        }
        rest = tail;
    }
    let negative = match rest {
        [b'-', tail @ ..] => {
            rest = tail;
            true
        }
        [b'+', tail @ ..] => {
            rest = tail;
            false
        }
        _ => false,
    };
    let mut value: i32 = 0;
    for &c in rest {
        if !c.is_ascii_digit() {
            break;
        }
        value = value.wrapping_mul(10).wrapping_add(i32::from(c - b'0'));
    }
    if negative {
        value.wrapping_neg()
    } else {
        value
    }
}

/// An input script being read.
#[derive(Debug)]
struct Script {
    data: Vec<u8>,
    pos: usize,
    /// Nothing is displayed while the script is read.
    quiet: bool,
    /// Output pauses at each screenful while the script is read.
    more_mode: bool,
    /// The script holds events of every kind, not only command lines.
    events: bool,
}

impl Script {
    fn getc(&mut self) -> Option<u8> {
        let c = self.data.get(self.pos).copied();
        if c.is_some() {
            self.pos += 1;
        }
        c
    }

    fn skip_line(&mut self) {
        while let Some(c) = self.getc() {
            if c == b'\n' || c == b'\r' {
                break;
            }
        }
    }

    /// The rest of the line, of which at most `max - 1` bytes are kept.
    fn read_param(&mut self, max: usize) -> Vec<u8> {
        let mut out = Vec::new();
        loop {
            match self.getc() {
                None | Some(b'\n') => break,
                Some(c) => {
                    if out.len() + 1 < max {
                        out.push(c);
                    }
                }
            }
        }
        truncate(&mut out, max - 1);
        out
    }

    /// A word of a tag, and the character that ended it.
    fn read_token(&mut self) -> (Vec<u8>, Option<u8>) {
        let mut c = self.getc();
        while c.is_some_and(is_space) {
            c = self.getc();
        }
        let mut out = Vec::new();
        while let Some(ch) = c {
            if out.len() >= 31 || ch == b'>' || is_space(ch) {
                break;
            }
            out.push(ch);
            c = self.getc();
        }
        (out, c)
    }

    /// Find the next event, leaving the position at its parameter.
    fn read_event_type(&mut self) -> Option<(i32, u32)> {
        if !self.events {
            // A command script: each command line starts with `>`.
            loop {
                match self.getc()? {
                    b'>' => return Some((EVT_LINE, 0)),
                    _ => self.skip_line(),
                }
            }
        }
        loop {
            if self.getc()? != b'<' {
                self.skip_line();
                continue;
            }
            let (tag, mut c) = self.read_token();
            let mut attrs = 0;
            while c.is_some_and(is_space) {
                let (attr, next) = self.read_token();
                c = next;
                if attr.is_empty() {
                    break;
                }
                if attr.eq_ignore_ascii_case(b"overwrite") {
                    attrs |= ATTR_OVERWRITE;
                }
                if matches!(c, Some(b'>' | b'\n' | b'\r') | None) {
                    break;
                }
            }
            if c != Some(b'>') {
                self.skip_line();
                continue;
            }
            let found = EVENT_TAGS
                .iter()
                .find(|(name, _)| tag.eq_ignore_ascii_case(name.as_bytes()));
            match found {
                Some((_, event)) => return Some((*event, attrs)),
                None => self.skip_line(),
            }
        }
    }

    /// The next line of input in the script.
    fn read_line(&mut self) -> Option<(i32, Vec<u8>)> {
        loop {
            if self.events {
                let (event, _) = self.read_event_type()?;
                match event {
                    EVT_LINE | EVT_TIMEOUT => return Some((event, self.read_param(LINE_MAX))),
                    _ => self.skip_line(),
                }
            } else {
                match self.getc()? {
                    b'>' => return Some((EVT_LINE, self.read_param(LINE_MAX))),
                    b'\n' | b'\r' => {}
                    _ => self.skip_line(),
                }
            }
        }
    }
}

/// A file that text is formatted into.
#[derive(Debug)]
struct LogFile {
    name: String,
    formatter: Formatter,
}

/// A file that input is recorded in.
#[derive(Debug)]
struct CommandLog {
    name: String,
    /// Every event is recorded, in the form an event script has. Otherwise
    /// only command lines are.
    events: bool,
}

/// The state of the console apart from the main window's formatter.
#[derive(Debug, Default)]
pub(crate) struct IoState {
    /// Formats text for the status line.
    status: Option<Formatter>,
    /// Text displayed goes to the status line.
    in_status: bool,
    /// The transcript.
    log: Option<LogFile>,
    command_log: Option<CommandLog>,
    /// Scripts being read, the innermost last.
    scripts: Vec<Script>,
    /// A line of input was started and has not been finished off.
    read_in_progress: bool,
    /// The line of input last read.
    read_buf: Vec<u8>,
    /// Log consoles by handle, less one.
    log_consoles: Vec<Option<LogFile>>,
    /// The clock reading that the program's count of milliseconds starts
    /// from: the reading when the program first asked.
    tick_zero: Option<i64>,
}

impl Vm {
    // ---- output ----

    /// Display text on the console.
    pub(crate) fn print(&mut self, text: &[u8]) {
        if self.io.in_status {
            if let Some(status) = &mut self.io.status {
                status.format_text(text);
            }
            return;
        }
        self.console.format_text(text);
        self.send_output();
        if let Some(log) = &mut self.io.log {
            log.formatter.format_text(text);
            self.send_log();
        }
    }

    /// Hand finished output to the host. A script read quietly shows
    /// nothing.
    fn send_output(&mut self) {
        let output = self.console.take_output();
        if !output.is_empty() && !self.io.scripts.last().is_some_and(|script| script.quiet) {
            self.host.print(&output);
        }
    }

    /// Write finished transcript text to its file.
    fn send_log(&mut self) {
        if let Some(log) = &mut self.io.log {
            let output = log.formatter.take_output();
            if !output.is_empty() {
                // A transcript that cannot be written is not the
                // program's concern.
                let _ = self.host.append_file(&log.name, output.as_bytes());
            }
        }
    }

    /// Whether displayed text goes to the transcript too.
    fn log_enabled(&self) -> bool {
        !self.io.in_status && self.io.log.is_some()
    }

    /// Write out the unfinished line.
    pub(crate) fn flush_console(&mut self, newline: Newline) {
        if self.io.in_status {
            if let Some(status) = &mut self.io.status {
                status.flush(newline);
            }
            return;
        }
        self.console.flush(newline);
        self.send_output();
        if let Some(log) = &mut self.io.log {
            log.formatter.flush(newline);
            self.send_log();
        }
    }

    /// Add text to the transcript as it is.
    fn log_raw(&mut self, text: &[u8]) {
        if let Some(log) = &mut self.io.log {
            log.formatter.write_raw(&String::from_utf8_lossy(text));
            self.send_log();
        }
    }

    /// The cursor is at the start of a line because the player ended one.
    fn note_input_line(&mut self) {
        if self.io.in_status {
            if let Some(status) = &mut self.io.status {
                status.note_input_line();
            }
        } else {
            self.console.note_input_line();
        }
        if let Some(log) = &mut self.io.log {
            log.formatter.note_input_line();
        }
    }

    /// Send displayed text to the status line, or back to the main window.
    fn set_status_mode(&mut self, on: bool) {
        if on && self.io.status.is_none() {
            self.io.status = Some(Formatter::new(CONSOLE_WIDTH));
        }
        if on == self.io.in_status {
            return;
        }
        self.io.in_status = on;
        if !on {
            if let Some(status) = &mut self.io.status {
                status.format_text(b"\n");
                status.flush(Newline::None);
                let text = status.take_output();
                self.host.status_line(text.trim_end());
            }
        }
    }

    // ---- scripts and logs ----

    /// Read input from a script before asking the player for any. The
    /// script is the contents of a command file: a line starting with `>`
    /// is a command, or, in a file that starts with the line
    /// `<eventscript>`, each line is an event such as `<line>look`.
    pub fn push_input_script(&mut self, data: &[u8], quiet: bool, pause_output: bool) {
        // The reference VM reads the file as text, which on Windows drops
        // the carriage return of each line ending.
        let mut text = Vec::with_capacity(data.len());
        for (index, &c) in data.iter().enumerate() {
            if c != b'\r' || data.get(index + 1) != Some(&b'\n') {
                text.push(c);
            }
        }
        const HEADER: &[u8] = b"<eventscript>\n";
        let events = text.starts_with(HEADER);
        let mut quiet = quiet;
        let mut more_mode = pause_output;
        if let Some(enclosing) = self.io.scripts.last() {
            quiet |= enclosing.quiet;
            more_mode &= enclosing.more_mode;
        }
        self.io.scripts.push(Script {
            data: text,
            pos: if events { HEADER.len() } else { 0 },
            quiet,
            more_mode,
            events,
        });
    }

    /// Record an event in the command log.
    fn log_event(&mut self, tag: &[u8], param: Option<&[u8]>) {
        let Some(log) = &self.io.command_log else {
            return;
        };
        let tag = tag.strip_prefix(b"<").unwrap_or(tag);
        let name = tag.strip_suffix(b">").unwrap_or(tag);
        let mut out = Vec::new();
        if log.events {
            let mut param = param;
            if name.eq_ignore_ascii_case(b"key") {
                param = match param {
                    Some(b"\n") => Some(b"[enter]"),
                    Some(b"\t") => Some(b"[tab]"),
                    Some(b" ") => Some(b"[space]"),
                    other => other,
                };
            }
            if name.eq_ignore_ascii_case(b"notimeout") || name.eq_ignore_ascii_case(b"eof") {
                param = None;
            }
            if !name.is_empty() {
                // The reference VM writes the tag with any `>` the caller
                // put at its end, and then its own.
                out.push(b'<');
                out.extend_from_slice(tag);
                out.push(b'>');
                out.extend_from_slice(param.unwrap_or_default());
                out.push(b'\n');
            }
        } else if name.eq_ignore_ascii_case(b"line") {
            if let Some(param) = param {
                out.push(b'>');
                out.extend_from_slice(param);
                out.push(b'\n');
            }
        }
        if !out.is_empty() {
            let _ = self.host.append_file(&log.name, &out);
        }
    }

    // ---- reading ----

    /// Finish off a line of input that was started.
    fn read_line_done(&mut self) {
        if !self.io.read_in_progress {
            return;
        }
        // The transcript gets what the player typed. What a script
        // supplied is displayed, and reaches the transcript that way,
        // unless the script is read quietly.
        if self.log_enabled() && self.io.scripts.last().is_none_or(|script| script.quiet) {
            let mut text = self.io.read_buf.clone();
            text.push(b'\n');
            self.log_raw(&text);
        }
        self.note_input_line();
        self.io.read_in_progress = false;
    }

    /// Read a line of input: the kind of event that ended the read, and
    /// the text. The host cannot interrupt a read, so one with a timeout
    /// reports that timeouts are not available.
    fn read_line_timeout(&mut self, timeout: Option<i32>) -> (i32, Vec<u8>) {
        let mut event = 0;
        let mut from_script = false;
        let mut echo = false;
        if !self.io.read_in_progress {
            self.flush_console(Newline::Input);
        }
        while let Some(script) = self.io.scripts.last_mut() {
            let was_quiet = script.quiet;
            if let Some((found, line)) = script.read_line() {
                self.io.read_buf = line;
                event = found;
                from_script = true;
                echo = !was_quiet;
                break;
            }
            // The script is used up: go back to what was being read
            // before it.
            self.io.scripts.pop();
            let enclosing = self.io.scripts.last().map(|script| script.quiet);
            if enclosing == Some(was_quiet) {
                continue;
            }
            self.flush_console(Newline::None);
            if was_quiet && enclosing != Some(true) {
                // Output shows again from here, and the caller needs to
                // know that the display is out of date.
                if self.log_enabled() {
                    self.log_raw(b"\n");
                }
                self.note_input_line();
                self.log_event(event_tag(EVT_END_QUIET_SCRIPT).as_bytes(), None);
                return (EVT_END_QUIET_SCRIPT, Vec::new());
            }
            if enclosing.is_none() {
                break;
            }
        }
        self.io.read_in_progress = true;
        if !from_script {
            if timeout.is_some() {
                event = EVT_NOTIMEOUT;
            } else if let Some(line) = self.host.read_line() {
                self.io.read_buf = line.into_bytes();
                event = EVT_LINE;
                self.note_input_line();
            } else {
                event = EVT_EOF;
            }
        }
        if event == EVT_EOF {
            self.read_line_done();
            self.log_event(event_tag(event).as_bytes(), None);
            return (event, Vec::new());
        }
        let mut line = self.io.read_buf.clone();
        truncate(&mut line, LINE_MAX - 1);
        if echo {
            self.print(&line);
            self.print(b"\n");
        }
        if event == EVT_LINE {
            self.read_line_done();
        }
        let logged = self.io.read_buf.clone();
        self.log_event(event_tag(event).as_bytes(), Some(&logged));
        (event, line)
    }

    /// Read a line of input, or `None` at the end of input.
    fn read_line(&mut self) -> Option<Vec<u8>> {
        self.read_line_done();
        loop {
            match self.read_line_timeout(None) {
                (EVT_LINE, line) => return Some(line),
                (EVT_END_QUIET_SCRIPT, _) => {}
                _ => return None,
            }
        }
    }

    /// Read the next event of one of the given kinds from an event script,
    /// if one is being read: the event, its parameter, and its attributes.
    fn read_event_script(&mut self, filter: &[i32], max: usize) -> Option<(i32, Vec<u8>, u32)> {
        if !self.io.scripts.last()?.events {
            return None;
        }
        loop {
            let script = self.io.scripts.last_mut()?;
            let Some((event, attrs)) = script.read_event_type() else {
                self.io.scripts.pop();
                continue;
            };
            if !filter.contains(&event) {
                script.skip_line();
                continue;
            }
            let mut param = script.read_param(max);
            if event == EVT_KEY {
                match param.as_slice() {
                    b"[enter]" => param = b"\n".to_vec(),
                    b"[tab]" => param = b"\t".to_vec(),
                    b"[space]" => param = b" ".to_vec(),
                    _ => {}
                }
            }
            return Some((event, param, attrs));
        }
    }

    /// Read a key from the player, as its T3 name.
    fn read_key(&mut self) -> Option<Vec<u8>> {
        let key = self.host.read_key()?;
        Some(match key.as_str() {
            "\r" | "\r\n" => b"\n".to_vec(),
            _ => key.into_bytes(),
        })
    }

    /// A list for the result of an input function, with the event code
    /// first. The reference VM creates the list before anything that goes
    /// in it.
    fn event_list(&mut self, event: i32, len: usize) -> Value {
        let mut items = vec![Value::Nil; len];
        items[0] = Value::Int(event);
        self.new_list(items)
    }

    fn set_list_item(&mut self, list: Value, index: usize, value: Value) {
        if let Some(Object::List(items)) = self.objects.get_mut(list.obj_id()) {
            let mut changed = items.to_vec();
            changed[index] = value;
            *items = changed.into();
        }
    }

    /// Ask the player to choose among buttons, on a console that has only
    /// text: the number of the button chosen, counting from one.
    fn text_dialog(&mut self, prompt: &str, buttons: &[String], cancel: i32) -> i32 {
        if buttons.is_empty() {
            return 0;
        }
        // In a label, `&` marks the letter that chooses the button.
        let shortcut = |label: &str| {
            let mut chars = label.chars().skip_while(|&c| c != '&');
            chars.next().and_then(|_| chars.next())
        };
        let same = |a: char, b: char| a.to_uppercase().eq(b.to_uppercase());
        loop {
            let mut text = format!("\n{prompt}");
            for (index, label) in buttons.iter().enumerate() {
                if index != 0 {
                    text.push('/');
                }
                match label.split_once('&') {
                    Some((before, after)) if !after.is_empty() => {
                        let mut rest = after.chars();
                        let key = rest.next().unwrap_or(' ');
                        text.push_str(&format!("{before}({key}){}", rest.as_str()));
                    }
                    _ => text.push_str(label),
                }
            }
            text.push_str(" > ");
            self.host.print(&text);
            let Some(line) = self.host.read_line() else {
                return cancel;
            };
            let response = line.trim_start();
            let mut letters = response.chars();
            if let (Some(letter), None) = (letters.next(), letters.next()) {
                let chosen = buttons
                    .iter()
                    .position(|label| shortcut(label).is_some_and(|key| same(key, letter)));
                if let Some(index) = chosen {
                    return index as i32 + 1;
                }
            }
            // Otherwise the response has to be the start of one label.
            let matches: Vec<usize> = (0..buttons.len())
                .filter(|&index| {
                    let mut label = buttons[index].chars().filter(|&c| c != '&');
                    response
                        .chars()
                        .all(|c| label.next().is_some_and(|l| same(c, l)))
                })
                .collect();
            if let [index] = matches.as_slice() {
                return *index as i32 + 1;
            }
        }
    }

    /// `getTime(type)`: the date and time as a list, or a count of
    /// milliseconds for measuring intervals.
    pub(crate) fn get_time(&mut self, kind: i32) -> VmResult<Value> {
        let time = self.host.time();
        match kind {
            1 => {
                let seconds = time.unix_millis.div_euclid(1000);
                let local = seconds + i64::from(time.utc_offset_seconds);
                let days = local.div_euclid(86400);
                let second_of_day = local.rem_euclid(86400);
                let (year, month, day) = civil_from_days(days);
                let day_of_year = days - days_from_civil(year, 1, 1) + 1;
                // The first day of 1970 was a Thursday; Sunday is day 1.
                let day_of_week = (days + 4).rem_euclid(7) + 1;
                let mut items: Vec<Value> = [
                    year,
                    month,
                    day,
                    day_of_week,
                    day_of_year,
                    second_of_day / 3600,
                    second_of_day / 60 % 60,
                    second_of_day % 60,
                ]
                .iter()
                .map(|&n| Value::Int(n as i32))
                .collect();
                // The count of seconds, which stops fitting an integer
                // in 2038.
                items.push(match i32::try_from(seconds) {
                    Ok(seconds) => Value::Int(seconds),
                    Err(_) => {
                        let big = crate::bignum::Big::from_int(seconds, 20);
                        self.new_big(big)
                    }
                });
                Ok(self.new_list(items))
            }
            2 => {
                let zero = *self.io.tick_zero.get_or_insert(time.unix_millis);
                Ok(Value::Int(
                    (time.unix_millis.wrapping_sub(zero) & 0x7fff_ffff) as i32,
                ))
            }
            _ => Err(VmError::new(code::BAD_VAL_BIF)),
        }
    }

    // ---- the functions ----

    /// Run a `tads-io` function other than `tadsSay`. False means the
    /// function is not implemented.
    pub(crate) fn bif_io(&mut self, index: u16, argc: u32) -> VmResult<bool> {
        let bad_value = || VmError::new(code::BAD_VAL_BIF);
        match index {
            // setLogFile(file, type?)
            1 => {
                Self::check_argc_range(argc, 1, 2)?;
                let file = self.arg(0);
                let kind = if argc >= 2 {
                    self.cast_to_int(self.arg(1))?
                } else {
                    1
                };
                if !(1..=3).contains(&kind) {
                    return Err(bad_value());
                }
                let mut ok = true;
                if file == Value::Nil {
                    if kind == 1 {
                        self.io.log = None;
                    } else {
                        self.io.command_log = None;
                    }
                } else {
                    let name = self.file_name_arg(file)?;
                    let start: &[u8] = if kind == 3 { b"<eventscript>\n" } else { b"" };
                    ok = self.host.write_file(&name, start).is_ok();
                    if kind == 1 {
                        self.io.log = ok.then(|| LogFile {
                            name,
                            formatter: Formatter::new(CONSOLE_WIDTH),
                        });
                    } else {
                        self.io.command_log = ok.then_some(CommandLog {
                            name,
                            events: kind == 3,
                        });
                    }
                }
                self.r0 = if ok { Value::True } else { Value::Nil };
            }
            // clearScreen()
            2 => {
                Self::check_argc(argc, 0)?;
                self.flush_console(Newline::None);
                if self.io.in_status {
                    if let Some(status) = &mut self.io.status {
                        status.empty_buffers();
                    }
                } else {
                    self.console.empty_buffers();
                    if let Some(log) = &mut self.io.log {
                        log.formatter.empty_buffers();
                    }
                    self.host.clear_screen();
                }
            }
            // morePrompt()
            3 => {
                Self::check_argc(argc, 0)?;
                if self.io.scripts.is_empty() {
                    self.flush_console(Newline::None);
                    self.host.print("[More]");
                    while let Some(key) = self.read_key() {
                        if matches!(key.as_slice(), b" " | b"\n") {
                            break;
                        }
                    }
                    self.host.print("\r      \r");
                }
            }
            // inputLine()
            4 => {
                Self::check_argc(argc, 0)?;
                self.r0 = match self.read_line() {
                    Some(line) => self.new_string(line),
                    None => Value::Nil,
                };
            }
            // inputKey()
            5 => {
                Self::check_argc(argc, 0)?;
                let key = match self.read_event_script(&[EVT_KEY], 32) {
                    Some((_, key, _)) => key,
                    None => {
                        self.flush_console(Newline::Input);
                        self.read_key().unwrap_or_else(|| b"[eof]".to_vec())
                    }
                };
                self.r0 = self.new_string(key.clone());
                self.log_event(b"key", Some(&key));
            }
            // inputEvent(timeout?)
            6 => {
                Self::check_argc_range(argc, 0, 1)?;
                let timeout = self.timeout_arg(argc)?;
                const FILTER: [i32; 6] = [
                    EVT_KEY,
                    EVT_TIMEOUT,
                    EVT_NOTIMEOUT,
                    EVT_HREF,
                    EVT_EOF,
                    EVT_COMMAND,
                ];
                let (event, param) = match self.read_event_script(&FILTER, 1024) {
                    Some((event, param, _)) => (event, param),
                    None => {
                        self.flush_console(Newline::Input);
                        if timeout.is_some() {
                            (EVT_NOTIMEOUT, Vec::new())
                        } else {
                            match self.read_key() {
                                Some(key) => (EVT_KEY, key),
                                None => (EVT_EOF, Vec::new()),
                            }
                        }
                    }
                };
                let has_detail = matches!(event, EVT_KEY | EVT_HREF | EVT_COMMAND);
                let list = self.event_list(event, if has_detail { 2 } else { 1 });
                let tag = event_tag(event).as_bytes();
                match event {
                    EVT_KEY | EVT_HREF => {
                        let text = self.new_string(param.clone());
                        self.set_list_item(list, 1, text);
                        self.log_event(tag, Some(&param));
                    }
                    EVT_COMMAND => {
                        let command = leading_int(&param);
                        self.set_list_item(list, 1, Value::Int(command));
                        self.log_event(tag, Some(command.to_string().as_bytes()));
                    }
                    _ => self.log_event(tag, None),
                }
                self.r0 = list;
            }
            // inputDialog(icon, prompt, buttons, default, cancel)
            7 => {
                Self::check_argc(argc, 5)?;
                self.int_arg(self.arg(0))?;
                let prompt = self.require_str(self.arg(1))?;
                let prompt = String::from_utf8_lossy(&prompt).into_owned();
                let buttons = self.dialog_buttons(self.arg(2))?;
                let choice = |vm: &Self, value: Value| match value {
                    Value::Nil => Ok(0),
                    _ => vm.int_arg(value),
                };
                choice(self, self.arg(3))?;
                let cancel = choice(self, self.arg(4))?;
                let response = match self.read_event_script(&[EVT_DIALOG], 32) {
                    Some((_, param, _)) => {
                        self.log_event(b"dialog", Some(&param));
                        leading_int(&param)
                    }
                    None => {
                        self.flush_console(Newline::Input);
                        let response = self.text_dialog(&prompt, &buttons, cancel);
                        self.log_event(b"dialog", Some(response.to_string().as_bytes()));
                        response
                    }
                };
                self.r0 = Value::Int(response);
            }
            // timeDelay(milliseconds)
            9 => {
                Self::check_argc(argc, 1)?;
                let delay = self.cast_to_int(self.arg(0))?;
                self.flush_console(Newline::None);
                self.host.sleep(delay.max(0) as u32);
            }
            // systemInfo(which, ...)
            10 => {
                if argc < 1 {
                    return Err(VmError::new(code::WRONG_NUM_OF_ARGS));
                }
                let which = self.int_arg(self.arg(0))?;
                if (1..=18).contains(&which) || (20..=34).contains(&which) {
                    Self::check_argc(argc, 1)?;
                }
                self.r0 = match which {
                    1 => Value::True,
                    2 => {
                        let version = crate::identity::version_number();
                        self.new_str(&format!(
                            "{}.{}.{}",
                            (version >> 16) & 0xffff,
                            (version >> 8) & 0xff,
                            version & 0xff
                        ))
                    }
                    3 => {
                        let name = self.host.system_name();
                        self.new_str(&name)
                    }
                    // A console of plain text: no HTML, pictures, sounds,
                    // links, colors, or banner windows.
                    4..=18 | 20..=27 | 31..=33 => Value::Int(0),
                    // The class of interpreter: text only.
                    34 => Value::Int(1),
                    _ => Value::Nil,
                };
            }
            // statusMode(mode)
            11 => {
                Self::check_argc(argc, 1)?;
                let mode = self.int_arg(self.arg(0))?;
                self.set_status_mode(mode != 0);
            }
            // statusRight(text)
            12 => {
                Self::check_argc(argc, 1)?;
                let text = self.require_str(self.arg(0))?;
                self.host.status_right(&String::from_utf8_lossy(&text));
            }
            // setScriptFile(file, flags?)
            14 => {
                Self::check_argc_range(argc, 1, 2)?;
                match self.arg(0) {
                    Value::Nil => {
                        self.io.scripts.pop();
                        self.r0 = Value::True;
                    }
                    Value::Int(SCRIPT_GET_STATUS) => {
                        self.r0 = match self.io.scripts.last() {
                            Some(script) => Value::Int(
                                if script.quiet { SCRIPT_QUIET } else { 0 }
                                    | if script.more_mode { 0 } else { SCRIPT_NONSTOP }
                                    | if script.events { SCRIPT_EVENT } else { 0 },
                            ),
                            None => Value::Nil,
                        };
                    }
                    Value::Int(_) => return Err(bad_value()),
                    file => {
                        let flags = if argc >= 2 {
                            self.cast_to_int(self.arg(1))?
                        } else {
                            0
                        };
                        let name = self.file_name_arg(file)?;
                        self.r0 = match self.host.read_file(&name) {
                            Ok(data) => {
                                self.push_input_script(
                                    &data,
                                    flags & SCRIPT_QUIET != 0,
                                    flags & SCRIPT_NONSTOP == 0,
                                );
                                Value::True
                            }
                            Err(_) => Value::Nil,
                        };
                    }
                }
            }
            // getLocalCharSet(which): everything outside the VM is UTF-8.
            15 => {
                Self::check_argc(argc, 1)?;
                self.r0 = match self.int_arg(self.arg(0))? {
                    1..=3 => self.new_str("utf8"),
                    _ => Value::Nil,
                };
            }
            // flushOutput()
            16 => {
                Self::check_argc(argc, 0)?;
                self.flush_console(Newline::None);
            }
            // inputLineTimeout(timeout?)
            17 => {
                Self::check_argc_range(argc, 0, 1)?;
                let timeout = self.timeout_arg(argc)?;
                let (event, line) = self.read_line_timeout(timeout);
                let has_text = matches!(event, EVT_LINE | EVT_TIMEOUT);
                let list = self.event_list(event, if has_text { 2 } else { 1 });
                if has_text {
                    let text = self.new_string(line);
                    self.set_list_item(list, 1, text);
                }
                self.r0 = list;
            }
            // inputLineCancel(reset)
            18 => {
                Self::check_argc(argc, 1)?;
                self.read_line_done();
            }
            // bannerCreate(...): there are no banner windows.
            19 => {
                Self::check_argc_range(argc, 7, 8)?;
                self.r0 = Value::Nil;
            }
            // bannerDelete(handle): deleting a banner that is not there
            // does nothing.
            20 => Self::check_argc(argc, 1)?,
            // The other banner functions need a banner, and no handle is
            // one.
            21..=29 => {
                let expected = match index {
                    25 | 26 => 3,
                    27 => 2,
                    29 => 4,
                    _ => 1,
                };
                if index == 22 {
                    Self::check_argc_range(argc, 1, 32767)?;
                } else {
                    Self::check_argc(argc, expected)?;
                }
                return Err(bad_value());
            }
            // logConsoleCreate(file, charset, width)
            30 => {
                Self::check_argc(argc, 3)?;
                let width = self.cast_to_int(self.arg(2))?;
                let name = self.file_name_arg(self.arg(0))?;
                if self.host.write_file(&name, b"").is_err() {
                    return Err(VmError::new(code::CREATE_FILE));
                }
                let console = LogFile {
                    name,
                    formatter: Formatter::new(width.max(1) as usize),
                };
                let slot = match self.io.log_consoles.iter().position(Option::is_none) {
                    Some(slot) => slot,
                    None => {
                        self.io.log_consoles.push(None);
                        self.io.log_consoles.len() - 1
                    }
                };
                self.io.log_consoles[slot] = Some(console);
                self.r0 = Value::Int(slot as i32 + 1);
            }
            // logConsoleClose(handle)
            31 => {
                Self::check_argc(argc, 1)?;
                let slot = self.log_console_arg(self.arg(0))?;
                if let Some(mut console) = self.io.log_consoles[slot].take() {
                    console.formatter.flush(Newline::None);
                    let output = console.formatter.take_output();
                    let _ = self.host.append_file(&console.name, output.as_bytes());
                }
                self.r0 = Value::Nil;
            }
            // logConsoleSay(handle, ...)
            32 => {
                Self::check_argc_range(argc, 1, 32767)?;
                let handle = self.int_arg(self.arg(0))?;
                // Handle -1 is the transcript.
                let slot = if handle == -1 {
                    None
                } else {
                    Some(self.log_console_arg(self.arg(0))?)
                };
                for index in 1..argc as usize {
                    let Some(text) = self.say_text(self.arg(index))? else {
                        continue;
                    };
                    match slot {
                        Some(slot) => {
                            if let Some(console) = &mut self.io.log_consoles[slot] {
                                console.formatter.format_text(&text);
                                let output = console.formatter.take_output();
                                let _ = self.host.append_file(&console.name, output.as_bytes());
                            }
                        }
                        None => {
                            if self.log_enabled() {
                                if let Some(log) = &mut self.io.log {
                                    log.formatter.format_text(&text);
                                }
                                self.send_log();
                            }
                        }
                    }
                }
            }
            // logInputEvent([type, parameter?])
            33 => {
                Self::check_argc(argc, 1)?;
                let event = self.arg(0);
                let Some(len) = self.list_like_len(event)? else {
                    return Err(VmError::new(code::BAD_TYPE_BIF));
                };
                if len < 1 {
                    return Err(bad_value());
                }
                let kind = self.list_like_get(event, 1)?;
                let param = if len >= 2 {
                    let value = self.list_like_get(event, 2)?;
                    Some(self.as_str(value).ok_or_else(bad_value)?.to_vec())
                } else {
                    None
                };
                if let Some(text) = self.as_str(kind) {
                    let mut tag = text.to_vec();
                    truncate(&mut tag, 127);
                    self.log_event(&tag, param.as_deref());
                } else if matches!(kind, Value::Int(_)) || self.as_big(kind).is_some() {
                    let code = self.cast_to_int(kind)?;
                    self.log_event(event_tag(code).as_bytes(), param.as_deref());
                } else {
                    return Err(bad_value());
                }
                self.r0 = Value::Nil;
            }
            _ => {
                return Ok(false);
            }
        }
        Ok(true)
    }

    /// The optional timeout argument of an input function: none given, or
    /// nil, is no timeout.
    fn timeout_arg(&mut self, argc: u32) -> VmResult<Option<i32>> {
        if argc == 0 || self.arg(0) == Value::Nil {
            Ok(None)
        } else {
            self.cast_to_int(self.arg(0)).map(Some)
        }
    }

    /// The slot of the log console a handle names.
    fn log_console_arg(&self, value: Value) -> VmResult<usize> {
        let handle = self.int_arg(value)?;
        usize::try_from(handle)
            .ok()
            .and_then(|handle| handle.checked_sub(1))
            .filter(|&slot| matches!(self.io.log_consoles.get(slot), Some(Some(_))))
            .ok_or_else(|| VmError::new(code::BAD_VAL_BIF))
    }

    /// The labels of a dialog's buttons: a standard set by number, or a
    /// list of labels, each a string or the number of a standard label.
    fn dialog_buttons(&mut self, value: Value) -> VmResult<Vec<String>> {
        if let Value::Int(set) = value {
            let labels: &[&str] = match set {
                1 => &["&OK"],
                2 => &["&OK", "&Cancel"],
                3 => &["&Yes", "&No"],
                4 => &["&Yes", "&No", "&Cancel"],
                _ => &[],
            };
            return Ok(labels.iter().map(|label| (*label).to_owned()).collect());
        }
        let Some(len) = self.list_like_len(value)? else {
            return Err(VmError::new(code::BAD_TYPE_BIF));
        };
        let mut labels = Vec::new();
        for index in 1..=len.min(10) {
            let item = self.list_like_get(value, index)?;
            if let Some(text) = self.as_str(item) {
                labels.push(String::from_utf8_lossy(&text).into_owned());
            } else if let Value::Int(standard) = item {
                let label = match standard {
                    1 => "&OK",
                    2 => "&Cancel",
                    3 => "&Yes",
                    4 => "&No",
                    _ => continue,
                };
                labels.push(label.to_owned());
            }
        }
        Ok(labels)
    }
}
