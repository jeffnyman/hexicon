# System

Hexicon's system library: what every program is built on, below any library such as Hexad.

- `system.toml`: the library's manifest. A game names it among its libraries as `system`.
- `_main.hxn`: the program's entry point, the basic exception classes, and the framework that initializes a program's modules.
- `file.hxn`, `tok.hxn`, `gramprod.hxn`, `multmeth.hxn`: files, the tokenizer, grammar productions, and multi-methods.
- `dynfunc.hxn`: functions compiled while a program runs. `reflect.hxn`: what a program can find out about itself.
- The headers (`.h`): the virtual machine's built-in functions and classes, as a program sees them. `hexicon.h` brings in the rest: `vm.h` (the virtual machine's own functions), `general.h`, `io.h`, `systype.h` (the built-in classes, among them `ProgramObject`), and the others a program includes as it needs them.

The compiler has all of this built in, as it has Hexad. A build finds it here with nothing else installed. A game can have a system library of its own: a library named `system` in a folder `-Fs` or `HEXICON_LIB` names comes before the one built in.

## What is still the virtual machine's

The virtual machine binds a program to its built-in functions and classes by identifiers (`'hexicon-gen/030008'`, `'hexicon-object/030005'`), and finds some of the program's classes by the names they are exported under (`StackInfo`, `RuntimeError`). A header that declares a function set gives its functions their names. The identifier and version are what the machine binds to.
