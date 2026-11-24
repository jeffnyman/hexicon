# Hexicon Vocabulary

Status: draft for discussion, 2026-11-24

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

In the tables, "Uses" is the count of each name in Hexad, the system library, and the test programs, as a measure of the ripple. Status is **proposed** until decided.

## 1. Messages

The compiler's messages, and the VM's error messages, in the words an author uses. `wording.rs` already rewords the common ones; these are words to use, and to stop using, across all of them.

| Now | Proposed | Example | Status |
|---|---|---|---|
| symbol | name | "undefined symbol "x"" becomes ""x" is not defined anywhere" (done for the common ones) | proposed |
| intrinsic class | built-in class | "intrinsic class superclass "X" is not an intrinsic class" | proposed |
| intrinsic list, intrinsic name | built-in function set | in errors about `intrinsic` declarations | proposed |
| lvalue | something that can be assigned to | "invalid lvalue - cannot assign to expression on left of "="" becomes "can't assign to this: only a variable, property, or list element can be given a value" | proposed |
| formal parameter list | parameter list | "expected a symbol in formal parameter list" | proposed |
| property pointer | property reference (`&name`) | the guide, and messages about `&` | proposed |
| object file | (gone) | done in the break | done |

## 2. Built-in classes

| Now | Proposed | Uses | Notes | Status |
|---|---|---|---|---|
| `LookupTable` | `Map` | 106 | | proposed |
| `WeakRefLookupTable` | `WeakMap` | 4 | | proposed |
| `LookupTableIterator` | `MapIterator` | few | | proposed |
| `StringBuffer` | `StringBuilder` | 21 | C#, Java, and Kotlin's name for the same thing | proposed |
| `RexPattern` | `Regex` | 73 | | proposed |
| `AnonFuncPtr` | `Function` | 5 | the class of a function value | proposed |
| `DynamicFunc` | `CompiledCode` | 18 | code compiled while the game runs; open to a better name | proposed |
| `GrammarProd` | `GrammarRule` | 39 | matches the guide's word, and `verb` declarations | proposed |
| `IntrinsicClass` | `BuiltinClass` | 8 | | proposed |
| `IntrinsicClassModifier` | `BuiltinClassModifier` | few | | proposed |
| `StackFrameDesc`, `StackFrameRef` | `StackFrame`, `StackFrameRef` | few | | proposed |
| `Vector` | keep | | a growable list; the name is standard (C++, Rust) | keep |
| `List`, `String`, `Object`, `Collection`, `Iterator`, `Date`, `TimeZone`, `File`, `FileName`, `TemporaryFile`, `ByteArray`, `BigNumber`, `CharacterSet`, `Dictionary`, `ProgramObject` | keep | | already plain | keep |

`BigNumber` could be `Decimal` (C#, Python), which says what it is: a number with exact decimal digits. Open.

## 3. Built-in functions

Functions of `hexicon-gen` and `hexicon-io`, and methods of the built-in classes. The `vm` functions were renamed in milestone 0 (`collectGarbage`, `namedArg`, and the rest).

| Now | Proposed | Uses | Notes | Status |
|---|---|---|---|---|
| `dataType(x)` | `typeOf(x)` | 195 | and `TypeSString`/`TypeDString`/`TypeFuncPtr`/`TypeBifPtr` become `TypeString`/`TypePrintString`/`TypeFunction`/`TypeBuiltin` | proposed |
| `firstObj`, `nextObj` | keep for now; add an `objects(cls)` loop later | 88 each | a new form is a language change, not a rename | open |
| `rexMatch`, `rexSearch`, `rexSearchLast`, `rexGroup`, `rexReplace` | `regexMatch`, `regexSearch`, `regexSearchLast`, `regexGroup`, `regexReplace` | 320 | | proposed |
| `toString`, `toInteger`, `toNumber` | keep | | | keep |
| `makeString`, `makeList` | `repeatString`, `repeatList` | few | they repeat a value | proposed |
| `sgn` | `sign` | few | | proposed |
| `getTime` | keep | | | keep |
| `savepoint`, `undo` | keep | | | keep |
| `getFuncParams` | `functionParams` | few | | proposed |
| `ofKind` | keep, or `isA` | 368 | `ofKind` is clear in an IF sentence (`obj.ofKind(Door)`); `isA` is shorter | open |
| `propDefined`, `propType`, `getPropList`, `getPropParams`, `propInherited` | `hasProp`, `propType`, `propList`, `propParams`, `propInherited` | 110 | | proposed |
| `getSuperclassList`, `setSuperclassList` | `superclasses`, `setSuperclasses` | few | | proposed |
| `createInstance`, `createClone`, `createTransientInstance` | keep | | `new X` is the usual way; these are for code that builds objects by class | keep |
| `getNext`, `isNextAvailable`, `resetIterator`, `getCurKey`, `getCurVal` | `next`, `hasNext`, `reset`, `key`, `value` | few | | proposed |
| `htmlify` | `escapeHtml` | 15 | | proposed |
| `specialsToHtml`, `specialsToText` | keep | | | open |
| `inputLine`, `inputKey`, `inputEvent` | keep | | | keep |
| `banner*` (16 functions) | `window*` | 69 | they make and change the extra windows of the display; "banner" is a term of art | open |
| `morePrompt` | `pausePrompt` | few | | proposed |
| `get` and `getX` generally | drop `get` where nothing is set | | the rule behind several rows above | proposed |

## 4. The language's own words

These change how every source looks, so each is its own decision. The counts are why.

| Now | Possible | Uses | Notes | Status |
|---|---|---|---|---|
| `nil` | `null` | 4,340 | the largest change on this page. `nil` is also Ruby's, Lua's, Go's, and Swift's, so it is not unfamiliar; it means both "no value" and false, which `null` does not say | open, leaning keep |
| `local` | `var` or `let` | 3,197 | `local` is Lua's word too and reads plainly | open, leaning keep |
| `intrinsic` | `builtin` or `native` | 316 | only the system library's headers write it | proposed: `builtin` |
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

## Open questions

1. `nil` and `local`: keep, as leaning above, or change with the rest?
2. `BigNumber` or `Decimal`; `banner` or `window`; `ofKind` or `isA`.
3. Whether section 3's `get` rule is applied generally, or only to the names listed.
