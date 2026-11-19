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
| Parser and code generation | The reference has nothing that shows a syntax tree, so the two are checked together: the same source compiled by both compilers, and the whole image compared byte for byte, along with the messages each compiler gives (`tools/compile-compare.sh`). The sources are hand-written programs for each feature (`tools/compile-tests`), a few hundred small ones that each probe one case or one mistake (`gen-cases.py` there), random programs made of whatever the compiler supports so far (`gen-random.py`), then the conformance suite, then the games with walkthroughs. Programs that are meant not to compile have to give the reference's messages |
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
    src/parse.rs          the parser: tokens to a syntax tree and the program's symbols
    src/ast.rs            the syntax tree
    src/fold.rs           working out constant expressions
    src/flow.rs           how control leaves a statement: what can and cannot be reached
    src/emit.rs           the code generator: the syntax tree to byte code
    src/codegen.rs        the code stream and the instruction writer, with its peephole rules
    src/image.rs          the image writer: pool layout, addresses, and the image's blocks
    src/compile.rs        the two readings of the source, and what comes back to the caller
    src/ppexpr.rs         constant expressions for #if, until the parser takes them over
  hexicon-cli/            `hexicon build`, `hexicon preprocess`, and `hexicon tokens`
tools/
  pp-compare.sh           preprocesses files with Hexicon and the reference's test program, and compares;
                          with --tokens, compares the tokens
  reference/tok_dump.cpp  lists a file's tokens using the reference's own tokenizer
  compile-compare.sh      compiles files with Hexicon and with `t3make`, and compares the images
                          and the messages
  image-diff.py           compares two images, leaving out the parts that mean nothing
  compile-tests/          programs for compile-compare.sh, and the generator of random ones
```

The compiler reads files through a small interface the caller supplies, as the VM reaches the outside through its host. That keeps it usable inside the VM, in a browser, and in tests.

## Status

- Milestone 1: the preprocessor is written and matches the reference (2026-11-19). `hexicon preprocess` writes a file's preprocessed text in the form the reference's preprocess-only mode does. Over 579 source files, about three million lines of output, it is identical to the reference's, messages included: the suite's 12 preprocessor tests and 381 test programs, every file of adv3 and its extensions, Return to Ditch Day, every file of adv3Lite, and Airport. Still to do in this milestone: `#if` expressions through the real expression parser.

- Milestone 2: done (2026-11-19). `hexicon tokens` lists a file's tokens as the compiler proper reads them: after preprocessing, with keywords recognized and strings translated. Over the same 579 files, it lists the same tokens as the reference, with the same messages.
- Milestone 3: under way. `hexicon build` compiles a single-file program of functions to an image, and the program runs: it can call the virtual machine's built-in functions, so it can print (2026-11-19). It has `intrinsic` declarations of built-in function sets; parameters (plain, `...`, and `[rest]`); locals and nested scopes; `if`, `while`, `do`, `for`, `switch`, `break` and `continue` with and without labels, labels and `goto`, `try` with `finally`, `throw`, and `return`; and expressions over integers, strings, lists, `nil`, `true`, locals, and calls to functions by name or through a value, with indexing, assignment to an element, strings in double quotes that are displayed, and every operator except those that need objects. Strings of both kinds can have expressions embedded in them (`<<...>>`), with `<<if>>` and `<<unless>>`, a format (`<<%d x>>`), and string templates (`string template`). A `catch` clause is read, but the class it names has to be an object, and there are no objects yet; the same goes for `<<one of>>` and `<<first time>>` in a string. Over 13 hand-written programs, 280 small ones that each probe one case or one mistake, and 4,350 random programs, it writes the same image as `t3make`, byte for byte, and gives the same messages. The exceptions, about one random program in a hundred, are a constant divided by zero, where the reference's messages depend on what is in its memory (see the findings), and constant arithmetic that overflows an integer, which needs floating-point constants. The compiler's predefined macros are in (`__TADS3`, the version numbers, `__DATE__`, `__TIME__`, and the system's name, which for Hexicon is `HEXICON`). Anything else in the language is reported as not supported yet. Still to do in this milestone: properties and method calls, and with them objects enough for `catch` and `<<one of>>`; `foreach` and `for` over a collection; lookup-table lists; anonymous functions; floating-point and regular-expression constants; optional, named, and typed parameters; and the dynamic compiler itself.

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

- The check is `t3make -nodef -nopre` on a single file against Hexicon's image, the whole file compared. Some of the reference's image cannot be reproduced and is left out of the comparison: the time stamp; the four bytes of the build configuration in the header; two bytes of padding after each exported symbol's value in the `SYMD` block, which the reference never sets; and, in a function's table of locals, the field for a local's place in a shared context, which the reference never sets for a local that is not shared and which then holds whatever its memory did. Hexicon writes zeros in the last two. Rather than have the comparison know the image's layout, `hexicon build --unspecified` lists these stretches of its own image, and the comparison skips them in both.
- The reference reads a source file twice: once only to learn what it defines (the symbol file `t3make` writes), and again, knowing that, to generate code. Hexicon does the same, because it shows: a function can be called before the line that defines it, and some messages come only from one reading. A `while` whose condition is always false is warned about on the first reading and not the second; an `if` on the second and not the first. The reference's messages from the first reading come out before those of the second, and so do Hexicon's.
- Constants are folded twice as well. Each operator is folded as it is parsed, when both operands are already constants. After the whole program has been read, names are looked up, and a name that is a function becomes a constant; then only the comma, `+`, `-`, `==`, `!=`, `&&`, `||`, `!`, `?:`, and `??` are tried again.
- Once a function's name has become a constant, a call through it is taken to leave a value whether or not the function returns one. That matters in `a ? f() : g()`, where a branch that leaves no value has `nil` pushed for it: after folding, no call by name is such a branch.
- A statement's code depends on small things that are easy to get wrong and that the comparison found: `x * 0` evaluates `x` and pushes zero even when the value is not wanted, leaving it on the stack; an integer from -128 to 126 is pushed in two bytes, but 127 takes five; a jump to the very next instruction is taken back out when the label is placed; and nothing is written after an instruction that cannot be fallen out of until a label is placed.
- A local declared as the single statement of an `if` or a loop, with no braces, belongs to the scope around. `if (a) local b = 1;` declares `b` for the rest of the enclosing block. A `for` has a scope of its own for the locals in its parentheses, and a braceless body's locals go there.
- Each scope that declares locals takes new slots for them, and slots are never used again by a later scope, though the reference has a comment wishing they were.
- Warnings about locals that are never used, or never given a value, come after all the code has been generated, function by function and scope by scope, in the order of the reference's hash table, and are reported at the line of the declaration. An unused parameter is not warned about by default.
- Two places where the reference's result depends on what happens to be in its memory, and Hexicon's does not. Adding `nil` to a constant string (`'ab' + nil`) reads past the end of an empty string in the reference; in Hexicon it adds nothing. A constant division by zero is reported once by Hexicon; the reference goes on to use a flag it never set, and usually reports it twice.
- In the reference built for Windows, a constant multiplication that overflows 32 bits wraps around, because its arithmetic there is 32 bits wide before it looks for the overflow; built elsewhere it becomes a floating-point number. Hexicon wraps for now, and will have to decide when it has floating-point constants.
- `name { ... }` is not a function without parameters; without the keyword `function` it is taken for an object.
- A name in a message is cut to 60 bytes, with `...` after it, by the reference's message formatter. Hexicon does the same for every name it shows.
- A symbol longer than 80 characters draws a warning that it has been truncated, but the tokenizer passes it on whole. A local with such a name gets a broken entry in its function's table: the reference collects the names of locals in a stream of their own before moving them to the constant pool, does not move one longer than 80, and leaves the table holding the name's offset in that stream as if it were an address in the pool. It is the same number every time, so Hexicon writes it too.
- A source file that does not say what character set it is in (no byte order mark, no `#charset`) is read by `t3make` in the system's own character set, which on the Windows machine the reference was built on is code page 1252; the reference's tokenizer test program reads it as ASCII. Hexicon reads it as UTF-8, which is what a new source file is most likely to be in and does not depend on the machine; `-cs name` says otherwise. The comparisons pass `-cs cp1252` or `-cs us-ascii` to match the program they compare with.
- A built-in function is known by the number of its set and its number in the set, in the order the `intrinsic` declarations give them. Built-in functions are not in what the first reading of the source hands to the second: each reading declares them as it comes to them. A name of a built-in function without parentheses is a call with no arguments.
- A set of built-in functions declared twice keeps the later version, but by overwriting: the reference compares as many bytes as the new version has, and if the new is greater, writes them over the start of the old. `'seven/123456789'` and then `'seven/2'` leave `seven/223456789`. Hexicon does the same. A set declared first without a version and then with one gets, in the reference, whatever followed the name in its memory; in Hexicon it gets the version.
- Calling a built-in function numbered above 255 in its set crashes the reference compiler. Hexicon generates the long form of the instruction.
- A constant list is a count and five bytes for each element; the strings and lists among its elements follow it in the pool, and the elements hold their addresses. Lists are not shared the way short strings are. The longest list in the source, constant or not, counts toward the size of the pool's pages. For an element that is `nil` or `true`, only the first of the five bytes means anything, and the comparison skips the rest.
- The reference folds `==`, `!=`, `!`, `&&`, and `||` by overwriting the left operand's value in place, and when that operand is a constant list, what is left is a list that claims to be `true` or `nil`. Whatever asks for its value sees the truth value: `return [1] == [1];` returns true, and `if ([1] == [2])` is a condition that is always false. But where code is generated for it as an expression, the list is pushed: `local a = [1] == [1];` sets `a` to `[1]`. It is the same every time, so Hexicon does it too.
- Assigning to an element (`a[i] = v`) is three steps: the value, then the container and the index, then `setind`, which gives a new container; and then that is stored back into `a` if `a` can be assigned to, or discarded. The instruction writer turns the common case, a local indexed by a small constant, into one instruction.
- A string in double quotes is displayed where it stands (`say`), and using it for its value is an error. In `a ? "x" : f()` it counts as a branch with no value.
- The reference stops after working out constants if that found anything wrong, before generating any code. So a program with a `case` that is not a constant, or a constant index out of range, gets none of the messages that generating code would give (an undefined name, a local never used).
- A `switch` is the value, then one instruction holding a table: the number of cases, a kind and four bytes and a jump for each, and a jump for when nothing matches. The table is written empty, and each `case` and the `default` fills in its own entry when its code is come to. Each jump is measured from where it is written. A case that is `nil` or `true` has four bytes that mean nothing, as in a constant list. The number of cases is counted by the parser, and the count includes a `case` that the parser then drops for coming before anything reachable.
- The end of a `switch` is given a place to jump to only if working out the control flow says that what follows can be reached. If it cannot, a `break` in the switch generates nothing.
- A `case` or `default` has to be followed by a statement: `default: }` is a syntax error.
- A function's labels are in a table of their own, of the size of the table of global names (512 chains, where a scope's locals have 32), and the warnings about labels that nothing refers to come in that table's order, after the function's code is generated.
- A use of a label is recorded when the parser works out a function's control flow, so a `goto` in code that cannot be reached does not count, and its label is reported as not referenced.
- The code generator works out control flow again for each `switch` and each `finally` block. A `goto`, `break`, or `continue` naming a label that it meets then is recorded as a use of the label of that name in the last function of the source file, whatever function the statement is in, because the parser is still holding that function's table. So a label in the last function that nothing refers to gets no warning if another function has a `switch` with a `goto` to a label of the same name. Hexicon does the same.
- A statement whose label's name was already taken has no label as far as the reference is concerned, and looking up a name the function does not have also finds no label, so the two match: `break nosuch;` inside such a statement leaves it, with no complaint beyond the one about the name. Hexicon does the same.
- `continue` can name a label only if the label is directly on a loop (or on another label that is).
- A `finally` block is a subroutine within the function. Leaving the protected code any way at all calls it first (`ljsr`): falling out of the end, `break`, `continue`, `goto`, and `return`; and so does the handler for an exception, which keeps the exception in a local, calls the block, and throws the exception again. The block starts by putting where to go back to in a local, and ends with `lret`. The two locals come after the function's own: where to go back to, and then the exception. A `return` with a value keeps the value in one more local while the blocks run; that local is added when the code is generated, one for the whole function.
- A function with a `try` has a table of exception handlers between its code and its table of local names: a count, and ten bytes for each (the first and last byte of the protected code, the class caught or zero for all, and where the handler is).
- If a `finally` block cannot be fallen out of (it returns or throws), no jump past it is generated after the call.
- The parser takes a `catch` block to be outside its `try`, and the code generator takes it to be inside. A `goto` from a `catch` block to a label in the same block therefore counts as leaving the `try`, and calls the `finally` block before jumping.
- `goto` cannot jump into a `finally` block. It can jump into anything else, a loop or a `try` included.
- `goto` with no label after it crashes the reference compiler, which goes to write a message that names a token without having been given one. Hexicon reports the token it found.
- A string with expressions embedded in it comes from the tokenizer in pieces, and the parser puts them together. In double quotes, `"one <<two>> three"` is `"one ", say(two), " three"`: each piece of text is displayed, and each expression is worked out and what it leaves is displayed (`sayval`). An expression that leaves nothing, a call to a function that returns no value, is worked out for what it does. In single quotes, `'one <<two>> three'` is `'one ' + (two) + ' three'`. The additions are not worked out when they are parsed but with the other constants afterward, so a string with only constants embedded is a constant, and can be a `case`.
- An empty piece of text is left out, but for the first piece of a string in single quotes, which makes what follows a string whatever is added to it.
- The parser reads all the tokens of an embedded expression, up to the next piece of the string, before it decides what they are. So a mistake in an embedded expression that runs over several lines is reported on the line where the next piece of the string ends.
- `<<if x>>`, `<<unless x>>`, `<<else if x>>`, `<<else unless x>>`, `<<else>>`, and `<<end>>` make the conditional operator of what is between them; `otherwise` can be written for `else`. `<<end>>` can be left out: the end of the string, or the end of whatever the `if` is inside of, ends it. Without an `else`, the other branch is an empty string. `else` is taken for the end of an `if` wherever it is, and is an error if there is none. The other words (`end`, `otherwise`, `or`, `only`, and the phrases that end `<<one of>>`) mean something only inside their construct; anywhere else they are ordinary names in an expression.
- The words of these constructs are matched as text, whatever kind of token has it. `<<'or'>>`, a string, is taken for `<<or>>`. The exception is `else`, which has to be the keyword once it has been recognized: `<<'else'>>` ends the branch it is in and is then an error.
- `<<%d x>>`, an expression after a format, is `sprintf('%d', x)`. The function is found by its name among the program's global names, so the program has to declare the built-in function, and a local variable called `sprintf` makes no difference.
- `<<one of>>` and `<<first time>>` keep their state in an object of the class `OneOfIndexGen`, which the program has to define. Until there are objects, they are read and then reported as needing the class, as the reference reports them for a program without it.
- `string template <<tokens>> function;` makes an embedded expression of those tokens a call to the function. `*` among the tokens stands for one or more tokens, which are the function's argument; what they are is looked for among the templates in turn, so templates nest. The templates are tried in the order they were defined, and the first that fits is used. Tokens are compared as text.
- A template's function need not be defined yet. If it is not, the template declares it, as taking one argument if there is a `*` and none if not, and a definition that follows has to agree. If it is defined already, it has to take that many arguments and return a value. A function that was declared by a template first and defined afterward is not checked for returning a value.
- A function that is declared and never defined is an error only if something refers to it: `unresolved external reference`, with no place in the source, after everything is generated. Several are reported in the order of the table of global names.
- The compiler gives up after a hundred errors: the hundred-and-first is followed by `fatal error: too many errors`, at the place being read or generated, and nothing after that. This holds while reading the source, working out constants, and generating code. It is what ends the reference's attempts to finish a string that the source ends in the middle of, which would otherwise never end.
- A name that nothing defines is reported once. The reference then notes the name as known to be undefined, and later uses of it, in whatever function, generate nothing and say nothing.
- A string token named in a message is cut to twenty characters, with `...` if there was more, and its control characters are written as escapes (`\n`). For a control character without an escape of its own the reference means to give the code in hexadecimal, but adds the character `0` to each digit, which gives letters and backquotes. Hexicon does the same.
- Three things about strings that the reference crashes or hangs on. An empty embedding with text after it, when nothing comes before the two in a string in double quotes or in a branch of an `<<if>>` there (`"<<>>text"`), leaves a hole in the syntax tree, and the reference crashes on it; Hexicon displays the text. `string template <<` with no `>>` before the end of the source never stops reading; Hexicon reports the missing `>>`. And a template whose function's name is a built-in function's is taken to be a function of the program's own, reading what is not there; Hexicon reports that the function does not match the template.
- The order of an image's blocks: `ENTP`, `SYMD`, `FNSD`, `MCLD`, the code pool (`CPDF` and its `CPPG` pages), the constant pool, the objects, `SINI`, and `EOF`. Even a program of two functions has nine metaclasses in `MCLD` and 21 exported symbols in `SYMD` (`LastProp`, `Constructor`, `Destructor`, the three grammar properties, and one for each operator that can be overloaded).
- A pool's page size is the smallest power of two, from 2,048 up, that holds the pool's largest item. Items are laid out in the order they were generated, and one that does not fit in what is left of a page starts the next. Code has two streams, main and static; the static stream starts on a new page. It is never empty: the compiler always generates a function there for registering multi-methods, which is a bare `retnil` when there are none. The constant pool's pages are stored XORed with a mask (0xDF from `t3make`).
- A compiled function is a 10-byte header (parameter count, with the top bit for variable arguments; optional parameter count; number of locals; stack slots needed, locals included; offset of the exception table; offset of the debug table), the code, the exception table if any, and the debug table. The stack figure comes from counting pushes and pops as the code is generated, not from analyzing it afterward.
- A function has a debug table even when the program is not compiled for debugging, if it has any local variables or parameters: the table then has no line records, only the frames of local names. A name of one or two characters is stored in the table; a longer one is stored in the constant pool, once per name for the whole program, and the table has its address. The locals of a frame are written in the order the reference's hash table gives them, which is by the sum of the name's bytes modulo the table size, and within one sum, the most recently defined first. Hexicon has to order them the same way.
- The peephole rules look at the last instruction written, or the last two, and rewrite in place. They are ported one for one. Two discards become a `disc1 2`, but a third is not added to it: three in a row are a `disc1 2` and a `disc`. A label resets what the rules remember, since a jump can arrive there.
- At the end of a function, a `retnil` is added unless the last instruction cannot be fallen out of (a return, a throw, or a jump). Then jumps to jumps are shortened.
- The reference's driver for testing expressions alone is no use (see above), and its whole-program test driver compiles for debugging, which `t3make` by default does not. `t3make` is the one to compare with.

Open after the first part of milestone 3:

- Constant arithmetic that overflows an integer is reported as not supported, until there are floating-point constants. Multiplication is the exception, and wraps, as in the reference built for Windows.
- The messages of the first reading of the source are kept and those of the second added, as the reference does, so a warning from the preprocessor comes out twice. That is the reference's behavior, and is not pleasant; Hexicon's own language may want it otherwise.
- Warnings that the reference gives only when asked for every warning (an unused parameter, for one) are not given at all.
- An image's exported symbols and metaclass list are the fixed ones of a program without objects or properties.
- The time stamp in an image, and `__DATE__` and `__TIME__`, are in UTC from `hexicon build`. The reference uses local time.

Open after milestone 1:

- In a message, a name with a character that is not ASCII is shown by the reference with its nearest ASCII stand-in (`cafe` for `café`). Hexicon shows the character's code instead.
- `#if` expressions are evaluated by a small evaluator of the preprocessor's own: integers, strings, `true`, `nil`, and the operators on them. The reference uses the compiler's expression parser and constant folding, which also covers floating-point numbers. The parser replaces the evaluator when it exists.
- The preprocessor's own comparison is with the reference's test program. The predefined macros and the default character set are now covered by the comparison of compiled programs instead.
- Speed has not been looked at. An argument is copied each time it is expanded.
