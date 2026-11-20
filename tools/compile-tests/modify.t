// A program to run: second definitions. `replace` puts a new definition
// in an old one's place; `modify` puts a new one in front of the old,
// which it can still reach.
#include <systype.h>
#include <tadsgen.h>
#include <tadsio.h>
#include <t3.h>

// Results go three to a line, so that the output fits on one screen.
out: object count = 0;
line(val) { tadsSay(val); tadsSay(++out.count % 3 == 0 ? '\n' : ' | '); }

// ---- functions ----

greet(name) { return 'Hello, ' + name; }
price(amount) { return amount * 2; }
label(text, mark = '.') { return text + mark; }

// The old definition is gone: nothing can call it.
replace greet(name) { return 'Good day, ' + name; }

// The old definition is still there, as `replaced`.
modify price(amount) { return replaced(amount) + 1; }

// `replaced` is a value like any function, and means the same in a
// function nested in the new definition.
modify label(text, mark = '!')
{
    local old = replaced;
    local twice = {: replaced(text, mark) + replaced(text) };
    return old(text, '?') + ' ' + twice();
}

// ---- objects ----

class Animal: object
    name = 'animal'
    sound = 'a noise'
    legs = 4
    describe() { return name + ' makes ' + sound; }
;

dog: Animal
    name = 'dog'
    sound = 'a bark'
    describe() { return inherited() + ' loudly'; }
;

cat: Animal name = 'cat' sound = 'a meow';

// What the new definition does not say, the old one still does; and
// `inherited` in the new reaches the old.
modify dog
    sound = 'a woof'
    tricks = 2
    describe() { return inherited() + ', and knows ' + tricks + ' tricks'; }
;

// `replace` before a property takes the old definitions' property
// away, so `inherited` goes past them to the class.
modify dog
    replace describe() { return 'Our ' + inherited(); }
;

// A class can be modified too, after objects have been made from it.
modify Animal
    legs = 'four'
    describe() { return '[' + inherited() + ']'; }
;

// And an object replaced outright keeps nothing of what it was.
replace cat: object name = 'a different cat';

// ---- the classes built in ----

modify List
    last() { return self[length()]; }
    joined(sep)
    {
        local text = '';
        for (local i = 1 ; i <= length() ; ++i)
            text += (i > 1 ? sep : '') + self[i];
        return text;
    }
;

modify String
    twice() { return self + self; }
;

// What is added to a class is there for the classes that are kinds of
// it: every object, of any class, is an Object.
modify Object
    kind = 'thing'
    isAnimal() { return ofKind(Animal); }
;

// A second `modify` reaches the first with `inherited`.
modify List
    last() { return 'last: ' + inherited(); }
;

_main(args)
{
    line(greet('Ann'));
    line(price(10));
    line(label('wait'));

    line(dog.describe());
    line(dog.sound);
    line(dog.legs);
    line(new Animal().describe());
    line(cat.name);
    line(cat.propDefined(&sound) ? 'still an animal' : 'no longer an animal');
    line(dog.ofKind(Animal) ? 'a kind of animal' : 'not an animal');

    local list = ['a', 'b', 'c'];
    line(list.last());
    line(list.joined('-'));
    line('ab'.twice());
    line(dog.kind);
    line(list.kind);
    line(dog.isAnimal() ? 'yes' : 'no');
    line(list.isAnimal() ? 'yes' : 'no');
    line(new Vector(1).kind);
}
