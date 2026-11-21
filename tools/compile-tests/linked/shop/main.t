// A program to run, in several source files: this one has where it
// starts, and uses what the others define.
#include <systype.h>
#include <tadsgen.h>
#include <tadsio.h>
#include <t3.h>
#include <dict.h>

// Results go one to a line.
line(val) { tadsSay(toString(val)); tadsSay('\n'); }

// Kinds of goods. The other files use these too: a name one file
// defines is known in all of them.
enum fresh, dried, tinned;

_main(args)
{
    // Objects and classes from another file.
    line(apple.name + ': ' + toString(apple.price));
    line(total([apple, rice, beans]));
    line(rice.kind == dried);
    line(beans.label());

    // A function from another file, that calls back into this one.
    line(describeAll());

    // What every file adds to a built-in class is there together.
    line([3, 1, 2].smallest() + [3, 1, 2].largest());

    // The words all the files gave their objects are in one dictionary.
    line(shopWords.findWord('apple').length() != 0);
    line(shopWords.findWord('tin').length() != 0);
    line(shopWords.findWord('pear').length() != 0);

    // Each file's numbers that are not integers are objects of its own.
    line(apple.weight + rice.weight);

    // The last file changed some of this: see sale.t.
    line(shopWords.findWord('pippin').length() != 0);
}

// Called from another file.
kindName(kind)
{
    switch (kind)
    {
    case fresh: return 'fresh';
    case dried: return 'dried';
    default: return 'tinned';
    }
}

modify List
    smallest()
    {
        local best = self[1];
        foreach (local x in self)
            if (x < best) best = x;
        return best;
    }
;
