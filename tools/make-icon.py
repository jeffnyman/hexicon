"""Draw Hexicon's icon: crates/hexicon-app/icon/hexicon.svg, and from the
same shapes hexicon.ico, which the player and the files of its games show
on Windows.

The mark is three hexagons, one inside another: a dark one, a light ring,
and a warm one in the middle. Drawn here in Python alone, sampled sixteen
times a pixel, so it needs nothing installed.

usage: python tools/make-icon.py
"""

import math
import struct
import zlib
from pathlib import Path

OUT = Path(__file__).resolve().parent.parent / "crates" / "hexicon-app" / "icon"

# Each layer: its circumradius, as a part of half the icon's width, and
# its color; later layers are drawn over earlier ones. A ring is a layer
# with the one under it drawn again inside it.
LAYERS = [
    (0.98, (0x2F, 0x3E, 0x7A)),  # the dark hexagon
    (0.80, (0xF2, 0xE6, 0xC9)),  # the light ring...
    (0.66, (0x2F, 0x3E, 0x7A)),  # ...is the dark again inside it
    (0.38, (0xE0, 0x9A, 0x3A)),  # the warm middle
]

SIZES = [16, 24, 32, 48, 64, 128, 256]
SAMPLES = 4  # a side: 16 samples a pixel


def inside(dx, dy, radius):
    """Whether a point is in a hexagon with a point at the top."""
    apothem = radius * math.sqrt(3) / 2
    x, y = abs(dx), abs(dy)
    return x <= apothem and x * 0.5 + y * (math.sqrt(3) / 2) <= apothem


def color_at(dx, dy):
    top = None
    for radius, color in LAYERS:
        if inside(dx, dy, radius):
            top = color
    return top


def draw(size):
    half = size / 2
    rows = []
    for py in range(size):
        row = bytearray([0])  # the PNG filter: none
        for px in range(size):
            r = g = b = covered = 0
            for sy in range(SAMPLES):
                for sx in range(SAMPLES):
                    x = (px + (sx + 0.5) / SAMPLES - half) / half
                    y = (py + (sy + 0.5) / SAMPLES - half) / half
                    c = color_at(x, y)
                    if c:
                        r, g, b = r + c[0], g + c[1], b + c[2]
                        covered += 1
            if covered:
                row += bytes([r // covered, g // covered, b // covered,
                              round(255 * covered / SAMPLES ** 2)])
            else:
                row += bytes(4)
        rows.append(bytes(row))
    return png(size, b"".join(rows))


def png(size, raw):
    def chunk(kind, data):
        body = kind + data
        return struct.pack(">I", len(data)) + body + struct.pack(">I", zlib.crc32(body))
    header = struct.pack(">IIBBBBB", size, size, 8, 6, 0, 0, 0)
    return (b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", header)
            + chunk(b"IDAT", zlib.compress(raw, 9)) + chunk(b"IEND", b""))


def ico(images):
    """An .ico of PNG images, as Windows has read since Vista."""
    out = struct.pack("<HHH", 0, 1, len(images))
    offset = 6 + 16 * len(images)
    for size, data in images:
        side = 0 if size >= 256 else size
        out += struct.pack("<BBBBHHII", side, side, 0, 0, 1, 32, len(data), offset)
        offset += len(data)
    return out + b"".join(data for _, data in images)


def svg():
    def points(radius):
        r = radius * 128
        return " ".join(
            f"{128 + r * math.cos(math.radians(-90 + 60 * k)):.2f},"
            f"{128 + r * math.sin(math.radians(-90 + 60 * k)):.2f}"
            for k in range(6))
    shapes = "\n".join(
        f'  <polygon points="{points(radius)}" fill="#{c[0]:02x}{c[1]:02x}{c[2]:02x}"/>'
        for radius, c in LAYERS)
    return ('<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 256 256" '
            'width="256" height="256">\n' + shapes + "\n</svg>\n")


def main():
    OUT.mkdir(parents=True, exist_ok=True)
    (OUT / "hexicon.svg").write_text(svg(), encoding="utf-8", newline="\n")
    (OUT / "hexicon.ico").write_bytes(ico([(s, draw(s)) for s in SIZES]))
    print(f"wrote {OUT / 'hexicon.svg'} and {OUT / 'hexicon.ico'}")


if __name__ == "__main__":
    main()
