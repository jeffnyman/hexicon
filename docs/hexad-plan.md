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

## Open questions

1. Is adv3Lite the right base?
2. How long should Hexad stay compatible with adv3Lite games?
3. Which of adv3's features matter most to bring back? The senses, nested rooms and postures, and report grouping are the likely ones.
4. Should Hexad live in this repository (`lib/hexad`) or in its own?
5. Should the English messages be a separate module from the start, as adv3Lite's `english` is?

## Milestones

1. **The fork.** Hexad is adv3Lite 2.2.3.2 under its new name, built from a `hexicon.toml`. Exit: Airport and the manual's exercises replay identically on it.
2. **In Hexicon's language.** Once the converter exists (language milestone 2), Hexad's sources become `.hxn`. Exit: images identical to milestone 1's.
3. **The language's features adopted.** Checked names, action declarations, and modules come in as the language gains them. Each step is checked by the same games still playing the same.
4. **adv3's depth as modules.** The senses, nested rooms and postures, and whatever else is chosen. Each is optional and comes with a test game of its own.
