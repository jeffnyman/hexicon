// Labels, goto, and break and continue that name a label.
f(x)
{
    local n = 0;
top:
    ++n;
    if (n < x)
        goto top;
    goto done;
    n = 1000;
done:
    return n;
}

g(x, y)
{
    local total = 0;
outer:
    for (local i = 0 ; i < x ; ++i)
    {
    inner:
        for (local j = 0 ; j < y ; ++j)
        {
            if (j == 2)
                continue outer;
            if (j == 3)
                continue inner;
            if (i == 4)
                break outer;
            if (i == 5)
                break inner;
            ++total;
        }
    }
    return total;
}

h(x)
{
    local n = 0;
block:
    {
        ++n;
        if (x)
            break block;
        ++n;
    }
loop:
    while (n < 10)
    {
        do
        {
            ++n;
            if (n == 5)
                continue loop;
            if (n == 7)
                break loop;
        } while (n < 3);
    }
both: again:
    do
    {
        ++n;
        if (n < 20)
            continue both;
        if (n < 30)
            continue again;
    } while (n < 40);
    return n;
}

// A switch with labels in it, and a jump into a block.
s(x)
{
    local r = 0;
    switch (x)
    {
    case 1:
        goto two;
    case 2:
    two:
        r = 2;
        break;
    default:
        r = 3;
    }
    if (r == 2)
        goto inside;
    while (r < 10)
    {
        ++r;
    inside:
        r += 2;
    }
    return r;
}

unused(x)
{
never:
    x = 1;
also_never:
    return x;
}

_main(args)
{
    return f(1) + g(2, 3) + h(4) + s(5) + unused(6);
}
