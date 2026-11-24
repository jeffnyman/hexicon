// Conditions: if and else, comparisons, && and ||, ?: and ??.
foo(x, y) { if (x > y) return x; else return y; }
bar(x, y)
{
    if (x == y) return 1;
    if (x != y) return 2;
    if (x < y) return 3;
    if (x <= y) return 4;
    if (x >= y) return 5;
    if (!(x > y)) return 6;
    if (!x) return 7;
    if (x == nil) return 8;
    if (x != nil) return 9;
    if (x) ; else return 10;
    if (x) ;
    if (x && y) return 11;
    if (x || y) return 12;
    if (x && y || !x && !y) return 13;
    if ((x || y) && (x > 1 || y > 1)) return 14; else return 15;
}
baz(x, y)
{
    local a = x && y;
    local b = x || y;
    local c = x > 1 && y > 1;
    local d = x > 1 || y > 1;
    local e = x ? y : 3;
    local f = x ?? y;
    local g = (x > y ? x : y) + 1;
    local h = !(x && y);
    local i = x == y, j = x != y, k = x < y;
    x && y;
    x || y;
    x ? y : 3;
    x ?? y;
    x ? foo(1, 2) : bar(3, 4);
    return a ? b : c ? d : e ? f : g ? h : i ? j : k;
}
consts(x)
{
    if (true) x = 1;
    if (nil) x = 2; else x = 3;
    if (1 > 2) x = 4;
    if (1 < 2 && 2 < 3) x = 5;
    local a = true && x;
    local b = nil || x;
    local c = nil && x;
    local d = true || x;
    local e = true ? x : 2;
    local f = nil ?? x;
    local g = 1 ?? x;
    local h = 'a' < 'b', i = 'abc' == 'abc', j = 'a' != 'b', k = 1 == 'a';
    return a ? b : c ? d : e ? f : g ? h : i ? j : k;
}
_main(args)
{
    return foo(1, 2) + bar(1, 2) + baz(1, 2) + consts(1);
}
