# System

Hexicon's system library: what every program is built on, below any library such as Hexad.

- `system.toml`: the library's manifest. A game names it among its libraries as `system`.
- `_main.hxn`: the program's entry point, the basic exception classes, and the framework that initializes a program's modules.
- `file.hxn`, `tok.hxn`, `gramprod.hxn`, `multmeth.hxn`: files, the tokenizer, grammar productions, and multi-methods.
- `dynfunc.hxn`: functions compiled while a program runs. `reflect.hxn`: what a program can find out about itself.
- The headers (`.h`): the virtual machine's built-in functions and classes, as a program sees them. `hexicon.h` brings in the rest: `vm.h` (the virtual machine's own functions), `general.h`, `io.h`, `systype.h` (the built-in classes, among them `ProgramObject`), and the others a program includes as it needs them.

The compiler has all of this built in, as it has Hexad. A build looks for the system library and its headers where it is told to (`-FL` and `-FI`, or `lib` and `include` beside the program), and then here, so a game builds with nothing else installed.

## What is still the virtual machine's

The virtual machine binds a program to its built-in functions and classes by identifiers (`'tads-gen/030008'`, `'tads-object/030005'`), and finds some of the program's classes by the names they are exported under (`StackInfo` as `'T3StackInfo'`). Those are the machine's, not the library's, and stay until it goes its own way.
