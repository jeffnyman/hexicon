"""Generate crates/hexicon-vm/data/charmaps.bin from the reference VM's
compiled character maps (tads3/charmap/*.tcm).

The result is every map in one file, which the VM embeds: for each map, a
byte giving the length of its name, the name, the map's length as a
32-bit little-endian integer, and the map as it stands.

Usage: python tools/gen-charmaps.py <path to the charmap directory>
"""

import glob
import os
import re
import struct
import sys

out = bytearray()
paths = sorted(glob.glob(os.path.join(sys.argv[1], "*.tcm")))
for path in paths:
    name = os.path.splitext(os.path.basename(path))[0].lower().encode("ascii")
    with open(path, "rb") as map_file:
        data = map_file.read()
    out += struct.pack("<B", len(name)) + name
    out += struct.pack("<I", len(data)) + data


def compiled(pairs, entries):
    """Build a map in the compiled format: the character for each byte,
    then the bytes for each character (the first entry being what to
    write for a character with no entry), then the characters written
    with more than one byte, which count as expansions."""
    head = b"".join(struct.pack("<HH", char, byte) for char, byte in pairs)
    body = b"".join(
        struct.pack("<HB", char, len(text)) + text for char, text in entries
    )
    long = [(char, text) for char, text in entries if len(text) > 1]
    tail = b"".join(
        struct.pack("<HB", char, len(text))
        + b"".join(struct.pack("<H", byte) for byte in text)
        for char, text in long
    )
    start = 6 + len(head)
    return (
        struct.pack("<IH", start, len(pairs))
        + head
        + struct.pack("<HI", len(entries), len(body))
        + body
        + struct.pack("<HI", len(long), sum(len(text) for _, text in long))
        + tail
    )


# The reference also has two maps built in, for names that match no file:
# plain ASCII, with its own stand-ins for other characters, and Latin-1,
# with none. They go in under names no program can ask for.
with open(
    os.path.join(sys.argv[1], "..", "charmap.cpp"), encoding="latin-1"
) as source_file:
    source = source_file.read()
table = source[source.index("static ascii_map_t ascii_mapping[] =") :]
table = table[: table.index("};")]
ascii_entries = [(0, b"?")]
for char, number, text in re.findall(
    r'\{\s*(\d+),\s*(?:\{\s*(\d+)\s*\}|"((?:[^"\\]|\\.)*)")\s*\}', table
):
    if int(char) == 0:
        break
    if number:
        value = bytes([int(number)])
    else:
        value = text.encode("latin-1").decode("unicode_escape").encode("latin-1")
    ascii_entries.append((int(char), value))
builtin = {
    b"#ascii": compiled([(n, n) for n in range(128)], ascii_entries),
    b"#latin1": compiled(
        [(n, n) for n in range(256)],
        [(0, b"?")] + [(n, bytes([n])) for n in range(256)],
    ),
}
for name, data in builtin.items():
    out += struct.pack("<B", len(name)) + name
    out += struct.pack("<I", len(data)) + data
print(len(ascii_entries) - 1, "built-in ASCII entries")

os.makedirs("crates/hexicon-vm/data", exist_ok=True)
with open("crates/hexicon-vm/data/charmaps.bin", "wb") as output_file:
    output_file.write(out)
print(len(paths), "maps,", len(out), "bytes")
