//! Hexicon's player: a window that plays games.
//!
//! It shows the same page as Hexicon in a web browser, in the system's own
//! web view (WebView2 on Windows, WebKit on macOS and Linux), and the game
//! runs in the page, as the browser's does. The page's files are built into
//! the program and given to the web view from it; nothing is fetched from
//! anywhere. A game named on the command line is opened at the start; the
//! page's "Open a game", or a game dropped on the window, opens any other.
//!
//! The files a game writes, its saved games above all, are files in a
//! folder of the player's: `Hexicon` in their documents, with a folder for
//! each game, named after its file. The page asks for them when it opens a
//! game, and sends each one the game writes or deletes. The player's
//! settings are kept by the web view, in the user's folder for programs'
//! data.
//!
//! usage: hexicon-player [game.hxg]
//!
//! HEXICON_GAMES_FOLDER, if it is set, is the folder for the games' files.
//! HEXICON_CHECK_CHOOSE, for checks, answers every file dialog with the
//! path it names (or none, if it is empty) instead of showing it.

use std::borrow::Cow;
use std::cell::RefCell;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;

use tao::event::{Event, WindowEvent};
use tao::event_loop::{ControlFlow, EventLoopBuilder};
use tao::window::WindowBuilder;
use wry::http::{Request, Response, StatusCode, header::CONTENT_TYPE};
use wry::{WebContext, WebViewBuilder};

/// The page's files: their names, their contents, and their types.
const FILES: &[(&str, &[u8], &str)] = &[
    (
        "index.html",
        include_bytes!("../../../web/index.html"),
        "text/html; charset=utf-8",
    ),
    (
        "main.js",
        include_bytes!("../../../web/main.js"),
        "text/javascript; charset=utf-8",
    ),
    (
        "style.css",
        include_bytes!("../../../web/style.css"),
        "text/css; charset=utf-8",
    ),
    (
        "pkg/hexicon_wasm.js",
        include_bytes!("../../../web/pkg/hexicon_wasm.js"),
        "text/javascript; charset=utf-8",
    ),
    (
        "pkg/hexicon_wasm_bg.wasm",
        include_bytes!("../../../web/pkg/hexicon_wasm_bg.wasm"),
        "application/wasm",
    ),
];

/// The protocol the page is given by. On Windows a web view reaches it as
/// `http://hexicon.localhost/`, and elsewhere as `hexicon://localhost/`.
const PROTOCOL: &str = "hexicon";

/// Where the game named on the command line is, for the page to load.
const GAME_PATH: &str = "game/";

/// Where a game's files are, for the page to load: `files/<game>`.
const FILES_PATH: &str = "files/";

fn main() {
    let game = std::env::args_os().nth(1).map(PathBuf::from);
    if let Some(path) = &game {
        if !path.is_file() {
            eprintln!("hexicon-player: {}: there is no such file", path.display());
            std::process::exit(1);
        }
    }

    let event_loop = EventLoopBuilder::<PageEvent>::with_user_event().build();
    let proxy = event_loop.create_proxy();
    let window = match WindowBuilder::new()
        .with_title("Hexicon")
        .with_inner_size(tao::dpi::LogicalSize::new(1000.0, 760.0))
        .build(&event_loop)
    {
        Ok(window) => window,
        Err(error) => {
            eprintln!("hexicon-player: no window: {error}");
            std::process::exit(1);
        }
    };

    let base = if cfg!(any(windows, target_os = "android")) {
        format!("http://{PROTOCOL}.localhost/")
    } else {
        format!("{PROTOCOL}://localhost/")
    };
    let mut address = format!("{base}index.html");
    if let Some(path) = &game {
        let name = path.file_name().map_or_else(
            || "game.hxg".to_owned(),
            |name| name.to_string_lossy().into_owned(),
        );
        address.push_str(&format!("?game=/{GAME_PATH}{}", encode(&name)));
    }

    let folder = games_folder();
    // The files the player has chosen in a dialog, outside the game's
    // folder: a game can write those, and no others outside it.
    let chosen: Rc<RefCell<HashSet<PathBuf>>> = Rc::default();
    let message_chosen = chosen.clone();
    let mut context = WebContext::new(data_folder());
    let game_file = game.clone();
    let protocol_folder = folder.clone();
    let title_proxy = proxy.clone();
    let built = WebViewBuilder::new_with_web_context(&mut context)
        .with_url(&address)
        .with_custom_protocol(PROTOCOL.to_owned(), move |_id, request| {
            respond(&request, game_file.as_deref(), &protocol_folder)
        })
        .with_ipc_handler(move |request| {
            if let Some(event) = take_message(request.body(), &folder, &message_chosen.borrow()) {
                let _ = proxy.send_event(event);
            }
        })
        .with_document_title_changed_handler(move |title| {
            let _ = title_proxy.send_event(PageEvent::Title(title));
        })
        .build(&window);
    let webview = match built {
        Ok(webview) => webview,
        Err(error) => {
            eprintln!("hexicon-player: the system's web view could not be started: {error}");
            std::process::exit(1);
        }
    };

    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::Wait;
        match event {
            // The page's title is the window's: the game's, once it says.
            Event::UserEvent(PageEvent::Title(title)) => window.set_title(&title),
            Event::UserEvent(PageEvent::Script(script)) => {
                let _ = webview.evaluate_script(&script);
            }
            // Shown here, and not while the web view is telling the
            // window of the page's message, so that the web view is not
            // held up while the player chooses.
            Event::UserEvent(PageEvent::Dialog {
                save,
                folder,
                prompt,
                kind,
            }) => {
                let asked = Asked {
                    save,
                    folder: &folder,
                    prompt: &prompt,
                    kind: &kind,
                };
                let script = choose_file(&window, &asked, &mut chosen.borrow_mut());
                let _ = webview.evaluate_script(&script);
            }
            Event::WindowEvent {
                event: WindowEvent::CloseRequested,
                ..
            } => {
                // The web view goes with the window.
                let _ = &webview;
                *control_flow = ControlFlow::Exit;
            }
            _ => {}
        }
    });
}

/// What the page tells the window.
#[derive(Debug)]
enum PageEvent {
    /// The page's title changed.
    Title(String),
    /// Something for the page to run: the answer to what it sent.
    Script(String),
    /// The game asks for a file, to save or not, in its folder, with its
    /// prompt, and of a kind: `game` for a saved game, `text`, or `file`.
    Dialog {
        save: bool,
        folder: PathBuf,
        prompt: String,
        kind: String,
    },
}

/// A file dialog the game asks for.
struct Asked<'a> {
    save: bool,
    folder: &'a Path,
    prompt: &'a str,
    kind: &'a str,
}

/// The files of a kind, as a dialog shows them: their description and
/// their extension.
fn kind_filter(kind: &str) -> Option<(&'static str, &'static str)> {
    match kind {
        "game" => Some(("Saved games", "hxs")),
        "text" => Some(("Text files", "txt")),
        _ => None,
    }
}

/// The answer to a request from the page: one of its files, the game, or
/// the game's files.
fn respond(
    request: &Request<Vec<u8>>,
    game: Option<&Path>,
    folder: &Path,
) -> Response<Cow<'static, [u8]>> {
    let path = request.uri().path().trim_start_matches('/');
    if let Some(name) = path.strip_prefix(FILES_PATH) {
        let listing = game_files(&game_folder(folder, &decode(name)));
        return answer(
            StatusCode::OK,
            "text/plain; charset=utf-8",
            Cow::Owned(listing.into_bytes()),
        );
    }
    if let Some(name) = path.strip_prefix(GAME_PATH) {
        if let Some(game) = game {
            let wanted = decode(name);
            let actual = game.file_name().map(|n| n.to_string_lossy().into_owned());
            if actual.as_deref() == Some(wanted.as_str()) {
                if let Ok(bytes) = std::fs::read(game) {
                    return answer(
                        StatusCode::OK,
                        "application/octet-stream",
                        Cow::Owned(bytes),
                    );
                }
            }
        }
        return answer(
            StatusCode::NOT_FOUND,
            "text/plain",
            Cow::Borrowed(b"no such game"),
        );
    }
    let path = if path.is_empty() { "index.html" } else { path };
    match FILES.iter().find(|(name, _, _)| *name == path) {
        Some((_, contents, kind)) => answer(StatusCode::OK, kind, Cow::Borrowed(contents)),
        None => answer(
            StatusCode::NOT_FOUND,
            "text/plain",
            Cow::Borrowed(b"not found"),
        ),
    }
}

fn answer(
    status: StatusCode,
    kind: &str,
    body: Cow<'static, [u8]>,
) -> Response<Cow<'static, [u8]>> {
    Response::builder()
        .status(status)
        .header(CONTENT_TYPE, kind)
        .body(body)
        .unwrap_or_else(|_| Response::new(Cow::Borrowed(b"")))
}

/// Where the web view keeps what the page stores, in the user's folder
/// for programs' data: on Windows `%LOCALAPPDATA%\Hexicon`, on macOS
/// `~/Library/Application Support/Hexicon`, and elsewhere
/// `$XDG_DATA_HOME/hexicon` (`~/.local/share/hexicon`).
fn data_folder() -> Option<PathBuf> {
    let var = |name: &str| std::env::var_os(name).map(PathBuf::from);
    let folder = if cfg!(windows) {
        var("LOCALAPPDATA")?.join("Hexicon")
    } else if cfg!(target_os = "macos") {
        var("HOME")?.join("Library/Application Support/Hexicon")
    } else {
        var("XDG_DATA_HOME")
            .or_else(|| var("HOME").map(|home| home.join(".local/share")))?
            .join("hexicon")
    };
    std::fs::create_dir_all(&folder).ok()?;
    Some(folder)
}

/// The folder for the games' files, where the player can find them:
/// `Hexicon` in their documents, or in their home folder if they have no
/// folder for documents. `HEXICON_GAMES_FOLDER` names another, for a
/// player kept on a stick, say, or for a check that must not touch the
/// player's own.
fn games_folder() -> PathBuf {
    if let Some(folder) = std::env::var_os("HEXICON_GAMES_FOLDER") {
        return PathBuf::from(folder);
    }
    dirs::document_dir()
        .or_else(dirs::home_dir)
        .unwrap_or_default()
        .join("Hexicon")
}

/// The folder for one game's files: named after the game's file, without
/// `.hxg`.
fn game_folder(folder: &Path, game: &str) -> PathBuf {
    let name = game
        .strip_suffix(".hxg")
        .or_else(|| game.strip_suffix(".HXG"))
        .unwrap_or(game);
    folder.join(file_name_for(name))
}

/// A game's files, for the page: a line for each, its name as it goes in
/// an address and its contents in base64. A folder that is not there yet
/// has none.
fn game_files(folder: &Path) -> String {
    let mut listing = String::new();
    let Ok(entries) = std::fs::read_dir(folder) else {
        return listing;
    };
    for entry in entries.flatten() {
        let file = entry.file_name().to_string_lossy().into_owned();
        // The player's own files, part written, start with a dot.
        if file.starts_with('.') || !entry.path().is_file() {
            continue;
        }
        if let Ok(contents) = std::fs::read(entry.path()) {
            let name = name_from_file(&file);
            listing.push_str(&format!("{} {}\n", encode(&name), BASE64.encode(contents)));
        }
    }
    listing
}

/// Do what the page asks of a game's files, which it sends as a line of
/// words: `write <game> <name> <contents>`, `delete <game> <name>`,
/// `show <game>`, or `dialog <open|save> <game> <prompt> <kind>`, the names and
/// the prompt as they go in an address and the contents in base64. What
/// goes wrong comes back as a script for the page to run; a dialog, as
/// the event that shows it.
///
/// A game's file is in its folder, unless it is one the player chose in a
/// dialog somewhere else, which a game names by the whole of its path.
fn take_message(message: &str, folder: &Path, chosen: &HashSet<PathBuf>) -> Option<PageEvent> {
    let mut words = message.split(' ');
    let verb = words.next()?;
    if verb == "dialog" {
        let save = words.next()? == "save";
        return Some(PageEvent::Dialog {
            save,
            folder: game_folder(folder, &decode(words.next()?)),
            prompt: decode(words.next().unwrap_or("")),
            kind: words.next().unwrap_or("file").to_owned(),
        });
    }
    let game = game_folder(folder, &decode(words.next()?));
    let name = words.next().map(decode);
    let result = match (verb, &name) {
        ("write", Some(name)) => file_path(&game, name, chosen).and_then(|path| {
            BASE64
                .decode(words.next().unwrap_or(""))
                .map_err(|error| error.to_string())
                .and_then(|contents| write_file(&path, &contents).map_err(|e| e.to_string()))
        }),
        ("delete", Some(name)) => {
            file_path(&game, name, chosen).and_then(|path| match std::fs::remove_file(path) {
                Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
                    Err(error.to_string())
                }
                _ => Ok(()),
            })
        }
        ("show", None) => show_folder(&game).map_err(|error| error.to_string()),
        _ => return None,
    };
    let error = result.err()?;
    Some(PageEvent::Script(format!(
        "hexiconPlayer.failed({}, {})",
        js_string(name.as_deref().unwrap_or("")),
        js_string(&error)
    )))
}

/// Where one of a game's files is: in its folder, or, for a whole path,
/// where the player chose it to be.
fn file_path(game: &Path, name: &str, chosen: &HashSet<PathBuf>) -> Result<PathBuf, String> {
    let path = Path::new(name);
    if !path.is_absolute() {
        return Ok(game.join(file_name_for(name)));
    }
    if chosen.contains(path) {
        Ok(path.to_owned())
    } else {
        Err("it is outside the game's folder, and was not chosen there".to_owned())
    }
}

/// Write a file: first beside it, then in its place, so that a file is
/// never left half written.
fn write_file(path: &Path, contents: &[u8]) -> std::io::Result<()> {
    let folder = path.parent().unwrap_or(Path::new("."));
    std::fs::create_dir_all(folder)?;
    let file = path.file_name().unwrap_or_default().to_string_lossy();
    let part = folder.join(format!(".{file}.part"));
    std::fs::write(&part, contents)?;
    std::fs::rename(&part, path)
}

/// Show the system's dialog for a file the game asks for, starting in the
/// game's folder, and give the page what the player chose, as a script:
/// `hexiconPlayer.fileChosen(name, contents)`, with the name the game is
/// to know it by (its name in the folder, or the whole of its path
/// elsewhere), and for a file to open, its contents in base64; or
/// `hexiconPlayer.fileChosen(null)` for none.
///
/// The dialog shows the files of the kind the game asks for (saved games,
/// `.hxs`, or text, `.txt`), with all files a choice when opening one; a
/// file to save of that kind, named without an extension, is given it.
fn choose_file(
    window: &tao::window::Window,
    asked: &Asked,
    chosen: &mut HashSet<PathBuf>,
) -> String {
    let Asked {
        save,
        folder,
        prompt,
        kind,
    } = *asked;
    let _ = std::fs::create_dir_all(folder);
    let mut dialog = rfd::FileDialog::new()
        .set_title(prompt.trim().trim_end_matches(['>', ':']).trim())
        .set_directory(folder)
        .set_parent(window);
    let filter = kind_filter(kind);
    if let Some((description, extension)) = filter {
        dialog = dialog.add_filter(description, &[extension]);
        if !save {
            dialog = dialog.add_filter("All files", &["*"]);
        }
    }
    // A check, which cannot use the dialog, gives the answer instead: a
    // path, or nothing for none.
    let picked = if let Some(answer) = std::env::var_os("HEXICON_CHECK_CHOOSE") {
        Some(PathBuf::from(answer)).filter(|path| !path.as_os_str().is_empty())
    } else if save {
        dialog.save_file()
    } else {
        dialog.pick_file()
    };
    let Some(mut path) = picked else {
        return "hexiconPlayer.fileChosen(null)".to_owned();
    };
    if let Some((_, extension)) = filter {
        if save && path.extension().is_none() {
            path.set_extension(extension);
        }
    }
    let in_folder = path
        .parent()
        .is_some_and(|parent| same_folder(parent, folder));
    let name = if in_folder {
        name_from_file(&path.file_name().unwrap_or_default().to_string_lossy())
    } else {
        chosen.insert(path.clone());
        path.to_string_lossy().into_owned()
    };
    if save {
        return format!("hexiconPlayer.fileChosen({})", js_string(&name));
    }
    match std::fs::read(&path) {
        Ok(contents) => format!(
            "hexiconPlayer.fileChosen({}, {})",
            js_string(&name),
            js_string(&BASE64.encode(contents))
        ),
        Err(error) => format!(
            "hexiconPlayer.failed({}, {}); hexiconPlayer.fileChosen(null)",
            js_string(&name),
            js_string(&error.to_string())
        ),
    }
}

/// Whether two paths are the same folder, however each is written.
fn same_folder(one: &Path, other: &Path) -> bool {
    match (one.canonicalize(), other.canonicalize()) {
        (Ok(one), Ok(other)) => one == other,
        _ => one == other,
    }
}

/// Open a game's folder in the system's file manager.
fn show_folder(folder: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(folder)?;
    let manager = if cfg!(windows) {
        "explorer"
    } else if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    std::process::Command::new(manager).arg(folder).spawn()?;
    Ok(())
}

/// The name of the file a game's file is kept in. A game can name a file
/// anything, and a folder cannot have every name, so what a folder cannot
/// have is written as `%` and its code: a slash, a colon, a name the
/// system keeps for itself (`CON`), a dot or a space at the end. Every
/// other name is the file's own, so a saved game is found by its name.
fn file_name_for(name: &str) -> String {
    const KEPT: &[&str] = &["CON", "PRN", "AUX", "NUL"];
    let mut out = String::new();
    let count = name.chars().count();
    let stem = name.split('.').next().unwrap_or("").to_ascii_uppercase();
    let kept = KEPT.contains(&stem.as_str())
        || (stem.len() == 4
            && (stem.starts_with("COM") || stem.starts_with("LPT"))
            && stem.as_bytes()[3].is_ascii_digit());
    for (index, c) in name.chars().enumerate() {
        let last = index + 1 == count;
        let escape = c.is_control()
            || "<>:\"/\\|?*%".contains(c)
            || (last && (c == '.' || c == ' '))
            || (index == 0 && (kept || c == '.'));
        if escape {
            let mut bytes = [0; 4];
            for byte in c.encode_utf8(&mut bytes).bytes() {
                out.push_str(&format!("%{byte:02X}"));
            }
        } else {
            out.push(c);
        }
    }
    if out.is_empty() {
        out.push_str("%00");
    }
    out
}

/// The name a game knows a file by, from the file's own name: its
/// escapes undone.
fn name_from_file(file: &str) -> String {
    if file == "%00" {
        String::new()
    } else {
        decode(file)
    }
}

/// Text as a string in JavaScript.
fn js_string(text: &str) -> String {
    let mut out = String::from("\"");
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c if (c as u32) < 0x20 || c == '\u{2028}' || c == '\u{2029}' => {
                out.push_str(&format!("\\u{:04x}", c as u32));
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// A file name as it goes in an address.
fn encode(name: &str) -> String {
    let mut out = String::new();
    for byte in name.bytes() {
        if byte.is_ascii_alphanumeric() || b"-._~".contains(&byte) {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

/// A name from an address, its escapes undone.
fn decode(name: &str) -> String {
    let bytes = name.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(byte) = u8::from_str_radix(&name[i + 1..i + 3], 16) {
                out.push(byte);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_files_name_is_its_own_where_a_folder_can_have_it() {
        assert_eq!(file_name_for("my save"), "my save");
        assert_eq!(file_name_for("start.sav"), "start.sav");
        assert_eq!(file_name_for("caf\u{e9}"), "caf\u{e9}");
    }

    #[test]
    fn what_a_folder_cannot_have_is_escaped_and_comes_back() {
        for (name, file) in [
            ("a/b", "a%2Fb"),
            ("..", "%2E%2E"),
            (".hidden", "%2Ehidden"),
            ("end.", "end%2E"),
            ("end ", "end%20"),
            ("con", "%63on"),
            ("COM1.txt", "%43OM1.txt"),
            ("what?", "what%3F"),
            ("100%", "100%25"),
            ("", "%00"),
        ] {
            assert_eq!(file_name_for(name), file, "{name}");
            assert_eq!(name_from_file(file), name, "{file}");
        }
    }

    #[test]
    fn a_file_outside_the_folder_is_only_one_the_player_chose() {
        let game = Path::new("games").join("skull");
        let outside = std::env::temp_dir().join("elsewhere.sav");
        let mut chosen = HashSet::new();
        assert_eq!(
            file_path(&game, "first save", &chosen),
            Ok(game.join("first save"))
        );
        // A name with a slash in it is a name, not a way out.
        assert_eq!(
            file_path(&game, "../escape", &chosen),
            Ok(game.join("%2E.%2Fescape"))
        );
        let whole = outside.to_string_lossy().into_owned();
        assert!(file_path(&game, &whole, &chosen).is_err());
        chosen.insert(outside.clone());
        assert_eq!(file_path(&game, &whole, &chosen), Ok(outside));
    }

    #[test]
    fn a_dialog_is_asked_for_by_a_message() {
        let folder = Path::new("games");
        match take_message(
            "dialog save skull.hxg Save%20game%20to%20file%20> game",
            folder,
            &HashSet::new(),
        ) {
            Some(PageEvent::Dialog {
                save,
                folder: game,
                prompt,
                kind,
            }) => {
                assert!(save);
                assert_eq!(kind_filter(&kind), Some(("Saved games", "hxs")));
                assert_eq!(game, folder.join("skull"));
                assert_eq!(prompt, "Save game to file >");
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_games_files_are_written_listed_and_deleted() {
        let folder = std::env::temp_dir().join(format!("hexicon-app-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        let none = HashSet::new();
        let send = |message: &str| match take_message(message, &folder, &none) {
            Some(PageEvent::Script(script)) => Some(script),
            Some(other) => panic!("{other:?}"),
            None => None,
        };
        let one = BASE64.encode(b"one");
        let two = BASE64.encode(b"two");
        assert_eq!(send(&format!("write skull.hxg a%2Fb {one}")), None);
        assert_eq!(send(&format!("write skull.hxg plain {two}")), None);
        let game = game_folder(&folder, "skull.hxg");
        assert_eq!(game, folder.join("skull"));
        assert_eq!(std::fs::read(game.join("a%2Fb")).unwrap(), b"one");
        let mut listing: Vec<String> = game_files(&game).lines().map(str::to_owned).collect();
        listing.sort();
        assert_eq!(listing, [format!("a%2Fb {one}"), format!("plain {two}")]);
        assert_eq!(send("delete skull.hxg plain"), None);
        assert_eq!(send("delete skull.hxg plain"), None);
        assert_eq!(game_files(&game), format!("a%2Fb {one}\n"));
        let failed = send("write skull.hxg x !!!").unwrap();
        assert!(
            failed.starts_with("hexiconPlayer.failed(\"x\", "),
            "{failed}"
        );
        assert_eq!(send("nonsense"), None);
        let _ = std::fs::remove_dir_all(&folder);
    }
}
