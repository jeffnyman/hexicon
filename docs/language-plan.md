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
2. **Hexicon checks itself.** The converter's round trip is done. Hexad, the converted games, and Hexicon's own conformance programs have recorded transcripts that serve as Hexicon's own regression tests. Hexicon's language and library use no TADS names. TADS 3 is now a compatibility mode kept apart from the rest, in its own crate or module, so that nothing in Hexicon's own code carries its names or its quirks. Begun (2026-11-24): Hexicon ships its own system library and headers (`lib/system`, built into the compiler), so a game builds with nothing from TADS 3 installed. They are TADS 3's, by permission, and are to be renamed.
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
2. **A converter rewrites TADS 3 source as Hexicon source** (`hexicon convert`). It is checked by a round trip: adv3, adv3Lite, Airport, and Ditch Day are converted and compiled, and each image must be identical to the original's when the program is not preinitialized. With preinitialization, each game must play the same. The same tool is how authors move their games, so the check is also what lets them trust it.
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
# or, where the order of libraries and sources matters, one list:
# modules = [{ library = "system" }, "reflect.t", { library = "adv3/adv3" }, "main"]
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
system-main = true                       # false is -nodef
```

A library's manifest says what a `.tl` says:

```toml
[library]
name = "adv3Lite Library"
sources = ["action.t", "actions.t"]      # ...
libraries = ["$(LANGUAGE)"]              # $(NAME) is a macro's value, as in a .tl
# or modules = [...], as for a game, where the order matters
resources = ["webuires"]
system-main = true                       # false is nodef
header = "advlite.h"                     # what `import adv3Lite;` gives a file

[needs]                                  # needmacro
LANGUAGE = "this selects the language the library uses"
```

- **A source named without an extension is a `.hxn` file.** A TADS 3 source in a manifest is named with its `.t`.
- **A module is named without its extension**, as in a `.tl` or `.t3m`. Programs can see the names, so this keeps images the same.
- **A library is a folder with a manifest, a `.toml` manifest, or a `.tl` file, named without its extension.** A library named `adv3Lite` is `adv3Lite/hexicon.toml`, `adv3Lite.toml`, or `adv3Lite.tl`, looked for in that order where its name says and then along the library paths. A manifest can sit beside other libraries this way, as adv3Lite's several `.tl` files do in one folder. Any kind can be built on any other.
- **The order of sources and libraries is kept.** With `sources` and `libraries`, a game's libraries come before its sources and a library's sources before its libraries, as is usual. A `modules` list gives any other order, which a project such as Ditch Day's needs, since the order shows in the image.
- **A name the manifest does not know is an error**, so a misspelled key is not silently ignored.
- **`header` is Hexicon's own** (milestone 5): the file a Hexicon source gets when it imports the library, relative to the library's folder. A `.tl` has none.

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

- **`hexicon.h` is part of the compiler**, with the rest of the system library (`lib/system`, built in since 2026-11-24). An `#include` that finds nothing on the include path looks among the compiler's own headers last, so a file of the same name on the include path comes first. Messages place it in `<hexicon>/system/hexicon.h`. Because it is built in, it works wherever the compiler does, in WebAssembly too.
- **A function set can be declared twice in one program, under different names.** The reference accepts it, and gives the image one entry for the set; the second declaration only adds names. Declaring the same name again in the same place is accepted too. So `hexicon.h` and `tads.h` can be included together, which Hexad needs while its sources still include `advlite.h`.
- **`hexicon.h` includes the reference's `tadsgen.h`, `tadsio.h`, and `systype.h`**, whose names are already neutral, rather than copies of them. So `tadsSay` is still declared beside `print`, until Hexicon has its own copies of the system headers.
- **An intrinsic class cannot be renamed this way.** The reference says "intrinsic class has been previously defined as \"TadsObject\"" when a second name is given to one. `TadsObject` keeps its name for now. Authors rarely write it, since `object` and `class` stand for it. `T3StackInfo` is a class in the system library's `_main.t`, not in a header, so renaming it waits for Hexicon's own system library.
- **The check is a lowering check.** The same program written with each set of names gives an identical image (a unit test, and a test program that calls most of the renamed functions). With preinitialization, one object's number differs, because the build gives preinitialization the program's symbols and macros, and the two programs define different names.

### 1. Names the compiler checks

TADS 3 creates a property the first time any source names it, so a misspelling is silent. If a game says `isTakable` where the library means `isTakeable`, the object simply never gets the property and the test is `nil`. `dobjFor(Takee)` is the same mistake at the scale of a whole handler: it defines `verifyDobjTakee` and the rest, and nothing ever calls them. The reference never warns about either.

Once a program is linked, the compiler knows every property that anything defines, assigns, or declares. A property that is only ever read is almost always a misspelling. Hexicon files get a warning for it, with the nearest defined name as a suggestion. A program that builds property names at run time can declare them. The check changes no code, so there is nothing to lower.

As built (milestone 3), the check is the other way round. A misspelled *read* is already reported, as an undefined symbol, because a name that nothing defines is not a property at all. What is silent is a misspelled *definition*: `isTakable = nil` defines a property that nothing reads, and the library's `isTakeable` keeps its inherited value. So the compiler reports a value given in a game's own Hexicon source to a property that:

- nothing in the program reads, by name or by address (`&name`, which is how libraries ask whether a hook is defined);
- no library gives a value, and no source declares (`property name;`);
- is one slip from a property the program does read: a letter left out, added, or changed, two swapped, or a difference of case. Names that differ only in a final `_` or a final digit are a family, not a slip.

```
plane.hxn(725): warning: property "missinqQ" is given a value here, but nothing reads it - did you mean "missingQ"?
start.hxn(536): warning: handler "dobj ClimpUp" is never called - did you mean "dobj ClimbUp"?
```

Libraries' sources are not checked: a library defines properties for games to read, and which it reads depends on what a game includes. TADS 3 sources are not checked either.

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

As built (milestone 4, 2026-11-24), the syntax is:

```
action Take(dobj): TAction;
action PutIn(dobj, iobj): TIAction;
action Jump: IAction
    execAction(cmd) { ... }
;

Thing
    dobj Take {
        preCond = [touchObj]
        verify() { if (!isTakeable) illogical(cannotTakeMsg); }
        action() { actionMoveInto(gActor); }
    }
    dobj Search as LookIn
    iobj PutIn { ... }
;
```

- **An action** names the objects it takes (`dobj`, `iobj`, `aobj`, in that order) and its classes, and then has the properties of any object definition. It lowers to what adv3Lite's `DefineTIActionSub(PutIn, TIAction)` makes: the object, `baseActionClass = PutIn`, and for each stage and each object the property that names its handler (`verDobjProp = &verifyDobjPutIn`, `verIobjProp = &verifyIobjPutIn`, `remapDobjProp = ...`, in the macros' order). The objects are written out rather than worked out from the class, so the compiler needs to know nothing of Hexad's classes.
- **A handler** is the role and the action, then a block of the handler's methods and properties, which lowers to `propertyset '*DobjTake' { ... }`. `dobj Search as LookIn` handles one action as another of the same role, and lowers to the six methods `asDobjFor(LookIn)` makes. Anything that expands to a block can follow `dobj Search` instead, so adv3Lite's other redirecting macros (`asIobjFor`, `asDobjWithoutVerifyFor`) still work. A handler, like any property, ends without a semicolon.
- **The words are not reserved.** `action`, `dobj`, `iobj`, and `aobj` start these forms only when a name follows them, where nothing in TADS 3 could, so a property called `action` or a local called `dobj` is still a name.
- **The lowering is to tokens.** The parser reads the tokens the macros would have made, each given the place of the form, so messages and the debugger's line records are the same as the macros give.
- **The lowering is adv3Lite's, and so Hexad's** (open question 2). A Hexicon file on adv3 can use the handler form, but adv3's own macros also check the action's name in a debug build, which this does not.

### 3. Modules instead of headers

Every file of a TADS 3 game starts with `#include <adv3.h>` or `<advlite.h>`, and every name in the game and its library shares one namespace. Hexicon files would say `import hexad;` instead. That brings in the library's declarations (templates, actions, and the macros that remain) without pasting text, and a name that clashes can be qualified. Names in the image stay global, made unique where they need to be, so modules lower without a VM change. The preprocessor stays for conditional compilation.

As built (milestone 5, 2026-11-24), the first step of this, decided as the one to take now:

```toml
# lib/hexad/hexicon.toml
[library]
name = "Hexad"
header = "advlite.h"
```

```
// start.hxn
import hexad;
```

- **A library names its interface.** `header` in its manifest is the file a source gets by importing it. `import hexad;` is the library a build names `hexad` (its folder, or its `.toml` file's name), and gives the file that header, read exactly as `#include` reads it. So the program, and its image, are the same.
- **Only what is in the build can be imported.** `import hexad;` with no Hexad in the build says `import hexad: no library "hexad" in this build`. A library with no `header` (any `.tl`) says it has none to import.
- **An import is a line of its own** (`import name;`) in a Hexicon source, read by the preprocessor as a directive is, and skipped in a block `#if` leaves out. Importing a library twice in one file is importing it once.
- **What it buys now** is that games no longer name a library's files: what sits behind `import` is the library's to change. **What it leaves for later:**
  - **A compiled interface:** the header read once per build and its macros, templates, and declarations reused by each importing file, rather than read again by every one (Airport's build reads `advlite.h` once for each of its 50 or so modules). The language and the images stay as they are.
  - **Namespaces:** qualified names and clashes made unique. Large, and at odds with libraries that find objects and macros by their global names while a program runs, so they are listed here only so as not to close them off.
  - **The converter** does not yet write `import`: a converted adv3Lite game is on the converted adv3Lite, whose manifest, made from a `.tl`, names no header. Games on Hexad write `import hexad;`.

### 4. Messages written for the reader

Hexicon files get messages with the line and column and the line of source itself, and they come from declarations rather than from inside a macro expansion. TADS 3 files keep the reference's messages, which the comparison checks.

### Later, and only if wanted

- Optional type annotations (`isTakeable: bool`), checked by the compiler and erased in the image.
- Pattern matching over lists and objects.

Both are large and neither is needed for Hexad. They are listed so that the early changes do not close them off.

## Open questions

1. How far should the language go? This plan is conservative: TADS 3 plus checks and declarations. A bolder language is possible, at the cost of the converter round trip and of authors reading it on sight. With TADS 3 to be retired, reading it on sight matters less over time than it does now.
2. Should handler syntax serve both libraries or only Hexad? With TADS 3 to be retired, Hexad alone is the likely answer. Decided (2026-11-24): the declarations lower as adv3Lite's macros do, which is Hexad's lowering.

## Milestones

1. **The dialect switch and the manifest.** `.hxn` files compile as `.t` files do, without the parity-only behavior, and `hexicon.toml` builds a project. Exit: Airport's sources, renamed and built from a manifest, give an image identical to the one built from `airport.t3m`. Done (2026-11-23). `tools/compile-tests/manifest-compare.sh` builds Airport as `.hxn` sources from a `hexicon.toml`, with adv3Lite and its English library described by manifests written from their `.tl` files. The image is identical to the one built from `airport.t3m`.
2. **The converter.** Exit: adv3, adv3Lite, Airport, and Ditch Day each convert, compile, and give images identical to the originals'. Done (2026-11-23), with one qualification about preinitialization (below). `hexicon convert <folder> <new-folder>` converts a game or a library into a new folder:
   - **Sources:** `.t` files become `.hxn` files and headers stay `.h`. Each has Hexicon's names for the built-in functions in its code and comments, but never in the text of a string, and gets `#include <hexicon.h>` where its code uses one. The scanner that finds names knows comments, the three kinds of quotes, regular expressions (`R'...'`), expressions embedded in strings, and the directives whose text is not code (`#include`, `#error`, `#charset`, `#line`).
   - **Names already in use:** a rename is not made if its new name is already used in the code being converted. The converter says so.
   - **Project and library files:** a `.t3m` becomes `hexicon.toml`, or `<name>.toml` if its folder has more than one. A `.tl` becomes `<name>.toml` beside it. A source the folder holds is named without its extension, and one from elsewhere (Ditch Day's `reflect`, from the system library) with its `.t`. `-FL` and `-FI` name where one computer keeps the system library, so they are left out with a note.
   - **Everything else** is copied.

   `tools/compile-tests/convert-compare.sh` checks the round trip for adv3, adv3Lite, Airport, and Ditch Day. Without preinitialization, the images are identical. With it, the objects the program makes while preinitializing are numbered differently. The converted program's tables of symbols and macros have the 13 names and 11 macros `hexicon.h` declares and nothing else new, and adv3Lite and Ditch Day's reflection keep those tables in the image. So the preinitialized builds are compared by play instead. Ditch Day's whole walkthrough (7,104 lines) and a short Airport script give the same transcripts. The compilers' messages are the same too: none.
3. **Checked names.** Exit: no false warnings over adv3Lite, Hexad, and the converted games, and every misspelling a test plants is caught. Done (2026-11-24), as described under change 1 (`crates/hexicon-compiler/src/names.rs`):
   - **Over 15 games converted to Hexicon** (Airport, Ditch Day, the ten exercises of adv3Lite's manual, the adv3 sample game, and Heidi from *Getting Started in TADS 3*), on adv3, adv3Lite, and Hexad, it gives no false warnings. Every warning it gave was a real bug:
     - Airport and Exercises 11 and 23: `missinqQ` where the library reads `missingQ`, so the question written for it is not the one used.
     - Exercise 19: `travelBarrier = [alarmBarrier]` where the library reads `travelBarriers`, so the connector has no barrier.
     - Exercise 21: `dobjFor(ClimpUp) asDobjFor(Board)`, which redirects an action that does not exist rather than CLIMB UP.
     - Ditch Day: `openWhenRevaled` where the library reads `openWhenRevealed`, in a hint.
     - Heidi: `is asExit(south)` where `in asExit(south)` was meant, so the room has no IN exit.
   - **Planted misspellings are caught**, a property's and a handler's, in the unit tests.
   - **Run over Hexad's own sources** while it was being built, it found two of adv3Lite's own slips, now fixed in Hexad (see the Hexad plan).
   - **The comparisons that build Hexicon sources** (`convert-compare.sh`, `manifest-compare.sh`) show these warnings and leave them out of what they compare. The converted Ditch Day treats warnings as errors, so its comparison turns that off.
4. **Action declarations.** Exit: the converter writes them for adv3Lite's actions, and the round trip still gives identical images. Done (2026-11-24), as described under change 2:
   - **The converter writes them** in any file that includes `advlite.h`: each `DefineTAction(Take)` and the other `Define...Action` macros as an `action`, each `dobjFor(Take)` as `dobj Take`, and `dobjFor(Search) asDobjFor(LookIn)` as `dobj Search as LookIn`. Macro definitions, strings, comments, and files on adv3 are left alone. It now also takes Hexicon sources, which brings them up to date, and it no longer notes a rename it could not make when the program uses only the new name.
   - **The round trip** (`convert-compare.sh`): adv3Lite, with its extensions and English, and Airport converted with the declarations give images identical to the originals'. Ditch Day and adv3 are not touched.
   - **Hexad adopted them** (548 declarations in 25 files). Airport and the ten exercises built on it give images identical to those built on the Hexad before, preinitialized and not, and in debug builds, line records included, but for the build time and the folder's name.
   - **Checked names speak the new form:** `handler "dobj ClimpUp" is never called - did you mean "dobj ClimbUp"?`
5. **Modules.** Exit: Hexad and a game built with `import` rather than headers, with identical images. Done (2026-11-24), as described under change 3:
   - **Hexad imports itself:** its manifest names `advlite.h` as its header, and its 77 sources say `import hexad;` where they included it. Airport and the ten exercises built on it give images identical to those on the Hexad before, preinitialized, and in debug builds but for the build time and the folder's name.
   - **A game imports Hexad:** `hexad-compare.sh` now also builds Airport from `.hxn` sources that say `import hexad;`, from a manifest, and checks that its image is the one built from Airport's TADS 3 sources on Hexad.
   - **Tests** cover an import against an include, an import twice, one inside `#if 0`, a library not in the build or without a header, a TADS 3 source, and a manifest's `header`.
