// Arithmetic, constants of every size, and constant folding.
_main(args)
{
    local a = 1;
    local b = a + 2;
    local c = 0, d = nil, e = true, f = 127, g = 126, h = -128, i = -129;
    local j = 100000, k = 2 * 3 + 4, l = (7 - 2) * (8 / 2) % 5;
    local m = a - 1, n = a + 1, o = a * 1, p = 1 * a, q = a * 0, r = 0 * a;
    local s = a / 1, t = 0 / a, u = a % 1, v = 0 % a;
    local w = a << 2, x = a >> 1, y = a >>> 1, z = ~a;
    local aa = a & b, bb = a | b, cc = a ^ b, dd = -a, ee = +a, ff = !a;
    local gg = 1 << 4, hh = 256 >> 2, ii = -1 >>> 28, jj = ~0, kk = -(3), ll = !0;
    local mm = 5 & 3, nn = 5 | 3, oo = 5 ^ 3;
    b += 1; b -= 1; b += 5; b -= 5; b += -1; b += 1000; b += c; b -= c;
    b *= 2; b /= 2; b %= 2; b &= 1; b |= 1; b ^= 1; b <<= 1; b >>= 1; b >>>= 1;
    a * 0;
    a % 1;
    return b * 3 + c + f + g + h + i + j + k + l + m + n + o + p + q + r + s
        + t + u + v + w + x + y + z + aa + bb + cc + dd + ee + gg + hh + ii
        + jj + kk + mm + nn + oo;
}
