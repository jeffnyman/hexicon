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

- Hexad's own `get` methods (`getOutermostRoom`, `getPlayerChar`), by section 3's rule and the namespace check. Decided (2026-11-25): **they keep `get`.** Hexad defines 109 of them, used 801 times. Without `get`, 46 would take a name already in use, mostly with another meaning: `getActor` would be `actor`, used 809 times as a property and a local; `getDestination` would be `destination`; `getText`, `text`; `getKey`, `key`. Class by class, as section 3 of vocabulary.md does it, every class an author works with keeps `get` because one of its names is taken: `Thing` (11 methods, `getOutermostRoom` among them), `Room`, `Action`, the travel connectors, and the topic and conversation classes. The 33 classes that could drop it hold the library's own plumbing (`getResErr`, `getOutOfNested`, `getRemoteSmellList`, `getXML`), which authors do not call; renaming those alone would leave `outOfNested()` beside `getOutermostRoom()`, a library in two styles for no gain to authors. The `now` object covers the values authors read most.
- The banner classes as panes, to match `paneCreate`. Done (2026-11-25): `BannerWindow` is `Pane`, and `BannerUIWindow`, `BannerOutputStream`, `CustomBannerWindow` (the `customPane` extension), and `WebBannerWin` are `PaneUI`, `PaneOutputStream`, `CustomPane`, and `WebPane`; `statuslineBanner` is `statuslinePane`, and the methods follow (`showPane`, `removePane`, `writeToPane`, `paneArgs`). The sources are `pane.hxn` and `extensions/customPane.hxn`, and the system's `SysInfoBanners` is `SysInfoPanes`. The `<banner>` tags in the text a game prints are markup, and keep their name, as does `vmBanner`, which is the VM's start-up line, not a window. The VM, the host interface (`PaneSpec`, `PaneInfo`, `pane_create`), the terminal, the WebAssembly crate, and the web page (`game.panes()`, `.pane`) followed the same day.

## Writing games on Hexad

Games written for Hexicon from the start show what an author meets, which the converted exercises cannot. Each goes in the playground's `game-tests`, with a script, so that `tools/game-check.py` plays it from then on. The plan, decided 2026-11-25: Cloak of Darkness first, as a check of the authoring experience; then Ruins (2,233 lines, written for adv3), the first port from adv3, to list concretely what Hexad lacks; then Square Circle (16,620 lines, adv3), as the real probe.

### Cloak of Darkness (2026-11-25)

The standard small game, written fresh in about 150 lines: three rooms, a cloak that darkens the bar, a hook, and a message in the sawdust that blundering in the dark tramples. It uses `import hexad;`, a `verb ... for` rule (HANG), `dobj` handlers, `now`, two `Achievement`s, and a room's `roomBeforeAction` and `cannotGoThatWayInDark`. Its script plays to the losing end, restarts, and plays to the winning one. What it found:

- **A game could not have an object called `message`.** Hexad's function `message()`, which `showMsg` calls, took the name, and in this game it is the obvious one. Fixed the same day (below): the function is `showMessage`, beside `buildMessage`, and the game's object is `message`.
- **A `verb ... for` rule reports in the other action's words.** HANG CLOAK ON HOOK says "You put the cloak on the small brass hook." The rule's `verbPhrase` is used where the parser asks a question, but the report is `PutOn`'s. A game that cares gives the hook its own report.
- **Refusing to leave something belongs in `verify()`, not `check()`.** Refused in `check()`, DROP CLOAK first took the cloak off ("first taking off the cloak") and then refused. This is how Hexad works, and the guide should say so where it introduces the stages.
- **In the dark, a command that names an object fails before the room hears of it** ("You see no sawdust here"), because nothing is in scope; only commands without objects (JUMP) reach `roomBeforeAction`. That is right for Hexad, but a game whose darkness punishes every action has to know it.
- What worked without trouble: the templates, `now.actionIn(Travel, Look, Inventory)`, a double-quoted string as an exit (`north = "..."`), the room's `isLit` as an expression, `Achievement`s adding up the maximum score themselves, and `finishGameMsg` with a string.

### Exercise 23 (2026-11-25)

The last exercise of adv3Lite's manual, *An Eventful Walk* (1,531 lines), and the only game test that reaches Hexad's scenes, hint system, and menus, its kinds of event list (`StopEventList`, `SyncEventList`, `ShuffledEventList`, `RandomEventList`, `CyclicEventList`, `ExternalEventList`), `CommandRule`s, and `State`s. It had never run since it was converted: its build lost the define its menu needs (see the runtime errors, above). Its script is now 60 entries: it explores, opens the hints at the start and again once the dark cave has opened a new goal, finds the branch, lights it at the bonfire, fetches the bucket, brings down the rockfall (which starts the second scene and changes the man's state, and his complaints cycle), fills the bucket, puts out the fire, and wins with 10 of 10. All of that works on Hexad as it is. What it found:

- **A connector with no destination is now taken as no exit at all.** adv3Lite added `nilBlocksTravel`, true by default, after the manual's exercises were written; this one has three connectors that exist only to play an event list or end the game (the fallen tree to the west, the river to the north, the way back to the car), and each said "You can't go that way" until it was given `nilBlocksTravel = nil`. That is the exercise's to change, since a connector whose destination is worked out and comes to nothing should block; but nothing tells an author who meets it, and the guide should say so where it shows a connector with an event list.
- **The menus work in a plain terminal**, as numbered lists that take typed answers. A script for `game-check` gives their answers as key events (`<key>1`, `<key>[space]`, `<key>q`), since a menu reads keys, not lines.
- **Two things with the same name are offered as they are named:** "Which do you mean, (1) the cave entrance or (2) the cave entrance?", for the room outside the cave, whose name in the exercise is "the cave entrance", and the cave mouth. The exercise's naming, but the question could tell them apart.
- The exercise's own slips are left as they are: `missinqQ` for `missingQ` (which the compiler reports), and a description that ends without a space before what follows it ("full of water.In the bucket").

### The names a game cannot have (2026-11-25)

Cloak's `message` led to a survey: a game on Hexad defining an object for each of 288 common nouns of interactive fiction (`key`, `door`, `lamp`, `sign`, `rope`, `room`, `table`, and the rest), each in a build of its own. 21 were refused, because a name belongs to one thing in the whole program, its libraries included, and a property's name counts:

| What has the name | Names | |
|---|---|---|
| A Hexad function | `message` | freed: the function is now `showMessage` |
| `now`'s properties | `room`, `player`, `topic`, `fact` | kept. They were free before `now` (checked by building against Hexad as it was), and are its plainest names; a game's player is `me` by custom, and its rooms have names of their own |
| A Hexad macro | `exit` (as in `exit;`) | kept: the idiom is common. An object named `exit` expands the macro, and the message is about `throw`, which is confusing |
| Built-in methods | `match` (`String`), `car` (`List`) | kept |
| Hexad's properties for long | `actor`, `person`, `score`, `count`, `title`, `direction`, `matches` | kept: renaming them is not worth it |
| Directions | `north`, `south`, `east`, `west`, `up`, `down` | kept: they are how exits are written |

The other 267 are free. And the message now says what has the name, with a note on why: `"room" is already the name of a property, so it cannot also name an object`, where before it said "something else". Only a change to the language, in which an object's name and a property's do not share one namespace, would free the rest (vocabulary.md, section 3).

### Ruins (2026-11-25)

The first port from adv3: the sample game of the Inform Designer's Manual, from its adv3 version, rewritten on Hexad in 1,189 lines (the adv3 game is 1,578, with two files of utilities Hexad does not need). An archaeologist, five treasures to photograph where they lie and pack, a sodium lamp that burns down and is pushed from room to room, a stone ball rolled into a chasm, a priest seen only through a mask, a dictionary of glyphs, scuttling claws in the dark, and a cage that turns the player into a warthog and back. Its script, 90 commands, plays to the full score of 30.

**Hexad has most of adv3's part of it.** Lamps that run out (`extensions/fueled`), objects pushed between rooms (`canPushTravel`), stairways and connectors with barriers, `Booth`, `Platform`, `Distant`, `MultiLoc`, `Noise` and `Odor`, `Consultable` with topics matched by object, list, and pattern, the whole of the conversation the priest needs (`AskTopic`, `AltTopic`, `GiveShowTopic`, `CommandTopic`, the defaults), `EventList` and `Daemon`, score ranks, and `setPlayer`.

**What it found wrong in Hexad, and fixed:**

- **A game could not include an extension** (fixed in the change before: `include` in a manifest).
- **`explainTravelBarrier` crashed as declared.** The library declared it with two parameters and called it with one unless a flag said two, so a barrier written as declared stopped the game with "wrong number of arguments". It is now declared with one and called with as many as the game's method takes.
- **Pushing a lit lamp into a dark room described the room as dark**, because the lamp followed the player after the look. adv3's version of the game worked round the same thing with a hidden light. The pushed object now goes ahead, hidden, and comes back for the move, so its light is there when the player looks.
- **The exit list counted a refusal as an exit**: "From here you could go down or up to the forest", in the forest, after UP was refused. It now lists an exit given by a message or a method only if trying it led somewhere else. Airport and Exercise 13 had the same slip in their transcripts ("south to the terminal" in the Terminal), which are now right.
- **Three tests of the form `typeOf(x == TypeObject)`**, always true, since they take the type of a comparison. One was a real bug: after `withPresent` or `withPast`, the old tense went to `Narrator` rather than back to `gameMain`. All three are fixed. The compiler could warn of this form.

**What adv3 has and Hexad does not**, and what the port did instead:

| adv3 | Hexad, in this port |
|---|---|
| Levels of brightness: a lamp that dims, a warthog that sees in the dark, a lamp that can be found in the dark when off | `isLit` and `visibleInDark`, which were enough |
| `canPushTravelVia`: whether an object can be pushed through a given connector | refused in `beforeMovePushable`, with `exit` |
| `ObjectPreCondition`: the camera taken first, as an implicit action | `check()` refuses until the camera is held |
| `dobjFor(Default)`: one handler for any action | `Decoration` with `decorationActions` |
| Weak words (`'(glyph)'`): "glyph" alone matches no glyph | none: LOOK UP GLYPH finds the first glyph |
| Postures: the body left lying in the cage | Hexad's `postures` extension was not used; left out |
| Changing the messages of NPCs and the player (`npcMessages`, `playerMessages`): the priest's "You talk in riddles" for unknown words | left out |

**What an author meets:**

- **`steps` is a Hexad property**, so the stairway is `stoneSteps`; the message now says so plainly.
- **A runtime error stopped the game and said only what went wrong**, not where: "wrong number of arguments", "nil object reference". Both took reading the library to place. Fixed the same day:
  - **Every image records the line each statement is on** (the compiler's line records, which only `-d` wrote before), and names its source files. It costs about 14% of a game's size: 1.71 MB to 1.95 MB for a new game on Hexad. Only `-d` adds what a debugger needs as well (the symbols, the macros, and each line's address).
  - **A runtime error shows where it happened**, as the compiler's messages do: `--> start.hxn:35`, then the lines of the game's own that led there. The built-in libraries' lines are shown down to the first of the game's, five at most, and the rest counted.
  - **A function given the wrong number of arguments is named by where it is**, and what it takes: "wrong number of arguments: the function at start.hxn:51 takes 3, and was given 2".
  - **Hexad's command loop shows the error and goes on** to the next command, rather than ending the game; UNDO takes back what the command had done.
  - It found Exercise 23's old failure at once: the exercise's preinitialization fails while it is built, so it runs again when the game starts, where `globalSymbols()` is nil (`<hexicon>/hexad/misc.hxn:2780`).
  - **A failed preinitialization now fails the build** (2026-11-25). The system library caught the error, showed it, and finished as if all were well, so the build said "Built", exited with success, and wrote an image that ran the failed preinitialization again at start-up, failing there instead, as Exercise 23 did. Now the error is shown with where it happened, the build says `error: preinitialization failed: ...`, exits with failure, and writes no image.
  - **That showed Exercise 23's cause:** its menu lists `topInstructionsMenu`, which Hexad defines only when a game defines `INSTRUCTIONS_MENU`, as its original project did (`-D INSTRUCTIONS_MENU`) and its manifest had lost. With the define it plays, for the first time since it was converted; its script is still only VERSION, CREDITS, and LOOK. Its build also warns of `missinqQ`, the exercise's own slip for `missingQ`.
  - Warnings from the built-in libraries' sources are shown in a game's build, as any source's are, with the library's file and line: checked by planting a misspelling in Hexad. (One that seemed hidden had been cut off by a pipe.)
- **`goInstead` does not end the command it replaces**, so JUMP in the burial shaft went up and then "jumps on the spot, fruitlessly" as well; `exit` after it does.
- **A connector's destination has to be a room.** One leading into the cage, a `Booth`, did nothing at all, silently.
- **The actor of the command cannot be taken out of play during it.** Moving the warthog to nil while it acts stopped the game; it is hidden instead.
- SMELL in the square chamber does not find the tortilla odour of the mist, which is a `MultiLoc` there.

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
4. **Hexad's own words** (above). Each decided rename is made through Hexad, its English, its extensions, the guide, and the game tests together, and checked by the games playing the same. Done so far: the built-in renames (vocabulary.md, 2026-11-24), so Hexad uses `Map`, `Decimal`, `typeOf`, `regexMatch`, and the rest; and `CommandRule`, `showMsg`, and `msgText` (2026-11-25), with the games playing the same. And `now` (2026-11-25), above. And the banner classes as panes (2026-11-25). Hexad's own `get` methods were weighed and kept (above). Hexad's words are done.
5. **adv3's depth as modules.** The senses, nested rooms and postures, and whatever else is chosen. Each is optional and comes with a test game of its own.
