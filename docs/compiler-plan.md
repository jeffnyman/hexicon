# Hexicon Compiler Plan

Status: draft, 2026-11-19

## Goal

Build the Hexicon compiler in Rust: a program that turns source text into an image the Hexicon VM runs. The first target is compiling unmodified TADS 3 source, meaning a game's own files together with the library it is built on (adv3 or adv3Lite), into an image that plays the same as one built by the stock `t3make`.

The same compiler, built into the VM, is the dynamic compiler: the part that lets a running program compile code, which `DynamicFunc`, stack frame references, and `GrammarProd.addAlt` need. The ten conformance tests that still fail all wait for it.

Hexicon's own language comes after that, as changes to a compiler that already handles everything TADS 3 has.

## Scope

In scope:

- The preprocessor: comments, line splicing, strings that run over lines, `#include`, `#define` with arguments and variable arguments (`#foreach`, `#ifempty`, `#ifnempty`, `#argcount`), token pasting and stringizing, `#if` and its relatives, `#pragma`, `#line`, `#error`, `#charset`
- The tokenizer, including strings with embedded expressions (`<<...>>`) and their special forms (`<<if>>`, `<<one of>>`, `<<first time>>`, format specs)
- The parser for the whole language: expressions, statements, functions, objects and classes, `modify` and `replace`, templates, property sets, `grammar` rules, `dictionary`, `enum`, `intrinsic` and `intrinsic class`, `export`, anonymous functions and short-form anonymous functions, operator overloading
- Constant folding, as far as a program can tell that it happened
- Code generation to T3 byte code
- Symbol files and object files, or something that does their job: a library's sources are compiled separately and linked
- The linker and the image writer, including debug records
- Project files (`.t3m`) and library files (`.tl`)
- Preinitialization, which already exists: it is running the image in the VM and saving the result
- The dynamic compiler interface in the VM

Deferred:

- The JavaScript back end the reference has (`tcjs*`), which was never finished there
- Resource bundling (`t3res`), which is separate from compiling
- Hexicon's own language

## Principles

The VM plan's parity principle carries over. Where the reference compiler's behavior is deterministic and a program or its author can see it, Hexicon does the same thing, odd cases included. That covers what text a macro expands to, what a string with embedded expressions turns into, which errors and warnings are reported, and what the compiled program does.

It does not cover the bytes of the image. Two compilers can lay out an image differently and produce programs that behave identically, so image comparison is a tool, not a requirement. What is required is that the image runs the same.

The reference is the specification of record. Its source is in `../_tads/tads-runner/tads3` (`tc*.cpp`, `tc*.h`); the documents in `../_tads/tads-sources/t3doc` say what was intended.

## How each stage is checked

| Stage | Checked against |
|---|---|
| Preprocessor | The reference's preprocessed text (`test_tok -P`, `t3make -P`), line for line, over the suite's 12 preprocessor tests, every file of adv3 and adv3Lite, and the sources of the games in the playground that have them |
| Tokenizer | The reference's token listing (`test_tok`) over the same files |
| Parser | Parsing every file of both libraries, the suite's 381 test programs, and Return to Ditch Day without error; and reporting the reference's errors for the tests that are meant to fail |
| Code generation | Running Hexicon-compiled programs under both VMs and comparing output with the same programs compiled by `t3make`: first the conformance suite, then the games with walkthroughs |
| Dynamic compiler | The ten conformance tests that need it |
| Whole | Airport (adv3Lite's tutorial game) and Return to Ditch Day built from source by Hexicon alone, replayed against the reference build |

## Milestones

1. **Preprocessor.** Text in, preprocessed text out, identical to the reference's.
2. **Tokenizer and parser.** The whole language to a syntax tree, with the reference's diagnostics.
3. **Expressions and functions to byte code.** Enough for the dynamic compiler. Exit: the ten remaining conformance tests pass.
4. **Objects, grammar, and the rest of the language.** Enough to compile a single-file program to an image. Exit: the conformance suite's programs, compiled by Hexicon, give the same output as when compiled by `t3make`.
5. **Separate compilation, linking, project files.** Exit: the sample game, Airport, and Return to Ditch Day built from source and replayed identically.
6. **Hexicon's own language.** Its file extensions and its differences from TADS 3 are decided here, not before. The compiler accepts `.t`, `.h`, `.tl`, and `.t3m` throughout.

## Workspace

```
crates/
  hexicon-compiler/       the compiler as a library: no file system or console of its own
    src/pp.rs             the preprocessor
    src/lex.rs            the scanner that finds the next token on a line
    src/ppexpr.rs         constant expressions for #if, until the parser takes them over
  hexicon-cli/            `hexicon preprocess`, and later `hexicon build`
tools/
  pp-compare.sh           preprocesses files with Hexicon and the reference's test program, and compares
```

The compiler reads files through a small interface the caller supplies, as the VM reaches the outside through its host. That keeps it usable inside the VM, in a browser, and in tests.

## Status

- Milestone 1: the preprocessor is written and matches the reference (2026-11-19). `hexicon preprocess` writes a file's preprocessed text in the form the reference's preprocess-only mode does. Over 579 source files, about three million lines of output, it is identical to the reference's, messages included: the suite's 12 preprocessor tests and 381 test programs, every file of adv3 and its extensions, Return to Ditch Day, every file of adv3Lite, and Airport. Still to do in this milestone: the character sets beyond ASCII, UTF-8, and Latin-1; the compiler's own predefined macros; and `#if` expressions through the real expression parser, which waits for milestone 2.

## Findings

From milestone 1:

- The preprocessor is a port of the reference's, routine for routine, and not a design of its own, because what it has to reproduce is the exact text a macro expands to, and that comes out of the order the reference does things in. A macro's stored expansion has marker bytes for its parameters; expanded text carries markers that say which macro each part came from, which is how a macro is kept from expanding inside its own expansion.
- A line is one line of output for each line of source, so that line numbers still mean something afterward. A string that runs over several lines is joined into one, and whatever follows the string on its last line becomes a line of its own.
- The reference keeps a line as text that ends at a zero byte together with a separate count of its length, and a `//` comment ends the text without changing the count. Text spliced on afterward goes after the count, where nothing that reads up to the zero sees it. The effect a programmer can see: in a macro call that runs over lines, a `//` comment loses the rest of the argument it is in. `PAIR(one, // note` followed by `two)` gives the macro an empty second argument. Hexicon keeps its line the same way and does the same.
- The stack that `#foreach`, `#ifempty`, and `#ifnempty` nest on is static in the reference, shared by every expansion in progress. Expanding a macro inside an argument of another starts it over. Hexicon shares it too.
- A file named in an `#include` with quotes is looked for only beside the file that includes it, then on the include path. The reference has a loop that looks like a search up through the files that led to this one, but every pass of it looks in the same folder.
- `#pragma newline_spacing`, `#pragma sourceTextGroup`, `#pragma message`, and `#error` are for the compiler proper. When only preprocessing, they are passed through, with macros expanded in the last two.
- A source file's character set is found when it is opened: a byte order mark, or `#charset "name"` as the very first thing in the file, with exactly one space after `#charset`. Without either, it is ASCII, and a byte that is not ASCII becomes U+FFFD. A `#charset` naming a set that is not known is left in the text, where the preprocessor reports that it could not be loaded.
- The messages are the reference's short forms, word for word, and are reported at the same lines. An `#if` left open at the end of a file is reported against the line after the last one.
- An `#if` condition that cannot be evaluated counts as true, as in the reference.
- The reference's test program (`test_tok -P`) is the oracle, and it differs from the compiler proper in three ways that the comparison inherits: it defines a handful of macros of its own (`_WIN32` and the like) and not the compiler's (`__TADS3`, `__DEBUG`, and so on); it names files without their folders; and it cannot load the character set tables, so files in `cp1252` or `win1251` are compared as files whose character set failed to load.

Open after milestone 1:

- Character sets other than ASCII, UTF-8, UTF-16, and Latin-1. The VM has the reference's 43 tables; they need to be somewhere both the VM and the compiler can use them.
- In a message, a name with a character that is not ASCII is shown by the reference with its nearest ASCII stand-in (`cafe` for `café`), from the same tables. Hexicon shows the character's code instead.
- `#if` expressions are evaluated by a small evaluator of the preprocessor's own: integers, strings, `true`, `nil`, and the operators on them. The reference uses the compiler's expression parser and constant folding, which also covers floating-point numbers. The parser replaces the evaluator when it exists.
- The comparison is with the reference's test program. Comparing with `t3make -P` as well would cover the predefined macros and the character sets.
- Speed has not been looked at. An argument is copied each time it is expanded.
