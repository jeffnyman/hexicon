# Tests that need only Hexicon

What is here checks Hexicon with nothing else installed.

## programs

`programs/` holds the compiler's test programs:

- `gen/`: small programs, each a probe of one case of the compiler.
- `hand/`: longer programs, and, as folders, programs built from several
  files.

Beside each program is a `.expect` file: a fingerprint of the image it
builds, what building it says, and what it prints when it runs.

```
python tools/program-check.py            # check them all
python tools/program-check.py hello      # or some
python tools/program-check.py --accept   # record what Hexicon does now
```

The programs were TADS 3 programs, written to compare Hexicon's
compiler with the reference compiler, and were made Hexicon programs on
2026-11-24. Each was kept only if the reference, with the original, did
what Hexicon did with it: both built an image or neither did, and both
printed the same. Three were kept with what Hexicon does differently by
design: what the VM says about itself, and how `hexicon` reports an
image it cannot run. A program that never finishes is built but not
run.

A difference means Hexicon now does something else. If that is meant,
`--accept` records it; look at what changed first (the new results are
in `build/program-check`).

## Games

`tools/game-check.py` builds and plays whole games on Hexad (Airport and
the exercises of adv3Lite's manual, in Hexicon) and compares their
transcripts with those recorded before. The games are not ours, so they
are kept outside this repository, by default in
`../hexicon-playground/game-tests`; without them, the check says so and
passes. A game with scenarios has them played by `hexicon test` as well,
and they must all pass (see `docs/story-testing.md`). The guide's own
example games are played by `tools/doc-check.py`.
