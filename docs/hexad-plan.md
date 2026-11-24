# Hexad Library Plan

Status: draft for discussion, 2026-11-23

## Goal

Hexad is the library for Hexicon games. It takes what adv3 and adv3Lite each do best, and it is written in Hexicon's own language as that language arrives ([language-plan.md](language-plan.md)).

Existing games are unaffected. They keep building against adv3 and adv3Lite, which Hexicon goes on compiling exactly as the reference does. Hexad is for new games and for authors who choose to move.

## The two libraries

| | adv3 | adv3Lite |
|---|---|---|
| Author | Michael J. Roberts | Eric Eve, built from adv3 and Mercury |
| Version in the corpus | 3.1.3 | 2.2.3.2 |
| Size, library and English | about 98,500 lines | about 83,700 lines, plus 30 extensions |
| License | The permission given for this project | MIT. Story files are free of it, and the notice is kept in copies |
| How it is included | All of it | 41 core modules, any of which can be left out, and cut-down sets (`adv3Liter`, `adv3Litest`) |

What each does better:

| Area | adv3 | adv3Lite |
|---|---|---|
| Senses | A full model: sight, sound, smell, and touch through containers, materials, and connectors, with distance and light for each sense | Sight and light. `senseRegion` lets one room see or hear another, and the `sensory` extension makes sounds and smells into objects |
| Containment | Nesting of any depth, nested rooms, and postures | One object can be in, on, under, and behind at once (`contType`, `remapIn` and the rest). Postures are an extension |
| Doing an action | verify, check, action, and report, with ranked verify results | The same, plus `Doer`s, which replace or redirect a command by rules, and the `rules` extension |
| Conversation | `ActorState`, `ConvNode`, `TopicEntry`, and `AgendaItem` | Those, carried over and extended, plus facts, beliefs, and thoughts |
| Story structure | Events and daemons | Those, plus `Scene`s and `Region`s, and the `relations` extension |
| Messages | Message objects (`playerActionMessages`, `libMessages`) | A message for each id (`DMsg`, `BMsg`), which a game overrides by id (`CustomMessages`). This suits translation |

In short, adv3 simulates more deeply, and adv3Lite is easier to write for and more modular. A library that combines them wants adv3Lite's shape, with adv3's depth available where a game asks for it.

## Approach

**Recommended: start from adv3Lite, and bring adv3's depth back as optional modules.**

- adv3Lite is already a combination: Eric Eve built it from adv3 and Mercury. He kept adv3's way of doing actions and conversation and made the world model simpler.
- It is modular already, so adding modules extends a design that exists rather than imposing one.
- Its license allows it.
- It has a test corpus from the first day. Hexad begins as adv3Lite, so Airport and the ten exercises in adv3Lite's manual play the same on it, and every later change is checked by their still playing the same.

The alternatives:

- **Start from adv3 and make it smaller.** adv3's sense model runs through `Thing`, `Actor`, travel, the lister, and the resolver. It cannot be made optional without rewriting all of them, which is option three with a worse starting point.
- **Write a new library in Hexicon's language.** It could be cleaner, but there would be no games to check it against for a long time. Better for later parts than for the base.

The hard part is adv3's sense model. Making sound, smell, and touch pass through containers and connectors means changing how scope is computed, and adv3Lite computes scope its own way. That module needs its own design when the time comes. Nested rooms and postures are simpler, because adv3Lite's `postures` extension is a start on them.

## Compatibility

- **adv3Lite games: yes, at first.** Hexad starts out source compatible with adv3Lite. It breaks compatibility only where a change is worth it, lists each break, and has the converter handle what it can.
- **adv3 games: no.** adv3 is too different. Those games keep adv3.

## Decided (2026-11-23)

- **Hexad starts from adv3Lite.**
- **Hexad lives in this repository, in `lib/hexad`, as the built-in library.** The language and the library change together: a language change and Hexad's adoption of it go in one commit and are tested together, much as Rust ships its standard library. `lib/hexad` has its own `hexicon.toml` and its own license notice (adv3Lite's MIT notice, which is kept), so that it could move to a repository of its own later if that is ever wanted.
- **Hexad uses no TADS names.** As Hexicon names for the built-in functions arrive (language plan, change 0), Hexad moves to them.

## Open questions

1. How long should Hexad stay compatible with adv3Lite games? With TADS 3 to be retired, this matters most while authors are moving games over.
2. Which of adv3's features matter most to bring back? The senses, nested rooms and postures, and report grouping are the likely ones.
3. Should the English messages be a separate module from the start, as adv3Lite's `english` is?

## Milestones

1. **The fork.** Hexad is adv3Lite 2.2.3.2 under its new name, built from a `hexicon.toml`. Exit: Airport and the manual's exercises replay identically on it. Done (2026-11-23):
   - **What is in `lib/hexad`:** adv3Lite's sources, `advlite.h`, its English, and its extensions, byte for byte, with its license. Its `adv3Lite.tl` and English `.tl` are now `hexicon.toml` manifests named Hexad and Hexad English, with the sources in the same order.
   - **What is left out:** the manual, the templates, and the cut-down variants (`adv3Liter`, `adv3Litest`). The web variant's `browser.t` and `menuweb.t` are kept, but are not in the manifest yet.
   - **The check (`tools/compile-tests/hexad-compare.sh`):** Airport and the ten exercises of the manual, each built on adv3Lite and on Hexad and preinitialized, give identical images, which is stronger than replaying them. Exercise 23 warns of an undefined `topInstructionsMenu` and stops with a run-time error while it is preinitialized; it does the same on adv3Lite.
   - **What still said adv3Lite:** the code, so the fork is a pure copy in the history. The library's `ModuleID` named it "adv3Lite Library" until milestone 3's first step.
2. **In Hexicon's language.** Hexad's sources become `.hxn` through the converter (language milestone 2, done 2026-11-23). Exit: images identical to milestone 1's without preinitialization, and the same play with it. Done (2026-11-23):
   - **The conversion:** `hexicon convert` made Hexad's 80 sources and headers `.hxn` files, with 28 names renamed. It now also updates a manifest that is already there: a `.t` source the conversion makes a `.hxn` file is named without its extension.
   - **The images differ when preinitialized.** Ten of Hexad's files now include `hexicon.h`, so a game on Hexad has 13 more function names and 11 more macros than one on adv3Lite. adv3Lite looks through those tables during preinitialization, so the objects it makes then are numbered differently.
   - **So the check was byte for byte both ways.** Without preinitialization, each game on Hexad was compared with the same game on adv3Lite as it is. With preinitialization, it was compared with adv3Lite plus `#include <hexicon.h>` on the same lines of the same files as Hexad: the same names and macros, module for module, differing only in how the calls are spelled. Airport and all ten exercises gave identical images both ways, 22 of 22.
   - **Corrected later:** this milestone's report said Exercise 15 played differently on Hexad because of that numbering. It does not. The exercise's `detonator` calls `randomize()` when the game starts, which made it play differently on every run, even of one image, until `--norand` covered it (milestone 3). Played repeatably, Exercise 15 is the same on Hexad, on adv3Lite with `hexicon.h`, and on adv3Lite as it is.
3. **The language's features adopted.** Checked names, action declarations, and modules come in as the language gains them. Each step is checked by the same games still playing the same. In progress:
   - **Hexad names itself** (2026-11-23). Its `ModuleID` is now `moduleHexad`, named "Hexad", version 0.1.0 (Hexicon's), "based on adv3Lite by Eric Eve". The VM's line in VERSION drops "T3": "VM (hexicon) version 0.1.0". This is Hexad's first break with adv3Lite: a game that refers to `moduleAdv3Lite` must now say `moduleHexad`.
   - **The check is now play, with the differences recorded** (`tools/compile-tests/hexad-compare.sh`). Hexad no longer matches adv3Lite byte for byte, and will match it less as it changes. So each game is built on both, preinitialized, and played from a command script in `tools/compile-tests/hexad/`: VERSION and CREDITS, then 230 to 420 commands chosen by `tools/explore.py` on adv3Lite. The transcripts must differ exactly as `NAME.diff` there records; `--accept` records the differences when a change to Hexad is meant to make them. Today each game differs only in VERSION's two lines. Exercise 23 stops with a run-time error before its first prompt, on both, so only its build is compared.
   - **Two of adv3Lite's slips fixed** (2026-11-24), found by checked names (language milestone 3) run over Hexad's sources. A `ListGroup`'s `groupSuffix`, which game code is to define, was never shown, because the library displayed `groupSufffix`. And an action's `optionsExcplained = nil` set nothing: it now sets `optionsExplained`, to the same nil it had. Neither changes what Airport or the exercises show.
   - **Actions and handlers are declarations** (2026-11-24, language milestone 4). `hexicon convert` rewrote Hexad's 548 uses of adv3Lite's macros as `action Take(dobj): TAction`, `dobj Take { ... }`, and `dobj Search as LookIn`. Games built on it give identical images, debug builds included. `advlite.h` still defines the macros, for games not yet converted and for the forms with no declaration of their own (`asDobjWithoutVerifyFor`, `MMTIAction`).
   - **Hexad is imported** (2026-11-24, language milestone 5). Its manifest names `advlite.h` as the header a Hexicon source gets by `import hexad;`, and its own sources import it rather than including it. The images are the same.
   - **`hexicon run --norand` makes play repeatable** even for a game that reseeds at random. It used to leave only the start-up seed out, as the reference's `-norand` does, so a `randomize()` in the program still drew on the system's entropy. Now every seed is the same.
4. **adv3's depth as modules.** The senses, nested rooms and postures, and whatever else is chosen. Each is optional and comes with a test game of its own.
