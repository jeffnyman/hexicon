# Hexicon Vocabulary

Status: decided, 2026-11-24. Messages and built-in classes done; the rest to come in the order below.

## Goal

Hexicon's words should be ones an author today recognizes on sight. Many of the names it inherited are real concepts with dated or specialist names: `LookupTable` for what most languages call a map, `intrinsic` for built-in, "symbol" for name. This page decides each word once, so the renames can then be made in chunks without deciding again.

Nothing here changes what a program does. A name in source is only a name: the image binds the built-in classes and functions by their identifiers (`'lookuptable/030003'`, `hexicon-gen/030008`), not by what a header calls them. So every rename below is a change to headers, Hexad, the tests, and the docs, with no change to the VM or the image format.

## How a word is chosen

- **The word an author already knows wins**, from the languages most authors have met: JavaScript, Python, C#, Swift, Kotlin. `Map` over `LookupTable`, `StringBuilder` over `StringBuffer`.
- **Plain over technical.** "name" over "symbol", "built-in" over "intrinsic", "can't assign to" over "invalid lvalue". A word should explain itself to someone who has never read a compiler book.
- **Interactive fiction's own words stay** where they are already plain to its authors: room, thing, door, actor, the player.
- **No aliases.** A renamed word is renamed everywhere, in one chunk: Hexad, the system library, the tests, the guide. There are no old games to keep working and no second library, so nothing needs the old name.
- **A change of the language's look is decided separately** from a change of a name in a library. `nil`, `local`, and `intrinsic` are words every source is written in. A class or function name is only a name.

This revises "What earns a change" in [language-plan.md](language-plan.md), which keeps names a TADS 3 author would know and calls a change of spelling alone not worth the move. That was written while TADS 3 games were Hexicon's tests. Since the break, readability for new authors is reason enough.

## Order of the work

1. **Messages and the guide.** What authors meet first, and the cheapest: no source changes.
2. **Built-in classes and functions.** The system headers, then every use in Hexad and the tests.
3. **The language's own words**, if any are changed at all.
4. **Hexad's words**, decided with Hexad's design rather than here (see the last section).

In the tables, "Uses" is the count of each name in Hexad, the system library, and the test programs, as a measure of the ripple. Status is **decided** (to be made), **done**, or **keep**.

## 1. Messages

Done (2026-11-24). The compiler's messages and the VM's error messages use these words. `wording.rs` says the common ones again more fully, with notes.

| Was | Now | Example | Status |
|---|---|---|---|
| symbol | name | "symbol "x" is already defined" became "the name "x" is already defined"; "undefined symbol" became "undefined name", which `wording.rs` says as ""x" is not defined anywhere" | done |
| intrinsic class, intrinsic function | built-in class, built-in function | "the superclass "X" is not a built-in class" | done |
| intrinsic list, intrinsic name | the function set, the function set's name | "expected function name in the function set" | done |
| lvalue | (said as what can be assigned to) | "can't assign to the expression on the left of "="", noted "only a variable, a property, or an element of a list can be given a value"; "can't apply "++" to this expression" | done |
| formal parameter | parameter | "expected a comma in parameter list" | done |
| preprocessor symbol | macro, macro name | "library requires the macro "VOICE" to be defined" | done |
| property pointer | property reference (`&name`) | the guide, and the VM's "property reference required" | done |
| object file | (gone) | done in the break | done |

Kept: "metaclass" in the messages about a damaged image file, and in `hexicon dump`'s listings, where it names a part of the image format. The keyword is quoted as it is written (`"intrinsic class"`) until it changes (section 4).

## 2. Built-in classes

Done (2026-11-24), in the system library, Hexad, the test programs, the guide, and the game tests. The headers that declare them are renamed with them: `map.h`, `decimal.h`, `stringbuilder.h`, `compiledcode.h` (and `.hxn`), and `grammarrule.h` (and `.hxn`). So are the names the VM finds in an image's exports (`GrammarRule.tokenList`, `CompiledCode.CompilerException`), and `BigNumber`'s formatting flags (`DecimalPoint`, `DecimalExp`, and the rest). The `[key -> value]` list makes a `Map`, and its messages say "a [key -> value] list".

The identifiers in the image (`'lookuptable/030003'`, `'bignumber/030001'`) are unchanged, as is the VM's own Rust code, whose type names are its own business. A program's properties can be numbered differently after a rename, since the compiler numbers some of them in the order of their names; what a program does is the same.

| Was | Now | Uses | Notes | Status |
|---|---|---|---|---|
| `LookupTable` | `Map` | 106 | | done |
| `WeakRefLookupTable` | `WeakMap` | 4 | | done |
| `LookupTableIterator` | `MapIterator` | few | | done |
| `BigNumber` | `Decimal` | 76 | it gives exact decimal digits at a chosen precision, as `Decimal` does in C# and Python; "big number" suggests large whole numbers | done |
| `StringBuffer` | `StringBuilder` | 21 | C#, Java, and Kotlin's name for the same thing | done |
| `RexPattern` | `Regex` | 73 | | done |
| `AnonFuncPtr` | `Function` | 5 | the class of a function value | done |
| `DynamicFunc` | `CompiledCode` | 18 | code compiled while the game runs | done |
| `GrammarProd` | `GrammarRule` | 39 | matches the guide's word, and `verb` declarations | done |
| `IntrinsicClass` | `BuiltinClass` | 8 | | done |
| `IntrinsicClassModifier` | `BuiltinClassModifier` | few | | done |
| `StackFrameDesc`, `StackFrameRef` | `StackFrame`, `StackFrameRef` | few | | done |
| `Vector` | keep | | a growable list; the name is standard (C++, Rust) | keep |
| `List`, `String`, `Object`, `Collection`, `Iterator`, `Date`, `TimeZone`, `File`, `FileName`, `TemporaryFile`, `ByteArray`, `CharacterSet`, `Dictionary`, `ProgramObject` | keep | | already plain | keep |

## 3. Built-in functions

Functions of `hexicon-gen` and `hexicon-io`, and methods of the built-in classes. The `vm` functions were renamed in milestone 0 (`collectGarbage`, `namedArg`, and the rest).

| Now | Proposed | Uses | Notes | Status |
|---|---|---|---|---|
| `dataType(x)` | `typeOf(x)` | 195 | and `TypeSString`/`TypeDString`/`TypeFuncPtr`/`TypeBifPtr` become `TypeString`/`TypePrintString`/`TypeFunction`/`TypeBuiltin` | decided |
| `firstObj`, `nextObj` | keep for now | 88 each | an `objects(cls)` loop may come later, as a language change rather than a rename | keep |
| `rexMatch`, `rexSearch`, `rexSearchLast`, `rexGroup`, `rexReplace` | `regexMatch`, `regexSearch`, `regexSearchLast`, `regexGroup`, `regexReplace` | 320 | | decided |
| `toString`, `toInteger`, `toNumber` | keep | | | keep |
| `makeString`, `makeList` | `repeatString`, `repeatList` | few | they repeat a value | decided |
| `sgn` | `sign` | few | | decided |
| `getTime` | keep | | | keep |
| `savepoint`, `undo` | keep | | | keep |
| `getFuncParams` | `functionParams` | few | | decided |
| `ofKind` | keep | 368 | `obj.ofKind(Door)` reads as a sentence, and in Hexad an `is` name is a true-or-false property (`isOpen`), which `isA` would look like | keep |
| `propDefined`, `propType`, `getPropList`, `getPropParams`, `propInherited` | `hasProp`, `propType`, `propList`, `propParams`, `propInherited` | 110 | | decided |
| `getSuperclassList`, `setSuperclassList` | `superclasses`, `setSuperclasses` | few | | decided |
| `createInstance`, `createClone`, `createTransientInstance` | keep | | `new X` is the usual way; these are for code that builds objects by class | keep |
| `getNext`, `isNextAvailable`, `resetIterator`, `getCurKey`, `getCurVal` | `next`, `hasNext`, `reset`, `key`, `value` | few | | decided |
| `htmlify` | `escapeHtml` | 15 | | decided |
| `specialsToHtml`, `specialsToText` | keep | | | keep |
| `inputLine`, `inputKey`, `inputEvent` | keep | | | keep |
| `banner*` (16 functions) | `pane*` | 69 | they split the display into panes, such as the status line; "window" would clash with the browser's window | decided |
| `morePrompt` | `pausePrompt` | few | | decided |
| `get` and `getX` generally | drop `get` where what follows is a noun (`getArg` becomes `arg`, `getLocalCharSet` becomes `localCharSet`); a setter keeps `set` | | for every built-in; Hexad's own `get` methods are decided with Hexad | decided |

## 4. The language's own words

These change how every source looks, so each is its own decision. The counts are why.

| Now | Possible | Uses | Notes | Status |
|---|---|---|---|---|
| `nil` | keep | 4,340 | `nil` means both "no value" and false (`isOpen = nil`); `null` would say unknown rather than false, and need a separate `false`, a change of meaning and not of spelling. Ruby, Lua, Go, and Swift have `nil` too | keep |
| `local` | keep | 3,197 | plain English, and Lua's word too | keep |
| `intrinsic` | `builtin` | 316 | only the system library's headers write it | decided |
| `propertyset` | keep | 45 | Hexad writes it once; handlers (`dobj Take { }`) replaced its common use | keep |
| `targetprop`, `targetobj`, `definingobj`, `argcount` | keep | 33 | rare, and they say what they are | keep |
| `inherited`, `delegated` | keep | 607 | plain English | keep |
| `foreach` | keep | 338 | `for (x in list)` also works | keep |
| `self`, `modify`, `replace`, `template`, `object`, `class`, `extern`, `export`, `transient` | keep | | | keep |
| double-quoted strings that print | keep | | the heart of how a game is written | keep |

## 5. Hexad's words

Hexad's vocabulary is adv3Lite's, and it is what authors write most. Its words are decided with Hexad's design ([hexad-plan.md](hexad-plan.md)), not here, because many of them come with a change of how the library works. Candidates to take up there:

| Now | Uses | The question |
|---|---|---|
| `gActor`, `gDobj`, `gIobj`, `gPlayerChar` (the `g` macros) | 962 | plain names (`actor`, `dobj`, `player`) or a `cmd.` object |
| `dobj`, `iobj` | 1,249 | the parser's own words; keep, or "object" and "second object"? |
| `DMsg`, `BMsg` | 802 | one `msg(...)` form |
| `Doer` | 69 | a rule that replaces or redirects a command; `Rule` or `CommandRule`? |
| `verify`, `check`, `action`, `report` | | the four stages; plain already, but `verify` is unusual |

## Decided

1. `nil` and `local` stay (2026-11-24).
2. `BigNumber` becomes `Decimal`, the `banner` functions become `pane` functions, and `ofKind` stays.
3. The `get` rule applies to every built-in function and method. Hexad's own methods wait for Hexad's design.
