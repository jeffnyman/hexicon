# System

Hexicon's system library: what every program is built on, below any library such as Hexad.

- `system.toml`: the library's manifest. A game names it among its libraries as `system`.
- `lifecycle.hxn`: the program's entry point, `_main`, which initializes the program and calls its `main(args)`, and again after each RESTART; the default output; and what is done as the program ends.
- `initobjects.hxn`: the objects that run before the program is saved as an image (`PreinitObject`) and each time it starts (`InitObject`).
- `exceptions.hxn`: the exception classes, the run-time errors the virtual machine throws, and stack traces.
- `utilities.hxn`: going through every instance of a class, and what the compiler's `<<one of>>` lists use.
- `file.hxn`, `tok.hxn`, `grammarrule.hxn`, `multmeth.hxn`: files, the tokenizer, grammar productions, and multi-methods.
- `compiledcode.hxn`: functions compiled while a program runs. `reflect.hxn`: what a program can find out about itself.
- The headers (`.h`): the virtual machine's built-in functions and classes, as a program sees them. `hexicon.h` brings in the rest: `vm.h` (the virtual machine's own functions), `general.h`, `io.h`, `systype.h` (the built-in classes, among them `ProgramObject`), and the others a program includes as it needs them.

The compiler has all of this built in, as it has Hexad. A build finds it here with nothing else installed. A game built on it is its author's to license as they choose: see [LIBRARY-EXCEPTION](../../LIBRARY-EXCEPTION). A game can have a system library of its own: a library named `system` in a folder `-Fs` or `HEXICON_LIB` names comes before the one built in.

## What is still the virtual machine's

The virtual machine binds a program to its built-in functions and classes by identifiers (`'hexicon-gen/030008'`, `'hexicon-object/030005'`), and finds some of the program's classes by the names they are exported under (`StackInfo`, `RuntimeError`). A header that declares a function set gives its functions their names. The identifier and version are what the machine binds to.
