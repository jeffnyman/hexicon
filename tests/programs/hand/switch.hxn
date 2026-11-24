// The switch statement: tables of cases, falling through, and leaving.
f(x)
{
    local r = 0;
    switch (x)
    {
    case 1:
        r = 10;
        break;
    case 2:
    case 3:
        r = 20;
    case 'four':
        r += 1;
        break;
    case nil:
        return 'nil';
    case true:
        return 'true';
    case [1, 2]:
        r = 50;
        break;
    case f:
        r = 60;
        break;
    case 1 + 6:
        r = 70;
    default:
        r += 100;
    }
    return r;
}

// No default: what follows can be reached.
g(x)
{
    switch (x)
    {
    case 1:
        return 'one';
    case 2:
        return 'two';
    }
    return 'other';
}

// Every way out returns: nothing follows the switch.
h(x)
{
    switch (x)
    {
    case 1:
        return 'one';
    default:
        return 'other';
    }
}

// Nothing in it at all, and only a default.
e(x)
{
    switch (x) { }
    switch (x) { default: ; }
    switch (x) { default: x = 1; }
    switch (x) { case 1: ; }
    switch (x + 1) { case 1: default: case 2: x = 3; }
    return x;
}

// In loops, and inside one another.
n(x, y)
{
    local total = 0;
    for (local i = 0 ; i < x ; ++i)
    {
        switch (i)
        {
        case 0:
            continue;
        case 1:
            switch (y)
            {
            case 1:
                total += 1;
                break;
            default:
                total += 2;
            }
            total += 10;
            break;
        case 2:
            {
                local inner = i * 2;
                total += inner;
            }
            break;
        default:
            if (i > 5)
                break;
            total += 100;
        }
        if (total > 1000)
            break;
    }
    while (x > 0)
    {
        switch (x--)
        {
        case 3:
            return total;
        }
    }
    return total;
}

_main(args)
{
    return f(1) + g(2) + h(3) + e(4) + n(5, 6);
}
