#!/usr/bin/env python3
"""Compare two program images.

usage: image-diff.py <first> <second> <unspecified>

The third file lists the parts of the first image that mean nothing,
each as an offset and a length on a line: `hexicon build --unspecified`
writes it. They are the time stamp, the bytes in the header that are the
compiler's own, and fields that are written but never read. Those parts
are not compared, in either image.

Prints nothing and exits with 0 if the images are otherwise the same;
if not, says which block the first difference in each differing block
is in, and exits with 1.
"""

import struct
import sys

SIGNATURE = b"HX-image\r\n\x1a"
HEADER_LEN = len(SIGNATURE) + 2 + 28 + 4 + 24


def blocks(data, path):
    """The image's blocks as (name, offset, length), header included."""
    if not data.startswith(SIGNATURE):
        sys.exit(f"{path}: not a program image")
    found = [("header", 0, HEADER_LEN)]
    pos = HEADER_LEN
    while pos + 10 <= len(data):
        kind = data[pos : pos + 4].decode("latin-1")
        (size,) = struct.unpack_from("<I", data, pos + 4)
        found.append((kind, pos, 10 + size))
        pos += 10 + size
    return found


def main():
    if len(sys.argv) != 4:
        sys.exit(__doc__)
    with open(sys.argv[1], "rb") as ours_file:
        ours = bytearray(ours_file.read())
    with open(sys.argv[2], "rb") as theirs_file:
        theirs = bytearray(theirs_file.read())
    ours_blocks = blocks(ours, sys.argv[1])
    theirs_blocks = blocks(theirs, sys.argv[2])
    with open(sys.argv[3]) as unspecified_file:
        for line in unspecified_file:
            offset, length = map(int, line.split())
            ours[offset : offset + length] = bytes(length)
            theirs[offset : offset + length] = bytes(length)

    same = True
    for index, (a, b) in enumerate(zip(ours_blocks, theirs_blocks)):
        if (a[0], a[1]) != (b[0], b[1]):
            print(
                f"block {index}: the first is {a[0]} at {a[1]},"
                f" the second {b[0]} at {b[1]}"
            )
            same = False
            break
        mine = ours[a[1] : a[1] + a[2]]
        other = theirs[b[1] : b[1] + b[2]]
        if mine == other:
            continue
        same = False
        at = next(
            (i for i, (x, y) in enumerate(zip(mine, other)) if x != y),
            min(len(mine), len(other)),
        )
        print(
            f"block {index} {a[0]}: differs at byte {at} of the block"
            f" (lengths {len(mine)} and {len(other)})"
        )
        # Pages of constants are stored under a mask; show what is under.
        mask = mine[16] if a[0] == "CPPG" and at > 16 else 0
        start = max(0, at - 8)
        show = lambda data, mask=mask, start=start, at=at: bytes(
            x ^ mask for x in data[start : at + 16]
        ).hex(" ")
        print("  first:  " + show(mine))
        print("  second: " + show(other))
    if same and len(ours_blocks) != len(theirs_blocks):
        print(f"{len(ours_blocks)} blocks, the second has {len(theirs_blocks)}")
        same = False
    sys.exit(0 if same else 1)


main()
