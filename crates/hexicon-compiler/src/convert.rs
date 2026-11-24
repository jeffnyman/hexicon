//! Converting TADS 3 source to Hexicon: what `hexicon convert` does to
//! each file.
//!
//! A source file keeps its text, except that the names Hexicon gives
//! the built-in functions ([`RENAMES`]) replace the TADS names, in code
//! and in comments but never in the text of a string, and a file whose
//! code uses one includes `<hexicon.h>`. In a file that includes adv3Lite's
//! header (`advlite.h`), its macros for actions and their handlers
//! become Hexicon's declarations ([`FORMS`]). A project file (`.t3m`) becomes
//! a game's manifest and a library (`.tl`) a library's, each saying what
//! the original said. The converted program compiles to the same image
//! as the original: that is how the converter is checked.

use std::collections::HashSet;
use std::fmt::Write as _;

use crate::project::parse_opt_file;

/// The TADS names of the built-in functions and their constants, and the
/// names `hexicon.h` gives them.
pub const RENAMES: &[(&str, &str)] = &[
    ("t3RunGC", "collectGarbage"),
    ("t3SetSay", "setSay"),
    ("t3GetVMVsn", "vmVersion"),
    ("t3GetVMID", "vmId"),
    ("t3GetVMBanner", "vmBanner"),
    ("t3GetVMPreinitMode", "inPreinit"),
    ("t3DebugTrace", "debugTrace"),
    ("t3GetGlobalSymbols", "globalSymbols"),
    ("t3AllocProp", "newProperty"),
    ("t3GetStackTrace", "stackTrace"),
    ("t3GetNamedArg", "namedArg"),
    ("t3GetNamedArgList", "namedArgList"),
    ("tadsSay", "print"),
    ("T3DebugCheck", "DebugTraceCheck"),
    ("T3DebugBreak", "DebugTraceBreak"),
    ("T3DebugLog", "DebugTraceLog"),
    ("T3SetSayNoFunc", "SetSayNoFunc"),
    ("T3SetSayNoMethod", "SetSayNoMethod"),
    ("T3GlobalSymbols", "GlobalSymbolTable"),
    ("T3PreprocMacros", "MacroTable"),
    ("T3GetStackLocals", "StackTraceLocals"),
    ("T3GetStackDesc", "StackTraceDesc"),
    ("T3MacroHasArgs", "MacroHasArgs"),
    ("T3MacroHasVarargs", "MacroHasVarargs"),
];

/// adv3Lite's macros that define actions, each with the objects the
/// action takes and its class: `DefineTIAction(PutIn)` becomes
/// `action PutIn(dobj, iobj): TIAction`. A class of `None` is the macro's
/// second argument and those after it (`DefineTActionSub(Take, Cls)`,
/// `DefineAction(Jump, IAction)`).
pub const FORMS: &[(&str, &[&str], Option<&str>)] = &[
    ("DefineAction", &[], None),
    ("DefineSystemAction", &[], Some("SystemAction")),
    ("DefineIAction", &[], Some("IAction")),
    ("DefineLiteralAction", &[], Some("LiteralAction")),
    ("DefineTopicAction", &[], Some("TopicAction")),
    ("DefineNumericAction", &[], Some("NumericAction")),
    ("DefineTAction", &["dobj"], Some("TAction")),
    ("DefineTActionSub", &["dobj"], None),
    ("DefineLiteralTAction", &["dobj"], Some("LiteralTAction")),
    ("DefineTopicTAction", &["dobj"], Some("TopicTAction")),
    ("DefineNumericTAction", &["dobj"], Some("NumericTAction")),
    ("DefineTIAction", &["dobj", "iobj"], Some("TIAction")),
    ("DefineTIActionSub", &["dobj", "iobj"], None),
    (
        "DefineTIAAction",
        &["dobj", "iobj", "aobj"],
        Some("TIAAction"),
    ),
    ("DefineTIAActionSub", &["dobj", "iobj", "aobj"], None),
];

/// adv3Lite's macros for an action's handlers for one of its objects,
/// the macro that handles it as another action's, and the role Hexicon
/// writes: `dobjFor(Search) asDobjFor(LookIn)` is `dobj Search as LookIn`.
const HANDLERS: &[(&str, &str, &str)] = &[
    ("dobjFor", "asDobjFor", "dobj"),
    ("iobjFor", "asIobjFor", "iobj"),
    ("aobjFor", "asAobjFor", "aobj"),
];

/// Where a name was found in a source file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Name {
    start: usize,
    end: usize,
    /// In code, or in a comment.
    in_code: bool,
    /// In a directive (`#define`), whose code is a macro's.
    in_directive: bool,
}

/// A reader of a source file's text that finds the names in its code
/// and its comments, and passes over the text of its strings. It knows
/// what the language's tokenizer knows of where those begin and end:
/// comments, strings in single, double, and triple quotes, regular
/// expressions (`R'...'`), which have no escapes, the expressions
/// embedded in strings (`<<...>>`), and the directives whose text is
/// not code (`#include`, `#error`).
struct Scanner<'t> {
    text: &'t [u8],
    pos: usize,
    names: Vec<Name>,
    /// In a directive, which runs to the end of its line and over the
    /// newlines a backslash splices.
    in_directive: bool,
}

fn is_name_start(c: u8) -> bool {
    c.is_ascii_alphabetic() || c == b'_'
}

fn is_name_char(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_'
}

impl Scanner<'_> {
    fn at(&self, offset: usize) -> u8 {
        self.text.get(self.pos + offset).copied().unwrap_or(0)
    }

    fn done(&self) -> bool {
        self.pos >= self.text.len()
    }

    fn name(&mut self, in_code: bool) {
        let start = self.pos;
        while !self.done() && is_name_char(self.at(0)) {
            self.pos += 1;
        }
        self.names.push(Name {
            start,
            end: self.pos,
            in_code,
            in_directive: self.in_directive,
        });
    }

    /// Comment text up to `end` (or to the end of the line, if `end` is
    /// a newline), its names noted.
    fn comment(&mut self, end: &[u8]) {
        while !self.done() && !self.text[self.pos..].starts_with(end) {
            if is_name_start(self.at(0))
                && (self.pos == 0 || !is_name_char(self.text[self.pos - 1]))
            {
                self.name(false);
            } else {
                self.pos += 1;
            }
        }
        if end != b"\n" {
            self.pos = (self.pos + end.len()).min(self.text.len());
        }
    }

    /// Skip to the end of the line, past a newline that a backslash
    /// splices to the next.
    fn rest_of_line(&mut self) {
        while !self.done() && self.at(0) != b'\n' {
            self.pos += 1;
        }
    }

    /// Code, up to the end of the text or, if `embedded`, the `>>` that
    /// ends an expression embedded in a string.
    fn code(&mut self, embedded: bool) {
        let text = self.text;
        let mut line_start = !embedded;
        let mut prev_name: &[u8] = b"";
        while !self.done() {
            let c = self.at(0);
            if embedded && c == b'>' && self.at(1) == b'>' {
                self.pos += 2;
                return;
            }
            match c {
                b'\n' => {
                    let before = text[..self.pos].trim_ascii_end();
                    if !before.ends_with(b"\\") || text[..self.pos].ends_with(b"\n") {
                        self.in_directive = false;
                    }
                    line_start = !embedded;
                    self.pos += 1;
                    prev_name = b"";
                    continue;
                }
                b' ' | b'\t' | b'\r' | 0x0c => {
                    self.pos += 1;
                    continue;
                }
                b'#' if line_start => {
                    self.in_directive = true;
                    self.pos += 1;
                    while matches!(self.at(0), b' ' | b'\t') {
                        self.pos += 1;
                    }
                    let start = self.pos;
                    while is_name_char(self.at(0)) {
                        self.pos += 1;
                    }
                    let directive = &text[start..self.pos];
                    if matches!(directive, b"include" | b"error" | b"charset" | b"line") {
                        self.rest_of_line();
                    }
                }
                b'/' if self.at(1) == b'/' => {
                    self.pos += 2;
                    self.comment(b"\n");
                }
                b'/' if self.at(1) == b'*' => {
                    self.pos += 2;
                    self.comment(b"*/");
                }
                b'\'' | b'"' => {
                    let raw = prev_name == b"R";
                    self.string(c, raw);
                }
                _ if is_name_start(c) => {
                    let start = self.pos;
                    self.name(true);
                    prev_name = &text[start..self.pos];
                    line_start = false;
                    continue;
                }
                _ if c.is_ascii_digit() => {
                    while is_name_char(self.at(0)) || self.at(0) == b'.' {
                        self.pos += 1;
                    }
                }
                _ => self.pos += 1,
            }
            line_start = false;
            prev_name = b"";
        }
    }

    /// A string, from its opening quote.
    fn string(&mut self, quote: u8, raw: bool) {
        let triple = self.at(1) == quote && self.at(2) == quote;
        let width = if triple { 3 } else { 1 };
        self.pos += width;
        while !self.done() {
            let c = self.at(0);
            if c == b'\\' && !raw {
                self.pos += 2;
            } else if c == quote && (!triple || (self.at(1) == quote && self.at(2) == quote)) {
                self.pos += width;
                return;
            } else if c == b'<' && self.at(1) == b'<' && !raw {
                self.pos += 2;
                self.code(true);
            } else {
                self.pos += 1;
            }
        }
    }
}

fn scan(text: &[u8]) -> Vec<Name> {
    let mut scanner = Scanner {
        text,
        pos: 0,
        names: Vec::new(),
        in_directive: false,
    };
    scanner.code(false);
    scanner.names
}

/// The names a source file's code uses, outside its comments and the
/// text of its strings.
pub fn code_names(text: &[u8]) -> HashSet<String> {
    scan(text)
        .into_iter()
        .filter(|name| name.in_code)
        .map(|name| String::from_utf8_lossy(&text[name.start..name.end]).into_owned())
        .collect()
}

/// The renames to make in a program whose code uses `used`: those
/// whose new name it does not use already, and the ones left out.
pub fn usable_renames(used: &HashSet<String>) -> (Vec<(&'static str, &'static str)>, Vec<String>) {
    let mut renames = Vec::new();
    let mut skipped = Vec::new();
    for &(old, new) in RENAMES {
        if used.contains(new) {
            // A program converted already uses the new names and not
            // the old: there is nothing to say.
            if !used.contains(old) {
                continue;
            }
            skipped.push(format!(
                "{old} is not renamed {new}: the program already uses the name {new}"
            ));
        } else {
            renames.push((old, new));
        }
    }
    (renames, skipped)
}

/// A source file converted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConvertedSource {
    /// Its new text.
    pub text: Vec<u8>,
    /// How many names were renamed, in code and in comments.
    pub renamed: usize,
    /// How many actions and handlers were written as Hexicon's
    /// declarations.
    pub declarations: usize,
}

/// Whether a file includes adv3Lite's header, whose macros for actions
/// and handlers it may then use.
fn includes_advlite(text: &[u8]) -> bool {
    text.split(|&c| c == b'\n').any(|line| {
        let line = line.trim_ascii_start();
        line.strip_prefix(b"#").is_some_and(|rest| {
            let rest = rest.trim_ascii_start();
            rest.starts_with(b"include")
                && rest
                    .to_ascii_lowercase()
                    .windows(9)
                    .any(|w| w == b"advlite.h")
        })
    })
}

/// The arguments of a macro call at `pos` (after its name), each a name,
/// and where the call ends; or nothing, if that is not what is there.
fn call_args(text: &[u8], mut pos: usize) -> Option<(Vec<&[u8]>, usize)> {
    let skip = |pos: &mut usize| {
        while matches!(text.get(*pos), Some(b' ' | b'\t')) {
            *pos += 1;
        }
    };
    skip(&mut pos);
    if text.get(pos) != Some(&b'(') {
        return None;
    }
    pos += 1;
    let mut args = Vec::new();
    loop {
        skip(&mut pos);
        let start = pos;
        if !text.get(pos).copied().is_some_and(is_name_start) {
            return None;
        }
        while text.get(pos).copied().is_some_and(is_name_char) {
            pos += 1;
        }
        args.push(&text[start..pos]);
        skip(&mut pos);
        match text.get(pos) {
            Some(b',') => pos += 1,
            Some(b')') => return Some((args, pos + 1)),
            _ => return None,
        }
    }
}

/// The edits that make adv3Lite's macros for actions and handlers
/// Hexicon's declarations: each the span of text replaced, and what
/// replaces it.
fn declaration_edits(text: &[u8], names: &[Name]) -> Vec<(usize, usize, Vec<u8>)> {
    let mut edits = Vec::new();
    let mut next_free = 0;
    for name in names {
        if !name.in_code || name.in_directive || name.start < next_free {
            continue;
        }
        let word = &text[name.start..name.end];
        if let Some(&(_, redirect, role)) = HANDLERS.iter().find(|(m, _, _)| m.as_bytes() == word) {
            let Some((args, end)) = call_args(text, name.end) else {
                continue;
            };
            let [action] = args[..] else {
                continue;
            };
            let mut new = [role.as_bytes(), b" ", action].concat();
            let mut stop = end;
            // `dobjFor(Search) asDobjFor(LookIn)`: the same role, handled
            // as another action.
            let mut after = end;
            while text.get(after).is_some_and(u8::is_ascii_whitespace) {
                after += 1;
            }
            if text[after..].starts_with(redirect.as_bytes())
                && !text
                    .get(after + redirect.len())
                    .copied()
                    .is_some_and(is_name_char)
                && let Some((other, other_end)) = call_args(text, after + redirect.len())
                && let [other] = other[..]
            {
                new.extend_from_slice(&text[end..after]);
                new.extend_from_slice(b"as ");
                new.extend_from_slice(other);
                stop = other_end;
            }
            edits.push((name.start, stop, new));
            next_free = stop;
        } else if let Some(&(_, roles, class)) = FORMS.iter().find(|(m, _, _)| m.as_bytes() == word)
        {
            let Some((args, end)) = call_args(text, name.end) else {
                continue;
            };
            let (action, classes): (&[u8], Vec<&[u8]>) = match (class, &args[..]) {
                (Some(class), [action]) => (action, vec![class.as_bytes()]),
                (None, [action, classes @ ..]) if !classes.is_empty() => (action, classes.to_vec()),
                _ => continue,
            };
            let mut new = [b"action ", action].concat();
            if !roles.is_empty() {
                new.push(b'(');
                new.extend_from_slice(roles.join(", ").as_bytes());
                new.push(b')');
            }
            new.extend_from_slice(b": ");
            new.extend_from_slice(&classes.join(&b", "[..]));
            edits.push((name.start, end, new));
            next_free = end;
        }
    }
    edits
}

/// Convert a source file or a header: the names in `renames` replaced in
/// its code and comments, and `#include <hexicon.h>` added if its code
/// uses one of them; and in a file that includes adv3Lite's header, its
/// macros for actions and handlers written as Hexicon's declarations.
pub fn convert_source(text: &[u8], renames: &[(&str, &str)]) -> ConvertedSource {
    let names = scan(text);
    let mut edits: Vec<(usize, usize, Vec<u8>)> = Vec::new();
    let mut renamed = 0;
    let mut in_code = false;
    for name in &names {
        let word = &text[name.start..name.end];
        let Some(&(_, new)) = renames.iter().find(|(old, _)| old.as_bytes() == word) else {
            continue;
        };
        edits.push((name.start, name.end, new.as_bytes().to_vec()));
        renamed += 1;
        in_code |= name.in_code;
    }
    let mut declarations = 0;
    if includes_advlite(text) {
        let forms = declaration_edits(text, &names);
        declarations = forms.len();
        edits.extend(forms);
    }
    edits.sort_by_key(|edit| edit.0);
    let mut out = Vec::with_capacity(text.len());
    let mut last = 0;
    for (start, end, new) in edits {
        if start < last {
            continue;
        }
        out.extend_from_slice(&text[last..start]);
        out.extend_from_slice(&new);
        last = end;
    }
    out.extend_from_slice(&text[last..]);
    if in_code && !contains(&out, b"<hexicon.h>") {
        out = with_include(&out);
    }
    ConvertedSource {
        text: out,
        renamed,
        declarations,
    }
}

fn contains(text: &[u8], what: &[u8]) -> bool {
    text.windows(what.len()).any(|w| w == what)
}

/// `text` with `#include <hexicon.h>` after its first `#include`, or
/// else after a `#charset` on its first line, or else first.
fn with_include(text: &[u8]) -> Vec<u8> {
    let mut lines = Vec::new();
    let mut start = 0;
    for (i, &c) in text.iter().enumerate() {
        if c == b'\n' {
            lines.push((start, i + 1));
            start = i + 1;
        }
    }
    if start < text.len() {
        lines.push((start, text.len()));
    }
    let directive = |&(s, e): &(usize, usize), word: &[u8]| {
        let line = text[s..e].trim_ascii_start();
        line.strip_prefix(b"#")
            .is_some_and(|rest| rest.trim_ascii_start().starts_with(word))
    };
    let after = lines
        .iter()
        .find(|line| directive(line, b"include"))
        .or_else(|| lines.first().filter(|line| directive(line, b"charset")));
    let (at, newline): (usize, &[u8]) = match after {
        Some(&(s, e)) => {
            let crlf = text[s..e].ends_with(b"\r\n");
            let ends = text[s..e].ends_with(b"\n");
            match (ends, crlf) {
                (true, true) => (e, b"\r\n"),
                (true, false) => (e, b"\n"),
                // The last line, with no newline after it.
                _ => (e, b"\n"),
            }
        }
        None => (0, b"\n"),
    };
    let mut out = Vec::with_capacity(text.len() + 24);
    out.extend_from_slice(&text[..at]);
    if at > 0 && !text[..at].ends_with(b"\n") {
        out.extend_from_slice(newline);
    }
    out.extend_from_slice(b"#include <hexicon.h>");
    out.extend_from_slice(newline);
    out.extend_from_slice(&text[at..]);
    out
}

/// A string written as TOML writes one.
fn quote(text: &str) -> String {
    let mut out = String::from("\"");
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            c if c.is_control() => {
                let _ = write!(out, "\\u{:04X}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// A key written as TOML writes one: bare if it can be.
fn key(text: &str) -> String {
    if !text.is_empty()
        && text
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        text.to_owned()
    } else {
        quote(text)
    }
}

fn list(items: &[String]) -> String {
    if items.len() <= 3 && items.iter().map(String::len).sum::<usize>() < 60 {
        let quoted: Vec<String> = items.iter().map(|item| quote(item)).collect();
        return format!("[{}]", quoted.join(", "));
    }
    let mut out = String::from("[\n");
    for item in items {
        let _ = writeln!(out, "    {},", quote(item));
    }
    out.push(']');
    out
}

/// A source or a library in the order a project or a library gives them.
enum Part {
    Source(String),
    Library(String, Vec<String>),
}

fn write_modules(out: &mut String, parts: &[Part], libraries_first: bool) {
    let first_source = parts.iter().position(|p| matches!(p, Part::Source(_)));
    let first_library = parts.iter().position(|p| matches!(p, Part::Library(..)));
    let last_source = parts.iter().rposition(|p| matches!(p, Part::Source(_)));
    let last_library = parts.iter().rposition(|p| matches!(p, Part::Library(..)));
    let in_order = match (first_source, last_source, first_library, last_library) {
        (Some(fs), Some(ls), Some(fl), Some(ll)) => {
            if libraries_first {
                ll < fs
            } else {
                ls < fl
            }
        }
        _ => true,
    };
    let library = |name: &str, exclude: &[String], key_name: &str| {
        if exclude.is_empty() && key_name == "name" {
            quote(name)
        } else if exclude.is_empty() {
            format!("{{ {key_name} = {} }}", quote(name))
        } else {
            format!(
                "{{ {key_name} = {}, exclude = {} }}",
                quote(name),
                list(exclude)
            )
        }
    };
    if in_order {
        let sources: Vec<String> = parts
            .iter()
            .filter_map(|p| match p {
                Part::Source(s) => Some(s.clone()),
                Part::Library(..) => None,
            })
            .collect();
        let libraries: Vec<String> = parts
            .iter()
            .filter_map(|p| match p {
                Part::Library(name, exclude) => Some(library(name, exclude, "name")),
                Part::Source(_) => None,
            })
            .collect();
        let libs = if libraries.is_empty() {
            String::new()
        } else if libraries.len() <= 3 {
            format!("libraries = [{}]\n", libraries.join(", "))
        } else {
            let mut text = String::from("libraries = [\n");
            for lib in &libraries {
                let _ = writeln!(text, "    {lib},");
            }
            text.push_str("]\n");
            text
        };
        let srcs = if sources.is_empty() {
            String::new()
        } else {
            format!("sources = {}\n", list(&sources))
        };
        if libraries_first {
            out.push_str(&libs);
            out.push_str(&srcs);
        } else {
            out.push_str(&srcs);
            out.push_str(&libs);
        }
    } else {
        out.push_str("modules = [\n");
        for part in parts {
            let item = match part {
                Part::Source(s) => quote(s),
                Part::Library(name, exclude) => library(name, exclude, "library"),
            };
            let _ = writeln!(out, "    {item},");
        }
        out.push_str("]\n");
    }
}

/// A source's name as a manifest gives it: without `.t` if it is one of
/// the files being converted, which become `.hxn` files, and with it
/// otherwise.
fn source_entry(name: &str, is_local: &mut dyn FnMut(&str) -> bool) -> String {
    let file = name.rsplit(['/', '\\']).next().unwrap_or(name);
    let (stem, has_ext) = match file.rsplit_once('.') {
        Some((_, ext)) if ext.eq_ignore_ascii_case("t") => (&name[..name.len() - 2], true),
        Some(_) => return name.to_owned(),
        None => (name, false),
    };
    if is_local(&format!("{stem}.t")) {
        stem.to_owned()
    } else if has_ext {
        name.to_owned()
    } else {
        format!("{name}.t")
    }
}

/// What a conversion made, and what it could not carry over.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConvertedManifest {
    /// The manifest.
    pub text: String,
    /// What was left out, and why.
    pub notes: Vec<String>,
}

/// Convert a project file (`.t3m`) to a game's manifest. `name` is the
/// game's name; `is_local` says whether a source, named as a path
/// relative to the project's folder with `.t` after it, is one of the
/// files being converted.
pub fn convert_project(
    text: &[u8],
    name: &str,
    is_local: &mut dyn FnMut(&str) -> bool,
) -> ConvertedManifest {
    let args = parse_opt_file(text);
    let mut notes = Vec::new();
    let mut build: Vec<String> = Vec::new();
    let mut defines: Vec<String> = Vec::new();
    let mut include_paths = Vec::new();
    let mut library_paths = Vec::new();
    let mut parts: Vec<Part> = Vec::new();
    let mut resources = Vec::new();
    let mut image = None;
    let mut i = 0;
    // The value of an option: the rest of the word, or the next one.
    let value = |i: &mut usize, len: usize| -> String {
        let arg = &args[*i];
        if arg.len() > len {
            arg[len..].to_owned()
        } else {
            *i += 1;
            args.get(*i).cloned().unwrap_or_default()
        }
    };
    while i < args.len() {
        let arg = args[i].clone();
        match arg.as_str() {
            "-res" => {
                resources.extend(args[i + 1..].iter().cloned());
                break;
            }
            "-source" => {
                i += 1;
                let source = args.get(i).cloned().unwrap_or_default();
                parts.push(Part::Source(source_entry(&source, is_local)));
            }
            "-lib" => {
                i += 1;
                let lib = args.get(i).cloned().unwrap_or_default();
                parts.push(Part::Library(strip_tl(&lib), Vec::new()));
            }
            "-x" => {
                i += 1;
                let module = args.get(i).cloned().unwrap_or_default();
                match parts.last_mut() {
                    Some(Part::Library(_, exclude)) => exclude.push(module),
                    _ => notes.push(format!("-x {module} follows no library, and is left out")),
                }
            }
            "-pre" => build.push("preinit = true".to_owned()),
            "-nopre" => build.push("preinit = false".to_owned()),
            "-nodef" => build.push("system-main = false".to_owned()),
            "-d" => build.push("debug = true".to_owned()),
            "-v" => build.push("verbose = true".to_owned()),
            "-w0" => build.push("warnings = false".to_owned()),
            "-we" | "-we+" => build.push("warnings-as-errors = true".to_owned()),
            "-Gstg" => build.push("source-text-group = true".to_owned()),
            "-w1" | "-we-" | "-a" | "-al" | "-q" | "-nobanner" | "-errnum" | "-errnum+"
            | "-errnum-" | "-statpct" | "-FC" => {}
            "-statprefix" => {
                value(&mut i, 11);
            }
            _ if arg.starts_with("-D") => {
                let def = value(&mut i, 2);
                match def.split_once('=') {
                    Some((n, v)) => defines.push(format!("{} = {}", key(n), quote(v))),
                    None => defines.push(format!("{} = true", key(&def))),
                }
            }
            _ if arg.starts_with("-U") => {
                let undef = value(&mut i, 2);
                defines.push(format!("{} = false", key(&undef)));
            }
            _ if arg.starts_with("-I") => include_paths.push(value(&mut i, 2)),
            _ if arg.starts_with("-Fs") => library_paths.push(value(&mut i, 3)),
            _ if arg.starts_with("-Fy") || arg.starts_with("-Fo") => {
                value(&mut i, 3);
            }
            _ if arg.starts_with("-cs") => {
                let charset = value(&mut i, 3);
                build.push(format!("charset = {}", quote(&charset)));
            }
            _ if arg.starts_with("-o") => image = Some(value(&mut i, 2)),
            _ if arg.starts_with("-FL") || arg.starts_with("-FI") => {
                let path = value(&mut i, 3);
                notes.push(format!(
                    "{} {path} names where this computer keeps the system library, \
                     and is left out",
                    &arg[..3]
                ));
            }
            _ if arg.starts_with('-') => {
                notes.push(format!("the option {arg} is not carried over"));
            }
            _ => {
                if arg.len() > 3 && arg[arg.len() - 3..].eq_ignore_ascii_case(".tl") {
                    parts.push(Part::Library(strip_tl(&arg), Vec::new()));
                } else {
                    parts.push(Part::Source(source_entry(&arg, is_local)));
                }
            }
        }
        i += 1;
    }
    if !include_paths.is_empty() {
        build.push(format!("include-paths = {}", list(&include_paths)));
    }
    if !library_paths.is_empty() {
        build.push(format!("library-paths = {}", list(&library_paths)));
    }

    let mut out = String::from("[game]\n");
    let _ = writeln!(out, "name = {}", quote(name));
    write_modules(&mut out, &parts, true);
    if let Some(image) = image {
        let _ = writeln!(out, "image = {}", quote(&image));
    }
    if !resources.is_empty() {
        let _ = writeln!(out, "resources = {}", list(&resources));
    }
    if !defines.is_empty() {
        out.push_str("\n[defines]\n");
        for line in defines {
            out.push_str(&line);
            out.push('\n');
        }
    }
    if !build.is_empty() {
        out.push_str("\n[build]\n");
        for line in build {
            out.push_str(&line);
            out.push('\n');
        }
    }
    ConvertedManifest { text: out, notes }
}

fn strip_tl(name: &str) -> String {
    if name.len() > 3 && name[name.len() - 3..].eq_ignore_ascii_case(".tl") {
        name[..name.len() - 3].to_owned()
    } else {
        name.to_owned()
    }
}

/// Convert a manifest that names TADS 3 sources: each string that names
/// a `.t` file being converted (as `is_local` says, given the name
/// relative to the manifest's folder) loses its `.t`, since the file is
/// now a `.hxn` source. Comments and everything else are kept as they
/// are.
pub fn convert_manifest(text: &str, is_local: &mut dyn FnMut(&str) -> bool) -> (String, usize) {
    let mut out = String::with_capacity(text.len());
    let mut changed = 0;
    for line in text.split_inclusive('\n') {
        let mut rest = line;
        loop {
            // A comment runs to the end of the line; a string is in
            // double quotes, with backslash escapes.
            let Some(at) = rest.find(['#', '"']) else {
                out.push_str(rest);
                break;
            };
            out.push_str(&rest[..at]);
            if rest[at..].starts_with('#') {
                out.push_str(&rest[at..]);
                break;
            }
            let body = &rest[at + 1..];
            let mut end = None;
            let mut escaped = false;
            for (i, c) in body.char_indices() {
                match c {
                    _ if escaped => escaped = false,
                    '\\' => escaped = true,
                    '"' => {
                        end = Some(i);
                        break;
                    }
                    _ => {}
                }
            }
            let Some(end) = end else {
                out.push_str(&rest[at..]);
                break;
            };
            let content = &body[..end];
            let is_t = content.len() > 2
                && content[content.len() - 2..].eq_ignore_ascii_case(".t")
                && !content.contains('\\');
            if is_t && is_local(content) {
                out.push('"');
                out.push_str(&content[..content.len() - 2]);
                out.push('"');
                changed += 1;
            } else {
                out.push('"');
                out.push_str(content);
                out.push('"');
            }
            rest = &body[end + 1..];
        }
    }
    (out, changed)
}

/// Convert a library (`.tl`) to a library's manifest. `is_local` says
/// whether a source, named as a path relative to the library's folder
/// with `.t` after it, is one of the files being converted; one whose
/// name has a macro's value in it is taken to be.
pub fn convert_library(text: &[u8], is_local: &mut dyn FnMut(&str) -> bool) -> ConvertedManifest {
    let text = String::from_utf8_lossy(text);
    let mut notes = Vec::new();
    let mut name = None;
    let mut parts = Vec::new();
    let mut resources = Vec::new();
    let mut needs = Vec::new();
    let mut nodef = false;
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let name_end = line
            .find(|c: char| c == ':' || c.is_whitespace())
            .unwrap_or(line.len());
        let var = line[..name_end].to_ascii_lowercase();
        let Some(value) = line[name_end..].trim_start().strip_prefix(':') else {
            if var == "nodef" {
                nodef = true;
            } else {
                notes.push(format!("the line \"{line}\" is not carried over"));
            }
            continue;
        };
        let value = value.trim_start();
        match var.as_str() {
            "name" => name = Some(value.to_owned()),
            "source" => {
                let entry = if value.contains("$(") {
                    value.to_owned()
                } else {
                    source_entry(value, is_local)
                };
                parts.push(Part::Source(entry));
            }
            "library" => parts.push(Part::Library(value.to_owned(), Vec::new())),
            "resource" => resources.push(value.to_owned()),
            "needmacro" => {
                let mut words = value.splitn(2, char::is_whitespace);
                let macro_name = words.next().unwrap_or("").to_owned();
                let why = words.next().unwrap_or("").trim_start().to_owned();
                needs.push((macro_name, why));
            }
            _ => notes.push(format!("the line \"{line}\" is not carried over")),
        }
    }
    let mut out = String::from("[library]\n");
    if let Some(name) = name {
        let _ = writeln!(out, "name = {}", quote(&name));
    }
    write_modules(&mut out, &parts, false);
    if !resources.is_empty() {
        let _ = writeln!(out, "resources = {}", list(&resources));
    }
    if nodef {
        out.push_str("system-main = false\n");
    }
    if !needs.is_empty() {
        out.push_str("\n[needs]\n");
        for (macro_name, why) in needs {
            let _ = writeln!(out, "{} = {}", key(&macro_name), quote(&why));
        }
    }
    ConvertedManifest { text: out, notes }
}
