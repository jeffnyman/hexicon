# Hexad

Hexad is the library for Hexicon games: the world model, the parser, and the messages a game is built on.

What Hexad is to become, and how, is in [docs/hexad-plan.md](../../docs/hexad-plan.md).

## What is here

- `hexicon.toml`: the library's manifest. Its sources are in the order adv3Lite's `adv3Lite.tl` gives them.
- The library's sources, in Hexicon's language (`.hxn`), and `advlite.h`, the header its games include. They were converted from adv3Lite's `.t` files by `hexicon convert`, which gave the built-in functions Hexicon's names (`hexicon.h`) and changed nothing else.
- `english/`: the English messages and parsing, with a manifest of their own, chosen by `LANGUAGE`.
- `extensions/`: adv3Lite's optional extensions, which a game includes by naming them among its sources (`"extensions/relations"` in a manifest).
- `browser.t` and `menuweb.t`, which belong to adv3Lite's web variant and are not in the manifest yet.

adv3Lite's manual, templates, and cut-down variants (`adv3Liter`, `adv3Litest`) are not part of the library.

## How it differs from adv3Lite

- The built-in functions have Hexicon's names (`hexicon.h`).
- Its sources import it (`import hexad;`) rather than including `advlite.h`.
- Actions and their handlers are Hexicon's declarations (`action Take(dobj): TAction`, `dobj Take { ... }`, `dobj Search as LookIn`), not `DefineTAction`, `dobjFor`, and `asDobjFor`. They make the same program. `advlite.h` keeps the macros, so a game may use either.
- The library's `ModuleID` is `moduleHexad`, which VERSION shows as "Hexad version 0.1.0", where adv3Lite has `moduleAdv3Lite`.

`tools/compile-tests/hexad-compare.sh` plays adv3Lite's games on both libraries and checks that this is all that differs.

## Using it

`hexicon new my-game` starts a game on Hexad, ready to build and play. Hexad is built into Hexicon, as its system library is, so a game needs nothing else.

A game names Hexad among its libraries, after the system library (`system`), with `LANGUAGE` defined:

```toml
[game]
libraries = ["system", "hexad"]
sources = ["start"]

[defines]
LANGUAGE = "english"
```

A Hexad found where a build is told to look (`library-paths`, `-Fs`) comes before the built-in one, which is how this folder's copy is worked on.

Each Hexicon source of the game imports it, which gives the file Hexad's header (`advlite.h`, named in Hexad's manifest):

```
import hexad;
```

A TADS 3 project file can name it too: `-lib hexad`, with `-Fs` giving the folder that holds `hexad`, and its sources include `<advlite.h>`.
