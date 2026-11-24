# The Hexicon Guide

For writing games with Hexicon.

- [Getting Started](getting-started.md): getting Hexicon, starting a game, and writing *The Lighthouse*, a small game, step by step.

More is to come: a reference to Hexad, the library games are built on, and to Hexicon's language.

## For the guide's writers

Every example in [examples](examples) is a game, and `python tools/doc-check.py` builds and plays each one, from its `walk.txt`, against the transcript recorded in its `walk.out`. It also checks that each code block marked `hexicon` in these pages is part of an example's source, and each marked `transcript` part of a recorded transcript, word for word. So a page shows only code that builds and output a game really gives. When a change to Hexad is meant to change what an example prints, `--accept` records the new transcripts.
