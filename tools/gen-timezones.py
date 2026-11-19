"""Generate crates/hexicon-vm/data/timezones.bin from the reference VM's
compiled time zone database (tads3/tz/timezones.t3tz), which its own tools
build from the IANA zoneinfo data.

The result holds the same zones in Hexicon's own layout. Numbers are
little-endian, and a name is a byte giving its length and then the name.

    "HXTZ", and the format version as a 32-bit integer: 1
    the number of zones, of links, and of abbreviations, 32 bits each
    for each zone: its name, the two letters of its country (or zeros),
        and the offset in the file of its record, 32 bits
    for each link, which is another name for a zone: the name, and the
        zone's index, 32 bits
    for each abbreviation: its name, a count as a byte, and for each zone
        that uses it: the zone's index, 32 bits; the offset from UTC in
        milliseconds, 32 bits, signed; and "S", "D", or "B" for standard
        time, daylight time, or both
    for each zone, its record: the record's length, 16 bits, and then
        the number of transitions, 16 bits
        the number of time types, 16 bits
        the number of rules, a byte
        the length of the abbreviations, a byte
        for each transition: the day number and the time of day in
            milliseconds, 32 bits each, signed, and the index of the time
            type it changes to, a byte
        for each rule: the offset of its abbreviation, a byte; the month;
            how the day is chosen; the day; the weekday, a byte each; the
            time of day in milliseconds, 24 bits, and a byte saying which
            clock that is on; and the offset from UTC and the daylight
            saving, in milliseconds, 32 bits each, signed
        for each time type: the offset from UTC and the daylight saving,
            32 bits each, signed, and the offset of its abbreviation, a
            byte
        the abbreviations, each ended by a zero byte
        a zero byte, or else the country as two letters, the coordinates
            as 15 characters and a zero byte, and a comment as a length
            byte, the text, and a zero byte

Usage: python tools/gen-timezones.py <path to timezones.t3tz>
"""

import struct
import sys

with open(sys.argv[1], "rb") as source_file:
    source = source_file.read()
if source[:4] != b"T3TZ":
    sys.exit("not a compiled time zone database: " + sys.argv[1])


def u32(at):
    return struct.unpack_from("<I", source, at)[0]


def name_at(at):
    """A name in the source: a length byte, the name, and a zero byte.
    Returns the name and the position after it."""
    length = source[at]
    return source[at + 1 : at + 1 + length], at + 1 + length + 1


# Each table in the source follows its length, which counts four bytes more
# than the table, and a count of entries.
zones_at = 24
zones_end = zones_at + u32(16) - 4
links_at = zones_end + 8
links_end = links_at + u32(zones_end) - 4
abbrs_at = links_end + 8
abbrs_end = abbrs_at + u32(links_end) - 4

zones = []
at = zones_at
while at < zones_end:
    name, at = name_at(at)
    position = u32(at)
    country = source[at + 4 : at + 6]
    size = struct.unpack_from("<H", source, position)[0]
    zones.append((name, country, source[position + 2 : position + 2 + size]))
    at += 6

links = []
at = links_at
while at < links_end:
    name, at = name_at(at)
    links.append((name, u32(at)))
    at += 4

abbrs = []
at = abbrs_at
while at < abbrs_end:
    name, at = name_at(at)
    count = source[at]
    at += 1
    abbrs.append((name, [source[at + 9 * i : at + 9 * i + 9] for i in range(count)]))
    at += 9 * count


def named(name):
    return struct.pack("<B", len(name)) + name


head = b"HXTZ" + struct.pack("<IIII", 1, len(zones), len(links), len(abbrs))
link_table = b"".join(named(name) + struct.pack("<I", zone) for name, zone in links)
abbr_table = b"".join(
    named(name) + struct.pack("<B", len(uses)) + b"".join(uses) for name, uses in abbrs
)
zone_table_size = sum(1 + len(name) + 2 + 4 for name, _, _ in zones)
position = len(head) + zone_table_size + len(link_table) + len(abbr_table)
zone_table = bytearray()
records = bytearray()
for name, country, record in zones:
    zone_table += named(name) + country + struct.pack("<I", position + len(records))
    records += struct.pack("<H", len(record)) + record

out = head + bytes(zone_table) + link_table + abbr_table + bytes(records)
with open("crates/hexicon-vm/data/timezones.bin", "wb") as output_file:
    output_file.write(out)
print(
    f"{len(zones)} zones, {len(links)} links, "
    f"{len(abbrs)} abbreviations, {len(out)} bytes"
)
