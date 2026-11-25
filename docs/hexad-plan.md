# Hexad Library Plan

Status: draft for discussion, 2026-11-23; brought up to date after the break, 2026-11-25

## Goal

Hexad is the library for Hexicon games. It takes what adv3 and adv3Lite each do best, and it is written in Hexicon's own language as that language arrives ([language-plan.md](language-plan.md)).

Hexad is Hexicon's one library. It began as a fork of adv3Lite (milestone 1), and since the break with TADS 3 (language plan, "Retiring TADS 3") it is changed freely: no TADS 3 game builds on Hexicon, so there is no compatibility to keep. Games for Hexicon are written, or rewritten, on Hexad.

## Where it came from

The two libraries Hexad was drawn from, as a reference for what to bring from each.

| | adv3 | adv3Lite |
|---|---|---|
| Version in the corpus | 3.1.3 | 2.2.3.2 |
| Size, library and English | about 98,500 lines | about 83,700 lines, plus 30 extensions |
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

**Decided: start from adv3Lite, and bring adv3's depth back as optional modules.** The reasons, as they were weighed:

- adv3Lite is already a combination, built from adv3 and Mercury. It kept adv3's way of doing actions and conversation and made the world model simpler.
- It is modular already, so adding modules extends a design that exists rather than imposing one.
- It can be used as Hexad's start: permission is given for this project.
- It has a test corpus from the first day. Hexad begins as adv3Lite, so Airport and the ten exercises in adv3Lite's manual play the same on it, and every later change is checked by their still playing the same.

The alternatives:

- **Start from adv3 and make it smaller.** adv3's sense model runs through `Thing`, `Actor`, travel, the lister, and the resolver. It cannot be made optional without rewriting all of them, which is option three with a worse starting point.
- **Write a new library in Hexicon's language.** It could be cleaner, but there would be no games to check it against for a long time. Better for later parts than for the base.

The hard part is adv3's sense model. Making sound, smell, and touch pass through containers and connectors means changing how scope is computed, and adv3Lite computes scope its own way. That module needs its own design when the time comes. Nested rooms and postures are simpler, because adv3Lite's `postures` extension is a start on them.

## Compatibility

None is owed (2026-11-24). Hexicon builds no TADS 3 game, and the converter is gone. Hexad's tests are its own: the games in the playground's `game-tests`, played by `tools/game-check.py`, which began as adv3Lite's manual exercises and now move with Hexad. A change to Hexad that a game uses updates the game in the same change, and a change meant to alter what a game shows is accepted by recording its new transcript.

## Decided (2026-11-23)

- **Hexad starts from adv3Lite.**
- **Hexad lives in this repository, in `lib/hexad`, as the built-in library.** The language and the library change together: a language change and Hexad's adoption of it go in one commit and are tested together, much as Rust ships its standard library. `lib/hexad` has its own `hexicon.toml`, so that it could move to a repository of its own later if that is ever wanted.
- **Hexad uses no TADS names.** As Hexicon names for the built-in functions arrive (language plan, change 0), Hexad moves to them.
- **Hexicon has one library (2026-11-24).** TADS had two, adv3 and adv3Lite, and authors having to choose between them (adv3Lite being not much lighter) cost TADS a great deal of momentum. So Hexad is the only library Hexicon ships or documents. adv3 is never offered as a second choice, even converted. A converted adv3 may be kept outside the repository, to build adv3 games as tests and to read when porting a feature. What Hexad takes from adv3 becomes Hexad's own, with the same classes and the same guide, switched on by an author at most, and never a second tier. Hexad's design decisions come first and say what to take.

## Hexad's words

Decided, 2026-11-25: all of the below, `now` included. [vocabulary.md](vocabulary.md) renamed what is built into Hexicon; these are the library's own. The same cost applies (vocabulary.md, section 3): a name a library defines is taken for the whole program, so a short noun is a poor name for anything Hexad defines globally, and a macro is worse, because it replaces the word everywhere, a property or local of that name included.

### The current turn's values: the `g` macros

`gActor`, `gDobj`, `gIobj`, `gAction`, `gCommand`, `gPlayerChar`, `gLocation`, `gRoom`, `gTopic`, and about 50 more (962 uses of the commonest four). Each is a macro for a property of `libGlobal` or of the current action: `gActor` is `libGlobal.curActor`, `gDobj` is `gAction.curDobj`.

They cannot become plain words. `actor` is used 1,480 times in Hexad as a property and a local, `player` 994, `action` 1,690, `dobj` 854; a macro called `actor` would rewrite all of them. The choices:

| | Looks like | Takes |
|---|---|---|
| **A. Keep the `g` names** | `gActor`, `gDobj` | nothing new; `g` is a convention to explain once ("g for the current turn's") |
| **B. One object for the current turn** | `now.actor`, `now.dobj`, `now.room` | one name: `now`, `turn`, or `current`. None is used in Hexad or the game tests outside strings, but a game could want `current` (Exercise 23 has a river with one) |
| **C. Both, for a while** | either | against "no aliases" (vocabulary.md) |

Decided: **B**, named `now`, with its properties forwarding to where the values live now. It reads as English, it is how other languages expose such state, and it takes one name instead of fifty. The cost is the 962 and more uses, mechanical, and a property call where a macro expanded inline, which the VM's lookup cache makes cheap.

Done (2026-11-25). `now` is an object in `misc.hxn`, after `libGlobal`, and the 60 macro definitions are gone from `hexad.h`:

- **Values** are properties: `now.actor`, `now.action`, `now.command`, `now.dobj`, `now.iobj`, `now.aobj` (also for `gAcc`), `now.literal`, `now.number`, `now.player`, `now.location`, `now.room`, `now.actorRoom`, `now.turns`, `now.topic`, `now.topicText`, `now.topicMatch`, `now.lastAction`, `now.lastCommand`, `now.lastTopic`, `now.lastFact`, `now.verifyDobj`, `now.verifyIobj`, `now.tentativeDobj`, `now.tentativeIobj`, `now.verifyList`, `now.actionList`, `now.actionListStr`, `now.actionListObj`, `now.verbWord`, `now.verbPhrase`, `now.commandTokens` (was `gCommandToks`), and `now.outputStream` (was `gOutStream`).
- **Macros with arguments** are methods: `now.actionIs(Take)`, `now.actionIn(Take, Drop)`, `now.travelActionIs(northDir)`, `now.tentativeDobjIn(lst)`, `now.tokensInclude(tok)`, `now.revealed(key)`, `now.reveal(key)`, `now.unreveal(key)`, `now.known(obj)`, `now.setKnown(obj)`, `now.setSeen(obj)`, `now.fact(tag)`, and `now.factDesc(tag)`. `gTravelActionIs(north)` pasted `Dir` onto its argument, which a method cannot, so `now.travelActionIs` takes the direction itself.
- **What the library assigns** it assigns where the value lives: `gAction = cmd.action` is now `libGlobal.curAction = cmd.action`, and `gDobj = obj` is `now.action.curDobj = obj`.
- **Not turn values, so not on `now`:** `gLibMessages`, `gExitLister`, `gHintManager`, and `gExtraHintManager` pointed at library objects that may be left out; code reads `libGlobal.libMessageObj` and the rest. (`exitLister` and the hint managers are already the objects' own names.) `gInformed`, `gStance`, and `gMood` called `getActor`, which is a method of the object they were used in (a topic entry, an agenda item), not a global, so they could not move to `now`; their uses are written out (`getActor.informedAbout(key)`, `getActor().stance`). `gStanceIs`, `gStanceIn`, `gMoodIs`, `gMoodIn`, `gSetStance`, and `gSetMood` did the same and pasted words, and nothing used them; they are gone, and `aStanceIs` and the other macros that name their actor remain.
- **`gMessageParams` stays a macro**, as `messageParams(obj, cont)`, because it makes the parameters' names from its arguments' names.
- **Templates take `@now.player`** (language change, 2026-11-25): after a template value's operator (`@`, `+`, `->`), a value's properties can follow it by name without parentheses, as a macro in parentheses could before. Nothing else can follow, so a list after it is a template value of its own. `tests/programs/hand/templates.hxn` has a case; Airport's `TellTopic @now.player` is another.

Every game plays the same.

### Messages: `DMsg` and `BMsg`

`DMsg(take, 'Taken. ')` shows a message by its id, and `BMsg(...)` gives it as a string; a game replaces a message by its id (802 uses). The macro gives the bare id (`take`, not `'take'`), so the form is worth keeping, under clearer names. Done (2026-11-25): `showMsg(id, text)` and `msgText(id, text)`.

### `Doer`

A rule that replaces or redirects a command (`Doer 'take Thing'`, 69 uses). `Rule` is taken by the `rules` extension's class. Done (2026-11-25): `CommandRule`, with its family (`CommandRuleCmd`, `CommandRuleParser`, `execCommandRule`, `findCommandRules`, and the rest) and its source, `commandRule.hxn`. The debugging option that shows which rule ran is `DEBUG RULES`, was `DEBUG DOERS`; a game's own debug output says "[Executing CommandRule; ...]".

### Kept

- `dobj`, `iobj`, `aobj`: the parser's words for a command's objects, and now the language's for handlers (`dobj Take { ... }`). The guide teaches them.
- `verify`, `check`, `action`, `report`: the four stages of an action, already plain.
- `Thing`, `Room`, `Door`, `Actor` and the rest of the world model.

### Later

- Hexad's own `get` methods (`getOutermostRoom`, `getPlayerChar`), by section 3's rule and the namespace check.
- The banner classes as panes, to match `paneCreate`. Done (2026-11-25): `BannerWindow` is `Pane`, and `BannerUIWindow`, `BannerOutputStream`, `CustomBannerWindow` (the `customPane` extension), and `WebBannerWin` are `PaneUI`, `PaneOutputStream`, `CustomPane`, and `WebPane`; `statuslineBanner` is `statuslinePane`, and the methods follow (`showPane`, `removePane`, `writeToPane`, `paneArgs`). The sources are `pane.hxn` and `extensions/customPane.hxn`, and the system's `SysInfoBanners` is `SysInfoPanes`. The `<banner>` tags in the text a game prints are markup, and keep their name, as does `vmBanner`, which is the VM's start-up line, not a window.

## Open questions

1. Which of adv3's features matter most to bring back? The senses, nested rooms and postures, and report grouping are the likely ones.
2. Hexad's words: decided and made, above, except the later ones.

Settled since the draft: compatibility with adv3Lite games (none is owed, above), and whether the English messages are a module of their own (they are, as `lib/hexad/english` with its own manifest).

## Milestones

1. **The fork.** Hexad is adv3Lite 2.2.3.2 under its new name, built from a `hexicon.toml`. Exit: Airport and the manual's exercises replay identically on it. Done (2026-11-23):
   - **What is in `lib/hexad`:** adv3Lite's sources, `advlite.h`, its English, and its extensions, byte for byte. Its `adv3Lite.tl` and English `.tl` are now `hexicon.toml` manifests named Hexad and Hexad English, with the sources in the same order.
   - **What is left out:** the manual, the templates, and the cut-down variants (`adv3Liter`, `adv3Litest`). The web variant's `browser.t` and `menuweb.t` are kept, but are not in the manifest yet.
   - **The check (`tools/compile-tests/hexad-compare.sh`):** Airport and the ten exercises of the manual, each built on adv3Lite and on Hexad and preinitialized, give identical images, which is stronger than replaying them. Exercise 23 warns of an undefined `topInstructionsMenu` and stops with a run-time error while it is preinitialized; it does the same on adv3Lite.
   - **What still said adv3Lite:** the code, so the fork is a pure copy in the history. The library's `ModuleID` named it "adv3Lite Library" until milestone 3's first step.
2. **In Hexicon's language.** Hexad's sources become `.hxn` through the converter (language milestone 2, done 2026-11-23). Exit: images identical to milestone 1's without preinitialization, and the same play with it. Done (2026-11-23):
   - **The conversion:** `hexicon convert` made Hexad's 80 sources and headers `.hxn` files, with 28 names renamed. It now also updates a manifest that is already there: a `.t` source the conversion makes a `.hxn` file is named without its extension.
   - **The images differ when preinitialized.** Ten of Hexad's files now include `hexicon.h`, so a game on Hexad has 13 more function names and 11 more macros than one on adv3Lite. adv3Lite looks through those tables during preinitialization, so the objects it makes then are numbered differently.
   - **So the check was byte for byte both ways.** Without preinitialization, each game on Hexad was compared with the same game on adv3Lite as it is. With preinitialization, it was compared with adv3Lite plus `#include <hexicon.h>` on the same lines of the same files as Hexad: the same names and macros, module for module, differing only in how the calls are spelled. Airport and all ten exercises gave identical images both ways, 22 of 22.
   - **Corrected later:** this milestone's report said Exercise 15 played differently on Hexad because of that numbering. It does not. The exercise's `detonator` calls `randomize()` when the game starts, which made it play differently on every run, even of one image, until `--norand` covered it (milestone 3). Played repeatably, Exercise 15 is the same on Hexad, on adv3Lite with `hexicon.h`, and on adv3Lite as it is.
3. **The language's features adopted.** Checked names, action declarations, and modules come in as the language gains them. Each step is checked by the same games still playing the same. Done, as far as the language has gone. The tools this milestone names, `hexad-compare.sh`, `tools/explore.py`, and `hexicon convert`, were removed in the break; Hexad is checked by `tools/game-check.py` since. The steps:
   - **Hexad names itself** (2026-11-23). Its `ModuleID` is now `moduleHexad`, named "Hexad", version 0.1.0 (Hexicon's). The VM's line in VERSION drops "T3": "VM (hexicon) version 0.1.0". This is Hexad's first break with adv3Lite: a game that refers to `moduleAdv3Lite` must now say `moduleHexad`.
   - **The check is now play, with the differences recorded** (`tools/compile-tests/hexad-compare.sh`). Hexad no longer matches adv3Lite byte for byte, and will match it less as it changes. So each game is built on both, preinitialized, and played from a command script in `tools/compile-tests/hexad/`: VERSION and CREDITS, then 230 to 420 commands chosen by `tools/explore.py` on adv3Lite. The transcripts must differ exactly as `NAME.diff` there records; `--accept` records the differences when a change to Hexad is meant to make them. Today each game differs only in VERSION's two lines. Exercise 23 stops with a run-time error before its first prompt, on both, so only its build is compared.
   - **Hexicon's own names** (2026-11-24). Hexad's header is `hexad.h`, it is built on Hexicon's system library (`hexicon.h`, `ProgramObject`, `FileTypeSave`), and its own `adv3LibInit`, `adv3LibPreinit`, and the settings keys `adv3.exits` and `adv3.notify` are `hexadInit`, `hexadPreinit`, `hexad.exits`, and `hexad.notify`. Its files' titles say Hexad. `hexad-compare.sh` converts adv3Lite's games with `hexicon convert` to build them on it, and they play as recorded.
   - **Its comments in its own words** (2026-11-24). Hexad's and the system library's comments no longer speak of TADS, adv3, adv3Lite, or Mercury, nor credit anyone, and name the files as they now are (`events.hxn`, not `events.t`). One message changed with them, the system library's about a circular module dependency, which named `_main.t`. Games play the same.
   - **Built into Hexicon** (2026-11-24), as the system library is, so a game names `hexad` among its libraries and needs no path to it. A Hexad a build is told where to find comes first. `hexicon new` starts a game on it. The WebAssembly player, which builds no games, leaves it out.
   - **Credits removed** (2026-11-24), with permission. Hexad's sources, README, and manifest no longer carry copyright or attribution lines, its module IDs no bylines, and adv3Lite's license file is gone: Hexad is to be changed freely from here. Only comments and the unshown bylines changed, so games play the same.
   - **Two of adv3Lite's slips fixed** (2026-11-24), found by checked names (language milestone 3) run over Hexad's sources. A `ListGroup`'s `groupSuffix`, which game code is to define, was never shown, because the library displayed `groupSufffix`. And an action's `optionsExcplained = nil` set nothing: it now sets `optionsExplained`, to the same nil it had. Neither changes what Airport or the exercises show.
   - **Actions and handlers are declarations** (2026-11-24, language milestone 4). `hexicon convert` rewrote Hexad's 548 uses of adv3Lite's macros as `action Take(dobj): TAction`, `dobj Take { ... }`, and `dobj Search as LookIn`. Games built on it give identical images, debug builds included. `advlite.h` still defines the macros, for games not yet converted and for the forms with no declaration of their own (`asDobjWithoutVerifyFor`, `MMTIAction`).
   - **Verb rules are declarations** (2026-11-24, language milestone 6). `hexicon convert` rewrote Hexad's 288 `VerbRule`s as `verb Take ('take' | 'get') multiDobj: ...`, with `for` where a rule's name is not its action's (`verb SetDownWhat for Drop 'set' 'down':`). Games built on it give identical images. `hexad.h` still defines `VerbRule`, for games not yet converted and for its shortcut macros (`DefineTVerb` and the rest).
   - **Hexad is imported** (2026-11-24, language milestone 5). Its manifest names `advlite.h` as the header a Hexicon source gets by `import hexad;`, and its own sources import it rather than including it. The images are the same.
   - **`hexicon run --norand` makes play repeatable** even for a game that reseeds at random. It used to leave only the start-up seed out, as the reference's `-norand` does, so a `randomize()` in the program still drew on the system's entropy. Now every seed is the same.
4. **Hexad's own words** (above). Each decided rename is made through Hexad, its English, its extensions, the guide, and the game tests together, and checked by the games playing the same. Done so far: the built-in renames (vocabulary.md, 2026-11-24), so Hexad uses `Map`, `Decimal`, `typeOf`, `regexMatch`, and the rest; and `CommandRule`, `showMsg`, and `msgText` (2026-11-25), with the games playing the same. And `now` (2026-11-25), above. And the banner classes as panes (2026-11-25). Next: Hexad's own `get` methods.
5. **adv3's depth as modules.** The senses, nested rooms and postures, and whatever else is chosen. Each is optional and comes with a test game of its own.
