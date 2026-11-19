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
| Tokenizer | A listing of the tokens the reference's tokenizer finds (`tools/reference/tok_dump.cpp`), over the same files |
| Parser and code generation | The reference has nothing that shows a syntax tree, so the two are checked together: the same source compiled by both compilers (`test_prs_top -u` for single-file programs, `t3make` for the rest), the byte code compared function by function, and the images run and their output compared. First small programs, then the conformance suite, then the games with walkthroughs. Programs that are meant not to compile have to give the reference's messages |
| Dynamic compiler | The ten conformance tests that need it |
| Whole | Airport (adv3Lite's tutorial game) and Return to Ditch Day built from source by Hexicon alone, replayed against the reference build |

## Milestones

1. **Preprocessor.** Text in, preprocessed text out, identical to the reference's.
2. **Tokenizer.** The stream of tokens the parser reads, identical to the reference's.
3. **Functions: parser and code generator together.** Expressions and statements, parsed, folded, and compiled to byte code, with enough of an image writer to make a single-file program of functions runnable. Checked by compiling the same source with both compilers and comparing the byte code and the output. This is also what the dynamic compiler needs. Exit: the ten remaining conformance tests pass.
4. **Objects, grammar, and the rest of the language.** Enough to compile any single-file program to an image. Exit: the conformance suite's programs, compiled by Hexicon, give the same output as when compiled by `t3make`.
5. **Separate compilation, linking, project files.** Exit: the sample game, Airport, and Return to Ditch Day built from source and replayed identically.
6. **Hexicon's own language.** Its file extensions and its differences from TADS 3 are decided here, not before. The compiler accepts `.t`, `.h`, `.tl`, and `.t3m` throughout.

## Workspace

```
crates/
  hexicon-compiler/       the compiler as a library: no file system or console of its own
    src/pp.rs             the preprocessor
    src/lex.rs            the scanner that finds the next token on a line
    src/tok.rs            the tokenizer: the stream of tokens the parser reads
    src/codegen.rs        the code stream and the instruction writer, with its peephole rules
    src/ppexpr.rs         constant expressions for #if, until the parser takes them over
  hexicon-cli/            `hexicon preprocess` and `hexicon tokens`, and later `hexicon build`
tools/
  pp-compare.sh           preprocesses files with Hexicon and the reference's test program, and compares;
                          with --tokens, compares the tokens
  reference/tok_dump.cpp  lists a file's tokens using the reference's own tokenizer
```

The compiler reads files through a small interface the caller supplies, as the VM reaches the outside through its host. That keeps it usable inside the VM, in a browser, and in tests.

## Status

- Milestone 1: the preprocessor is written and matches the reference (2026-11-19). `hexicon preprocess` writes a file's preprocessed text in the form the reference's preprocess-only mode does. Over 579 source files, about three million lines of output, it is identical to the reference's, messages included: the suite's 12 preprocessor tests and 381 test programs, every file of adv3 and its extensions, Return to Ditch Day, every file of adv3Lite, and Airport. Still to do in this milestone: the compiler's own predefined macros; and `#if` expressions through the real expression parser, which waits for the parser.

- Milestone 2: done (2026-11-19). `hexicon tokens` lists a file's tokens as the compiler proper reads them: after preprocessing, with keywords recognized and strings translated. Over the same 579 files, it lists the same tokens as the reference, with the same messages.
- Milestone 3: started (2026-11-19). The instruction writer, with the reference's peephole rules, is written and tested (`src/codegen.rs`). What an image and a compiled function have to look like, byte for byte, has been worked out from the reference and is recorded in the findings. Nothing is compiled yet: the parser, the code generators for expressions and statements, and the image writer are next.

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

From milestone 2, so far:

- The reference has no program that lists tokens: its tokenizer test only writes lines. `tools/reference/tok_dump.cpp` is a driver of Hexicon's own, built against the reference's sources by the same build that builds its test programs, and `pp-compare.sh --tokens` compares against it. The kinds of token are numbered here as they are there, so the listings can be compared as text.
- A string's token carries the translated text: escape sequences are replaced by the characters they stand for. A string with embedded expressions is several tokens: the text up to the first `<<`, the tokens of the expression, the text after `>>`, and so on. A format (`<<%2d n>>`) is a token of its own after the text before it. Whether `>>` ends an embedded expression or is a shift depends on whether any brackets are open inside the expression.
- What comes after `<<`, such as `if`, `one of`, and `first time`, is ordinary tokens at this stage. Making sense of them is the parser's job.
- After a character that is not valid, the reference skips the character after it as well. So does Hexicon.
- A `#pragma message` is shown as the file is read, and `#error` is reported as an error with the rest of its line, the space after `#error` included.

- The compiler knows ASCII, UTF-8, UTF-16, and Latin-1 itself, and asks whoever is using it for any other character set a source file names. The command-line program answers from the VM's tables, which are the reference's, so a source file can be in any of the sets a program can use. This closed a gap that was hidden at first: the reference's test programs could not find their character set tables, so files in `cp1252` and `win1251` were being compared as files whose character set failed to load. The reference build now puts the tables where the test programs look (`bin/charmap`), and those files are compared decoded.
- How the parser can be checked, which decides the order of the work from here. The reference has no way to show a syntax tree. Its expression test program (`test_prs`, now part of the reference build) shows what an expression folds to and the code it compiles to, but it has not kept up with the compiler: it crashes on an expression that uses a local variable, so it is only good for constants. What does work is `test_prs_top -u`, which compiles a whole single-file program, writes a runnable image, and lists the code. So the check on the parser is the code that comes out the far end: the same program compiled by both, with the byte code compared function by function and the two images run. The parser and the code generator are therefore built together, a part of the language at a time, and not one after the other.

From milestone 3, so far:

- The check is `t3make -nodef -nopre` on a single file against Hexicon's image, compared block by block. Three things in the reference's image cannot be reproduced and are left out of the comparison: the time stamp, the four bytes of the build configuration in the header, and two bytes of padding after each exported symbol's value in the `SYMD` block, which the reference never sets.
- The order of an image's blocks: `ENTP`, `SYMD`, `FNSD`, `MCLD`, the code pool (`CPDF` and its `CPPG` pages), the constant pool, the objects, `SINI`, and `EOF`. Even a program of two functions has nine metaclasses in `MCLD` and 21 exported symbols in `SYMD` (`LastProp`, `Constructor`, `Destructor`, the three grammar properties, and one for each operator that can be overloaded).
- A pool's page size is the smallest power of two, from 2,048 up, that holds the pool's largest item. Items are laid out in the order they were generated, and one that does not fit in what is left of a page starts the next. Code has two streams, main and static; the static stream starts on a new page. It is never empty: the compiler always generates a function there for registering multi-methods, which is a bare `retnil` when there are none. The constant pool's pages are stored XORed with a mask (0xDF from `t3make`).
- A compiled function is a 10-byte header (parameter count, with the top bit for variable arguments; optional parameter count; number of locals; stack slots needed, locals included; offset of the exception table; offset of the debug table), the code, the exception table if any, and the debug table. The stack figure comes from counting pushes and pops as the code is generated, not from analyzing it afterward.
- A function has a debug table even when the program is not compiled for debugging, if it has any local variables or parameters: the table then has no line records, only the frames of local names. A name of one or two characters is stored in the table; a longer one is stored in the constant pool, once per name for the whole program, and the table has its address. The locals of a frame are written in the order the reference's hash table gives them, which is by the sum of the name's bytes modulo the table size, and within one sum, the most recently defined first. Hexicon has to order them the same way.
- The peephole rules look at the last instruction written, or the last two, and rewrite in place. They are ported one for one. Two discards become a `disc1 2`, but a third is not added to it: three in a row are a `disc1 2` and a `disc`. A label resets what the rules remember, since a jump can arrive there.
- At the end of a function, a `retnil` is added unless the last instruction cannot be fallen out of (a return, a throw, or a jump). Then jumps to jumps are shortened.
- The reference's driver for testing expressions alone is no use (see above), and its whole-program test driver compiles for debugging, which `t3make` by default does not. `t3make` is the one to compare with.

Open after milestone 1:

- In a message, a name with a character that is not ASCII is shown by the reference with its nearest ASCII stand-in (`cafe` for `café`). Hexicon shows the character's code instead.
- `#if` expressions are evaluated by a small evaluator of the preprocessor's own: integers, strings, `true`, `nil`, and the operators on them. The reference uses the compiler's expression parser and constant folding, which also covers floating-point numbers. The parser replaces the evaluator when it exists.
- The comparison is with the reference's test program. Comparing with `t3make -P` as well would cover the predefined macros and the character sets.
- Speed has not been looked at. An argument is copied each time it is expanded.
