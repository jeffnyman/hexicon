# Hexicon Language Plan

Status: draft for discussion, 2026-11-23

## Goal

Hexicon's own language is TADS 3 with the changes that writing games in it has shown are worth making, and room to keep growing apart from it. It is the language of the Hexad library ([hexad-plan.md](hexad-plan.md)) and of new games.

TADS 3 itself stays as it is. Every existing game and library keeps compiling and running exactly as it does with the reference, and that comparison keeps checking the compiler and the VM. Diverging means adding a second language beside TADS 3, not changing TADS 3.

## Two dialects, one compiler

- **A source file is TADS 3 or Hexicon, and its extension says which.** TADS 3 files compile exactly as they do now, with the reference's quirks. Nothing in this plan changes them.
- **Both dialects compile to the same symbols, objects, and image.** A Hexicon file can use adv3Lite's classes, and a TADS 3 file can use what a Hexicon file defines. That makes the move gradual: a library or a game can change one file at a time.
- **Hexicon files drop behavior kept only for parity.** Examples are a preprocessor warning reported twice, the reference's check for the end of an `#if` line one token late, and `'ab' + nil` reading the reference's memory. Messages can give the column and show the line.
- **The image stays T3 until something needs more.** A feature that needs the VM's help gets it as an addition that existing images never use, so the VM keeps running every TADS 3 game unchanged. A new image format is a decision for when a feature requires one, not before.

## How a language with no reference stays tested

Today every stage is checked against the reference. A Hexicon-only feature has nothing to compare against, so the checking has to come from somewhere else. Three methods keep most of the byte-for-byte checking:

1. **Each change is defined by what it means in TADS 3.** A change has a lowering: the TADS 3 it stands for. Two programs, one written with the feature and one written out in TADS 3, must produce identical images. Anything that lowers is checked byte for byte, the way the compiler is checked now.
2. **A converter rewrites TADS 3 source as Hexicon source** (`hexicon convert`). It is checked by a round trip: adv3, adv3Lite, Airport, and Ditch Day are converted and compiled, and each image must be identical to the original's. The same tool is how authors move their games, so the check is also what lets them trust it.
3. **Changes that do not lower are tested as the VM's own features are,** with unit tests and conformance programs of our own. The aim is to keep this group small.

## File extensions

| What | TADS 3 | Hexicon (proposed) |
|---|---|---|
| Source | `.t` | `.hxn` |
| Header | `.h` | None needed once modules exist (below). A `.h` can still be included. |
| Library manifest | `.tl` | `hexicon.toml` |
| Project | `.t3m` | `hexicon.toml` |
| Image | `.t3` | `.t3` while the format is T3 |
| Saved game | `.t3v` | `.t3v` while the format is T3 |

The obvious names are taken: `.hex` is Intel HEX and `.hx` is Haxe. As far as I know, `.hxn` is not in common use.

One manifest format for games and libraries replaces `.t3m` and `.tl`, and their command-line options (`-D`, `-I`, `-lib`, `-o`). It is in the spirit of `Cargo.toml`, and it fits the goal of a modern toolchain. A sketch:

```toml
[game]
name = "Banana"
sources = ["src/*.hxn"]

[library]
hexad = { modules = ["senses", "postures"] }
```

`.t3m` and `.tl` stay accepted for TADS 3 projects.

## What earns a change

- **It removes a common kind of mistake or a lot of ceremony.** A change that is only a different spelling is not worth the move.
- **A TADS 3 author can read Hexicon source on sight.** These stay as they are:
  - objects and classes, the `+` location syntax, and templates;
  - double-quoted strings that print, `<<...>>`, and `<<one of>>`;
  - `modify` and `replace`;
  - `nil`.
- **It lowers to TADS 3 where it can,** for the testing reasons above.

## Candidate changes

These are in rough order of value for the cost. The syntax shown is illustrative only; settling it is part of each change's own design.

### 1. Names the compiler checks

TADS 3 creates a property the first time any source names it, so a misspelling is silent. If a game says `isTakable` where the library means `isTakeable`, the object simply never gets the property and the test is `nil`. `dobjFor(Takee)` is the same mistake at the scale of a whole handler: it defines `verifyDobjTakee` and the rest, and nothing ever calls them. The reference never warns about either.

Once a program is linked, the compiler knows every property that anything defines, assigns, or declares. A property that is only ever read is almost always a misspelling. Hexicon files get a warning for it, with the nearest defined name as a suggestion. A program that builds property names at run time can declare them. The check changes no code, so there is nothing to lower.

### 2. Actions and their handlers as declarations, not macros

In both libraries an action is made with macros from `adv3.h` or `advlite.h`:

- `DefineTAction(Take)` makes the class and its property pointers;
- `VerbRule(Take)` makes the grammar;
- `dobjFor(Take)` is `propertyset '*DobjTake'`;
- `logical` is a macro too.

Because these are macros, the compiler does not know what an action is, and a mistake inside one is reported in the expansion. In Hexicon, an action and a handler are declarations the compiler understands:

```
action Take: TAction
    grammar ('take' | 'pick' 'up' | 'get') multiDobj | 'pick' multiDobj 'up';

Thing
    handle Take as dobj {
        preCond = [touchObj]
        verify { if (!isTakeable) illogical(cannotTakeMsg); }
        check { ... }
        action { ... }
    }
```

Each declaration lowers to exactly what the macros produce today, so the images are identical. The main open question is that adv3 and adv3Lite lower handlers to slightly different property names. Two answers are possible: the library could declare its own lowering, much as templates are declared now, or Hexicon could support only Hexad's lowering.

### 3. Modules instead of headers

Every file of a TADS 3 game starts with `#include <adv3.h>` or `<advlite.h>`, and every name in the game and its library shares one namespace. Hexicon files would say `import hexad;` instead. That brings in the library's declarations (templates, actions, and the macros that remain) without pasting text, and a name that clashes can be qualified. Names in the image stay global, made unique where they need to be, so modules lower without a VM change. The preprocessor stays for conditional compilation.

### 4. Messages written for the reader

Hexicon files get messages with the line and column and the line of source itself, and they come from declarations rather than from inside a macro expansion. TADS 3 files keep the reference's messages, which the comparison checks.

### Later, and only if wanted

- Optional type annotations (`isTakeable: bool`), checked by the compiler and erased in the image.
- Pattern matching over lists and objects.

Both are large and neither is needed for Hexad. They are listed so that the early changes do not close them off.

## Open questions

1. Are the extensions and the `hexicon.toml` manifest right?
2. How far should the language go? This plan is conservative: TADS 3 plus checks and declarations. A bolder language is possible, at the cost of the converter round trip and of authors reading it on sight.
3. Should handler syntax serve both libraries or only Hexad?

## Milestones

1. **The dialect switch and the manifest.** `.hxn` files compile as `.t` files do, without the parity-only behavior, and `hexicon.toml` builds a project. Exit: Airport's sources, renamed and built from a manifest, give an image identical to the one built from `airport.t3m`.
2. **The converter.** Exit: adv3, adv3Lite, Airport, and Ditch Day each convert, compile, and give images identical to the originals'.
3. **Checked names.** Exit: no false warnings over adv3Lite, Hexad, and the converted games, and every misspelling a test plants is caught.
4. **Action declarations.** Exit: the converter writes them for adv3Lite's actions, and the round trip still gives identical images.
5. **Modules.** Exit: Hexad and a game built with `import` rather than headers, with identical images.
