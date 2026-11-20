"""Cut programs off at random places.

usage: gen-truncated.py <folder> <count> <seed> <file>...

Each program written is the start of one of the files, up to a place
chosen at random. A source that ends in the middle of something is
where a parser is most likely to go round for ever, or to say something
different from the reference; compile the results with
tools/compile-compare.sh to find out.
"""

import os
import random
import sys


def main():
    if len(sys.argv) < 5:
        sys.exit(__doc__)
    folder, count, seed = sys.argv[1], int(sys.argv[2]), int(sys.argv[3])
    sources = sys.argv[4:]
    rng = random.Random(seed)
    os.makedirs(folder, exist_ok=True)
    for index in range(count):
        source = rng.choice(sources)
        with open(source, "rb") as file:
            data = file.read()
        cut = rng.randrange(len(data) + 1)
        stem = os.path.splitext(os.path.basename(source))[0]
        name = f"t{seed}_{index:04d}_{stem}.t"
        with open(os.path.join(folder, name), "wb") as out:
            out.write(data[:cut])
    print(f"{count} programs in {folder}")


main()
