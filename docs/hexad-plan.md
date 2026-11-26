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

- **A connector with no destination is now taken as no exit at all.** adv3Lite added `nilBlocksTravel`, true by default, after the manual's exercises were written; this one has three connectors that exist only to play an event list or end the game (the fallen tree to the west, the river to the north, the way back to the car), and each said "You can't go that way" until it was given `nilBlocksTravel = nil`. That is the exercise's to change, since a connector whose destination is worked out and comes to nothing should block; but nothing tells an author who meets it, and the guide should say so where it shows a connector with an event list. Done (2026-11-25): "Things and Rooms" shows one, the attic's window, with `nilBlocksTravel = nil` and why.
- **The menus work in a plain terminal**, as numbered lists that take typed answers. A script for `game-check` gives their answers as key events (`<key>1`, `<key>[space]`, `<key>q`), since a menu reads keys, not lines.
- **Two things with the same name are offered as they are named:** "Which do you mean, (1) the cave entrance or (2) the cave entrance?", for the room outside the cave, whose name in the exercise is "the cave entrance", and the cave mouth. The exercise's naming, but a room and a thing in it should not tie. Done (2026-11-25): a `Room` starts with a `vocabLikelihood` of -10, so the parser picks the thing. Three other transcripts had the same question and now pick the thing too: Airport's ticket counter and security guard (not the ticket area or the security gate), Exercise 11's trunk (not the top of the tree), and Exercise 19's front door (not the front drive).
- The exercise's own slips are left as they are: `missinqQ` for `missingQ` (which the compiler reports), and a description that ends without a space before what follows it ("full of water.In the bucket").

### Square Circle (begun 2026-11-25)

Eric Eve's 2004 game, written for adv3: 16,620 lines in eleven files, the prison and the fields (7,520), five characters (about 5,000), a file of verbs of its own (2,475), the hints, the books, and `DupDobj.t`. It is the third rung, and the real probe of what Hexad lacks. **Its licence allows changing the source for one's own use but not distributing a changed version**, so the port lives in the private playground (`game-tests/squarecircle`) and never in this repository; only what it finds is recorded here.

It uses 148 of adv3's classes. Hexad has 96 of them by name (seven in extensions: `roomparts`, `postures`, `sensory`, `footnote`). Of the other 52, most do the same thing another way, as Ruins found (`Readable`, `Openable`, `LockableWithKey`, `DarkRoom`, `TravelPushable`, the connectors, `Person`, the conversation states and suggested topics). The rest are the gaps to weigh: senses across rooms (`DistanceConnector`, `Occluder`), `MultiInstance`, `PresentLater`, a character who goes with the player (`AccompanyingState`), containers of several parts (`ComplexContainer`, `ComplexComponent`), `PushTravelBarrier`, `RestrictedContainer`, `StretchyContainer`, `AutoClosingDoor`, `PermanentAttachment`, and `FakeConnector`. The code is mostly mechanical now: 300 `dobjFor` handlers, 56 `VerbRule`s, and about 200 uses of the `g` macros.

**The standard is the original.** The playground keeps the image Eric Eve released (`games/SquareCircle.t3`, version 1.2) and the `hexicon` built before the break with TADS 3, which plays it. His walkthrough (`SCWalkThru.txt`), made an event script of 179 entries, plays the original to a win with 19 of 20 points, repeatably (`--norand`; the game reads no clock), and its transcript, `reference.out`, is what the port is compared with. The walkthrough was written for an earlier release: this one's hermit asks who you are before he will talk about anything else, so the script answers "no idea". The point it misses is for reading the statement the man in the white coat wants signed, and he appears only when the player is caught; a second script, in which the player is caught, will cover him and the rest of the routes that fail. The comparison is of the story's text: where adv3 and Hexad word a library message differently ("Obvious exits lead..." against "From here you could go..."), the difference is the library's and is noted as such.

The chunks: (1) the reference, done; (2) the world, the prison and the fields with the books, the verbs, and `DupDobj`, playable without the characters, done; (3) the five characters and their conversation; (4) the hints, the second script, and the comparison with the reference; (5) the findings, and with them the answer to what Hexad should take from adv3.

**The world (chunk 2, 2026-11-25).** The prison, the fields and the books (8,300 lines of adv3) are made by a converter for what is mechanical (vocabulary, handlers, the `g` macros, message parameters, adv3's names for Hexad's properties, room parts, `MultiInstance`) and then about 210 changes by hand, kept as patches, so that the port can be rebuilt from the original as the converter learns; the verbs (2,500 lines, mostly adv3's grammar machinery) are rewritten by hand. A file of adv3's classes built on Hexad's (`Hidden`, `Readable`, `OutOfReach`, `PresentLater`, `AutoClosingDoor`, `NoTravelMessage`, and the rest) covers what the game uses of adv3 that Hexad does another way, and empty stand-ins take the characters' places. The walkthrough plays through the world to the guardian, where every difference from the reference is Hexad's wording or order ("In the small bookcase you see..." for "The small bookcase contains...", "Unlocked." against "You unlock the drawer with the small brass key."); past him, without the characters, it plays on through the courtyard, the alley, the fields and the forest with no runtime error and 8 of the world's points. What it found:

- **A stack overflow said only "VM Error: stack overflow"**, not where: reporting it built a stack trace of thousands of levels, which overflowed again. The trace of an overflow now keeps the 40 innermost and 40 outermost levels, the error is reported like any other (`--> file:line`), and a run of levels on one line is shown once, as "(the same, again and again)". `HEXICON_TRACE_ERRORS` also gives the source line now.
- **An object defined on a MultiLoc object was in all of that object's rooms.** adv3 makes a room's own floor so (`alleyGround: defaultGround`); on Hexad it took `defaultGround`'s worked-out list of rooms for its own. Two floors in a room made the room's contents contain themselves (a floor's contents are its room's), and the sensory extension, which gathers a room's contents every turn, overflowed in a third of the rooms. Hexad now notes every MultiLoc's declared locations before it works any out.
- **Opening something said so twice** when the thing's own `makeOpen` described the opening: the opening lister added "You open the large steel panel." when nothing in it was worth listing. It now leaves that to Open's report, which says the same when nothing else has been said.
- **Hexad's macro `stop`** (for its rules) takes the name from any method, as `exit` does: the game's siren had a `stop()`.
- **A TravelBarrier's `canTravelerPass` is called with two arguments**, and adv3's took one; `explainTravelBarrier` was made to fit its declaration (Ruins), but `canTravelerPass` was not.
- **Weak words work** in Hexad's vocabulary, written `(word)` as in adv3; Ruins said Hexad had none.
- **adv3 shows a sound or smell only when listened for**, unless it says otherwise; Hexad's sensory extension announces each on the first turn. And a room's description says "You are on the bed." where adv3 says it only in the heading (`pcListedInLook`). The port sets both as adv3 has them.
- Still to do: senses across rooms (adv3's `DistanceConnector` and `Occluder`, seeing the courtyard from the roofs), marked in the port; Hexad's `SenseRegion` is the candidate.

**The characters (chunk 3, 2026-11-25).** The guardian, the hermit, the receptionist, the man in the white coat, and the pig (4,900 lines of adv3) go through the same converter and patches as the world, and adv3's conversation is built on Hexad's in a file of its own (about 360 lines): the ready and in-conversation states, `SpecialTopic` (as a `SayTopic` that its keywords turn into), the suggested topics, `HermitActorState`, a character who goes with the player (`AccompanyingState`), and a conversation a character starts at a node. adv3's senses across rooms are ported too: each `DistanceConnector` is a `SenseRegion` of its two rooms, and an `Occluder`'s `occludeObj` is asked by `isVisibleFrom`. The walkthrough now plays to the win, 19 of 20 points as on the original, with no runtime error; 130 of its 181 responses are the reference's word for word, and the other 51 differ only in Hexad's wording or order. What it found:

- **Suggested topics came out in a scrambled order** ("You could refuse to answer or tell him you don't know"). The lister sorts by `listOrder`, the same for all of them, and the sort, which is the reference VM's quicksort on purpose, does not keep the order among equals. The suggestion lister now keeps the order the topics were defined in among equals. Hexad's contents listers sort the same way (eleven places), so "a dark blue book, a green book, and a red book" where adv3 lists them as defined; fixing them reorders every game's transcript, and is left for chunk 5.
- **From afar, a character's things were listed** ("In the black holster is a machine-pistol."): after a remote thing's special description, Hexad listed the contents of its contents, which skipped the check that keeps an actor's inventory out. It now lists the thing itself, as the local listing does.
- **"Okay, You're no longer wearing..."**: Hexad's message had `{I}` mid-sentence.
- **A verb ending after an embedded expression loses its subject.** `"{I} <<stat ? nil : 'un'>>screw{s/d}"` gave "You unscrews": the string reaches the output in pieces, and Hexad works out each piece's parameters on their own, so the ending has no subject and takes the third person. adv3's message builder remembers the last subject across pieces. (A tag split the same way, `"<.convnode <<key>>>"`, failed too, but that string was the port's own: adv3 also filters output piece by piece, and the game never splits a tag.)
- **Where adv3's characters behave differently**, the port follows adv3, and each is a question for chunk 5: a state's topics come before the character's in Hexad (so a state's default answer beat the hermit's own topics), where adv3 weighs them together; Hexad skips a character's agenda and script on a turn it is talked to, and runs a state's script only where the player can see it, where adv3 runs both (the pig finishes his meal out of sight); Hexad says goodbye whenever the player walks off, adv3 only for a character in conversation; in adv3 a character's entering an in-conversation state is being in conversation, while Hexad goes by the player's interlocutor; and a character told to go through a door goes without a word, since only `Actor.travelVia` says it leaves.
- **Smaller differences**: Hexad's `{s}` and `{es}` are a noun's plural endings, adv3's a verb's; `isOpen` is true in Hexad for anything that cannot be opened, in adv3 only for a container; adv3 marks a thing described when it is read and its text is its description; and a `location` given by an expression (`trough.remapIn`) is not added to that thing's contents, where Hexad's way is `subLocation`.
- **The wording that differs** is listed in the port's notes: chiefly the contents ("In the trough you see..." for "The trough contains..."), an implicit take that does not say where from, a closed container's examination that does not say it is closed, "You put the document in the envelope." for "Done.", and the punctuation of suggestions.

**The hints, the failing routes, and the comparison (chunk 4, 2026-11-25).** The hints (30 goals) go onto Hexad's hint system as they are, but for the name of one condition (adv3's `openWhenDescribed` is Hexad's `openWhenExamined`), and the menu opens the same goals at the same moments as the original's. Three more scripts, each recorded from the original, cover what the walkthrough does not: walking out of the gates once released, being caught by the man in the white coat, reading his statement (the point the walkthrough misses), refusing, and waking in the cell; opening the blue door with the alarm on and answering no; and signing, which ends the game. `tools/game-check.py` now plays every script a game has (`walk-NAME.txt` beside `walk.txt`, checked as `game/NAME`). With the library's known phrasings put into one form on both sides, every remaining difference in the four scripts is the library's, none the story's: 166 of the walkthrough's 181 responses match, and the rest are paragraph order, several objects reported together, travel seen from afar, and the like. The turn count is one more in each, from one command: adv3 counted no time for the turn on which it made the guardian walk off. What it found:

- **An implicit action carried out by an action's notifications was never announced** when the action shows its result as it acts. The cell stands the player up first for READ NOTE (`tryImplicitAction(Stand)` in its `roomBeforeAction`): adv3 said "(first standing up)", Hexad nothing, since it shows implicit actions with an action's report and Read and Examine have none. They are now shown once the notifications are done.
- **A topic could only be something already seen or known.** The man in the white coat comes out of his hut holding the statement after the room was described, so it had never been seen: asking him for it got his default answer, and the topic was not suggested. Anything the player can see now can be a topic, and suggested.
- `hexad-fixes` checks both.
- **To weigh**: Hexad does not show an implicit IAction's own text (it is how the default "Okay, you're now standing." is hidden), so a game's own message for one is lost; `discover` stores `isHidden`, so a hidden thing's own rule for being hidden stops working once it is found; and a suggestion list with a topic that has no name says "Among other things", where adv3 counted only its suggested topics.

**What Hexad should take from adv3 (chunk 5, 2026-11-25).** The answer the port gives is: little of adv3's machinery, and a short list of its behavior. The port is 13,650 lines on Hexad from 17,260 of adv3, made by a converter of 51 rules and 238 changes by hand, with two files of adv3's ways on Hexad's: 311 lines for the world (`Hidden`, `Readable`, `PresentLater`, `AutoClosingDoor`, the Occluder) and 368 for the conversation (the ready and in-conversation states, `SpecialTopic`, the suggested topics, `AccompanyingState`). Every one of adv3's 148 classes the game uses was met with Hexad's own or a few lines on them; nothing needed a new mechanism in Hexad, and the senses across rooms, the gap Ruins and chunk 2 feared, are Hexad's `SenseRegion` taken two rooms at a time. Those two files stay in the port: Hexad is the only library, and adv3's classes are not offered to authors. What adv3 does better is in the behavior of things Hexad already has, and each of those below is a change to Hexad that every game would see.

Fixed as the port went (chunks 2 to 4): a stack overflow reported where it happened; a floor defined on a MultiLoc in its own room only; opening not reported twice; suggested topics in the order they were defined; a remote character's belongings not listed; the doff message's capital; implicit actions done by notifications announced; and anything in view a possible topic.

Proposed, for a decision each (they change transcripts, so each would be made with its games re-recorded):

| | What adv3 does | What Hexad does now | Proposal |
|---|---|---|---|
| 1 | Lists things with the same `listOrder` in a steady order, that of its contents lists (the order the objects are numbered in, which is not quite the order of the source) | Lists them in the order its unstable sort leaves them ("a dark blue book, a green book, and a red book") | **Take it**, and go one better: break ties by order of definition in the contents listers and the suggestion lister |
| 2 | Says that a character it moves by a command (`nestedActorAction(npc, GoThrough, door)`) leaves, as for any travel | Says so only for `Actor.travelVia`; a commanded GoThrough or Go moves the character without a word | **Take it**: announce the travel of a character the player can see, however it was started |
| 3 | A topic's default answer yields to a real topic, wherever each is defined | A state's topics come first, so a state's `DefaultAskTopic` answers a question the character's own `AskTopic` matches | **Take it**: compare the state's best match and the character's, and let a default win only when nothing else matches |
| 4 | Remembers the last subject across the pieces of a string, so a verb ending after an embedded expression agrees with it | Each piece starts afresh: "You unscrews" | **Take it, carefully**: keep the subject of the last piece while one double-quoted string is being shown (the compiler knows where a string's pieces begin and end) |
| 5 | Examining a closed container says "It's closed." | Says nothing of it | **Take it**: the player otherwise cannot tell |
| 6 | "(You could say yes or no.)" | "(You could say yes; or say no)": yes and no are sections of their own | **Take it**: list yes and no as one section |
| 7 | An implicit take says where from: "(first taking the document from the envelope)" | "(first taking the document)" | Worth having when the thing was not directly in the room; a smaller change |
| 8 | Runs a character's agenda and its state's script on a turn it is talked to, and out of the player's sight (silently) | Skips both on a turn it is talked to, and runs a state's script only where the player can see | **Keep Hexad's**, and say so in the guide: a script that changes the world (the pig finishing his meal) belongs in a daemon or an agenda item, not a fidget list |
| 9 | Says goodbye on walking off only for a character in an in-conversation state | Says goodbye whenever the player walks off from the current interlocutor | **Keep Hexad's**: it has no separate in-conversation states |
| 10 | Shows an implicit action's own text | Shows only its announcement (how "Okay, you're now standing." is hidden) | **Keep Hexad's**; a game says its own text for an implicit action where it calls it, as the port's cell does |
| 11 | `isOpen` only for containers; `{s}` a verb ending; `described` for examined; `location = x.subContainer` | `isOpen` for anything that cannot be opened; `{s}` a plural ending; `examined`; `subLocation = &remapIn` | **Keep Hexad's**: these are its names and ways, and a port meets them once |

The library's wording otherwise ("In the trough you see..." for "The trough contains...", "Done." against "You put the document in the envelope.", the score's sentence) is Hexad's choice, and stays.

**Done, 2026-11-25: proposals 2 and 3.** Travel started by a command (GO THROUGH, CLIMB, ENTER, GO BACK, and the rest) now goes through `travelOnCommand(conn, actor)`, which sends a character other than the player by `Actor.travelVia`, so that its leaving and arriving are reported where the player can see them, as travel by a direction already was. And when an actor's state has only a default answer, a topic of the actor's own that matches is used instead. The port's two workarounds are gone and it plays as before; `hexad-fixes` checks both (the courier sent off through an archway, and his own topic about the letter against his state's default). A Passage still reports a character's leaving as "leaves the area", where a Door says "leaves through the door": a smaller matter, not changed.

**Done, 2026-11-26: proposals 1, 5 and 6, and the Passage.** The listers sort through `sortInOrder`, which keeps things with the same `listOrder` in the order the source defines them (their `sourceTextOrder`, counted within each file), and anything not defined in the source in the order it already had: "a sprig of lavender, an apple, a scarf, and a candle", as the attic's source has them, where the sort had left "a candle, a sprig of lavender, an apple, and a scarf". This is not quite adv3's order, which is that of the objects' numbers: Square Circle's round room now describes its globe before its table, as its source does, and adv3 the other way round; but the walkthrough matches the reference in three more places. Examining something openable and closed says so ("It's closed."), unless its `closedStatusReportable` is nil; Exercise 21's window, whose description says it, sets that. Yes and no are suggested together ("say yes or no"), except when suggestions are numbered or links. And a Passage reports a character leaving through it as a Door does. Every changed response in the 18 transcripts and the guide's examples was checked to be one of these and nothing else, and the guide says what the order and the closed status are.

**Done, 2026-11-26: proposals 4 and 7.** A verb ending just after an embedded expression agrees with the subject before it: "{I} <<stat ? '' : 'un'>>screw{s/d}" gives "You unscrew", not "You unscrews". Each piece of such a string is worked out on its own, so the message processor now remembers the last subject a piece named, until a sentence ends within the same command, and gives it to a piece that starts in the middle of a word (`screw{s/d}`) or with a verb ending (`{s/d}` after `<<gVerbName>>`); a piece that starts anything else starts with no subject, as before, so "{I} see that <<box.theName>> {is} broken" is not made "am". On the way it found that a piece starting with a verb ending, as in "{I} <<verb>>{s/d}", hung the game: the rewriting of `verb{s/d}` looked for a verb before the start of the text and never moved on. It now gives the ending alone, and no longer drops the first letter of a verb at the very start. And an implicit take of something the player already carries inside something else says where from, as adv3 does: "(first taking the document from the square white envelope)". The port's three workarounds for the verb endings are gone; Square Circle's walkthrough matches the reference in 141 of its 181 responses. `hexad-fixes` checks all three (AWAKEN, YAWN, and the glove in the satchel).

**Done, 2026-11-26: two leftovers.** Things from two files in one place are listed in the order of the build's files, then of their definitions in each: every file that imports Hexad has its objects' `sourceTextGroup` (Hexad's header turns the groups on), and `sortInOrder` now compares that first, as Hexad's command rules already did. Before, a thing defined early in a later file came before one defined late in an earlier file. `hexad-fixes` checks it with a second source. And a list of suggestions after a response ends with a full stop, as adv3's did: "(You could say yes or no.)". What still differs from adv3's is its comma before the last "or", even between two ("tell him you don't know, or refuse to answer"), which Hexad leaves out.

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

### The Tide Mill (2026-11-26)

The first game written for Hexad rather than ported to it, from the guide alone, to find what writing one is like: what the guide leaves out, what players type that Hexad does not understand, and what Hexad does that it should not. Ada, who keeps a tide mill, has broken her leg, and the bakery needs its flour by morning: find the key to her cottage, the crank handle, and the oil; free the wheel, fill the hopper, open the sluice, set the stones going, and bring her the flour. Eight rooms, one character, seven points, 700 lines. It is ours, so it could go anywhere, but for now it is in the playground's `game-tests/tidemill`, with two scripts: the walkthrough (49 commands, to the full score) and `walk-phrasings`, about 70 commands in the words a player might use instead.

Written first from the guide, it won, and then a player's commands found most of what follows. Where a thing was the game's own slip (a topic left out, a noun in a description with no object), it was put right in the game and is not listed.

**Fixed in Hexad: an answer to "What do you want to look that up in?" vanished.** LOOK UP LANTERN asks what to look it up in, and whatever the player typed next, a book or a new command such as JUMP, did nothing and said nothing. The rule for LOOK UP ... IN had its two answer forms the wrong way round, so the answer was read as a topic, which anything is. Both are now right; `hexad-fixes` checks a book given as the answer and a new command given instead.

**What the guide does not say**, and the game needed (each found by reading Hexad's sources):

| Needed | What Hexad has |
|---|---|
| A handler naming the other object: is the thing being fitted the crank handle? | `now.dobj`, `now.iobj` |
| A timed event: the flour comes three turns after the stones start | `new Fuse(obj, &method, turns)`; the guide names `Fuse` and `Daemon` but not how to make one |
| A ladder to climb, as well as UP | `StairwayUp` and `StairwayDown`, with a `destination`; `goInstead(dir)` for a thing that sends the player a way |
| A handle fitted to a windlass | `SimpleAttachable`, `allowableAttachments`, `attachedTo` |
| Oil poured on the windlass | `isPourable`, `fluidName`, `allowPourOntoMe`, and the handler's `action()`, since Hexad's own pouring does nothing |
| A character mentions the wheel; ASK ABOUT WHEEL gets the default, since the player has not seen it | `familiar = true`, or `<.known obj>` in what she says |
| A greeting that is not the first one | `HelloTopic` with a `StopEventList`: every conversation starts with a greeting, so the first one's words came back |
| Fidgets one turn in three, not every turn | `eventPercent` |
| OPEN DRAWER, for a desk's part | the guide says players never name a part; they do, so the part's noun goes in the thing's vocabulary |
| A name such as Ada | a capitalized name is proper ("Ada won't let you pick her up"), which works, and is not said |

And one surprise: a misspelled name in an expression (`gDobj`, adv3's name) is a warning, and the game builds, though the guide shows an undefined name as an error. Here the undefined name became a property of the object, nil at run time.

**What players type that Hexad does not understand.** The game added two actions and five rules for Hexad's own: KNOCK ON, RAISE or LIFT, FIT ... TO, WIND, EMPTY ... INTO, OIL ... WITH, and PULL or HOOK ... WITH. Still not understood: LOOK OUT OF THE WINDOW (read as examining "out window"), CONSULT LEDGER without a topic, FILL, and HOOK DRIFTWOOD on its own. Most are on Ditch Day's list too (below). USE is refused with a reason, which is the right answer.

**What Hexad does that it should not:**

- **It corrects a verb into another verb.** WIND WINDLASS became "(find windlass)" and asked what to look it up in; HOOK DRIFTWOOD became "(look driftwood)" and examined it. A misspelled noun is worth correcting; a word the game does not know as a verb is more often a verb it lacks, and turning it into another action misleads the player.
- **EXAMINE ME says "You see nothing special about you."**, where it means yourself.
- **HELP offers what the game does not have**: HINT and EXTRA ON in a game with neither, which then say so.
- **Scenery is one object for each noun.** The game has ten decorations, each four lines, for the hedges, the range, the quilts and the rest. A room's scenery as one list of names and descriptions would save most of that.

Recommended (2026-11-26): put the guide's gaps right first, since each cost a search of the library; then the verbs, with Ditch Day's list; then the spelling of verbs, EXAMINE ME, and HELP; and scenery as a list as a design of its own.

**Done, 2026-11-26: the guide's gaps.** Each is now in the guide, in the example of the page it belongs to, so doc-check keeps it true: `now.dobj` and `now.iobj`, that both objects of a two-object action must accept, fuses and daemons (the Belfry's bell hums for two turns after it is rung), and pouring, in Actions; stairs and ladders (`StairwayUp`, `StairwayDown`) and `goInstead`, in Things and Rooms (the Attic's ladder); attaching (`SimpleAttachable`), and a part's noun among its thing's (OPEN DRAWER), in Containers (the Study); `<.known>`, a greeting that is not the first, `eventPercent`, and proper names, in Characters (the Ferry); and the warning for an undefined name, in Getting Started. Writing that last one found that the compiler's "did you mean" looked only at the whole program's names, so `stats` was taken for `State` rather than the method's own `stat`: a slip for a local or a parameter in scope is now the one suggested, with a test. Two small things seen on the way, left for later: an undefined object named inside a string's `<< >>` is reported at the end of its object, not the line it is on; and examining a thing with another attached says "You see a small picture attached to the brass hook", where "The small picture is attached to it" would read better.

**Done, 2026-11-26: the verbs.** First, what made adding them safe: a game that defined an action Hexad also defined failed to build ("the name "Jump" redefined"), so each verb Hexad gained could break the games that had written their own, four of the playground's among them (`KnockOn`, `FillWith`, `Empty`, `Lift`). Now a game's own action, or verb rule, of the same name as a built-in library's takes its place, with a warning at its line ("the action "KnockOn" takes the place of the built-in library's"); anything else of the same name is still an error. A compiler test checks both. Then the verbs (recommended set, 2026-11-26):

- **New actions**, each refused by default with a message of its own, for a game to give a handler where it makes sense: KNOCK ON (or AT, or RAP, TAP; KNOCK alone asks on what; a door is the likeliest thing meant, and nobody answers), RAISE or LIFT, LOWER, FILL and FILL ... WITH (`isFillable`, `canFillWithMe`; FILL alone asks with what), REPAIR, FIX, or MEND, PLAY, BUY or PURCHASE ("The crate isn't for sale."), and CONSULT on its own, which asks what about.
- **EMPTY does something by itself**: a container has what can be taken out of it tipped out where the player is ("You empty a pebble and a shell out of the crate."), opening it first; EMPTY ... INTO puts it all in another, if there is room and it is not inside the first; and a thing that pours is poured. And POUR, which printed nothing at all for a pourable thing with no handler, now says it was done.
- **New words for actions Hexad has**: FIT or FIX ... TO (attach), WIND (turn), GRAB and SNATCH (take), SLIDE and ROLL (push, and push a way), POWER ON and OFF, ACCOMPANY and GO WITH (follow), SWAT (attack), CROSS (go through), LOOK OUT OF (look through), and EMPTY ... ONTO (pour onto).

`hexad-fixes` checks each in a new shed. The Tide Mill now uses Hexad's KNOCK, RAISE, FIT, WIND, and EMPTY instead of its own, and plays the same; its LOOK OUT OF THE WINDOW now works. Seen on the way: LIFT for a thing that can be taken is refused as RAISE is, where a player may mean TAKE or LOOK UNDER; and the error for a name defined twice is still the old one's words, with no file or line.

**Done, 2026-11-26: the three small things.**

- **EXAMINE ME says "You see nothing special about yourself."** Hexad had the reflexive (the actor named as an object in a sentence it is the subject of), but checked for it only where a message marks a noun as an object, and `{the 1}` and its kind are not marked; it now counts too when the sentence already has its subject. Every change in the games' transcripts is of that kind: "about you" and "behind you" and "under you" became "yourself". (Hexad's note of having passed the sentence's verb, which the reflexive could have used, is never made: `noteVerb` is never called.)
- **HELP offers HINT and EXTRA ON only in a game that has them**: hints when the game has a hint menu, extra hints when it has any. It asked only whether the systems that show them were there, which they always are.
- **A command's first word is corrected only to a word with the same first letter.** WIND and HOOK are words the Tide Mill did not know as verbs, and were taken for FIND and LOOK; a word in a verb's place that the game does not know is more often a verb it lacks than a misspelling, and the first letter is the one least often mistyped. TKAE is still TAKE, and EXMAINE EXAMINE. The exercises' transcripts show what else it changes, all for the better: READ SORT is no longer "(head port)", SHOW FLIES TO SINCE no longer "(how lips to mine)", and OOPS with nothing to correct no longer "(tops)", examining a work surface.

`hexad-fixes` checks each.

**Proposal, 2026-11-26: scenery as a list.** The Tide Mill has ten decorations, each its own object of three or four lines, for the nouns its descriptions mention. Hexad already has a way to write them together, adv3Lite's `Scenery` (in `scenery.hxn`), which the guide never mentions:

```
+ Scenery
    scenList = [
        ['range; cold black; stove fire', 'A black iron range, gone cold. '],
        ['plates; blue willow; plate; them', 'Blue willow-pattern plates. '],
        ['gears; wooden; cogs', {: "Wooden cogs<<if mill.running>>, turning<<end>>. " }]
    ]
;
```

Put into a copy of the Tide Mill, it plays both scripts word for word as before. But it is a list of lists of strings: a description that changes must be wrapped as a function (`{: "..." }`), a third element gives each item its own "not important" message by one of four kinds of value, and the items are made while the game starts, as objects of a class of their own (`ScenItem`), so nothing else about them can be said. Two ways on:

1. **Put `Scenery` in the guide as it is**, on the page of things and rooms. Nothing new to build.
2. **A form of Hexicon's own**, as `action` and `verb` are, that the compiler turns into one `Decoration` for each line:

   ```
   + scenery
       'range; cold black; stove fire': "A black iron range, gone cold. "
       'plates; blue willow; plate; them': "Blue willow-pattern plates. "
       'gears; wooden; cogs': "Wooden cogs<<if mill.running>>, turning<<end>>. "
   ;
   ```

   Each is an ordinary decoration, where it would have been, with its description written as any other is, changing or not; one that needs more (its own `notImportantMsg`, a smell) is written as a `Decoration` in full instead. Nothing is made at run time. The cost is the compiler's: reading the form, with its errors, and a test.

Recommended: 2, with the guide showing it; `Scenery` stays for games that have it. And found on the way, for the guide whichever is chosen: a plural noun needs `them` among its pronouns, or Hexad says "The plates is not important."; the guide's vocabulary paragraph does not say so.

**Done, 2026-11-26: `scenery` (2, as recommended).** The compiler reads `+ scenery`, with as many `+` signs as any object, and then pairs of vocabulary and description up to a `;`, and turns each pair into `+ Decoration 'vocabulary' "description";` where the form is; a description is any string in double quotes, embedded code and all. It says what is wrong with a pair that has no colon, a description not in double quotes, or something that is neither a pair nor the end (a compiler test each), and a program case (`tests/programs/hand/scenery`) builds one on a `Decoration` of its own and shows each piece in its place, its description changing with the room. The guide has it on the page of things and rooms, in the Attic (its dust, boards, and roof), with the note on plurals there, and among Hexicon's own forms on the language page. The Tide Mill's nine plain decorations are now `scenery`, and it plays both its scripts as before; its rooms have one or two pieces each, so it is no shorter, and the saving is in a room with many, and in not naming each. Its plural nouns now have `them`.

**Done, 2026-11-26: Ditch Day's parser hooks and missing classes.** Its two hooks, `GenericObject` and `DisambigDeferrer`, are each a few lines on one adv3 method, `filterResolveList`, which lets an object take itself out of what the player's words matched. Hexad has the same method on every `Thing` (`Unthing` uses it to give way to anything real), so what it lacked was a way to say so without knowing of the method:

- **`yieldsToOthers`**: a thing gives way to any other the words match, as a thing that stands for many does (BOX means the one hatbox, not the boxes under the eaves). **`yieldsTo`**: the things, or classes, it gives way to. Neither applies when the player asks for all (ALL, or a plural, so X BOXES shows both), and a thing gives way only to things the words fit as well as they fit it: Hexad keeps only the best matches before it asks any to step aside, so a painting called "painting of a lamp" still beats a desk lamp for LAMP. `vocabLikelihood` weighs a thing up or down without ruling it out.
- **`Keyring`** (Ditch Day's `Keyring`): only keys go on it; a key on a keyring the player holds is at hand to lock and unlock with, both when UNLOCK finds a key and for the key's own precondition (`keyAtHand`); and a key taken while holding a ring goes on it ("Taken. You put it on the keyring."), unless `collectsKeys` is nil, while one taken off it stays off.
- **`Vaporous`** (smoke, mist, steam): a decoration that answers EXAMINE, SMELL, LISTEN TO, LOOK THROUGH, LOOK IN, and SEARCH, and is too insubstantial for anything else. Square Circle's port had an empty one of its own, now gone; its four scripts play the same on Hexad's.
- **`PermanentAttachment` and `MultiFaceted`** needed nothing new: Hexad's `AttachableComponent` is attached to its location from the start and cannot be detached or taken, and `MultiLoc` (one thing in several rooms) and `getFacets` (several objects for one thing, sharing pronouns, as a door's sides do) cover the other.

`hexad-fixes` checks each in a new workroom. The guide has them: giving way, `vocabLikelihood`, `MultiLoc` and `getFacets`, and `Vaporous`, `Odor`, and `Noise` on the page of things and rooms (the Attic's boxes and hatbox); the keyring and `AttachableComponent` on the page of containers (the Study's keyring, which locks the cabinet from the ring).

Open question, for the next time Hexad gains a class: Square Circle did not build until its own `Vaporous` went, since a game's own definition takes the place of the library's only for actions and verb rules. The same rule could cover classes, with the same warning.

### Return to Ditch Day (surveyed 2026-11-26)

Michael Roberts's 2004 game, written to show adv3 at its fullest: 36,160 lines in 21 files, twice Square Circle. Before deciding on a port, a survey of what it uses (the playground's `tools/adv3-classes.py` and `tools/adv3-calls.py`, which compare a game's adv3 classes and calls with Hexad's). **Its licence, like Square Circle's, forbids distributing a changed version**, so a port would live in the private playground.

- **Classes.** It builds on 125 of adv3's classes (Square Circle, counted the same way: 111). Hexad has 72 by name. Of the 53 it lacks, most are the ones Square Circle met and the port covers with a few lines on Hexad's: `PresentLater` (50 uses), `NoTravelMessage`, `Readable`, `Person`, the conversation states, `ComplexContainer`, `Lockable`. The travel classes (`TravelWithMessage`, `ThroughPassage`, `FakeConnector`) are Hexad's connectors with a `travelDesc`; the tour guide (`TourGuide`, `GuidedTourState`) is Hexad's `FollowAgendaItem`; `Vehicle` is `isVehicle`; `OnOffControl` is a Switch. Four have nothing like them in Hexad: `PermanentAttachment` (a thing fixed to another for good), `MultiFaceted` (one thing seen as separate objects in several places), `Vaporous` (smoke or fog: seen and smelled, not touched), and `Keyring`.
- **adv3's machinery.** It calls 43 of adv3's methods that Hexad never mentions, 75 times in all (Square Circle: 22, 48 times). Most have Hexad names (`makePresentByKey`, `setIt`, `spellInt`, `travelTo`). Two groups are new: adv3's reports (`addReport`, `forEachReport`, `defaultDescReport`), which Hexad does not have, and the parser's (`getOrigText`, `getWordMatches`, `intersectNounLists`).
- **What its author added to adv3** (`mods.t`, 1,310 lines) is the most telling part: about twenty verbs adv3 lacked and Hexad lacks too (USE, KNOCK, REPAIR, PLAY, RAISE and LOWER, BUY and PAY, SLIDE and ROLL, HIDE IN, CROSS, HANG ON, TEST WITH, ACCOMPANY, HOLD); preconditions (hands empty, switched on or off, not worn); worn things (always worn, worn from the start, on the hands); and help for the parser: a generic object standing for many unimplemented ones (the books on a shelf) that gives way to a real one, objects that step aside in disambiguation by a rule, "the door to X" as a noun phrase, chairs for several, and keypads.
- **Its own modules** Hexad has already: `subtime.t` is Hexad's `subtime` extension, and Hexad has a path finder of its own (Ditch Day wrote one because adv3 had none), footnotes, hints, and the system's `reflect`.

So a port would find little new in Hexad's mechanisms: four small classes, the report machinery, and some parser hooks, at the cost of twice Square Circle's work. What it shows best is in `mods.t`: what the author of adv3 found missing while writing its showcase. That list is the input to an improvement pass (a fuller set of verbs, generic objects, rule-based disambiguation, the common preconditions, a keyring, vapor), and the parser hooks are worth a port of their own pieces later, as a probe of how far Hexad's parser can be extended. Recommended (2026-11-26): no full port for now.

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

1. Hexad's words: decided and made, above, except the later ones.

Settled since the draft: compatibility with adv3Lite games (none is owed, above); whether the English messages are a module of their own (they are, as `lib/hexad/english` with its own manifest); and which of adv3's features to bring back (2026-11-26). The likely ones were the senses, nested rooms and postures, and report grouping. Square Circle, which uses 148 of adv3's classes, answered it (Square Circle, "What Hexad should take from adv3", above): none needs bringing back. The senses across rooms are Hexad's `SenseRegion`, nested rooms and postures its rooms and the `postures` extension, and what adv3 did better was in the behavior of things Hexad has, which is now Hexad's too (listing in the source's order, announcing commanded travel, the closed status, implicit actions announced, and the rest). Report grouping came up only as wording: Hexad reports what one command did to several objects in one line, where adv3 gave each its own, and that is Hexad's choice.

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
5. **adv3's depth as modules.** The senses, nested rooms and postures, and whatever else was chosen, each optional and with a test game of its own. Not needed (2026-11-26): the Square Circle port found Hexad has each already, and brought adv3's better behavior into Hexad itself rather than into modules (open questions, above). The milestone is closed; what a game written for Hexad finds missing will be added as it is found, with a test game, as the extensions are.
