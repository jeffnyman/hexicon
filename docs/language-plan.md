# Hexicon Language Plan

Status: draft for discussion, 2026-11-23

## Goal

Hexicon's own language is TADS 3 with the changes that writing games in it has shown are worth making, and room to keep growing apart from it. It is the language of the Hexad library ([hexad-plan.md](hexad-plan.md)) and of new games.

The long-term direction (2026-11-23) is that Hexicon becomes its own thing, as if it had always been, and TADS 3 is retired: TADS 3 gave Hexicon its foundation, not its identity. Hexicon may in the end not play TADS 3 games at all. Until then, TADS 3 stays as it is. Every existing game and library keeps compiling and running exactly as it does with the reference, because that comparison is what checks the compiler and the VM. [Retiring TADS 3](#retiring-tads-3) says when that can change.

## Decided

- Source files are `.hxn` and projects and libraries are `hexicon.toml` (2026-11-23).
- Hexad starts from adv3Lite and lives in this repository as the built-in library ([hexad-plan.md](hexad-plan.md)).
- Nothing in Hexicon's own language keeps a TADS name for its own sake. Names such as `t3GetGlobalSymbols` get Hexicon names.

## Retiring TADS 3

TADS 3 is retired in stages, each one bought by having something else take over its job of checking the toolchain.

1. **Now: both.** The reference comparison is the backbone of testing, and Hexicon's language is checked by lowering to TADS 3.
2. **Hexicon checks itself.** The converter's round trip is done. Hexad, the converted games, and Hexicon's own conformance programs have recorded transcripts that serve as Hexicon's own regression tests. Hexicon's language and library use no TADS names. TADS 3 is now a compatibility mode kept apart from the rest, in its own crate or module, so that nothing in Hexicon's own code carries its names or its quirks.
3. **Hexicon's VM goes its own way.** When a feature needs a change that T3 images cannot have, such as a new image format or a different object model, TADS 3 support is either dropped or kept as a frozen loader for old images. That is decided then, knowing what it costs. The games in the playground are all TADS 3 games, and dropping TADS 3 means they no longer play.

Stage 2 has to come before stage 3. Without the reference or tests of its own, nothing would tell us that the compiler or the VM is right.

## Two dialects, one compiler

- **A source file is TADS 3 or Hexicon, and its extension says which.** TADS 3 files compile exactly as they do now, with the reference's quirks. Nothing in this plan changes them.
- **Both dialects compile to the same symbols, objects, and image.** A Hexicon file can use adv3Lite's classes, and a TADS 3 file can use what a Hexicon file defines. That makes the move gradual: a library or a game can change one file at a time.
- **Hexicon files drop behavior kept only for parity.** So far (2026-11-23) there are two such changes:
  - A warning is reported once. The reference reads each source twice and reports a warning from the first reading again in the second.
  - An `#if`, `#elif`, or `#line` with something after its expression is an error. The reference checks for the end of the line one token late, so `#if 1 2` passes there.

  `'ab' + nil` already adds nothing in both dialects, since the reference's result depends on its memory. Later, messages can give the column and show the line. An included file is read in the dialect of the source that includes it.
- **The image stays T3 until something needs more.** A feature that needs the VM's help gets it as an addition that existing images never use, so the VM keeps running every TADS 3 game unchanged. A new image format is a decision for when a feature requires one, not before.

## How a language with no reference stays tested

Today every stage is checked against the reference. A Hexicon-only feature has nothing to compare against, so the checking has to come from somewhere else. Three methods keep most of the byte-for-byte checking:

1. **Each change is defined by what it means in TADS 3.** A change has a lowering: the TADS 3 it stands for. Two programs, one written with the feature and one written out in TADS 3, must produce identical images. Anything that lowers is checked byte for byte, the way the compiler is checked now.
2. **A converter rewrites TADS 3 source as Hexicon source** (`hexicon convert`). It is checked by a round trip: adv3, adv3Lite, Airport, and Ditch Day are converted and compiled, and each image must be identical to the original's. The same tool is how authors move their games, so the check is also what lets them trust it.
3. **Changes that do not lower are tested as the VM's own features are,** with unit tests and conformance programs of our own. The aim is to keep this group small.

## File extensions

| What | TADS 3 | Hexicon |
|---|---|---|
| Source | `.t` | `.hxn` |
| Header | `.h` | None needed once modules exist (below). A `.h` can still be included. |
| Library manifest | `.tl` | `hexicon.toml` |
| Project | `.t3m` | `hexicon.toml` |
| Image | `.t3` | `.t3` while the format is T3 |
| Saved game | `.t3v` | `.t3v` while the format is T3 |

The obvious names are taken: `.hex` is Intel HEX and `.hx` is Haxe. As far as I know, `.hxn` is not in common use.

One manifest format for games and libraries replaces `.t3m` and `.tl`, and their command-line options (`-D`, `-I`, `-lib`, `-o`). It is in the spirit of `Cargo.toml`, and it fits the goal of a modern toolchain. `hexicon build` reads `hexicon.toml` in the current folder by default, before `makefile.t3m`. It can also be given a manifest with `-f`, a folder holding one, or a `.toml` file among its arguments.

A game's manifest is read by turning it into the options a project file would give, so it builds what the same `.t3m` builds:

```toml
[game]
name = "Airport"                         # not used yet
sources = ["start", "gatearea", "plane", "npcs"]
libraries = ["system", "adv3Lite"]       # or { name = "adv3Lite", exclude = ["scene"] }
image = "airport.t3"                     # default: the first source's name
resources = ["pics"]                     # as after -res: file, or file=name

[defines]                                # -D; true is -D NAME, false is -U NAME
LANGUAGE = "english"

[build]
preinit = true                           # -pre and -nopre
debug = false                            # -d
warnings = true                          # -w0 when false
warnings-as-errors = false               # -we
verbose = false                          # -v
source-text-group = false                # -Gstg
charset = "cp1252"                       # -cs
include-paths = ["include"]              # -I
library-paths = ["../libs"]              # -Fs
```

A library's manifest says what a `.tl` says:

```toml
[library]
name = "adv3Lite Library"
sources = ["action.t", "actions.t"]      # ...
libraries = ["$(LANGUAGE)"]              # $(NAME) is a macro's value, as in a .tl
resources = ["webuires"]
system-main = true                       # false is nodef

[needs]                                  # needmacro
LANGUAGE = "this selects the language the library uses"
```

- **A source named without an extension is a `.hxn` file.** A TADS 3 source in a manifest is named with its `.t`.
- **A module is named without its extension**, as in a `.tl` or `.t3m`. Programs can see the names, so this keeps images the same.
- **A library is a folder with a manifest, or a `.tl` file named without its extension.** It is found where its name says, or along the library paths, and either kind can be built on the other.
- **A name the manifest does not know is an error**, so a misspelled key is not silently ignored.

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

### 0. Hexicon names for the built-in functions and classes

The VM's function sets are bound by their identifiers (`t3vm/010006`, `tads-gen/030008`), not by the names a header gives their functions. A header written for Hexicon can give the same functions new names. The image is unchanged, and the VM needs no change. The identifiers in the image change only at stage 3 of retiring TADS 3.

Done (2026-11-23) for the functions and constants. `#include <hexicon.h>` gives these names:

| TADS 3 | Hexicon |
|---|---|
| `t3RunGC` | `collectGarbage` |
| `t3SetSay` | `setSay` |
| `t3GetVMVsn` | `vmVersion` |
| `t3GetVMID` | `vmId` |
| `t3GetVMBanner` | `vmBanner` |
| `t3GetVMPreinitMode` | `inPreinit` |
| `t3DebugTrace` | `debugTrace` |
| `t3GetGlobalSymbols` | `globalSymbols` |
| `t3AllocProp` | `newProperty` |
| `t3GetStackTrace` | `stackTrace` |
| `t3GetNamedArg` | `namedArg` |
| `t3GetNamedArgList` | `namedArgList` |
| `tadsSay` | `print` |
| `T3DebugCheck`, `T3DebugBreak`, `T3DebugLog` | `DebugTraceCheck`, `DebugTraceBreak`, `DebugTraceLog` |
| `T3SetSayNoFunc`, `T3SetSayNoMethod` | `SetSayNoFunc`, `SetSayNoMethod` |
| `T3GlobalSymbols`, `T3PreprocMacros` | `GlobalSymbolTable`, `MacroTable` |
| `T3GetStackLocals`, `T3GetStackDesc` | `StackTraceLocals`, `StackTraceDesc` |
| `T3MacroHasArgs`, `T3MacroHasVarargs` | `MacroHasArgs`, `MacroHasVarargs` |

None of the new names is defined anywhere in adv3, adv3Lite, the system library, Airport, or Ditch Day. `say` was not used for `tadsSay`, because both libraries define a `say` function of their own.

What was found:

- **`hexicon.h` is part of the compiler** (`crates/hexicon-compiler/include`). An `#include` that finds nothing on the include path looks among the compiler's own headers last, so a file of the same name on the include path comes first. Messages place it in `<hexicon>/hexicon.h`. Because it is built in, it works wherever the compiler does, in WebAssembly too.
- **A function set can be declared twice in one program, under different names.** The reference accepts it, and gives the image one entry for the set; the second declaration only adds names. Declaring the same name again in the same place is accepted too. So `hexicon.h` and `tads.h` can be included together, which Hexad needs while its sources still include `advlite.h`.
- **`hexicon.h` includes the reference's `tadsgen.h`, `tadsio.h`, and `systype.h`**, whose names are already neutral, rather than copies of them. So `tadsSay` is still declared beside `print`, until Hexicon has its own copies of the system headers.
- **An intrinsic class cannot be renamed this way.** The reference says "intrinsic class has been previously defined as \"TadsObject\"" when a second name is given to one. `TadsObject` keeps its name for now. Authors rarely write it, since `object` and `class` stand for it. `T3StackInfo` is a class in the system library's `_main.t`, not in a header, so renaming it waits for Hexicon's own system library.
- **The check is a lowering check.** The same program written with each set of names gives an identical image (a unit test, and a test program that calls most of the renamed functions). With preinitialization, one object's number differs, because the build gives preinitialization the program's symbols and macros, and the two programs define different names.

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

1. How far should the language go? This plan is conservative: TADS 3 plus checks and declarations. A bolder language is possible, at the cost of the converter round trip and of authors reading it on sight. With TADS 3 to be retired, reading it on sight matters less over time than it does now.
2. Should handler syntax serve both libraries or only Hexad? With TADS 3 to be retired, Hexad alone is the likely answer.

## Milestones

1. **The dialect switch and the manifest.** `.hxn` files compile as `.t` files do, without the parity-only behavior, and `hexicon.toml` builds a project. Exit: Airport's sources, renamed and built from a manifest, give an image identical to the one built from `airport.t3m`. Done (2026-11-23). `tools/compile-tests/manifest-compare.sh` builds Airport as `.hxn` sources from a `hexicon.toml`, with adv3Lite and its English library described by manifests written from their `.tl` files. The image is identical to the one built from `airport.t3m`.
2. **The converter.** Exit: adv3, adv3Lite, Airport, and Ditch Day each convert, compile, and give images identical to the originals'.
3. **Checked names.** Exit: no false warnings over adv3Lite, Hexad, and the converted games, and every misspelling a test plants is caught.
4. **Action declarations.** Exit: the converter writes them for adv3Lite's actions, and the round trip still gives identical images.
5. **Modules.** Exit: Hexad and a game built with `import` rather than headers, with identical images.
