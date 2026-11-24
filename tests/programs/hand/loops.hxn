// Loops, break and continue.
count(n)
{
    local total = 0;
    for (local i = 0 ; i < n ; ++i)
        total += i;
    for (local i = n, local j = 0 ; i > 0 ; --i, j++)
    {
        if (i == 3) continue;
        if (i == 7) break;
        total += j;
    }
    for (;;)
    {
        if (total > 5) break;
        total++;
    }
    local k;
    for (k = 0 ; k < 3 ; k += 1) ;
    for (k = 0 ; ; ) { break; }
    return total + k;
}
whiles(n)
{
    local i = 0;
    while (i < n)
        i++;
    while (i > 0)
    {
        i--;
        if (i == 5) continue;
        if (i == 2) break;
    }
    do
        i++;
    while (i < 10);
    do
    {
        if (i == 15) break;
        if (i == 12) { i += 2; continue; }
        i++;
    } while (i < 20 && n);
    while (true) { if (i++ > 30) break; }
    while (nil) i = 0;
    do { i--; } while (nil);
    return i;
}
nested(n)
{
    local total = 0;
    for (local i = 0 ; i < n ; i++)
    {
        local j = i;
        while (j > 0)
        {
            local k = j;
            total += k;
            j--;
        }
    }
    {
        local i = 2;
        total += i;
    }
    {
        local longerName = 3;
        total += longerName;
    }
    return total;
}
_main(args)
{
    return count(10) + whiles(10) + nested(10);
}
