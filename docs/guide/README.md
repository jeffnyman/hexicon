# The Hexicon Guide

For writing games with Hexicon.

- [Getting Started](getting-started.md): getting Hexicon, starting a game, and writing *The Lighthouse*, a small game, step by step.

Hexad, the library games are built on:

- [Things and Rooms](things-and-rooms.md): what things are, what can be taken, what a room lists, the senses, hidden things, exits, and light and dark.
- [Containers, Surfaces, Doors, and Keys](containers-and-doors.md): what holds what, a thing with several parts, things hidden under and behind, locks, keys, and doors.
- [Characters and Conversation](characters.md): characters and their states, topic entries and what they answer, topics, revealing, and responses that change.
- [Actions and Their Handlers](actions.md): how a command is carried out, a thing's own handlers, and new actions and their grammar.

The language:

- [The Language](language.md): programs and files, values and operators, strings and what can go in them, lists, tables, statements, functions, objects and classes, templates, exceptions, and the preprocessor.

## For the guide's writers

Every example in [examples](examples) is a game, and `python tools/doc-check.py` builds and plays each one, from its `walk.txt`, against the transcript recorded in its `walk.out`. It also checks that each code block marked `hexicon` in these pages is part of an example's source, and each marked `transcript` part of a recorded transcript, word for word. So a page shows only code that builds and output a game really gives. When a change to Hexad is meant to change what an example prints, `--accept` records the new transcripts.
