#!/usr/bin/env python3
"""Build and play whole games on Hexad, and compare what they print with
what they printed before.

The games are kept outside this repository, in a folder of their own
(by default ../hexicon-playground/game-tests beside it), since they are
not ours. Each is a folder with a manifest (hexicon.toml), its sources,
a command script (walk.txt), and the transcript recorded from it
(walk.out). A game may have more scripts, each a walk-NAME.txt with its
walk-NAME.out, for routes the main one does not take; each is checked
as NAME after the game's name (squarecircle/caught). A game is built with `hexicon build` on the Hexad built into
Hexicon, and played with --norand, a clock that starts at the same time
(--clock), and the time zone UTC, so that it plays the same on every
run. Without the folder, the check says so and passes.

A script with a .doc beside its .out (walk.doc) is played a second time
with --rich, as on a rich display, and the document the game's markup
makes, checked, is compared with the .doc, as the transcript is with the
.out. To start one, make an empty .doc and record it with --accept.

The games are Airport and the exercises of adv3Lite's manual, written
in Hexicon by hexicon convert while it existed. Their transcripts were
first recorded where they matched the same games on adv3Lite, apart
from what Hexad means to do differently.

usage:
  game-check.py [game...]           check the games (all, or those named)
  game-check.py --accept [game...]  record what they print now

Environment:
  HEXICON     the hexicon executable (default: target/release/hexicon)
  GAME_TESTS  the folder of games
"""

import os
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
HEXICON = os.environ.get("HEXICON", str(ROOT / "target" / "release" / "hexicon"))
GAMES = Path(
    os.environ.get("GAME_TESTS", ROOT.parent / "hexicon-playground" / "game-tests")
)
WORK = ROOT / "build" / "game-check"
TIMEOUT = 600
CLOCK = "2000-01-01T09:15:00"


def scripts(game):
    """A game's scripts: (name, script, transcript), the main one first."""
    found = [(game.name, game / "walk.txt", game / "walk.out")]
    for extra in sorted(game.glob("walk-*.txt")):
        name = extra.stem[len("walk-"):]
        found.append((f"{game.name}/{name}", extra, extra.with_suffix(".out")))
    return found


def play(game):
    """Build a game and play each of its scripts: a list of transcripts in
    the order of scripts(game), or None and why there are none."""
    built = subprocess.run(
        [HEXICON, "build"],
        cwd=game,
        stdin=subprocess.DEVNULL,
        capture_output=True,
        timeout=TIMEOUT,
        check=False,
    )
    said = (built.stdout + built.stderr).decode("utf-8", "replace")
    found = re.search(r"^Built (.+)\.$", said, re.MULTILINE)
    if built.returncode != 0 or not found:
        return None, said
    image = found.group(1)
    return [
        (
            run(game, image, script),
            run(game, image, script, rich=True)
            if expect.with_suffix(".doc").exists()
            else None,
        )
        for _, script, expect in scripts(game)
    ], None


def run(game, image, script, rich=False):
    """Play one script on a built game: the transcript, or with rich, the
    document a rich display is given, checked, as text."""
    # The same random choices, the same time of day, and the same zone,
    # on every run.
    done = subprocess.run(
        [
            HEXICON,
            "run",
            "--norand",
            "--clock",
            CLOCK,
            "--rich" if rich else "--plain",
            "--nomore",
            "-I",
            script.name,
            image,
        ],
        cwd=game,
        stdin=subprocess.DEVNULL,
        capture_output=True,
        timeout=TIMEOUT,
        check=False,
        env={**os.environ, "TZ": "Etc/UTC"},
    )
    return (done.stdout + done.stderr).decode("utf-8", "replace").replace(
        "\r", ""
    )


def main():
    args = sys.argv[1:]
    accept = args[:1] == ["--accept"]
    if accept:
        args = args[1:]
    if args[:1] in (["-h"], ["--help"]):
        print(__doc__)
        return 0
    if not GAMES.is_dir():
        print(f"no games to check: {GAMES} is not there (see GAME_TESTS)")
        return 0
    games = sorted(
        p
        for p in GAMES.iterdir()
        if (p / "hexicon.toml").exists() and (p / "walk.txt").exists()
    )
    if args:
        games = [g for g in games if g.name in args]
        missing = set(args) - {g.name for g in games}
        if missing:
            sys.exit(f"no such game: {', '.join(sorted(missing))}")
    same = different = 0
    for game in games:
        transcripts, failed = play(game)
        if transcripts is None:
            different += 1
            print(f"DIFF {game.name}: it did not build\n{failed}")
            continue
        for (name, _, expect), (transcript, document) in zip(
            scripts(game), transcripts
        ):
            outputs = [(name, expect, transcript, ".out")]
            if document is not None:
                outputs.append(
                    (f"{name} (rich)", expect.with_suffix(".doc"), document, ".doc")
                )
            for shown, expect, text, suffix in outputs:
                if accept:
                    expect.write_text(text, encoding="utf-8", newline="\n")
                    print(f"recorded {shown} ({text.count(chr(10))} lines)")
                    continue
                want = expect.read_text(encoding="utf-8") if expect.exists() else ""
                if text == want:
                    same += 1
                    print(f"SAME {shown} ({text.count(chr(10))} lines)")
                else:
                    different += 1
                    WORK.mkdir(parents=True, exist_ok=True)
                    got = WORK / f"{name.replace('/', '-')}{suffix}"
                    got.write_text(text, encoding="utf-8", newline="\n")
                    print(f"DIFF {shown}: see {got}")
    if not accept:
        print(f"same={same} different={different}")
    return 1 if different else 0


if __name__ == "__main__":
    sys.exit(main())
