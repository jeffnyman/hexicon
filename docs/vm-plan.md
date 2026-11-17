# Hexicon VM Plan

Status: draft, 2026-11-17

## Goal

Build the Hexicon virtual machine in Rust. The first milestone target is running unmodified TADS 3 game images (`.t3` files produced by the stock `t3make` compiler) with output identical to the reference `t3run` interpreter, natively and in the browser via WebAssembly.

The Hexicon compiler comes later. Until it exists, all test programs are compiled with the reference TADS 3 compiler.

## Scope

In scope for the VM:

- T3 image loading (all block types defined in the T3 spec)
- The bytecode interpreter (full T3 instruction set)
- Object model: TadsObject, multiple inheritance, property lookup, `modify`/`replace` results as they appear in images
- Core intrinsic classes: String, List, Vector, LookupTable, WeakRefLookupTable, BigNumber, ByteArray, StringBuffer, Iterator, IndexedIterator, AnonFuncPtr, Dictionary, GrammarProduction, StringComparator, CharacterSet, RexPattern, IntrinsicClass, IntClassModifier, StackFrameDesc/Ref, File, FileName, TemporaryFile, Date, TimeZone
- Intrinsic function sets: `t3vm`, `tads-gen`, `tads-io`, `t3test`, regex
- Garbage collection, including weak references and finalizers
- Undo
- Save and restore
- Console output formatting (the `vmconsol` behavior: tag handling, word wrap, paging hooks)

Deferred:

- `tads-net`, HttpServer, HttpRequest, and the WebUI (replaced later by a native browser front end)
- DynamicFunc (needs a compiler at runtime; revisit once the Hexicon compiler exists)
- The debugger protocol
- HTML TADS rich output (banners, images, sound). The host interface must leave room for it, but the first front ends are plain text.

Out of scope:

- TADS 2 games

## Reference material

All in `../_tads/tads-sources`:

| What | Where |
|---|---|
| T3 VM spec (image format, opcodes, machine model, save format) | `t3doc/techman/t3spec/*.htm` |
| Reference VM source | `tads3/vm*.cpp`, `tads3/vm*.h` |
| Interpreter loop | `tads3/vmrun.cpp` |
| Image loader | `tads3/vmimage.cpp` |
| Metaclass registry and version strings | `tads3/vmmccore.h`, `tads3/vm*.h` |
| Test programs and expected logs | `tads3/test/data` (381 `.t` files), `tads3/test/log` (161 logs) |
| Libraries for game-level tests | `tads3/lib/adv3`, plus adv3Lite (separate download) |

The reference binaries (`t3make`, `t3run`) are built from `../_tads/tads-runner` (captain-mayhem's 64-bit, CMake-based update of the same sources). The original `tads-sources` tree remains the place to read the code and the home of the test suite. `tools/reference/build.bat` builds them, along with the original test suite programs (`test_exec`, `test_tok`, `test_prs_top`, `t3pre`, `test_obj`), into `build/reference/bin`. `tools/reference/run-suite.sh` runs the original suite against them.

Oracle validation (2026-11-17): 158 of 162 comparisons reproduce the original logs, including the three sample games and a complete 438-move playthrough of Return to Ditch Day. The 4 that do not are `datefmt`, `datelocale`, `datesave_s`, and `datesave_r`: the logs were recorded in US Pacific time, and `datelocale.t`, `datesave.t`, and their logs also exist only as UTF-8 re-encoded copies.

Two fixtures are reconstructed by `run-suite.sh`, because no known copy of the suite (`tads-sources`, `tads-runner`, FrobTADS) has them intact:

- `hashes.src` survives only with LF line endings. Converting it to CRLF reproduces the expected MD5 and SHA-256 values.
- `dirtest-file-a.txt`, `-b.txt`, and `-c.txt` are missing. The test only lists them by name and size, so stand-ins of the logged sizes (2,250, 1,281, and 1,087 bytes) are generated.

The Ditch Day test is a reconstruction, because the original harness and command script (`m_walkthru.cmd`) are not available. The game is built from `../_tads/return-to-ditch-day-src` in QA test mode, the commands are recovered from the prompts in the expected log, and only the play transcript is compared (the source distribution has no `gameinfo.txt`, so the build listing differs).

Neither checkout has the suite intact: `tads-sources` has the newest tests but re-encoded and whitespace-trimmed text and no binary fixtures, while `tads-runner` has an older suite with the original bytes. `run-suite.sh` merges the two.

The reference VM is the specification of record wherever the spec documents are silent or disagree with it.

The test suite license allows using it for testing but requires redistributing it complete and unchanged. We do not vendor it; the test harness locates it on disk or fetches it.

## Workspace layout

```
hexicon/
  Cargo.toml              workspace
  crates/
    hexicon-image/        .t3 image parsing and validation; no VM dependency
    hexicon-vm/           values, object table, interpreter, metaclasses, GC, undo, save/restore
    hexicon-host/         the host interface trait and shared host types
    hexicon-cli/          native terminal runner: `hexicon run`, `hexicon dump`
    hexicon-wasm/         wasm-bindgen bindings for browsers
  tools/
    conformance/          runs the TADS 3 test suite and transcript diffs
  web/                    minimal browser front end used for testing the WASM build
  docs/
```

`hexicon-vm` has no direct access to files, terminals, clocks, or randomness. Everything external goes through `hexicon-host`, which keeps the VM portable to WASM and makes runs deterministic under test.

## Key design decisions

### Values

A Rust enum mirroring `vm_val_t`: nil, true, empty, integer, object, property ID, constant string, constant list, code offset, function pointer, enumerator, built-in function pointer, and the remaining T3 types. Values are `Copy` and small (target: 16 bytes).

### Objects and memory

- An object table indexed by T3 object ID, as in the reference VM. Object IDs are visible to programs and persist in save files, so allocation must follow the same rules.
- Objects are stored in the table and referenced by ID, not by `Rc`. This fits Rust well and avoids reference cycles.
- Mark-and-sweep garbage collection over the table, with weak-reference handling and finalizer queues matching the reference behavior.
- Metaclass dispatch through an enum of object kinds rather than trait objects, so save/restore and undo can match on concrete types.

### Resumable interpreter

This is the most important decision for WASM. In a browser, the VM cannot block a thread waiting for the player to type.

- Bytecode frames live on an explicit VM stack, not the Rust call stack.
- When the program calls an input function, the interpreter returns `NeedsInput` to the host. The host resumes it when input arrives.
- Some native methods call back into bytecode (sort comparators, `forEach`, `mapAll`). Those callbacks run nested. Requesting input from inside such a callback is an error at first; if real games turn out to need it, we handle it then.

This keeps one code path for native and browser builds, with no Asyncify, no SharedArrayBuffer, and no special server headers.

### Strings

UTF-8 internally, as in the reference VM, with character-index operations done the same way so results match exactly.

### Regular expressions

T3 has its own regex dialect (`<alpha>`, `<nocase>`, `%1` groups, and so on). We port the reference engine's semantics; we do not use the `regex` crate.

### Numbers

BigNumber is a port of the reference decimal implementation (`vmbignum.cpp`, `vmbignumlib.cpp`), not a general-purpose crate, so rounding and formatting match exactly.

### Save files

Goal: read and write save files compatible with the reference interpreter for the same game image. Fixing the in-memory model early keeps this achievable. If exact compatibility proves too costly, we fall back to a Hexicon save format and document it.

### Determinism

The host provides randomness and time. Under test, both are fixed, so transcripts are reproducible across runs and platforms.

## Testing

1. **Unit tests** in each crate.
2. **Image corpus.** The parser and disassembler must load every `.t3` we can find (compiled tests, adv3 samples, IFDB games) without error.
3. **Conformance suite.** Compile the reference test programs with the reference `t3make` once, then run them under Hexicon and compare byte for byte with the reference logs. The pass count is the main progress metric.
4. **Transcript diffs.** Play real games under both `t3run` and Hexicon with the same input script and seed, and diff the output. This covers adv3, adv3Lite, and a growing list of published games.
5. **WASM parity.** Run the same conformance and transcript tests against the WASM build in CI (Node or a headless browser).

Building the reference toolchain natively (`t3make`, `t3run`) is a prerequisite and is part of milestone 0.

## Milestones

| # | Milestone | Done when |
|---|---|---|
| 0 | Foundations | Cargo workspace, CI (fmt, clippy, test, `wasm32` build), reference `t3make`/`t3run` built by script, test programs compiled to images |
| 1 | Image loader and disassembler | `hexicon dump` loads and disassembles every image in the corpus |
| 2 | Core interpreter | Full opcode set, TadsObject, inheritance, exceptions, `t3vm` functions, constant strings and lists; simple test programs pass |
| 3 | Core intrinsic classes | String, List, Vector, LookupTable, BigNumber, ByteArray, StringBuffer, iterators, anonymous functions, regex; most of the conformance suite passes |
| 4 | Runtime services | GC, undo, save/restore, Dictionary, GrammarProduction, StringComparator; an adv3 sample game plays to completion |
| 5 | I/O | Console formatting, File, FileName, Date, TimeZone; transcript diffs match on adv3 and adv3Lite games |
| 6 | WebAssembly | Browser build plays the same games with identical transcripts |
| 7 | Hardening | Corpus of published games passes transcript diffs; performance comparable to `t3run`; fuzzing the image loader |

## Open questions

- Is read/write compatibility with existing `.t3v` save files a hard requirement or a nice-to-have?
- Should HTML TADS games (banners, images, sound) be supported in the first browser front end, or text only first?
- Minimum supported Rust version policy (proposal: latest stable minus two releases).
- License for Hexicon itself (GPLv2-or-later vs. GPLv3).
