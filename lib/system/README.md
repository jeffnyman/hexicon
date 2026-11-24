# System

Hexicon's system library: what every program is built on, below any library such as Hexad.

- `system.toml`: the library's manifest. A game names it among its libraries as `system`.
- `_main.t`: the program's entry point, the basic exception classes, and the framework that initializes a program's modules.
- `file.t`, `tok.t`, `gramprod.t`, `multmeth.t`: files, the tokenizer, grammar productions, and multi-methods.
- `dynfunc.t`: functions compiled while a program runs.
- The headers (`.h`): the virtual machine's built-in functions and classes, as a program sees them. `hexicon.h` gives the built-in functions Hexicon's names.

The compiler has all of this built in. A build looks for the system library and its headers where it is told to (`-FL` and `-FI`, or `lib` and `include` beside the program), and then here, so a game builds with nothing else installed.

## Where it comes from

These files are derived from TADS 3, by Michael J. Roberts, and are part of Hexicon with his permission. Each keeps its copyright notice. For now they are as TADS 3 has them, apart from the line in each that says what it is part of, so that programs built on them are the same as they were. Their names and the names within them are to become Hexicon's own, as the rest of Hexicon has.
