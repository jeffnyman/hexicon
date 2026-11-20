// A program to run: objects, classes, properties, and methods, saying
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

class Thing: object
    name = 'thing'
    weight = 1
    describe() { "The <<name>> weighs <<weight>>. "; }
    heavier(other) { return weight > other.weight ? self : other; }
;

class Container: Thing
    contents = []
    count = 0
    describe()
    {
        "The <<name>> weighs <<total()>> in all, and holds <<count>> thing<<if count != 1>>s<<end>>. ";
    }
    total()
    {
        local sum = weight;
        for (local i = 1 ; i <= count ; ++i)
            sum += contents[i].weight;
        return sum;
    }
    add(item) { contents += item; ++count; return self; }
;

class Lit: object
    lit = true
    light() { return lit ? 'lit' : 'dark'; }
;

coin: Thing
    name = 'coin'
;

lamp: Thing, Lit
    name = 'lamp'
    weight = 5
    describe() { "The <<name>> is <<light()>>. "; }
;

box: Container
    name = 'box'
    weight = 3
;

// An object with no name of its own.
Thing
    name = 'pebble'
    weight = 2
    register = (box.add(self))
;

class Problem: object
    text = 'a problem'
;

class Worse: Problem
    text = 'something worse'
;

risky(n)
{
    if (n == 1)
        throw Problem;
    if (n == 2)
        throw Worse;
    return 'nothing wrong';
}

attempt(n)
{
    try
    {
        return risky(n);
    }
    catch (Worse w)
    {
        return 'caught: ' + w.text;
    }
    catch (Problem p)
    {
        return 'caught: ' + p.text;
    }
}

_main(args)
{
    t3SetSay(say);
    coin.describe();
    lamp.describe();
    lamp.lit = nil;
    lamp.describe();
    "\n";
    box.add(coin).add(lamp);
    box.describe();
    "\n";
    local prop = coin.heavier(lamp);
    "The heavier is the <<prop.name>>. ";
    coin.weight += 10;
    "Now it is the <<coin.heavier(lamp).name>>.\n";
    tadsSay(attempt(0) + '; ' + attempt(1) + '; ' + attempt(2) + '.\n');
}
