# Hexad

Hexad is the library for Hexicon games: the world model, the parser, and the messages a game is built on.

It begins as adv3Lite 2.2.3.2, by Eric Eve, which was itself built from Michael J. Roberts's adv3 and Mercury libraries. adv3Lite's license is in [LICENSE.MD](LICENSE.MD), and its notice is kept with every copy. What Hexad is to become, and how, is in [docs/hexad-plan.md](../../docs/hexad-plan.md).

## What is here

- `hexicon.toml`: the library's manifest. Its sources are in the order adv3Lite's `adv3Lite.tl` gives them.
- The library's sources, in Hexicon's language (`.hxn`), and `advlite.h`, the header its games include. They were converted from adv3Lite's `.t` files by `hexicon convert`, which gave the built-in functions Hexicon's names (`hexicon.h`) and changed nothing else.
- `english/`: the English messages and parsing, with a manifest of their own, chosen by `LANGUAGE`.
- `extensions/`: adv3Lite's optional extensions, which a game includes by naming them among its sources (`"extensions/relations"` in a manifest).
- `browser.t` and `menuweb.t`, which belong to adv3Lite's web variant and are not in the manifest yet.

adv3Lite's manual, templates, and cut-down variants (`adv3Liter`, `adv3Litest`) are not part of the library.

## Using it

A game names Hexad among its libraries, with `LANGUAGE` defined:

```toml
[game]
libraries = ["system", "hexad"]
sources = ["start"]

[defines]
LANGUAGE = "english"

[build]
library-paths = ["path/to/hexicon/lib"]
```

A TADS 3 project file can name it too: `-lib hexad`, with `-Fs` giving the folder that holds `hexad`.
