// A program to run: methods that stand for operators, methods borrowed
// from another object with `delegated`, and the count of arguments.
#include <systype.h>
#include <tadsgen.h>
#include <tadsio.h>
#include <t3.h>

say(val) { tadsSay(val); }

// Results go one to a line.
line(val) { tadsSay(val); tadsSay('\n'); }

// A pair of numbers that can be added, scaled and indexed like the
// numbers and lists the language has itself. In each method the object
// is the left operand, and the parameters are the others.
class Pair: object
    x = 0
    y = 0
    construct(x_, y_) { x = x_; y = y_; }

    operator +(other) { return new Pair(x + other.x, y + other.y); }
    operator -(other) { return new Pair(x - other.x, y - other.y); }
    operator *(times) { return new Pair(x * times, y * times); }
    operator negate() { return new Pair(-x, -y); }

    // `pair[1]`, and `pair[1] = value`, which gives the pair to keep.
    operator [](index) { return index == 1 ? x : y; }
    operator []=(index, val)
    {
        return index == 1 ? new Pair(val, y) : new Pair(x, val);
    }

    // An operator's method can do anything at all.
    operator <<(text) { return text + ' sent to ' + show(); }
    operator ~() { return new Pair(y, x); }

    show() { return '(' + x + ', ' + y + ')'; }
;

// Something to borrow methods from.
class Greeter: object
    name = 'the greeter'
    greet() { return 'hello from ' + name; }
    greetAll([people]) { return name + ' greets ' + people.length() + ' people'; }
;

// `delegated` runs another object's method with this object as `self`,
// though this object is no kind of that one.
guest: object
    name = 'the guest'
    helper = Greeter
    greet() { return delegated Greeter.greet(); }

    // With no property named, it is the property being run.
    greetAll([people]) { return delegated Greeter(people...); }

    // The object to borrow from can be worked out when the method runs.
    greetAgain() { return delegated helper.greet(); }
    greetBy(prop) { return delegated Greeter.(prop)(); }
;

// How many arguments a call gave, whatever the parameters are.
count(...) { return argcount; }

_main(args)
{
    local a = new Pair(1, 2);
    local b = new Pair(10, 20);

    line((a + b).show());
    line((b - a).show());
    line((a * 3).show());
    line((-a).show());
    line((a + b * 2 - a).show());
    line(a[1] + a[2]);
    a[2] = 7;
    line(a.show());
    line(a << 'a note');
    line((~a).show());

    // An operator's method is a property like any other, with a name.
    line(a.propDefined(&operator +) ? 'pairs can be added' : 'pairs cannot be added');
    line(a.propDefined(&operator /) ? 'pairs can be divided' : 'pairs cannot be divided');
    line(a.(&operator -)(b).show());

    line(guest.greet());
    line(guest.greetAll('Ann', 'Ben', 'Cy'));
    line(guest.greetAgain());
    line(guest.greetBy(&greet));
    line(count());
    line(count(1, 2, 3));
}
