#!/usr/bin/env python3
"""Write the small programs that each probe one case of the compiler.

usage: gen-cases.py <folder>

The programs are for tools/compile-compare.sh. Most are a line or two:
one way of writing something, or one mistake, to see that Hexicon makes
of it what the reference compiler does, messages included. A program
without a `_main` function is given an empty one. A program of several
source files is a folder of them.

Cases known to come out differently are not here; the compiler plan's
findings say what they are and why.
"""

import os
import sys

programs = {}

programs["jumps1"] = """
f(a, b, c)
{
    if (a) { if (b) return 1; else return 2; } else { if (c) return 3; }
    if (a) { if (b) { if (c) return 4; } }
    if (a) { while (b) { if (c) break; b--; } }
    while (a) { if (b) continue; if (c) return 5; a--; }
    while (a) { while (b) { if (c) break; else continue; } }
    if (a && b) { } else { return 6; }
    if (a || b) c = 1; else c = 2;
    if (!a) c = 3;
    if (!(a && b)) c = 4;
    if (!(a || b)) c = 5;
    if (!!a) c = 6;
    return a ? (b ? 1 : 2) : (c ? 3 : 4);
}
g(a, b, c)
{
    local x = a && b && c;
    local y = a || b || c;
    local z = (a && b) || c;
    local w = a && (b || c);
    local v = !a && !b;
    local u = (a ? b : c) && a;
    local t = a ?? b ?? c;
    local s = (a, b, c);
    local r = (a = b, c);
    a, b, c;
    return x + y + z + w + v + u + t + s + r;
}
h(a)
{
    do { if (a) return 1; } while (a);
    do { if (a) break; a++; } while (true);
    for (;;) { if (a) return 2; }
}
"""

programs["jumps2"] = """
f(a, b)
{
    while (a)
    {
        if (b) { a--; continue; }
        else { b++; break; }
    }
    if (a) return true; else return nil;
}
g(a, b)
{
    if (a) return true;
    if (b) return nil;
    return a == b;
}
h(a, b) { return a ? true : nil; }
i(a, b) { return a && b; }
j(a, b) { return a || b; }
k(a, b) { return !a; }
l(a, b) { if (a) { return; } else { return; } }
m(a, b) { if (a) return f(a, b); return g(a, b); }
n(a, b) { return a ? f(a, b) : g(a, b); }
o(a) { return a; a = 1; }
p(a) { for (;;) { } }
q(a) { while (true) { a++; } }
r(a) { do { a++; } while (true); }
"""

programs["manylocals"] = (
    "f(p0, p1, p2, p3, p4, p5)\n{\n"
    + "".join(f"    local v{i} = {i};\n" for i in range(300))
    + "    v260 = v270 + p4 + p5; v290 += 1; v290 += 500; v280++; --v299; v7 += 2; v2 -= v3;\n"
    + "    p4 = 1; p5 += 2; p0++; ++p1; p2 -= 1;\n"
    + "    v299 = nil; v298 = 0; v297 = 1; v5 = nil; v6 = 0; v7 = 1;\n"
    + "    return v0 + v5 + v6 + v255 + v256 + v299 + p0 + p3 + p4;\n}\n"
)

programs["funcs"] = """
function withKeyword(a) { return a; }
function declared;
function declared(a) { return a + 1; }
function kwNoParens { return 2; }
callsAll() { return withKeyword(1) + declared(2) + kwNoParens(); }
;
;;
"""

programs["folds"] = """
f(a)
{
    local x1 = 10 / 3, x2 = -10 / 3, x3 = 10 % 3, x4 = -10 % 3, x5 = 10 % -3;
    local x6 = 1 << 31, x7 = 1 << 32, x8 = 1 << 33, x9 = -1 >> 31, x10 = -1 >>> 31;
    local x11 = 2147483647, x12 = -2147483647, x13 = 0x7fffffff, x14 = 0xffffffff;
    local x15 = 1 + 2 * 3 - 4 / 2, x16 = (1, 2, 3), x17 = 1 == 1, x18 = 1 != 1;
    local x19 = 1 < 2, x20 = 2 <= 2, x21 = 3 > 4, x22 = 4 >= 5;
    local x23 = nil == nil, x24 = true == true, x25 = nil == true, x26 = nil != 0;
    local x27 = 'a' + 'b', x28 = 'a' + 1, x29 = 1 + 'a', x30 = 'x' + true;
    local x31 = !nil, x32 = !true, x33 = !0, x34 = !1, x35 = !'s';
    local x36 = true && true, x37 = true && nil, x38 = nil || nil, x39 = 1 && 2;
    local x40 = 0 || 5, x41 = 0 && 5, x42 = 'a' == 'a', x43 = 'a' == 'b';
    local x44 = 'abc' < 'abd', x45 = 'b' > 'abc', x46 = 'a' <= 'a';
    local x47 = true ? 1 : 2, x48 = nil ? 1 : 2, x49 = 0 ? 1 : 2, x50 = 5 ?? 6;
    local x51 = a + 1 + 2, x52 = 1 + 2 + a, x53 = a + (1 + 2), x54 = 2 * 3 * a;
    local x55 = f == f, x56 = f != f, x57 = f == nil, x58 = (f, 1);
    local x59 = -0, x60 = - -1, x61 = ~~5, x62 = +5, x63 = 127 + 1, x64 = 126 + 1;
    local x65 = a && true, x66 = a || nil, x67 = true && a, x68 = nil || a;
    local x69 = 1 ? a : 2, x70 = a ?? 1, x71 = nil ?? a;
    return x1 + x2 + x3 + x4 + x5 + x6 + x7 + x8 + x9 + x10 + x11 + x12 + x13
        + x14 + x15 + x16 + x17 + x18 + x19 + x20 + x21 + x22 + x23 + x24 + x25
        + x26 + x27 + x28 + x29 + x30 + x31 + x32 + x33 + x34 + x35 + x36 + x37
        + x38 + x39 + x40 + x41 + x42 + x43 + x44 + x45 + x46 + x47 + x48 + x49
        + x50 + x51 + x52 + x53 + x54 + x55 + x56 + x57 + x58 + x59 + x60 + x61
        + x62 + x63 + x64 + x65 + x66 + x67 + x68 + x69 + x70 + x71;
}
"""

programs["asi"] = """
f(a, b)
{
    local x, y, z;
    x = y = z = a;
    x = (y = 1) + (z = 2);
    x += y += z;
    x = y++ + ++z;
    x = y-- - --z;
    x = a++ + ++b;
    x = a-- - --b;
    a += 1; a -= 1; a *= 2; b = a += 3;
    x++; ++x; x--; --x; a++; ++a; a--; --a;
    y = x++; y = ++x; y = x--; y = --x; y = a++; y = ++a;
    y = (x += 1); y = (x += 100); y = (x += 1000); y = (x += z); y = (x -= z); y = (x -= 1);
    y = (x *= 2); y = (a *= 2); y = (a += 2); y = (a -= 2);
    x = nil; x = 0; x = 1; x = 2; y = (x = nil); y = (x = 0); y = (x = 1);
    a = nil; a = 0; a = 1; y = (a = nil);
    x = x; x = y; x = a; a = x; a = b;
    f = 1;
    return x + y + z;
}
"""

programs["noret"] = """
none() { }
one(a) { return a; }
f(t)
{
    t += t ? one(1) : none();
    if (t ? one(1) : none()) return 1;
    if ((++t ? -2 : none())) return 1;
    t = (t ? 2 : none());
    return t;
}
"""

# Nothing at all in the constant pool.
programs["nopool"] = "_main(a) { }\n"
programs["nopool2"] = "_main() { }\n"

# The preprocessor's messages come from both readings of the source.
programs["ppwarn"] = "#define A 1\n#define A 2\n_main(args) { return A; }\n"
programs["pperror"] = "#error stop here\n_main(args) { return 1; }\n"
programs["ppinclude"] = '#include "nosuchfile.h"\n_main(args) { return 1; }\n'
programs["ppmacro"] = (
    "#define ADD(a, b) ((a) + (b))\n#define TWICE(x) ADD(x, x)\n"
    "_main(args) { return TWICE(args) + __LINE__; }\n"
)
programs["ppif"] = (
    "#ifdef __TADS3\n_main(args) { return __TADS_VERSION_MAJOR; }\n"
    "#else\n_main(args) { return 2; }\n#endif\n"
)

# Strings that decide the size of the constant pool's pages.
programs["str1000"] = "_main(args) { return '" + "x" * 1000 + "'; }\n"
programs["str1100"] = "_main(args) { return '" + "y" * 1100 + "'; }\n"
programs["str5000"] = (
    "_main(args) { return '" + "z" * 5000 + "' + '" + "w" * 3000 + "'; }\n"
)
programs["str20000"] = "_main(args) { return '" + "v" * 20000 + "'; }\n"
programs["strmany"] = (
    "_main(args)\n{\n"
    + "".join(
        f"    if (args == {i}) return 'string number {i} " + "s" * (i % 37) + "';\n"
        for i in range(400)
    )
    + "    return nil;\n}\n"
)

# Many functions, calling forward and back.
programs["manyfuncs"] = (
    "".join(
        f"f{i}(a) {{ return f{(i * 7 + 3) % 200}(a + {i}) + f{(i + 1) % 200}(a); }}\n"
        for i in range(200)
    )
    + "_main(args) { return f0(1); }\n"
)

programs["escapes"] = (
    "_main(args) { return 'a\\nb\\tc\\\\d\\'e\\\"f\\x41\\u0042\\101 \\^x\\vy\\  z\\b'; }\n"
)
# A source file that does not say what character set it is in.
programs["unicode"] = "_main(args) { return 'café 中文'; }\n"
# Names as long as a name can be, and longer.
programs["longname"] = (
    "_main(args)\n{\n    local " + "n" * 80 + " = 1;\n    local " + "m" * 81 + " = 2;\n"
    "    return " + "n" * 80 + " + " + "m" * 81 + ";\n}\n"
)
programs["main_last"] = (
    "a() { return 1; }\nb() { return 2; }\n_main(args) { return a() + b(); }\n"
)
programs["recursion"] = (
    "_main(args) { return _main(args) + f(args); }\n"
    "f(x) { return x ? f(x - 1) : _main(x); }\n"
)
programs["comments"] = (
    "/* a comment */ _main(args) // another\n{\n    /* inside */ return 1; // end\n}\n"
)
programs["hex"] = "_main(args) { return 0x10 + 0xFF + 010 + 0 + 0x0 + 00; }\n"
programs["mulover"] = "_main(args) { return 65536 * 65536 + 100000 * 100000; }\n"


def adds(count):
    return "".join(f"    x += {i * 1000 + 300};\n" for i in range(count))


# Functions longer than a page of code, and enough code for several pages.
programs["big"] = (
    "_main(args)\n{\n    local x = 0;\n"
    + adds(400)
    + "    return x + f1(1) + f2(2) + f3(3) + last('s');\n}\n"
    + "".join(
        f"f{k}(x)\n{{\n" + adds(150 * k) + "    return x;\n}\n" for k in (1, 2, 3)
    )
    + "last(s) { return s + 'tail'; }\n"
)

# The built-in functions most programs declare.
BUILTINS = """intrinsic 't3vm/010006'
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
}
intrinsic 'tads-io/030007'
{
    tadsSay(val, ...);
}
"""

programs["bif1"] = (
    BUILTINS
    + """
_main(args)
{
    tadsSay('hello\n');
    tadsSay('a', 'b', 1);
    local t = dataType(args);
    local m = max(1, 2, 3) + min(4, 5);
    local s = toString(m) + toString(m, 16) + toString(m, 16, nil);
    t3RunGC();
    t3RunGC;
    local v = t3GetVMVsn;
    tadsSay(args...);
    tadsSay(t, args..., s, v);
    return getArg(1) + rand(10);
}
"""
)

# Sets declared more than once, with and without versions. (A set first
# declared without a version and then with one is not here: the
# reference leaves whatever was in its memory after the name.)
programs["bif2"] = """
intrinsic 'tads-gen/030001' { dataType(val); }
intrinsic 'tads-gen/030008' { dataType2(val); second(a?); }
intrinsic 'tads-gen/030005' { third(a, b?, c?); }
intrinsic 'five' { f0(); }
intrinsic 'six/1' { g0(); }
intrinsic 'seven/123456789' { h0(); }
intrinsic 'seven/2' { h1(); }
_main(args)
{
    return dataType(1) + dataType2(2) + second() + second(1) + third(1) + third(1, 2, 3)
        + f0() + g0() + h0() + h1();
}
"""

# A big set. (Calling a function numbered above 255 is not here: the
# reference crashes.)
programs["bif3"] = (
    "intrinsic 'many/1'\n{\n"
    + "".join(f"    fn{i}();\n" for i in range(300))
    + "}\n_main(args) { return fn0() + fn255(); }\n"
)

programs["dstr1"] = (
    BUILTINS
    + """
_main(args)
{
    "Hello, world!\n";
    "second";
    "Hello, world!\n";
    if (args) "yes"; else "no";
    args ? "one" : "two";
    args ? "one" : tadsSay('two');
    "a", "b";
    for (local i = 0 ; i < 3 ; ++i) "loop ";
    "";
}
"""
)

programs["list1"] = """
f(a, b)
{
    local e = [];
    local c = [1, 2, 3];
    local s = ['a', 'b', 'a'];
    local n = [1, 'two', nil, true, [3, 4], [], f, ['x', ['y']]];
    local v = [a, b];
    local w = [a, 1, 'x', [b]];
    local i1 = c[1], i2 = c[a], i3 = [10, 20, 30][2], i4 = [a, b][1], i5 = n[5][1];
    local j = [1, 2] + [3], k = [1, 2] + 3, l = [1, 2, 3, 2] - 2, m = [1, 2, 3] - [1, 3];
    local o = [1] + 'x', p = 'x' + [1], q = [1, 2] == [1, 2], r = [1] != [2];
    local t = c + a, u = c - a, x = [[1, 2], [3, 4]][2][1];
    [1, 2, 3];
    [a, b];
    c[1];
    return e + c + s + n + v + w + i1 + i2 + i3 + i4 + i5 + j + k + l + m + o + p + q + r + t + u + x;
}
_main(args) { return f(1, 2); }
"""

# A constant list whose value the reference overwrites in place: it
# passes for true or nil, and is still generated as the list.
programs["list_bool"] = """
f(a)
{
    local q = [1, 2] == [1, 2], r = [1] != [2], s = ![1], t = [1] || a, u = [1] == 1;
    local v = [[1] == [1], 2], w = ([1] == [1]) ? 3 : 4;
    if ([1] == [1]) a = 1;
    if ([1] == [2]) a = 2;
    a = ([1] == [1]);
    a = ([1] == [2]);
    return [1] == [1];
}
g() { return [1] == [2]; }
_main(args) { return f(1) + g(); }
"""

programs["list2"] = """
f(a, b, i)
{
    local c = [1, 2, 3];
    c[1] = a;
    c[i] = b;
    c[2] = c[3];
    c[i + 1] = 5;
    a[1] = 2;
    a[i] = 3;
    local d = (c[1] = 9);
    d = (a[1] = 9);
    c[1] += 2;
    c[i] -= a;
    c[1]++;
    ++c[i];
    d = c[1]++;
    d = --c[i];
    d = (c[2] *= 3);
    a[1] += 1;
    a[i]++;
    c[1][2] = 3;
    c[i][a] = b;
    c[1][2][3] = 4;
    c[1][2] += 4;
    c[1][2]++;
    d = (c[1][2] = 7);
    f(1, 2, 3)[1] = 2;
    [1, 2][1] = 3;
    c[0] = 1; c[127] = 1; c[128] = 1; c[255] = 1; c[256] = 1; c[-1] = 1;
    return c + d;
}
_main(args) { return f(1, 2, 3); }
"""

# Lists that decide the size of the constant pool's pages, and one too
# long for the short instruction that makes a list.
programs["list3"] = (
    "_main(args)\n{\n    local big = ["
    + ", ".join(str(i) for i in range(600))
    + "];\n    local dyn = [args, "
    + ", ".join(str(i) for i in range(300))
    + "];\n    return big + dyn;\n}\n"
)
programs["list4"] = (
    "_main(args)\n{\n    local dyn = [args, "
    + ", ".join(str(i) for i in range(900))
    + "];\n    return dyn;\n}\n"
)

# Indexing a local with a small constant has instructions of its own.
programs["locals_idx"] = """
f(p0)
{
    local a = [1], b = [2], c = [3], d = [4], e = [5], f6 = [6], g = [7];
    a[1] = 1; b[2] = 2; g[3] = 3; g[p0] = 4; a[p0] = 5; p0[1] = 6;
    a[200] = 1; a[1000] = 1;
    return a[1] + b[2] + g[3] + g[p0] + a[p0] + p0[1] + a[0] + a[200] + a[1000] + g[1000]
        + c + d + e + f6;
}
_main(args) { return f(1); }
"""

# Every kind of constant as a case, cases that repeat, and labels among
# the cases.
programs["switch1"] = """
f(x)
{
    switch (x)
    {
    case 0: case 1: case 126: case 127: case 128: case -1: case 65536: case 2147483647:
        return 1;
    case 'a': case 'a': case '': case 'a string that is too long to be shared with another like it':
        return 2;
    case nil: case true:
        return 3;
    case []: case [1]: case [1, 'a', [nil, true]]:
        return 4;
    case f: case _main:
        return 5;
    case 1: case 2 * 3 + 1: case 'a' + 'b': case [1] + 2: case 1 == 1: case [1] == [1]:
        return 6;
    case (1, 2):
    here:
    case 9:
    default:
    there:
        return 7;
    }
}
_main(args) { return f(args); }
"""

# Where the end of a switch can and cannot be reached.
programs["switch2"] = """
a(x) { switch (x) { case 1: return 1; default: return 2; } }
b(x) { switch (x) { case 1: return 1; default: break; } return 3; }
c(x) { switch (x) { case 1: x = 2; default: x = 3; } return x; }
d(x) { switch (x) { default: while (true) { if (x) break; } } return x; }
e(x) { for (;;) { switch (x) { case 1: continue; default: return 2; } } }
g(x) { out: switch (x) { case 1: break out; default: return 2; } return 3; }
h(x) { switch (x) { case 1: throw x; default: goto done; } done: return 3; }
i(x) { switch (x) { case 1: break; break; case 2: return 1; x = 1; x = 2; default: return 2; } return x; }
j(x) { switch (x) { case 1: if (true) return 1; else break; default: return 2; } }
k(x) { switch (x) { case 1: if (nil) return 1; else break; default: return 2; } return 3; }
_main(args) { return a(1) + b(1) + c(1) + d(1) + e(1) + g(1) + h(1) + i(1) + j(1) + k(1); }
"""

programs["goto1"] = """
f(x)
{
    goto a;
    {
    a:
        x = 1;
        {
            goto b;
        }
    }
b:
    if (x) goto c; else goto a;
c:
d:
e:
    goto d;
    goto e;
}
g(x)
{
    while (x)
    {
    top:
        x--;
        if (x == 5) goto top;
        if (x == 6) goto out;
        if (x == 7) break;
        if (x == 8) continue;
    }
out:
    do { goto top; } while (x);
last: ;
    goto last;
}
_main(args) { return f(1) + g(2); }
"""

# break and continue with labels, through every kind of statement.
programs["label1"] = """
f(x)
{
a:  for (;;) {
b:      while (x) {
c:          do {
d:              switch (x) {
                case 1: break a;
                case 2: break b;
                case 3: break c;
                case 4: break d;
                case 5: continue a;
                case 6: continue b;
                case 7: continue c;
                case 8: break;
                case 9: continue;
                }
            } while (x);
        }
    }
e:  x = 1;
g:  { x = 2; if (x) break g; x = 3; }
h:  if (x) break h; else { break h; }
i: j: k: while (x) { if (x == 1) continue i; if (x == 2) continue j; if (x == 3) break k; continue k; }
    return x;
}
_main(args) { return f(1); }
"""

# The locals a finally block needs, and the one a return through it
# needs, come after the function's own.
programs["finally1"] = """
a(x) { local l1 = 1; try { return l1 + x; } finally { local l2 = 2; x = l2; } }
b(x) { try { try { return x; } finally { x = 1; } } finally { x = 2; } }
c(x) { try { return; } finally { x = 1; } }
d(x) { try { return nil; } finally { x = 1; } }
e(x) { try { return true; } finally { x = 1; } }
g(x) { try { return x; } finally { return 2; } }
h(x) { try { x = 1; } finally { throw x; } }
i(x) { try { throw x; } finally { } }
j(x) { try { } finally { } return x; }
k(x) { try ; finally ; return x; }
l(x) { try x = 1; finally x = 2; return x; }
m(x) { try { return a(x); } finally { x = 1; } }
n(x) { try { "text"; return "more"; } finally { x = 1; } }
o(x) { try { return x; } finally { try { return x + 1; } finally { x = 3; } } }
q(x) { while (x) { try { if (x == 1) break; if (x == 2) continue; if (x == 3) return 3; } finally { x--; } } return x; }
r(x) { try { local t1 = x; try { local t2 = t1; return t2; } finally { local t3 = 3; x = t3; } } finally { local t4 = 4; x = t4; } }
_main(args)
{
    return a(1) + b(1) + c(1) + d(1) + e(1) + g(1) + h(1) + i(1) + j(1) + k(1) + l(1) + m(1)
        + n(1) + o(1) + q(1) + r(1);
}
"""

# goto out of, within, and between protected blocks.
programs["finally2"] = """
a(x)
{
    try { if (x) goto out; x = 1; inside: x = 2; goto inside; } finally { x = 3; }
out:
    return x;
}
b(x)
{
    try { try { goto out; } finally { x = 1; } } finally { x = 2; }
out:
    try { goto out; } finally { x = 3; }
}
c(x)
{
    try { x = 1; } finally { here: x = 2; if (x) goto here; }
    goto in;
    try { in: x = 4; } finally { x = 5; }
    return x;
}
d(x)
{
lbl:
    while (x)
    {
        try { try { if (x == 1) break lbl; if (x == 2) continue lbl; if (x == 3) goto lbl; }
              finally { x = 1; } }
        finally { x = 2; }
    }
    return x;
}
e(x)
{
    switch (x)
    {
    case 1:
        try { break; } finally { x = 2; }
    case 2:
        try { return x; } finally { x = 3; }
    }
    return x;
}
_main(args) { return a(1) + b(1) + c(1) + d(1) + e(1); }
"""

programs["throw1"] = """
f(x) { throw x; }
g(x) { throw 'text'; x = 1; }
h(x) { if (x) throw [1, 2]; else throw x + 1; }
i(x) { throw "text"; }
_main(args) { return f(1) + g(1) + h(1) + i(1); }
"""

# Mistakes, and code that draws a warning. Each is a program of its own,
# since what a compiler does after an error depends on how it recovers.
SPRINTF = """intrinsic 'tads-gen/030008'
{
    dataType(val); getArg(idx); firstObj(cls?, flags?); nextObj(obj, cls?, flags?);
    randomize(...); rand(x, ...); toString(val, radix?, isSigned?); toInteger(val, radix?);
    getTime(typ?); rexMatch(pat, str, idx?); rexSearch(pat, str, idx?); rexGroup(n);
    rexReplace(pat, str, repl, flags?, idx?); savepoint(); undo(); saveGame(fname, metaTab?);
    restoreGame(fname); restartGame(); max(val, ...); min(val, ...); makeString(val, rpt?);
    getFuncParams(func); toNumber(val, radix?); sprintf(fmt, ...);
}
"""

programs["embed1"] = """
f() { return 1; }
g() { }
_main(args)
{
    "a<<1>>b"; "<<args>>"; "x<<f()>>"; "<<g()>>y"; "<<>>"; "a<<>>b<<  >>";
    "<<args>><<args>>"; "<<1, 2>>"; "<<args = 1>><<args += 2, args>>";
    "a<<"b<<1>>c">>d"; "<<'x<<1>>y'>>"; "<<'q<<"r<<args>>s">>t'>>";
    "a<<
      args
      + 1>>b
      c";
    "<<args ? "x" : "y">>c<<args ? f() : 1>><<args ? g() : g()>>";
    "<<[1, 2][1]>><<[args]>><<args[1]>><<f>><<nil>><<true>>";
}
"""

programs["embed2"] = """
f() { return 1; }
g() { }
_main(args)
{
    local s = 'p<<args>>q<<3>>';
    s = '<<1>><<2>>'; s = 'a<<1>>b'; s = '<<>>'; s = '<<args>>'; s = 'a<<>>b';
    s = 'a<<'b'>>c' + 'p<<'q<<args>>r'>>s';
    s = '<<1, 2>><<args>>' + ['a<<1>>', 'b<<args>>'];
    s = 'a<<f()>>b<<g()>>c<<f>>';
    s = 'a<<args>>b'[1];
    s += 'x<<s>>' + '<<nil>><<true>><<[1]>><<[args]>>';
    switch (args)
    {
    case 'x<<1>>y': return 1;
    case 'x<<if nil>>a<<else>>b<<end>>': return s;
    case 'a<<1>>' == 'a1': return 2;
    }
    return 'a<<1>>' == 'a1';
}
"""

programs["embed3"] = """
_main(args)
{
    "a<<if args>>yes<<else>>no<<end>>z"; "<<if args>>yes"; "<<if args>><<else>>";
    "<<unless args>>u<<else if args > 1>>v<<otherwise>>w<<end>>"; "<<if args>><<end>>";
    "<<if args>>a<<if args > 1>>b<<else>>c<<end>>d<<else>>e<<if 1>>f<<end>><<end>>g";
    "<<if args>>one<<else>>two"; "<<unless args>><<otherwise unless args>>x<<end>>";
    "<<if args>>a<<else unless args>>b<<otherwise if args>>c<<else>>d<<end>>e";
    "<<if nil>>a<<end>><<if true>>b<<end>><<unless 1>>c<<else>>d<<end>>";
    "<<if args = 1>>a<<end>>"; "<<if args>><<args>><<else>><<args + 1>><<end>>";
    "<<if args>>a<<end>><<end>>"; "<<end>><<only>><<or>><<stopping>><<otherwise>>";
    local s = 'q<<if args>>yes<<end>>';
    s = '<<if args>>a<<else unless args>>b<<otherwise if 3>>c';
    s = '<<if nil>>a<<else if true>>b<<end>>';
    s = '<<if args>><<end>>' + '<<if args>>a<<else>>';
    s = '<<if args>><<args>><<else>><<if s>>x<<end>>y<<end>>z<<unless args>>w';
    return s;
}
"""

programs["embed4"] = (
    SPRINTF
    + """
_main(args)
{
    "<<%d args>> <<%5.2f args + 1>><<%s 'x' >><<% d 5>><<%-5.2f 1>><<%% 3>>";
    "<<%x args, 2>><<%d if>><<if args>><<%d args>><<end>>";
    local s = '<<%x args>>' + '<<%s s>><<%c 65>>';
    local sprintf = 1;
    "<<%d args>>";
    return s + sprintf;
}
"""
)

programs["embed5"] = """
string template <<* in words>> spell;
string template <<nope>> nope;
string template <<the * thing>> spell;
string template <<>> nope;
string template << * >> spell;
string template <<the * of *>> pair;
string template <<big *>> big;
string template <<* said>> said;
string template <<'*' x>> spell;
string template <<if only>> nope;
spell(x) { return x; }
nope() { return 'n'; }
pair(x) { return x; }
big(x) { return x; }
said(x) { return x; }
_main(args)
{
    "<<args in words>> <<nope>> <<the args in words thing>> <<args>>";
    "<<the 1 of 2>> <<the big 1 thing>> <<big the 2 thing>> <<big nope>> <<nope said>>";
    "<<big big 1 + 2 said>> <<the nope thing>> <<the thing>> <<big>> <<nope nope>>";
    "<<1 x>> <<'*' x>> <<if only>>a<<end>> <<%d nope>> <<%d args in words>>";
    local s = '<<nope>><<the 1 thing>><<args, 2>>';
    local spell = 1;
    "<<spell in words>>";
    return s;
}
"""

programs["embed6"] = """
string template <<a>> fa;
string template <<b>> fb;
string template <<c *>> fc;
used() { return 1; }
string template <<d>> used;
later() { }
string template <<e>> later;
string template <<f g h i j>> later;
_main(args) { local q = 'x<<d>>'; return q; }
"""

programs["obj1"] = """
class Animal: object
    name = 'animal'
    legs = 4
    describe() { return name + ' has ' + legs; }
    speak(what) { "<<name>> says <<what>>"; }
;
dog: Animal
    name = 'dog'
    owner = nil
    bark() { speak('woof'); return self.describe(); }
;
cat: Animal { name = 'cat' lives = 9 friend = dog }
_main(args) { dog.bark(); cat.speak('meow'); return cat.friend.name; }
"""

programs["obj2"] = """
box: object
    contents = [1, 2, 'three']
    desc = "A box. "
    size = 2 + 3
    total = (size * 2)
    label = 'box ' + 'one'
    open = true
    check() { size = 5; self.size += 1; total++; return open; }
;
_main(args)
{
    box.size = 3; box.size += 2; box.total++; --box.size;
    local b = box; b.size = 1; b.size *= 2; b.check();
    return b.contents;
}
"""

programs["obj3"] = """
class Base: object
    v = 1
;
Base
    v = 2
;
object
    v = 3
;
Base v = 4;
class C: object;
C x = 1;
C { y = 2 }
object { z = 3 };
a: object { x = 1; y = 2; m() { } };
b: object { x = 1 }
c: object
{
    x = 1
}
"""

programs["obj4"] = """
a: object
    f(x) { return x.name; }
    g(x, p) { return x.(p); }
    h(x) { return x.name(1, 2); }
    i(x, p) { return x.(p)(3); }
    j() { return self.(name); }
    k(p) { self.(p) = 1; self.(p) += 2; a.(p) = 3; a.(p)++; }
    l(x, p) { x.(p) = 1; x.(p) -= 2; x.name = 3; x.name /= 4; return x.name++; }
    m(x) { local y = x.name; y.name.name = x; x.name.name(1).name += 1; return y.(x).(y); }
    n(x...) { return self.m(x...) + m(x...) + a.m(x...) + x.m(x...) + x.(m)(x...) + self.(x)(x...) + a.(x)(x...); }
    o(x) { ++self.name; self.name--; name += 1; --name; x.(name)--; return self.name(1) + name(2, 3); }
    name = 'a'
;
_main(args)
{
    args.x.y.z = 1; args[1].x = 2; args.x[1] = 3; args.x[1].y++;
    return a.f(a) + a.g(a, 1) + args.length() + args.length;
}
"""

programs["obj5"] = """
a: object
    m() { return undefinedProp + other(1) ; }
    n() { newProp = 1; return self; }
;
f() { return a.neverDefined; }
_main(args) { a.alsoNew = 1; a.callIt(1, 2); return f(); }
"""

programs["obj6"] = """
class A: object a = 1;
class B: object b = 2;
c: A, B c = 3;
d: A, B construct(x) { c = x; } ;
class E: A, B;
A, B, E;
f: A, B construct() { if (a) return; b = 1; };
g: object construct(x, y) { a = x; b = y; };
"""

programs["obj7"] = """
class Err: object msg = 'err';
class Other: Err;
f(x)
{
    try { throw x; }
    catch (Err e) { return e.msg; }
    catch (Other o) { return o; }
    finally { "done"; }
}
_main(args) { try { f(1); } catch (Err e) { } }
"""

programs["obj8"] = """
a: object x = 1;
b: object x = 2;
_main(args)
{
    local l = [a, b, a.x];
    if (a == b) return 1;
    if (a == a) return 2;
    if (a != nil) return [a] + b;
    switch (args) { case a: return 3; case b: return 4; }
    return a + 1 + l + (a - 1) + 'x' + a + (a ? 1 : 2) + (a && b) + !a;
}
"""

programs["obj9"] = """
ab: object ab1 = 1 ba1 = 2 x = 3;
ba: object;
cab: object;
bca: object bac = 1 cab1 = 2 abc = 3;
acb: object;
f(o) { o.bca1 = 1; o.acb1 = 2; o.cba1 = 3; o.x1 = 4; return o.y1 + o.abc + o.ba1; }
g(o) { return o.b1ca + o.a1bc + o.c1ab + o.ab1 + o.abc; }
_main(args) { return [ab, ba, cab, bca, acb]; }
"""

programs["obj10"] = """
a: object
    x = 1
    m() { return 1; }
    o { return 3; }
    p(a, b) { return a + b; }
    q = (1, 2)
    r = [x, 1]
    s = self
    t = "<<x>>"
    u = 'a<<x>>'
    v = (x = 2)
    w = nil
    y = true
    z = [nil, true, a, [a]]
    aa = "plain"
    bb = (m(), p(1, 2))
    cc = a
    dd = a.x
    ee = (self.x ? 'yes' : 'no')
    ff = [1, 2][1]
    gg = -1
    hh = 'a' + 1
    ii = _main
    jj = nil + 1
;
_main(args) { "<<a.x>><<a.m()>><<a.t>>"; return 'x<<a.u>>y'; }
"""

programs["obj11"] = """
a: object
    m(x)
    {
        local unused;
        local n = 0;
    again:
        for (local i = 1 ; i <= x ; ++i)
        {
            try { if (i == limit) break again; n += step; }
            finally { count++; }
        }
        switch (n) { case 1: return self; default: return n; }
        n = 2;
    }
    limit = 10
    step = 2
    count = 0
;
_main(args) { return a.m(5); }
"""

programs["obj12"] = """
class C: object;
f(args)
{
    try { args = 2; }
    catch (C e) { do { return args; } while (args); }
    finally { args = 3; }
}
g(args)
{
    try { args = 2; }
    catch (C e) { local x = 1; local left = 2; do { return (x + left); } while (args); }
    finally { goto L0; args; L0: while (args) { } do { } while (args); }
}
h(args)
{
    try { args = 1; }
    finally { try { args = 2; } catch (C e) { return args; } finally { args = 3; } }
}
_main(args)
{
    out: while (args)
    {
        try { args = 2; }
        catch (C e)
        {
            if (args) break;
            if (args > 1) goto here;
            while (args)
            {
                if (args) break out;
                if (args > 2) continue out;
                if (args > 3) goto here;
                if (args > 4) goto away;
                return 1;
            }
        here:
            switch (args) { case 1: break out; case 2: goto away; default: return 2; }
            a: { break a; }
            b: { if (args) break out; }
        }
        finally { args = 3; }
    }
away:
    return f(args) + g(args) + h(args);
}
"""

programs["inh1"] = """
class A: object
    name = 'a'
    m(x) { return x; }
    construct(n) { name = n; }
;
class B: A
    m(x) { return inherited(x) + inherited.m(x) + inherited A.m(x) + inherited.name; }
    n([x]) { return inherited(x...) + inherited.m(x...) + inherited A.(x)(x...) + inherited.(x)(1); }
    o() { inherited; inherited(); inherited.o; inherited A; inherited A(); return inherited A.o; }
    construct(n) { inherited(n); }
    p() { return [targetprop, targetobj, definingobj, invokee, self.targetprop, self.targetprop(1)]; }
    q(x) { return x.targetprop + inherited.targetprop + inherited A.targetprop(1); }
;
_main(args)
{
    local b = new B('x');
    local c = new A(args...);
    local d = new transient B(1, 2);
    new B;
    return [b, c, d, new A, new A(), new transient A];
}
"""

programs["addr1"] = """
a: object x = 1 y = &x z = [&x, &w, &a, &_main] m() { return &q; };
f() { }
_main(args)
{
    local p = &x;
    local l = [&x, &y, &nosuchprop, &f, &a];
    switch (p) { case &x: return 1; case &y: return 2; }
    if (&x == &y) return 3;
    if (&x == &x) return 4;
    return a.(p) + a.(&x) + args.(&w)(1) + (&x) + l;
}
"""

programs["addr2"] = """
_main(args) { local x = &later; local y = &ab; local z = &neverDefined; local w = &ba; return [x, y, z, w]; }
later() { }
a: object ba = 1 ab = 2;
"""

programs["meta1"] = """
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
    static generate(func, n);
}
_main(args) { return args.length() + [1, 2].length(); }
"""

programs["meta2"] = """
intrinsic class Vector 'vector/030005': Collection
{
    toList(start?, cnt?);
    length();
}
intrinsic class String 'string/030008': Object
{
    length();
    substr(start, len?);
}
intrinsic class Object 'root-object/030004'
{
    ofKind(cls);
}
intrinsic class Collection 'collection/030000': Object
{
    createIterator();
}
intrinsic class Iterator 'iterator/030001': Object
{
    getNext();
}
intrinsic class BigNumber 'bignumber/030001': Object { formatString(); }
intrinsic class IntrinsicClass 'intrinsic-class/030001': Object { isIntrinsicClass(obj); }
intrinsic class ByteArray 'bytearray/030002': Object { }
a: object x = 1;
_main(args)
{
    local v = new Vector(10);
    local w = new transient Vector(args...);
    v.ofKind(Vector);
    Vector;
    return [Vector, String, a, v.length(), 'abc'.substr(2), Vector.length, String.substr(1),
            new ByteArray(1, 2), w, new Vector, new Vector()];
}
"""

programs["meta3"] = """
intrinsic class TadsObject 'tads-object/030005'
{
    createInstance(...);
    static createClone();
}
intrinsic class Lookup 'lookuptable/030003' { isKeyPresent(key); }
intrinsic class Dict 'dictionary2/030001' { addWord(obj, str, prop); }
intrinsic class Old 'list/030001' { ba1(); ab1(); }
class C: object ab1 = 1 isKeyPresent = 2;
_main(args) { return new Lookup(8, 16).isKeyPresent(1) + C.createInstance() + args.ba1(); }
"""

programs["export1"] = """
property alpha, beta;
property gamma;
export alpha 'Alpha.name';
export beta;
export thing 'TheThing';
export f 'func';
export neverMentioned 'never';
export alpha 'Alpha.name';
thing: object alpha = 1 gamma = 2;
f() { }
_main(args) { return thing.beta; }
"""

COLLECTION = """intrinsic class Object 'root-object/030004' { ofKind(cls); }
intrinsic class Collection 'collection/030000': Object { createIterator(); createLiveIterator(); }
intrinsic class List 'list/030008': Collection { length(); }
"""

programs["forin1"] = COLLECTION + """
a: object lst = [1, 2] cur = nil;
_main(args)
{
    local sum = 0;
    foreach (local x in args) sum += x;
    foreach (a.cur in a.lst) { if (a.cur) continue; break; }
    local y;
    foreach (y in [1, 2, 3]) ;
    foreach (args[1] in args) { }
    for (local i in 1..10) sum += i;
    for (local i in args) sum += i;
    for (local i = 1, local j in args, local k in 1..sum step 2, y in sum..1 step -1 ; i < 10 ; ++i)
        sum += j + k;
    for (y in args.length..sum step args[1]) { if (y) break; }
    for (local i in 1..3 step 'x') ;
    lbl: for (local q in a.lst) { foreach (local r in q) { continue lbl; } }
    for (local i in 1..2, local j in 3..4 ; ; ) { if (i + j) break; }
    for (a.cur in a.lst) ;
    foreach (local unused in args) ;
    return sum;
}
"""

programs["forin2"] = COLLECTION + """
f(lst)
{
    foreach (local x in lst)
    {
        try { if (x) return x; }
        finally { lst = nil; }
    }
    for (local i in 1..lst)
        return i;
    for (local i in lst) { }
    foreach (local x in lst) { return 1; }
    lst = 2;
}
g(lst)
{
    for (local i in lst)
        return i;
}
h(lst)
{
    foreach (local x in lst)
        return x;
}
_main(args) { return f(args) + g(args) + h(args); }
"""

programs["isin1"] = """
a: object;
b: object;
_main(args)
{
    local x = args is in (1, 2, 3);
    local y = args not in (a, b);
    if (args is in (1)) return 1;
    if (1 is in (1, 2)) return 2;
    if (1 is in (2, 3)) return 3;
    if (1 not in (2, args)) return 4;
    if (1 is in (args, 2, 1, x)) return 5;
    if (a is in (b, a)) return 6;
    args is in (1, 2);
    args not in ();
    if (1 is in ()) return 7;
    if ('' not in ()) return 8;
    x = [1] is in ();
    while (args is in (x)) { }
    x = (args is in (x, y) ? 1 : 2) + (1 not in (1) ? 3 : 4);
    return x + y + (args is in (a, 'str', [1], nil)) + ('s' is in ('s')) + (nil not in (args))
        + (args is in (x) is in (true)) + (args == x is in (1)) + (args is in (1) == 2);
}
"""

# Anonymous functions, and what they share with the functions around
# them.
programs["anon1"] = """
_main(args)
{
    local f = {x: x + 1};
    local g = function(a, b) { return a + b; };
    return f(1) + g(2, 3);
}
"""

programs["anon2"] = """
_main(args)
{
    local n = 5;
    local q = 7;
    local f = {x: x + n};
    n = 6;
    local g = {: f(args) + {y: y + n + q}(2) };
    return f(1) + g();
}
"""

programs["anon3"] = """
a: object
    v = 3
    m(p)
    {
        local f = {x: x + v + p};
        local g = method(x) { return x + v; };
        local h = function() { return self; };
        return f(1) + g(2) + h();
    }
    k() { return ({: targetprop }); }
;
_main(args)
{
    return a.m(1);
}
"""

programs["anon4"] = """
_main(args)
{
    local a = 1, b = 2, c = 3, d;
    local f = {x: {y: {z: x + y + z + a}}};
    local g = function(p, q) {
        local r = p + b;
        return {: r + q + c };
    };
    b += 2;
    c++;
    --c;
    a = a * 2;
    b -= 1;
    c <<= 1;
    local h = {: d = 5 };
    h();
    return f(1)(2)(3) + g(1, 2)() + a + b + c + d;
}
"""

programs["anon5"] = """
f(a, b, [rest])
{
    local g = {: a + rest.length() };
    local h = {x: b = x };
    try { h(1); } catch (Exception e) { local k = {: e }; k(); }
    for (local i = 1 ; i < 3 ; ++i) { local z = {: i * 2 }; z(); }
    return g;
}
g(x, y) { return {: y }; }
h(x, y) { return {x: x }; }
k(x) { local x1 = x; { local x2 = 2; return {: x1 + x2 }; } }
_main(args)
{
    local u;
    local w = 1;
    local v = {: "hello" };
    {: w };
    v();
    {: "x<<w>>y" }();
    return {a, b: a + b}(1, 2) + f(1, 2)() + g(1,2)() + h(1,2)(3) + k(1)() + (function() { return 1; })() + new function(q) { return q; }(2);
}
"""

programs["anon6"] = """
class Exception: object;
a: object
    v = 3
    m(p)
    {
        local f = {x: x + v + p};
        local g = method(x) { return x + v + self.v; };
        local h = function() { return self; };
        return f(1) + g(2) + h();
    }
    k() { return ({: targetprop }); }
    k2() { return ({: {: definingobj } }); }
    k3(q) { return ({: {: q + v } }); }
    k4() { return ({: inherited() }); }
    k5() { return method() { return {: self }; }; }
    k6() { return {: "str" }; }
    k7() { local z = 1; return method() { return z; }; }
    k8() { return method() { return {: targetobj }; }; }
    p1 = {: v }
    p2 = {: 1 }
    p3 = ({x: x + v} (2))
    p4 = [{: self }, {: targetprop }]
    p5 = method() { return v; }
    p6 = {: "hello" }
;
b: object
    q1 = {: 2 }
    m1() { return {: targetprop }; }
    q2 = {: self }
    m2() { return 1; }
    q3 = {: self }
;
f1() { return {: nosuch }; }
f2() { return {: self }; }
_main(args)
{
    return a.m(1);
}
"""

programs["anon7"] = """
_main(args)
{
    local a = 1;
    local unused;
    local f = function(x) {
        local inner;
        local n = 0;
        lbl:
        n++;
        if (n < 3) goto lbl;
        return {: a + inner };
    };
    local g = function() { a = 2; };
    local h = function(z) { if (z) return 1; };
    local i = {: local q = 3 };
    return f(1)() + g() + h(1);
}
"""

programs["anon8"] = """
_main(args)
{
    local a = 1;
    local f = function(x) {
        local b = x;
        return function(y) {
            local c = y;
            return function(z) {
                a += 1;
                b += a;
                c = b + z;
                return {: a + b + c + x + y + z };
            };
        };
    };
    local g = {x: {y: a + y}};
    local h = {a: {b: a + b}};
    local a2 = 5;
    {
        local a2 = 6;
        local i = {: a2 };
        i();
    }
    local j = {: a2 };
    return f(1)(2)(3)() + g(1)(2) + h(1)(2) + j();
}
"""

programs["anon9"] = """
class Exception: object;
intrinsic class Object 'root-object/030004' { ofKind(cls); }
intrinsic class Collection 'collection/030000': Object { createIterator(); }
a: object
    v = 1
    m1(lst)
    {
        local total = 0;
        foreach (local x in lst)
            total += {: x + v }();
        for (local i in 1..3)
        {
            local f = {: i };
            total += f();
        }
        for (local i = 1, local j in lst ; i < 2 ; ++i)
            total += {: i + j }();
        return total;
    }
    m2(p, [more])
    {
        try { return ({: p + more.length() }); }
        catch (Exception e) { return ({: e }); }
        finally { local z = {: more }; z(); }
    }
    m3()
    {
        local n = 0;
        local inc = {: ++n };
        local dec = {: n-- };
        local add = {k: n += k };
        local idx = {k: n = k };
        local idx2 = {k: n[1] };
        local cmp = {: n == nil ? 1 : n };
        local sw = function(k) { switch (n) { case 1: return k; default: return n; } };
        inc(); dec(); add(2); idx(1); idx2(1); cmp(); sw(1);
        n = n ?? 3;
        n = n && 1;
        return n++ + ++n + n-- + --n;
    }
    m4()
    {
        return function() {
            return function() {
                return function() { return v + self.v; };
            };
        };
    }
    m5(q)
    {
        local f = method(k) { return q + k + v; };
        local g = method() { return ({: targetprop }); };
        local h = {: method() { return q; } };
        return f(1) + g() + h();
    }
    m6()
    {
        "one";
        return ({: "two" });
    }
    m7(x)
    {
        return ({: {: {: x } } });
    }
    m8(x)
    {
        return ({: {: {: inherited(x) } } });
    }
    m9 = ({: v } () + {: self.v } ())
;
_main(args)
{
    return a.m1(args) + a.m2(1) + a.m3() + a.m9;
}
"""

programs["anon10"] = """
f1() { local a = 1; local f = {: a }; local g = {: a }; return f() + g(); }
f2() { local a = 1, b = 2, c = 3; local f = {: c }; local g = {: a }; local h = {: b }; return f() + g() + h(); }
f3(p1, p2, p3) { local f = {: p3 + p1 }; return f; }
f4(p1) { local x = 1; if (p1) { local y = 2; return {: x + y + p1 }; } return {: x }; }
f5() { local zz = 1, yy = 2, ab = 3, ba = 4, abc = 5, cab = 6; return {: zz + yy + ab + ba + abc + cab }; }
f6() { local zz = 1, yy = 2, ab = 3, ba = 4, abc = 5, cab = 6; return {: {: cab + abc + ba + ab + yy + zz } }; }
f7() { local a = 1; local f = {: a }; local b = 2; local g = {: b }; local c = 3; return {: a + b + c + f() + g() }; }
f8() { local a; local f = {: a = 1 }; return f; }
f9() { local a = 1; local f = {: 1 }; return f() + a; }
f10() { local a = 1; return {: {: 1 } }; }
f11() { local a = 1; return {: {: a } }; }
f12(a) { return {b: {c: {d: a + b + c + d } } }; }
f13(a) { return {b: {c: {d: b + d } } }; }
f14(a) { return {b: {c: {d: d } } }; }
f15(a) { return {b: {c: {d: c + a } } }; }
f16() { local f = {: f }; return f; }
f17() { local f; f = {: f }; return f; }
f18(a) { local f = {: a }; local g = {: f }; return {: g }; }
_main(args)
{
    return f1() + f2() + f3(1, 2, 3) + f4(1) + f5() + f6() + f7() + f8() + f9() + f10() + f11()
        + f12(1) + f13(1) + f14(1) + f15(1) + f16() + f17() + f18(1);
}
"""

programs["anon11"] = """
a: object
    p1 = [1, {: 2 }]
    p2 = {x: x }
    p3 = {: p1 }
    p4 = (function() { return self; })
    p5 = {: targetprop }
    p6 = {: {: self } }
    p7 = method() { return self; }
    p8 = new function() { return 1; }
    p9 = ({: 1 }() + 2)
    p10 = {: "hello" }
    p11 = "say <<{: 1 }()>>"
    p12 = '<<{: 1 }()>>'
    m1() { return 1; }
    p13 = {: self }
    m2() { return {: targetprop }; }
    p14 = {: self }
    p15 = {: {: 1 } }
;
f() { return {: 1 }; }
b: object
    p1 = {: self }
;
c: object
    m() { return {: {: definingobj } }; }
    p1 = {: self }
    p2 = {: targetobj }
;
_main(args)
{
    return a.p1 + b.p1;
}
"""

mistakes = {
    "e_undef": "_main(args) { return undefinedThing; }",
    "e_undefcall": "_main(args) { return undefinedFunc(1); }",
    "e_undefasi": "_main(args) { undefinedVar = 1; }",
    "e_argc": "f(a) { return a; } _main(args) { return f() + f(1, 2); }",
    "e_argc2": "f(a, ...) { return a; } _main(args) { return f() + f(1, 2); }",
    "e_argc3": "two(a, b) { return a; } _main(args) { return two(args...); }",
    "e_break": "_main(args) { break; }",
    "e_continue": "_main(args) { continue; }",
    "e_breaklbl": "_main(args) { while (args) { break foo; } }",
    "e_lvalue1": "_main(args) { 1 = 2; }",
    "e_lvalue2": "_main(args) { 1++; }",
    "e_lvalue3": "f() { } _main(args) { f = 2; f++; f += 1; }",
    "e_lvalue4": "_main(args) { (args + 1) = 2; ++(args + 1); }",
    "e_redef1": "f() { } f() { } _main(args) { }",
    "e_redef2": "f(a) { } f(a, b) { } _main(args) { }",
    "e_localredef": "_main(args) { local a; local a; local args; }",
    "e_formalredef": "f(a, a) { } _main(args) { }",
    "e_nosemi": "_main(args) { local a = 1 return a; }",
    "e_nosemi2": "_main(args) { args = 1 }",
    "e_norbrace": "_main(args) { return 1;",
    "e_norparen": "_main(args) { return (1 + 2; }",
    "e_extrarparen": "_main(args) { return 1 + 2); }",
    "e_badprimary": "_main(args) { return 1 + ; }",
    "e_badprimary2": "_main(args) { return * 2; }",
    "e_ifnoparen": "_main(args) { if args return 1; }",
    "e_ifnorparen": "_main(args) { if (args return 1; }",
    "e_else": "_main(args) { else return 1; }",
    "e_whilenoparen": "_main(args) { while args { } }",
    "e_dowhile": "_main(args) { do { } until (args); }",
    "e_for1": "_main(args) { for (local i) { } }",
    "e_for2": "_main(args) { for (args) { } }",
    "e_for3": "_main(args) { for (args = 1; args) { } }",
    "e_for4": "_main(args) { for (args = 1, ; ; ) { } }",
    "e_for5": "_main(args) { for args = 1; ; ) { } }",
    "e_local1": "_main(args) { local; }",
    "e_local2": "_main(args) { local 1; }",
    "e_local3": "_main(args) { local a b; return a + b; }",
    "e_local4": "_main(args) { local a = ; }",
    "e_formal2": "f(a,) { } _main(args) { }",
    "e_formal3": "f(a { } _main(args) { }",
    "e_formal4": "f(1) { } _main(args) { }",
    "e_formal5": "f(..., a) { } _main(args) { }",
    "e_formal6": "f([a], b) { } _main(args) { }",
    "e_formal7": "f([a) { } _main(args) { }",
    "e_formal8": "f([1]) { } _main(args) { }",
    "e_top1": "1; _main(args) { }",
    "e_top3": "function 1 { } _main(args) { }",
    "e_top4": "f; _main(args) { }",
    "e_body1": "f() return 1; _main(args) { }",
    "e_body2": "f(); _main(args) { }",
    "e_quest": "_main(args) { return args ? 1; }",
    "e_arg1": "f(a, b) { } _main(args) { f(1 2); }",
    "e_arg2": "f(a, b) { } _main(args) { f(1, 2; }",
    "e_arg3": "f(a, b) { } _main(args) { f(1, ); }",
    "e_constcmp": "_main(args) { return 1 < 'a'; }",
    "e_constnum": "_main(args) { return 'a' * 2; }",
    "e_constnum2": "_main(args) { return nil - 1; }",
    "e_constadd": "_main(args) { return nil + 1; }",
    "e_neg": "_main(args) { return -'a'; }",
    "e_bnot": "_main(args) { return ~nil; }",
    "e_pos": "_main(args) { return +true; }",
    "e_nomain": "f() { }",
    "e_retexpr": "_main(args) { return }",
    "e_stmt": "_main(args) { ) return 1; }",
    "e_eof1": "_main(args) { if (args",
    "e_eof2": "_main(args) { return f(1,",
    "e_eof3": "_main(args",
    "e_eof4": "_main(args) { local a =",
    "e_callconst": "_main(args) { return 1(2); }",
    "e_callstr": "_main(args) { return 'a'(2); }",
    "e_toomany": "f(...) { } _main(args) { f("
    + ", ".join(str(i) for i in range(130))
    + "); }",
    "w_unreach": "_main(args) { return 1; args = 2; args = 3; }",
    "w_unreach2": "_main(args) { while (true) { } return 1; }",
    "w_unreach3": "_main(args) { for (;;) { break; args = 1; } }",
    "w_retboth": "_main(args) { if (args) return 1; return; }",
    "w_retfall": "_main(args) { if (args) return 1; }",
    "w_asicond": "_main(args) { if (args = 1) return 1; while (args = 2) { }"
    " do { } while (args = 3); for (; args = 4; ) { } return 2; }",
    "w_iftrue": "_main(args) { if (true) return 1; else return 2; }",
    "w_iffalse": "_main(args) { if (nil) return 1; if (0) return 2; return 3; }",
    "w_forfalse": "_main(args) { for (; nil; ) { args = 1; } for (; nil; args = 2) ; return 3; }",
    "w_whilefalse": "_main(args) { while (nil) { args = 1; } while (0) ; return 3; }",
    "w_unused": "_main(args) { local a; local b = 1; local c; return c; }",
    "w_unusedparam": "f(a, [b]) { } _main(args) { }",
    "e_bif1": BUILTINS + "_main(args) { return dataType() + dataType(1, 2) + max(); }",
    "e_bif2": BUILTINS + "_main(args) { dataType = 1; dataType++; return 1; }",
    "e_bif3": "intrinsic { f(); } _main(args) { }",
    "e_bif4": "intrinsic 'x' f(); } _main(args) { }",
    "e_bif5": "intrinsic 'x' { f; g() 1; } _main(args) { }",
    "e_bif6": "intrinsic 'x' { f(a?, b); } _main(args) { }",
    "e_bif7": "intrinsic 'x' { f();",
    "e_bif9": "intrinsic 'x' { f(); } f() { } _main(args) { return f(); }",
    "e_dstr1": '_main(args) { local a = "text"; return a; }',
    "e_dstr2": '_main(args) { return "text"; }',
    "e_dstr3": 'f(x) { } _main(args) { f("text"); }',
    "e_list1": "_main(args) { return [1, 2; }",
    "e_list2": "_main(args) { return [1 2]; }",
    "e_list3": "_main(args) { return [1, ]; }",
    "e_list4": "_main(args) { return [1, 2)]; }",
    "e_list5": "_main(args) { return [1, 2][3]; }",
    "e_list6": "_main(args) { return [1, 2][0] + [1, 2]['a'] + 1[2] + nil[1]; }",
    "e_list7": "_main(args) { return args[1; }",
    "e_list8": "_main(args) { return [1, 2] < [3]; }",
    "e_list9": "_main(args) { return [1, 2] * 2 + 1 + [2]; }",
    "e_list10": "_main(args) { return [1, 2",
    "e_sw1": "_main(args) { switch args) { case 1: ; } }",
    "e_sw2": "_main(args) { switch (args { case 1: ; } }",
    "e_sw3": "_main(args) { switch (args) case 1: ; } }",
    "e_sw4": "_main(args) { switch (args) { args = 1; args = 2; case 1: ; } }",
    "e_sw5": "_main(args) { switch (args) { args = 1;",
    "e_sw6": "_main(args) { switch (args) {",
    "e_sw7": "_main(args) { local q; switch (args) { case args: ; case 1 + args: ; } return nosuch; }",
    "e_sw8": "_main(args) { switch (args) { case 1 ; default ; } }",
    "e_sw9": "_main(args) { switch (args) { default: ; default: ; default: ; } }",
    "e_sw10": "_main(args) { case 1: ; default: ; case 2 }",
    "e_sw11": "_main(args) { switch (args) { case 1: { case 2: ; default: ; } } }",
    "e_sw12": "_main(args) { switch (args) { case 1: if (args) case 2: ; } }",
    "e_sw13": "_main(args) { switch (args) { lbl: case 1: ; other: default: ; case 2: ; } }",
    "e_sw14": "_main(args) { switch (args) { case",
    "e_sw15": "_main(args) { switch (args) { case 1: } }",
    "e_sw16": "_main(args) { switch (args) { case 1: continue; } }",
    "e_sw17": "_main(args) { switch (;) { } }",
    "e_sw18": "_main(args) { switch (args) { case 1: case : ; } }",
    "e_sw19": "_main(args) { switch (args) { case 1: while (args) { case 2: ; } } }",
    "e_sw20": "_main(args) { switch (args) { lbl: args = 1; case 1: goto lbl; } }",
    "e_lbl1": "_main(args) { a: args = 1; a: args = 2; a: ; goto a; }",
    # (A `goto` with no label after it is not here: the reference crashes
    # writing its message.)
    "e_lbl2": "_main(args) { goto nosuch; goto a b; }",
    "e_lbl3": "_main(args) { a: while (args) { break b; continue b; } }",
    "e_lbl4": "_main(args) { a: { continue a; } b: if (args) continue b; c: args = 1; }",
    "e_lbl5": "_main(args) { a: ; a: { break nosuch; } goto a; }",
    "e_lbl6": "_main(args) { a: ; a: while (args) { continue nosuch; } goto a; }",
    "e_lbl7": "_main(args) { try { args = 1; } finally { in: args = 2; } goto in; }",
    "e_lbl8": "_main(args) { try { goto in; } finally { try { in: ; } finally { } } }",
    "e_lbl9": "_main(args) { a: }",
    "e_lbl10": "f() { a: ; goto a; } _main(args) { goto a; }",
    "e_lbl11": "_main(args) { a: b: break a; c: d: break d; e: g: continue e; }",
    "e_lbl12": "_main(args) { a: break; b: continue; }",
    "e_lbl13": "_main(args) { local a; a: a = 1; goto a; }",
    "e_lbl14": "_main(args) { args: args = 1; goto args; return args:; }",
    "e_try1": "_main(args) { try { args = 1; } return 1; }",
    "e_try2": "_main(args) { catch (a b) { } finally { } catch { } catch (a) }",
    "e_try3": "_main(args) { try { } catch (NoSuch e) { return e; } }",
    "e_try4": "f() { } _main(args) { try { } catch (f e) { } catch (_main e) { } }",
    "e_try5": "_main(args) { try { } catch NoSuch e) { } }",
    "e_try6": "_main(args) { try { } catch (1 e) { } catch (A 2) { } catch (A) { } catch () { } }",
    "e_try7": "_main(args) { try { } catch (A e { } }",
    "e_try8": "_main(args) { try { } finally { } finally { } }",
    "e_try9": "_main(args) { try { } catch (A e) { } finally { } catch (A e) { } }",
    "e_try10": "_main(args) { try",
    "e_try11": "_main(args) { try { } finally",
    "e_try12": "_main(args) { try { } catch (A e) { local e; local f; return f; } }",
    "e_try13": "_main(args) { try { } catch (A e) x: { goto x; } finally { goto x; } }",
    "e_throw1": "_main(args) { throw; }",
    "e_throw2": "_main(args) { throw 1 }",
    "e_throw3": "_main(args) { throw",
    "e_foldstop": "_main(args) { local q; return [1, 2][3] + nosuch; }",
    "w_lbl1": "_main(args) { a: args = 1; bb: cc: args = 2; { d: ; } }",
    "w_lbl2": "_main(args) { return 1; goto a; a: return 2; }",
    "w_lbl3": "_main(args) { return 1; args = 2; a: return 2; args = 3; b: ; goto b; }",
    "w_lbl4": "_main(args) { if (nil) goto a; a: if (true) return 1; else goto b; b: ; }",
    "w_lbl5": "_main(args) { a: while (args) { break a; } b: while (true) { continue b; } return 1; }",
    "w_lbl6": "_main(args) { while (nil) a: args = 1; for (; nil; ) b: args = 2; for (; nil; ) { c: ; } }",
    "w_lbl7": "f(x) { switch (x) { case 1: goto a; } } _main(args) { a: return f(args); }",
    "w_lbl8": "f(x) { try { } finally { break a; } } g(x) { a: ; b: ; } _main(args) { a: b: return f(args); }",
    "w_lbl9": "f(x) { switch (x) { case 1: continue b; } } _main(args) { a: b: return f(args); }",
    "w_lbl10": "_main(args) { switch (args) { case 1: goto a; } return 1; a: ; } f(x) { a: ; }",
    "w_sw1": "_main(args) { switch (args) { case 1: return 1; args = 1; default: return 2; } args = 3; }",
    "w_sw2": "_main(args) { switch (args) { case 1: return 1; } }",
    "w_sw3": "_main(args) { switch (args) { case 1: return 1; default: return; } }",
    "w_try1": "_main(args) { try { return 1; } finally { args = 1; } args = 2; }",
    "w_try2": "_main(args) { try { args = 1; } finally { return; } args = 2; }",
    "w_try3": "_main(args) { try { return 1; } finally { return; } }",
    "w_throw1": "_main(args) { throw args; args = 1; }",
    "w_throw2": "_main(args) { if (args) throw args; else return 1; }",
    "w_argnoval": "f() { } g(a) { } _main(args) { g(f()); }",
    "e_emb1": '_main(args) { "a<<args b>>c"; }',
    "e_emb2": '_main(args) { "a<<1 +>>c"; }',
    "e_emb3": '_main(args) { "a<<(1>>b"; }',
    "e_emb4": "_main(args) { \"a<<1;>>b\"; local s = 'a<<args args>>b'; }",
    "e_emb5": '_main(args) { local x = "a<<args>>b"; x = "a<<args>>"; }',
    "e_emb6": '_main(args) { local y = ("<<args>>b", 1); local z = args ? "a<<1>>" : "b"; }',
    "e_emb7": '_main(args) { "<<args>>" + 1; "<<args>>"(1); }',
    "e_emb8": '_main(args) { "a<<else>>b"; }',
    "e_emb9": '_main(args) { "<<else if args>>x"; "a<<else unless 1>>b"; }',
    "e_emb10": '_main(args) { "<<if>>a<<end>>"; "<<unless>>b"; }',
    "e_emb11": '_main(args) { "<<if args>>a<<else>>b<<else>>c<<end>>"; }',
    "e_emb12": '_main(args) { "<<if args>>a<<or>>b<<end>>c<<only>>d<<stopping>>e<<at random>>f"; }',
    "e_emb13": '_main(args) { "<<one of>>a<<or>>b<<at random>>"; }',
    "e_emb14": "_main(args) { \"<<first time>>a<<only>>\"; local s = 'x<<one of>>a<<or>>b<<cycling>>'; }",
    "e_emb15": '_main(args) { "<<one of>>a<<or>>b<<or>><<if args>>c<<end>><<stopping>>"; }',
    "e_emb16": '_main(args) { "<<one of>>a<<||>>b<<then purely at random>>c<<one of>>d<<first time>>e"; }',
    "e_emb17": '_main(args) { "<<one of>>a<<if args>>b<<or>>c<<end>>d<<as decreasingly likely outcomes>>"; }',
    "e_emb18": '_main(args) { "<<%d>>"; }',
    "e_emb19": '_main(args) { "<<%>>"; }',
    "e_emb20": '_main(args) { "<<%d args>>"; }',
    "e_emb21": '_main(args) { "a<<args>>b" ; "a<<args>>b',
    "e_emb22": '_main(args) { "a<<args; "b"; }',
    "e_emb23": '_main(args) { "a<<args}\nf() { "x<<1 2>>y<<3>>z<<4 5"; }',
    "e_emb24": '_main(args) { "<<\nargs +\n1 +\n>>\n"; "<<args>>\n<<args args>>\nz"; }',
    "e_emb25": '_main(args) { "<<if args>>a<<end"; }',
    "e_emb26": "_main(args) { 'a<<if args>>b<<else'; 'c<<end>>'; }",
    "e_emb27": '_main(args) { "<<nosuch>><<end>><<or>>"; }',
    "e_emb28": "_main(args) { local s = '<<\"a\">>'; return s; }",
    "e_emb29": "_main(args) { \"<<if args>>x<<else>>y<<end>><<end>>\"; return 'a<<}>>'; }",
    "e_emb30": '_main(args) { "<<[>>"; "<<)>>"; "<<(>>"; "<<]>>"; }',
    "e_tpl1": 'string template <<* in words>> spell;\n_main(args) { "<<args in words>>"; }',
    "e_tpl2": "string template <<the * of *>> bad;\n_main(args) { }",
    "e_tpl3": "nope() { }\nstring template <<nope2>> nope;\n_main(args) { }",
    "e_tpl4": "string template <<* in words>> spell;\nspell(x, y) { return x; }\n_main(args) { }",
    "e_tpl5": "string template nope3 nope;\n_main(args) { }",
    "e_tpl6": "string template <<nope3 nope;\n_main(args) { }",
    "e_tpl7": "string template <<a b>> 3;\n_main(args) { }",
    "e_tpl8": "string template <<q>> _main;\n_main(args) { }",
    "e_tpl9": "string template <<a>> fa;\n_main(args) { fa(1); undefinedthing(); }",
    "e_tpl10": 'string template <<p>> ab;\nstring template <<q>> ba;\nstring template <<r>> zz;\nstring template <<s>> aa;\nstring template <<t *>> cab;\nstring template <<u>> bca;\n_main(args) { "<<s>><<r>><<q>><<p>><<u>><<t 1>>"; }',
    "e_tpl11": "f(x) { return x; }\nstring template <<a *>> f;\nstring template <<b>> f;\nstring template <<c *>> g;\ng() { return 1; }\nstring template <<d *>> h;\nh(...) { }\nstring template <<e *>> k;\nk(a, b) { }\n_main(args) { }",
    "e_tpl12": "string template <<a>> fa;\n_main(args) { return fa; }",
    "e_tpl13": "string template <<a>> fa;\n_main(args) { return [fa, 1]; }",
    "e_tpl15": "string template;\nstring template <<a>>;\n_main(args) { }",
    "e_tpl16": "string template <<a>> fa\n_main(args) { }",
    "e_undef1": "_main(args) { nosuch; nosuch; nosuch(); nosuch = 1; other; return nosuch + other; }",
    "e_undef2": 'f() { nosuch(); } g() { "<<nosuch>>"; }\n_main(args) { nosuch; try { } catch (nosuch e) { } }',
    "e_undef3": "_main(args) { try { } catch (nosuch e) { } try { } catch (nosuch e) { } nosuch = 1; }",
    "e_obj1": "a: object\n    x = 1\n    x = 2\n    y = 3\n    sourceTextOrder = 9\n;\na: object;\nf() { }\nf: object;\ng: object z = 1;\ng() { }\n_main: object;\n_main(args) { }",
    "e_obj2": "a object x = 1;\nb: x = 1;\ne: e;\nf: g;\ng: f;\nh: object x 1 y = ; z ;\n_main(args) { }",
    "e_obj3": "i: object x = 1\nj: object y = 2;\n_main(args) { }",
    "e_obj4": "a: object x = 1;\ni: object x = 1\na: object y = 2;\n_main(args) { }",
    "e_obj5": "f() { return 1; }\na: object x = 1;\n_main(args) { self; self.x; x; x = 1; x(1); a.f; a.f(1); a.a; f.x; 'str'.x; [1].x; nil.x; args.f; args.(f); args.(a); a.(f)(1); nosuch.x; nosuch.x = 1; f.x = 1; a.(1) = 2; }",
    "e_obj6": "a: Nosuch x = 1;\nf() { }\nb: f;\nc: a, Nosuch2, f;\n_main(args) { }",
    "e_obj7": "a: object\n    x = 1\n    m() { return 1 }\n    n = { return 2; }\n    y = 2\n;\n_main(args) { }",
    "e_obj8": "f() { }\na: object f = 1 g() { } _main = 2;\ng() { }\n_main(args) { }",
    "e_obj9": "a: object x = 1;\n_main(args) { a = 1; a(); a++; return a[1] + a.x.(a) + a.(a.x); }",
    "e_obj10": "a: object x = 1;\nb: a, object;\nc: object {\n x = 1\n_main(args) { }",
    "e_obj11": "a: object { x = 1 class b: object;\n_main(args) { }",
    "e_obj12": "a: object x = 1 class b: object;\n_main(args) { }",
    "e_obj13": "a: object x = 1 , y = 2 ) z = 3 ] w = 4 } v = 5;\n_main(args) { }",
    "e_obj14": "a: object 1 2 x = 3;\n_main(args) { }",
    "e_obj15": "a: object x = ;\nb: object m( { }\n_main(args) { }",
    "e_obj16": "class a: object;\nclass a: object;\nclass f() { }\nclass 1;\nclass;\n_main(args) { }",
    "e_obj17": "a: object m() { return nosuch.x + self.(nosuch2) + nosuch3(); }\n;\nf() { return nosuch4 + nosuch5(); }\n_main(args) { }",
    "e_obj18": "a: object x = 1;\na x = 2\n_main(args) { }",
    "e_obj19": "a: object x = 1;\na\n",
    "e_obj20": "a: object x = (1;\nb: object y = 2;\n_main(args) { }",
    "e_obj21": "a: object x = 1; y = 2;\n_main(args) { }",
    "e_obj22": "a: object m() { local q; return self.; }\n n = self.(1 ;\n;\n_main(args) { return a.; }",
    "e_obj23": "a: object x = 1;\nf() { try { } catch (a e) { } catch (f g) { } catch (nosuch h) { } }\n_main(args) { }",
    "e_obj24": "object;\nobject x = 1\n_main(args) { }",
    "e_obj25": "a: object construct() { return 1; }\n;\nb: object construct() { return self; }\n;\n_main(args) { }",
    "w_obj1": "a: object m() { local q; return 1; q = 2; }\n n() { if (x) return 1; }\n x = 1\n;\n_main(args) { }",
    "w_obj2": "a: object m() { return y; }\n n() { y = 1; return a.z + self.w + v(1); }\n;\n_main(args) { return a.u; }",
    "e_unres1": "_main(args) { if (true) args = 1; else { a: args = 2; } goto a; }",
    "e_unres2": "f(x) { if (nil) { b: x = 1; } if (x) goto b; goto b; }\n_main(args) { if (true) args = 1; else { a: args = 2; } if (args) goto a; return f(args); }",
    "e_unres3": "f(x) { unused: if (nil) { b: x = 1; } if (x) goto b; local q; }\ng(x) { if (nil) { c: while (x) { if (x) break; x = 2; } } other: goto c; }\nh(x) { try { if (nil) { d: x = 1; } } finally { x = 2; } goto d; }\n_main(args) { return f(args) + g(args) + h(args) + nosuch; }",
    "e_addr1": "f() { }\n_main(args) { local q; return [&q, &args, &1, &(f), &tadsSay, &'x', & &f, &(args)]; }",
    "e_addr2": "_main(args) { return &later; }\nlater: object;",
    "e_addr3": "a: object x = &y;\ny() { }\nz: object w = &y;\n_main(args) { return a.x; }",
    "e_new1": "f() { }\na: object;\n_main(args) { new f; new f(); new 1; new args; new args(1); new (a); new nosuch; new nosuch(1); new a()(); new [1]; }",
    "e_inh1": "f() { }\na: object;\n_main(args) { inherited; inherited.x; inherited a.x; inherited f.x; inherited nosuch.x; targetprop; targetobj; definingobj; invokee; }",
    "e_inh2": "f() { }\na: object m() { inherited f.x; inherited nosuch.x(1); return inherited.; };\n_main(args) { }",
    "e_meta1": "intrinsic class;\nintrinsic class 1;\nintrinsic class A;\nintrinsic class B 'b/030000'\nintrinsic class C 'c/030000': ;\nintrinsic class D 'd/030000': Nosuch { }\nf() { }\nintrinsic class E 'e/030000': f { }\n_main(args) { }",
    "e_meta2": "f() { }\nintrinsic class f 'f/030000' { }\nintrinsic class A 'a/030000' { f(); 1; x y; z(a,); }\nintrinsic class A 'a/030001' { }\nintrinsic class B 'a/030000' { }\n_main(args) { }",
    "e_meta3": "intrinsic class A 'a/030000' { x(); }\nb: A;\nA: object;\nA() { }\n_main(args) { A(); A = 1; A.x = 2; &A; new A.x; try { } catch (A e) { } return inherited A.x(); }",
    "e_meta4": "intrinsic class A 'a/030000' { x();\n_main(args) { }",
    "e_export1": "property a b, 1, , c d;\nproperty;\nproperty x\nexport;\nexport 1;\nexport a b;\nexport a 'x' y;\n_main(args) { }",
    "e_export2": "export a 'LastProp';\nexport b 'operator +';\nexport c 'Constructor';\nexport d 'same';\nexport e 'same';\nexport f 'same';\nexport tadsSay 'bif';\nintrinsic 'tads-io/030007' { tadsSay(val, ...); }\n_main(args) { }",
    "e_export3": "f() { }\nproperty f, _main;\nexport g;\ng() { }\n_main(args) { }",
    "e_forin1": "_main(args) { foreach (local x in args) ; for (local i in args) ; }",
    "e_forin2": COLLECTION + "_main(args)\n{\n    foreach;\n    foreach (;\n    foreach (local 1 in args) ;\n    foreach (x args) ;\n    foreach (local x in args ;\n    for (local i in ) ;\n    for (local i in 1.. ) ;\n    for (i in 1..2 step ) ;\n    for (local i in args ; ; ) ;\n    for (local i in args, ) ;\n}",
    "e_forin3": COLLECTION + "_main(args) { for (local i in 1..2 ; i < 2 ; ) ; for (1 in args) ; foreach (2 in args) ; for (nosuch in args) ; foreach (local x in nosuch2) ; }",
    "e_forin4": COLLECTION + "_main(args) { foreach (local x in args) { local y = x; } return x; }\nf() { foreach (local a in 1) break; continue; }",
    "e_forin5": COLLECTION + "_main(args) { foreach (local x in args",
    "e_forin6": COLLECTION + "_main(args) { for (local x in 1..",
    "e_isin1": "_main(args)\n{\n    args is in 1, 2);\n    args is in (1 2);\n    args is in (1, ;\n    args not in (1, 2;\n    args is 3;\n    args not 4;\n}",
    "e_isin2": "_main(args) { return args is in (nosuch, 1) + (args is in (1",
}

# The compiler gives up after a hundred errors, whether it finds them
# reading the source, working out constants, or generating code.
mistakes["e_many1"] = (
    "f() { local q; }\n_main(args) {\n"
    + "".join(f"  u{n}();\n" for n in range(120))
    + "}\ng() { local r; }\n\n"
)
mistakes["e_many2"] = (
    "f() { local q; }\n_main(args) {\n"
    + "".join(f"  u{n}();\n" for n in range(100))
    + "}\ng() { local r; }"
)
mistakes["e_many6"] = "_main(args) {\n" + "  u();\n" * 150 + "}"
mistakes["e_many3"] = "_main(args) {\n" + "  1 2;\n" * 150 + "}"
mistakes["e_many4"] = "_main(args) {\n" + "  [1][2];\n" * 101 + "}"
mistakes["e_many7"] = "_main(a) { for (a x"
mistakes["e_many8"] = "_main(a) { for (local i = 1, "
mistakes["e_many5"] = "_main(args) {\n" + "  1 2;\n" * 100 + "}"

mistakes["e_anon1"] = """_main(args)
{
    local a, b = 1, c;
    local f = {x: a };
    local g = {x, y: local t = x, local u = y, t + u };
    local g2 = {: local t, t };
    local g3 = {: local t = 1 };
    local h = {x: x; };
    local i = {x x };
    local j = function(a b) { return a; };
    local k = function(Thing a) { return a; };
    local l = function(a) multimethod { return a; };
    local m = {x, y: x};
    local n = {x, [y]: y};
    local o = {...: 1};
    local p = {[q]: q};
    local r = {x, : x};
    local s = { 1 + 2 };
    local t = function { return 1; };
    local u = function() return 1;
    local v = method;
    while (b) { local w = {: 1 }; local w2 = function() { break; }; lbl: w(); w2(); goto lbl; }
    return f;
}
"""

mistakes["e_anon2"] = """_main(args)
{
    local f = {x: x
"""

mistakes["e_anon3"] = """_main(args)
{
    local f = {x
"""

mistakes["e_anon4"] = """_main(args)
{
    local f = function(x) { return {y: 
"""

mistakes["e_anon5"] = """_main(args)
{
    local a = {x: local, x };
    local b = {x: local 1 };
    local c = {x: local y = , x };
    local d = {x: local y, local y, y };
    local e = {x: local y = 1, };
    local f = {x, x: x };
    local g = {x: };
    local h = { };
    local i = {: };
    local j = {x: x };;
    local k = {x: return x; };
    local l = function(x) { return x };
    local m = function(x) { local x; return x; };
    local n = method(x) { return self + x + targetprop; };
    local o = function() { return self + targetobj; };
    local p = {: self };
    local q = {: inherited() };
    local r = {: break };
    local s = function() { continue; };
    local t = {: args = 1, args };
    return a;
}
"""

mistakes["e_anon6"] = """f(x)
{
    local unused1 = {y: 1 };
    local g = function(z) { local unused2; local unset; return unset; };
    local h = function(z) { local w = 1; };
    local i = {q: q };
    {r: r + x };
    function(s) { return s; };
    if ({: 1 }) return 1;
    while ({: nil }()) { }
    switch (x) { case 1: return {: 2 }(); }
    return {: 1 } + {: 2 } == {: 3 } ? {: 4 } : {: 5 };
}
g(x) { return function() { if (x) return 1; else return; }; }
h(x) { return function() { if (x) return 1; }; }
i(x) { return function() { return; }(); }
j(x) { return {: x = 1 }; }
k(x) { local y; return {: y }; }
l(x) { local y = 1; return {: 1 }; }
m() { local y; local f = {: y = 1 }; return f; }
_main(args) { return f(1); }
"""

mistakes["e_anon7"] = """a: object
    p1 = {: }
    p2 = {x: x
    p3 = 1
;
_main(args) { return a; }
"""

mistakes["e_anon8"] = """a: object
    m() { return function() { return {: ; }; }
    p3 = 1
;
_main(args) { return a.m() + function(; }
"""

mistakes["e_anon9"] = """_main(args)
{
    return new method { return 1; } + new function;
}
"""

mistakes["e_anon10"] = """_main(args)
{
    local a = 1;
    lbl:
    local f = function() { goto lbl; };
    local g = function() { lbl2: return 1; };
    local h = function() { lbl: a++; if (a < 5) goto lbl; };
    try { local i = {: a }; i(); } finally { local j = function() { return a; }; j(); }
    goto lbl2;
    return f + g + h;
}
"""


# Optional parameters, parameters passed by name, and the calls that pass
# them.
programs["param1"] = (
    BUILTINS
    + """
f(a, b?) { return [a, b]; }
g(a, b = 5) { return a + b; }
h(a, b = a * 2, c = b + 1) { return [a, b, c]; }
k(a?, b?, c?) { return a; }
_main(args)
{
    f(1); f(1, 2); g(1); g(1, 2); h(1); h(1, 7); h(1, 7, 9); k(); k(1, 2, 3);
}
"""
)

programs["param2"] = (
    BUILTINS
    + """
f(a, n:) { return a + n; }
g(n:?) { return n; }
h(n: = 4) { return n; }
k(n: = tadsSay('x')) { return n; }
m(x:, y:, abc:, zz:?) { return x; }
_main(args)
{
    f(1, n: 2); f(n: 2, 5); g(); g(n: 3); h(); h(n: 1); k(n: 6); k();
    m(x: 1, y: 2, abc: 3);
    m(abc: 3, y: 2, x: 1, zz: 0);
}
"""
)

programs["param3"] = (
    BUILTINS
    + """
obj: object
    m(a, b = name, c: = 3, d:?) { return [a, b, c, d]; }
    name = 'nm'
    p(x) { return m(x, d: 1, c: 2); }
    q() { return self.m(1, 2, d: 4); }
    r(pp) { return self.(pp)(1, d: 4) + inherited.m(2, c: 1) + inherited(c: 2); }
    s(pp) { return obj.(pp)(d: 4) + other.m(c: 1) + inherited other.m(d: 2) + name(d: 1); }
    t(pp) { return pp.name(d: 4) + pp.(pp)(c: 1) + inherited.(pp)(d: 2) + self.(pp)(d: 1); }
;
other: obj
    m(a, b?, zz: = nil, [rest]) { return inherited(a, b, zz: 5, rest...); }
;
_main(args)
{
    local o = obj;
    obj.m(1); o.m(1, c: 9); o.p(2);
    new obj(a: 1);
    new transient obj(1, a: 1, b: 2);
    toString(5, radix: 2);
    args(n: 1, o.m(n: 2, 3, c: args(4, n: 5)));
    args(a: 1, [1, 2]..., b: 2);
}
"""
)

programs["param4"] = (
    BUILTINS
    + """
obj1: object pp = 1;
h(a = [1, 2], b = obj1, c = &pp, d = obj1 == obj1, e = 1 + 2, ff = nil ? 1 : 2,
  g1 = [obj1, 1], h1 = h, i1 = !obj1, j1 = obj1.pp, k1 = [1, 2][1], l1 = (nil ?? obj1))
{
    return a;
}
k(a: = 1 + 2, b: = obj1, c: = 'str', d: = [1], e: = -1, ff: = [obj1], g1: = h, h1: = &pp,
  i1: = nil, j1: = true)
{
    return a;
}
_main(args) { h(); k(); }
"""
)

programs["param5"] = (
    BUILTINS
    + """
f(a = 1, b: = 2)
{
    local g = {: a + b};
    return g();
}
h(a?, b?)
{
    return ({x = a: x + b});
}
k(a?, b = 1) { }
m(a?, [rest]) { return rest; }
n(a = 1, ...) { return a; }
o(a:?, [rest]) { return rest; }
_main(args)
{
    f(); f(3, b: 4); h(1, 2)(); h(1, 2)(5);
    local f1 = {a, b = 3: a + b};
    local g1 = function(a, n: = 2) { return a + n; };
    local h1 = method(a: = self, b?) { return a; };
    local k1 = {a = 1, b?, c: = 2, d:: a};
    f1(1); g1(1, n: 5); h1(); k1(d: 1);
    m(); m(1, 2); n(); n(1, 2, 3); o(1, 2); o(a: 1);
}
"""
)

programs["param6"] = (
    BUILTINS
    + """
// Optional parameters with nothing else, enough of them that their
// locals pass the instructions with a local's number built in.
f(a?, b?, c?, d?, e?, g?, h?, i?) { return i; }
class A: object
    construct(a = 1, b: = 2) { x = a + b; }
    x = nil
;
// A try of the function's own after one the compiler makes.
g(a: = f())
{
    try { return a; } catch (A e) { return e; } finally { f(); }
}
_main(args)
{
    f(1, 2, 3, 4, 5, 6, 7, 8);
    new A(); new A(1); new A(b: 3);
    g();
    "<<f(1)>> and <<g(a: 1)>>";
}
"""
)

mistakes["e_param1"] = (
    BUILTINS
    + """
f(a?, b) { }
k(c = , d) { }
n(e?: , g) { }
h(i = 3 j) { }
h2(k2: = ) { }
h3(l3 ? m3) { }
h4(n4 = 1 ?) { }
h5(o5:: p5) { }
p(a?) { }
_main(args)
{
    p(a: 1...);
    p(1, 2, a: );
    p(1, 2, 3, 4);
    p(a: 1, a: 2, 3, 4);
    tadsSay(a: 1);
    tadsSay(1, a: [1,2]...);
    p(x: "dstr");
    p("dstr", y: 1);
    local y = {a: , b:: a + b};
    local z = {a?, b = 2: a};
    local w = {a:?, b: a};
}"""
)

mistakes["e_param2"] = """intrinsic 'tads-io/030007' { tadsSay(val, ...); }
f(a?) { }
k(x)
{
    x++;
    return x;
}
g(a:) { }
obj: object
  m(a?) { }
  p = (nosuch1 + 1)
  n(b: = 3) {
     tadsSay(1);
  }
  q(b?) { }
;
h(c?) { }
_main(args) { f(); g(a: 1); }"""


# A missing operand is taken to be the last number read: here 5, which
# is no index of the list.
mistakes["e_param3"] = """_main(args)
{
    local a = [1, 2][2];
    local b = [1, 5]["""


# Enumerations.
programs["enum1"] = (
    BUILTINS
    + """
enum red, green, blue;
enum token tokWord, tokNum;
f(x) { return x == red ? 1 : 2; }
enum ab, ba, zz, a1, b0, later;
obj: object
    color = red
    list = [red, green, [blue, tokWord]]
    m() { return color == blue; }
;
_main(args)
{
    local c = green;
    switch (c) { case red: tadsSay('r'); break; case green: tadsSay('g'); break; case early: break; }
    // The reference never finds two of them equal when it compares
    // them itself.
    tadsSay(red == red ? 'eq' : 'ne');
    tadsSay(red != red ? 'ne2' : 'eq2');
    tadsSay(red == green ? 'eq3' : 'ne3');
    tadsSay([red, green] - red == [green] ? 'y' : 'n');
    tadsSay([red] == [red] ? 'y' : 'n');
    tadsSay(c == green ? 'y' : 'n');
    tadsSay((red ? 'T' : 'F') + (!red ? 'T' : 'F'));
    tadsSay(f(ab) + f(ba) + f(zz) + f(a1) + f(b0) + f(later) + f(early));
    if (red) tadsSay('!');
    local l = [ab, ba, zz, a1, b0, later, early, tokNum];
    return l.length() + (red is in (red, green) ? 1 : 0) + (red ?? 1) + (red, 1);
}
enum early;
enum red, tokWord;
enum token green;
"""
)

mistakes["e_enum1"] = (
    BUILTINS
    + """
enum ;
enum a b;
enum c, ;
enum d,, e;
enum 5, g;
enum token;
f() { }
enum f;
enum h;
h() { }
enum i;
i: object;
obj1: object p2 = 1;
enum obj1;
enum p2;
enum q;
property q;
enum r;
class C: r;
export r;
m(x)
{
    r = 1;
    switch(x) { case r: case r: return 1; }
    for (local r in [1]) ;
    "<<r>>";
    return [&r, r.x, r(1), new r, r + 1, r < q, r + 'x', 'x' + r, [1][r], -r, r && 1, x.(r), r - 1, [r, 1] + r, r++];
}
_main(args) { }
enum last"""
)

# Properties whose values are worked out before the program starts.
programs["static1"] = (
    BUILTINS
    + """
counter: object n = 0 next() { return ++n; };
a: object
    p1 = static counter.next()
    p2 = static [1, 2, 3]
    p3 = static (counter.next() + p1)
    p4 = static 'const'
    p5 = static "dstr"
    p6 = static nosuchprop
    p7 = (counter.next())
    m() { return p1; }
;
f() { return 1; }
class B: object
    q1 = static [counter.next(), f(), self]
    q2 = static tadsSay('init q2')
    q3 = static {: counter.next() }
    q4 = static {: self }
    q5 = static (new B())
    q6 = static (targetprop)
    q7 = static
        [1,
         f()]
;
b2: B q1 = static 7 + f();
_main(args)
{
    tadsSay(a.p1); tadsSay(a.p1); tadsSay(a.p3); tadsSay(a.p7); tadsSay(a.p7);
    tadsSay(B.q1.length()); tadsSay(b2.q1); tadsSay(a.p6 == nil ? 'nil' : 'x');
}
"""
)

mistakes["e_static1"] = (
    BUILTINS
    + """
_main(args) { }
a: object p = static ;
b: object p = static static 1;
g: object p = static f()
 q = static ("x", 1) r = static nosuch(1);
h: object p = static function() { return 1; } q = static method() { return self; };
i: object p = static 1 + nosuch p2 = static [nosuch2] ;
j: object construct = static 1 x = static inherited();
l: object m() { return u1.q; } q = 1;
n(x) { return x.u1; }
c: object static p = 1;
d: object p(x) = static 1;"""
)


# Second definitions: `replace` and `modify`, of functions, of objects,
# and of the classes built in.
programs["modify1"] = (
    BUILTINS
    + """
f(x) { return x + 1; }
g(x) { return x * 2; }
h(x) { return 'h'; }
replace f(x) { return x + 10; }
modify g(x) { local r = replaced; return r(x) + replaced(x) + 100; }
modify function h(x) { return replaced(x) + '!'; }
replace function h(x) { return 'new h'; }
q(x) { return 'q'; }
modify q(x) { return ({: replaced(x) + '!' })(); }
k(a, b?) { return a; }
modify k(a, b?, c?) { return replaced(a) + replaced(a, 2) + replaced(a, [1]...); }
v(a, ...) { return a; }
modify v(a, [rest]) { return replaced(a, rest...) + replaced(a, n: 1); }
_main(args)
{
    tadsSay(f(1)); tadsSay(g(1)); tadsSay(h(1)); tadsSay(k(1)); tadsSay(q(1)); tadsSay(v(1));
}
"""
)

programs["modify2"] = (
    BUILTINS
    + COLLECTION
    + """
class Base: object
    name = 'base'
    describe() { return 'I am ' + name; }
;
thing: Base
    name = 'thing'
    weight = 5
    describe() { return inherited() + ', weight ' + weight; }
;
other: Base name = 'other';
modify thing
    weight = 7
    color = 'red'
    describe() { return inherited() + ', ' + color; }
;
object name = 'no name';
modify thing
    replace weight = 9
    replace describe() { return 'replaced: ' + inherited(); }
    size = static [weight]
;
modify Base
    describe() { return '[' + inherited() + ']'; }
    extra = 1
;
replace other: object name = 'replaced other' describe() { return name; };
class Gone: object x = 1;
replace class Gone: Base x = 2;
aa: object v = 1;
modify aa v = 2;
modify aa { v = 3 w = 4 }
replace aa: Base, Gone v = 5;
modify aa v = 6;
_main(args)
{
    tadsSay(thing.describe()); tadsSay(other.describe());
    tadsSay(new Base().describe()); tadsSay(aa.v); tadsSay(thing.sourceTextOrder);
    tadsSay(thing.ofKind(Base) ? 'y' : 'n'); tadsSay(Gone.x);
}
"""
)

programs["modify3"] = (
    BUILTINS
    + COLLECTION
    + """
modify List
    second() { return self[2]; }
    describe() { return 'a list'; }
;
f() { return 1; }
modify Object
    hello() { return 'hello from ' + name; }
    name = 'something'
    made = static f()
;
modify List
    second() { return 'second: ' + inherited(); }
    third = 3
;
thing: object name = 'thing';
modify Collection count = 0;
modify List;
_main(args)
{
    local l = [1, 2, 3];
    tadsSay(l.second()); tadsSay(l.describe()); tadsSay(thing.hello());
    tadsSay(l.hello()); tadsSay(l.third); tadsSay(l.count);
}
"""
)

mistakes["e_modify1"] = (
    BUILTINS
    + """
f() { }
replace f(x) { }
g() { }
modify g(x) { }
l() { }
replace function l;
replace ;
replace 5;
b: object;
replace b() { }
modify b() { }
m() { }
replace m: object;
_main(args) { }"""
)

mistakes["e_modify2"] = (
    BUILTINS
    + """
replace a: object;
modify b x = 1;
c: object;
c: object;
d: object replace x = 1;
e: object;
modify e replace x = 1 replace 5 = 2;
modify 5 x = 1;
g: object;
replace g x = 1;
h: object;
modify h: object x = 1;
enum i;
modify i x = 1;
j: object p = 1;
modify j p = 2 p = 3;
k: object x = static f() y = 1 z() { };
f() { }
modify k replace x = 2 replace z = 3 replace q = 4;
_main(args) { }"""
)

mistakes["e_modify3"] = (
    BUILTINS
    + COLLECTION
    + """
modify List length() { return 1; };
modify List length = 1 createIterator() { } ofKind = 2 x = 3;
replace List: object;
modify List: object x = 1;
modify List replace x = 1;
class List2: List;
modify Nope x = 1;
_main(args) { }"""
)

mistakes["e_modify7"] = (
    BUILTINS
    + """
h() { return replaced(); }
i() { return replaced; }
a: object m() { return replaced(1); } p = (replaced);
j(a) { }
modify j(a) { return replaced() + replaced(1, 2); }
k() { }
modify k() { }
replace k() { return replaced(); }
n(x) { return 1; }
modify n(x) { return ({: replaced(x) })(); }
replace n(x) { return 2; }
_main(args) { }"""
)

# `replace` and `modify` of a function before the function: the
# reference finds it when it puts the program together, and each of
# these is all it then says.
mistakes["e_modify4"] = "replace f() { }\n_main(args) { }"
mistakes["e_modify5"] = "modify f() { return replaced(); }\ng() { }\nmodify g() { }\n_main(args) { }"
mistakes["e_modify6"] = "replace f() { }\nf() { }\nmodify a x = 1;\na: object;\n_main(args) { }"


# Names declared before they are defined, and objects that are not
# saved.
programs["extern1"] = (
    BUILTINS
    + """
extern function f(x);
extern function g;
extern method m2(a, b?);
extern object later;
extern class Cls;
extern object never;
extern class NeverCls;
extern function unused(a, b, ...);
_main(args) { tadsSay(f(1)); tadsSay(g()); tadsSay(later.name); tadsSay(new Cls().name); }
f(x) { return x + 1; }
g() { return 'g'; }
later: Cls name = 'later';
class Cls: object name = 'cls';
object name = 'anon';
transient t1: object name = 't1';
transient Cls name = 'anon transient';
t2: object;
transient object name = 'x';
modify t1 extra = 1;
replace transient t2: Cls;
extern object later;
extern class Cls;
extern function f(y);
"""
)

mistakes["e_extern1"] = (
    BUILTINS
    + """
extern;
extern foo;
extern object;
extern object 5;
extern function;
extern function f1(x) { }
extern function f2 { }
f4(x) { }
extern function f4(x, y);
extern function f5(x);
f5(x, y) { }
a1: object;
extern class a1;
class a2: object;
extern object a2;
extern function a2;
f6() { }
extern object f6;
transient f7() { }
transient;
transient 5;
_main(args) { }"""
)

# What `extern` declares and nothing defines: found when the program is
# put together, if anything uses it.
mistakes["e_extern2"] = (
    BUILTINS
    + """
extern function f(x);
extern function g(a, b?, ...);
extern object o;
extern class C;
extern object unused;
x: C;
_main(args) { g(1, 2, 3, 4); return [f(1), o, new C()]; }"""
)

mistakes["e_extern3"] = "extern object o;\nmodify o x = 1;\nextern object p;\nreplace p: object;\n_main(args) { }"
mistakes["e_extern4"] = "extern function g;\ng(a, b) { return a; }\n_main(args) { g(1, 2); }"

# Object templates.
programs["template1"] = (
    BUILTINS
    + """
object template 'name' "desc"?;
object template 'name' @location "desc"?;
object template [items];
class Thing: object
    name = 'thing' location = nil desc = "nothing special" items = []
    show() { tadsSay(name); tadsSay(':'); desc; tadsSay(location == nil ? '-' : location.name); }
;
Thing template 'name' 'vocab' @location? "desc"?;
Thing template +weight 'name' | "desc";
Thing template ->dest 'name';
class Room: Thing dest = nil;
Room template 'name' "desc" ->dest?;
Room template [exits] inherited;
hall: Room 'Hall' "A long hall." ->kitchen;
kitchen: Room [1, 2, 3] 'Kitchen' "A kitchen. <<name>> it is." exits2 = 1;
lamp: Thing 'lamp' 'brass lamp' @hall "It is <<weight>> heavy." weight = 3;
Thing 'coin' 'gold coin' @kitchen;
Thing +5 "a five";
Thing +6 'six';
box: Thing { 'box' 'wooden box' weight = 9 }
bag: Thing 'bag' 'cloth bag' { weight = 1 };
object 'plain' "A plain object.";
object [1, 2];
door: Thing ->hall 'door';
r3: Room [4, 5] 'Cellar' 'cellar vocab' @hall "Dark.";
fn: Thing 'fn' 'a function' @({: self }) "Says <<{: name }()>>.";
transient Thing 'gone' 'not saved';
say(val) { tadsSay(val); }
_main(args)
{
    t3SetSay(say);
    hall.show(); kitchen.show(); lamp.show(); box.show(); bag.show(); door.show(); r3.show();
    tadsSay(hall.dest.name); tadsSay(lamp.vocab); tadsSay(r3.vocab);
}
"""
)

programs["template2"] = (
    BUILTINS
    + """
class A: object;
class B: object;
A template 'a';
B template 'b';
AB: A, B 'x';
BA: B, A 'x';
class A2: A;
A2 template 'a2';
X: A, A2 'x';
Y: A2, A 'x';
Z: B, A2 'z';
class C: A;
C template inherited "c";
C template [d] inherited @e;
C 'x' "y";
C [1] 'x' @nil;
C [1] @nil;
object template +f;
C template inherited -g;
C +1 -2;
C -2;
C 'x' -2;
object template 'h' | "i" | [j]? @k;
object 'x' @nil;
object @nil;
object [1] @nil;
object template 'l' ->m +n -o *p /q %r &s !t ~u ,v [w] "y";
object 'x' ->1 +2 -3 *4 /5 %6 &q !7 ~8 ,9 [1] "m";
object template 'aa' 'bb'? 'cc';
object 'x' 'y';
object 'x' 'y' 'z';
Later 'x';
class Later: A;
transient object template 'tt' 'uu' 'vv' 'ww';
_main(args) { return AB.a + BA.b; }
"""
)

mistakes["e_template1"] = (
    BUILTINS
    + """
object template;
object template 'a b';
object template 'if';
object template [5];
object template [a;
object template @;
object template @5 'x';
object template 'a' 'a';
object template 'a' | "a" | [a];
object template 'a' | "a" 'a';
object template foo;
f() { }
object template 'f';
f template 'a';
object template 'c' | ;
object template 'a' ? | "b" ?;
string template 'a';
object template 'a' { }
_main(args) { }"""
)

mistakes["e_template2"] = (
    BUILTINS
    + """
object 'x';
class C: object;
C template 'a';
C "d";
object template 'a' "b";
object 'x' a = 'y';
object 'x' 'y';
object "x" + 5;
extern class E;
E 'x';
E2: E 'y';
class D: Nope 'x';
_main(args) { }"""
)


# Objects inside objects: `+` signs before a definition.
programs["plus1"] = (
    BUILTINS
    + """
+ property location;
class Thing: object name = 'thing' location = nil;
hall: Thing;
+ table: Thing;
++ cup: Thing;
++ Thing name = 'anon';
+++ lid: Thing;
+ chair: Thing location = cup;
+ class Furniture: Thing;
++ onFurn: Thing;
class Plain: Thing;
+ afterClass: Thing;
+ transient ghost: Thing;
+ transient Thing;
+ object name = 'x';
modify table extra = 1;
++ underModify: Thing;
+ besideModify: Thing;
x2: Thing;
modify Thing more = 2;
+ y2: Thing;
replace cup: Thing;
+++ z: Thing;
++++ deep: Thing;
+ property loc2;
+ w: Thing;
object template 'name' @location;
+ Thing 'tpl' @hall;
+ both: Thing 'tpl' @hall loc2 = 5;
modify Object objExtra = 1;
+ underMeta: Thing;
+ Thing { name = 'braces' }
++ last: Thing;
_main(args) { tadsSay(lid.location.name); tadsSay(underModify.location.extra); tadsSay(last.loc2.name); }
"""
)

mistakes["e_plus1"] = """+ a: object;
+ property;
+ property 5;
+ property 6
+ property f;
f() { }
+ b: object;
+ property loc;
x: object;
+++ y: object;
+ 5;
++ property loc;
+ class;
+ transient class C2: object;
+ modify x;
+ c: object
+ d: object;
_main(args) { }
"""

mistakes["e_plus2"] = """+ property loc;
x: object;
+ replace x: object;
++ function g() { }
+ x: object;
+ x 'a';
+ +;
+ property loc, loc2;
+ y: object + z: object;
_main(args) { }
"""

# An object as the value of a property.
programs["nested1"] = (
    BUILTINS
    + """
class Thing: object name = 'thing';
class Thing2: object;
object template 'name';
a: Thing
  inner: Thing { name = 'inner' deep: Thing { name = 'deep' m() { return lexicalParent; } } }
  other: object { }
  tpl: Thing 'tname' { x = 1 }
  tpl2: Thing { 'tname2' x = 2 }
  fn = ({: self })
  list = [1, 2]
  two: Thing, Thing2 { f() { return ({: lexicalParent.name }); } }
  lexicalParent = 77
;
modify a n2: Thing { } replace inner: Thing { name = 'replaced' };
modify Object helper: object { v = 1 };
class C: object sub: Thing { };
b: Thing { nested: Thing { } }
transient t: Thing sub: Thing { };
_main(args) { tadsSay(a.inner.name); tadsSay(a.tpl.lexicalParent.name); tadsSay(a.two.f()()); tadsSay(b.helper.v); }
"""
)

mistakes["e_nested1"] = """a: object
  lexicalParent = 1
  n: object { lexicalParent = 2 }
  m: object { x = 1 x = 2 }
  k: Undefined { }
  f: object { y }
  g: f2 { }
  h: 5 { }
;
f2() { }
_main(args) { }
"""

mistakes["e_nested2"] = """b: object
  n: object x = 1;
b2: object
  n2: object;
c: object
  n: object { p: object q = 1 }
;
d: object
  c: object { };
e: object
  n: object { p: object { r: object s = 2 } t = 3 } u = 4
;
_main(args) { }
"""

# Groups of properties that share a pattern for their names.
programs["propset1"] = (
    BUILTINS
    + """
a: object
  propertyset 'ver*Take' { Dobj = 1 Iobj() { return 2; } Sub: object { z = 3 propertyset 'in*' { ner = 9 } } ; ; propertyset 'x*' { A = 4 B(q) { return q; } } C = 5 }
  propertyset '*' { plain = 1 }
  propertyset '*Only' { }
  propertyset 'pre*' (who, *) { V = 6 }
  after = 5
;
b: object { propertyset 'p*' { q = 1 } r = 2 }
class K: object propertyset 'k*' { m { return 1; } n(a, b?, [c]) { return a; } };
modify a propertyset 'ver*Take' { replace Dobj = 10 };
_main(args) { tadsSay(a.verDobjTake + a.verIobjTake() + a.verxATake + a.verxBTake(5) + a.verCTake + a.verSubTake.inner + a.plain + a.preV + b.pq); }
"""
)

mistakes["e_propset1"] = """a: object
  propertyset 5 { }
  propertyset 'no star' { x = 1 }
  propertyset '**' { y = 1 }
  propertyset '5*' { z = 1 }
  propertyset '' { z0 = 1 }
  propertyset 'a*' ( { }
  propertyset 'b*' (x y, , *) { }
  propertyset 'c*' (x,) { }
  propertyset 'd*' (*, *) { }
  propertyset 'e*' w = 1 }
  propertyset 'g*' { x = 2 x = 3 }
  propertyset 'h*' { ; class
;
b: object
  propertyset;
c: object { propertyset 'a*' { x = 1 } 
_main(args) { }
"""

mistakes["e_propset2"] = """a: object
  propertyset 'a*' { propertyset 'b*' { propertyset 'c*' { propertyset 'd*' { propertyset 'e*' { propertyset 'f*' {
  propertyset 'g*' { propertyset 'h*' { propertyset 'i*' { propertyset 'j*' { propertyset 'k*' { propertyset 'l*' {
    deep = 1
  } } } } } } } } } } } }
  propertyset 'abcdefghijklmnopqrstuvwxyz*abcdefghijklmnopqrstuvwxyz' { propertyset 'abcdefghijklmnopqrstuvwxyz*' { long = 1 } }
  propertyset 'q*' { f: object { } g: a { } h }
;
f() { }
_main(args) { }
"""


# Methods that stand for operators.
programs["op1"] = (
    BUILTINS
    + """
class Vec: object
    x = 0 y = 0
    construct(x_, y_) { x = x_; y = y_; }
    operator +(o) { return new Vec(x + o.x, y + o.y); }
    operator -(o) { return new Vec(x - o.x, y - o.y); }
    operator *(k) { return new Vec(x * k, y * k); }
    operator /(k) { return new Vec(x / k, y / k); }
    operator %(k) { return new Vec(x % k, y % k); }
    operator ^(k) { return 'xor' + k; }
    operator <<(k) { return 'shl' + k; }
    operator >>(k) { return 'ashr' + k; }
    operator >>>(k) { return 'lshr' + k; }
    operator ~() { return 'bnot'; }
    operator |(k) { return 'or' + k; }
    operator &(k) { return 'and' + k; }
    operator negate() { return new Vec(-x, -y); }
    operator [](i) { return i == 1 ? x : y; }
    operator []=(i, v) { if (i == 1) x = v; else y = v; return self; }
    show() { return '(' + x + ',' + y + ')'; }
;
other: object
    operator + = 5
    operator negate { return 'neg'; }
    operator [ ] (i) { return i * 2; }
;
modify Vec
    replace operator ^(k) { return 'XOR' + k; }
;
_main(args)
{
    local a = new Vec(1, 2), b = new Vec(10, 20);
    tadsSay((a + b).show()); tadsSay((b - a).show()); tadsSay((a * 3).show());
    tadsSay((b / 2).show()); tadsSay((b % 3).show()); tadsSay((-a).show()); tadsSay('\\n');
    tadsSay(a ^ 1); tadsSay(a << 2); tadsSay(a >> 3); tadsSay(a >>> 4); tadsSay(~a);
    tadsSay(a | 5); tadsSay(a & 6); tadsSay('\\n');
    tadsSay(a[1]); tadsSay(a[2]); a[2] = 9; tadsSay(a.show()); tadsSay(other[21]); tadsSay(-other); tadsSay('\\n');
    local p = &operator +, q = &operator []=, r = &operator negate, s = &operator [];
    tadsSay(a.(p)(b).show()); tadsSay(other.(r)()); tadsSay(other.(s)(4));
    tadsSay(other.propType(&operator +)); tadsSay('\\n');
}
"""
)

mistakes["e_op1"] = """a: object
    operator +() { }
    operator -(a, b) { }
    operator negate(a) { }
    operator ~(a) { }
    operator [](a, b) { }
    operator []=(a) { }
    operator *(a, ...) { }
    operator /([a]) { }
    operator %(a?) { }
    operator foo(a) { }
    operator = 5
    operator ==(a) { }
    operator [ = 5
    operator [5] = 6
    operator + = 1
    operator + = 2
    propertyset 'x*' { operator -(a) { } operator <<(b) { } }
    operator
;
_main(args) { local x = &operator, y = &operator foo, z = &operator ==; }
"""

# A method of another object run as this one's, and the count of arguments.
programs["deleg1"] = (
    BUILTINS
    + """
class A: object
    name = 'A'
    who() { return 'A.who as ' + name; }
    add(a, b) { return name + (a + b); }
    many([lst]) { return name + lst.length(); }
;
class B: object
    name = 'B'
    who() { return 'B.who as ' + name; }
;
c: object
    name = 'c'
    target = A
    who() { return delegated A.who(); }
    who2() { return delegated B.who; }
    who3() { return delegated target.who(); }
    who4() { return delegated (target).who(); }
    who5(p) { return delegated A.(p)(); }
    who6() { return delegated A(); }
    add(a, b) { return delegated A(a, b) + delegated A.add(b, a); }
    many([lst]) { return delegated A.many(lst...) + delegated A.(&many)(1, lst...); }
    who7() { return delegated A.targetprop(); }
    who8() { local f = {: delegated B.who() }; return f(); }
    cnt(a?, b?, [r]) { return argcount; }
    lst = [A, B]
    who9() { return delegated lst[2].who(); }
    who10() { return delegated self.target.who(); }
    who11() { delegated A.who(); return delegated new B().who(); }
;
f(a?, b?) { return argcount + ({: argcount })(); }
_main(args)
{
    tadsSay(c.who()); tadsSay(c.who2()); tadsSay(c.who3()); tadsSay(c.who4()); tadsSay(c.who5(&who)); tadsSay(c.who6()); tadsSay('\\n');
    tadsSay(c.add(1, 2)); tadsSay(c.many(1, 2, 3)); tadsSay(c.who7()); tadsSay(c.who8()); tadsSay(c.cnt()); tadsSay(c.cnt(1, 2, 3)); tadsSay('\\n');
    tadsSay(c.who9()); tadsSay(c.who10()); tadsSay(c.who11()); tadsSay(f(1)); tadsSay(argcount); tadsSay('\\n');
}
"""
)

mistakes["e_deleg1"] = """class A: object who() { return 1; };
f() { return delegated A.who(); }
g() { return delegated A; }
c: object
    m1() { return delegated; }
    m2() { return delegated A.; }
    m4() { return delegated .who(); }
    m5() { return delegated A.who().x; }
    m6() { return delegated undefinedThing.who(); }
    m7() { return delegated A.undefinedProp(1, 2); }
    m8 = (delegated A.who())
    m9 = static (delegated A.who())
;
_main(args) { return delegated A.who() + argcount; }
"""

mistakes["e_deleg2"] = """class A: object who() { return 1; };
f() { return delegated A.who(); }
g() { return delegated A; }
c: object
    m6() { return delegated undefinedThing.who(); }
    m7() { return delegated A.undefinedProp(1, 2); }
    m8 = (delegated A.who())
    m9 = static (delegated A.who())
    m10() { delegated A; delegated A.who; return ({: delegated A.x }); }
    operator +(x) { return argcount; }
;
_main(args) { return delegated A.who() + argcount + f.(&operator +) + g.(&operator []=)(1); }
"""


# Numbers that are not integers, and regular expressions.
programs["float1"] = """_main(args) { local x = 1.5; local y = R"a+"; return [x, 2.25, y, 2147483647 + 1, 0x100000000, 3.0 * 2, 1e3, -2147483648, 4294967296 - 4294967295]; }
"""

programs["float2"] = """#include <bignum.h>
#include <tads.h>
obj: object
    half = 0.5
    big = 4294967296
    lst = [1.5, 2, 'x', 3.25e2]
    calc = (1.5 + 2)
    pat = R'a+b'
    pats = [R"x*", R'%d+']
;
show(v) { tadsSay(toString(v)); tadsSay(' '); }
_main(args)
{
    show(1.5 + 2.25); show(1.5 - 2); show(2 * 1.5); show(7.5 % 2);
    show(-1.5); show(-(-1.5)); show(+1.5); show(1e3); show(1.0e-3); show(.5); show(5.); tadsSay('\\n');
    show(2147483647 + 1); show(-2147483647 - 2); show(2147483647 + 1 - 1); show(4294967296 - 4294967295);
    show(0x7fffffff + 1); show(0xffffffff); show(0x100000000); show(077777777777); show(-2147483648); show(-(-2147483648)); tadsSay('\\n');
    show(1.5 < 2); show(2.5 > 2.25); show(1.0 == 1); show(1 == 1.0); show(1.5 != 1.5); show(2147483648 >= 2147483647);
    show(1.5 ? 'y' : 'n'); show(0.0 ? 'y' : 'n'); show(!0.0); show(!1.5); show(0.0 && 1); show(0.5 || nil); tadsSay('\\n');
    show('a' + 1.5); show(1.5 + 'a'); show('v' + 2147483648); show([1] + 1.5); show([1.5, 2.5] - 1.5); show(1.5 is in (1, 1.5));
    show(obj.half); show(obj.big); show(obj.lst[4]); show(obj.calc); show(1.50 == 1.5); show(1.5.getWhole()); tadsSay('\\n');
    show(4294967296 * 2); show(4294967296 / 4294967296); show(2147483648 / 2);
    show(1.23456789012345678901234567890 + 1); show(100000 * 100000); show(-2147483648 / 1);
    show(3000000000); show(3000000000 - 1000000000); show(1e10 - 1e10 + 5); tadsSay('\\n');
    for (local i in 1..2 step 0.5) show(i);
    for (local i in 2..1 step -0.5) show(i);
    switch (1.5) { case 1.5: show('case'); break; default: show('default'); }
    show(rexMatch(obj.pat, 'aaab')); show(rexMatch(R'%d+', '123x')); show(rexMatch(obj.pats[1], 'xxxy')); tadsSay('\\n');
}
"""

mistakes["e_float1"] = """_main(args)
{
    local a = 1.5 | 2;
    local b = 1 & 2.5;
    local c = 1.5 << 1;
    local d = 2 >> 1.0;
    local e = 1.5 ^ 2.5;
    local f = ~1.5;
    local g = 1.5 / 0;
    local h = 1.5 / 0.0;
    local i = 1 % 0.0;
    local j = 1.5 < 'a';
    local k = nil + 1.5;
    local l = 1.5 - 'a';
    local m = 1.5[1];
    local n = [1, 2][1.5];
    local o = -'a' + +'b' + -R'x' + +R'y';
    local p = R'a' + 'b';
    local q = R'a' < R'b';
    local r = R'a'[1];
    local s = 1.5 * nil;
    local t = 4294967296 | 1;
    local u = 2147483648 >>> 1;
    return [a, b, c, d, e, f, g, h, i, j, k, l, m, n, o, p, q, r, s, t, u];
}
"""


# Lists of keys and values, which are lookup tables.
programs["lookup1"] = """#include <lookup.h>
#include <tads.h>
obj: object
    tab = ['a' -> 1, 'b' -> 2]
    deflt = [1 -> 'one', * -> 'other']
    only = [* -> 0]
    nested = [[1 -> 2], ['k' -> ['x' -> 'y']]]
    m(k) { return [k -> k * 2, k + 1 -> [k]]; }
;
f(x) { return x; }
_main(args)
{
    local t = ['one' -> 1, 'two' -> 2, 'three' -> f(3), * -> -1];
    tadsSay(t['one']); tadsSay(t['three']); tadsSay(t['nine']); tadsSay(obj.tab['b']); tadsSay(obj.deflt[7]);
    tadsSay(obj.only[1]); tadsSay(obj.nested[2]['k']['x']); tadsSay(obj.m(4)[5][1]); tadsSay('\\n');
    [1 -> f(2)];
    ['a' -> 1];
    local e = [ ];
    local u = [f(1) -> f(2), * -> f(3)];
    tadsSay(u[1]); tadsSay(u[9]); tadsSay(e.length());
    tadsSay([1 -> 2].ofKind(LookupTable) ? ' table' : ' list'); tadsSay(['a' -> 'b']['a']); tadsSay('\\n');
}
"""

mistakes["e_lookup1"] = """_main(args)
{
    local a = [1, 2 -> 3];
    local b = [1 -> 2, 3];
    local c = [1 -> 2, 3, 4 -> 5];
    local d = [1 -> 2 -> 3];
    local e = [1 -> 2, * -> 3, 4 -> 5];
    local f = [1, * -> 2];
    local g = [* -> 1, 2];
    local h = [1 -> ];
    local i = [-> 1];
    local j = [1 -> 2,];
    local k = [* 2];
    local l = [1 -> 2, 3 -> 4 -> 5, 6];
    local m = [1 -> 2 3 -> 4];
    local n = [* -> 1 -> 2];
    local o = [1 -> 2, * -> 3;
    return [a, b, c, d, e, f, g, h, i, j, k, l, m, n, o] + [1 -> 2] - [3 -> 4] + [1 -> 2][1];
}
"""

# A choice among alternatives in a string.
programs["oneof1"] = (
    BUILTINS
    + """
say(val) { tadsSay(val); }
class OneOfIndexGen: object
    numItems = 1
    listAttrs = ''
    idx = 0
    getNextIndex()
    {
        if (listAttrs == 'seq,stop')
            return idx < numItems ? ++idx : numItems;
        idx = idx % numItems + 1;
        return idx;
    }
;
obj: object
    name = 'obj'
    desc = "It is <<one of>>red<<or>>green<<or>>blue<<cycling>>. "
    text = 'A <<one of>>big<<or>>small<<stopping>> thing'
    mixed = 'The <<one of>><<name>><<or>>other<<shuffled>> one'
    once = "<<first time>>Hello! <<only>>Again. "
    deep = "<<one of>>a<<one of>>1<<or>>2<<cycling>><<or>>b<<if name == 'obj'>>!<<end>><<at random>> "
;
_main(args)
{
    t3SetSay(say);
    obj.desc; obj.desc; obj.desc; obj.desc; say('\\n');
    say(obj.text); say(obj.text); say(obj.text); say('\\n');
    say(obj.mixed); say(obj.mixed); say('\\n');
    obj.once; obj.once; obj.deep; obj.deep; obj.deep; say('\\n');
    local s = 'x<<one of>>1<<or>>2<<or>>3<<then at random>>y';
    "<<one of>>p<<or>>q<<sticky random>> <<s>> <<one of>>only<<purely at random>>\\n";
}
"""
)

programs["oneof2"] = """extern class OneOfIndexGen;
_main(args)
{
    return 'x<<one of>>1<<or>>2<<cycling>>y';
}
class OneOfIndexGen: object;
f() { return "<<one of>>a<<or>><<one of>>b<<or>>c<<half shuffled>><<then purely at random>>"; }
"""

mistakes["e_oneof1"] = """_main(args)
{
    local a = 'x<<one of>>1<<or>>2<<cycling>>y';
    "<<one of>>p<<or>>q<<stopping>>";
    "<<first time>>p<<only>>";
    return a;
}
"""

mistakes["e_oneof2"] = """class OneOfIndexGen: object;
getNextIndex() { }
numItems() { }
_main(args)
{
    local a = 'x<<one of>>1<<or>>2<<cycling>>y';
    "<<one of>>p<<or>>q<<stopping>>";
    return a;
}
"""


# Dictionaries, and the words objects have in them.
programs["dict1"] = (
    BUILTINS
    + """
dictionary gDict;
dictionary property noun, adjective, plural;
class Thing: object
    noun = 'thing' 'object'
    name = 'x'
;
lamp: Thing
    noun = 'lamp' 'lantern' 'Lamp'
    adjective = 'brass' 'old'
;
key: Thing
    adjective = 'brass'
    noun = 'key'
;
Thing noun = 'anon' 'lamp';
plain: object name = 'plain';
class Sub: Thing plural = 'subs';
sub1: Sub noun = 'sub';
_main(args) { }
"""
)

programs["dict2"] = """dictionary property noun, adjective;
before: object name = 'no dict yet';
dictionary gDict;
class Thing: object noun = 'thing';
lamp: Thing noun = 'lamp' adjective = 'brass';
modify lamp noun = 'lantern' weight = 1;
key: Thing noun = 'key' adjective = 'small';
modify key replace noun = 'passkey';
modify Thing adjective = 'generic';
coin: Thing;
replace coin: Thing noun = 'coin2';
box: Thing
    noun = 'box'
    lid: Thing { noun = 'lid' adjective = 'hinged' }
    adjective = 'wooden'
;
_main(args) { return gDict; }
"""

programs["dict4"] = """dictionary property noun;
dictionary zDict;
object noun = 'only anon';
class K: object noun = 'k';
K;
K noun = 'more' 'k';
_main(args) { }
"""

programs["dict5"] = """dictionary gDict;
dictionary property noun, adjective, plural;
class Top: object noun = 'top';
class Mid: Top adjective = 'mid';
class Low: Mid plural = 'lows';
x1: Low noun = 'x1';
class Other: object noun = 'other' plural = 'others';
x2: Low, Other;
x3: Other, Low adjective = 'x3';
class Late: Low;
zz: Late;
aa: Late;
_main(args) { }
"""

programs["dict6"] = """#include <dict.h>
#include <tads.h>
dictionary gDict;
dictionary property noun, adjective, plural;
+ property location;
object template 'name' @location?;
class Thing: object
    name = 'thing'
    location = nil
    noun = 'thing' 'object'
;
room: Thing 'room' noun = 'room' 'chamber';
+ lamp: Thing 'lamp' noun = 'lamp' 'lantern' adjective = 'brass' 'old' 'brass';
++ wick: Thing 'wick'
    noun = 'wick'
    holder: Thing { name = 'holder' noun = 'holder' adjective = 'tiny' }
    adjective = 'burnt'
    propertyset 'x*' { y = 1 }
;
+ Thing 'coin' noun = 'coin' 'coins' plural = 'coins';
coin2: Thing noun = 'coin';
replace coin2: Thing noun = 'token';
class Fancy: Thing adjective = 'fancy';
vase: Fancy noun = 'vase' noun2 = static ['a'];
modify vase adjective = 'blue' replace noun = 'urn';
transient ghost: Thing noun = 'ghost';
say(v) { tadsSay(v); }
names(lst) { local s = ''; foreach (local o in lst) s += o.name + ' '; return s; }
show(o) { say(o.name); say(':'); say(o.noun); say('/'); say(o.adjective); say('/'); say(o.plural); say(' '); }
_main(args)
{
    t3SetSay(say);
    show(Thing); show(room); show(lamp); show(wick); show(wick.holder); show(coin2); say('| ');show(Fancy); show(vase); show(ghost);
}
"""

programs["dict7"] = """dictionary property noun;
dictionary third;
C: object noun = 'inthird';
dictionary first;
a: object noun = 'infirst' 'both';
dictionary second;
b: object noun = 'insecond' 'both';
_main(args) { return [first, second, third]; }
"""

mistakes["e_dict1"] = """dictionary property noun;
a: object noun = 'x';
dictionary;
dictionary 5;
dictionary property;
dictionary property 5, adjective plural, , weight;
dictionary property size;
dictionary gDict;
dictionary gDict;
b: object
    noun = 5
    adjective = 'a' 'b' 5
    plural() { }
    size: object { }
    weight = 'w', 'x'
    noun = 'again'
;
object template 'noun';
object template 'name' @adjective;
c: gDict;
d: object noun = 'x' + 'y';
e: object noun = static 'x';
f: object noun = ['x', 'y'];
g: object noun = 'x' 'y' "z";
class gDict: object;
modify gDict x = 1;
replace gDict: object;
_main(args) { return gDict; }
"""


# A dictionary's name in code.
programs["dict8"] = """dictionary gDict;
dictionary property noun;
c: gDict noun = 'c';
d: object
    m() { local y = gDict.foo; inherited gDict.foo(); return delegated gDict.foo(); }
    p = gDict
    q = [gDict]
    r = static gDict
;
_main(args) { local x = &gDict; gDict.foo = 3; gDict.(&noun)(1); return [gDict + 1, x, d.p, gDict]; }
"""

mistakes["e_dict3"] = """dictionary gDict;
dictionary property noun;
c: gDict noun = 'c';
d: object
    m() { local y = gDict.foo; inherited gDict.foo(); return delegated gDict.foo(); }
    p = gDict
    q = [gDict]
    r = static gDict
;
_main(args) { local x = new gDict; local z = new gDict(1); x = &gDict; gDict = 5; return gDict + 1; }
"""


# Grammar productions.
programs["gram1"] = """#include <gramprod.h>
#include <tads.h>
dictionary gDict;
dictionary property noun, adjective;
enum token tokWord, tokInt;
class Match: object;
grammar nounPhrase(simple): noun->noun_ : Match
    name = 'simple'
;
grammar nounPhrase(adj): adjective->adj_ nounPhrase->np_ : Match;
grammar nounPhrase: 'the' nounPhrase->np_ | [badness 50] 'a' nounPhrase->np_ : Match;
grammar command(take): ('take' | 'get' | 'pick' 'up') nounPhrase->dobj_ : Match
    verb = 'take'
;
grammar command(put): 'put' nounPhrase->dobj_ ('in' | 'into' | ) nounPhrase->iobj_ ('please' | ) : Match;
grammar command: <noun adjective>->word_ tokInt->num_ * : Match;
grammar number: tokInt->val_ | tokWord : object;
grammar empty: : object;
grammar later;
_main(args) { }
"""

programs["gram2"] = """#include <gramprod.h>
#include <tads.h>
dictionary gDict;
dictionary property noun;
enum token tokWord;
class Match: object name = 'match';
grammar np(a): noun->n_ : Match;
grammar np(b): 'the' np->np_ : Match extra = 1;
grammar np(c): 'c1' | 'c2' : Match;
grammar vp(x): 'go' np->np_ : Match;
modify grammar np(a): tokWord->w_ : more = 2;
modify grammar np(b): : kept = 3;
replace grammar np(c): 'c3' ('c4' | 'c5') : Match;
grammar vp(x2): later->l_ 'q' : Match;
grammar later(only): [badness 10 + 5] undefinedSub->u_ | [badness 3] 'z' : Match;
grammar undefinedSub;
lamp: object noun = 'lamp';
grammar zz: (('a' | 'b') ('c' | 'd') | 'e') 'f' (| 'g') : object;
say(v) { tadsSay(v); }
_main(args)
{
    local m = np.parseTokens([['lamp', tokWord, 'lamp']], gDict);
    say(m.length());
    foreach (local x in np.getGrammarInfo()) { say(' '); say(x.gramMatchObj.grammarInfo()[1]); say('/'); say(x.gramMatchObj.grammarTag); }
    say('\\n'); say(zz.getGrammarInfo().length());
}
"""

mistakes["e_gram1"] = """class Match: object;
f() { }
enum plain;
grammar;
grammar 5;
grammar np(a: 'x' : Match;
grammar np(b) 'x' : Match;
grammar np(c): f : Match;
grammar np(d): Match : Match;
grammar np(e): plain : Match;
grammar np(f): 'x' -> 5 : Match;
grammar np(g): 'x' -> f : Match;
grammar np(h): <f 5> 'y' : Match;
grammar np(i): <a b 'x' : Match;
grammar np(j): * 'x' | 'y' * : Match;
grammar np(k): 'x' [badness 5] : Match;
grammar np(l): [goodness 5] 'x' : Match;
grammar np(m): [5] 'x' : Match;
grammar np(n): ('x' | 'y' : Match;
grammar np(o): ('x' | 'y')->p_ 'z' : Match;
grammar np(p): 5 + 'x' : Match;
grammar np(a): 'dup' : Match;
grammar f(a): 'x' : Match;
grammar Match(a): 'x' : Match;
grammar Match: 'x' : Match;
modify grammar np: 'x' : ;
replace grammar np: 'x' : Match;
modify grammar np(zz): 'x' : ;
grammar np(q): 'x' | : Match;
grammar np(r): : Match;
grammar np(s): 'x' ;
grammar np(t): 'x' { }
_main(args) { }
"""

mistakes["e_gram2"] = """class Match: object;

grammar np(a): missing->m_ other : Match;
grammar vp: np 'x' : Match;
grammar declared;
grammar np(b): declared 'y' : Match;
_main(args) { return [np, vp, missing]; }
"""

mistakes["e_gram3"] = """class Match: object;
grammar np(a): missing->m_ : Match;
grammar vp(x): 'x' : Match;
_main(args) { local x = new np; x = np.foo; np = 3; return vp(1); }
modify np y = 1;
replace vp: object;
class vp(x): Match;
"""


# Objects written in expressions, and questions about names.
programs["inl1"] = """#include <tadsgen.h>
#include <tads.h>
class Thing: object name = 'thing' describe() { return 'a ' + name; };
make(label, n)
{
    local count = 0;
    return object: Thing {
        name = label
        size = n * 2
        fixed = 7
        text = 'const'
        desc = "I am <<name>>. "
        bump() { return ++count; }
        twice(x) { return x * 2 + n; }
        made = static (count + 100)
        part: Thing { name = 'part of ' + label }
        construct() { count = 10; }
    };
}
other: object
    m()
    {
        local o = object { v = self.w  f() { return self; } };
        return o;
    }
    w = 5
;
say(v) { tadsSay(v); }
_main(args)
{
    t3SetSay(say);
    local a = make('box', 3), b = make('jar', 4);
    say(a.describe()); say(' '); say(a.size); say(' '); say(a.fixed); say(a.text); say(' '); a.desc;
    say(a.bump()); say(a.bump()); say(b.bump()); say(' '); say(a.twice(5)); say(' '); say(a.made); say(' ');
    say(a.part.name); say(' '); say(a.part.lexicalParent == nil ? 'nil' : 'obj'); say('\\n');
    local o = other.m(); say(o.v); say(o.f() == o ? ' same' : ' other'); say(o.ofKind(Thing) ? ' thing' : ' plain');
    local e = object { };
    say(e.ofKind(TadsObject) ? ' obj' : ' ?');
    say(defined(Thing) ? ' def' : ' undef'); say(defined(nope) ? ' def' : ' undef'); say(defined(make) ? ' def' : ' undef');
    say(__objref(Thing) == Thing ? ' ref' : ' noref'); say(__objref(nope) == nil ? ' nil' : ' ?');
    say(__objref(make, warn) == nil ? ' nil' : ' ?'); say(__objref(nope2, warn) == nil ? ' nil' : ' ?');
    if (defined(nope)) say('never');
    say('\\n');
}
"""

programs["inl2"] = """#include <tadsgen.h>
#include <tads.h>
class A: object name = 'A' who() { return name; };
class B: object tag = 'B';
holder: object
    name = 'holder'
    made = object: A, B { name = 'made' }
    lazy = (object { v = 1 })
    fresh() { return object: A { name = 'fresh!' owner = static self.name who() { return name + '?'; } }; }
    chain(n)
    {
        local k = n;
        return ({x: object { sum = x + k + n  inner: A { name = 'in' + k } get() { return ({: k++ }); } }});
    }
    grouped = static object { propertyset 'on*' { Start = 1 Stop() { return 2; } } operator +(o) { return 'plus'; } }
;
f(a, b?)
{
    for (local i = 0 ; i < 2 ; ++i)
    {
        local o = object: A { idx = i  desc = "<<idx>>;" };
        o.desc;
    }
    return object { args = [a, b]  count = argcount };
}
say(v) { tadsSay(v); }
_main(args)
{
    t3SetSay(say);
    say(holder.made.who()); say(holder.made.tag); say(holder.lazy.v); say(holder.fresh().who());
    local o = holder.chain(5)(10); say(o.sum); say(o.inner.name); local g = o.get(); say(g()); say(g()); say(o.sum); say('\\n');
    say(holder.grouped.onStart); say(holder.grouped.onStop()); say(holder.grouped + 1); say(' ');
    local r = f(1); say(r.count); say(r.args.length());
    if (defined(A) && !defined(Zed)) say(' ok');
    while (defined(Zed)) { say('never'); }
    say(__objref(A, error).name); say(__objref(holder).name);
    say('\\n');
}
"""

mistakes["e_inl1"] = """class Thing: object;
f() { }
_main(args)
{
    local a = object Thing { };
    local b = object: { };
    local c = object: Thing;
    local d = object { x = 1 x = 2 y };
    local e = object { replace z = 1 };
    local g = defined(1), h = defined(), i = defined(a, b);
    local j = __objref(1), k = __objref(a, b, c), l = __objref(a, maybe);
    local m = __objref(f, error), n = __objref(zip, error);
    return [a, b, c, d, e, g, h, i, j, k, l, m, n];
}
"""

mistakes["e_inl2"] = """class Thing: object;
_main(args)
{
    local a = object: Thing { x = 1 };
    local b = object { m() { return 1; } };
    return [a, b];
}
"""

mistakes["e_inl3"] = """#include <tadsgen.h>
#include <tads.h>
f() { }
_main(args)
{
    local c = object { x = undefinedThing };
    local d = object { m() { return undefinedOther; } };
    local e = __objref(Nope, warn), g = __objref(f, warn);
    object { };
    return [c, d, e, g, defined(a), __objref(a)];
}
"""


# Multi-methods: functions with types for their parameters.
programs["mm1"] = """#include <tadsgen.h>
#include <tads.h>
class Thing: object;
class Box: Thing;
_multiMethodRegister(base, func, [types]) { tadsSay('reg '); tadsSay(types.length()); tadsSay('\\n'); }
_multiMethodBuildBindings() { tadsSay('build\\n'); }
_multiMethodCall(base, args) { tadsSay('call '); tadsSay(args.length()); return 7; }
_multiMethodCallInherited(from, [args]) { return 8; }
put(Thing a, Box b) { return 1; }
put(Box a, b) { return inherited(a, b); }
put(a, String s, ...) { return inherited<Thing, Box>(a, s); }
show(x) multimethod { return 0; }
show(Thing x) { return 2; }
_main(args)
{
    tadsSay(put(Thing, Box)); tadsSay(show(1)); local f = put; tadsSay(f(1, 2, 3)); tadsSay('\\n');
}
"""

programs["mm2"] = """#include <tadsgen.h>
#include <tads.h>
class Thing: object;
class Box: Thing;
_multiMethodRegister(base, func, [types]) { }
_multiMethodBuildBindings() { }
_multiMethodCall(base, args) { return 7; }
_multiMethodCallInherited(from, [args]) { return 8; }
ab(Thing a) { return 1; }
ba(Box a) { return 2; }
ab(a) multimethod { return 3; }
ba(Box a, List b, ...) { return 4; }
ab(Thing a, [rest]) { return inherited(a); }
modify ab(a) multimethod { return replaced(a) + 1; }
replace ba(Box a) { return inherited<Box, List, ...>(a, [], 5); }
zz() multimethod { return 0; }
zz(a, b?) multimethod { return 1; }
plain(Thing a) { return ({: inherited(a) }); }
obj: object m(Thing) { return 1; } ;
_main(args) { return [ab(1), ba(2), zz(), ab, plain(1)]; }
"""

mistakes["e_mm1"] = """class Thing: object;
f(a) { }
f(Thing a) { }
g(Thing a) { }
g(a) { }
h(Thing a) { }
h(Thing a) { }
k(Nope a) { }
enum red;
m(red a, f b) { }
n(a) multimethod multimethod { }
o: object p(a) multimethod { } q(Thing a) { } ;
r(Thing a, b: , c?) { return inherited<>(a) + inherited<Thing b>(a) + inherited<Thing, >(a) + inherited<Thing>; }
s(a) { return {x: 1 multimethod} ; }
t(a multimethod) { }
_main(args) { }
"""

mistakes["e_mm2"] = """class Thing: object;
a(Thing x) { return inherited(x); }
b(Thing x) { return inherited<Thing, Thing>(x); }
c(x) { return inherited<Thing>(x); }
d(Nope x) { return 1; }
_main(args) { return [a(1), b(2), c(3), d(4)]; }
"""

mistakes["e_mm3"] = """class Thing: object;
d(Nope x) { return 1; }
_main(args) { return d(4); }
"""

mistakes["e_mm4"] = """class Thing: object;
d(Thing x) { return 1; }
_main(args) { return d; }
"""

mistakes["e_mm5"] = """class Thing: object;
_multiMethodCall(a, b) { }
_multiMethodBuildBindings() { }
d(Thing x) { return 1; }
_main(args) { return d(1); }
"""

mistakes["e_mm6"] = """class Thing: object;
_multiMethodCall(a, b) { }
_multiMethodRegister(a, b, c) { }
d(Thing x) { return 1; }
_main(args) { return d(1); }
"""

mistakes["e_mm7"] = """class Thing: object;
extern function e1(Thing a);
extern function e2(a) multimethod;
extern function e3(Thing a, ...);
e1(Thing a) { return 1; }
e2(a) multimethod { return 2; }
_main(args) { return [e1(1), e2(2), e3(3)]; }
"""


# Programs of several source files, put together in the order of the
# files' names: what one file defines and another uses, what two files
# both define, and what each adds to things they share.
linked = {}

linked["l_anon"] = {
    "a": """#include <tads.h>
class Room: object desc = 'room';
object template 'name';
Room 'hall';
Room 'den' north = hall2 inner: Room { 'nested' } ;
hall2: Room 'hall2';
_main(args) { return object: Room { name = 'inline' m() { return hall2; } }; }
""",
    "b": """#include <tads.h>
object template 'name';
Room 'cellar';
+ property location;
shed: Room 'shed';
+ Room 'inside shed';
++ Room 'deeper' ;
f() { return [new Room(), object: Room { desc = 'inline2' }, shed]; }
""",
}

linked["l_dict"] = {
    "a": """#include <tads.h>
dictionary gDict;
dictionary property noun, adjective;
class Thing: object noun = 'thing' 'item';
lamp: Thing noun = 'lamp' adjective = 'brass';
_main(args) { return [lamp.noun, key.noun, key.adjective]; }
""",
    "b": """#include <tads.h>
dictionary gDict;
dictionary property noun, adjective, plural;
key: Thing noun = 'key' adjective = 'brass' 'small' plural = 'keys';
class Odd: Thing noun = 'oddity';
strange: Odd, Thing adjective = 'strange';
""",
}

linked["l_dupfunc"] = {
    "a": """#include <tads.h>
f(x) { return 1; }
_main(args) { return f(1) + g(2); }
""",
    "b": """#include <tads.h>
f(x) { return 2; }
g(x) { return 3; }
""",
}

linked["l_dupobj"] = {
    "a": """#include <tads.h>
o: object x = 1;
class C: object;
_main(args) { return o.x; }
""",
    "b": """#include <tads.h>
o: object x = 2;
class C: object;
""",
}

linked["l_enum"] = {
    "a": """#include <tads.h>
enum red, green, blue;
enum token tokWord;
_main(args) { return [red, blue, tokPunct, yellow]; }
""",
    "b": """#include <tads.h>
enum blue, yellow, red;
enum token tokPunct, tokWord;
f(x) { switch (x) { case red: return 1; case yellow: return 2; case tokWord: return 3; } return [green, blue]; }
""",
}

linked["l_errgen"] = {
    "a": """#include <tads.h>
f(x) { return y; }
_main(args) { return f(1) + g(2); }
""",
    "b": """#include <tads.h>
g(x) { return 'a' - 1; }
""",
}

linked["l_errparse"] = {
    "a": """#include <tads.h>
f(x) { return x; }
_main(args) { return f(1) + g(2); }
""",
    "b": """#include <tads.h>
g(x) { return x +; }
h(x) { return undefinedThing(x); }
""",
}

linked["l_errsym"] = {
    "a": """#include <tads.h>
f(x) { return x; }
_main(args) { return f(1) + g(2); }
""",
    "b": """#include <tads.h>
g(x) { return x; }
g(x) { return x; }
o: object;
o: object;
""",
}

linked["l_export"] = {
    "a": """#include <tads.h>
export f 'entry';
export o;
f(x) { return 1; }
o: object;
_main(args) { }
""",
    "b": """#include <tads.h>
export g 'other';
export f 'entry2';
export p 'entry';
export newProp;
g(x) { return o; }
p: object;
""",
}

linked["l_gram"] = {
    "a": """#include <tads.h>
dictionary gDict;
dictionary property noun;
enum token tokWord;
grammar np: 'the' noun->n_ : object;
grammar np(short): noun->n_ : object;
grammar cmd: 'take' np->obj_ | 'get' np->obj_ : object;
_main(args) { return cmd; }
""",
    "b": """#include <tads.h>
dictionary gDict;
dictionary property noun, adjective;
enum token tokWord;
grammar np(adj): adjective->a_ np->n_ : object;
grammar cmd(drop): 'drop' np->obj_ : object;
grammar vp: tokWord->w_ * : object;
grammar np: 'that' [badness 10] : object;
rock: object noun = 'rock' adjective = 'grey';
""",
}

linked["l_icmod"] = {
    "a": """#include <tads.h>
modify List first() { return self[1]; };
modify Object describe() { return 'obj'; };
_main(args) { return [[1].first(), [2].second(), args.describe()]; }
""",
    "b": """#include <tads.h>
modify List second() { return self[2]; } first2() { return inherited(); };
modify List third = 3;
modify String shout() { return toUpper(); };
""",
    "c": """#include <tads.h>
modify Object describe2() { return 'obj2'; };
modify List fourth = 4;
""",
}

linked["l_kind"] = {
    "a": """#include <tads.h>
thing(x) { return 1; }
other: object x = 1;
_main(args) { return thing(1) + other.x + red; }
enum red;
""",
    "b": """#include <tads.h>
thing: object y = 2;
other(x) { return 2; }
f() { return thing.y + other(1) + self.red; }
""",
}

linked["l_mm"] = {
    "a": """#include <tads.h>
class Thing: object;
class Box: Thing;
put(Thing a, Box b) { return 1; }
put(Box a, b) { return inherited(a, b); }
_main(args) { return [put(Thing, Box), show(Thing), show];  }
_multiMethodRegister(base, func, params) { }
_multiMethodBuildBindings() { }
""",
    "b": """#include <tads.h>
put(Thing a, Thing b) { return 3; }
show(Box a) { return inherited<Thing>(a); }
show(Thing a) { return put(a, a); }
show(a) multimethod { return 0; }
_multiMethodCall(base, args) { return nil; }
_multiMethodCallInherited(from, [args]) { return nil; }
""",
}

linked["l_nomain"] = {
    "a": """#include <tads.h>
f(x) { return g(x); }
""",
    "b": """#include <tads.h>
g(x) { return 1; }
""",
}

linked["l_props"] = {
    "a": """#include <tads.h>
a1: object alpha = 1 beta = 2 m() { return self.gamma + self.delta; };
_main(args) { return [a1.alpha, &beta, &epsilon, a1.zeta]; }
""",
    "b": """#include <tads.h>
b1: object gamma = 1 beta = 2 eta = 3 m() { return self.alpha + a1.theta; };
f() { return [&epsilon, &iota]; }
""",
    "c": """#include <tads.h>
c1: object iota = 1 kappa = 2;
g() { return b1.eta + a1.alpha; }
""",
}

linked["l_static"] = {
    "a": """#include <tads.h>
a1: object x = static (3.5 + 1) y = 1.25 pat = R'a+b' big = 99999999999;
_main(args) { return [a1.x, b1.x, 2.5, R'x*']; }
""",
    "b": """#include <tads.h>
b1: object x = static [a1.y, 7.5] y = 2.25 pat = R'a+b';
f() { return [1.25, 99999999999, R'a+b']; }
""",
}

linked["l_strings"] = {
    "a": """#include <tads.h>
f() { return ['same', 'only a', 'same', 'a string that is long enough not to be shared with others', "dq <<g()>> end"]; }
_main(args) { local same = 1, onlyA = 2; return f() + g() + same + onlyA; }
""",
    "b": """#include <tads.h>
g() { local same = 1, onlyB = 2; return ['same', 'only b', 'a string that is long enough not to be shared with others', same, onlyB]; }
h(a, b:, c:?) { return a + b + (c ?? 0); }
k() { return h(1, b: 2) + h(1, c: 3, b: 4); }
""",
}

linked["l_unres"] = {
    "a": """#include <tads.h>
extern function nowhere(x);
extern object nothing;
extern class Missing;
_main(args) { return nowhere(1) + g(2) + nothing.x; }
""",
    "b": """#include <tads.h>
extern function elsewhere(x, y);
g(x) { return elsewhere(x, x) + new Missing(); }
""",
}


# A second definition in one file of what another file defines:
# `modify` and `replace` of objects, functions, and grammar rules.
linked["l_moderr2"] = {
    "a": """#include <tads.h>
dictionary gDict;
enum red;
f(a) { return 1; }
o: object;
_main(args) { return f(1); }
""",
    "b": """#include <tads.h>
modify gDict x = 1;
replace f(a, b) { return 2; }
modify o x = 1;
replace o: object y = 2;
modify o z = 3;
""",
}

linked["l_moderr3"] = {
    "a": """#include <tads.h>
modify later x = 1;
_main(args) { return later.x; }
""",
    "b": """#include <tads.h>
later: object y = 2;
""",
}

linked["l_moderr4"] = {
    "a": """#include <tads.h>
replace gone: object x = 1;
_main(args) { return gone.x; }
""",
    "b": """#include <tads.h>
gone: object y = 2;
""",
}

linked["l_modfunc"] = {
    "a": """#include <tads.h>
f(a) { return 1; }
g(a) { return 10; }
k(a) { return 100; }
modify k(a) { return replaced(a) + 1; }
_main(args) { tadsSay(toString(f(1) + g(2) + k(3)) + '\\n'); }
""",
    "b": """#include <tads.h>
replace f(a) { return 2; }
modify g(a) { return replaced(a) + 1; }
modify k(a) { local r = replaced; return r(a) + 1000; }
""",
    "c": """#include <tads.h>
modify f(a) { return replaced(a) + 5; }
modify g(a) { return ({: replaced(a) * 2 })(); }
replace k(a) { return 7; }
""",
}

linked["l_modgram"] = {
    "a": """#include <tads.h>
dictionary gDict;
dictionary property noun;
grammar np(basic): 'the' noun->n_ : object x = 1;
grammar np(other): 'a' noun->n_ : object x = 2;
grammar vp(go): 'go' np->n_ : object;
_main(args) { return np; }
""",
    "b": """#include <tads.h>
dictionary gDict;
dictionary property noun;
modify grammar np(basic): 'that' noun->n_ : y = 2;
replace grammar np(other): 'some' noun->n_ : object z = 3;
modify grammar vp(go): : w = 4;
""",
}

linked["l_modmix"] = {
    "a": """#include <tads.h>
class Thing: object name = 'thing' describe() { return name; };
rock: Thing name = 'rock';
modify Thing describe() { return '[' + inherited() + ']'; };
_main(args) { tadsSay(rock.describe() + ' ' + toString(rock.weight) + ' ' + [1,2].count2() + '\\n'); }
""",
    "b": """#include <tads.h>
modify Thing weight = 5 describe() { return '<' + inherited() + '>'; };
modify rock weight = 9;
modify List count2() { return toString(length()); };
""",
    "c": """#include <tads.h>
modify Thing replace weight = 6;
modify rock describe() { return inherited() + '!'; };
modify List count2() { return inherited() + '?'; };
""",
}

linked["l_modobj"] = {
    "a": """#include <tads.h>
dictionary gDict;
dictionary property noun;
class Base: object b = 1;
o: Base x = 1 y = 2 noun = 'old' m() { return 1; };
_main(args) { return [o.x, h()]; }
""",
    "b": """#include <tads.h>
dictionary gDict;
dictionary property noun;
modify o replace x = 5 z = 3 noun = 'new' m() { return inherited() + 1; };
h() { return o.z; }
""",
    "c": """#include <tads.h>
dictionary gDict;
dictionary property noun;
modify o replace y = 7 replace m() { return 9; } w = 1 noun = 'newer';
modify o v = 2;
modify Base c = 2;
""",
}

linked["l_modstatic"] = {
    "a": """#include <tads.h>
o: object x = static [1, 2] y = static (x + [3]) m() { return 1; };
p: object x = static [5];
_main(args) { return o.x; }
""",
    "b": """#include <tads.h>
modify o z = static (y + [4]) replace x = static [9];
replace p: object w = static [6];
""",
}

linked["l_repobj"] = {
    "a": """#include <tads.h>
dictionary gDict;
dictionary property noun;
class Base: object b = 1;
o: Base x = 1 y = 2 noun = 'old' m() { return 1; };
p: Base noun = 'pold';
modify p noun = 'pmod' q = 1;
_main(args) { return [o.x]; }
""",
    "b": """#include <tads.h>
dictionary gDict;
dictionary property noun;
replace o: object z = 3 noun = 'new';
replace p: Base noun = 'pnew';
replace class Base: object b = 2 c = 3;
h() { return o.z; }
""",
}


# A call in a parameter's default value to a function that returns nothing.
programs["defval2"] = (
    BUILTINS
    + """
f(a) { a = 1; }
g(a, b = (a ? f(a) : 2)) { a = 1; }
_main(args) { }
"""
)

programs["defval3"] = (
    BUILTINS
    + """
f(a, b = (a ? f(a) : 2)) { a = 1; }
_main(args) { }
"""
)

# Multi-methods and static properties both: which runs first.
programs["mm10"] = (
    BUILTINS
    + """
class Thing: object;
o: object x = static [1, 2] y = static (x + [3]);
put(Thing a) { return 1; }
put(a) multimethod { return 2; }
_multiMethodRegister(base, func, params) { }
_multiMethodBuildBindings() { }
_multiMethodCall(base, args) { return nil; }
_multiMethodCallInherited(from, [args]) { return nil; }
_main(args) { return [o.x, put(1)]; }
"""
)

# A template for a class that the file does not otherwise use.
linked["l_tplref"] = {
    "a": """#include <tads.h>
object template 'name';
Cls0 template 'name' @loc;
Other template "desc";
first: object 'first';
_main(args) { return first; }
""",
    "b": """#include <tads.h>
object template 'name';
Cls0 template 'name' @loc;
class Cls0: object;
class Other: object;
second: Cls0 'second' @first;
""",
}


# A mistake in one file or in several: the reference stops at the first
# step of the build that fails.
linked["l_err2a"] = {
    "a": """#include <tads.h>
f(x) { return x +; }
_main(args) { return f(1); }
""",
    "b": """#include <tads.h>
g(x) { return x -; }
""",
}

linked["l_err2b"] = {
    "a": """#include <tads.h>
f(x) { return undefinedA(x); }
_main(args) { return f(1) + 'a' - 1; }
""",
    "b": """#include <tads.h>
g(x) { return undefinedB(x) + 'b' - 2; }
""",
}

linked["l_err2c"] = {
    "a": """#include <tads.h>
f(x) { local unused; return 1; }
_main(args) { return f(1); }
""",
    "b": """#include <tads.h>
g(x) { local unused2; return x +; }
""",
}

linked["l_err2d"] = {
    "a": """#include <tads.h>
#include "nothere.h"
f(x) { return 1; }
_main(args) { return f(1); }
""",
    "b": """#include <tads.h>
g(x) { return 2; }
""",
}


# The words of grammar rules go in the dictionary only from a file
# that has an object for the dictionary.
linked["l_gramdict1"] = {
    "a": """#include <tads.h>
dictionary gDict;
grammar np: 'a' : object
;
_main(args) { return np; }
""",
    "b": """#include <tads.h>
dictionary gDict;
grammar cmd: 'b' : object
;
""",
}

linked["l_gramdict2"] = {
    "a": """#include <tads.h>
dictionary gDict;
grammar np: 'a' : object
;
named: object;
_main(args) { return np; }
""",
    "b": """#include <tads.h>
dictionary gDict;
grammar cmd: 'b' : object
;
object x = 1;
""",
}

linked["l_gramdict3"] = {
    "a": """#include <tads.h>
dictionary gDict;
grammar np: 'a' : object
;
class C: object;
_main(args) { return np; }
""",
    "b": """#include <tads.h>
grammar cmd: 'b' : object
;
dictionary gDict;
grammar cmd: 'c' : object
;
later: object;
""",
}


# A grammar rule's words in a program with no object for the dictionary.
programs["gramdict1"] = """dictionary gDict;
grammar np: 'a' : object
;
_main(args) { return np; }
"""


def main():
    if len(sys.argv) != 2:
        sys.exit(__doc__)
    folder = sys.argv[1]
    os.makedirs(folder, exist_ok=True)
    for name, text in programs.items():
        if "_main" not in text:
            text += "_main(args) { return 1; }\n"
        with open(
            os.path.join(folder, name + ".t"), "w", newline="\n", encoding="utf-8"
        ) as out:
            out.write(text.lstrip("\n"))
    for name, text in mistakes.items():
        with open(
            os.path.join(folder, name + ".t"), "w", newline="\n", encoding="utf-8"
        ) as out:
            out.write(text + "\n")
    for name, files in linked.items():
        os.makedirs(os.path.join(folder, name), exist_ok=True)
        for file, text in files.items():
            with open(
                os.path.join(folder, name, file + ".t"), "w", newline="\n", encoding="utf-8"
            ) as out:
                out.write(text)
    print(f"{len(programs) + len(mistakes) + len(linked)} programs in {folder}")


main()
