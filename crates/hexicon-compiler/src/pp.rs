//! The preprocessor: source files in, lines of preprocessed text out.
//!
//! This follows the reference compiler's preprocessor step for step,
//! because what a macro expands to is something a program's author sees
//! and depends on, down to the spacing. Where the reference does
//! something surprising, so does this, and a comment says so.
//!
//! The reference keeps a line as text that ends at a zero byte, with a
//! separate count of how long it is, and the two can disagree: cutting a
//! line short at a `//` comment writes the zero and leaves the count
//! alone. Text spliced on afterward goes after the count, where anything
//! that reads up to the zero never sees it. The line buffer here is kept
//! the same way, so a zero byte inside it ends the text for every reader
//! that the reference has reading up to a zero.

use std::collections::HashMap;

use crate::lex::{
    self, ARGCOUNT_FLAG, EmbedCtx, FOREACH_FLAG, FORMAL_FLAG, FULLY_EXPANDED_FLAG, IFEMPTY_FLAG,
    IFNEMPTY_FLAG, LexCtx, LexNote, MACRO_EXP_END, MACRO_ID_LEN, Tok, Token, at, count_quotes,
    is_space, is_sym,
};

#[path = "tok.rs"]
mod tok;

pub use tok::SrcToken;

/// The most arguments a macro can have.
const MAX_MACRO_ARGS: usize = 128;
/// The deepest `#if` blocks can nest.
const MAX_IF_NESTING: usize = 100;
/// The deepest `#foreach`, `#ifempty`, and `#ifnempty` can nest in a
/// macro's expansion.
const MAX_FOREACH_NESTING: usize = 10;

/// How the preprocessor gets at source files.
pub trait SourceFiles {
    /// The contents of a file, or `None` if it cannot be read.
    fn read(&mut self, name: &str) -> Option<Vec<u8>>;
    /// Whether a file is there to be read.
    fn exists(&mut self, name: &str) -> bool;

    /// Text in a named character set, as UTF-8, or `None` if the set is
    /// not known. The compiler knows ASCII, UTF-8, UTF-16, and Latin-1
    /// itself, and asks here for any other that a source file's
    /// `#charset` names.
    fn decode(&mut self, charset: &str, bytes: &[u8]) -> Option<Vec<u8>> {
        let _ = (charset, bytes);
        None
    }
}

/// How serious a problem is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    /// The program is still compiled.
    Warning,
    /// The program cannot be compiled.
    Error,
    /// Not a problem: text the source asked to be shown while it is
    /// compiled, with `#pragma message`.
    Message,
    /// The compiler has given up: nothing is read or reported after
    /// this.
    Fatal,
    /// The compiler could not do what the source asks, and the program
    /// cannot be compiled, though the source may have nothing wrong
    /// with it that the language forbids.
    Internal,
}

/// How many errors are reported before the compiler gives up.
pub(crate) const MAX_ERRORS: usize = 100;

/// What is said on giving up.
pub(crate) const TOO_MANY_ERRORS: &str =
    "too many errors (try fixing the first couple of errors and recompiling)";

/// The library a line imports, if it is `import name;` and nothing else.
fn import_line(line: &[u8]) -> Option<String> {
    let rest = line.trim_ascii().strip_prefix(b"import")?;
    if !rest.first().is_some_and(|c| c.is_ascii_whitespace()) {
        return None;
    }
    let rest = rest.trim_ascii_start().strip_suffix(b";")?.trim_ascii_end();
    let valid =
        rest.first().is_some_and(|&c| crate::lex::is_syminit(c)) && rest.iter().all(|&c| is_sym(c));
    valid.then(|| String::from_utf8_lossy(rest).into_owned())
}

/// A problem found in the source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    /// The file the problem is in, as the source named it.
    pub file: String,
    /// The line in that file.
    pub line: u32,
    /// Whether it stops the program being compiled.
    pub severity: Severity,
    /// What is wrong.
    pub message: String,
    /// More about it, said below it: in a Hexicon source, what to look
    /// at.
    pub note: Option<String>,
    /// The lines of the source to show with it, first and last; with
    /// none, the line it is on.
    pub lines: Option<(u32, u32)>,
    /// It is in a Hexicon source, and said in Hexicon's words (see
    /// [`crate::wording`]), and shown with the source.
    pub hexicon: bool,
}

impl Default for Diagnostic {
    fn default() -> Self {
        Self {
            file: String::new(),
            line: 0,
            severity: Severity::Error,
            message: String::new(),
            note: None,
            lines: None,
            hexicon: false,
        }
    }
}

/// A line of preprocessed text and where it came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Line {
    /// The text, without a line ending.
    pub text: Vec<u8>,
    /// Which file it came from: an index for [`Preprocessor::file_name`].
    pub file: usize,
    /// The number of the line in that file.
    pub line: u32,
}

/// What a newline inside a string turns into.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NewlineSpacing {
    /// The newline and the spaces after it become one space.
    Collapse,
    /// They vanish.
    Delete,
    /// They stay.
    Preserve,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum IfState {
    /// In an `#if` whose condition was true.
    IfYes,
    /// In an `#if` whose condition was false, with no branch taken yet.
    IfNo,
    /// In an `#elif` after a branch that was taken.
    IfDone,
    /// In the `#else` of an `#if` that was false throughout.
    ElseYes,
    /// In the `#else` of an `#if` that had a true branch.
    ElseNo,
}

#[derive(Debug, Clone, Copy)]
struct IfLevel {
    state: IfState,
    desc: usize,
    line: u32,
}

/// A source file as the program knows it.
#[derive(Debug, Clone)]
struct FileDesc {
    /// The name the file was opened by.
    name: String,
    /// The name without the folder.
    root: String,
}

/// A source file being read.
#[derive(Debug)]
struct Stream {
    desc: usize,
    /// The file's text as UTF-8.
    data: Vec<u8>,
    pos: usize,
    /// The number of the next line to be read.
    next_line: u32,
    in_comment: bool,
    /// How many `#if` levels were open when the file was started; it
    /// cannot close those.
    init_if_level: usize,
    /// The spacing mode to go back to when a file this one includes ends.
    newline_spacing: NewlineSpacing,
    /// The file names a character set that is not known.
    charset_error: bool,
}

impl Stream {
    /// The next line of the file, with a newline on the end unless the
    /// file ends without one. `None` at the end of the file.
    fn read_raw_line(&mut self) -> Option<Vec<u8>> {
        if self.pos >= self.data.len() {
            return None;
        }
        let data = &self.data;
        let start = self.pos;
        let mut i = start;
        while i < data.len() {
            let c = data[i];
            // A line ends at a newline, a return, or a line separator
            // (U+2028); a newline and a return together are one ending.
            if c == b'\n' || c == b'\r' {
                let mut line = data[start..i].to_vec();
                line.push(b'\n');
                i += 1;
                if i < data.len() && (data[i] == b'\n' || data[i] == b'\r') && data[i] != c {
                    i += 1;
                }
                self.pos = i;
                return Some(line);
            }
            if c == 0xe2 && data.get(i + 1) == Some(&0x80) && data.get(i + 2) == Some(&0xa8) {
                let mut line = data[start..i].to_vec();
                line.push(b'\n');
                self.pos = i + 3;
                return Some(line);
            }
            i += 1;
        }
        self.pos = data.len();
        Some(data[start..].to_vec())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MacroKind {
    /// An ordinary `#define`.
    Define,
    /// `__LINE__`.
    Line,
    /// `__FILE__`.
    File,
}

/// A macro.
#[derive(Debug, Clone)]
struct Macro {
    name: Vec<u8>,
    kind: MacroKind,
    /// Defined with a parameter list, even an empty one.
    has_args: bool,
    argc: usize,
    has_varargs: bool,
    /// The expansion, with the parameters replaced by markers.
    expansion: Vec<u8>,
    /// The expansion as the source wrote it, and the parameters' names,
    /// for the program's table of macros.
    orig: Vec<u8>,
    params: Vec<Vec<u8>>,
}

/// A macro as the program's table of macros has it, which a program can
/// ask for when it is run before its image is written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkMacro {
    /// The macro's name.
    pub name: Vec<u8>,
    /// Defined with a parameter list, even an empty one.
    pub has_args: bool,
    /// The last parameter stands for any number of arguments.
    pub has_varargs: bool,
    /// The parameters' names.
    pub params: Vec<Vec<u8>>,
    /// The expansion as the source wrote it.
    pub expansion: Vec<u8>,
}

impl Macro {
    fn min_argc(&self) -> usize {
        if self.has_varargs {
            self.argc.saturating_sub(1)
        } else {
            self.argc
        }
    }
}

/// One level of `#foreach`, `#ifempty`, or `#ifnempty` being expanded.
#[derive(Debug, Clone, Copy)]
struct ExpandLevel {
    typ: Tok,
    is_iterator: bool,
    delim: u8,
    /// Where the part that is repeated starts.
    start: usize,
    /// Which of the variable arguments is being substituted.
    arg: usize,
    part: u32,
}

/// The preprocessor.
pub struct Preprocessor<'a> {
    files: &'a mut dyn SourceFiles,
    descs: Vec<FileDesc>,
    streams: Vec<Stream>,
    include_paths: Vec<String>,
    /// Files not to be included again.
    include_once: Vec<String>,
    all_once: bool,
    /// `#pragma sourceTextGroup` has said to turn the source text groups
    /// on or off since the parser last asked.
    src_group_change: Option<bool>,

    macros: Vec<Macro>,
    /// The names that were ever undefined, or defined anew in a
    /// different way: the reference keeps these out of its object file.
    undefs: std::collections::HashSet<Vec<u8>>,
    defines: HashMap<Vec<u8>, usize>,

    /// The line being worked on. See the note at the top of the file on
    /// zero bytes.
    linebuf: Vec<u8>,
    /// Where reading has got to in the line.
    p: usize,
    /// The macro-expanded line.
    expbuf: Vec<u8>,
    /// A macro whose name stands where an object's name goes in the line
    /// being read (`stop: Fixture ...`, with `stop` a macro), and what it
    /// stands for: the first error on the line says so.
    macro_as_name: Option<(Vec<u8>, Vec<u8>)>,
    /// Text read ahead and put back, to be the next line.
    unsplicebuf: Vec<u8>,
    curtok: Token,

    /// The quote of a string that is open at the end of what has been
    /// read, or zero.
    in_quote: u8,
    in_triple: bool,
    /// Embedded expressions, as comment removal follows them.
    comment_ec: EmbedCtx,
    /// Embedded expressions, as macro expansion follows them.
    macro_ec: EmbedCtx,
    newline_spacing: NewlineSpacing,

    if_stack: Vec<IfLevel>,
    /// How many `#if` levels are nested inside one that is not being
    /// compiled.
    if_false_level: usize,

    last_desc: Option<usize>,
    last_line: u32,
    /// The value of the last integer token read. The reference never
    /// clears a token's integer value, so every token after has it.
    last_int_val: u32,
    appended_desc: Option<usize>,
    appended_line: u32,

    /// Shared by every expansion in progress, as in the reference, where
    /// it is static: a macro expanded inside an argument of another
    /// starts it over.
    expand_stack: Vec<ExpandLevel>,

    /// Only preprocessing: directives that the compiler proper would act
    /// on are passed through.
    pp_only: bool,
    /// Tokens are read from the macro-expanded line, not the line buffer.
    tok_in_expbuf: bool,
    /// Embedded expressions, as the tokenizer follows them.
    main_ec: EmbedCtx,
    /// The token most recently read, and the one before it.
    cur: SrcToken,
    prv: SrcToken,
    /// Tokens put back to be read again, the next one last.
    pushed: Vec<SrcToken>,
    /// Name files by their names without folders, so that output is the
    /// same wherever the files are.
    test_report: bool,
    notes: Vec<LexNote>,
    diagnostics: Vec<Diagnostic>,
    /// How many of the problems found were errors.
    errors: usize,
    /// So many errors were found that nothing more is read.
    gave_up: bool,
    /// The character set of a source file that does not say what it is
    /// in.
    default_charset: String,
    /// Tokens are being read for the expression of a directive, which
    /// changes what some problems are reported as.
    pp_expr: bool,
    /// The libraries a Hexicon source can import, with their headers.
    imports: Vec<(String, Option<String>)>,
    /// The libraries imported already: importing one again is nothing.
    imported: Vec<String>,
    /// While the current token is one a Hexicon form was lowered to,
    /// the form's place: see [`SrcToken::lowered`].
    lowered_pos: Option<(usize, u32)>,
}

impl<'a> Preprocessor<'a> {
    /// A preprocessor that reads files through `files`.
    pub fn new(files: &'a mut dyn SourceFiles) -> Self {
        let mut pp = Self {
            files,
            descs: Vec::new(),
            streams: Vec::new(),
            include_paths: Vec::new(),
            include_once: Vec::new(),
            all_once: false,
            src_group_change: None,
            macros: Vec::new(),
            undefs: std::collections::HashSet::new(),
            defines: HashMap::new(),
            linebuf: Vec::new(),
            p: 0,
            expbuf: Vec::new(),
            macro_as_name: None,
            unsplicebuf: Vec::new(),
            curtok: Token::new(Tok::Eof),
            in_quote: 0,
            in_triple: false,
            comment_ec: EmbedCtx::default(),
            macro_ec: EmbedCtx::default(),
            newline_spacing: NewlineSpacing::Collapse,
            if_stack: Vec::new(),
            if_false_level: 0,
            last_desc: None,
            last_line: 0,
            last_int_val: 0,
            appended_desc: None,
            appended_line: 0,
            expand_stack: Vec::new(),
            pp_only: true,
            tok_in_expbuf: false,
            main_ec: EmbedCtx::default(),
            cur: SrcToken::new(Tok::Invalid),
            prv: SrcToken::new(Tok::Invalid),
            pushed: Vec::new(),
            test_report: false,
            notes: Vec::new(),
            diagnostics: Vec::new(),
            errors: 0,
            gave_up: false,
            default_charset: "utf-8".to_owned(),
            pp_expr: false,
            imports: Vec::new(),
            imported: Vec::new(),
            lowered_pos: None,
        };
        pp.add_special(b"__LINE__", MacroKind::Line);
        pp.add_special(b"__FILE__", MacroKind::File);
        pp
    }

    /// A preprocessor whose only input is the expression of a directive,
    /// its macros already expanded, for the compiler's parser to read as
    /// the reference has it do: the tokens of `text`, then the end of
    /// the directive, which reads as the end of the source.
    pub(crate) fn for_directive_expr(files: &'a mut dyn SourceFiles, text: &[u8]) -> Self {
        let mut pp = Self::new(files);
        pp.pp_only = false;
        pp.pp_expr = true;
        pp.expbuf = text.to_vec();
        pp.expbuf.push(lex::END_PP_LINE);
        pp.tok_in_expbuf = true;
        pp
    }

    /// Name files without their folders in `__FILE__` and in messages.
    pub fn set_test_report_mode(&mut self, on: bool) {
        self.test_report = on;
    }

    /// The libraries a Hexicon source can import (`import hexad;`), each
    /// with the header importing it gives, if it has one.
    pub fn set_imports(&mut self, imports: Vec<(String, Option<String>)>) {
        self.imports = imports;
    }

    /// Set the character set of source files that do not say what they
    /// are in, with a byte order mark or `#charset`. It is UTF-8 unless
    /// this is called.
    pub fn set_default_charset(&mut self, name: &str) {
        name.clone_into(&mut self.default_charset);
    }

    /// Add a folder to look for included files in.
    pub fn add_include_path(&mut self, path: &str) {
        self.include_paths.push(path.to_owned());
    }

    /// Define a macro without parameters.
    pub fn define(&mut self, name: &str, expansion: &str) {
        let id = self.macros.len();
        self.macros.push(Macro {
            name: name.as_bytes().to_vec(),
            kind: MacroKind::Define,
            has_args: false,
            argc: 0,
            has_varargs: false,
            expansion: expansion.as_bytes().to_vec(),
            orig: expansion.as_bytes().to_vec(),
            params: Vec::new(),
        });
        self.defines.insert(name.as_bytes().to_vec(), id);
    }

    /// Define a macro as the program's table of macros has it, with its
    /// parameters, for code compiled while the program runs.
    pub fn define_macro(&mut self, mac: &LinkMacro) {
        let expansion = self.parse_expansion(&mac.expansion, &mac.params, mac.has_varargs);
        let id = self.macros.len();
        self.macros.push(Macro {
            name: mac.name.clone(),
            kind: MacroKind::Define,
            has_args: mac.has_args,
            argc: mac.params.len(),
            has_varargs: mac.has_varargs,
            expansion,
            orig: mac.expansion.clone(),
            params: mac.params.clone(),
        });
        self.defines.insert(mac.name.clone(), id);
    }

    /// Remove a macro's definition.
    pub fn undefine(&mut self, name: &str) {
        self.undefine_bytes(name.as_bytes());
    }

    fn add_special(&mut self, name: &[u8], kind: MacroKind) {
        let id = self.macros.len();
        self.macros.push(Macro {
            name: name.to_vec(),
            kind,
            has_args: false,
            argc: 0,
            has_varargs: false,
            expansion: Vec::new(),
            orig: Vec::new(),
            params: Vec::new(),
        });
        self.defines.insert(name.to_vec(), id);
    }

    /// Start reading a source file. Returns false if it cannot be read.
    pub fn set_source(&mut self, name: &str) -> bool {
        let Some(data) = self.files.read(name) else {
            return false;
        };
        let desc = self.file_desc(name);
        let (data, charset_error) = decode_source(&data, self.files, &self.default_charset);
        self.streams.push(Stream {
            desc,
            data,
            pos: 0,
            next_line: 1,
            in_comment: false,
            init_if_level: self.if_stack.len(),
            newline_spacing: self.newline_spacing,
            charset_error,
        });
        true
    }

    /// The name of a file that a [`Line`] came from. In test report mode
    /// it is the name without the folder.
    pub fn file_name(&self, file: usize) -> &str {
        match self.descs.get(file) {
            Some(desc) if self.test_report => &desc.root,
            Some(desc) => &desc.name,
            None => "",
        }
    }

    /// How many errors have been found so far.
    pub(crate) fn error_count(&self) -> usize {
        self.errors
    }

    /// Whether so many errors have been found that nothing more is
    /// read: from then on the source is at its end.
    pub fn gave_up(&self) -> bool {
        self.gave_up
    }

    /// Record a problem. One error too many is the last thing recorded.
    pub(crate) fn push_diagnostic(&mut self, diagnostic: Diagnostic) {
        if self.gave_up {
            return;
        }
        let is_error = diagnostic.severity == Severity::Error;
        self.diagnostics.push(diagnostic);
        if is_error {
            self.errors += 1;
            if self.errors > MAX_ERRORS {
                let file = self
                    .last_desc
                    .map(|desc| self.file_name(desc).to_owned())
                    .unwrap_or_default();
                self.diagnostics.push(Diagnostic {
                    file,
                    line: self.last_line,
                    severity: Severity::Fatal,
                    message: TOO_MANY_ERRORS.to_owned(),
                    ..Default::default()
                });
                self.gave_up = true;
            }
        }
    }

    /// Take the problems found so far.
    pub fn take_diagnostics(&mut self) -> Vec<Diagnostic> {
        std::mem::take(&mut self.diagnostics)
    }

    /// The next line of preprocessed text, or `None` at the end of the
    /// source. There is a line of output, possibly empty, for each line
    /// of the source.
    pub fn next_line(&mut self) -> Option<Line> {
        if self.gave_up || self.read_line_pp() || self.gave_up {
            return None;
        }
        // The text ends at the first zero byte: see the note at the top.
        let end = self
            .linebuf
            .iter()
            .position(|&c| c == 0)
            .unwrap_or(self.linebuf.len());
        Some(Line {
            text: self.linebuf[..end].to_vec(),
            file: self.last_desc.unwrap_or(0),
            line: self.last_line,
        })
    }

    // ---- files ----

    /// The names of the files read, by their numbers.
    pub fn file_names(&self) -> Vec<String> {
        self.descs.iter().map(|desc| desc.name.clone()).collect()
    }

    fn file_desc(&mut self, name: &str) -> usize {
        if let Some(index) = self.descs.iter().position(|desc| desc.name == name) {
            return index;
        }
        self.descs.push(FileDesc {
            name: name.to_owned(),
            root: root_name(name).to_owned(),
        });
        self.descs.len() - 1
    }

    /// A file's name as a string in single quotes, for `__FILE__`.
    fn quoted_file_name(&self, desc: usize) -> Vec<u8> {
        let name = self.file_name(desc);
        let mut out = vec![b'\''];
        for &c in name.as_bytes() {
            if c == b'\\' || c == b'\'' {
                out.push(b'\\');
            }
            out.push(c);
        }
        out.push(b'\'');
        out
    }

    // ---- problems ----

    fn report(&mut self, severity: Severity, message: String) {
        let file = self
            .last_desc
            .map(|desc| self.file_name(desc).to_owned())
            .unwrap_or_default();
        // An object named with a macro's name: the first error says why.
        let note = if severity == Severity::Error {
            self.macro_as_name.take().map(|(name, orig)| {
                format!(
                    "\"{}\" is a macro, which stands for \"{}\", so it cannot be an object's \
                     name: give the object a name of its own",
                    show_name(&name),
                    String::from_utf8_lossy(&orig).trim()
                )
            })
        } else {
            None
        };
        self.push_diagnostic(Diagnostic {
            file,
            line: self.last_line,
            severity,
            message,
            note,
            ..Default::default()
        });
    }

    fn error(&mut self, message: impl Into<String>) {
        self.report(Severity::Error, message.into());
    }

    fn warning(&mut self, message: impl Into<String>) {
        self.report(Severity::Warning, message.into());
    }

    /// Report what the scanner noticed.
    fn flush_notes(&mut self) {
        for note in std::mem::take(&mut self.notes) {
            let text = show_name;
            match note {
                LexNote::DecimalInOctal(number) => self.error(format!(
                    "decimal digit found in octal constant \"{}\"",
                    text(&number)
                )),
                LexNote::NonAsciiSymbol(symbol) => {
                    let name = text(&symbol);
                    let odd = String::from_utf8_lossy(&symbol)
                        .chars()
                        .find(|c| !c.is_ascii())
                        .map_or(0, u32::from);
                    self.error(format!(
                        "the name \"{name}\" contains non-ASCII character (U+{odd:04x})"
                    ));
                }
                LexNote::SymbolTruncated(symbol) => {
                    let kept = lex::SYM_MAX_LEN - 1;
                    self.warning(format!(
                        "the name \"{}\" truncated to \"{}\"",
                        text(&symbol),
                        text(&symbol[..kept.min(symbol.len())])
                    ));
                }
                LexNote::EmbeddingTooDeep => {
                    self.error("embedded expressions in strings are nested too deeply");
                }
                LexNote::UntermString => {
                    self.error("unterminated string in preprocessor constant expression");
                }
            }
        }
    }

    // ---- reading tokens ----

    /// The next token of the current line, for reading a directive.
    /// Embedded expressions are not followed.
    fn next_on_line(&mut self) -> Tok {
        let mut cx = LexCtx {
            ec: None,
            expanding: false,
            in_pp_expr: false,
            notes: &mut self.notes,
        };
        self.curtok = lex::next_on_line(&self.linebuf, &mut self.p, &mut cx);
        self.flush_notes();
        self.curtok.typ
    }

    fn curtok_text(&self) -> Vec<u8> {
        self.curtok.text(&self.linebuf).to_vec()
    }

    /// The next token of `src`, for macro expansion. With a `limit`, the
    /// text is only that long, though a token that starts within it can
    /// run past, as it can in the reference.
    fn next_tok(
        &mut self,
        src: &[u8],
        limit: Option<usize>,
        pos: &mut usize,
        expanding: bool,
    ) -> Token {
        let old_ec = self.macro_ec;
        let mut cx = LexCtx {
            ec: Some(&mut self.macro_ec),
            expanding,
            in_pp_expr: false,
            notes: &mut self.notes,
        };
        let mut tok = lex::next_on_line(src, pos, &mut cx);
        if let Some(end) = limit.or(Some(src.len())) {
            if tok.pos >= end {
                tok = Token::new(Tok::Eof);
                tok.pos = end;
                self.macro_ec = old_ec;
            }
        }
        self.flush_notes();
        tok
    }

    /// The next token of a macro's stored expansion, which ends where its
    /// text does.
    fn next_exp_tok(&mut self, exp: &[u8], pos: &mut usize) -> Token {
        let mut cx = LexCtx {
            ec: Some(&mut self.macro_ec),
            expanding: true,
            in_pp_expr: false,
            notes: &mut self.notes,
        };
        let tok = lex::next_on_line(exp, pos, &mut cx);
        self.flush_notes();
        tok
    }

    // ---- lines ----

    fn clear_linebuf(&mut self) {
        self.linebuf.clear();
        self.p = 0;
        self.tok_in_expbuf = false;
    }

    fn in_false_if(&self) -> bool {
        match self.if_stack.last() {
            None => false,
            Some(level) => {
                self.if_false_level != 0
                    || matches!(
                        level.state,
                        IfState::IfNo | IfState::IfDone | IfState::ElseNo
                    )
            }
        }
    }

    fn push_if(&mut self, state: IfState) {
        if self.if_stack.len() >= MAX_IF_NESTING {
            self.error("#if nesting too deep");
            return;
        }
        if self.in_false_if() {
            self.if_false_level += 1;
        }
        self.if_stack.push(IfLevel {
            state,
            desc: self.last_desc.unwrap_or(0),
            line: self.last_line,
        });
    }

    fn pop_if(&mut self) {
        self.if_false_level = self.if_false_level.saturating_sub(1);
        self.if_stack.pop();
    }

    /// Read a line of source into the line buffer, or onto the end of
    /// what is there. Lines that end in a backslash are joined, and
    /// comments are taken out. Returns where the new text starts, or
    /// `None` at the end of all the source.
    fn read_line(&mut self, append: bool) -> Option<usize> {
        self.streams.last()?;
        if !append {
            self.clear_linebuf();
            let stream = self.streams.last()?;
            self.last_desc = Some(stream.desc);
            self.last_line = stream.next_line;
        }
        let start_len = self.linebuf.len();

        // Text that was read ahead and put back is the next line. It has
        // been through all of this already.
        if !self.unsplicebuf.is_empty() {
            let text = std::mem::take(&mut self.unsplicebuf);
            self.linebuf.extend_from_slice(&text);
            self.last_desc = self.appended_desc;
            self.last_line = self.appended_line;
            return Some(start_len);
        }

        if append {
            let stream = self.streams.last()?;
            self.appended_desc = Some(stream.desc);
            self.appended_line = stream.next_line;
        }

        loop {
            let stream = self.streams.last_mut()?;
            let Some(text) = stream.read_raw_line() else {
                // The file ended. If it ended in the middle of a line,
                // that is the line.
                if self.linebuf.len() > start_len {
                    break;
                }
                // An #if that the file opened and did not close.
                let init_level = stream.init_if_level;
                while self.if_stack.len() > init_level {
                    let level = self.if_stack[self.if_stack.len() - 1];
                    let name = self.file_name(level.desc).to_owned();
                    self.error(format!(
                        "#if without #endif (line {}, file {})",
                        level.line, name
                    ));
                    self.pop_if();
                }
                self.streams.pop();
                let stream = self.streams.last()?;
                if !append {
                    self.last_desc = Some(stream.desc);
                    self.last_line = stream.next_line;
                }
                self.newline_spacing = stream.newline_spacing;
                continue;
            };
            self.linebuf.extend_from_slice(&text);
            let len = self.linebuf.len();
            if len != 0 && self.linebuf[len - 1] != b'\n' {
                // The last line of a file, with no newline: the next read
                // finds the end of the file.
            } else if len > 1 && self.linebuf[len - 2] == b'\\' {
                self.linebuf.truncate(len - 2);
                stream.next_line += 1;
            } else {
                if len != 0 {
                    self.linebuf.truncate(len - 1);
                }
                stream.next_line += 1;
                break;
            }
        }

        self.process_comments(start_len);
        Some(start_len)
    }

    /// Break the line at `at`: what comes after it is put back to be read
    /// as the next line.
    fn unsplice_line(&mut self, at: usize) {
        let at = at.min(self.linebuf.len());
        let mut rest = self.linebuf[at..].to_vec();
        rest.extend_from_slice(&self.unsplicebuf);
        self.unsplicebuf = rest;
        self.linebuf.truncate(at);
    }

    /// Take the comments out of the line from `start` on, leaving a space
    /// for each, and keep track of whether the text ends inside a string.
    fn process_comments(&mut self, start: usize) {
        let mut trailing_sp_after_bs = false;
        let mut src = start;
        let mut dst = start;
        let mut in_comment = self.streams.last().is_some_and(|s| s.in_comment);
        let mut nested_warning = false;
        let mut cut_short = false;

        while at(&self.linebuf, src) != 0 {
            let mut cur = self.linebuf[src];
            let next = at(&self.linebuf, src + 1);
            if in_comment {
                if cur == b'*' && next == b'/' {
                    src += 1;
                    in_comment = false;
                } else if cur == b'/' && next == b'*' {
                    nested_warning = true;
                }
                src += 1;
                continue;
            } else if self.in_quote != 0 {
                if cur == b'\\' {
                    // An escape: the backslash and what follows it are
                    // copied as they are.
                    src += 1;
                    self.linebuf[dst] = cur;
                    dst += 1;
                    cur = at(&self.linebuf, src);
                    if cur == 0 {
                        break;
                    }
                    if self.in_triple && cur == self.in_quote {
                        let mut count = count_quotes(&self.linebuf, src, cur);
                        while count > 1 {
                            self.linebuf[dst] = cur;
                            dst += 1;
                            src += 1;
                            count -= 1;
                        }
                    }
                } else if cur == self.in_quote {
                    if self.in_triple {
                        let count = count_quotes(&self.linebuf, src, cur);
                        if count >= 3 {
                            for _ in 1..count {
                                self.linebuf[dst] = cur;
                                dst += 1;
                                src += 1;
                            }
                            self.in_quote = 0;
                        }
                    } else {
                        self.in_quote = 0;
                    }
                } else if cur == b'<' && next == b'<' {
                    // An embedded expression: out of the string for now.
                    src += 1;
                    self.comment_ec.start_expr(self.in_quote, self.in_triple);
                    self.in_quote = 0;
                    self.linebuf[dst] = b'<';
                    dst += 1;
                }
            } else {
                if cur == b'\\' {
                    trailing_sp_after_bs = true;
                } else if !is_space(cur) {
                    trailing_sp_after_bs = false;
                }
                if cur == b'/' {
                    match next {
                        b'*' => {
                            in_comment = true;
                            cur = b' ';
                        }
                        b'/' => {
                            // The rest of the line is a comment. The text
                            // ends here, but the buffer's length is left
                            // as it was: see the note at the top.
                            self.linebuf[dst] = 0;
                            cut_short = true;
                            break;
                        }
                        _ => {}
                    }
                } else if cur == b'"' || cur == b'\'' {
                    self.in_quote = cur;
                    self.in_triple = count_quotes(&self.linebuf, src, cur) >= 3;
                    if self.in_triple {
                        src += 2;
                        self.linebuf[dst] = cur;
                        self.linebuf[dst + 1] = cur;
                        dst += 2;
                    }
                } else if cur < 0x09 {
                    // These are the preprocessor's own markers, and must
                    // not come in from outside.
                    cur = b' ';
                } else if self.comment_ec.in_expr() && (cur == b'(' || cur == b')') {
                    self.comment_ec.add_parens(if cur == b'(' { 1 } else { -1 });
                } else if self.comment_ec.in_expr()
                    && self.comment_ec.parens() == 0
                    && cur == b'>'
                    && next == b'>'
                {
                    // The end of an embedded expression: back in the
                    // string.
                    self.in_quote = self.comment_ec.quote();
                    self.in_triple = self.comment_ec.triple();
                    self.comment_ec.end_expr();
                    src += 1;
                    self.linebuf[dst] = b'>';
                    dst += 1;
                }
            }
            self.linebuf[dst] = cur;
            dst += 1;
            src += 1;
        }

        if let Some(stream) = self.streams.last_mut() {
            stream.in_comment = in_comment;
        }
        if nested_warning {
            self.warning("\"/*\" found within \"/* ... */\" - nested comments are not allowed");
        }
        if cut_short {
            return;
        }
        self.linebuf.truncate(dst);
        if trailing_sp_after_bs {
            self.warning("line ends with whitespace following backslash");
        }
    }

    /// The line ends inside a string: join lines onto it until the string
    /// ends, and put back whatever comes after that.
    fn splice_string(&mut self) {
        let mut unterm: u8 = 0;
        let in_quote = self.in_quote;
        let in_triple = self.in_triple;
        let ends_in_newline =
            |buf: &[u8], from: usize| buf.len() - from >= 2 && buf.ends_with(b"\\n");
        let mut explicit_nl = ends_in_newline(&self.linebuf, 0);
        let mut p: Option<usize> = None;

        'splice: loop {
            match self.newline_spacing {
                NewlineSpacing::Collapse => {
                    if !explicit_nl {
                        self.linebuf.push(b' ');
                    }
                }
                NewlineSpacing::Delete => {}
                NewlineSpacing::Preserve => self.linebuf.extend_from_slice(b"\\n"),
            }

            let Some(new_line) = self.read_line(true) else {
                break;
            };
            let mut q = new_line;

            // The spaces that start the new line go, unless they are to
            // be preserved or the line before ended in an explicit
            // newline.
            if self.newline_spacing != NewlineSpacing::Preserve && !explicit_nl {
                while is_space(at(&self.linebuf, q)) {
                    q += 1;
                }
            }
            explicit_nl = ends_in_newline(&self.linebuf, new_line);
            if q > new_line {
                self.linebuf.drain(new_line..q.min(self.linebuf.len()));
            }

            // A line with only a brace or a semicolon on it suggests that
            // the string was never closed.
            q = new_line;
            if matches!(at(&self.linebuf, q), b'}' | b';') {
                q += 1;
                while is_space(at(&self.linebuf, q)) {
                    q += 1;
                }
                if at(&self.linebuf, q) == 0 {
                    let line = self.appended_line;
                    self.error(format!(
                        "string appears unterminated (';' or '}}' appears alone on line {line})"
                    ));
                    unterm = in_quote;
                    self.in_quote = 0;
                    self.process_comments(new_line);
                    p = Some(new_line);
                    break 'splice;
                }
            }

            // Look for the end of the string.
            q = new_line;
            loop {
                let cur = at(&self.linebuf, q);
                if cur == b'\\' {
                    q += 1;
                    if in_triple && at(&self.linebuf, q) == in_quote {
                        let count = count_quotes(&self.linebuf, q, in_quote);
                        q += count - 1;
                    } else if at(&self.linebuf, q) == b'<' {
                        while at(&self.linebuf, q) == b'<' {
                            q += 1;
                        }
                    }
                } else if cur == in_quote {
                    if in_triple {
                        let count = count_quotes(&self.linebuf, q, cur);
                        if count >= 3 {
                            p = Some(q + count);
                            break 'splice;
                        }
                    } else {
                        p = Some(q + 1);
                        break 'splice;
                    }
                } else if cur == b'<' && at(&self.linebuf, q + 1) == b'<' {
                    q += 2;
                    if at(&self.linebuf, q) == b'%' {
                        lex::scan_sprintf_spec(&self.linebuf, &mut q);
                    }
                    p = Some(q);
                    break 'splice;
                } else if cur == 0 {
                    p = Some(q);
                    break;
                }
                q += 1;
            }
        }

        if let Some(p) = p {
            self.unsplice_line(p);
        }
        if unterm != 0 {
            self.linebuf.push(unterm);
        }
    }

    /// Read the next line and preprocess it. Returns true at the end of
    /// the source.
    fn read_line_pp(&mut self) -> bool {
        if self.read_line(false).is_none() {
            return true;
        }
        let started_in_string = self.in_quote != 0;
        self.p = 0;
        while is_space(at(&self.linebuf, self.p)) {
            self.p += 1;
        }

        if !started_in_string && at(&self.linebuf, self.p) == b'#' {
            self.p += 1;
            // A comment that is open at the end of the line makes the
            // next line part of the directive.
            while self.streams.last().is_some_and(|s| s.in_comment) {
                if self.read_line(true).is_none() {
                    break;
                }
            }
            self.next_on_line();
            if self.curtok.typ == Tok::Eof {
                self.clear_linebuf();
                return false;
            }
            let keyword = self.curtok_text();
            if self.curtok.typ != Tok::Sym {
                self.invalid_directive(&keyword);
                self.clear_linebuf();
                return false;
            }
            let in_false = self.in_false_if();
            // Only the directives that open and close #if blocks matter
            // in a block that is not being compiled.
            let (known, always): (bool, bool) = match keyword.as_slice() {
                b"charset" | b"pragma" | b"include" | b"define" | b"error" | b"undef" | b"line" => {
                    (true, false)
                }
                b"if" | b"ifdef" | b"ifndef" | b"else" | b"elif" | b"endif" => (true, true),
                _ => (false, false),
            };
            if !known {
                self.invalid_directive(&keyword);
            } else if in_false && !always {
                self.clear_linebuf();
            } else {
                match keyword.as_slice() {
                    b"charset" => self.pp_charset(),
                    b"pragma" => self.pp_pragma(),
                    b"include" => self.pp_include(),
                    b"define" => self.pp_define(),
                    b"if" => self.pp_if(),
                    b"ifdef" => self.pp_ifdef_or_ifndef(true),
                    b"ifndef" => self.pp_ifdef_or_ifndef(false),
                    b"else" => self.pp_else(),
                    b"elif" => self.pp_elif(),
                    b"endif" => self.pp_endif(),
                    b"error" => self.pp_error(),
                    b"undef" => self.pp_undef(),
                    _ => self.pp_line(),
                }
            }
            // A directive is one line, so a string cannot carry on from
            // it.
            self.in_quote = 0;
        } else if let Some(name) = (!started_in_string && !self.in_false_if())
            .then(|| import_line(&self.linebuf[self.p.min(self.linebuf.len())..]))
            .flatten()
        {
            self.pp_import(&name);
        } else {
            if self.in_false_if() {
                self.clear_linebuf();
                self.expbuf.clear();
            } else {
                if self.in_quote != 0 {
                    self.splice_string();
                }
                self.p = 0;
                self.expand_macros_curline(true, false, false);
            }
            if self.pp_only {
                self.linebuf = self.expbuf.clone();
            } else {
                // The compiler proper reads its tokens from the expanded
                // line.
                self.tok_in_expbuf = true;
                self.p = 0;
            }
        }
        false
    }

    fn invalid_directive(&mut self, _keyword: &[u8]) {
        self.error("invalid preprocessor directive");
    }

    // ---- macros ----

    fn find_define(&self, name: &[u8]) -> Option<usize> {
        self.defines.get(name).copied()
    }

    fn undefine_bytes(&mut self, name: &[u8]) {
        if let Some(&id) = self.defines.get(name) {
            if self.macros[id].kind == MacroKind::Define {
                self.defines.remove(name);
                self.undefs.insert(name.to_vec());
            }
        }
    }

    /// The macros as the source left them, as the reference writes them
    /// to its object file: those defined now, but for `__LINE__` and
    /// `__FILE__` and any name that was ever undefined or defined anew,
    /// in the order of the reference's table of them (chain by chain,
    /// the sum of the name's bytes in 512, and in a chain the newest
    /// first).
    pub fn link_macros(&self) -> Vec<LinkMacro> {
        let mut ids: Vec<usize> = self
            .defines
            .values()
            .copied()
            .filter(|&id| {
                let mac = &self.macros[id];
                mac.kind == MacroKind::Define && !self.undefs.contains(&mac.name)
            })
            .collect();
        ids.sort_by_key(|&id| (macro_chain(&self.macros[id].name), std::cmp::Reverse(id)));
        ids.into_iter()
            .map(|id| {
                let mac = &self.macros[id];
                LinkMacro {
                    name: mac.name.clone(),
                    has_args: mac.has_args,
                    has_varargs: mac.has_varargs,
                    params: mac.params.clone(),
                    expansion: mac.orig.clone(),
                }
            })
            .collect()
    }

    /// What a macro without arguments expands to.
    fn expansion(&self, id: usize) -> Vec<u8> {
        match self.macros[id].kind {
            MacroKind::Define => self.macros[id].expansion.clone(),
            MacroKind::Line => self.last_line.to_string().into_bytes(),
            MacroKind::File => match self.last_desc {
                Some(desc) => self.quoted_file_name(desc),
                None => Vec::new(),
            },
        }
    }

    /// Expand the macros in the current line, from where reading has got
    /// to, into the expansion buffer.
    fn expand_macros_curline(
        &mut self,
        read_more: bool,
        allow_defined: bool,
        append: bool,
    ) -> bool {
        if !append {
            self.macro_as_name = None;
        }
        let mut src = std::mem::take(&mut self.linebuf);
        let mut expbuf = std::mem::take(&mut self.expbuf);
        let mut pos = self.p;
        let err = self.expand_macros(
            &mut src,
            None,
            &mut pos,
            &mut expbuf,
            read_more,
            allow_defined,
            append,
        );
        self.linebuf = src;
        self.expbuf = expbuf;
        self.p = pos;
        if err {
            return true;
        }
        if self.pp_only {
            remove_expansion_flags(&mut self.expbuf);
        }
        false
    }

    /// Run something that works on the line buffer while the line is
    /// being held as `src`.
    fn with_linebuf<R>(&mut self, src: &mut Vec<u8>, run: impl FnOnce(&mut Self) -> R) -> R {
        std::mem::swap(src, &mut self.linebuf);
        let result = run(self);
        std::mem::swap(src, &mut self.linebuf);
        result
    }

    /// Expand the macros in `src` from `pos`, onto `expbuf`. With
    /// `read_more`, `src` is the line buffer, expansions are put back
    /// into it to be scanned again, and more lines are read to complete a
    /// macro's arguments. Returns true on an error.
    #[allow(clippy::too_many_arguments)]
    fn expand_macros(
        &mut self,
        src: &mut Vec<u8>,
        limit: Option<usize>,
        pos: &mut usize,
        expbuf: &mut Vec<u8>,
        read_more: bool,
        allow_defined: bool,
        append: bool,
    ) -> bool {
        let mut subexp = Vec::new();
        if !append {
            expbuf.clear();
        }
        let mut startofs = *pos;
        let mut tok = self.next_tok(src, limit, pos, true);
        while tok.typ != Tok::Eof {
            if tok.typ == Tok::Sym && !tok.fully_expanded {
                let name = tok.text(src).to_vec();
                let end = limit.unwrap_or(src.len());
                let entry = self.find_define(&name);
                let expandable =
                    entry.is_some_and(|id| !scan_for_prior_expansion(src, *pos, end, id));
                if expandable || (allow_defined && name == b"defined") {
                    let macro_ofs = tok.pos;
                    if read_more
                        && let Some(id) = entry
                        && self.macros[id].kind == MacroKind::Define
                        && stands_as_name(src, macro_ofs, *pos)
                    {
                        self.macro_as_name = Some((name.clone(), self.macros[id].orig.clone()));
                    }
                    let expanded = match entry {
                        Some(id) => {
                            let (err, did) = self.expand_macro(
                                &mut subexp,
                                src,
                                limit,
                                pos,
                                macro_ofs,
                                id,
                                read_more,
                                allow_defined,
                            );
                            if err {
                                return true;
                            }
                            did
                        }
                        None => {
                            if self.expand_defined(&mut subexp, src, limit, pos) {
                                return true;
                            }
                            true
                        }
                    };
                    if !expanded {
                        tok = self.next_tok(src, limit, pos, true);
                        continue;
                    }
                    expbuf.extend_from_slice(slice(src, startofs, macro_ofs));
                    startofs = *pos;
                    let end = limit.unwrap_or(src.len());
                    if read_more {
                        // Put the expansion in place of the macro and
                        // carry on from its start, so that it is scanned
                        // along with what follows it.
                        let from = macro_ofs.min(src.len());
                        let to = startofs.clamp(from, src.len());
                        src.splice(from..to, subexp.iter().copied());
                        *pos = macro_ofs;
                        startofs = macro_ofs;
                        tok = self.next_tok(src, limit, pos, true);
                        continue;
                    }
                    subexp.extend_from_slice(slice(src, startofs, end));
                    let mut sub_pos = 0;
                    return self.expand_macros(
                        &mut subexp,
                        None,
                        &mut sub_pos,
                        expbuf,
                        false,
                        allow_defined,
                        true,
                    );
                }
            }
            tok = self.next_tok(src, limit, pos, true);
        }
        expbuf.extend_from_slice(slice(src, startofs, tok.pos));
        false
    }

    /// Expand one macro, whose name has just been read. Returns whether
    /// there was an error, and whether the macro was expanded: one that
    /// takes arguments is left alone if none follow.
    #[allow(clippy::too_many_arguments)]
    fn expand_macro(
        &mut self,
        expbuf: &mut Vec<u8>,
        src: &mut Vec<u8>,
        limit: Option<usize>,
        pos: &mut usize,
        macro_ofs: usize,
        id: usize,
        read_more: bool,
        allow_defined: bool,
    ) -> (bool, bool) {
        let startofs = *pos;
        expbuf.clear();
        let mut argofs = vec![0usize; MAX_MACRO_ARGS + 1];
        let mut arglen = vec![0usize; MAX_MACRO_ARGS + 1];

        if self.macros[id].has_args {
            let (err, found) =
                self.parse_macro_actuals(src, limit, pos, id, &mut argofs, &mut arglen, read_more);
            if err {
                return (true, false);
            }
            if !found {
                *expbuf = slice(src, macro_ofs, startofs).to_vec();
                *pos = startofs;
                return (false, false);
            }
        }

        if self.macros[id].argc != 0 {
            match self.substitute_macro_actuals(id, src, &argofs, &arglen, allow_defined) {
                Some(text) => *expbuf = text,
                None => return (true, false),
            }
        } else {
            *expbuf = self.expansion(id);
        }
        expbuf.extend_from_slice(&lex::macro_end_marker(id));
        (false, true)
    }

    /// Read the arguments of a macro call: where each one is in `src`.
    /// Returns whether there was an error, and whether there were any
    /// arguments at all.
    #[allow(clippy::too_many_arguments)]
    fn parse_macro_actuals(
        &mut self,
        src: &mut Vec<u8>,
        limit: Option<usize>,
        pos: &mut usize,
        id: usize,
        argofs: &mut [usize],
        arglen: &mut [usize],
        read_more: bool,
    ) -> (bool, bool) {
        let mut spliced = false;
        let mut argc = 0;
        let (macro_argc, has_varargs, min_argc, name) = {
            let m = &self.macros[id];
            (m.argc, m.has_varargs, m.min_argc(), show_name(&m.name))
        };

        let mut tok = self.next_tok(src, limit, pos, true);
        if tok.typ == Tok::Eof && read_more {
            tok = self.actual_splice_next_line(src, pos);
            spliced = true;
        }
        if tok.typ != Tok::LPar {
            if spliced {
                self.with_linebuf(src, |pp| pp.unsplice_line(tok.pos));
            }
            return (false, false);
        }

        argofs[argc] = tok.end();
        tok = self.next_tok(src, limit, pos, true);
        while tok.typ != Tok::RPar {
            if (argc >= macro_argc && !has_varargs) || argc >= MAX_MACRO_ARGS {
                self.error(format!("too many arguments for macro \"{name}\""));
                while tok.typ != Tok::RPar && tok.typ != Tok::Eof {
                    tok = self.next_tok(src, limit, pos, true);
                }
                break;
            }

            // An argument runs to a comma or the closing bracket, outside
            // any brackets of its own.
            let (mut parens, mut brackets, mut braces) = (0i32, 0i32, 0i32);
            loop {
                if tok.typ == Tok::Comma && parens == 0 && braces == 0 && brackets == 0 {
                    break;
                }
                if tok.typ == Tok::RPar && parens == 0 {
                    break;
                }
                match tok.typ {
                    Tok::LPar => parens += 1,
                    Tok::RPar => parens -= 1,
                    Tok::LBrace => braces += 1,
                    Tok::RBrace => braces -= 1,
                    Tok::LBrack => brackets += 1,
                    Tok::RBrack => brackets -= 1,
                    _ => {}
                }
                tok = self.next_tok(src, limit, pos, true);
                if tok.typ == Tok::Eof && read_more {
                    tok = self.actual_splice_next_line(src, pos);
                    spliced = true;
                }
                if tok.typ == Tok::Eof {
                    break;
                }
            }
            if tok.typ == Tok::Eof {
                break;
            }

            arglen[argc] = trim_end(src, argofs[argc], tok.pos) - argofs[argc];
            argc += 1;

            if tok.typ == Tok::Comma {
                argofs[argc] = tok.end();
                tok = self.next_tok(src, limit, pos, true);
            } else if tok.typ == Tok::RPar {
                break;
            }
        }

        if tok.typ != Tok::RPar {
            self.error(format!("missing ')' in argument list for macro \"{name}\""));
            return (true, false);
        }

        // Take the spaces off the start and end of each argument.
        for i in 0..argc {
            let start = argofs[i];
            let end = start + arglen[i];
            let mut p = start;
            while p < end && is_space(at(src, p)) {
                p += 1;
            }
            let removed = p - start;
            argofs[i] += removed;
            arglen[i] -= removed;
            let start = start + removed;
            arglen[i] -= end - trim_end(src, start, end);
        }

        if spliced {
            self.with_linebuf(src, |pp| pp.unsplice_line(tok.end()));
        }

        if argc < min_argc {
            while argc < macro_argc {
                argofs[argc] = 0;
                arglen[argc] = 0;
                argc += 1;
            }
            self.warning(format!("not enough arguments for macro \"{name}\""));
        }
        if has_varargs && argc < MAX_MACRO_ARGS {
            argofs[argc] = 0;
            arglen[argc] = 0;
        }
        (false, true)
    }

    /// A macro's arguments run past the end of the line: read the next
    /// line onto the end of this one and get the first token from it.
    fn actual_splice_next_line(&mut self, src: &mut Vec<u8>, pos: &mut usize) -> Token {
        src.push(b' ');
        loop {
            let new_line = self.with_linebuf(src, |pp| {
                let new_line = pp.read_line(true);
                if pp.in_quote != 0 {
                    pp.splice_string();
                }
                new_line
            });
            let Some(new_line) = new_line else {
                let mut tok = Token::new(Tok::Eof);
                tok.pos = src.len();
                return tok;
            };
            *pos = new_line;
            let tok = self.next_tok(src, None, pos, true);
            if tok.typ != Tok::Eof {
                return tok;
            }
        }
    }

    /// Build a macro's expansion with its arguments in place. `None` on
    /// an error.
    fn substitute_macro_actuals(
        &mut self,
        id: usize,
        src: &mut Vec<u8>,
        argofs: &[usize],
        arglen: &[usize],
        allow_defined: bool,
    ) -> Option<Vec<u8>> {
        let exp = self.macros[id].expansion.clone();
        let macro_argc = self.macros[id].argc;
        let has_varargs = self.macros[id].has_varargs;
        let last_arg = macro_argc - 1;

        let mut subexp: Vec<u8> = Vec::new();
        let mut start = 0usize;
        let mut expsrc = 0usize;
        let eof = Token::new(Tok::Eof);
        let mut prvtok = eof;
        let mut prvprvtok = eof;
        self.expand_stack.clear();

        let mut tok = self.next_exp_tok(&exp, &mut expsrc);
        while tok.typ != Tok::Eof {
            // The delimiter that ends the current part of a #foreach or
            // an #ifempty.
            if let Some(top) = self.expand_stack.last().copied() {
                if at(&exp, tok.pos) == top.delim {
                    if tok.pos > start {
                        subexp.extend_from_slice(&exp[start..tok.pos]);
                    }
                    expsrc = tok.pos;
                    match top.typ {
                        Tok::MacroForeach => {
                            let more = argofs[top.arg + 1] != 0;
                            let mut end_foreach = false;
                            if top.part == 0 {
                                if !more {
                                    // The last time round: leave out the
                                    // part that goes between items.
                                    skip_delimited_group(&exp, &mut expsrc, 1);
                                    end_foreach = true;
                                } else {
                                    expsrc += 1;
                                    if let Some(level) = self.expand_stack.last_mut() {
                                        level.part += 1;
                                    }
                                }
                            } else {
                                expsrc += 1;
                                end_foreach = true;
                            }
                            if end_foreach {
                                if !more {
                                    self.expand_stack.pop();
                                } else if let Some(level) = self.expand_stack.last_mut() {
                                    level.arg += 1;
                                    expsrc = level.start;
                                    prvtok = eof;
                                    prvprvtok = eof;
                                    level.part = 0;
                                }
                            }
                        }
                        Tok::MacroIfEmpty | Tok::MacroIfNEmpty => {
                            expsrc += 1;
                            self.expand_stack.pop();
                        }
                        _ => {}
                    }
                    start = expsrc;
                    tok = self.next_exp_tok(&exp, &mut expsrc);
                    continue;
                }
            }

            if tok.typ == Tok::MacroForeach && has_varargs {
                if tok.pos > start {
                    subexp.extend_from_slice(&exp[start..tok.pos]);
                }
                if self.expand_stack.len() >= MAX_FOREACH_NESTING {
                    self.error("#foreach/#ifempty/#ifnempty nesting too deep");
                } else if argofs[last_arg] == 0 {
                    // No variable arguments: nothing to repeat.
                    skip_delimited_group(&exp, &mut expsrc, 2);
                } else {
                    let delim = at(&exp, expsrc);
                    expsrc += 1;
                    self.expand_stack.push(ExpandLevel {
                        typ: tok.typ,
                        is_iterator: true,
                        delim,
                        start: expsrc,
                        arg: last_arg,
                        part: 0,
                    });
                }
                start = expsrc;
                tok = self.next_exp_tok(&exp, &mut expsrc);
                continue;
            }

            if matches!(tok.typ, Tok::MacroIfEmpty | Tok::MacroIfNEmpty) && has_varargs {
                if tok.pos > start {
                    subexp.extend_from_slice(&exp[start..tok.pos]);
                }
                let is_empty = argofs[last_arg] == 0;
                let expand = is_empty == (tok.typ == Tok::MacroIfEmpty);
                if expand {
                    if self.expand_stack.len() >= MAX_FOREACH_NESTING {
                        self.error("#foreach/#ifempty/#ifnempty nesting too deep");
                    } else {
                        let delim = at(&exp, expsrc);
                        expsrc += 1;
                        let (is_iterator, arg) = match self.expand_stack.last() {
                            Some(outer) => (outer.is_iterator, outer.arg),
                            None => (false, 0),
                        };
                        self.expand_stack.push(ExpandLevel {
                            typ: tok.typ,
                            is_iterator,
                            delim,
                            start: 0,
                            arg,
                            part: 0,
                        });
                    }
                } else {
                    skip_delimited_group(&exp, &mut expsrc, 1);
                }
                start = expsrc;
                tok = self.next_exp_tok(&exp, &mut expsrc);
                continue;
            }

            if tok.typ == Tok::MacroArgCount && has_varargs {
                if tok.pos > start {
                    subexp.extend_from_slice(&exp[start..tok.pos]);
                }
                let mut i = last_arg;
                while argofs[i] != 0 {
                    i += 1;
                }
                subexp.extend_from_slice((i - last_arg).to_string().as_bytes());
                start = expsrc;
                tok = self.next_exp_tok(&exp, &mut expsrc);
                continue;
            }

            if tok.typ == Tok::MacroFormal {
                let mut p = tok.pos;
                let mut argnum = usize::from(at(&exp, tok.pos + 1)).saturating_sub(1);

                // In a #foreach, the variable-argument parameter stands
                // for the one argument this time round is for.
                if let Some(top) = self.expand_stack.last() {
                    if argnum == last_arg && top.is_iterator {
                        argnum = top.arg;
                    }
                }
                // Outside one, it stands for all of them, commas and all.
                let argnum_len =
                    if self.expand_stack.is_empty() && has_varargs && argnum == last_arg {
                        let mut i = argnum;
                        while i < MAX_MACRO_ARGS && argofs[i] != 0 {
                            i += 1;
                        }
                        if i == argnum {
                            0
                        } else {
                            argofs[i - 1] + arglen[i - 1] - argofs[argnum]
                        }
                    } else {
                        arglen[argnum]
                    };

                let mut pasting = false;
                let mut pasting_at_left = false;
                let mut pasting_at_right = false;
                let mut stringize = false;
                let mut stringize_qu = b'"';
                let mut stringize_type = Tok::Invalid;

                loop {
                    if prvtok.typ == Tok::PoundPound {
                        // Pasting onto what is at the left: the ## and
                        // the spaces before it go.
                        pasting = true;
                        pasting_at_left = true;
                        p = prvtok.pos;
                        let mut prv_ch = 0;
                        while p > start {
                            prv_ch = exp[p - 1];
                            if !is_space(prv_ch) {
                                break;
                            }
                            p -= 1;
                        }
                        // ", ## args" with no arguments loses the comma.
                        if prv_ch == b','
                            && has_varargs
                            && argnum == last_arg
                            && argofs[argnum] == 0
                        {
                            p -= 1;
                        }
                        break;
                    } else if matches!(prvtok.typ, Tok::Pound | Tok::PoundAt) {
                        p = prvtok.pos;
                        stringize = true;
                        stringize_type = prvtok.typ;
                        stringize_qu = if prvtok.typ == Tok::Pound {
                            b'"'
                        } else {
                            b'\''
                        };
                        prvtok = prvprvtok;
                        prvprvtok = eof;
                    } else {
                        break;
                    }
                }

                if p > start {
                    subexp.extend_from_slice(&exp[start..p]);
                }
                prvprvtok = prvtok;
                prvtok = tok;
                tok = self.next_exp_tok(&exp, &mut expsrc);

                let mut paste_at_right_tok = eof;
                if tok.typ == Tok::PoundPound {
                    pasting = true;
                    pasting_at_right = true;
                    let old_expsrc = expsrc;
                    let old_tok = tok;
                    tok = self.next_exp_tok(&exp, &mut expsrc);
                    paste_at_right_tok = tok;
                    if stringize && tok.typ == stringize_type {
                        expsrc = old_expsrc;
                        tok = old_tok;
                    } else {
                        prvprvtok = prvtok;
                        prvtok = old_tok;
                    }
                    start = tok.pos;
                } else {
                    start = prvtok.end();
                }

                let arg_start = argofs[argnum];
                if pasting && stringize {
                    let mut add_open = true;
                    let mut add_close = true;
                    if subexp.last() == Some(&stringize_qu) {
                        subexp.pop();
                        add_open = false;
                    }
                    if pasting_at_right && at(&exp, tok.pos) == stringize_qu {
                        add_close = false;
                    }
                    stringize_macro_actual(
                        &mut subexp,
                        slice(src, arg_start, arg_start + argnum_len),
                        stringize_qu,
                        add_open,
                        add_close,
                    );
                    if !add_close {
                        subexp.extend_from_slice(slice(&exp, tok.pos + 1, tok.end()));
                        prvprvtok = prvtok;
                        prvtok = tok;
                        tok = self.next_exp_tok(&exp, &mut expsrc);
                        start = tok.pos;
                    }
                } else if pasting {
                    let mut argp = arg_start;
                    let mut len = argnum_len;
                    if pasting_at_left && at(src, argp) == FULLY_EXPANDED_FLAG {
                        argp += 1;
                        len = len.saturating_sub(1);
                    }
                    // Two strings pasted together become one.
                    let mut quote_char = 0;
                    let first = at(src, argp);
                    if pasting_at_left
                        && (first == b'\'' || first == b'"')
                        && subexp.last() == Some(&first)
                    {
                        subexp.pop();
                        quote_char = first;
                        argp += 1;
                        len = len.saturating_sub(1);
                    }
                    let mut done = false;
                    if pasting_at_right {
                        let mut old_tok = Token::new(Tok::Invalid);
                        let mut atok = Token::new(Tok::Invalid);
                        let mut q = argp;
                        while q < argp + len {
                            atok = self.next_tok(src, Some(usize::MAX), &mut q, true);
                            if atok.typ == Tok::Eof || atok.pos >= argp + len {
                                break;
                            }
                            old_tok = atok;
                        }
                        if old_tok.typ == Tok::Sym && old_tok.fully_expanded {
                            // The last token is marked as expanded, but
                            // pasting makes a new token of it: take the
                            // mark off.
                            let mark = atok.pos.saturating_sub(1);
                            subexp.extend_from_slice(slice(src, argp, mark));
                            subexp.extend_from_slice(slice(src, atok.pos, argp + len));
                            done = true;
                        } else if quote_char != 0
                            && paste_at_right_tok.len != 0
                            && at(&exp, paste_at_right_tok.pos) == quote_char
                        {
                            subexp.extend_from_slice(slice(
                                src,
                                argp,
                                argp + len.saturating_sub(1),
                            ));
                            subexp.extend_from_slice(slice(
                                &exp,
                                paste_at_right_tok.pos + 1,
                                paste_at_right_tok.end(),
                            ));
                            start = paste_at_right_tok.end();
                            done = true;
                        }
                    }
                    if !done {
                        subexp.extend_from_slice(slice(src, argp, argp + len));
                    }
                } else if stringize {
                    stringize_macro_actual(
                        &mut subexp,
                        slice(src, arg_start, arg_start + argnum_len),
                        stringize_qu,
                        true,
                        true,
                    );
                } else {
                    // An ordinary use: the argument is expanded first,
                    // and what comes out is marked so that it is not
                    // expanded again as part of this macro's expansion.
                    let mut actual_exp = Vec::new();
                    let mut actual_pos = arg_start;
                    if self.expand_macros(
                        src,
                        Some(arg_start + argnum_len),
                        &mut actual_pos,
                        &mut actual_exp,
                        false,
                        allow_defined,
                        false,
                    ) {
                        return None;
                    }
                    self.mark_full_exp_tokens(&mut subexp, &actual_exp);
                }
                continue;
            }

            prvprvtok = prvtok;
            prvtok = tok;
            tok = self.next_exp_tok(&exp, &mut expsrc);
        }

        subexp.extend_from_slice(slice(&exp, start, tok.pos));
        Some(subexp)
    }

    /// Copy expanded text onto `dst`, marking each symbol that names a
    /// macro whose expansion it came from, so that it is left alone from
    /// now on.
    fn mark_full_exp_tokens(&mut self, dst: &mut Vec<u8>, srcbuf: &[u8]) {
        let from = dst.len();
        let mut ec = EmbedCtx::default();
        let mut p = 0;
        let mut start = 0;
        let end = srcbuf.len();
        let mut tok;
        loop {
            let old_ec = ec;
            let mut cx = LexCtx {
                ec: Some(&mut ec),
                expanding: true,
                in_pp_expr: false,
                notes: &mut self.notes,
            };
            tok = lex::next_on_line(srcbuf, &mut p, &mut cx);
            if tok.pos >= end {
                tok = Token::new(Tok::Eof);
                tok.pos = end;
                ec = old_ec;
            }
            if tok.typ == Tok::Eof {
                break;
            }
            if tok.typ == Tok::Sym && !tok.fully_expanded {
                if let Some(id) = self.find_define(tok.text(srcbuf)) {
                    if scan_for_prior_expansion(srcbuf, p, end, id) {
                        if tok.pos > start {
                            dst.extend_from_slice(&srcbuf[start..tok.pos]);
                        }
                        dst.push(FULLY_EXPANDED_FLAG);
                        start = tok.pos;
                    }
                }
            }
        }
        self.notes.clear();
        if tok.pos > start {
            dst.extend_from_slice(&srcbuf[start..tok.pos]);
        }
        remove_end_markers(dst, from);
    }

    /// `defined(NAME)` or `defined NAME` in an `#if`: one or zero.
    fn expand_defined(
        &mut self,
        subexp: &mut Vec<u8>,
        src: &[u8],
        limit: Option<usize>,
        pos: &mut usize,
    ) -> bool {
        let mut tok = self.next_tok(src, limit, pos, false);
        let paren = tok.typ == Tok::LPar;
        if paren {
            tok = self.next_tok(src, limit, pos, false);
        }
        if tok.typ != Tok::Sym {
            self.error(format!(
                "name required for defined() (found \"{}\")",
                show_name(tok.text(src))
            ));
            return true;
        }
        let found = self.find_define(tok.text(src)).is_some();
        *subexp = if found { b"1".to_vec() } else { b"0".to_vec() };
        if paren && self.next_tok(src, limit, pos, false).typ != Tok::RPar {
            self.error("missing ')' in defined()");
            return true;
        }
        false
    }

    // ---- directives ----

    fn pp_charset(&mut self) {
        // The directive is only meaningful at the very start of a file,
        // where it is taken when the file is opened. One that is still
        // here either is somewhere else or names a character set that is
        // not known.
        if self
            .streams
            .last()
            .is_some_and(|stream| stream.charset_error)
        {
            self.error("unable to load #charset");
        } else {
            self.error("unexpected or invalid #charset");
        }
        self.clear_linebuf();
    }

    fn pp_pragma(&mut self) {
        if self.next_on_line() != Tok::Sym {
            let text = self.curtok_text();
            self.warning(format!(
                "unrecognized #pragma \"{}\"; ignored",
                show_name(&text)
            ));
            return;
        }
        let keyword = self.curtok_text();
        match keyword.to_ascii_lowercase().as_slice() {
            b"once" => {
                if let Some(desc) = self.streams.last().map(|stream| stream.desc) {
                    let name = self.descs[desc].name.clone();
                    self.add_include_once(&name);
                }
                self.clear_linebuf();
            }
            b"all_once" => {
                match self.next_on_line() {
                    Tok::Plus | Tok::Eof => self.all_once = true,
                    Tok::Minus => self.all_once = false,
                    _ => self.warning("invalid syntax for #pragma"),
                }
                self.clear_linebuf();
            }
            b"message" => self.pragma_message(),
            b"newline_spacing" => self.pragma_newline_spacing(),
            b"sourcetextgroup" => {
                // This is for the compiler proper; when only
                // preprocessing, the line is passed on to it.
                if !self.pp_only {
                    self.pragma_source_text_group();
                }
            }
            _ => self.warning(format!(
                "unrecognized #pragma \"{}\"; ignored",
                show_name(&keyword)
            )),
        }
    }

    /// `#pragma sourceTextGroup`, `(on)`, or `(off)`: whether the
    /// objects defined from here on say which source file they are in.
    fn pragma_source_text_group(&mut self) {
        let on = match self.next_on_line() {
            Tok::Eof => Some(true),
            Tok::LPar if self.next_on_line() == Tok::Sym => {
                let on = match self.curtok_text().as_slice() {
                    b"on" => Some(true),
                    b"off" => Some(false),
                    _ => None,
                };
                if on.is_some() && self.next_on_line() != Tok::RPar {
                    None
                } else {
                    on
                }
            }
            _ => None,
        };
        match on {
            Some(on) => self.src_group_change = Some(on),
            None => self.warning("invalid syntax for #pragma"),
        }
        self.clear_linebuf();
    }

    /// What `#pragma sourceTextGroup` has said since this was last
    /// asked, if anything.
    pub(crate) fn take_source_text_group(&mut self) -> Option<bool> {
        self.src_group_change.take()
    }

    fn pragma_message(&mut self) {
        let startofs = self.curtok.end();
        self.expbuf = slice(&self.linebuf, 0, startofs).to_vec();
        if self.expand_macros_curline(false, false, true) {
            self.clear_linebuf();
            return;
        }
        if self.pp_only {
            // The line is passed on, with its macros expanded, for the
            // compiler proper to show.
            self.linebuf = self.expbuf.clone();
            return;
        }
        // Show what is in the parentheses: strings and names as they
        // are, numbers as numbers.
        let buf = std::mem::take(&mut self.expbuf);
        self.p = startofs;
        let mut message = String::new();
        let mut typ = self.next_message_token(&buf);
        if typ == Tok::LPar {
            typ = self.next_message_token(&buf);
        } else {
            self.warning("invalid syntax for #pragma");
        }
        while typ != Tok::RPar && typ != Tok::Eof {
            match typ {
                Tok::SStr | Tok::DStr | Tok::Sym => {
                    message.push_str(&String::from_utf8_lossy(&self.cur.text));
                }
                Tok::Int => message.push_str(&(self.cur.int_val as i32).to_string()),
                _ => {}
            }
            typ = self.next_message_token(&buf);
        }
        self.expbuf = buf;
        self.report(Severity::Message, message);
        self.clear_linebuf();
    }

    /// The next token of a `#pragma message`, with strings translated.
    fn next_message_token(&mut self, buf: &[u8]) -> Tok {
        lex::skip_ws_and_markers(buf, &mut self.p);
        if matches!(at(buf, self.p), b'"' | b'\'') {
            return self.xlat_string_to(buf, false);
        }
        let mut cx = LexCtx {
            ec: None,
            expanding: false,
            in_pp_expr: false,
            notes: &mut self.notes,
        };
        let tok = lex::next_on_line(buf, &mut self.p, &mut cx);
        self.cur = SrcToken::new(tok.typ);
        self.cur.text = tok.text(buf).to_vec();
        self.cur.int_val = tok.int_val;
        self.flush_notes();
        tok.typ
    }

    fn pragma_newline_spacing(&mut self) {
        if self.pp_only {
            return;
        }
        let mode = if self.next_on_line() != Tok::LPar || self.next_on_line() != Tok::Sym {
            None
        } else {
            match self.curtok_text().as_slice() {
                b"on" | b"collapse" => Some(NewlineSpacing::Collapse),
                b"off" | b"delete" => Some(NewlineSpacing::Delete),
                b"preserve" => Some(NewlineSpacing::Preserve),
                _ => None,
            }
        };
        match mode {
            Some(mode) if self.next_on_line() == Tok::RPar => self.newline_spacing = mode,
            _ => self.warning("invalid syntax for #pragma"),
        }
        self.clear_linebuf();
    }

    fn add_include_once(&mut self, name: &str) {
        if !self.find_include_once(name) {
            self.include_once.push(name.to_owned());
        }
    }

    fn find_include_once(&self, name: &str) -> bool {
        self.include_once
            .iter()
            .any(|once| file_names_equal(once, name))
    }

    fn pp_include(&mut self) {
        // If the line is just a file name in quotes or angle brackets,
        // it is taken as it is; otherwise macros in it are expanded
        // first.
        let start = self.p;
        let mut expand = true;
        let mut q = self.p;
        while is_space(at(&self.linebuf, q)) {
            q += 1;
        }
        let close = match at(&self.linebuf, q) {
            b'<' => Some(b'>'),
            b'"' => Some(b'"'),
            _ => None,
        };
        if let Some(close) = close {
            q += 1;
            while at(&self.linebuf, q) != 0 && at(&self.linebuf, q) != close {
                q += 1;
            }
            if at(&self.linebuf, q) == close {
                q += 1;
                while is_space(at(&self.linebuf, q)) {
                    q += 1;
                }
                if at(&self.linebuf, q) == 0 {
                    expand = false;
                }
            }
        }
        self.p = start;

        let line: Vec<u8>;
        let mut p;
        if expand {
            if self.expand_macros_curline(false, false, false) {
                self.clear_linebuf();
                return;
            }
            remove_expansion_flags(&mut self.expbuf);
            line = self.expbuf.clone();
            p = 0;
        } else {
            line = self.linebuf.clone();
            p = self.p;
        }

        while is_space(at(&line, p)) {
            p += 1;
        }
        let (close, is_local) = match at(&line, p) {
            b'"' => (b'"', true),
            b'<' => (b'>', false),
            _ => {
                self.error("syntax error in #include");
                self.clear_linebuf();
                return;
            }
        };
        p += 1;
        let name_start = p;
        while at(&line, p) != 0 && at(&line, p) != close {
            p += 1;
        }
        if at(&line, p) == 0 {
            self.error("syntax error in #include");
            self.clear_linebuf();
            return;
        }
        let name = String::from_utf8_lossy(&line[name_start..p]).into_owned();
        p += 1;
        while is_space(at(&line, p)) {
            p += 1;
        }
        if at(&line, p) != 0 {
            self.warning("extraneous characters following #include filename");
        }
        self.clear_linebuf();
        self.include_file(&name, is_local, false);
    }

    /// `import name;`, in Hexicon: the header the library of that name
    /// says to give the files that import it, read as `#include` would
    /// read it.
    fn pp_import(&mut self, name: &str) {
        self.clear_linebuf();
        if self.imported.iter().any(|have| have == name) {
            return;
        }
        let found = self.imports.iter().find(|(have, _)| have == name).cloned();
        match found {
            Some((_, Some(header))) => {
                self.imported.push(name.to_owned());
                self.include_file(&header, false, true);
            }
            Some((_, None)) => self.error(format!(
                "import {name}: library \"{name}\" names no header to import"
            )),
            None => self.error(format!(
                "import {name}: no library \"{name}\" in this build"
            )),
        }
    }

    /// Read a file named by `#include` or found by `import`: in quotes
    /// (`is_local`), beside the file that includes it, and then, as in
    /// angle brackets, along the include path. A header a library names
    /// for `import` (`as_found`) is where its path says, if it is there.
    fn include_file(&mut self, name: &str, is_local: bool, as_found: bool) {
        let name = name.to_owned();
        // A file named in quotes is looked for beside the file that
        // includes it, and then, like one in angle brackets, in each
        // folder of the include path.
        let absolute = is_absolute(&name);
        let mut full_name = None;
        if as_found && self.files.exists(&name) {
            full_name = Some(name.clone());
        }
        if is_local && full_name.is_none() {
            if let Some(desc) = self.last_desc {
                let candidate = join_path(dir_name(&self.descs[desc].name), &name);
                if self.files.exists(&candidate) {
                    full_name = Some(candidate);
                }
            }
        }
        if full_name.is_none() {
            for index in 0..self.include_paths.len() {
                let candidate = join_path(&self.include_paths[index], &name);
                if self.files.exists(&candidate) {
                    full_name = Some(candidate);
                    break;
                }
            }
        }
        if full_name.is_none() && absolute && self.files.exists(&name) {
            full_name = Some(name.clone());
        }
        // Last, the system library's headers, which are part of the
        // compiler.
        let built_in = if full_name.is_none() {
            crate::builtin::system_header(&name)
        } else {
            None
        };
        if built_in.is_some() {
            full_name = Some(format!("{}/{name}", crate::builtin::SYSTEM));
        }

        let Some(full_name) = full_name else {
            self.error(format!("cannot open include file \"{name}\""));
            return;
        };
        if self.find_include_once(&full_name) {
            return;
        }
        let data = match built_in {
            Some(data) => data.to_vec(),
            None => {
                let Some(data) = self.files.read(&full_name) else {
                    self.error(format!("cannot open include file \"{full_name}\""));
                    return;
                };
                data
            }
        };
        let desc = self.file_desc(&full_name);
        if let Some(stream) = self.streams.last_mut() {
            stream.newline_spacing = self.newline_spacing;
        }
        let (data, charset_error) = decode_source(&data, self.files, &self.default_charset);
        self.streams.push(Stream {
            desc,
            data,
            pos: 0,
            next_line: 1,
            in_comment: false,
            init_if_level: self.if_stack.len(),
            newline_spacing: self.newline_spacing,
            charset_error,
        });
        if self.all_once {
            self.add_include_once(&full_name);
        }
    }

    fn pp_define(&mut self) {
        if self.next_on_line() != Tok::Sym {
            let text = self.curtok_text();
            self.error(format!("invalid name \"{}\" for #define", show_name(&text)));
            self.clear_linebuf();
            return;
        }
        let name = self.curtok_text();
        let mut params: Vec<Vec<u8>> = Vec::new();
        let mut has_varargs = false;
        let mut has_args = false;

        // A parameter list has to follow the name with no space between.
        if at(&self.linebuf, self.p) == b'(' {
            has_args = true;
            self.p += 1;
            let mut tok = self.next_on_line();
            let mut done = tok == Tok::RPar;
            while !done {
                if params.len() >= MAX_MACRO_ARGS {
                    self.error(format!(
                        "too many parameters for macro \"{}\" (maximum {MAX_MACRO_ARGS})",
                        show_name(&name)
                    ));
                    self.clear_linebuf();
                    return;
                }
                if tok == Tok::Eof {
                    self.error("missing ')' in macro parameter list");
                    self.clear_linebuf();
                    return;
                }
                if tok != Tok::Sym {
                    let text = self.curtok_text();
                    self.error(format!(
                        "invalid macro parameter name \"{}\"",
                        show_name(&text)
                    ));
                    self.clear_linebuf();
                    return;
                }
                params.push(self.curtok_text());
                tok = self.next_on_line();
                match tok {
                    Tok::Comma => tok = self.next_on_line(),
                    Tok::Ellipsis => {
                        tok = self.next_on_line();
                        has_varargs = true;
                        if tok != Tok::RPar {
                            let text = self.curtok_text();
                            self.error(format!(
                                "'...' in macro parameter list must be followed by ')' - found \"{}\"",
                                show_name(&text)
                            ));
                            self.clear_linebuf();
                            return;
                        }
                        done = true;
                    }
                    Tok::RPar => done = true,
                    _ => {
                        let text = self.curtok_text();
                        self.error(format!(
                            "expected ',' or ')' in macro parameter list (found \"{}\")",
                            show_name(&text)
                        ));
                        self.clear_linebuf();
                        return;
                    }
                }
            }
        }

        while is_space(at(&self.linebuf, self.p)) {
            self.p += 1;
        }
        if name == b"defined" {
            self.error("cannot #define reserved macro name \"defined\"");
            self.clear_linebuf();
            return;
        }
        // The expansion is the rest of the line, as far as its text
        // goes, without the spaces at the end, unless a backslash comes
        // before the last of them.
        let expan_start = self.p;
        let mut expan_end = expan_start;
        while at(&self.linebuf, expan_end) != 0 {
            expan_end += 1;
        }
        while expan_end > expan_start
            && is_space(self.linebuf[expan_end - 1])
            && !(expan_end - expan_start > 1 && self.linebuf[expan_end - 2] == b'\\')
        {
            expan_end -= 1;
        }
        let orig = self.linebuf[expan_start..expan_end].to_vec();
        let expansion = self.parse_expansion(&orig, &params, has_varargs);
        let argc = params.len();

        if let Some(old) = self.find_define(&name) {
            let old_macro = &self.macros[old];
            // Defining a macro again the same way is not a redefinition.
            if old_macro.has_args == has_args
                && old_macro.argc == argc
                && equal_collapse_spaces(&expansion, &self.expansion(old))
            {
                self.clear_linebuf();
                return;
            }
            self.warning(format!("redefinition of macro \"{}\"", show_name(&name)));
            self.undefs.insert(name.clone());
        }
        let id = self.macros.len();
        self.macros.push(Macro {
            name: name.clone(),
            kind: MacroKind::Define,
            has_args,
            argc,
            has_varargs,
            expansion,
            orig,
            params,
        });
        self.defines.insert(name, id);
        self.clear_linebuf();
    }

    /// Put a macro's expansion in the form it is stored in: each use of a
    /// parameter becomes a marker, and so do `#foreach`, `#argcount`,
    /// `#ifempty`, and `#ifnempty` after the variable-argument parameter.
    fn parse_expansion(&mut self, orig: &[u8], params: &[Vec<u8>], has_varargs: bool) -> Vec<u8> {
        if params.is_empty() {
            return orig.to_vec();
        }
        let mut out = Vec::with_capacity(orig.len());
        let mut ec = EmbedCtx::default();
        let mut start = 0;
        let mut src = 0;
        let mut tok;
        loop {
            let mut cx = LexCtx {
                ec: Some(&mut ec),
                expanding: false,
                in_pp_expr: false,
                notes: &mut self.notes,
            };
            tok = lex::next_on_line(orig, &mut src, &mut cx);
            if tok.typ == Tok::Eof {
                break;
            }
            if tok.typ != Tok::Sym {
                continue;
            }
            // With two parameters of one name, the later one is found.
            let Some(index) = params.iter().rposition(|param| param == tok.text(orig)) else {
                continue;
            };
            let mut arg_len = params[index].len();
            let mut flag = FORMAL_FLAG;
            if has_varargs && index == params.len() - 1 && at(orig, src) == b'#' {
                let after = |word: &[u8]| {
                    orig.get(src + 1..src + 1 + word.len()) == Some(word)
                        && !is_sym(at(orig, src + 1 + word.len()))
                };
                if after(b"foreach") {
                    arg_len += 8;
                    flag = FOREACH_FLAG;
                } else if after(b"argcount") {
                    arg_len += 9;
                    flag = ARGCOUNT_FLAG;
                } else if after(b"ifempty") {
                    arg_len += 8;
                    flag = IFEMPTY_FLAG;
                } else if after(b"ifnempty") {
                    arg_len += 9;
                    flag = IFNEMPTY_FLAG;
                }
            }
            if tok.pos > start {
                out.extend_from_slice(&orig[start..tok.pos]);
            }
            start = tok.pos + arg_len;
            out.push(flag);
            if flag == FORMAL_FLAG {
                out.push((index + 1) as u8);
            }
        }
        self.flush_notes();
        if tok.pos > start {
            out.extend_from_slice(slice(orig, start, tok.pos));
        }
        // The stored text ends at a zero byte, if it has one.
        if let Some(end) = out.iter().position(|&c| c == 0) {
            out.truncate(end);
        }
        out
    }

    /// Read a name that has to be the only thing on the rest of the line.
    fn pp_get_lone_ident(&mut self) -> Option<Vec<u8>> {
        if self.next_on_line() != Tok::Sym {
            let text = self.curtok_text();
            self.error(format!("invalid name \"{}\" for #define", show_name(&text)));
            return None;
        }
        let name = self.curtok_text();
        if self.next_on_line() != Tok::Eof {
            self.error("extra characters on line");
            return None;
        }
        Some(name)
    }

    fn pp_ifdef_or_ifndef(&mut self, sense: bool) {
        let Some(name) = self.pp_get_lone_ident() else {
            self.clear_linebuf();
            self.push_if(IfState::IfYes);
            return;
        };
        let found = self.find_define(&name).is_some();
        self.push_if(if sense == found {
            IfState::IfYes
        } else {
            IfState::IfNo
        });
        self.clear_linebuf();
    }

    /// Evaluate the expression of an `#if` or an `#elif`, whose macros
    /// have been expanded into the expansion buffer: whether it is true,
    /// or nothing if it has no constant value.
    fn pp_parse_expr(&mut self) -> Option<bool> {
        let (value, problems) = crate::parse::eval_pp_condition(&self.expbuf);
        self.report_all(problems);
        value
    }

    /// Report the problems found in a directive's expression, at the
    /// directive.
    fn report_all(&mut self, problems: crate::parse::Problems) {
        for (severity, message) in problems {
            self.report(severity, message);
        }
    }

    fn pp_if(&mut self) {
        if self.expand_macros_curline(false, true, false) {
            self.clear_linebuf();
            self.push_if(IfState::IfYes);
            return;
        }
        self.clear_linebuf();
        // An expression that cannot be evaluated counts as true.
        let value = self.pp_parse_expr().unwrap_or(true);
        self.push_if(if value { IfState::IfYes } else { IfState::IfNo });
    }

    fn pp_elif(&mut self) {
        if self.expand_macros_curline(false, true, false) {
            self.clear_linebuf();
            return;
        }
        let Some(value) = self.pp_parse_expr() else {
            self.clear_linebuf();
            return;
        };
        let init_level = self.streams.last().map_or(0, |s| s.init_if_level);
        if self.if_stack.len() <= init_level {
            self.error("#elif without #if");
            self.clear_linebuf();
            return;
        }
        let state = self.if_stack.last().map(|level| level.state);
        match state {
            Some(IfState::IfYes) => self.change_if_state(IfState::IfDone),
            Some(IfState::IfNo) => {
                if value {
                    self.change_if_state(IfState::IfYes);
                }
            }
            Some(IfState::IfDone) => {}
            _ => self.error("#elif without #if"),
        }
        self.clear_linebuf();
    }

    fn change_if_state(&mut self, state: IfState) {
        if let Some(level) = self.if_stack.last_mut() {
            level.state = state;
        }
    }

    fn pp_else(&mut self) {
        if self.next_on_line() != Tok::Eof {
            self.error("extra characters on line");
        }
        let init_level = self.streams.last().map_or(0, |s| s.init_if_level);
        if self.if_stack.len() <= init_level {
            self.error("#else without #if");
            self.clear_linebuf();
            return;
        }
        let state = self.if_stack.last().map(|level| level.state);
        match state {
            Some(IfState::IfYes | IfState::IfDone) => self.change_if_state(IfState::ElseNo),
            Some(IfState::IfNo) => self.change_if_state(IfState::ElseYes),
            _ => self.error("#else without #if"),
        }
        self.clear_linebuf();
    }

    fn pp_endif(&mut self) {
        if self.next_on_line() != Tok::Eof {
            self.error("extra characters on line");
        }
        self.clear_linebuf();
        let init_level = self.streams.last().map_or(0, |s| s.init_if_level);
        // With no #if open at all, or none that this file opened.
        if self.if_stack.len() <= init_level {
            self.error("#endif without #if");
            return;
        }
        self.pop_if();
    }

    fn pp_error(&mut self) {
        let startofs = self.curtok.end();
        self.expbuf = slice(&self.linebuf, 0, startofs).to_vec();
        if self.expand_macros_curline(false, false, true) {
            self.clear_linebuf();
            return;
        }
        remove_expansion_flags(&mut self.expbuf);
        if self.pp_only {
            // The compiler proper reports the error; the line is passed
            // on to it.
            self.linebuf = self.expbuf.clone();
        } else {
            let message =
                String::from_utf8_lossy(slice(&self.expbuf, startofs, usize::MAX)).into_owned();
            self.error(format!("#error : {message}"));
            self.clear_linebuf();
        }
    }

    fn pp_undef(&mut self) {
        if let Some(name) = self.pp_get_lone_ident() {
            self.undefine_bytes(&name);
        }
        self.clear_linebuf();
    }

    fn pp_line(&mut self) {
        if self.expand_macros_curline(false, true, false) {
            self.clear_linebuf();
            return;
        }
        let text = self.expbuf.clone();
        self.clear_linebuf();
        let (value, problems) = crate::parse::eval_pp_line(&text);
        self.report_all(problems);
        if let Some((line, name)) = value {
            let desc = self.file_desc(&String::from_utf8_lossy(&name));
            if let Some(stream) = self.streams.last_mut() {
                stream.next_line = line as u32;
                stream.desc = desc;
            }
        }
    }
}

// ---- helpers ----

/// A piece of a buffer, with both ends kept within it.
fn slice(buf: &[u8], start: usize, end: usize) -> &[u8] {
    let end = end.min(buf.len());
    let start = start.min(end);
    &buf[start..end]
}

/// Where an argument that ends at `end` really ends, without the spaces
/// after it. A backslash before the spaces keeps one of them, since it
/// escapes it.
fn trim_end(buf: &[u8], start: usize, end: usize) -> usize {
    let mut p = end;
    let mut spaces = 0;
    while p > start {
        p -= 1;
        let c = at(buf, p);
        if !is_space(c) {
            p += 1;
            if c == b'\\' && spaces != 0 {
                p += 1;
            }
            break;
        }
        spaces += 1;
    }
    p
}

/// Whether the text from `pos` has a marker for the end of the expansion
/// of the macro `id`: if so, the text is part of that macro's own
/// expansion, and the macro is not expanded again within it.
fn scan_for_prior_expansion(buf: &[u8], pos: usize, end: usize, id: usize) -> bool {
    let mut p = pos;
    while p < end {
        if at(buf, p) == MACRO_EXP_END {
            if lex::macro_end_id(buf, p) == id {
                return true;
            }
            p += 1 + MACRO_ID_LEN;
        } else {
            p += 1;
        }
    }
    false
}

/// Take the markers out of text that is finished with macro expansion.
/// The text ends at a zero byte.
fn remove_expansion_flags(buf: &mut Vec<u8>) {
    let mut out = Vec::with_capacity(buf.len());
    let mut p = 0;
    while at(buf, p) != 0 {
        match buf[p] {
            MACRO_EXP_END => p += 1 + MACRO_ID_LEN,
            FULLY_EXPANDED_FLAG => p += 1,
            c => {
                out.push(c);
                p += 1;
            }
        }
    }
    *buf = out;
}

/// Take the markers for the ends of expansions out of `buf` from `from`.
fn remove_end_markers(buf: &mut Vec<u8>, from: usize) {
    let mut out = buf[..from].to_vec();
    let mut p = from;
    while p < buf.len() {
        if buf[p] == MACRO_EXP_END {
            p += 1 + MACRO_ID_LEN;
        } else {
            out.push(buf[p]);
            p += 1;
        }
    }
    *buf = out;
}

/// Skip the parts of a `#foreach` or `#ifempty` that are not wanted: from
/// the delimiter at `p`, past `parts` more of them.
fn skip_delimited_group(buf: &[u8], p: &mut usize, parts: u32) {
    let delim = at(buf, *p);
    if delim == 0 || delim == lex::END_PP_LINE {
        return;
    }
    *p += 1;
    let mut parts = parts;
    while parts != 0 {
        let c = at(buf, *p);
        if c == 0 || c == lex::END_PP_LINE {
            return;
        }
        if c == delim {
            parts -= 1;
            *p += 1;
        } else if c == FOREACH_FLAG {
            skip_delimited_group(buf, p, 2);
        } else if c == IFEMPTY_FLAG || c == IFNEMPTY_FLAG {
            skip_delimited_group(buf, p, 1);
        } else {
            *p += 1;
        }
    }
}

/// Add an argument to `out` as a string: runs of spaces become one, and
/// quotes and backslashes that would end or change the string are
/// escaped.
fn stringize_macro_actual(out: &mut Vec<u8>, actual: &[u8], quote: u8, open: bool, close: bool) {
    if open {
        out.push(quote);
    }
    let len = actual.len();
    let mut start = 0;
    let mut src = 0;
    let mut in_inner_quote = false;
    let mut inner_quote = 0u8;
    let mut prev = 0u8;
    while src < len {
        let cur = actual[src];
        if is_space(cur) && prev != b'\\' {
            out.extend_from_slice(&actual[start..src]);
            while src < len && is_space(actual[src]) {
                src += 1;
            }
            if prev != 0 && src < len {
                out.push(b' ');
            }
            prev = cur;
            start = src;
            continue;
        }
        if cur == quote || (cur == b'\\' && in_inner_quote) {
            out.extend_from_slice(&actual[start..src]);
            out.push(b'\\');
            start = src;
        }
        if prev != b'\\' {
            if in_inner_quote && cur == inner_quote {
                in_inner_quote = false;
            } else if !in_inner_quote && (cur == b'"' || cur == b'\'') {
                in_inner_quote = true;
                inner_quote = cur;
            }
        }
        prev = cur;
        src += 1;
    }
    out.extend_from_slice(&actual[start..src]);
    if close {
        out.push(quote);
    }
}

/// Whether two expansions are the same, counting any run of spaces as
/// the same as any other.
fn equal_collapse_spaces(a: &[u8], b: &[u8]) -> bool {
    let (mut i, mut j) = (0, 0);
    while i < a.len() && j < b.len() {
        if is_space(a[i]) && is_space(b[j]) {
            while i < a.len() && is_space(a[i]) {
                i += 1;
            }
            while j < b.len() && is_space(b[j]) {
                j += 1;
            }
            continue;
        }
        if a[i] != b[j] {
            return false;
        }
        i += 1;
        j += 1;
    }
    i == a.len() && j == b.len()
}

// ---- file names ----

fn is_separator(c: char) -> bool {
    c == '/' || c == '\\'
}

/// A file's name without its folder.
fn root_name(name: &str) -> &str {
    match name.rfind(is_separator) {
        Some(index) => &name[index + 1..],
        None => name,
    }
}

/// The folder a file is in, or an empty string if its name has none.
fn dir_name(name: &str) -> &str {
    match name.rfind(is_separator) {
        Some(index) => &name[..index],
        None => "",
    }
}

fn join_path(dir: &str, name: &str) -> String {
    if dir.is_empty() || is_absolute(name) {
        name.to_owned()
    } else if dir.ends_with(is_separator) {
        format!("{dir}{name}")
    } else {
        // As the reference joins them: with the system's separator.
        format!("{dir}{}{name}", std::path::MAIN_SEPARATOR)
    }
}

fn is_absolute(name: &str) -> bool {
    let bytes = name.as_bytes();
    name.starts_with(is_separator)
        || (bytes.len() >= 2 && bytes[1] == b':' && bytes[0].is_ascii_alphabetic())
}

/// Whether two names are for the same file, as far as the names show.
fn file_names_equal(a: &str, b: &str) -> bool {
    let normal = |name: &str| name.replace('\\', "/").to_ascii_lowercase();
    normal(a) == normal(b)
}

/// Which chain of the reference's table of macros a name is in.
pub(crate) fn macro_chain(name: &[u8]) -> usize {
    name.iter().map(|&byte| usize::from(byte)).sum::<usize>() % 512
}

/// A name as messages show it: a character that is not ASCII is given
/// as its code.
pub(crate) fn show_name(name: &[u8]) -> String {
    // The reference shows at most 60 bytes of a name, and marks one it
    // has cut short.
    const LIMIT: usize = 60;
    let cut = name.len() > LIMIT;
    let name = if cut { &name[..LIMIT] } else { name };
    let mut out = String::new();
    for c in String::from_utf8_lossy(name).chars() {
        if c.is_ascii() {
            out.push(c);
        } else {
            out.push_str(&format!("\\u{:04x}", u32::from(c)));
        }
    }
    if cut {
        out.push_str("...");
    }
    out
}

/// What stands in for a byte that the file's character set has no
/// character for.
const REPLACEMENT: &str = "\u{fffd}";

/// Text in one of the character sets that are built in, as UTF-8; `None`
/// for a set that is not known.
fn decode_charset(name: &str, bytes: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(bytes.len());
    match name.to_ascii_lowercase().as_str() {
        "utf-8" | "utf8" => out.extend_from_slice(bytes),
        "us-ascii" | "ascii" | "asc7dflt" => {
            for &c in bytes {
                if c.is_ascii() {
                    out.push(c);
                } else {
                    out.extend_from_slice(REPLACEMENT.as_bytes());
                }
            }
        }
        "iso-8859-1" | "iso8859-1" | "iso_8859-1" | "latin1" | "latin-1" | "iso1" | "8859-1"
        | "l1" | "cp819" | "iso-ir-100" => {
            for &c in bytes {
                out.extend_from_slice(char::from(c).encode_utf8(&mut [0; 4]).as_bytes());
            }
        }
        _ => return None,
    }
    Some(out)
}

/// The name in `#charset "name"`, if that is how the text starts, and
/// how long the directive is, up to and including its closing quote.
fn charset_directive(text: &[u8]) -> Option<(&[u8], usize)> {
    let rest = text.strip_prefix(b"#charset ")?;
    let spaces = rest
        .iter()
        .take_while(|&&c| c == b' ' || c == b'\t')
        .count();
    let rest = rest[spaces..].strip_prefix(b"\"")?;
    let len = rest
        .iter()
        .position(|&c| c == b'"' || c == b'\n' || c == b'\r')?;
    if rest[len] != b'"' {
        return None;
    }
    Some((&rest[..len], 9 + spaces + 1 + len + 1))
}

/// A source file's bytes as UTF-8, and whether the file names a
/// character set that is not known.
///
/// A file that starts with a byte order mark is in the Unicode encoding
/// the mark is for. Otherwise a `#charset "name"` directive at the very
/// start of the file says what it is in, and without one it is plain
/// in the default character set, which is UTF-8 unless whoever is
/// compiling says otherwise. A directive that is understood is taken out, leaving the rest
/// of its line, so that the lines after it keep their numbers; one that
/// is not is left for the preprocessor to complain about.
fn decode_source(
    data: &[u8],
    files: &mut dyn SourceFiles,
    default_charset: &str,
) -> (Vec<u8>, bool) {
    if let Some(rest) = data.strip_prefix(&[0xef, 0xbb, 0xbf]) {
        let skip = match charset_directive(rest) {
            Some((name, len)) if name.eq_ignore_ascii_case(b"utf-8") => len,
            Some((name, len)) if name.eq_ignore_ascii_case(b"utf8") => len,
            _ => 0,
        };
        return (rest[skip..].to_vec(), false);
    }
    for (mark, big_endian, names) in [
        ([0xff, 0xfe], false, ["unicodel", "utf-16le"]),
        ([0xfe, 0xff], true, ["unicodeb", "utf-16be"]),
    ] {
        let Some(rest) = data.strip_prefix(&mark) else {
            continue;
        };
        let units: Vec<u16> = rest
            .chunks_exact(2)
            .map(|pair| {
                if big_endian {
                    u16::from_be_bytes([pair[0], pair[1]])
                } else {
                    u16::from_le_bytes([pair[0], pair[1]])
                }
            })
            .collect();
        let text = String::from_utf16_lossy(&units).into_bytes();
        let skip = match charset_directive(&text) {
            Some((name, len))
                if names
                    .iter()
                    .any(|n| name.eq_ignore_ascii_case(n.as_bytes())) =>
            {
                len
            }
            _ => 0,
        };
        return (text[skip..].to_vec(), false);
    }
    let mut charset_error = false;
    if let Some((name, len)) = charset_directive(data) {
        let name = String::from_utf8_lossy(name);
        let text =
            decode_charset(&name, &data[len..]).or_else(|| files.decode(&name, &data[len..]));
        match text {
            Some(text) => return (text, false),
            None => charset_error = true,
        }
    }
    let text = decode_charset(default_charset, data)
        .or_else(|| files.decode(default_charset, data))
        .or_else(|| decode_charset("us-ascii", data))
        .unwrap_or_default();
    (text, charset_error)
}

/// Whether the word from `start` to `end` in a line stands where an
/// object's name goes: nothing but spaces and `+` signs before it, and a
/// colon after it (not `::`).
fn stands_as_name(line: &[u8], start: usize, end: usize) -> bool {
    let start = start.min(line.len());
    let end = end.clamp(start, line.len());
    if !line[..start]
        .iter()
        .all(|&c| c == b' ' || c == b'\t' || c == b'+')
    {
        return false;
    }
    let rest = &line[end..];
    let after = rest
        .iter()
        .position(|&c| c != b' ' && c != b'\t')
        .map(|i| &rest[i..]);
    matches!(after, Some([b':', next, ..]) if *next != b':') || after == Some(b":")
}
