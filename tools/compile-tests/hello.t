// A program that says something: built-in functions, strings to
// display, and lists.
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

fib(n) { return n < 2 ? n : fib(n - 1) + fib(n - 2); }

sum(lst, count)
{
    local total = 0;
    for (local i = 1 ; i <= count ; ++i)
        total += lst[i];
    return total;
}

_main(args)
{
    t3SetSay(say);
    "Hello from Hexicon!\n";
    local numbers = [];
    for (local i = 1 ; i <= 10 ; ++i)
        numbers += fib(i);
    numbers[3] = 100;
    tadsSay('The sum is ' + toString(sum(numbers, 10)) + '.\n');
    local words = ['one', 'two', 'three'];
    for (local i = 1 ; i <= 3 ; ++i)
    {
        tadsSay(words[i]);
        if (i < 3) ", "; else ".\n";
    }
}
