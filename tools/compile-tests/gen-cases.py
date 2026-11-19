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

# Mistakes, and code that draws a warning. Each is a program of its own,
# since what a compiler does after an error depends on how it recovers.
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
    "w_argnoval": "f() { } g(a) { } _main(args) { g(f()); }",
}


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
