#!/usr/bin/env python3
"""Write random programs for comparing compilers.

usage: gen-random.py <folder> <count> [seed]

Each program is made of the parts of the language Hexicon compiles so
far: functions, locals, the statements that need no objects (labels,
`goto`, `switch`, `try` with `finally`, and `throw` among them), and
expressions over numbers, strings, lists, locals, and calls to functions
and to built-in functions. The programs are
valid but mean nothing; they are for tools/compile-compare.sh, to find
source that the two compilers turn into different code.
"""

import os
import random
import sys

NAMES = ["a", "b", "c", "i", "n", "x", "y", "ab", "ba", "tmp", "count", "total",
         "index", "value", "result", "flag", "left", "right", "xx", "k9"]
BINARY = ["+", "-", "*", "/", "%", "&", "|", "^", "<<", ">>", ">>>",
          "==", "!=", "<", "<=", ">", ">=", "&&", "||"]
# Built-in functions to call: name, fewest arguments, most.
BUILTINS = [("tadsSay", 1, 3), ("toString", 1, 3), ("dataType", 1, 1),
            ("max", 1, 4), ("getArg", 1, 1), ("t3RunGC", 0, 0)]
PRELUDE = """intrinsic 't3vm/010006'
{
    t3RunGC();
    t3SetSay(val);
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
}
intrinsic 'tads-io/030007'
{
    tadsSay(val, ...);
}
"""
ASSIGN = ["=", "+=", "-=", "*=", "/=", "%=", "&=", "|=", "^=", "<<=", ">>=", ">>>="]


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

    def constant(self):
        rng = self.rng
        kind = rng.random()
        if kind < 0.55:
            return str(rng.choice([0, 1, 2, 3, 7, 100, 126, 127, 128, 255, 256, 1000,
                                   65536, 2147483647, rng.randint(0, 300)]))
        if kind < 0.65:
            return rng.choice(["nil", "true"])
        if kind < 0.7:
            return "[" + ", ".join(self.constant() for _ in range(rng.randint(0, 3))) + "]"
        if kind < 0.9:
            return "'" + rng.choice(["", "a", "abc", "hello", "x y", "one", "two"]) + "'"
        return "-" + str(rng.choice([1, 2, 128, 129, 1000]))

    def expr(self, scope, depth):
        rng = self.rng
        if depth <= 0 or rng.random() < 0.25:
            if scope and rng.random() < 0.92:
                return rng.choice(scope)
            return self.constant()
        kind = rng.random()
        if kind < 0.45:
            op = rng.choice(BINARY)
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
            args = ", ".join(self.expr(scope, depth - 2)
                             for _ in range(rng.randint(fewest, most)))
            return f"{name}({args})"
        if kind < 0.72:
            count = rng.randint(1, 3)
            return "[" + ", ".join(self.expr(scope, depth - 2) for _ in range(count)) + "]"
        if kind < 0.75:
            return f"{self.operand(scope, depth - 1)}[{self.expr(scope, depth - 2)}]"
        if kind < 0.79:
            name, argc, more = rng.choice(self.funcs[1:])
            count = argc + (rng.randint(0, 2) if more else 0)
            args = ", ".join(self.expr(scope, depth - 2) for _ in range(count))
            return f"{name}({args})"
        if kind < 0.83:
            return (f"({self.expr(scope, depth - 1)} ? {self.expr(scope, depth - 1)}"
                    f" : {self.expr(scope, depth - 1)})")
        if kind < 0.87:
            return f"({self.expr(scope, depth - 1)} ?? {self.expr(scope, depth - 1)})"
        if kind < 0.95 and scope:
            target = rng.choice(scope)
            if rng.random() < 0.3:
                target += f"[{self.expr(scope, depth - 2)}]"
                if rng.random() < 0.2:
                    target += f"[{self.expr(scope, depth - 2)}]"
            if rng.random() < 0.5:
                return f"({target} {rng.choice(ASSIGN)} {self.expr(scope, depth - 1)})"
            return rng.choice([f"{target}++", f"{target}--", f"++{target}", f"--{target}"])
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
        if kind < 0.06:
            return [pad + '"' + rng.choice(["", "text", "Hello!", "one two"]) + '";']
        if kind < 0.5 and scope:
            target = rng.choice(scope)
            if rng.random() < 0.25:
                target += f"[{self.expr(scope, 2)}]"
            return [f"{pad}{target} {rng.choice(ASSIGN)} {self.expr(scope, 3)};"]
        if kind < 0.7:
            return [f"{pad}{self.expr(scope, 3)};"]
        if kind < 0.8 and in_loop:
            return [pad + rng.choice(["break;", "continue;"] if in_loop == 2 else ["break;"])]
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
            return ([f"{pad}while ({self.expr(scope, 3)})"]
                    + self.body(scope, depth, indent, 2))
        if kind < 0.84:
            return ([pad + "do"] + self.body(scope, depth, indent, 2)
                    + [f"{pad}while ({self.expr(scope, 3)});"])
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
            return ([f"{pad}for ({init} ; {cond} ; {step})"]
                    + self.body(inner, depth, indent, 2))
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
            return ([pad + "try"] + self.body(scope, depth, indent, in_loop)
                    + [pad + "finally"] + self.body(scope, depth, indent, in_loop))
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

    def function(self, name, argc, more):
        params = [f"p{index}" for index in range(argc)]
        scope = list(params)
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
        for name, argc, more in funcs:
            lines += self.function(name, argc, more)
        return PRELUDE + "\n".join(lines) + "\n"


def main():
    if len(sys.argv) < 3:
        sys.exit(__doc__)
    folder, count = sys.argv[1], int(sys.argv[2])
    seed = int(sys.argv[3]) if len(sys.argv) > 3 else 1
    os.makedirs(folder, exist_ok=True)
    for index in range(count):
        rng = random.Random(seed * 100000 + index)
        with open(os.path.join(folder, f"r{seed}_{index:04}.t"), "w", newline="\n") as out:
            out.write(Program(rng).text())


main()
