// Scopes and the order of the names of locals in the tables.
_main(args)
{
    local a, b, c, ab, ba, abc, cab, bca, zzzz, yyyy, i1, i2, ii, hj;
    a = b = c = ab = ba = abc = cab = bca = zzzz = yyyy = i1 = i2 = ii = hj = 1;
    {
        local a = 2, inner = 3;
        a += inner;
        {
            local a = 4, innermost = 5;
            a += innermost;
        }
        { }
        { ; }
        { { local deep = 1; deep++; } }
    }
    for (local a = 0 ; a < 1 ; a++) { local inFor = a; inFor++; }
    for (local a = 0 ; a < 1 ; a++) local single = a;
    if (a) { local inIf = 1; inIf++; } else { local inElse = 2; inElse++; }
    if (a) local bare = 1;
    while (a < 0) { local inWhile = 1; inWhile++; }
    return a + b + c + ab + ba + abc + cab + bca + zzzz + yyyy + i1 + i2 + ii + hj;
}
noLocals() { return 1; }
onlyParams(first, second) { return first; }
unusedLocal() { local never; return 1; }
