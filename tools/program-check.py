#!/usr/bin/env python3
"""Check Hexicon against the programs in tests/programs, with nothing
else installed.

Each case is a Hexicon program (one `.hxn` file, or a folder of them
built together in the order of their names) and, beside it, a `.expect`
file: what building it says, a fingerprint of the image it builds, and
what it prints when it runs. A case is built without the default modules
and without preinitialization, and run with --norand. A program that
never finishes is only built.

The cases were recorded once, on 2026-11-24, from the compiler's test
programs, where the reference compiler and interpreter did with the
originals what Hexicon did with these (see tests/README.md). Three were
recorded with what Hexicon does differently by design: bif1, bif2, and
bif3, where the VM says what it is and `hexicon` reports an image it
cannot run.

usage:
  program-check.py [case...]     check the cases (all, or those named)
  program-check.py --accept [case...]
                                 record what Hexicon now does as what
                                 the cases expect, for a change that is
                                 meant to change it

Environment:
  HEXICON      the hexicon executable (default: target/release/hexicon)
"""

import hashlib
import os
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
CASES = ROOT / "tests" / "programs"
WORK = ROOT / "build" / "program-check"
HEXICON = os.environ.get("HEXICON", str(ROOT / "target" / "release" / "hexicon"))
TIMEOUT = 60

# What a case expects to print when it is not run: a program that never
# finishes, as some of those that probe the compiler do, is only built.
NOT_RUN = "(not run: it does not finish)\n"

def normalize(text):
    """Output as it is compared: no carriage returns, and one newline at
    the end."""
    text = text.replace("\r", "")
    return text.rstrip("\n") + "\n" if text.strip() else ""


def run(args, cwd):
    try:
        done = subprocess.run(
            args,
            cwd=cwd,
            stdin=subprocess.DEVNULL,
            capture_output=True,
            timeout=TIMEOUT,
            check=False,
        )
    except subprocess.TimeoutExpired:
        return None, "(timed out)\n"
    return done.returncode, normalize(
        done.stdout.decode("utf-8", "replace") + done.stderr.decode("utf-8", "replace")
    )


def sources_of(case):
    """A case's source files, relative to the folder it is built in."""
    if case.is_dir():
        return sorted(p.name for p in case.glob("*.hxn"))
    return [case.name]


def fingerprint(image, skip):
    """The image's SHA-256, with the bytes that mean nothing (the time
    stamp, and the like: `hexicon build --unspecified`) set to zero."""
    data = bytearray(image.read_bytes())
    for line in skip.read_text().split("\n"):
        if line.strip():
            offset, length = map(int, line.split())
            data[offset : offset + length] = bytes(length)
    return hashlib.sha256(data).hexdigest()


def hexicon_result(case, run_it=True):
    """What Hexicon does with a case: (messages, fingerprint or None,
    output). With `run_it` false, the program is built but not run."""
    folder = case if case.is_dir() else case.parent
    with tempfile.TemporaryDirectory() as tmp:
        tmp = Path(tmp)
        image, skip = tmp / "case.hxg", tmp / "case.skip"
        _, messages = run(
            [
                HEXICON,
                "build",
                "-nodef",
                "-nopre",
                "-o",
                str(image),
                "--unspecified",
                str(skip),
                *sources_of(case),
            ],
            folder,
        )
        if not image.exists():
            return messages, None, ""
        if not run_it:
            return messages, fingerprint(image, skip), NOT_RUN
        # By its name, which a message about the image shows.
        _, output = run(
            [HEXICON, "run", "--norand", "--sandbox", str(tmp), image.name], tmp
        )
        return messages, fingerprint(image, skip), output


def expect_text(messages, print_, output):
    return f"image: {print_ or 'none'}\n--- messages\n{messages}--- output\n{output}"


def expect_file(case):
    return (
        case.parent / (case.stem + ".expect")
        if not case.is_dir()
        else case.parent / (case.name + ".expect")
    )


def all_cases():
    found = []
    for group in sorted(p for p in CASES.iterdir() if p.is_dir()):
        for item in sorted(group.iterdir()):
            if item.suffix == ".hxn" or (item.is_dir() and any(item.glob("*.hxn"))):
                found.append(item)
    return found


def chosen(names):
    cases = all_cases()
    if not names:
        return cases
    picked = [
        c for c in cases if c.stem in names or f"{c.parent.name}/{c.stem}" in names
    ]
    missing = (
        set(names)
        - {c.stem for c in picked}
        - {f"{c.parent.name}/{c.stem}" for c in picked}
    )
    if missing:
        sys.exit(f"no such case: {', '.join(sorted(missing))}")
    return picked


def check(names, accept):
    same = different = 0
    for case in chosen(names):
        expect = expect_file(case)
        want = expect.read_text(encoding="utf-8") if expect.exists() else ""
        got = expect_text(*hexicon_result(case, not want.endswith(NOT_RUN)))
        if accept:
            expect.write_text(got, encoding="utf-8", newline="\n")
            continue
        if got == want:
            same += 1
        else:
            different += 1
            name = f"{case.parent.name}/{case.stem}"
            print(f"DIFF {name}")
            WORK.mkdir(parents=True, exist_ok=True)
            (WORK / f"{case.parent.name}_{case.stem}.got").write_text(
                got, encoding="utf-8", newline="\n"
            )
    if accept:
        print("recorded")
        return 0
    print(f"same={same} different={different}")
    return 1 if different else 0


def main():
    args = sys.argv[1:]
    if args[:1] == ["--accept"]:
        return check(args[1:], True)
    if args[:1] in (["-h"], ["--help"]):
        print(__doc__)
        return 0
    return check(args, False)


if __name__ == "__main__":
    sys.exit(main())
