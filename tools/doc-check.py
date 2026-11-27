"""Check the guide (docs/guide) against what Hexicon does.

usage: python tools/doc-check.py [--accept] [--hexicon PATH]

Each folder in docs/guide/examples is a game. It is built with Hexad as
it is in lib/hexad, and has to build without a warning. It is then
played from its command script, walk.txt, and what it prints has to be
walk.out, the transcript recorded for it. --accept records the
transcripts as they now are, for a change meant to change them.

Each Markdown file in docs/guide is checked too: a code block marked
`hexicon` or `toml` has to be, word for word, part of an example's source
or manifest, one marked `css` part of an example's style sheet, and one
marked `transcript` part of an example's transcript.

An example with a walk.doc is played a second time with --rich, as on a
rich display, and the document its markup makes, checked, has to be
walk.doc; a block marked `document` has to be part of one.
So the guide shows only code that builds and output that a game gives.

Exit status: 0 if all is as it should be, 1 otherwise.
"""

import argparse
import os
import re
import shutil
import subprocess
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
GUIDE = os.path.join(ROOT, "docs", "guide")
EXAMPLES = os.path.join(GUIDE, "examples")
OUT = os.path.join(ROOT, "build", "doc-check")
EXE = ".exe" if os.name == "nt" else ""


def text(path):
    with open(path, encoding="utf-8") as f:
        return f.read().replace("\r\n", "\n")


def image_of(folder):
    manifest = text(os.path.join(folder, "hexicon.toml"))
    found = re.search(r'^image\s*=\s*"([^"]+)"', manifest, re.MULTILINE)
    return found.group(1) if found else None


def check_example(name, hexicon, accept, problems):
    source = os.path.join(EXAMPLES, name)
    work = os.path.join(OUT, name)
    shutil.rmtree(work, ignore_errors=True)
    shutil.copytree(source, work)
    image = image_of(work)
    if image is None:
        problems.append(f"{name}: its manifest names no image")
        return
    built = subprocess.run(
        [hexicon, "build", "-Fs", os.path.join(ROOT, "lib")],
        cwd=work,
        capture_output=True,
        text=True,
        stdin=subprocess.DEVNULL,
        check=False,
    )
    said = (built.stdout + built.stderr).replace("\r\n", "\n").strip()
    if built.returncode != 0 or said != f"Built {image}.":
        problems.append(f"{name}: the build said:\n{said}")
        return
    played = subprocess.run(
        [hexicon, "run", "--plain", "--nomore", "--norand", "-I", "walk.txt", image],
        cwd=work,
        capture_output=True,
        stdin=subprocess.DEVNULL,
        check=False,
    )
    transcript = played.stdout.decode("utf-8", "replace").replace("\r\n", "\n")
    recorded = os.path.join(source, "walk.out")
    if accept:
        with open(recorded, "w", encoding="utf-8", newline="\n") as f:
            f.write(transcript)
    elif not os.path.exists(recorded) or text(recorded) != transcript:
        with open(
            os.path.join(work, "walk.out"), "w", encoding="utf-8", newline="\n"
        ) as f:
            f.write(transcript)
        problems.append(f"{name}: the transcript is not walk.out; see {work}")
        return
    print(f"SAME {name} ({transcript.count(chr(10))} lines)")

    # On a rich display, where the example has a document recorded.
    recorded = os.path.join(source, "walk.doc")
    if not os.path.exists(recorded):
        return
    played = subprocess.run(
        [hexicon, "run", "--rich", "--nomore", "--norand", "-I", "walk.txt", image],
        cwd=work,
        capture_output=True,
        stdin=subprocess.DEVNULL,
        check=False,
    )
    document = played.stdout.decode("utf-8", "replace").replace("\r\n", "\n")
    if accept:
        with open(recorded, "w", encoding="utf-8", newline="\n") as f:
            f.write(document)
    elif text(recorded) != document:
        with open(
            os.path.join(work, "walk.doc"), "w", encoding="utf-8", newline="\n"
        ) as f:
            f.write(document)
        problems.append(f"{name}: the document is not walk.doc; see {work}")
        return
    print(f"SAME {name} (rich) ({document.count(chr(10))} lines)")


def check_pages(problems):
    # Everything a block can be part of.
    sources, transcripts, documents = [], [], []
    for name in sorted(os.listdir(EXAMPLES)):
        folder = os.path.join(EXAMPLES, name)
        for file in sorted(os.listdir(folder)):
            path = os.path.join(folder, file)
            if file.endswith((".hxn", ".toml", ".h", ".css")):
                sources.append(text(path))
            elif file == "walk.out":
                transcripts.append(text(path))
            elif file == "walk.doc":
                documents.append(text(path))
    blocks = 0
    for page in sorted(os.listdir(GUIDE)):
        if not page.endswith(".md"):
            continue
        lines = text(os.path.join(GUIDE, page)).split("\n")
        i = 0
        while i < len(lines):
            fence = re.match(r"^```(\w*)\s*$", lines[i])
            if not fence:
                i += 1
                continue
            kind, start = fence.group(1), i + 1
            j = start
            while j < len(lines) and lines[j] != "```":
                j += 1
            block = "\n".join(lines[start:j]) + "\n"
            where = {
                "hexicon": sources,
                "toml": sources,
                "css": sources,
                "transcript": transcripts,
                "document": documents,
            }.get(kind)
            if where is not None:
                blocks += 1
                if not any(block in whole for whole in where):
                    problems.append(
                        f"{page}:{start}: this {kind} block is in no example:\n{block}"
                    )
            i = j + 1
    print(f"{blocks} blocks checked")


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--accept", action="store_true")
    parser.add_argument(
        "--hexicon",
        # Where cargo builds: target/, unless CARGO_TARGET_DIR says elsewhere.
        default=os.path.join(
            os.environ.get("CARGO_TARGET_DIR", os.path.join(ROOT, "target")),
            "release",
            "hexicon" + EXE,
        ),
    )
    args = parser.parse_args()
    problems = []
    for name in sorted(os.listdir(EXAMPLES)):
        if os.path.isdir(os.path.join(EXAMPLES, name)):
            check_example(name, args.hexicon, args.accept, problems)
    check_pages(problems)
    for problem in problems:
        print("DIFF " + problem)
    sys.exit(1 if problems else 0)


if __name__ == "__main__":
    main()
