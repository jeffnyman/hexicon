// The goods: a class and its objects.
#include <systype.h>
#include <tadsgen.h>
#include <t3.h>

dictionary shopWords;
dictionary property noun;

class Goods: object
    name = 'goods'
    price = 0
    kind = fresh
    weight = 0.5
    label() { return name + ' (' + kindName(kind) + ')'; }
;

apple: Goods name = 'apple' price = 3 noun = 'apple' 'fruit';
rice: Goods name = 'rice' price = 5 kind = dried weight = 1.25 noun = 'rice';
