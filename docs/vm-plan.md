# Hexicon VM Plan

Status: draft, 2026-11-18

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
    hexicon-bytecode/     the instruction set, decoder, and disassembler
    hexicon-vm/           values, object table, interpreter, metaclasses, GC, undo, save/restore
    hexicon-host/         the host interface trait and shared host types
    hexicon-cli/          native terminal runner: `hexicon run`, `hexicon dump`, `hexicon disasm`
    hexicon-wasm/         wasm-bindgen bindings for browsers (not started)
  tools/
    reference/            builds the reference toolchain and runs its test suite
    conformance/          runs the suite's programs under Hexicon and the reference, and compares
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

### Identity

Hexicon identifies itself as Hexicon. `t3GetVMID` returns `hexicon`, `t3GetVMBanner` returns `Hexicon VM` and the version, and `t3GetVMVsn` returns Hexicon's own version number, not the reference VM's `mjr-T3` and 3.1.3. The number keeps the T3 layout (major in the high 16 bits, then minor and patch in 8 bits each) because libraries take it apart to display it. Neither adv3 nor the system library compares the number against anything; adv3 only shows it for the VERSION command. A transcript that includes VERSION output therefore differs from the reference on that line, by design.

### Save files

Goal: read and write save files compatible with the reference interpreter for the same game image. Fixing the in-memory model early keeps this achievable. If exact compatibility proves too costly, we fall back to a Hexicon save format and document it.

### Determinism

The host provides randomness and time. Under test, both are fixed, so transcripts are reproducible across runs and platforms.

## Testing

1. **Unit tests** in each crate.
2. **Image corpus.** The parser and disassembler must load every `.t3` we can find (compiled tests, adv3 samples, IFDB games) without error.
3. **Conformance suite.** Compile the reference test programs with the reference `t3make` once, then run each image under both Hexicon and the reference interpreter and compare the output byte for byte. The pass count is the main progress metric. `tools/conformance/run.sh` does this; it needs the work directory `tools/reference/run-suite.sh` leaves behind. Both interpreters write UTF-8, so the comparison does not depend on a character set mapping.
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

### Status

- Milestone 0: done except CI, which waits on a hosted repository.
- Milestone 1: done (2026-11-17). `hexicon dump` and `hexicon disasm` handle all 138 images in the corpus, and the disassembly accounts for every byte of every code pool. The corpus is the images the reference suite compiles, the release and debug builds of Return to Ditch Day, and the `tads-runner` test games; set `HEXICON_T3_CORPUS` to run the corpus tests.

- Milestone 2: done (2026-11-18). The interpreter runs the full instruction set, with program-defined objects, multiple inheritance, `inherited` and `delegated`, operator overloading, exceptions, named arguments, anonymous functions, `foreach`, and the `t3vm` function set. 41 of the suite's 132 runnable programs produce output identical to the reference interpreter's; the other 91 stop at something a later milestone provides, and say what. Three things in this milestone's scope are deferred because they need intrinsic classes from milestone 3: promoting an overflowing integer to BigNumber, `t3GetGlobalSymbols` for images with debug symbols, and local variables and frame references in stack traces.
- Part of milestone 5 was done early, because without it even the simplest test program does not match: the console formatter (space and newline collapsing, tabs, capitalization, line wrapping, and the HTML subset a text console understands). Still to do there: input, log files, the status line, banners, and paging.

Findings from milestone 2 that later milestones depend on:

- The stack limit is observable. The reference VM allows 4,096 slots and spends 11 on every call; Hexicon keeps frames elsewhere but counts the same way, and a runaway recursion stops at the same depth under both (checked at 336 and 252 levels for two different functions).
- A constructor returns `self`. The `new` instructions rely on that to deliver the object; the VM only supplies it when a class has no constructor.
- An exception table's ranges are tested against a caller's return address, not the address of its call. The compiler makes each range run past the call.
- Catching an exception does not trim the stack. Whatever the function had pushed stays until it returns.
- A return to an address holding `getr0` performs the `getr0` as part of the return.
- The `inc` instruction does not check for overflow, though `dec` and the others do.
- When the VM creates a `RuntimeError`, it reads each message parameter's type from the slot before it, so the constructor only receives a parameter intact if it has the same type as its predecessor.
- Object IDs for new objects come off a free list that starts at the top of the newest 4,096-entry page. Hexicon reproduces the order, but not yet the exact sequence of temporary objects the reference VM allocates, so IDs can still differ. Where that turns out to be visible (the order of a LookupTable keyed by objects, for one), garbage collection timing will matter too.
- Intrinsic class methods are found through the property lists in the image's metaclass table, and an intrinsic class inherits the methods of the one it derives from (a list has `createIterator` from Collection).
- The console drops newlines that follow a newline, and anything that would start the output with a blank line. Blank lines have to be asked for.

Findings from milestone 1 that later milestones depend on:

- A method header gives no code length. A function's extent comes from its exception table and debug records when it has them, and otherwise from following its branches and from where the next function starts.
- Release builds carry debug records too, for any function with local variables. The frame tables in them are what `t3GetStackTrace` and the reflection library read.
- Pre-initialized images keep references to static initializer code that pre-initialization discarded. The VM must not treat these as errors at load time.
- The reference compiler emits a `switch` whose default branch points into the middle of an instruction (`rand.t` in the suite). The default is never taken. The VM must not validate branch targets ahead of execution.
- Ten opcodes in the reference interpreter are missing from the instruction set document: `iternext`, `getsetlcl1`, `getsetlcl1r0`, `dupr0`, and `getlcln0` through `getlcln5`.

## Open questions

- Is read/write compatibility with existing `.t3v` save files a hard requirement or a nice-to-have?
- Should HTML TADS games (banners, images, sound) be supported in the first browser front end, or text only first?
- Minimum supported Rust version policy (proposal: latest stable minus two releases).
- License for Hexicon itself (GPLv2-or-later vs. GPLv3).
