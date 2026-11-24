# The Language

Hexicon's language: what a program is made of. Its example, [examples/language](examples/language), is a program that tries each part of the language and prints what comes of it; each piece of code here is part of it, and each piece of output part of what it prints. It is built on the system library alone, without Hexad, to keep to the language itself.

## Programs and files

A program is a set of source files (`.hxn`) named in its manifest, `hexicon.toml`:

```toml
[game]
name = "A Tour of the Language"
libraries = ["system"]
sources = ["start"]
image = "language.hxg"
```

`libraries` are what the program is built on: `system` always, and for a game `hexad` too. `sources` are its own files, named without `.hxn`, in the order they are read. `image` is the file `hexicon build` writes.

A file starts by bringing in what it uses. A game says `import hexad;`, which brings in Hexad and, through it, everything else. A program without Hexad includes the system's header:

```hexicon
#include <hexicon.h>
```

A program on the system library alone starts at its `main()` function. (A game on Hexad has no `main()`: Hexad starts it, from `gameMain`.)

```hexicon
main(args)
{
    values();
    strings();
```

`//` starts a comment that runs to the end of the line, and `/*` one that runs to `*/`.

## Values

| Kind | Written | |
|---|---|---|
| integer | `42`, `-7`, `0x1F` | whole numbers, 32 bits |
| number | `3.14159`, `1e6` | with a decimal point or an exponent: decimals, kept exactly to many digits |
| string | `'text'` | text, kept as a value |
| true and false | `true`, `nil` | `nil` is false, and also "nothing": the value of what has none |
| list | `[1, 'two', nil]` | values in order |
| object | `rex`, `new Animal()` | see [Objects](#objects-and-classes) |
| property reference | `&sound` | a property itself, rather than its value |
| function | `{x: x * 2}` | see [Functions](#functions) |

```hexicon
    local n = 42;               // an integer
    local big = 3.14159;        // a number with a decimal point
    local s = 'text';           // a string
    local yes = true, no = nil; // true and false; nil is also "nothing"
```

`local` makes a variable, which lasts until the end of the block it is in.

### Operators

| | |
|---|---|
| arithmetic | `+ - * / %` |
| comparison | `== != < <= > >=` |
| logic | `&& || !` |
| bits | `& | ^ ~ << >> >>>` |
| choice | `a ? b : c`; `a ?? b`, which is `a` unless `a` is `nil`, and then `b` |
| membership | `x is in (a, b, c)`, `x not in (a, b, c)` |
| assignment | `= += -= *= /= %=` and the rest; `++` and `--` |

`+` also joins strings, and adds to lists. Division of integers gives an integer; a number with a decimal point gives one too:

```hexicon
    "<<7 / 2>> <<7 % 2>> <<7.0 / 2>>\n";
```

```transcript
3 1 3.5
```

## Strings

A string in single quotes is a value. A string in double quotes is *shown*: written as a statement on its own, it displays its text.

```hexicon
    local name = 'Ada';
    // Single quotes make a string; double quotes show one.
    local greeting = 'Hello, ' + name + '!';
    "<<greeting>>\n";
    // Code between << and >> in a shown string is worked out and shown.
    "<<name>> has <<name.length()>> letters.\n";
```

```transcript
Hello, Ada!
Ada has 3 letters.
```

What is shown between `<<` and `>>` has to be text or a number. `true`, `nil`, and other values are turned into text with `toString()` first.

In a string, `\n` starts a new line, `\b` leaves a blank one, `\t` is a tab, and `\'`, `\"`, and `\\` are the characters themselves. In a game, what is shown can also have tags in angle brackets, as HTML does: `<q>` and `</q>`, for instance, are quotation marks.

### Choices in a string

`<<if>>` shows what follows only if its condition holds, with `<<else>>` for otherwise and `<<end>>` to close it:

```hexicon
    "<<if name == 'Ada'>>It is Ada.<<else>>It is not.<<end>>\n";
```

`<<one of>>` gives alternatives, separated by `<<or>>`, one of which is shown each time; how it is chosen is the phrase it ends with:

```hexicon
    for (local i in 1..3)
        "<<one of>>first<<or>>second<<or>>third<<cycling>> ";
```

```transcript
first second third
```

| Ending | Chooses |
|---|---|
| `<<cycling>>` | each in turn, and then from the first again |
| `<<stopping>>` | each in turn, and then the last for ever |
| `<<at random>>` | at random, never the same twice running |
| `<<purely at random>>` | at random, repeats and all |
| `<<shuffled>>` | each once, in a random order, and then again in another |
| `<<half shuffled>>` | as shuffled, but shuffled again after only half of them |
| `<<sticky random>>` | one at random, the first time, and that one ever after |
| `<<as decreasingly likely outcomes>>` | at random, the first most often |
| `<<then at random>>`, `<<then purely at random>>`, `<<then shuffled>>`, `<<then half shuffled>>` | each in turn once, and after that as the rest of the phrase says |

### What strings can do

A string has methods of its own:

```hexicon
    "<<name.toUpper()>> <<name.substr(2)>> <<name.find('d')>>\n";
    "<<'a,b,c'.split(',').length()>> <<toString(12) + '!'>> <<toInteger('7') + 1>>\n";
```

```transcript
ADA da 2
3 12! 8
```

Positions in a string, like those in a list, count from 1.

## Lists

```hexicon
    local lst = [3, 1, 2];
    "<<lst.length()>> <<lst[1]>> <<lst.sort()>>\n";
    lst += 4;                                   // a new list, with 4 added
    "<<lst>> <<lst.indexOf(2)>> <<lst.subset({x: x > 1})>>\n";
    "<<lst.mapAll({x: x * 2})>> <<lst.car()>> <<lst.cdr()>>\n";
```

```transcript
3 3 1,2,3
3,1,2,4 3 3,2,4
6,2,4,8 3 1,2,4
```

A list never changes. `sort()` gives a new list, sorted, and leaves `lst` as it was; `+=` makes a new list and puts it in the variable. `subset()` keeps the elements a function says yes to, and `mapAll()` gives a list of what a function makes of each. `car()` is the first element, and `cdr()` the rest.

## Tables

A `Map` keeps values by keys:

```hexicon
    local ages = new Map();
    ages['Ada'] = 36;
    ages['Alan'] = 41;
    "<<ages['Ada']>> <<toString(ages.isKeyPresent('Alan'))>> <<toString(ages['Grace'])>>\n";
```

```transcript
36 true nil
```

A key with no value gives `nil`.

## Statements

```hexicon
    local total = 0;
    for (local i = 1; i <= 4; i++)
        total += i;
    "for: <<total>>\n";

    local n = 0;
    while (n < 3)
        n++;
    "while: <<n>>\n";

    foreach (local word in ['one', 'two'])
        "foreach: <<word>>\n";
```

`for (local i in 1..10)` counts through a range, and `foreach` goes through a list (or any collection). `do ... while (cond);` tests at the end. `break` leaves a loop and `continue` goes on to its next round:

```hexicon
    for (local i in 1..10)
    {
        if (i % 2 == 0)
            continue;
        if (i > 5)
            break;
        "<<i>> ";
    }
```

```transcript
1 3 5
```

`switch` chooses by value; each `case` runs on into the next unless it ends with `break`:

```hexicon
    switch (n)
    {
    case 1:
        "switch: one\n";
        break;
    case 3:
        "switch: three\n";
        break;
    default:
        "switch: something else\n";
    }
```

Braces group statements into one, and every statement but a block ends with `;`.

## Functions

A function is defined at the top of a file, with its parameters, and `return` gives its value:

```hexicon
// A parameter with ? may be left out; then it is nil.
greet(name, title?)
{
    return 'Hello, ' + (title ?? '') + name;
}

// A parameter with = has a value when it is left out.
repeat(text, times = 2)
{
```

```hexicon
// [rest] collects any more arguments, as a list.
sum([rest])
{
```

```transcript
Hello, Ada; Hello, Lady Lovelace
abab ababab 10
```

A function can also be a value, written where it is used: `{x: x * 2}` for one that works out a value from its parameters, or `function(a, b) { ... }` for one with statements:

```hexicon
    // A function as a value, written in place.
    local double = {x: x * 2};
    local add = function(a, b) { return a + b; };
    "<<double(5)>> <<add(2, 3)>>\n";
```

Either can use the local variables of the code it is written in.

## Objects and classes

A **class** describes a kind of object; an object is defined as one of a class, with what makes it different:

```hexicon
class Animal: object
    name = 'an animal'
    sound = 'nothing'

    // A method: a property whose value is code.
    speak() { return name + ' says ' + sound + '.'; }
;

class Dog: Animal
    sound = 'woof'
;

// An object of a class, with its own value for a property.
rex: Dog
    name = 'Rex'
;
```

An object has everything its class has, and its class everything *its* class has, up to `object`: `rex` has Dog's `sound`, and Animal's `speak()`. In a method, `name` is the object's own `name` (its `self.name`), so `speak()` speaks for whichever object it is called on. A class can have several classes it builds on, listed with commas, the first coming first.

```transcript
Rex says woof.
Fido says woof. And wags.
```

### Building on a method

A method of an object or a class can add to the one it replaces, by calling it with `inherited`:

```hexicon
// A method can add to the one its class has, with inherited.
fido: Dog
    name = 'Fido'
    speak() { return inherited() + ' And wags.'; }
;
```

### Templates

A **template** says what values written without names stand for, for a class. It is how `Room 'Rocky Shore' "..."` sets a room's title and description:

```hexicon
// A template says what values without names stand for, for a class.
class Bird: Animal;
Bird template 'name' 'sound';

tweety: Bird 'Tweety' 'tweet';
```

```transcript
Tweety says tweet.
```

### Changing what is defined elsewhere

`modify` adds to an object or a class defined elsewhere, or changes it, as if it had been written that way. A game changes Hexad's classes so:

```hexicon
// modify changes an object, or a class, defined elsewhere.
modify Dog
    legs = 4
;
```

### Objects made while the program runs

`new` makes an object of a class while the program runs. The class's `construct()` method, if it has one, is given what `new` is given:

```hexicon
    // An object made while the program runs.
    local cat = new Animal();
    cat.name = 'Tom';
    cat.sound = 'miaow';
    "<<cat.speak()>>\n";
```

### Properties as values

`&sound` is the property `sound` itself. `rex.(prop)` reads whichever property `prop` points to, and `propDefined()` asks whether an object has one:

```hexicon
    // A property named by a pointer to it, &prop, and read with .(...).
    local prop = &sound;
    "<<rex.(prop)>>\n";
```

Hexicon warns of a property given a value that nothing reads, when another of nearly the same name is read: a misspelling (see [Getting Started](getting-started.md)).

## Exceptions

`throw` stops what is running and goes to the nearest `catch` for that kind of exception. `finally` runs however the `try` ends:

```hexicon
// An exception of the program's own, made with new: its construct()
// method is given what new is.
class TooHeavy: Exception
    construct(weight) { self.weight = weight; }
    weight = 0
;
```

```hexicon
    try
    {
        "<<lift(50)>>\n";
        "<<lift(500)>>\n";
        "not reached\n";
    }
    catch (TooHeavy e)
    {
        "caught: <<e.weight>> is too heavy\n";
    }
    finally
    {
        "finally, always\n";
    }
```

```transcript
lifted 50
caught: 500 is too heavy
finally, always
```

## The preprocessor

Before a file is compiled, lines that start with `#` are worked out. `#define` makes a **macro**, a name that stands for text, which may take arguments. `#ifdef` keeps what follows only if a macro is defined, up to `#endif`. `#include` brings in a file's text.

```hexicon
#define GREETING 'hello'
#define TWICE(x) ((x) * 2)
```

```hexicon
#ifdef __HEXICON
    "This is Hexicon.\n";
#endif
```

`__HEXICON` is defined in every Hexicon source. A manifest's `[defines]` defines macros for the whole program: a game's `LANGUAGE = "english"` is one.

## Hexicon's own forms

The pages on Hexad describe the forms that are Hexicon's own:

- `import hexad;`, which brings a library into a file: [Getting Started](getting-started.md);
- `action Ring(dobj): TAction;`, which declares an action, and `dobj Ring { ... }` and `dobj Ring as Pull`, which handle one: [Actions and Their Handlers](actions.md).

The built-in functions (`print()`, `rand()`, `toString()`, and the rest) are declared in the system library's headers: `general.h`, `vm.h`, and `io.h`, which `hexicon.h` brings in.
