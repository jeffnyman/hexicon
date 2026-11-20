#!/usr/bin/env python3
"""Write random programs for comparing compilers.

usage: gen-random.py <folder> <count> [seed]

Each program is made of the parts of the language Hexicon compiles so
far: functions, locals, the statements that need no objects (labels,
`goto`, `switch`, `try` with `finally`, and `throw` among them), and
expressions over numbers, strings, lists, locals, and calls to functions
and to built-in functions. Strings have expressions embedded in them,
string templates included. Most programs have objects and classes, with
properties and methods. The programs are mostly
valid but mean nothing; they are for tools/compile-compare.sh, to find
source that the two compilers turn into different code.
"""

import os
import random
import sys

NAMES = [
    "a",
    "b",
    "c",
    "i",
    "n",
    "x",
    "y",
    "ab",
    "ba",
    "tmp",
    "count",
    "total",
    "index",
    "value",
    "result",
    "flag",
    "left",
    "right",
    "xx",
    "k9",
]
# Names for parameters passed by name: none of them a local's, since a
# parameter defined twice brings the reference down.
NAMED = ["nm0", "nm1", "opt"]
BINARY = [
    "+",
    "-",
    "*",
    "/",
    "%",
    "&",
    "|",
    "^",
    "<<",
    ">>",
    ">>>",
    "==",
    "!=",
    "<",
    "<=",
    ">",
    ">=",
    "&&",
    "||",
]
# Built-in functions to call: name, fewest arguments, most.
BUILTINS = [
    ("tadsSay", 1, 3),
    ("toString", 1, 3),
    ("dataType", 1, 1),
    ("max", 1, 4),
    ("getArg", 1, 1),
    ("t3RunGC", 0, 0),
]
PRELUDE = """intrinsic 't3vm/010006'
{
    t3RunGC();
    t3SetSay(val);
    t3GetVMVsn();
    t3GetVMID();
    t3GetVMBanner();
    t3GetVMPreinitMode();
    t3DebugTrace(mode, ...);
    t3GetGlobalSymbols(which?);
    t3AllocProp();
    t3GetStackTrace(level?, flags?);
    t3GetNamedArg(name, defval?);
    t3GetNamedArgList();
}
intrinsic 'tads-gen/030008'
{
    dataType(val);
    getArg(idx);
    firstObj(cls?, flags?);
    nextObj(obj, cls?, flags?);
    randomize(...);
    rand(x, ...);
    toString(val, radix?, isSigned?);
    toInteger(val, radix?);
    getTime(timeType?);
    rexMatch(pat, str, index?);
    rexSearch(pat, str, index?);
    rexGroup(groupNum);
    rexReplace(pat, str, replacement, flags?, index?, limit?);
    savepoint();
    undo();
    saveGame(filename, metaTable?);
    restoreGame(filename);
    restartGame();
    max(val1, ...);
    min(val1, ...);
    makeString(val, repeatCount?);
    getFuncParams(func);
    toNumber(val, radix?);
    sprintf(format, ...);
}
intrinsic 'tads-io/030007'
{
    tadsSay(val, ...);
}
intrinsic class Object 'root-object/030004'
{
    ofKind(cls);
    getSuperclassList();
    propDefined(prop, flags?);
}
intrinsic class Collection 'collection/030000': Object
{
    createIterator();
    createLiveIterator();
}
intrinsic class List 'list/030008': Collection
{
    subset(func);
    mapAll(func);
    length();
}
"""
ASSIGN = ["=", "+=", "-=", "*=", "/=", "%=", "&=", "|=", "^=", "<<=", ">>=", ">>>="]
# String templates a program can have: the definition, the function's
# number of parameters, and how an expression is written to use it.
TEMPLATES = [
    ("<<* in words>>", 1, "{} in words"),
    ("<<bold>>", 0, "bold"),
    ("<<the * thing>>", 1, "the {} thing"),
    ("<<big *>>", 1, "big {}"),
]
WORDS = ["", "", "a", "one two", " and ", ". ", "x", "It\\'s ", "\\n", "%"]
FORMATS = ["%d", "%5d", "%s", "%x", "%-3s", "%05.2f", "%c"]
# Names for properties. Some have the same letters as others, which puts
# them in the same chain of the reference's table of names.
PROPS = [
    "name",
    "size",
    "ab1",
    "ba1",
    "weight",
    "owner",
    "abc2",
    "cab2",
    "bca2",
    "describe",
    "count1",
    "total",
    "next1",
    "value1",
]


class Program:
    def __init__(self, rng):
        self.rng = rng
        # Each function: name, number of parameters, whether it takes more.
        self.funcs = [("_main", 1, False)]
        for index in range(rng.randint(1, 5)):
            self.funcs.append((f"fn{index}", rng.randint(0, 5), rng.random() < 0.2))
        # The labels of the function being written, and those on the
        # statements around the one being written, each with whether it
        # is on a loop.
        self.labels = []
        self.around = []
        # The string templates the program defines.
        self.templates = []
        # The objects: name, whether it is a class, and its superclasses.
        # An object without a name has none here.
        self.objs = []
        if rng.random() < 0.7:
            for index in range(rng.randint(1, 5)):
                is_class = rng.random() < 0.4
                name = f"{'Cls' if is_class else 'obj'}{index}"
                if rng.random() < 0.1 and not is_class:
                    name = None
                earlier = [obj[0] for obj in self.objs if obj[0]]
                supers = rng.sample(
                    earlier, min(len(earlier), rng.choice([0, 1, 1, 2]))
                )
                self.objs.append((name, is_class, supers))
        # The properties the program's source mentions.
        self.props = rng.sample(PROPS, rng.randint(2, 8))
        # Whether the code being written is a method's.
        self.in_method = False
        # How deep in embedded expressions the expression being written
        # is. `>>` there ends the expression, so no operator has it.
        self.embedding = 0
        if rng.random() < 0.3:
            self.templates = rng.sample(TEMPLATES, rng.randint(1, len(TEMPLATES)))

    def embedded_expr(self, scope, depth, plain=False):
        """An expression to go between `<<` and `>>`."""
        rng = self.rng
        self.embedding += 1
        text = self.expr(scope, depth)
        self.embedding -= 1
        if plain:
            return text
        if self.templates and rng.random() < 0.3:
            text = rng.choice(self.templates)[2].format(text)
        if rng.random() < 0.1:
            text = rng.choice(FORMATS) + " " + text
        return text

    def pick(self, ops):
        """An operator, but not one with `>>` in an embedded expression."""
        if self.embedding:
            ops = [op for op in ops if ">>" not in op]
        return self.rng.choice(ops)

    def embedded(self, scope, depth, quote, nested=0):
        """The inside of a string with expressions embedded in it."""
        rng = self.rng
        # The reference crashes on text after an empty embedding, in a
        # string in double quotes, if nothing has come before the two
        # in the string or in the branch of an `if` that they are in.
        parts = []
        for _ in range(rng.randint(1, 4)):
            parts.append(rng.choice(WORDS))
            kind = rng.random()
            if 0.6 <= kind < 0.9 and quote == '"' and not "".join(parts):
                kind = 0
            if kind < 0.6:
                parts.append(f"<<{self.embedded_expr(scope, depth)}>>")
            elif kind < 0.85 and nested < 2:
                word = rng.choice(["if", "if", "unless"])
                parts.append(f"<<{word} {self.embedded_expr(scope, depth, True)}>>")
                parts.append(self.embedded(scope, depth - 1, quote, nested + 1))
                while rng.random() < 0.3:
                    other = rng.choice(["else if", "else unless", "otherwise if"])
                    parts.append(
                        f"<<{other} {self.embedded_expr(scope, depth, True)}>>"
                    )
                    parts.append(self.embedded(scope, depth - 1, quote, nested + 1))
                if rng.random() < 0.5:
                    parts.append(rng.choice(["<<else>>", "<<otherwise>>"]))
                    parts.append(self.embedded(scope, depth - 1, quote, nested + 1))
                if rng.random() < 0.8:
                    parts.append("<<end>>")
            elif kind < 0.9:
                parts.append("<<>>")
            elif kind < 0.91:
                parts.append(
                    rng.choice(
                        [
                            "<<one of>>a<<or>>b<<stopping>>",
                            "<<first time>>a<<only>>",
                            "<<end>>",
                            "<<else>>",
                            "<<or>>",
                            "<<only>>",
                        ]
                    )
                )
        parts.append(rng.choice(WORDS))
        return "".join(parts)

    def obj_expr(self, scope, depth):
        """Something to go on the left of a `.`."""
        rng = self.rng
        kind = rng.random()
        named = [obj[0] for obj in self.objs if obj[0]]
        if kind < 0.35 and named:
            return rng.choice(named)
        if kind < 0.5 and self.in_method:
            return "self"
        if kind < 0.85 and scope:
            return rng.choice(scope)
        if kind < 0.9:
            return f"({self.expr(scope, depth - 1)})"
        return self.member(scope, depth - 1)

    def member(self, scope, depth):
        """A property of something."""
        rng = self.rng
        obj = self.obj_expr(scope, depth)
        if rng.random() < 0.1:
            return f"{obj}.({self.expr(scope, depth - 1)})"
        return f"{obj}.{rng.choice(self.props)}"

    def named_args(self, args, scope, depth):
        """Now and then, put arguments passed by name among a call's."""
        rng = self.rng
        if rng.random() < 0.12:
            for name in rng.sample(NAMED, rng.randint(1, 2)):
                value = self.expr(scope, depth - 2)
                args.insert(rng.randint(0, len(args)), f"{name}: {value}")

    def constant(self):
        rng = self.rng
        kind = rng.random()
        if kind < 0.55:
            return str(
                rng.choice(
                    [
                        0,
                        1,
                        2,
                        3,
                        7,
                        100,
                        126,
                        127,
                        128,
                        255,
                        256,
                        1000,
                        65536,
                        2147483647,
                        rng.randint(0, 300),
                    ]
                )
            )
        if kind < 0.65:
            return rng.choice(["nil", "true"])
        if kind < 0.7:
            return (
                "[" + ", ".join(self.constant() for _ in range(rng.randint(0, 3))) + "]"
            )
        if kind < 0.87:
            return (
                "'" + rng.choice(["", "a", "abc", "hello", "x y", "one", "two"]) + "'"
            )
        named = [obj[0] for obj in self.objs if obj[0]]
        if kind < 0.93 and named:
            return rng.choice(named)
        return "-" + str(rng.choice([1, 2, 128, 129, 1000]))

    def expr(self, scope, depth):
        rng = self.rng
        if depth <= 0 or rng.random() < 0.25:
            if scope and rng.random() < 0.92:
                return rng.choice(scope)
            return self.constant()
        if rng.random() < 0.05:
            return self.anon_expr(scope, depth)
        kind = rng.random()
        named = [obj[0] for obj in self.objs if obj[0]]
        if named and rng.random() < 0.04:
            # A new object, or the address of a property.
            if rng.random() < 0.5:
                return "&" + rng.choice(self.props + PROPS[:3])
            count = rng.choice([None, 0, 1, 2])
            args = (
                ""
                if count is None
                else (
                    "("
                    + ", ".join(self.expr(scope, depth - 2) for _ in range(count))
                    + ")"
                )
            )
            return f"new {rng.choice(['', '', 'transient '])}{rng.choice(named)}{args}"
        if self.objs and rng.random() < 0.15:
            # A property, a call to a method, or in a method a property
            # of the object itself by its name alone.
            if self.in_method and rng.random() < 0.15:
                # What this method overrides, or something about it.
                text = rng.choice(
                    [
                        "inherited",
                        "inherited",
                        "targetprop",
                        "targetobj",
                        "definingobj",
                        "inherited." + rng.choice(self.props),
                    ]
                    + ["inherited " + name for name in named[:1]]
                )
                if text.startswith("inherited"):
                    count = rng.randint(0, 2)
                    text += (
                        "("
                        + ", ".join(self.expr(scope, depth - 2) for _ in range(count))
                        + ")"
                    )
                    return text
            elif self.in_method and rng.random() < 0.3:
                text = rng.choice(self.props + ["self"])
            else:
                text = self.member(scope, depth)
            if text.isalnum() and text not in PROPS:
                return text
            if text != "self" and rng.random() < 0.3:
                count = rng.randint(0, 2)
                args = ", ".join(self.expr(scope, depth - 2) for _ in range(count))
                if scope and rng.random() < 0.1:
                    args += (", " if args else "") + rng.choice(scope) + "..."
                text += f"({args})"
            return text
        if kind < 0.45:
            op = self.pick(BINARY)
            left, right = self.expr(scope, depth - 1), self.expr(scope, depth - 1)
            # The reference reports a constant division by zero once or
            # twice, depending on what is in its memory; leave the
            # plainest case of it out.
            if op in "/%" and right.strip("()") == "0":
                right = "3"
            return f"{left} {op} {right}"
        if kind < 0.55:
            return f"({self.expr(scope, depth - 1)})"
        if kind < 0.65:
            return rng.choice(["!", "-", "~", "+"]) + self.operand(scope, depth - 1)
        if kind < 0.69:
            name, fewest, most = rng.choice(BUILTINS)
            args = ", ".join(
                self.expr(scope, depth - 2) for _ in range(rng.randint(fewest, most))
            )
            return f"{name}({args})"
        if kind < 0.72:
            count = rng.randint(1, 3)
            return (
                "[" + ", ".join(self.expr(scope, depth - 2) for _ in range(count)) + "]"
            )
        if kind < 0.75:
            return f"{self.operand(scope, depth - 1)}[{self.expr(scope, depth - 2)}]"
        if kind < 0.79:
            name, argc, more = rng.choice(self.funcs[1:])
            count = argc + (rng.randint(0, 2) if more else 0)
            args = [self.expr(scope, depth - 2) for _ in range(count)]
            self.named_args(args, scope, depth)
            return f"{name}({', '.join(args)})"
        if kind < 0.81:
            return (
                f"({self.expr(scope, depth - 1)} ? {self.expr(scope, depth - 1)}"
                f" : {self.expr(scope, depth - 1)})"
            )
        if kind < 0.83:
            count = rng.randint(0, 3)
            items = ", ".join(self.expr(scope, depth - 2) for _ in range(count))
            word = rng.choice(["is in", "not in"])
            return f"({self.operand(scope, depth - 1)} {word} ({items}))"
        if kind < 0.87:
            return f"({self.expr(scope, depth - 1)} ?? {self.expr(scope, depth - 1)})"
        if kind < 0.89:
            return "'" + self.embedded(scope, depth - 2, "'") + "'"
        if kind < 0.95 and scope:
            target = rng.choice(scope)
            if self.objs and rng.random() < 0.25:
                target = self.member(scope, depth)
            elif self.in_method and rng.random() < 0.2:
                target = rng.choice(self.props)
            if rng.random() < 0.3:
                target += f"[{self.expr(scope, depth - 2)}]"
                if rng.random() < 0.2:
                    target += f"[{self.expr(scope, depth - 2)}]"
            if rng.random() < 0.5:
                return f"({target} {self.pick(ASSIGN)} {self.expr(scope, depth - 1)})"
            return rng.choice(
                [f"{target}++", f"{target}--", f"++{target}", f"--{target}"]
            )
        return f"({self.expr(scope, depth - 1)}, {self.expr(scope, depth - 1)})"

    def operand(self, scope, depth):
        """An expression that a unary operator can go in front of."""
        text = self.expr(scope, depth)
        return text if text.isalnum() else f"({text})"

    def block(self, scope, depth, indent, in_loop):
        scope = list(scope)
        declared = set()
        lines = []
        for _ in range(self.rng.randint(0, 5)):
            lines.extend(self.statement(scope, declared, depth, indent, in_loop))
        return lines

    def block_in_braces(self, scope, depth, indent, in_loop):
        pad = "    " * indent
        inner = self.block(scope, depth - 1, indent + 1, in_loop)
        return [pad + "{"] + inner + [pad + "}"]

    def body(self, scope, depth, indent, in_loop):
        """The statement a loop or an `if` governs."""
        rng = self.rng
        pad = "    " * indent
        if rng.random() < 0.6:
            inner = self.block(scope, depth - 1, indent + 1, in_loop)
            return [pad + "{"] + inner + [pad + "}"]
        if rng.random() < 0.15:
            return [pad + "    ;"]
        # A single statement; a declaration here belongs to the scope
        # around, but leave those to the blocks.
        return self.simple(scope, indent + 1, in_loop)

    def simple(self, scope, indent, in_loop):
        rng = self.rng
        pad = "    " * indent
        kind = rng.random()
        if kind < 0.04:
            return [pad + '"' + rng.choice(["", "text", "Hello!", "one two"]) + '";']
        if kind < 0.12:
            return [pad + '"' + self.embedded(scope, 2, '"') + '";']
        if kind < 0.5 and scope:
            target = rng.choice(scope)
            if self.objs and rng.random() < 0.25:
                target = self.member(scope, 2)
            elif self.in_method and rng.random() < 0.2:
                target = rng.choice(self.props)
            if rng.random() < 0.25:
                target += f"[{self.expr(scope, 2)}]"
            return [f"{pad}{target} {rng.choice(ASSIGN)} {self.expr(scope, 3)};"]
        if kind < 0.7:
            return [f"{pad}{self.expr(scope, 3)};"]
        if kind < 0.8 and in_loop:
            return [
                pad
                + rng.choice(["break;", "continue;"] if in_loop == 2 else ["break;"])
            ]
        if kind < 0.83:
            # A label for this is chosen once the whole function is
            # written, so that the jump can go forward.
            return [pad + "goto @LABEL@;"]
        if kind < 0.86 and self.around:
            name, on_loop = rng.choice(self.around)
            word = "continue" if on_loop and rng.random() < 0.5 else "break"
            return [f"{pad}{word} {name};"]
        if kind < 0.87:
            return [f"{pad}throw {self.expr(scope, 2)};"]
        if kind < 0.93:
            return [f"{pad}return {self.expr(scope, 3)};"]
        return [pad + "return;"]

    def statement(self, scope, declared, depth, indent, in_loop, force=None):
        rng = self.rng
        pad = "    " * indent
        kind = rng.random() if force is None else force
        if (depth <= 0 and (force is None or force < 0.35)) or kind < 0.35:
            return self.simple(scope, indent, in_loop)
        if force is None and rng.random() < 0.08:
            return self.for_in(scope, depth, indent)
        if force is None and rng.random() < 0.06:
            return self.anon_stmt(scope, declared, depth, indent)
        if kind < 0.5:
            name = rng.choice(NAMES)
            # A block cannot declare a name twice.
            if name in declared:
                return self.simple(scope, indent, in_loop)
            declared.add(name)
            line = f"{pad}local {name}"
            if rng.random() < 0.8:
                line += f" = {self.expr(scope, 3)}"
            scope.append(name)
            return [line + ";"]
        if kind < 0.68:
            lines = [f"{pad}if ({self.expr(scope, 3)})"]
            lines += self.body(scope, depth, indent, in_loop)
            if rng.random() < 0.5:
                lines += [pad + "else"] + self.body(scope, depth, indent, in_loop)
            return lines
        if kind < 0.78:
            return [f"{pad}while ({self.expr(scope, 3)})"] + self.body(
                scope, depth, indent, 2
            )
        if kind < 0.84:
            return (
                [pad + "do"]
                + self.body(scope, depth, indent, 2)
                + [f"{pad}while ({self.expr(scope, 3)});"]
            )
        if kind < 0.90:
            inner = list(scope)
            init = ""
            if rng.random() < 0.6:
                name = rng.choice(NAMES)
                init = f"local {name} = {self.expr(scope, 2)}"
                inner.append(name)
            elif scope and rng.random() < 0.7:
                init = f"{rng.choice(scope)} = {self.expr(scope, 2)}"
            cond = self.expr(inner, 2) if rng.random() < 0.8 else ""
            step = self.expr(inner, 2) if rng.random() < 0.8 else ""
            return [f"{pad}for ({init} ; {cond} ; {step})"] + self.body(
                inner, depth, indent, 2
            )
        if kind < 0.93:
            lines = [f"{pad}switch ({self.expr(scope, 2)})", pad + "{"]
            default_at = rng.randint(0, 5)
            for index in range(rng.randint(0, 4)):
                if index == default_at:
                    lines.append(pad + "default:")
                else:
                    lines.append(f"{pad}case {self.constant()}:")
                    if rng.random() < 0.2:
                        lines.append(f"{pad}case {self.constant()}:")
                inner = self.block(scope, depth - 1, indent + 1, max(in_loop, 1))
                lines += inner or [pad + "    ;"]
                if rng.random() < 0.7:
                    lines.append(pad + "    break;")
            return lines + [pad + "}"]
        if kind < 0.96:
            lines = [pad + "try"] + self.block_in_braces(scope, depth, indent, in_loop)
            named = [obj[0] for obj in self.objs if obj[0]]
            catches = rng.randint(0, 2) if named else 0
            for index in range(catches):
                var = f"exc{index}"
                lines.append(f"{pad}catch ({rng.choice(named)} {var})")
                lines += self.block_in_braces(scope + [var], depth, indent, in_loop)
            if catches == 0 or rng.random() < 0.5:
                lines += [pad + "finally"] + self.block_in_braces(
                    scope, depth, indent, in_loop
                )
            return lines
        if kind < 0.98:
            name = f"L{len(self.labels)}"
            self.labels.append(name)
            on_loop = rng.random() < 0.6
            self.around.append((name, on_loop))
            force = rng.uniform(0.69, 0.89) if on_loop else rng.uniform(0.0, 0.34)
            inner = self.statement(scope, declared, depth - 1, indent, in_loop, force)
            self.around.pop()
            return [f"{pad[4:]}{name}:"] + inner
        inner = self.block(scope, depth - 1, indent + 1, in_loop)
        return [pad + "{"] + inner + [pad + "}"]

    def obj_def(self, name, is_class, supers):
        """An object or a class, with its properties and methods."""
        rng = self.rng
        head = "class " if is_class else ""
        if name:
            head += name + ": "
        head += ", ".join(supers) if supers else "object"
        braces = rng.random() < 0.3
        lines = [head, "{"] if braces else [head]
        for prop in rng.sample(self.props, rng.randint(0, len(self.props))):
            kind = rng.random()
            if kind < 0.4:
                lines.append(f"    {prop} = {self.constant()}")
            elif kind < 0.6:
                self.in_method = True
                lines.append(f"    {prop} = ({self.expr(list(self.props), 2)})")
                self.in_method = False
            elif kind < 0.7:
                self.in_method = True
                lines.append(
                    f'    {prop} = "{self.embedded(list(self.props), 2, chr(34))}"'
                )
                self.in_method = False
            else:
                self.in_method = True
                body = self.function(prop, rng.randint(0, 2), rng.random() < 0.1)
                self.in_method = False
                lines += ["    " + line for line in body]
        if rng.random() < 0.1 and len(supers) < 2:
            self.in_method = True
            body = self.function("construct", rng.randint(0, 2), False)
            self.in_method = False
            lines += ["    " + line for line in body if "return " not in line]
        lines.append("}" if braces else ";")
        return lines

    def anon_expr(self, scope, depth):
        """A short anonymous function, perhaps called on the spot."""
        rng = self.rng
        params = rng.sample(NAMES, rng.randint(0, 2))
        inner = list(scope) + params
        own = ""
        if rng.random() < 0.15:
            name = rng.choice(NAMES)
            if name not in params:
                own = f"local {name} = {self.expr(inner, depth - 2)}, "
                inner.append(name)
        text = "{" + ", ".join(params) + ": " + own + self.expr(inner, depth - 1) + " }"
        if rng.random() < 0.5:
            args = ", ".join(self.expr(scope, depth - 2) for _ in params)
            text = f"({text})({args})"
        return text

    def anon_stmt(self, scope, declared, depth, indent):
        """A local given an anonymous function or method with a body."""
        rng = self.rng
        pad = "    " * indent
        params = rng.sample(NAMES, rng.randint(0, 2))
        word = rng.choice(["function", "function", "method", "new function"])
        # The function has labels and loops of its own.
        labels, around = self.labels, self.around
        self.labels, self.around = [], []
        lines = self.block(list(scope) + params, depth - 1, indent + 1, 0)
        for index, line in enumerate(lines):
            if "@LABEL@" in line:
                if self.labels:
                    lines[index] = line.replace("@LABEL@", rng.choice(self.labels))
                else:
                    lines[index] = line.replace("goto @LABEL@;", ";")
        self.labels, self.around = labels, around
        name = rng.choice(NAMES)
        if name in declared or (scope and rng.random() < 0.3):
            target = rng.choice(scope) if scope else "nosuch"
        else:
            declared.add(name)
            scope.append(name)
            target = f"local {name}"
        head = f"{pad}{target} = {word}({', '.join(params)})"
        return [head, pad + "{"] + lines + [pad + "};"]

    def for_in(self, scope, depth, indent):
        """A loop over a collection or a range."""
        rng = self.rng
        pad = "    " * indent
        name = rng.choice(NAMES)
        inner = list(scope)
        if rng.random() < 0.7 or not scope:
            var = f"local {name}"
            inner.append(name)
        else:
            var = rng.choice(scope)
        if rng.random() < 0.4:
            head = f"foreach ({var} in {self.expr(scope, 2)})"
        elif rng.random() < 0.5:
            head = f"for ({var} in {self.expr(scope, 2)})"
        else:
            step = rng.choice(["", "", " step 2", " step -1", f" step {self.expr(scope, 1)}"])
            head = f"for ({var} in {self.expr(scope, 2)}..{self.expr(scope, 2)}{step})"
        return [pad + head] + self.body(inner, depth - 1, indent, 2)

    def function(self, name, argc, more):
        rng = self.rng
        params = [f"p{index}" for index in range(argc)]
        scope = list(params)
        # Parameters that can be left out, and parameters passed by
        # name. A default value can use the parameters before it.
        if rng.random() < 0.2:
            for index in range(rng.randint(1, 2)):
                param = f"q{index}"
                kind = rng.random()
                if kind < 0.4:
                    params.append(f"{param}?")
                else:
                    params.append(f"{param} = {self.expr(scope, 2)}")
                scope.append(param)
        if rng.random() < 0.2:
            for param in rng.sample(NAMED, rng.randint(1, 2)):
                at = rng.randint(0, len(params))
                kind = rng.random()
                if kind < 0.3:
                    text = f"{param}:"
                elif kind < 0.5:
                    text = f"{param}:?"
                elif kind < 0.75:
                    text = f"{param}: = {self.constant()}"
                else:
                    text = f"{param}: = {self.expr(scope[:at], 2)}"
                params.insert(at, text)
                scope.insert(at, param)
        if more:
            if self.rng.random() < 0.5:
                params.append("...")
            else:
                params.append("[rest]")
                scope.append("rest")
        lines = [f"{name}({', '.join(params)})", "{"]
        self.labels = []
        self.around = []
        lines += self.block(scope, 3, 1, 0)
        lines.append("}")
        # Now that the function's labels are known, give each `goto` one.
        for index, line in enumerate(lines):
            if "@LABEL@" in line:
                if self.labels:
                    lines[index] = line.replace("@LABEL@", self.rng.choice(self.labels))
                else:
                    lines[index] = line.replace("goto @LABEL@;", ";")
        return lines

    def text(self):
        funcs = list(self.funcs)
        self.rng.shuffle(funcs)
        lines = []
        # A template's function is defined before the template or after,
        # or not at all.
        for index, (pattern, argc, _) in enumerate(self.templates):
            params = "x" if argc else ""
            define = f"tpl{index}({params}) {{ return {'x' if argc else '1'}; }}"
            where = self.rng.random()
            if where < 0.45:
                lines.append(define)
            lines.append(f"string template {pattern} tpl{index};")
            if 0.45 <= where < 0.97:
                lines.append(define)
        # Objects go among the functions, but each after the classes
        # it is made from, so that one with no name of its own is known
        # for an object when it is read.
        parts = [self.function(name, argc, more) for name, argc, more in funcs]
        place = 0
        for name, is_class, supers in self.objs:
            place = self.rng.randint(place, len(parts))
            parts.insert(place, self.obj_def(name, is_class, supers))
            place += 1
        for part in parts:
            lines += part
        return PRELUDE + "\n".join(lines) + "\n"


def main():
    if len(sys.argv) < 3:
        sys.exit(__doc__)
    folder, count = sys.argv[1], int(sys.argv[2])
    seed = int(sys.argv[3]) if len(sys.argv) > 3 else 1
    os.makedirs(folder, exist_ok=True)
    for index in range(count):
        rng = random.Random(seed * 100000 + index)
        with open(
            os.path.join(folder, f"r{seed}_{index:04}.t"), "w", newline="\n"
        ) as out:
            out.write(Program(rng).text())


main()
