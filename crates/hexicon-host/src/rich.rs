//! The rich display's checker: turns what a program writes into a document
//! within Hexicon's profile of HTML and CSS.
//!
//! A program writes text with markup in it, in pieces, as it goes: HTML
//! TADS's tags (HTML 3.2 and its own, such as `<sound>` and `<bq>`), the
//! profile's additions (`class`, `style`, `<span>`, `<figure>`), named and
//! numbered entities, and the control characters strings compile to (`\n`,
//! `\b`, `\^`). The checker reads it a character at a time, so a tag or an
//! entity can be split between pieces, and gives out [`Item`]s:
//!
//! - TADS's own tags turned into the page's (`<bq>` is `<blockquote>`,
//!   `<nobr>` a `<span>` that does not wrap), or kept as elements the page
//!   handles itself (`<sound>`, `<title>`, `<aboutbox>`, `<body>`);
//! - only the elements and attributes the profile lists, and in `style`
//!   only the CSS properties it lists. Anything else is dropped: a script
//!   and what is in it, an `onclick`, a link or a picture that names
//!   anything outside the game's own resources;
//! - elements opened and closed in a well-formed order, however the
//!   program wrote them.
//!
//! Every rich host shows the same items, so the browser and the desktop
//! program show the same document. [`Item::markup`] writes an item as
//! text, which is how tests compare a game's document.

use std::collections::HashSet;

use crate::entities;

/// What a rich display can do, and what the player has chosen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RichDisplay {
    /// Pictures: JPEG, PNG, SVG, and WebP.
    pub pictures: bool,
    /// Sounds: WAV, Ogg, and MP3.
    pub sounds: bool,
    /// The player wants pictures shown.
    pub show_pictures: bool,
    /// The player wants sounds played.
    pub play_sounds: bool,
    /// The player wants music played.
    pub play_music: bool,
    /// The player wants links in the text.
    pub links: bool,
}

impl RichDisplay {
    /// A display that can do everything, with everything turned on.
    pub fn full() -> Self {
        RichDisplay {
            pictures: true,
            sounds: true,
            show_pictures: true,
            play_sounds: true,
            play_music: true,
            links: true,
        }
    }
}

/// An attribute, as the checker keeps it: a name in lower case, and its
/// value with entities replaced, or `None` for one written without a
/// value (`<img ismap>`).
pub type Attr = (String, Option<String>);

/// One piece of the checked document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Item {
    /// Text, with entities replaced and nothing to escape.
    Text(String),
    /// An element that holds others, opened.
    Open(String, Vec<Attr>),
    /// The element opened most recently, and not yet closed, closed.
    Close(String),
    /// An element that holds nothing: `<br>`, `<img>`, `<hr>`, `<area>`,
    /// and the page's own `<sound>`, `<body>`, and `<tab>`.
    Void(String, Vec<Attr>),
    /// The window's title.
    Title(String),
    /// A line the player typed, shown as it is.
    Input(String),
}

impl Item {
    /// The item as the profile's markup. Each line break, block, and
    /// input ends a line, so that a document reads, and compares, as a
    /// transcript does.
    pub fn markup(&self) -> String {
        match self {
            Item::Text(text) => escape(text, false),
            Item::Open(name, attrs) => format!("<{name}{}>", attrs_markup(attrs)),
            Item::Close(name) => {
                let end = if is_block(name) { "\n" } else { "" };
                format!("</{name}>{end}")
            }
            Item::Void(name, attrs) => {
                let end = if name == "br" || name == "hr" || name == "body" {
                    "\n"
                } else {
                    ""
                };
                format!("<{name}{}>{end}", attrs_markup(attrs))
            }
            Item::Title(text) => format!("<title>{}</title>\n", escape(text, false)),
            Item::Input(text) => format!("<kbd>{}</kbd>\n", escape(text, false)),
        }
    }
}

fn attrs_markup(attrs: &[Attr]) -> String {
    let mut out = String::new();
    for (name, value) in attrs {
        out.push(' ');
        out.push_str(name);
        if let Some(value) = value {
            out.push_str("=\"");
            out.push_str(&escape(value, true));
            out.push('"');
        }
    }
    out
}

fn escape(text: &str, attr: bool) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' if attr => out.push_str("&gt;"),
            '"' if attr => out.push_str("&quot;"),
            '\u{a0}' => out.push_str("&nbsp;"),
            _ => out.push(c),
        }
    }
    out
}

// ---- the profile ----

/// Elements that hold nothing.
const VOID: &[&str] = &[
    "br", "img", "hr", "area", "wbr", "sound", "body", "tab", "basefont",
];

/// Elements that hold others, and are kept as they are.
const CONTAINERS: &[&str] = &[
    // HTML 3.2, as HTML TADS has it.
    "a",
    "address",
    "b",
    "big",
    "blockquote",
    "caption",
    "center",
    "cite",
    "code",
    "dd",
    "dfn",
    "dir",
    "div",
    "dl",
    "dt",
    "em",
    "font",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "i",
    "kbd",
    "li",
    "map",
    "menu",
    "ol",
    "p",
    "pre",
    "q",
    "s",
    "samp",
    "small",
    "strike",
    "strong",
    "sub",
    "sup",
    "table",
    "td",
    "th",
    "tr",
    "tt",
    "u",
    "ul",
    "var",
    // The profile's additions.
    "span",
    "figure",
    "figcaption",
    // The page's own: what the about box shows, and a banner's text.
    "aboutbox",
    "banner",
];

/// Elements dropped with everything in them.
const DROPPED_WITH_CONTENTS: &[&str] = &[
    "script", "style", "iframe", "object", "embed", "noscript", "template", "frameset", "frame",
    "form", "select", "textarea", "button", "head", "svg", "math",
];

/// Attributes any kept element may have.
const GLOBAL_ATTRS: &[&str] = &["class", "style", "title", "lang", "dir"];

/// The attributes each element may have, besides the global ones.
fn element_attrs(name: &str) -> &'static [&'static str] {
    match name {
        "a" => &["href", "plain", "append", "noenter"],
        "img" => &[
            "src", "alt", "width", "height", "border", "align", "hspace", "vspace", "usemap",
            "ismap", "asrc", "hsrc",
        ],
        "area" => &[
            "shape", "coords", "href", "nohref", "alt", "append", "noenter",
        ],
        "map" => &["name"],
        "font" => &["face", "size", "color", "bgcolor"],
        "basefont" => &["size", "face", "color"],
        "table" => &[
            "border",
            "cellpadding",
            "cellspacing",
            "width",
            "height",
            "align",
            "bgcolor",
            "background",
        ],
        "tr" => &["align", "valign", "bgcolor", "background"],
        "td" | "th" => &[
            "align",
            "valign",
            "width",
            "height",
            "colspan",
            "rowspan",
            "nowrap",
            "bgcolor",
            "background",
        ],
        "caption" => &["align", "valign"],
        "p" | "div" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "center" => &["align"],
        "br" => &["clear", "height"],
        "hr" => &["width", "size", "align", "noshade", "src"],
        "ol" => &["start", "type", "compact"],
        "ul" | "dir" | "menu" => &["type", "compact"],
        "li" => &["type", "value"],
        "dl" => &["compact"],
        "body" => &[
            "bgcolor",
            "text",
            "link",
            "vlink",
            "alink",
            "hlink",
            "background",
        ],
        "sound" => &[
            "src",
            "layer",
            "repeat",
            "random",
            "cancel",
            "interrupt",
            "sequence",
            "alt",
            "volume",
            "fadein",
            "fadeout",
            "gapafter",
        ],
        "tab" => &["id", "to", "align", "indent", "multiple", "dp"],
        "banner" => &[
            "id",
            "align",
            "width",
            "height",
            "border",
            "remove",
            "removeall",
        ],
        _ => &[],
    }
}

/// Attributes that name one of the game's resources.
const RESOURCE_ATTRS: &[&str] = &["src", "asrc", "hsrc", "background"];

/// The CSS properties the profile lets `style` set.
const CSS_PROPERTIES: &[&str] = &[
    "color",
    "background",
    "background-color",
    "background-image",
    "background-repeat",
    "background-position",
    "background-size",
    "font",
    "font-family",
    "font-size",
    "font-weight",
    "font-style",
    "font-variant",
    "text-align",
    "text-decoration",
    "text-transform",
    "text-indent",
    "line-height",
    "letter-spacing",
    "word-spacing",
    "white-space",
    "margin",
    "margin-top",
    "margin-right",
    "margin-bottom",
    "margin-left",
    "padding",
    "padding-top",
    "padding-right",
    "padding-bottom",
    "padding-left",
    "border",
    "border-top",
    "border-right",
    "border-bottom",
    "border-left",
    "border-width",
    "border-style",
    "border-color",
    "border-radius",
    "width",
    "height",
    "min-width",
    "max-width",
    "min-height",
    "max-height",
    "display",
    "float",
    "clear",
    "list-style",
    "list-style-type",
    "list-style-position",
    "vertical-align",
    "flex-direction",
    "flex-wrap",
    "justify-content",
    "align-items",
    "gap",
];

/// The values `display` may have.
const DISPLAY_VALUES: &[&str] = &["block", "inline", "inline-block", "none", "flex"];

/// Elements that end a line, in the text form of a document.
fn is_block(name: &str) -> bool {
    matches!(
        name,
        "p" | "div"
            | "blockquote"
            | "center"
            | "h1"
            | "h2"
            | "h3"
            | "h4"
            | "h5"
            | "h6"
            | "ul"
            | "ol"
            | "dl"
            | "li"
            | "dt"
            | "dd"
            | "dir"
            | "menu"
            | "table"
            | "tr"
            | "pre"
            | "figure"
            | "figcaption"
            | "address"
            | "aboutbox"
            | "banner"
            | "caption"
    )
}

/// Elements that end an open paragraph when they start.
fn closes_paragraph(name: &str) -> bool {
    matches!(
        name,
        "p" | "div"
            | "blockquote"
            | "center"
            | "h1"
            | "h2"
            | "h3"
            | "h4"
            | "h5"
            | "h6"
            | "ul"
            | "ol"
            | "dl"
            | "dir"
            | "menu"
            | "table"
            | "pre"
            | "figure"
            | "address"
            | "hr"
    )
}

/// Whether a name is one of the game's resources: a relative path with
/// no address scheme, no root, and no way out of the game's own files.
fn is_resource_name(name: &str) -> bool {
    !name.is_empty()
        && !name.contains(':')
        && !name.contains('\\')
        && !name.starts_with('/')
        && !name.split('/').any(|part| part == "..")
}

/// Whether a link's target is a command, which is what a link in a game
/// is: words for the game to take as typed. An address on the web or
/// anywhere else is not.
fn is_command_link(href: &str) -> bool {
    let scheme_end = href.find(':');
    match scheme_end {
        None => true,
        // A colon after a space is part of the words ("say: hello").
        Some(i) => href[..i].contains(' ') || href[..i].is_empty(),
    }
}

/// The declarations of a `style` attribute that the profile allows, or
/// `None` if none are left.
fn check_style(style: &str) -> Option<String> {
    let kept = check_declarations(style, CSS_PROPERTIES);
    if kept.is_empty() {
        None
    } else {
        Some(kept.join("; "))
    }
}

/// The declarations, of those listed, that the profile allows, each as
/// `property: value`. A game's `!important` is taken out: the player's
/// own choices are the only ones that may come first.
fn check_declarations(declarations: &str, allowed: &[&str]) -> Vec<String> {
    let mut kept = Vec::new();
    for decl in declarations.split(';') {
        let Some((prop, value)) = decl.split_once(':') else {
            continue;
        };
        let prop = prop.trim().to_ascii_lowercase();
        let mut value = value.trim();
        if let Some(i) = value.to_ascii_lowercase().find("!important") {
            value = value[..i].trim_end();
        }
        if value.is_empty() || !allowed.contains(&prop.as_str()) {
            continue;
        }
        if !css_value_allowed(&prop, value) {
            continue;
        }
        kept.push(format!("{prop}: {value}"));
    }
    kept
}

/// What a `@font-face` rule may say.
const FONT_FACE_PROPERTIES: &[&str] = &[
    "font-family",
    "src",
    "font-weight",
    "font-style",
    "font-display",
    "unicode-range",
];

/// The words a `@media` query may use.
const MEDIA_WORDS: &[&str] = &[
    "all",
    "screen",
    "print",
    "and",
    "not",
    "only",
    "min-width",
    "max-width",
    "width",
    "orientation",
    "portrait",
    "landscape",
    "prefers-color-scheme",
    "light",
    "dark",
    "prefers-reduced-motion",
    "reduce",
    "no-preference",
    "prefers-contrast",
    "more",
    "less",
];

/// A game's style sheet, held to the profile, as text: each rule with
/// only the properties the profile lists, `@media` with only its
/// queries, `@font-face` with fonts among the game's resources, and every
/// other at-rule gone. Each selector is put under `scope`, the page's
/// element for the game, so a game styles its own text and not the
/// page's: `body`, `html`, and `:root` are `scope` itself.
pub fn check_stylesheet(css: &str, scope: &str) -> String {
    let css = strip_comments(css);
    let mut out = String::new();
    check_rules(&css, scope, &mut out, 0);
    out
}

fn strip_comments(css: &str) -> String {
    let mut out = String::with_capacity(css.len());
    let mut rest = css;
    while let Some(i) = rest.find("/*") {
        out.push_str(&rest[..i]);
        match rest[i + 2..].find("*/") {
            Some(j) => rest = &rest[i + 2 + j + 2..],
            None => return out,
        }
    }
    out.push_str(rest);
    out
}

fn check_rules(css: &str, scope: &str, out: &mut String, depth: usize) {
    let mut rest = css;
    loop {
        rest = rest.trim_start();
        if rest.is_empty() {
            return;
        }
        let brace = rest.find('{');
        // An at-rule that ends with ";" and has no block: @import,
        // @charset, @namespace. All go.
        if rest.starts_with('@') {
            if let Some(semi) = rest.find(';') {
                if brace.is_none_or(|b| semi < b) {
                    rest = &rest[semi + 1..];
                    continue;
                }
            }
        }
        let Some(open) = brace else {
            return;
        };
        let prelude = rest[..open].trim();
        let Some(len) = block_length(&rest[open..]) else {
            return;
        };
        let body = &rest[open + 1..open + len - 1];
        rest = &rest[open + len..];

        if let Some(at) = prelude.strip_prefix('@') {
            let (keyword, query) = at.split_once(char::is_whitespace).unwrap_or((at, ""));
            match keyword.to_ascii_lowercase().as_str() {
                "media" if depth < 4 && media_query_allowed(query) => {
                    let mut inner = String::new();
                    check_rules(body, scope, &mut inner, depth + 1);
                    if !inner.is_empty() {
                        out.push_str(&format!("@media {} {{\n{inner}}}\n", query.trim()));
                    }
                }
                "font-face" => {
                    let kept = check_declarations(body, FONT_FACE_PROPERTIES);
                    if kept.iter().any(|d| d.starts_with("src:")) {
                        out.push_str(&format!("@font-face {{ {} }}\n", kept.join("; ")));
                    }
                }
                _ => {}
            }
            continue;
        }

        let selectors = scope_selectors(prelude, scope);
        let kept = check_declarations(body, CSS_PROPERTIES);
        if !selectors.is_empty() && !kept.is_empty() {
            out.push_str(&format!("{selectors} {{ {} }}\n", kept.join("; ")));
        }
    }
}

/// The length of the block that starts at a "{", to its matching "}"
/// included.
fn block_length(css: &str) -> Option<usize> {
    let mut depth = 0;
    for (i, c) in css.char_indices() {
        match c {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i + 1);
                }
            }
            _ => {}
        }
    }
    None
}

fn media_query_allowed(query: &str) -> bool {
    query
        .split(|c: char| c.is_whitespace() || "(),:".contains(c))
        .filter(|word| !word.is_empty())
        .all(|word| {
            let word = word.to_ascii_lowercase();
            MEDIA_WORDS.contains(&word.as_str())
                || word.chars().all(|c| c.is_ascii_digit() || c == '.')
                || ["px", "em", "rem", "ch"].iter().any(|unit| {
                    word.strip_suffix(unit).is_some_and(|n| {
                        !n.is_empty() && n.chars().all(|c| c.is_ascii_digit() || c == '.')
                    })
                })
        })
}

/// Selectors put under the scope. One with anything that could reach
/// outside a selector is dropped.
fn scope_selectors(prelude: &str, scope: &str) -> String {
    let mut kept = Vec::new();
    for selector in prelude.split(',') {
        let selector = selector.trim();
        if selector.is_empty() || selector.contains(['<', '\\', ';', '@']) {
            continue;
        }
        let (first, rest) = match selector.find(|c: char| c.is_whitespace() || ">+~".contains(c)) {
            Some(i) => (&selector[..i], &selector[i..]),
            None => (selector, ""),
        };
        let first_lower = first.to_ascii_lowercase();
        let scoped = if ["body", "html", ":root"].contains(&first_lower.as_str()) {
            format!("{scope}{rest}")
        } else if let Some(more) = first_lower
            .strip_prefix("body")
            .filter(|m| m.starts_with(['.', ':', '[']))
        {
            // body.dark, body:hover: the scope with the rest of it.
            format!("{scope}{}{rest}", &first[first.len() - more.len()..])
        } else {
            format!("{scope} {selector}")
        };
        kept.push(scoped);
    }
    kept.join(", ")
}

fn css_value_allowed(prop: &str, value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    if lower.contains("expression")
        || lower.contains("javascript:")
        || lower.contains('\\')
        || lower.contains('<')
        || lower.contains("/*")
        || lower.contains('@')
    {
        return false;
    }
    if prop == "display" {
        let v = lower.trim_end_matches("!important").trim();
        return DISPLAY_VALUES.contains(&v);
    }
    // A url() names one of the game's resources, or the value goes.
    let mut rest = lower.as_str();
    let mut orig = value;
    while let Some(i) = rest.find("url(") {
        let after = &orig[i + 4..];
        let Some(close) = after.find(')') else {
            return false;
        };
        let target = after[..close].trim().trim_matches(['"', '\'']);
        if !is_resource_name(target) {
            return false;
        }
        rest = &rest[i + 4 + close + 1..];
        orig = &orig[i + 4 + close + 1..];
    }
    true
}

// ---- the checker ----

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    Text,
    /// In a tag: what has been read of it is in `tag`.
    Tag,
    /// In an entity: what has been read of it is in `entity`.
    Entity,
    /// In a comment, `<!-- ... -->`.
    Comment,
}

/// Reads what a program writes, and gives out the checked document.
#[derive(Debug)]
pub struct Checker {
    state: State,
    tag: String,
    quote: Option<char>,
    entity: String,
    /// Elements open now, innermost last.
    open: Vec<String>,
    /// Within an element dropped with its contents: its name, and how
    /// deep in it.
    dropping: Option<(String, usize)>,
    /// Within `<title>`: the title so far.
    title: Option<String>,
    /// Within `<log>`, whose text is for a transcript only.
    log_depth: usize,
    /// `\^` or `\v` is waiting for the next letter.
    case_next: Option<bool>,
    /// The last thing given out ended a line.
    at_line_start: bool,
    items: Vec<Item>,
    /// TADS's tags that stand for an element of the page's, open now:
    /// the names they were written with, so their end tags close them.
    aliases: Vec<(String, String)>,
    warned: HashSet<String>,
}

impl Default for Checker {
    fn default() -> Self {
        Self::new()
    }
}

impl Checker {
    /// A checker at the start of a document.
    pub fn new() -> Self {
        Checker {
            state: State::Text,
            tag: String::new(),
            quote: None,
            entity: String::new(),
            open: Vec::new(),
            dropping: None,
            title: None,
            log_depth: 0,
            case_next: None,
            at_line_start: true,
            items: Vec::new(),
            aliases: Vec::new(),
            warned: HashSet::new(),
        }
    }

    /// Read what the program wrote.
    pub fn write(&mut self, markup: &str) {
        for c in markup.chars() {
            self.read_char(c);
        }
    }

    /// A line the player typed, which is shown as it is and ends the line.
    pub fn input(&mut self, text: &str) {
        self.push(Item::Input(text.to_owned()));
        self.at_line_start = true;
    }

    /// The items given out since the last time.
    pub fn take(&mut self) -> Vec<Item> {
        std::mem::take(&mut self.items)
    }

    /// The names of elements that were dropped, once each: for a host
    /// that tells the author what the profile left out.
    pub fn dropped(&self) -> impl Iterator<Item = &str> {
        self.warned.iter().map(String::as_str)
    }

    fn read_char(&mut self, c: char) {
        match self.state {
            State::Tag => self.read_tag_char(c),
            State::Entity => self.read_entity_char(c),
            State::Comment => {
                self.tag.push(c);
                if self.tag.ends_with("-->") {
                    self.tag.clear();
                    self.state = State::Text;
                }
            }
            State::Text => match c {
                '<' => {
                    self.state = State::Tag;
                    self.tag.clear();
                    self.quote = None;
                }
                '&' => {
                    self.state = State::Entity;
                    self.entity.clear();
                }
                '\n' => self.line_break(),
                '\u{b}' => self.blank_line(),
                '\u{f}' => self.case_next = Some(true),
                '\u{e}' => self.case_next = Some(false),
                '\t' => self.text_char('\u{2003}'),
                '\u{15}' => self.text_char('\u{a0}'),
                c if (c as u32) < 0x20 => {}
                c => self.text_char(c),
            },
        }
    }

    fn read_tag_char(&mut self, c: char) {
        if self.tag.is_empty() && c != '/' && c != '!' && !c.is_ascii_alphabetic() {
            // Not a tag after all: a "<" in the text.
            self.state = State::Text;
            self.text_char('<');
            self.read_char(c);
            return;
        }
        if self.tag == "!--" {
            self.state = State::Comment;
            self.tag.clear();
            self.tag.push_str("--");
            self.read_char(c);
            return;
        }
        match self.quote {
            Some(q) if c == q => self.quote = None,
            Some(_) => {}
            None if c == '"' || c == '\'' => {
                // A quote starts a value only after "=".
                if self.tag.trim_end().ends_with('=') {
                    self.quote = Some(c);
                }
            }
            None if c == '>' => {
                let tag = std::mem::take(&mut self.tag);
                self.state = State::Text;
                self.tag_done(&tag);
                return;
            }
            None => {}
        }
        self.tag.push(c);
    }

    fn read_entity_char(&mut self, c: char) {
        if c == ';' {
            let name = std::mem::take(&mut self.entity);
            self.state = State::Text;
            match decode_entity(&name) {
                Some(ch) => self.text_char(ch),
                None => {
                    self.text_str(&format!("&{name};"));
                }
            }
            return;
        }
        let ok = c.is_ascii_alphanumeric() || (c == '#' && self.entity.is_empty());
        if ok && self.entity.len() < 12 {
            self.entity.push(c);
            return;
        }
        // Not an entity: the "&" and what followed it are text.
        let name = std::mem::take(&mut self.entity);
        self.state = State::Text;
        self.text_str(&format!("&{name}"));
        self.read_char(c);
    }

    // ---- text ----

    /// A character of text, in the case `\^` or `\v` asked for if one is
    /// waiting: which can be more than one character ("ß" in upper case is
    /// "SS").
    fn text_char(&mut self, c: char) {
        match self.case_next {
            Some(upper) if c.is_alphabetic() => {
                self.case_next = None;
                let changed: String = if upper {
                    c.to_uppercase().collect()
                } else {
                    c.to_lowercase().collect()
                };
                for c in changed.chars() {
                    self.text_char_raw(c);
                }
            }
            _ => self.text_char_raw(c),
        }
    }

    fn text_char_raw(&mut self, c: char) {
        if let Some(title) = &mut self.title {
            title.push(c);
            return;
        }
        if self.dropping.is_some() || self.log_depth > 0 {
            return;
        }
        // Spaces at the start of a line show nothing.
        if self.at_line_start && c == ' ' {
            return;
        }
        self.at_line_start = false;
        if let Some(Item::Text(text)) = self.items.last_mut() {
            text.push(c);
        } else {
            self.items.push(Item::Text(c.to_string()));
        }
    }

    fn text_str(&mut self, s: &str) {
        for c in s.chars() {
            self.text_char(c);
        }
    }

    fn line_break(&mut self) {
        if self.dropping.is_some() || self.log_depth > 0 || self.title.is_some() {
            return;
        }
        self.push(Item::Void("br".into(), Vec::new()));
        self.at_line_start = true;
    }

    fn blank_line(&mut self) {
        if self.dropping.is_some() || self.log_depth > 0 || self.title.is_some() {
            return;
        }
        if !self.at_line_start {
            self.push(Item::Void("br".into(), Vec::new()));
        }
        self.push(Item::Void("br".into(), Vec::new()));
        self.at_line_start = true;
    }

    fn push(&mut self, item: Item) {
        self.items.push(item);
    }

    // ---- tags ----

    fn tag_done(&mut self, tag: &str) {
        let (end, name, attrs) = parse_tag(tag);
        if name.is_empty() || name.starts_with('!') || name.starts_with('?') {
            return;
        }

        // Inside an element dropped with its contents, only its end
        // matters.
        if let Some((dropped, depth)) = &mut self.dropping {
            if *dropped == name {
                if end {
                    if *depth == 0 {
                        self.dropping = None;
                    } else {
                        *depth -= 1;
                    }
                } else {
                    *depth += 1;
                }
            }
            return;
        }

        match name.as_str() {
            "title" => {
                if end {
                    if let Some(title) = self.title.take() {
                        self.push(Item::Title(title.trim().to_owned()));
                        self.at_line_start = true;
                    }
                } else {
                    self.title = Some(String::new());
                }
                return;
            }
            "log" => {
                if end {
                    self.log_depth = self.log_depth.saturating_sub(1);
                } else {
                    self.log_depth += 1;
                }
                return;
            }
            // What is only kept out of a transcript is shown.
            "nolog" | "wrap" | "hiliteon" | "hiliteoff" | "noenter" => return,
            _ => {}
        }
        if self.title.is_some() || self.log_depth > 0 {
            return;
        }

        if DROPPED_WITH_CONTENTS.contains(&name.as_str()) {
            if !end {
                self.warned.insert(name.clone());
                self.dropping = Some((name, 0));
            }
            return;
        }

        // TADS's tags that are the page's under another name.
        let (page_name, extra_style): (&str, Option<&str>) = match name.as_str() {
            "bq" => ("blockquote", None),
            "credit" => ("cite", None),
            "listing" | "xmp" => ("pre", None),
            "nobr" => ("span", Some("white-space: nowrap")),
            other => (other, None),
        };
        let page_name = page_name.to_owned();

        if end {
            self.end_tag(&name, &page_name);
            return;
        }

        let is_void = VOID.contains(&page_name.as_str());
        if !is_void && !CONTAINERS.contains(&page_name.as_str()) {
            // Not in the profile: the element goes, what is in it stays.
            self.warned.insert(page_name);
            return;
        }

        let mut attrs = self.check_attrs(&page_name, attrs);
        if let Some(style) = extra_style {
            attrs.push(("style".into(), Some(style.to_owned())));
        }

        // A picture that is not one of the game's resources goes; its
        // words, if it has any, stay.
        if page_name == "img" && !attrs.iter().any(|(n, _)| n == "src") {
            if let Some((_, Some(alt))) = attrs.iter().find(|(n, _)| n == "alt") {
                let alt = alt.clone();
                self.text_str(&alt);
            }
            return;
        }
        if page_name == "sound" && !attrs.iter().any(|(n, _)| n == "src" || n == "cancel") {
            return;
        }

        if is_void {
            if closes_paragraph(&page_name) {
                self.close_paragraph();
            }
            let breaks = page_name == "br" || page_name == "hr";
            self.push(Item::Void(page_name, attrs));
            if breaks {
                self.at_line_start = true;
            }
            return;
        }

        if closes_paragraph(&page_name) {
            self.close_paragraph();
        }
        self.close_siblings(&page_name);
        if page_name != name {
            self.aliases.push((name, page_name.clone()));
        }
        self.open.push(page_name.clone());
        let starts_line = is_block(&page_name);
        self.push(Item::Open(page_name, attrs));
        if starts_line {
            self.at_line_start = true;
        }
    }

    /// An end tag: close the element it ends, and any still open inside
    /// it. One that ends nothing open is dropped.
    fn end_tag(&mut self, written: &str, page_name: &str) {
        let target = self
            .aliases
            .iter()
            .rev()
            .find(|(w, _)| w == written)
            .map(|(_, p)| p.clone())
            .unwrap_or_else(|| page_name.to_owned());
        let Some(at) = self.open.iter().rposition(|n| *n == target) else {
            return;
        };
        while self.open.len() > at {
            self.close_top();
        }
        if let Some(i) = self.aliases.iter().rposition(|(w, _)| w == written) {
            self.aliases.remove(i);
        }
    }

    fn close_top(&mut self) {
        if let Some(name) = self.open.pop() {
            let ends_line = is_block(&name);
            self.push(Item::Close(name));
            if ends_line {
                self.at_line_start = true;
            }
        }
    }

    /// A block starting ends a paragraph still open, as in HTML.
    fn close_paragraph(&mut self) {
        if let Some(at) = self.open.iter().rposition(|n| n == "p") {
            // Not across a table cell or a list item.
            if self.open[at..]
                .iter()
                .all(|n| !matches!(n.as_str(), "td" | "th" | "li" | "blockquote" | "div"))
            {
                while self.open.len() > at {
                    self.close_top();
                }
            }
        }
    }

    /// Elements that end the one before them of their kind: a list item,
    /// a table row or cell, a term or its definition.
    fn close_siblings(&mut self, name: &str) {
        let (ends, within): (&[&str], &[&str]) = match name {
            "li" => (&["li"], &["ul", "ol", "dir", "menu"]),
            "dt" | "dd" => (&["dt", "dd"], &["dl"]),
            "tr" => (&["tr", "td", "th"], &["table"]),
            "td" | "th" => (&["td", "th"], &["tr", "table"]),
            _ => return,
        };
        let Some(at) = self
            .open
            .iter()
            .rposition(|n| ends.contains(&n.as_str()) || within.contains(&n.as_str()))
        else {
            return;
        };
        if ends.contains(&self.open[at].as_str()) {
            while self.open.len() > at {
                self.close_top();
            }
        }
    }

    /// The attributes the profile lets an element keep, with what they
    /// name checked.
    fn check_attrs(&mut self, element: &str, attrs: Vec<Attr>) -> Vec<Attr> {
        let own = element_attrs(element);
        let mut kept = Vec::new();
        for (name, value) in attrs {
            if !GLOBAL_ATTRS.contains(&name.as_str()) && !own.contains(&name.as_str()) {
                if name.starts_with("on") {
                    self.warned.insert(format!("{element} {name}"));
                }
                continue;
            }
            let value = match (name.as_str(), value) {
                ("style", Some(v)) => match check_style(&v) {
                    Some(v) => Some(v),
                    None => continue,
                },
                ("href", Some(v)) => {
                    if !is_command_link(&v) {
                        self.warned.insert(format!("{element} href"));
                        continue;
                    }
                    Some(v)
                }
                (n, Some(v)) if RESOURCE_ATTRS.contains(&n) => {
                    if !is_resource_name(&v) {
                        self.warned.insert(format!("{element} {n}"));
                        continue;
                    }
                    Some(v)
                }
                (_, v) => v,
            };
            kept.push((name, value));
        }
        kept
    }
}

/// A tag's parts: whether it is an end tag, its name in lower case, and
/// its attributes.
fn parse_tag(tag: &str) -> (bool, String, Vec<Attr>) {
    let mut rest = tag.trim();
    let end = rest.starts_with('/');
    if end {
        rest = &rest[1..];
    }
    let rest = rest.trim_end_matches('/');
    let name_len = rest
        .find(|c: char| c.is_whitespace() || c == '/')
        .unwrap_or(rest.len());
    let name = rest[..name_len].to_ascii_lowercase();
    let mut attrs = Vec::new();
    let chars: Vec<char> = rest[name_len..].chars().collect();
    let mut i = 0;
    while i < chars.len() {
        while i < chars.len() && (chars[i].is_whitespace() || chars[i] == '/') {
            i += 1;
        }
        let start = i;
        while i < chars.len() && !chars[i].is_whitespace() && chars[i] != '=' {
            i += 1;
        }
        if start == i {
            i += 1;
            continue;
        }
        let attr: String = chars[start..i]
            .iter()
            .collect::<String>()
            .to_ascii_lowercase();
        while i < chars.len() && chars[i].is_whitespace() {
            i += 1;
        }
        if i < chars.len() && chars[i] == '=' {
            i += 1;
            while i < chars.len() && chars[i].is_whitespace() {
                i += 1;
            }
            let value: String = if i < chars.len() && (chars[i] == '"' || chars[i] == '\'') {
                let q = chars[i];
                i += 1;
                let vs = i;
                while i < chars.len() && chars[i] != q {
                    i += 1;
                }
                let v = chars[vs..i].iter().collect();
                i += 1;
                v
            } else {
                let vs = i;
                while i < chars.len() && !chars[i].is_whitespace() {
                    i += 1;
                }
                chars[vs..i].iter().collect()
            };
            attrs.push((attr, Some(decode_entities(&value))));
        } else {
            attrs.push((attr, None));
        }
    }
    (end, name, attrs)
}

/// The character an entity stands for: a name (`&mdash;`), or a number
/// (`&#8212;`, `&#x2014;`).
fn decode_entity(name: &str) -> Option<char> {
    if let Some(num) = name.strip_prefix('#') {
        let code = if let Some(hex) = num.strip_prefix(['x', 'X']) {
            u32::from_str_radix(hex, 16).ok()?
        } else {
            num.parse().ok()?
        };
        return char::from_u32(code);
    }
    entities::lookup(name)
}

/// A value with its entities replaced.
fn decode_entities(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut rest = value;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        let after = &rest[i + 1..];
        match after.find(';') {
            Some(j) if j <= 12 => match decode_entity(&after[..j]) {
                Some(c) => {
                    out.push(c);
                    rest = &after[j + 1..];
                }
                None => {
                    out.push('&');
                    rest = after;
                }
            },
            _ => {
                out.push('&');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

/// What happens on a rich display, in order: the main window's items, and
/// the panes'.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// Something for the main window.
    Main(Item),
    /// A pane was made.
    PaneCreated(u32),
    /// A pane was removed.
    PaneDeleted(u32),
    /// A pane's text was removed.
    PaneCleared(u32),
    /// Something for a text pane: what was written to it since it was
    /// last shown.
    Pane(u32, Vec<Item>),
}

impl Event {
    /// The event as text: a main window's item as [`Item::markup`]
    /// writes it, and a pane's as `<pane id="1">...</pane>`, on a line of
    /// its own.
    pub fn markup(&self) -> String {
        match self {
            Event::Main(item) => item.markup(),
            Event::PaneCreated(id) => format!("<pane id=\"{id}\" create></pane>\n"),
            Event::PaneDeleted(id) => format!("<pane id=\"{id}\" delete></pane>\n"),
            Event::PaneCleared(id) => format!("<pane id=\"{id}\" clear></pane>\n"),
            Event::Pane(id, items) => {
                let text: String = items.iter().map(Item::markup).collect();
                format!("<pane id=\"{id}\">{text}</pane>\n")
            }
        }
    }
}

/// A rich display's document: the main window's and each pane's, checked,
/// as a list of [`Event`]s in the order they were shown. What is written
/// to a pane is gathered until the pane is shown (`pane_flush`), or until
/// something else is written, so that it comes as one event.
#[derive(Debug, Default)]
pub struct Document {
    main: Checker,
    panes: std::collections::HashMap<u32, Checker>,
    /// The panes written to since they were last shown, in the order
    /// they were first written to.
    written: Vec<u32>,
    events: Vec<Event>,
}

impl Document {
    /// A document with nothing in it.
    pub fn new() -> Self {
        Self::default()
    }

    /// What the program wrote to the main window.
    pub fn write(&mut self, markup: &str) {
        self.show_panes();
        self.main.write(markup);
        self.take_main();
    }

    /// A line the player typed.
    pub fn input(&mut self, text: &str) {
        self.show_panes();
        self.main.input(text);
        self.take_main();
    }

    /// What the program wrote to a text pane.
    pub fn pane_write(&mut self, id: u32, markup: &str) {
        self.panes.entry(id).or_default().write(markup);
        if !self.written.contains(&id) {
            self.written.push(id);
        }
    }

    /// The panes are to show what they have been given.
    pub fn pane_flush(&mut self) {
        self.show_panes();
    }

    /// A pane was made.
    pub fn pane_created(&mut self, id: u32) {
        self.show_panes();
        self.events.push(Event::PaneCreated(id));
    }

    /// A pane was removed.
    pub fn pane_deleted(&mut self, id: u32) {
        self.show_panes();
        self.panes.remove(&id);
        self.events.push(Event::PaneDeleted(id));
    }

    /// A pane's text was removed: what it is given next starts afresh.
    pub fn pane_cleared(&mut self, id: u32) {
        self.show_panes();
        self.panes.insert(id, Checker::new());
        self.events.push(Event::PaneCleared(id));
    }

    /// The events since the last time.
    pub fn take(&mut self) -> Vec<Event> {
        std::mem::take(&mut self.events)
    }

    /// The events since the last time, as text.
    pub fn take_text(&mut self) -> String {
        self.take().iter().map(Event::markup).collect()
    }

    fn take_main(&mut self) {
        let items = self.main.take();
        self.events.extend(items.into_iter().map(Event::Main));
    }

    fn show_panes(&mut self) {
        for id in std::mem::take(&mut self.written) {
            if let Some(checker) = self.panes.get_mut(&id) {
                let items = checker.take();
                if !items.is_empty() {
                    self.events.push(Event::Pane(id, items));
                }
            }
        }
    }
}

/// Check a whole piece of markup at once, and give its document as text:
/// for tests, and for a host that checks a page at a time.
pub fn check(markup: &str) -> String {
    let mut checker = Checker::new();
    checker.write(markup);
    checker.take().iter().map(Item::markup).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_text_and_entities() {
        assert_eq!(
            check("Fish &amp; chips&mdash;&#33; &bogus; a < b"),
            "Fish &amp; chips\u{2014}! &amp;bogus; a &lt; b"
        );
    }

    #[test]
    fn control_characters() {
        assert_eq!(
            check("one\ntwo\u{b}\u{f}three \u{e}FOUR"),
            "one<br>\ntwo<br>\n<br>\nThree fOUR"
        );
    }

    #[test]
    fn tads_tags_become_the_pages() {
        assert_eq!(
            check("<bq>Quoted.</bq><nobr>12:30</nobr>"),
            "<blockquote>Quoted.</blockquote>\n<span style=\"white-space: nowrap\">12:30</span>"
        );
    }

    #[test]
    fn the_pages_own_elements_are_kept() {
        assert_eq!(
            check(
                "<title>The Skull</title><sound src='drip.wav' layer=ambient random=3><body bgcolor=black>Dark."
            ),
            "<title>The Skull</title>\n<sound src=\"drip.wav\" layer=\"ambient\" random=\"3\"><body bgcolor=\"black\">\nDark."
        );
    }

    #[test]
    fn scripts_and_handlers_are_dropped() {
        assert_eq!(
            check(
                "A<script>alert('x')</script>B<b onclick=\"steal()\">C</b><iframe src=x>D</iframe>E"
            ),
            "AB<b>C</b>E"
        );
    }

    #[test]
    fn only_the_games_resources_and_commands() {
        assert_eq!(
            check(
                "<img src='http://evil/x.png' alt='a cave'><img src='../x.png'><img src='cave.jpg'>"
            ),
            "a cave<img src=\"cave.jpg\">"
        );
        assert_eq!(
            check("<a href='north'>north</a> <a href='https://example.com'>web</a>"),
            "<a href=\"north\">north</a> <a>web</a>"
        );
    }

    #[test]
    fn style_keeps_only_the_profile() {
        assert_eq!(
            check(
                "<span style=\"color: red; position: fixed; background: url(http://x/y.png); background-image: url('stone.png'); display: grid\">x</span>"
            ),
            "<span style=\"color: red; background-image: url('stone.png')\">x</span>"
        );
        assert_eq!(
            check("<span style='behavior: url(x.htc)'>x</span>"),
            "<span>x</span>"
        );
    }

    #[test]
    fn elements_outside_the_profile_go_but_their_text_stays() {
        assert_eq!(check("<marquee>Hello</marquee>"), "Hello");
    }

    #[test]
    fn a_tag_split_between_writes() {
        let mut checker = Checker::new();
        checker.write("<img src='clock/");
        checker.write("9");
        checker.write(".jpg' border=0>after &md");
        checker.write("ash; done");
        let doc: String = checker.take().iter().map(Item::markup).collect();
        assert_eq!(
            doc,
            "<img src=\"clock/9.jpg\" border=\"0\">after \u{2014} done"
        );
    }

    #[test]
    fn elements_are_closed_in_order() {
        assert_eq!(
            check("<b>bold <i>both</b> plain</i>"),
            "<b>bold <i>both</i></b> plain"
        );
        assert_eq!(
            check("<ol><li>One<li>Two</ol>"),
            "<ol><li>One</li>\n<li>Two</li>\n</ol>\n"
        );
        assert_eq!(check("<p>One<p>Two"), "<p>One</p>\n<p>Two");
    }

    #[test]
    fn the_players_input() {
        let mut checker = Checker::new();
        checker.write(">");
        checker.input("look <b>");
        let doc: String = checker.take().iter().map(Item::markup).collect();
        assert_eq!(doc, "><kbd>look &lt;b></kbd>\n");
    }

    #[test]
    fn a_style_attribute_loses_important() {
        assert_eq!(
            check("<span style='color: red !important; font-size: 2em'>x</span>"),
            "<span style=\"color: red; font-size: 2em\">x</span>"
        );
    }

    #[test]
    fn a_stylesheet_is_scoped_and_held_to_the_profile() {
        let css = "@import url(http://x/y.css);\n\
                   /* the page */ body { background: #000; position: absolute; color: #eee }\n\
                   .roomname, h1 { font-variant: small-caps !important }\n\
                   @media (max-width: 40em) { p { margin: 0 } }\n\
                   @media (hover: hover) { p { color: red } }\n\
                   @font-face { font-family: Uncial; src: url('uncial.woff2') format('woff2') }\n\
                   @font-face { font-family: Far; src: url(https://x/y.woff2) }\n\
                   @keyframes spin { from { color: red } }\n\
                   a { color: blue; behavior: url(x.htc) }";
        assert_eq!(
            check_stylesheet(css, "#game"),
            "#game { background: #000; color: #eee }\n\
             #game .roomname, #game h1 { font-variant: small-caps }\n\
             @media (max-width: 40em) {\n#game p { margin: 0 }\n}\n\
             @font-face { font-family: Uncial; src: url('uncial.woff2') format('woff2') }\n\
             #game a { color: blue }\n"
        );
    }

    #[test]
    fn a_transcript_only_text_and_comments() {
        assert_eq!(check("A<log>for the log</log>B<!-- a <b> note -->C"), "ABC");
    }
}
