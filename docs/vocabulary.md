# Hexicon Vocabulary

Status: decided, 2026-11-24. Messages, built-in classes, built-in functions, and the `builtin` keyword done. Hexad's words are next, with Hexad's design.

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

Kept: "metaclass" in the messages about a damaged image file, and in `hexicon dump`'s listings, where it names a part of the image format. Messages quote the keyword as it is written, `"builtin class"` since section 4.

## 2. Built-in classes

Done (2026-11-24), in the system library, Hexad, the test programs, the guide, and the game tests. The headers that declare them are renamed with them: `map.h`, `decimal.h`, `stringbuilder.h`, `compiledcode.h` (and `.hxn`), and `grammarrule.h` (and `.hxn`). So are the names the VM finds in an image's exports (`GrammarRule.tokenList`, `CompiledCode.CompilerException`), and `BigNumber`'s formatting flags (`DecimalPoint`, `DecimalExp`, and the rest). The `[key -> value]` list makes a `Map`, and its messages say "a [key -> value] list".

The identifiers in the image (`'lookuptable/030003'`, `'bignumber/030001'`) are unchanged; they are part of the image format. The compiler's and the VM's own Rust code followed later the same day: its types, modules, and comments use these names and "built-in" too (`Object::Map`, `map.rs`, `decimal_obj.rs`, `string_builder.rs`, `compiled_code.rs`), and the decimal arithmetic crate is `hexicon-decimal`. A program's properties can be numbered differently after a rename, since the compiler numbers some of them in the order of their names; what a program does is the same.

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

Done (2026-11-24), in the same places as section 2. The `vm` functions were renamed earlier, in milestone 0 (`collectGarbage`, `namedArg`, and the rest).

### What a name costs

A built-in function, or a method of a built-in class, takes its name for the whole program. A game cannot have an object called `length` or `abs`:

```
error: "length" is already the name of something else, so it cannot also name an object
```

So a short noun is a poor name for a built-in: dropping `get` everywhere, as first decided, would have taken `key`, `path`, `floor`, `scale`, `body`, `cookie`, `date`, `name`, `value`, and `next` from authors, and `sgn` to `sign` would have taken the most common object in interactive fiction. Part of what `get` does is keep the built-ins out of the way. The rule became:

- **A class drops `get` only if none of its names would become one a game might want for an object.** A class keeps `get` throughout otherwise, so that no class is half one way and half the other.
- **No new name is a keyword** (`getSelf` and `getMethod` would have been `self` and `method`) **or an existing name** (`getAbs` would have been the function `abs`; `getGrammarInfo` the property `grammarInfo`, which grammar matches already define).
- **An `Object` method is checked against Hexad's own properties too,** since every object has them: Hexad's signals extension has a `propList` property, which would have hidden the method on its objects, so `getPropList` became `properties`.

### Renamed

| Was | Now | Status |
|---|---|---|
| `dataType(x)` | `typeOf(x)` | done |
| `TypeSString`, `TypeDString`, `TypeFuncPtr`, `TypeBifPtr` | `TypeString`, `TypePrintString`, `TypeFunction`, `TypeBuiltin` | done |
| `rexMatch`, `rexSearch`, `rexSearchLast`, `rexGroup`, `rexReplace` | `regexMatch`, `regexSearch`, `regexSearchLast`, `regexGroup`, `regexReplace` | done |
| `makeString`, `makeList` | `repeatString`, `repeatList` | done |
| `getArg`, `getFuncParams`, `getLocalCharSet` | `arg`, `functionParams`, `localCharSet` | done |
| `getHostName`, `getLocalIP`, `getLaunchHostAddr`, `getNetStorageURL`, `getNetEvent` | `hostName`, `localIP`, `launchHostAddr`, `netStorageURL`, `netEvent` | done |
| `morePrompt` | `pausePrompt` | done |
| `banner*` (`bannerCreate`, `bannerSay`, `bannerGetInfo`, ...) and the `Banner*` constants | `pane*` (`paneCreate`, `paneSay`, `paneInfo`, ...) and `Pane*` | done |
| `propDefined`, `getPropList`, `getPropParams` | `hasProp`, `properties`, `propParams` | done |
| `getSuperclassList`, `setSuperclassList` | `superclasses`, `setSuperclasses` | done |
| `isNextAvailable` | `hasNext` | done |
| `htmlify` and the `Htmlify*` flags | `escapeHtml` and `EscapeHtml*` | done |
| `Map`'s `getBucketCount`, `getEntryCount`, `getDefaultValue` | `bucketCount`, `entryCount`, `defaultValue` | done |
| `Regex`'s `getPatternString` | `patternString` | done |

Hexad's `BannerWindow` and its other banner classes are Hexad's own names, decided with Hexad (section 5).

### Kept

| Name | Why |
|---|---|
| `sgn` | `sign` would take the sign |
| `Iterator`'s `getNext`, `getCurKey`, `getCurVal`, `resetIterator` | `next`, `key`, `value`, `reset` |
| `Decimal`'s `get*` (`getAbs`, `getFloor`, `getPi`, ...) | `abs` is a function; `floor`, `scale`, `whole` are objects |
| `Date`'s and `TimeZone`'s `get*` | `date`, `location`, `history`, `rules` |
| `File`'s and `FileName`'s `get*` | `path`, `name`, `pos` |
| `HTTPRequest`'s and `HTTPServer`'s `get*` | `body`, `cookie`, `server`, `address` |
| `StackFrameRef`'s `get*` | `getSelf` cannot be `self` |
| `ProgramObject`'s `getMethod`, `setMethod` | `method` is a keyword |
| `getUnique`, `CharacterSet`'s `getName`, `CompiledCode`'s `getSource`, `GrammarRule`'s `getGrammarInfo` | `unique` is an adjective; `name`, `source`; `grammarInfo` is taken |
| `getTime`, `firstObj`, `nextObj`, `toString`, `toInteger`, `toNumber`, `savepoint`, `undo`, `ofKind`, `propType`, `propInherited`, `createInstance`, `createClone`, `createTransientInstance`, `specialsToHtml`, `specialsToText`, `inputLine`, `inputKey`, `inputEvent` | already plain, as decided |

A change to the language could remove the cost itself: if an object's name and a method's name did not share one namespace, as in most languages, the short names would be free to use. That is not planned; it is noted here so that a later change does not close it off.

## 4. The language's own words

These change how every source looks, so each is its own decision. The counts are why.

| Now | Possible | Uses | Notes | Status |
|---|---|---|---|---|
| `nil` | keep | 4,340 | `nil` means both "no value" and false (`isOpen = nil`); `null` would say unknown rather than false, and need a separate `false`, a change of meaning and not of spelling. Ruby, Lua, Go, and Swift have `nil` too | keep |
| `local` | keep | 3,197 | plain English, and Lua's word too | keep |
| `intrinsic` | `builtin` | 316 | only the system library's headers write it. Done (2026-11-24): `builtin 'hexicon-gen/030008' { ... }` and `builtin class Map 'lookuptable/030003': Collection { ... }`; comments that said "intrinsic" say "built-in". The image never held the keyword, so no image changed | done |
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
3. The `get` rule applies to every built-in function and method, class by class, where it takes no name a game would want (section 3). Hexad's own methods wait for Hexad's design.
