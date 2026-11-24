# Hexicon

Hexicon is a system for writing interactive fiction: games the player reads, and plays by typing commands such as `open the box` or `go north`. It has three parts:

- **the language** a game is written in, in `.hxn` files;
- **Hexad**, the library every game is built on: rooms, things, doors, characters, the parser, and the messages a game prints;
- **the `hexicon` program**, which builds a game into a single `.hxg` file and plays it, in a terminal or, through WebAssembly, in a browser.

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

## Documentation

[The Hexicon Guide](docs/guide/README.md) takes you from nothing to a small finished game, and covers Hexad and the language. The plans behind Hexicon are in [docs](docs).

## Testing

```
cargo test --workspace
python tools/program-check.py
python tools/doc-check.py
```

[tests/README.md](tests/README.md) says what each checks.

## License

Hexicon is free software: you can redistribute it and modify it under the terms of the GNU General Public License, version 3 or (at your option) any later version. See [LICENSE](LICENSE).
