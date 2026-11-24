// Calls: argument counts, variable arguments, values and no values.
none() { }
one(a) { return a; }
two(a, b) { return a + b; }
many(a, b, c, d, e, f) { return a + b + c + d + e + f; }
anyOf(...) { return 1; }
rest(a, [others]) { return others; }
onlyRest([everything]) { return everything; }
early(x) { if (x) return; x = 2; }
both(x) { if (x) return 1; }
ptr(f, g) { return f(1) + g(1, 2); }
spread(lst, a) { return anyOf(lst...) + anyOf(a, lst..., a, lst...) + rest(a, lst...) + ptr(lst...); }
before() { return after(1); }
after(x) { return x; }
argNames(alpha, beta, gamma, delta, epsilon) { return alpha + beta + gamma + delta + epsilon; }
_main(args)
{
    local total = 0;
    none();
    one(1);
    total = one(2) + two(3, 4);
    total += many(1, 2, 3, 4, 5, 6);
    total += anyOf() + anyOf(1) + anyOf(1, 2, 3);
    rest(1);
    rest(1, 2, 3);
    onlyRest();
    early(1);
    total += both(1);
    total += ptr(one, two);
    local f = one;
    total += f(3);
    f(4);
    total += two(one(1), two(2, one(3)));
    total += true ? none() : one(1);
    total += total ? one(1) : none();
    total ? none() : none();
    total += argNames(1, 2, 3, 4, 5);
    return total + before() + spread(args, 1);
}
