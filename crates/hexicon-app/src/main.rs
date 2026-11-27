//! Hexicon's player: a window that plays games.
//!
//! It shows the same page as Hexicon in a web browser, in the system's own
//! web view (WebView2 on Windows, WebKit on macOS and Linux), and the game
//! runs in the page, as the browser's does. The page's files are built into
//! the program and given to the web view from it; nothing is fetched from
//! anywhere. A game named on the command line is opened at the start; the
//! page's "Open a game" opens any other. What the page keeps, the player's
//! settings and the games' saved files, is kept in the user's own folder for
//! programs' data.
//!
//! usage: hexicon-player [game.hxg]

use std::borrow::Cow;
use std::path::{Path, PathBuf};

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

    let mut context = WebContext::new(data_folder());
    let game_file = game.clone();
    let built = WebViewBuilder::new_with_web_context(&mut context)
        .with_url(&address)
        .with_custom_protocol(PROTOCOL.to_owned(), move |_id, request| {
            respond(&request, game_file.as_deref())
        })
        .with_document_title_changed_handler(move |title| {
            let _ = proxy.send_event(PageEvent::Title(title));
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
}

/// The answer to a request from the page: one of its files, or the game.
fn respond(request: &Request<Vec<u8>>, game: Option<&Path>) -> Response<Cow<'static, [u8]>> {
    let path = request.uri().path().trim_start_matches('/');
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
