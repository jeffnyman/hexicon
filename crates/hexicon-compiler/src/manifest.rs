//! Hexicon's manifest, `hexicon.toml`: what a game is built from, or
//! what a library is made of.
//!
//! A game's manifest says what a `.t3m` project file says, and is read
//! by turning it into the same options; a library's says what a `.tl`
//! library says. A name the manifest does not know is an error, so that
//! a misspelling is not silently ignored.
//!
//! ```toml
//! [game]
//! sources = ["start.hxn", "plane.hxn"]
//! libraries = ["system", "adv3Lite"]
//!
//! [defines]
//! LANGUAGE = "english"
//! ```

use toml::{Table, Value};

/// The name a manifest has.
pub const MANIFEST: &str = "hexicon.toml";

/// Whether `path` names a manifest rather than a `.t3m` or `.tl` file.
pub(crate) fn is_manifest(path: &str) -> bool {
    path.len() >= 5 && path[path.len() - 5..].eq_ignore_ascii_case(".toml")
}

/// The file a manifest's source names: one named with no extension is
/// a Hexicon source.
pub(crate) fn source_name(name: &str) -> String {
    let file = name.rsplit(['/', '\\']).next().unwrap_or(name);
    if file.contains('.') {
        name.to_owned()
    } else {
        format!("{name}.hxn")
    }
}

/// What a library's manifest says.
#[derive(Debug, Default)]
pub(crate) struct Library {
    /// Its source files, relative to its folder, in order.
    pub sources: Vec<String>,
    /// The libraries it is built on, relative to its folder: a folder
    /// with a manifest, or a `.tl` library named without its extension.
    pub libraries: Vec<String>,
    /// Files and folders to bundle as resources.
    pub resources: Vec<String>,
    /// Macros a game has to define, and why.
    pub needs: Vec<(String, String)>,
    /// The build does not start with the system library's `_main`
    /// (`nodef` in a `.tl`).
    pub no_system_main: bool,
}

/// A table of the manifest, and its name for messages.
struct Section<'t> {
    table: &'t Table,
    name: &'static str,
}

impl Section<'_> {
    fn check_keys(&self, known: &[&str]) -> Result<(), String> {
        for key in self.table.keys() {
            if !known.contains(&key.as_str()) {
                return Err(format!("unknown key \"{key}\" in [{}]", self.name));
            }
        }
        Ok(())
    }

    fn string(&self, key: &str) -> Result<Option<String>, String> {
        match self.table.get(key) {
            None => Ok(None),
            Some(Value::String(s)) => Ok(Some(s.clone())),
            Some(_) => Err(format!("\"{key}\" in [{}] must be a string", self.name)),
        }
    }

    fn boolean(&self, key: &str) -> Result<Option<bool>, String> {
        match self.table.get(key) {
            None => Ok(None),
            Some(Value::Boolean(b)) => Ok(Some(*b)),
            Some(_) => Err(format!(
                "\"{key}\" in [{}] must be true or false",
                self.name
            )),
        }
    }

    fn strings(&self, key: &str) -> Result<Vec<String>, String> {
        match self.table.get(key) {
            None => Ok(Vec::new()),
            Some(Value::Array(items)) => items
                .iter()
                .map(|item| match item {
                    Value::String(s) => Ok(s.clone()),
                    _ => Err(self.not_strings(key)),
                })
                .collect(),
            Some(_) => Err(self.not_strings(key)),
        }
    }

    fn not_strings(&self, key: &str) -> String {
        format!("\"{key}\" in [{}] must be a list of strings", self.name)
    }
}

fn parse(text: &[u8]) -> Result<Table, String> {
    let text = std::str::from_utf8(text).map_err(|_| "the manifest is not UTF-8".to_owned())?;
    text.parse::<Table>()
        .map_err(|error| error.to_string().trim_end().to_owned())
}

fn section<'t>(top: &'t Table, name: &'static str) -> Result<Option<Section<'t>>, String> {
    match top.get(name) {
        None => Ok(None),
        Some(Value::Table(table)) => Ok(Some(Section { table, name })),
        Some(_) => Err(format!("\"{name}\" must be a table ([{name}])")),
    }
}

fn check_sections(top: &Table, known: &[&str]) -> Result<(), String> {
    for key in top.keys() {
        if !known.contains(&key.as_str()) {
            return Err(format!("unknown section [{key}]"));
        }
    }
    Ok(())
}

/// The options a game's manifest stands for, as `t3make` would take
/// them from a project file.
pub(crate) fn game_args(text: &[u8]) -> Result<Vec<String>, String> {
    let top = parse(text)?;
    if top.contains_key("library") {
        return Err("this is the manifest of a library, not of a game".to_owned());
    }
    check_sections(&top, &["game", "defines", "build"])?;
    let game = section(&top, "game")?.ok_or("there is no [game] section")?;
    game.check_keys(&["name", "sources", "libraries", "image", "resources"])?;
    let mut args: Vec<String> = Vec::new();
    let mut push = |items: &[&str]| args.extend(items.iter().map(|&s| s.to_owned()));

    if let Some(build) = section(&top, "build")? {
        build.check_keys(&[
            "preinit",
            "debug",
            "warnings",
            "warnings-as-errors",
            "verbose",
            "charset",
            "include-paths",
            "library-paths",
            "source-text-group",
        ])?;
        if build.boolean("debug")? == Some(true) {
            push(&["-d"]);
        }
        match build.boolean("preinit")? {
            Some(true) => push(&["-pre"]),
            Some(false) => push(&["-nopre"]),
            None => {}
        }
        if build.boolean("warnings")? == Some(false) {
            push(&["-w0"]);
        }
        if build.boolean("warnings-as-errors")? == Some(true) {
            push(&["-we"]);
        }
        if build.boolean("verbose")? == Some(true) {
            push(&["-v"]);
        }
        if build.boolean("source-text-group")? == Some(true) {
            push(&["-Gstg"]);
        }
        if let Some(charset) = build.string("charset")? {
            push(&["-cs", &charset]);
        }
        for path in build.strings("include-paths")? {
            push(&["-I", &path]);
        }
        for path in build.strings("library-paths")? {
            push(&["-Fs", &path]);
        }
    }

    if let Some(defines) = section(&top, "defines")? {
        for (name, value) in defines.table {
            match value {
                Value::String(s) => push(&["-D", &format!("{name}={s}")]),
                Value::Integer(n) => push(&["-D", &format!("{name}={n}")]),
                Value::Boolean(true) => push(&["-D", name]),
                Value::Boolean(false) => push(&["-U", name]),
                _ => {
                    return Err(format!(
                        "\"{name}\" in [defines] must be a string, a number, or true or false"
                    ));
                }
            }
        }
    }

    if let Some(image) = game.string("image")? {
        push(&["-o", &image]);
    }
    let _ = game.string("name")?;

    // The libraries, then the sources, as a project file has them.
    match game.table.get("libraries") {
        None => {}
        Some(Value::Array(items)) => {
            for item in items {
                match item {
                    Value::String(name) => push(&["-lib", name]),
                    Value::Table(table) => {
                        let lib = Section {
                            table,
                            name: "game.libraries",
                        };
                        lib.check_keys(&["name", "exclude"])?;
                        let name = lib
                            .string("name")?
                            .ok_or("a library given as a table needs a \"name\"")?;
                        push(&["-lib", &name]);
                        for module in lib.strings("exclude")? {
                            push(&["-x", &module]);
                        }
                    }
                    _ => return Err(bad_libraries()),
                }
            }
        }
        Some(_) => return Err(bad_libraries()),
    }
    let sources = game.strings("sources")?;
    if sources.is_empty() {
        return Err("[game] names no \"sources\"".to_owned());
    }
    for source in sources {
        push(&["-source", &source]);
    }

    let resources = game.strings("resources")?;
    if !resources.is_empty() {
        push(&["-res"]);
        for resource in resources {
            push(&[&resource]);
        }
    }
    Ok(args)
}

fn bad_libraries() -> String {
    "\"libraries\" in [game] must be a list of names, or of tables with a \"name\"".to_owned()
}

/// What a library's manifest says.
pub(crate) fn library(text: &[u8]) -> Result<Library, String> {
    let top = parse(text)?;
    if top.contains_key("game") {
        return Err("this is the manifest of a game, not of a library".to_owned());
    }
    check_sections(&top, &["library", "needs"])?;
    let lib = section(&top, "library")?.ok_or("there is no [library] section")?;
    lib.check_keys(&["name", "sources", "libraries", "resources", "system-main"])?;
    let _ = lib.string("name")?;
    let mut needs = Vec::new();
    if let Some(section) = section(&top, "needs")? {
        for key in section.table.keys() {
            let why = section.string(key)?.unwrap_or_default();
            needs.push((key.clone(), why));
        }
    }
    Ok(Library {
        sources: lib.strings("sources")?,
        libraries: lib.strings("libraries")?,
        resources: lib.strings("resources")?,
        needs,
        no_system_main: lib.boolean("system-main")? == Some(false),
    })
}
