# Hexicon

Hexicon is a system for writing interactive fiction: games the player reads, and plays by typing commands such as `open the box` or `go north`. It has three parts:

- **the language** a game is written in, in `.hxn` files;
- **Hexad**, the library every game is built on: rooms, things, doors, characters, the parser, and the messages a game prints;
- **the `hexicon` program**, which builds a game into a single `.hxg` file and plays it in a terminal; and **the player**, which plays it in a window of its own, with its pictures, sounds, and styled text, as a web browser does.

## Getting started

Hexicon is written in Rust. With Rust installed ([rustup.rs](https://rustup.rs)), build it from this folder:

```
cargo build --release
```

That makes one program, `hexicon`, in `target/release`, with Hexad and everything else a game needs inside it. Then:

```
hexicon new my-game
cd my-game
hexicon build
hexicon run my-game.hxg
```

## Playing in a window, or in a browser

The player, `hexicon-player`, shows a game in the system's web view (WebView2 on Windows, WebKit on macOS and Linux), and `web/` is the same page for a browser. Both need the WebAssembly build (see the top of `tools/build-web.sh` for what that takes):

```
tools/build-app.sh
target/release/hexicon-player my-game.hxg
```

A game can also be opened from the player's window, or dropped on it. The player keeps each game's saved games as files, in `Hexicon` in your documents, with a folder for each game; its "Saved games" button opens it. Saving and restoring show the system's own file dialog, starting in that folder.

On Windows, `tools/build-installer.sh` builds an installer, `build/installer/hexicon-<version>-setup.exe`, with [Inno Setup 6](https://jrsoftware.org/isinfo.php). It installs for you alone, with no administrator's rights: the player, which then opens `.hxg` files when they are opened in Explorer, and the compiler, on your PATH, unless you untick it. Settings > Apps uninstalls it. The programs are not signed yet, so Windows may warn before the installer runs, and an antivirus program may hold the player the first time it starts while it checks it.

On a Mac, `tools/build-mac-app.sh` builds the player as an app, `build/mac/Hexicon.app`, which opens `.hxg` files from the Finder, and a disk image to hand to others, `build/mac/Hexicon-<version>.dmg` (`UNIVERSAL=1` builds it for Apple silicon and Intel Macs both). The app is signed for the Mac that built it alone; another Mac will refuse it until it is signed and notarized for a release.

`tools/build-web.sh` builds the browser's page alone, into `web/`; `node tools/web-serve.js` serves it at `http://127.0.0.1:8765/web/index.html`.

## Documentation

[The Hexicon Guide](docs/guide/README.md) takes you from nothing to a small finished game, and covers Hexad and the language. The plans behind Hexicon are in [docs](docs).

## Testing

```
cargo test
python tools/program-check.py
python tools/doc-check.py
```

`cargo test` leaves out the player, which needs the page built first; with it built, `cargo test --workspace` includes it.

[tests/README.md](tests/README.md) says what each checks.

## License

Hexicon is free software: you can redistribute it and modify it under the terms of the GNU General Public License, version 3 or (at your option) any later version. See [LICENSE](LICENSE).

The libraries in `lib` (the system library and Hexad) carry an additional permission, [LIBRARY-EXCEPTION](LIBRARY-EXCEPTION): a game built on them may be distributed under terms of its author's choice.
