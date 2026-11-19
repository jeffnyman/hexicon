// Strings in the constant pool.
pick(n)
{
    if (n == 1) return 'one';
    if (n == 2) return 'two';
    if (n == 3) return 'one';
    if (n == 4) return '';
    if (n == 5) return 'a string that is forty characters long!!';
    if (n == 6) return 'a string that is forty characters long!!';
    if (n == 7) return 'thirty-nine characters long, this one!!';
    if (n == 8) return 'thirty-nine characters long, this one!!';
    if (n == 9) return 'con' + 'cat' + 1 + true;
    if (n == 10) return 'tab\there \'quoted\' caf\u00e9';
    return 'none';
}
_main(args)
{
    local description = 'local';
    local otherName = pick(1), description2 = description;
    return pick(2) + otherName + description2;
}
