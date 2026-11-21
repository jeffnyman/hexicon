// A program to run: a dictionary, and the words objects have in it.
#include <systype.h>
#include <tadsgen.h>
#include <tadsio.h>
#include <t3.h>
#include <dict.h>

say(val) { tadsSay(val); }

// Results go one to a line.
line(val) { tadsSay(toString(val)); tadsSay('\n'); }

// The dictionary the words of the objects that follow go in, and the
// properties whose values are words.
dictionary gDict;
dictionary property noun, adjective;

class Thing: object
    name = 'thing'

    // A class's words are words for each of its objects, not for the
    // class itself.
    noun = 'thing' 'object'
;

class Light: Thing
    adjective = 'bright'
;

// Words are strings one after another, with no brackets or commas.
lamp: Light
    name = 'lamp'
    noun = 'lamp' 'lantern'
    adjective = 'brass' 'old'
;

candle: Light
    name = 'candle'
    noun = 'candle'
    adjective = 'old'
;

key: Thing
    name = 'key'
    noun = 'key'
    adjective = 'brass'
;

// `replace` before a dictionary property of a `modify` takes away the
// words the object had for it. The object as it was is still there
// behind the new one, and still in the dictionary for its other words,
// so the key turns up twice below.
modify key
    replace noun = 'passkey'
;

// The objects a word is a word for, in the order of their names.
objects(word, prop)
{
    // The dictionary gives each object and then the property.
    local found = gDict.findWord(word, prop);
    local names = [];
    for (local i = 1 ; i <= found.length() ; i += 2)
        names += found[i].name;
    return names.sort().join(', ');
}

_main(args)
{
    line(objects('brass', &adjective));
    line(objects('old', &adjective));
    line(objects('bright', &adjective));
    line(objects('thing', &noun));
    line(objects('lantern', &noun));
    line(objects('passkey', &noun));
    line(gDict.isWordDefined('key') ? 'key is a word' : 'key is no word');
    line(gDict.isWordDefined('brass') ? 'brass is a word' : 'brass is no word');

    // A dictionary property's value is the list of the object's words
    // for it, its classes' first.
    line(lamp.noun.join(' '));
    line(lamp.adjective.join(' '));
    line(key.noun.join(' '));
    line(Thing.noun.join(' '));

    // The program can add words as it runs.
    gDict.addWord(candle, 'taper', &noun);
    line(objects('taper', &noun));
}
