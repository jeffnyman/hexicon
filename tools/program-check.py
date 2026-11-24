#!/usr/bin/env python3
"""Check Hexicon against the programs in tests/programs, with nothing
else installed.

Each case is a Hexicon program (one `.hxn` file, or a folder of them
built together in the order of their names) and, beside it, a `.expect`
file: what building it says, a fingerprint of the image it builds, and
what it prints when it runs. A case is built without the default modules
and without preinitialization, as the compiler's comparisons build
them, and run with --norand.

The cases were made from the compiler's test programs, which are TADS 3
sources (tools/compile-tests, and the programs gen-cases.py writes). For
each, the reference compiler built the original and the reference
interpreter ran it; Hexicon converted it, built it, and ran it; and the
case was kept only if both built an image or neither did, and the two
printed the same. So every recorded output is the reference's, and
every fingerprint is of an image whose program does what the
reference's did.

usage:
  program-check.py [case...]     check the cases (all, or those named)
  program-check.py --accept [case...]
                                 record what Hexicon now does as what
                                 the cases expect, for a change that is
                                 meant to change it
  program-check.py --record      make the cases again from the test
                                 programs, checked against the reference

Environment:
  HEXICON      the hexicon executable (default: target/release/hexicon)
  TADS_RUNNER  the reference's sources, for --record
               (default: ../_tads/tads-runner)
"""

import hashlib
import os
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
CASES = ROOT / "tests" / "programs"
WORK = ROOT / "build" / "program-check"
HEXICON = os.environ.get("HEXICON", str(ROOT / "target" / "release" / "hexicon"))
RUNNER = Path(os.environ.get("TADS_RUNNER", ROOT.parent / "_tads" / "tads-runner"))
REF_BIN = ROOT / "build" / "reference" / "bin"
TIMEOUT = 60

# What a case expects to print when it is not run: a program that never
# finishes, as some of those that probe the compiler do, is only built.
NOT_RUN = "(not run: it does not finish)\n"

# Cases whose output is meant to differ from the reference's: what the
# VM says about itself, and how the hexicon program reports an image it
# cannot run. They are recorded with Hexicon's output.
DIFFERENT_BY_DESIGN = {
    "gen/bif1": "the VM's name and version, and the image's name",
    "gen/bif2": "hexicon names itself and the image in a message",
    "gen/bif3": "hexicon names itself and the image in a message",
}


def normalize(text):
    """Output as the comparisons see it: no carriage returns, no heap
    report from a debug build of the reference, one newline at the end."""
    text = text.replace("\r", "")
    text = text.replace(
        "\n(T3VM) Memory blocks still in use:\n\nTotal blocks in use: 0\n", ""
    )
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


# ---- --record ------------------------------------------------------------


def native(path):
    return str(path)


def reference_result(sources, cwd, obj):
    """What the reference does with a program: (built an image, output)."""
    inc = RUNNER / "tads3" / "include"
    lib = RUNNER / "tads3" / "lib"
    image = obj / "ref.t3"
    for old in obj.glob("*"):
        old.unlink()
    run(
        [
            str(REF_BIN / "t3make.exe"),
            "-nodef",
            "-nopre",
            "-nobanner",
            "-a",
            "-I",
            str(inc),
            "-FL",
            str(lib),
            "-FI",
            str(inc),
            "-Fo",
            str(obj),
            "-Fy",
            str(obj),
            "-o",
            str(image),
            *sources,
        ],
        cwd,
    )
    if not image.exists():
        return False, ""
    _, output = run(
        [
            str(REF_BIN / "test_exec.exe"),
            "-sd",
            str(cwd),
            "-cs",
            "utf8",
            "-norand",
            str(image),
        ],
        cwd,
    )
    return True, output


def originals():
    """The test programs, as (group, name, source folder or file)."""
    src = WORK / "src"
    if src.exists():
        shutil.rmtree(src)
    gen = src / "gen"
    gen.mkdir(parents=True)
    subprocess.run(
        [
            sys.executable,
            str(ROOT / "tools" / "compile-tests" / "gen-cases.py"),
            str(gen),
        ],
        check=True,
        capture_output=True,
    )
    found = []
    for item in sorted(gen.iterdir()):
        if item.suffix == ".t" or item.is_dir():
            found.append(("gen", item))
    tests = ROOT / "tools" / "compile-tests"
    for item in sorted(tests.glob("*.t")):
        found.append(("hand", item))
    for item in sorted((tests / "linked").iterdir()):
        if item.is_dir():
            found.append(("hand", item))
    return found


def record():
    for need in [
        REF_BIN / "t3make.exe",
        REF_BIN / "test_exec.exe",
        RUNNER / "tads3" / "include",
    ]:
        if not need.exists():
            sys.exit(f"missing: {need}")
    if CASES.exists():
        shutil.rmtree(CASES)
    obj = WORK / "obj"
    obj.mkdir(parents=True, exist_ok=True)
    kept, left = 0, []
    for group, item in originals():
        name = item.stem if item.is_file() else item.name
        if item.is_dir():
            if list(item.glob("*.t3m")):
                left.append(
                    (f"{group}/{name}", "a project file, which this does not convert")
                )
                continue
            ref_sources = sorted(str(p) for p in item.glob("*.t"))
            if not ref_sources:
                left.append(
                    (f"{group}/{name}", "a folder of other cases, not a program")
                )
                continue
        else:
            ref_sources = [str(item)]
        ref_image, ref_output = reference_result(
            ref_sources, item if item.is_dir() else item.parent, obj
        )
        # Convert the program, by itself.
        conv_in = WORK / "convert-in"
        conv_out = WORK / "convert-out"
        for d in (conv_in, conv_out):
            if d.exists():
                shutil.rmtree(d)
        conv_in.mkdir(parents=True)
        for s in ref_sources:
            shutil.copy(s, conv_in)
        done = subprocess.run(
            [HEXICON, "convert", str(conv_in), str(conv_out)],
            capture_output=True,
            check=False,
        )
        if done.returncode != 0:
            left.append((f"{group}/{name}", "hexicon convert failed"))
            continue
        target_group = CASES / group
        target_group.mkdir(parents=True, exist_ok=True)
        hxn = sorted(conv_out.glob("*.hxn"))
        if item.is_dir():
            case = target_group / name
            case.mkdir()
            for f in hxn:
                shutil.copy(f, case)
        else:
            case = target_group / (name + ".hxn")
            shutil.copy(hxn[0], case)
        never_ends = ref_output == "(timed out)\n"
        messages, print_, output = hexicon_result(case, not never_ends)
        reason = None
        if ref_image != (print_ is not None):
            reason = (
                "the reference built an image"
                if ref_image
                else "only Hexicon built an image"
            )
        elif ref_image and not never_ends and output != ref_output:
            if f"{group}/{name}" in DIFFERENT_BY_DESIGN:
                print(
                    f"  {group}/{name}: kept with Hexicon's output: "
                    f"{DIFFERENT_BY_DESIGN[group + '/' + name]}"
                )
            else:
                reason = "the output differs from the reference's"
        if reason:
            left.append((f"{group}/{name}", reason))
            if case.is_dir():
                shutil.rmtree(case)
            else:
                case.unlink()
            continue
        expect_file(case).write_text(
            expect_text(messages, print_, output), encoding="utf-8", newline="\n"
        )
        kept += 1
    print(f"recorded {kept} cases; left out {len(left)}")
    for name, reason in left:
        print(f"  {name}: {reason}")
    return 0


def main():
    args = sys.argv[1:]
    if args[:1] == ["--record"]:
        return record()
    if args[:1] == ["--accept"]:
        return check(args[1:], True)
    if args[:1] in (["-h"], ["--help"]):
        print(__doc__)
        return 0
    return check(args, False)


if __name__ == "__main__":
    sys.exit(main())
