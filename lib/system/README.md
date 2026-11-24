# System

Hexicon's system library: what every program is built on, below any library such as Hexad.

- `system.toml`: the library's manifest. A game names it among its libraries as `system`.
- `_main.t`: the program's entry point, the basic exception classes, and the framework that initializes a program's modules.
- `file.t`, `tok.t`, `gramprod.t`, `multmeth.t`: files, the tokenizer, grammar productions, and multi-methods.
- `dynfunc.t`: functions compiled while a program runs.
- The headers (`.h`): the virtual machine's built-in functions and classes, as a program sees them. `hexicon.h` gives the built-in functions Hexicon's names.

The compiler has all of this built in, as it has Hexad. A build looks for the system library and its headers where it is told to (`-FL` and `-FI`, or `lib` and `include` beside the program), and then here, so a game builds with nothing else installed.

## What comes next

The files' names, and many names within them (`tads.h`, `TadsObject`), are older than Hexicon's own. They are to be renamed, as the built-in functions have been (`hexicon.h`). Programs built on these files are the same as before until then, which is how the change is checked.
