# Tests that need only Hexicon

What is here checks Hexicon with nothing else installed: no TADS 3, no
reference compiler or interpreter.

## programs

`programs/` holds the compiler's test programs as Hexicon programs:

- `gen/`: the small programs `tools/compile-tests/gen-cases.py` writes,
  each a probe of one case of the compiler.
- `hand/`: the programs in `tools/compile-tests` (and, as folders, those
  in `tools/compile-tests/linked` that are built from several files).

Beside each program is a `.expect` file: a fingerprint of the image it
builds, what building it says, and what it prints when it runs.

```
python tools/program-check.py            # check them all
python tools/program-check.py hello      # or some
python tools/program-check.py --accept   # record what Hexicon does now
```

The programs were made from TADS 3 sources, once, while the reference
was at hand (`--record`). For each, the reference compiler built the
original and the reference interpreter ran it, Hexicon converted it,
built it and ran it, and it was kept only if the two did the same: both
built an image or neither did, and both printed the same. So every
output recorded here is the reference's, apart from three that differ
by design (listed in the tool): what the VM says about itself, and how
`hexicon` reports an image it cannot run. A program that never finishes
is built but not run.

A difference means Hexicon now does something else. If that is meant,
`--accept` records it; look at what changed first (the new results are
in `build/program-check`).
