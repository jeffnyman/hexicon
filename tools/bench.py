"""Time walkthroughs of published games under Hexicon and the reference.

usage: python tools/bench.py [hexicon-executable]

Each walkthrough is replayed under both interpreters, the best of a few
runs is kept, and the two outputs are compared. The console hosts break
lines differently around key presses, so only the text, without its
white space, has to be the same.

Environment:
  GAMES     the folder of games (default: ../hexicon-playground/games)
  SCRIPTS   the folder of walkthroughs (default: its sibling scripts)
  REF       the reference interpreter (default: build/reference/bin/t3run)
  RUNS      how many runs to take the best of (default: 3)
  ONLY_HEX  1 to time Hexicon alone
"""

import os
import subprocess
import sys
import time

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
GAMES = os.environ.get("GAMES", os.path.join(ROOT, "..", "hexicon-playground", "games"))
SCRIPTS = os.environ.get("SCRIPTS", os.path.join(GAMES, "..", "scripts"))
EXE = ".exe" if os.name == "nt" else ""
HEX = (
    sys.argv[1]
    if len(sys.argv) > 1
    else os.path.join(ROOT, "target", "release", "hexicon" + EXE)
)
REF = os.environ.get(
    "REF", os.path.join(ROOT, "build", "reference", "bin", "t3run" + EXE)
)
RUNS = int(os.environ.get("RUNS", "3"))
ONLY_HEX = os.environ.get("ONLY_HEX") == "1"
WALKS = [
    ("DitchDay", "ditch3"),
    ("AllHope", "AllHope"),
    ("Cain", "cain"),
    ("MrsPepper", "MrsPepper"),
    ("SquareCircle", "SquareCircle"),
    ("Banana", "Banana"),
]


def run(cmd):
    start = time.perf_counter()
    out = subprocess.run(
        cmd, cwd=GAMES, stdin=subprocess.DEVNULL, capture_output=True, check=False
    ).stdout
    return time.perf_counter() - start, out.replace(b"\r", b"")


total_h = total_r = 0.0
for name, game in WALKS:
    script = f"{SCRIPTS}/{name}.walkthrough.txt"
    hex_cmd = [
        HEX,
        "run",
        "--plain",
        "--norand",
        "--nomore",
        "-I",
        script,
        f"{game}.t3",
    ]
    ref_cmd = [
        REF,
        "-cs",
        "utf8",
        "-plain",
        "-norand",
        "-nomore",
        "-I",
        script,
        f"{game}.t3",
    ]
    best_h = best_r = float("inf")
    for _ in range(RUNS):
        t, out_h = run(hex_cmd)
        best_h = min(best_h, t)
        if not ONLY_HEX:
            t, out_r = run(ref_cmd)
            best_r = min(best_r, t)
    total_h += best_h
    lines = out_h.count(b"\n")
    if ONLY_HEX:
        print(f"{name:13} lines={lines:5} hex={best_h:6.2f}s")
    else:
        total_r += best_r
        # The console hosts differ in blank lines and where a key
        # press breaks a line; the words must be the same.
        same = "same" if b"".join(out_h.split()) == b"".join(out_r.split()) else "DIFF"
        print(
            f"{name:13} lines={lines:5} hex={best_h:6.2f}s ref={best_r:6.2f}s "
            f"ratio={best_h / best_r:4.2f} {same}"
        )
print(
    f"total hex={total_h:.2f}s"
    + ("" if ONLY_HEX else f" ref={total_r:.2f}s ratio={total_h / total_r:.2f}")
)
