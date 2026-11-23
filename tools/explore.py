"""Explore a game with no walkthrough, under Hexicon and the reference.

usage: python tools/explore.py [options] game.t3 [game.t3 ...]

A game is played from the start by a script that grows a round at a time.
Each round, the script so far is replayed under the reference, and the
next commands are chosen from what the game last said: the words it used
are examined, what can be examined is taken, opened, read, searched, and
so on, and now and then the player moves on, looks, takes inventory,
undoes, or tries something the parser will refuse. The choices depend
only on the reference's output and the seed, so a run can be repeated.

Then the whole script is replayed under both interpreters, and the two
transcripts compared. The script is kept, next to the game or where
--out says, so that it can be replayed again later.

Options:
  --rounds N   rounds of commands to add (default 30)
  --seed N     the seed for the choices (default 1)
  --keys N     key presses to send before the first command (default:
               as many as the game needs to reach its first prompt)
  --start E    an event to send first, before any key presses: line:TEXT
               or key:KEY, as for a question the game asks before its
               first prompt (may be given more than once)
  --stdin      feed the commands as plain input instead of as an event
               script, for a game whose start the reference cannot be
               scripted through (a key is then one character of input)
  --out DIR    where to write NAME.explore.txt (default: beside the game)
  --hex PATH   the Hexicon executable (default target/release/hexicon)
  --ref PATH   the reference interpreter (default build/reference/bin/t3run)

Exit status: 0 if every game's transcripts are the same, 1 otherwise.
"""

import argparse
import os
import random
import re
import subprocess
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
EXE = ".exe" if os.name == "nt" else ""
TIMEOUT = 30

DIRECTIONS = ["n", "s", "e", "w", "ne", "nw", "se", "sw", "u", "d", "in", "out"]
DIR_WORDS = {
    "north": "n",
    "south": "s",
    "east": "e",
    "west": "w",
    "northeast": "ne",
    "northwest": "nw",
    "southeast": "se",
    "southwest": "sw",
    "up": "u",
    "down": "d",
    "upward": "u",
    "downward": "d",
    "inside": "in",
    "outside": "out",
    "fore": "fore",
    "aft": "aft",
    "port": "port",
    "starboard": "starboard",
}
# Verbs tried on something that turned out to be there.
VERBS = [
    "take",
    "open",
    "read",
    "search",
    "look in",
    "look under",
    "look behind",
    "push",
    "pull",
    "turn",
    "feel",
    "smell",
    "listen to",
    "close",
    "eat",
    "wear",
    "enter",
    "climb",
    "move",
    "break",
    "light",
    "unlock",
    "drop",
]
# Commands that take no object. Not HINT, ABOUT, or HELP: those often open
# a menu that reads single keys, and the reference waits for ever for a key
# once the script runs out.
PLAIN = [
    "look",
    "inventory",
    "score",
    "wait",
    "listen",
    "smell",
    "jump",
    "sleep",
    "yes",
    "no",
    "exits",
    "again",
    "undo",
    "think",
]
# Commands the parser should refuse, one way or another.
ODD = [
    "xyzzy",
    "take",
    "x",
    "put",
    "put it",
    "x the",
    "take all from me",
    "give it to",
    "n, s",
    "frobnicate the widget",
    "oops",
    "it",
    "take all",
    "drop all",
    "x all",
    "x me",
    "x it",
    "take it then drop it",
    "x 12345",
    'say "hello"',
    "tell me about it",
]
# Words not worth examining.
STOP = {
    "the",
    "and",
    "you",
    "your",
    "are",
    "with",
    "that",
    "this",
    "there",
    "here",
    "have",
    "has",
    "into",
    "onto",
    "from",
    "for",
    "not",
    "but",
    "its",
    "it's",
    "can",
    "can't",
    "don't",
    "see",
    "nothing",
    "unusual",
    "about",
    "which",
    "what",
    "where",
    "when",
    "who",
    "how",
    "was",
    "were",
    "will",
    "would",
    "could",
    "should",
    "them",
    "they",
    "their",
    "then",
    "than",
    "some",
    "any",
    "all",
    "one",
    "two",
    "very",
    "just",
    "also",
    "only",
    "out",
    "off",
    "over",
    "under",
    "above",
    "below",
    "behind",
    "through",
    "back",
    "way",
    "way.",
    "lead",
    "leads",
    "obvious",
    "exits",
    "exit",
    "north",
    "south",
    "east",
    "west",
    "northeast",
    "northwest",
    "southeast",
    "southwest",
    "up",
    "down",
    "story",
    "understand",
    "command",
    "word",
    "necessary",
    "seem",
    "seems",
    "like",
    "look",
    "looks",
    "little",
    "more",
    "much",
    "most",
    "other",
    "such",
    "too",
    "been",
    "being",
    "does",
    "doing",
    "done",
    "get",
    "gets",
    "got",
    "had",
    "his",
    "her",
    "him",
    "she",
    "he",
    "our",
    "ours",
    "yours",
    "mine",
    "myself",
    "yourself",
    "nor",
    "yet",
    "perhaps",
    "okay",
    "well",
    "upon",
    "while",
    "still",
    "already",
    "again",
    "each",
    "every",
}
REFUSED = re.compile(
    r"you see no|not necessary|doesn't understand|don't know|not important"
    r"|isn't important|can't see|is not something|no such|don't see",
    re.IGNORECASE,
)
GAME_OVER = re.compile(r"\bUNDO\b.*\bQUIT\b|RESTART.*UNDO", re.DOTALL)


def run(cmd, cwd, script, timeout=TIMEOUT):
    try:
        with open(script, "rb") as feed:
            out = subprocess.run(
                cmd,
                cwd=cwd,
                stdin=feed if STDIN else subprocess.DEVNULL,
                capture_output=True,
                timeout=timeout,
                check=False,
            ).stdout
    except subprocess.TimeoutExpired as e:
        out = (e.stdout or b"") + b"\n[timed out]\n"
    return out.replace(b"\r", b"").decode("utf-8", "replace")


START = []
STDIN = False


def write_script(path, keys, commands):
    with open(path, "w", encoding="utf-8", newline="\n") as f:
        if STDIN:
            # Plain input: a key is a character, a line a line.
            for event in START:
                kind, _, text = event.partition(":")
                f.write(text + "\n" if kind == "line" else text)
            f.write(" " * keys)
            f.writelines(c + "\n" for c in commands)
            return
        f.write("<eventscript>\n")
        for event in START:
            kind, _, text = event.partition(":")
            f.write(f"<{kind}>{text}\n")
        for _ in range(keys):
            f.write("<key>[space]\n")
        for c in commands:
            f.writelines("<line>" + c + "\n" for c in commands)


def ref_cmd(args, script, game):
    feed = [] if STDIN else ["-I", script]
    return [args.ref, "-cs", "utf8", "-plain", "-norand", "-nomore", *feed, game]


def hex_cmd(args, script, game):
    feed = [] if STDIN else ["-I", script]
    return [args.hex, "run", "--plain", "--norand", "--nomore", *feed, game]


def responses(transcript):
    """The reply to each command, in order (the text before the first is skipped)."""
    # A script's command is echoed after the prompt; plain input is not,
    # and the reply starts on the prompt's line.
    parts = re.split(r"^>" if STDIN else r"^>.*$", transcript, flags=re.MULTILINE)
    return [p.strip() for p in parts[1:]]


class Explorer:
    def __init__(self, seed):
        self.rng = random.Random(seed)
        self.commands = []
        self.examined = set()
        self.present = []  # words that examined to something
        self.tried = set()

    def nouns(self, text):
        words = re.findall(r"[a-z][a-z'-]{2,}", text.lower())
        seen = []
        for w in words:
            w = w.strip("'-")
            if len(w) < 3 or w in STOP or w in seen:
                continue
            seen.append(w)
        return seen

    def learn(self, replies):
        """Note which examined words named something."""
        for cmd, reply in zip(self.commands, replies):
            if cmd.startswith("x ") and not REFUSED.search(reply):
                w = cmd[2:]
                if w not in self.present:
                    self.present.append(w)

    def next_round(self, replies):
        last = replies[-1] if replies else ""
        if GAME_OVER.search(last):
            return ["undo"]
        recent = " ".join(replies[-6:])
        out = []
        fresh = [w for w in self.nouns(recent) if w not in self.examined]
        for w in fresh[:8]:
            self.examined.add(w)
            out.append("x " + w)
        for w in self.present[-10:]:
            verbs = [v for v in VERBS if (v, w) not in self.tried]
            if verbs and self.rng.random() < 0.5:
                v = self.rng.choice(verbs)
                self.tried.add((v, w))
                out.append(f"{v} {w}")
        out.append(self.rng.choice(PLAIN))
        if self.rng.random() < 0.3:
            out.append(self.rng.choice(ODD))
        if self.rng.random() < 0.15 and len(self.present) >= 2:
            a, b = self.rng.sample(self.present[-10:], 2)
            out.append(
                self.rng.choice(
                    [
                        f"put {a} in {b}",
                        f"put {a} on {b}",
                        f"give {a} to {b}",
                        f"show {a} to {b}",
                        f"ask {b} about {a}",
                        f"tell {b} about {a}",
                        f"unlock {b} with {a}",
                        f"hit {b} with {a}",
                    ]
                )
            )
        # Mostly go a way the text mentions, which is likely an exit.
        named = [
            DIR_WORDS[w]
            for w in re.findall(r"[a-z]+", recent.lower())
            if w in DIR_WORDS
        ]
        if named and self.rng.random() < 0.7:
            out.append(self.rng.choice(named))
        else:
            out.append(self.rng.choice(DIRECTIONS))
        if self.rng.random() < 0.2:
            out.append("take all")
        out.append("look")
        return out


def first_difference(a, b):
    ra, rb = re.split(r"(?m)^(?=>)", a), re.split(r"(?m)^(?=>)", b)
    for i, (x, y) in enumerate(zip(ra, rb)):
        if x != y:
            return i, x, y
    i = min(len(ra), len(rb))
    return i, "".join(ra[i : i + 1]), "".join(rb[i : i + 1])


def explore(args, game):
    game = os.path.abspath(game)
    cwd = os.path.dirname(game)
    name = os.path.splitext(os.path.basename(game))[0]
    out_dir = args.out or cwd
    script = os.path.join(out_dir, name + (".explore.in" if STDIN else ".explore.txt"))
    ex = Explorer(args.seed)
    keys = args.keys
    if keys is None:
        # Some games wait for a key before the first prompt; find how many.
        for keys in range(6):
            write_script(script, keys, ["look"])
            if responses(run(ref_cmd(args, script, game), cwd, script, 10)):
                break
        else:
            print(f"{name}: never reached a prompt; skipped", flush=True)
            return True
    replies = []
    for _ in range(args.rounds):
        ex.commands += ex.next_round(replies)
        write_script(script, keys, ex.commands)
        text = run(ref_cmd(args, script, game), cwd, script)
        replies = responses(text)
        ex.learn(replies)
        if "[timed out]" in text:
            # The game is waiting for something the script cannot give it,
            # such as a key, after the last command it echoed; replay what
            # came before that.
            ex.commands = ex.commands[: max(0, len(replies) - 1)]
            write_script(script, keys, ex.commands)
            break
        if len(replies) < len(ex.commands) // 2:
            # The game stopped reading commands (it quit, or it waits for
            # something the script does not give it); go no further.
            break
    ref = run(ref_cmd(args, script, game), cwd, script)
    hx = run(hex_cmd(args, script, game), cwd, script)
    n = len(ex.commands)
    if ref != hx and run(ref_cmd(args, script, game), cwd, script) != ref:
        # A game that seeds its own random numbers from the clock differs
        # from run to run, under either interpreter.
        print(
            f"{name}: the reference differs from itself from run to run; "
            f"not compared ({n} commands)",
            flush=True,
        )
        return True
    if ref == hx:
        print(
            f"{name}: same ({n} commands, {len(ref.splitlines())} lines, "
            f"{len(ex.present)} things found)",
            flush=True,
        )
        return True
    if re.sub(r"\s", "", ref) == re.sub(r"\s", "", hx):
        print(f"{name}: same but for white space ({n} commands)", flush=True)
        return True
    i, x, y = first_difference(ref, hx)
    print(f"{name}: DIFFERENT at command {i} of {n}; script {script}")
    print("--- reference")
    print(x[:1500])
    print("--- hexicon")
    print(y[:1500], flush=True)
    return False


def main():
    p = argparse.ArgumentParser(
        description=__doc__.splitlines()[0] if __doc__ else None
    )
    p.add_argument("games", nargs="+")
    p.add_argument("--rounds", type=int, default=30)
    p.add_argument("--seed", type=int, default=1)
    p.add_argument("--keys", type=int)
    p.add_argument("--start", action="append", default=[])
    p.add_argument("--stdin", action="store_true")
    p.add_argument("--out")
    p.add_argument(
        "--hex", default=os.path.join(ROOT, "target", "release", "hexicon" + EXE)
    )
    p.add_argument(
        "--ref", default=os.path.join(ROOT, "build", "reference", "bin", "t3run" + EXE)
    )
    args = p.parse_args()
    START.extend(args.start)
    global STDIN
    STDIN = args.stdin
    ok = all(explore(args, g) for g in args.games)
    sys.exit(0 if ok else 1)


if __name__ == "__main__":
    main()
