// A program to run: switch, labels, goto, and finally blocks, saying
// what they do as they do it.
intrinsic 't3vm/010006'
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
}
intrinsic 'tads-io/030007'
{
    tadsSay(val, ...);
}

say(val) { tadsSay(val); }

name(x)
{
    switch (x)
    {
    case 1:
        return 'one';
    case 2:
    case 3:
        return 'two or three';
    case 'four':
        return 'the word four';
    case nil:
        return 'nil';
    case [5, 6]:
        return 'a list';
    default:
        return 'something else';
    }
}

count(limit)
{
    local n = 0;
again:
    ++n;
    if (n < limit)
        goto again;
    return n;
}

pairs(rows, columns)
{
    local found = 0;
rows:
    for (local i = 1 ; i <= rows ; ++i)
    {
        for (local j = 1 ; j <= columns ; ++j)
        {
            if (j > i)
                continue rows;
            if (i + j > 7)
                break rows;
            ++found;
        }
    }
    return found;
}

guarded(x)
{
    try
    {
        "in ";
        if (x == 1)
            return 'early';
        "through ";
    }
    finally
    {
        "out ";
    }
    return 'late';
}

loops()
{
    local n = 0;
    for (local i = 0 ; i < 5 ; ++i)
    {
        try
        {
            if (i == 1)
                continue;
            if (i == 3)
                break;
            n += 10;
        }
        finally
        {
            ++n;
        }
    }
    return n;
}

_main(args)
{
    t3SetSay(say);
    tadsSay(name(1) + ', ' + name(3) + ', ' + name('four') + ', ' + name(nil) + ', '
            + name([5, 6]) + ', ' + name(7) + '.\n');
    tadsSay('Counted to ' + toString(count(12)) + '.\n');
    tadsSay('Pairs: ' + toString(pairs(6, 6)) + '.\n');
    tadsSay(guarded(1));
    "\n";
    tadsSay(guarded(2));
    "\n";
    tadsSay('Loops: ' + toString(loops()) + '.\n');
}
