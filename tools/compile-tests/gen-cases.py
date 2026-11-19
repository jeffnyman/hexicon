#!/usr/bin/env python3
"""Write the small programs that each probe one case of the compiler.

usage: gen-cases.py <folder>

The programs are for tools/compile-compare.sh. Most are a line or two:
one way of writing something, or one mistake, to see that Hexicon makes
of it what the reference compiler does, messages included. A program
without a `_main` function is given an empty one.

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
programs["str5000"] = "_main(args) { return '" + "z" * 5000 + "' + '" + "w" * 3000 + "'; }\n"
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
programs["main_last"] = "a() { return 1; }\nb() { return 2; }\n_main(args) { return a() + b(); }\n"
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
    "_main(args)\n{\n    local x = 0;\n" + adds(400)
    + "    return x + f1(1) + f2(2) + f3(3) + last('s');\n}\n"
    + "".join(f"f{k}(x)\n{{\n" + adds(150 * k) + "    return x;\n}\n" for k in (1, 2, 3))
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

programs["bif1"] = BUILTINS + """
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
    "intrinsic 'many/1'\n{\n" + "".join(f"    fn{i}();\n" for i in range(300))
    + "}\n_main(args) { return fn0() + fn255(); }\n"
)

programs["dstr1"] = BUILTINS + """
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
    "_main(args)\n{\n    local big = [" + ", ".join(str(i) for i in range(600))
    + "];\n    local dyn = [args, " + ", ".join(str(i) for i in range(300))
    + "];\n    return big + dyn;\n}\n"
)
programs["list4"] = (
    "_main(args)\n{\n    local dyn = [args, " + ", ".join(str(i) for i in range(900))
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

programs["embed4"] = SPRINTF + """
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
    "e_dstr1": "_main(args) { local a = \"text\"; return a; }",
    "e_dstr2": "_main(args) { return \"text\"; }",
    "e_dstr3": "f(x) { } _main(args) { f(\"text\"); }",
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
    "e_emb1": "_main(args) { \"a<<args b>>c\"; }",
    "e_emb2": "_main(args) { \"a<<1 +>>c\"; }",
    "e_emb3": "_main(args) { \"a<<(1>>b\"; }",
    "e_emb4": "_main(args) { \"a<<1;>>b\"; local s = 'a<<args args>>b'; }",
    "e_emb5": "_main(args) { local x = \"a<<args>>b\"; x = \"a<<args>>\"; }",
    "e_emb6": "_main(args) { local y = (\"<<args>>b\", 1); local z = args ? \"a<<1>>\" : \"b\"; }",
    "e_emb7": "_main(args) { \"<<args>>\" + 1; \"<<args>>\"(1); }",
    "e_emb8": "_main(args) { \"a<<else>>b\"; }",
    "e_emb9": "_main(args) { \"<<else if args>>x\"; \"a<<else unless 1>>b\"; }",
    "e_emb10": "_main(args) { \"<<if>>a<<end>>\"; \"<<unless>>b\"; }",
    "e_emb11": "_main(args) { \"<<if args>>a<<else>>b<<else>>c<<end>>\"; }",
    "e_emb12": "_main(args) { \"<<if args>>a<<or>>b<<end>>c<<only>>d<<stopping>>e<<at random>>f\"; }",
    "e_emb13": "_main(args) { \"<<one of>>a<<or>>b<<at random>>\"; }",
    "e_emb14": "_main(args) { \"<<first time>>a<<only>>\"; local s = 'x<<one of>>a<<or>>b<<cycling>>'; }",
    "e_emb15": "_main(args) { \"<<one of>>a<<or>>b<<or>><<if args>>c<<end>><<stopping>>\"; }",
    "e_emb16": "_main(args) { \"<<one of>>a<<||>>b<<then purely at random>>c<<one of>>d<<first time>>e\"; }",
    "e_emb17": "_main(args) { \"<<one of>>a<<if args>>b<<or>>c<<end>>d<<as decreasingly likely outcomes>>\"; }",
    "e_emb18": "_main(args) { \"<<%d>>\"; }",
    "e_emb19": "_main(args) { \"<<%>>\"; }",
    "e_emb20": "_main(args) { \"<<%d args>>\"; }",
    "e_emb21": "_main(args) { \"a<<args>>b\" ; \"a<<args>>b",
    "e_emb22": "_main(args) { \"a<<args; \"b\"; }",
    "e_emb23": "_main(args) { \"a<<args}\nf() { \"x<<1 2>>y<<3>>z<<4 5\"; }",
    "e_emb24": "_main(args) { \"<<\nargs +\n1 +\n>>\n\"; \"<<args>>\n<<args args>>\nz\"; }",
    "e_emb25": "_main(args) { \"<<if args>>a<<end\"; }",
    "e_emb26": "_main(args) { 'a<<if args>>b<<else'; 'c<<end>>'; }",
    "e_emb27": "_main(args) { \"<<nosuch>><<end>><<or>>\"; }",
    "e_emb28": "_main(args) { local s = '<<\"a\">>'; return s; }",
    "e_emb29": "_main(args) { \"<<if args>>x<<else>>y<<end>><<end>>\"; return 'a<<}>>'; }",
    "e_emb30": "_main(args) { \"<<[>>\"; \"<<)>>\"; \"<<(>>\"; \"<<]>>\"; }",
    "e_tpl1": "string template <<* in words>> spell;\n_main(args) { \"<<args in words>>\"; }",
    "e_tpl2": "string template <<the * of *>> bad;\n_main(args) { }",
    "e_tpl3": "nope() { }\nstring template <<nope2>> nope;\n_main(args) { }",
    "e_tpl4": "string template <<* in words>> spell;\nspell(x, y) { return x; }\n_main(args) { }",
    "e_tpl5": "string template nope3 nope;\n_main(args) { }",
    "e_tpl6": "string template <<nope3 nope;\n_main(args) { }",
    "e_tpl7": "string template <<a b>> 3;\n_main(args) { }",
    "e_tpl8": "string template <<q>> _main;\n_main(args) { }",
    "e_tpl9": "string template <<a>> fa;\n_main(args) { fa(1); undefinedthing(); }",
    "e_tpl10": "string template <<p>> ab;\nstring template <<q>> ba;\nstring template <<r>> zz;\nstring template <<s>> aa;\nstring template <<t *>> cab;\nstring template <<u>> bca;\n_main(args) { \"<<s>><<r>><<q>><<p>><<u>><<t 1>>\"; }",
    "e_tpl11": "f(x) { return x; }\nstring template <<a *>> f;\nstring template <<b>> f;\nstring template <<c *>> g;\ng() { return 1; }\nstring template <<d *>> h;\nh(...) { }\nstring template <<e *>> k;\nk(a, b) { }\n_main(args) { }",
    "e_tpl12": "string template <<a>> fa;\n_main(args) { return fa; }",
    "e_tpl13": "string template <<a>> fa;\n_main(args) { return [fa, 1]; }",
    "e_tpl15": "string template;\nstring template <<a>>;\n_main(args) { }",
    "e_tpl16": "string template <<a>> fa\n_main(args) { }",
    "e_undef1": "_main(args) { nosuch; nosuch; nosuch(); nosuch = 1; other; return nosuch + other; }",
    "e_undef2": "f() { nosuch(); } g() { \"<<nosuch>>\"; }\n_main(args) { nosuch; try { } catch (nosuch e) { } }",
    "e_undef3": "_main(args) { try { } catch (nosuch e) { } try { } catch (nosuch e) { } nosuch = 1; }",
}

# The compiler gives up after a hundred errors, whether it finds them
# reading the source, working out constants, or generating code.
mistakes["e_many1"] = "f() { local q; }\n_main(args) {\n" + "".join(
    f"  u{n}();\n" for n in range(120)) + "}\ng() { local r; }\n\n"
mistakes["e_many2"] = "f() { local q; }\n_main(args) {\n" + "".join(
    f"  u{n}();\n" for n in range(100)) + "}\ng() { local r; }"
mistakes["e_many6"] = "_main(args) {\n" + "  u();\n" * 150 + "}"
mistakes["e_many3"] = "_main(args) {\n" + "  1 2;\n" * 150 + "}"
mistakes["e_many4"] = "_main(args) {\n" + "  [1][2];\n" * 101 + "}"
mistakes["e_many5"] = "_main(args) {\n" + "  1 2;\n" * 100 + "}"


def main():
    if len(sys.argv) != 2:
        sys.exit(__doc__)
    folder = sys.argv[1]
    os.makedirs(folder, exist_ok=True)
    for name, text in programs.items():
        if "_main" not in text:
            text += "_main(args) { return 1; }\n"
        with open(os.path.join(folder, name + ".t"), "w", newline="\n", encoding="utf-8") as out:
            out.write(text.lstrip("\n"))
    for name, text in mistakes.items():
        with open(os.path.join(folder, name + ".t"), "w", newline="\n", encoding="utf-8") as out:
            out.write(text + "\n")
    print(f"{len(programs) + len(mistakes)} programs in {folder}")


main()
