// A program to run: the classes built into the virtual machine, declared
// by the system's own header files, and their methods at work.
#include <systype.h>
#include <tadsgen.h>
#include <tadsio.h>
#include <t3.h>

say(val) { tadsSay(val); }

class Thing: object
    name = 'thing'
    parts = []
    describe()
    {
        "The <<name>> has <<parts.length()>> part<<if parts.length() != 1>>s<<end>>";
        if (parts.length() != 0)
            ": <<parts.join(', ')>>";
        ". ";
    }
;

box: Thing
    name = 'box'
    parts = ['lid', 'base']
;

pebble: Thing
    name = 'pebble'
;

// The longest of some strings.
longest(strings)
{
    local best = '';
    for (local i = 1 ; i <= strings.length() ; ++i)
    {
        if (strings[i].length() > best.length())
            best = strings[i];
    }
    return best;
}

// The numbers up to a limit that are not in a list, and their sum.
missing(limit, taken)
{
    local sum = 0;
    for (local n in 1..limit)
    {
        local found = nil;
        foreach (local t in taken)
        {
            if (t == n)
                found = true;
        }
        if (!found)
        {
            "<<n>> ";
            sum += n;
        }
    }
    return sum;
}

_main(args)
{
    t3SetSay(say);
    "Missing: <<missing(8, [2, 3, 5, 7])>> in all.\n";
    "Counting down by threes:";
    for (local n in 10..1 step -3)
        " <<n>>";
    ". Each part of the box:";
    for (local part in box.parts)
        " <<part>>";
    ".\n";
    "Three <<3 is in (1, 2, 3) ? 'is' : 'is not'>> among the first three,
    and <<3 not in (4, 5) ? 'is not' : 'is'>> among the next two.\n";
    box.describe();
    pebble.describe();
    "\n";

    local v = new Vector(4);
    v.append('one');
    v.append('two');
    v.append('three');
    "A vector of <<v.length()>>, the longest being '<<longest(v.toList())>>'.\n";
    "<<'Hello'.toUpper()>>, <<'Hello'.length()>> letters, from the second: <<'Hello'.substr(2)>>.\n";
    "Sorted: <<[3, 1, 2].sort().join(' ')>>. Found at <<[5, 6, 7].indexOf(6)>>.\n";
    "The box is <<box.ofKind(Thing) ? 'a thing' : 'not a thing'>>,
    and <<box.propDefined(&parts) ? 'has' : 'lacks'>> parts of its own;
    the pebble <<pebble.propDefined(&parts, PropDefDirectly) ? 'has' : 'lacks'>> them.\n";
    "A list is <<dataType([]) == TypeList ? 'a list' : 'something else'>>,
    and <<[1].ofKind(List) ? 'of' : 'not of'>> the class List.\n";
}
