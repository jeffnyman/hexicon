#!/bin/bash
# Build Airport twice with Hexicon and compare the images: once as it is,
# from airport.t3m and adv3Lite.tl, and once as a Hexicon project, with
# its sources renamed to .hxn and both it and adv3Lite (with its English
# library) described by hexicon.toml manifests. The images have to be
# identical.
#
# On Windows, run this from Git Bash.
#
# usage: manifest-compare.sh
#
# Environment:
#   HEXICON      the hexicon executable (default: target/release/hexicon)
#   TADS_RUNNER  the reference's sources (default: ../_tads/tads-runner)
#   AIRPORT_SRC  Airport's sources (default: build/corpus/airport)
#   ADV3LITE     the adv3Lite library (default: build/corpus/adv3lite)

root=$(cd "$(dirname "$0")/../.." && pwd)
runner=${TADS_RUNNER:-$root/../_tads/tads-runner}
hex=${HEXICON:-$root/target/release/hexicon}
out=$root/build/manifest-compare
native() { cygpath -w "$1" 2>/dev/null || echo "$1"; }
LIB=$(native "$runner/tads3/lib")
INC=$(native "$runner/tads3/include")

rm -rf "$out"
mkdir -p "$out/libs" "$out/t3m" "$out/hxn"
cp -r "${ADV3LITE:-$root/build/corpus/adv3lite}" "$out/libs/adv3Lite"
src=${AIRPORT_SRC:-$root/build/corpus/airport}
cp -r "$src"/. "$out/t3m/"
rm -f "$out"/t3m/*.t3

# The Hexicon project: the sources renamed, and manifests written from
# the project file and the libraries.
for f in "$src"/*.t; do
  cp "$f" "$out/hxn/$(basename "${f%.t}").hxn"
done
python - "$out" <<'EOF'
import os, sys
out = sys.argv[1]

def tl_lines(path):
    for line in open(path, encoding='utf-8', errors='replace'):
        line = line.strip()
        if line and not line.startswith('#'):
            key, _, value = line.partition(':')
            yield key.strip().lower(), value.strip()

def quote(s):
    return '"' + s.replace('\\', '\\\\').replace('"', '\\"') + '"'

def library(tl, folder):
    sources, libraries, needs = [], [], []
    for key, value in tl_lines(tl):
        if key == 'source':
            sources.append(value + '.t')
        elif key == 'library':
            # "$(LANGUAGE)/$(LANGUAGE)" is the folder with its manifest.
            head, _, tail = value.rpartition('/')
            libraries.append(head if head == tail else value)
        elif key == 'needmacro':
            name, _, why = value.partition(' ')
            needs.append((name, why.strip()))
    with open(os.path.join(folder, 'hexicon.toml'), 'w', newline='\n') as f:
        f.write('[library]\n')
        f.write('sources = [\n' + ''.join(f'    {quote(s)},\n' for s in sources) + ']\n')
        if libraries:
            f.write('libraries = [' + ', '.join(map(quote, libraries)) + ']\n')
        if needs:
            f.write('\n[needs]\n' + ''.join(f'{n} = {quote(w)}\n' for n, w in needs))

lite = os.path.join(out, 'libs', 'adv3Lite')
library(os.path.join(lite, 'adv3Lite.tl'), lite)
library(os.path.join(lite, 'english', 'english.tl'), os.path.join(lite, 'english'))

with open(os.path.join(out, 'hxn', 'hexicon.toml'), 'w', newline='\n') as f:
    f.write('''[game]
name = "Airport"
sources = ["start", "gatearea", "plane", "npcs"]
libraries = ["system", "adv3Lite"]
image = "hex.t3"

[defines]
LANGUAGE = "english"

[build]
preinit = true
charset = "cp1252"
library-paths = ["../libs"]
''')
EOF

( cd "$out/t3m" && "$hex" build -cs cp1252 -FL "$LIB" -FI "$INC" -Fs "$(native "$out/libs/adv3Lite")" \
    -o t3m.t3 --unspecified t3m.skip -f airport.t3m ) > "$out/t3m.msg" 2>&1 < /dev/null
( cd "$out/hxn" && "$hex" build -FL "$LIB" -FI "$INC" --unspecified hex.skip ) \
  > "$out/hxn.msg" 2>&1 < /dev/null
tr -d '\r' < "$out/t3m.msg" | grep -v $'^\t' > "$out/t3m.txt"
# The Hexicon sources are checked for properties given values that
# nothing reads, which the TADS 3 ones are not; Airport has one, given
# twice. Those warnings are shown, and left out of the comparison, as
# is the line a Hexicon project's build ends with ("Built hex.t3."),
# which a project file's does not.
grep -h "nothing reads it" "$out/hxn.msg" | tr -d '\r' | sed 's/^/  /'
tr -d '\r' < "$out/hxn.msg" | grep -v $'^\t' | grep -v "nothing reads it" \
  | grep -v '^Built .*\.$' > "$out/hxn.txt"
if ! diff "$out/hxn.txt" "$out/t3m.txt" > "$out/msgdif"; then
  echo "DIFF messages ($(grep -c '^[<>]' "$out/msgdif") lines)"
  exit 1
elif [ ! -f "$out/t3m/t3m.t3" ] || [ ! -f "$out/hxn/hex.t3" ]; then
  echo "DIFF an image is missing"
  exit 1
elif python "$root/tools/image-diff.py" "$out/hxn/hex.t3" "$out/t3m/t3m.t3" "$out/hxn/hex.skip" > "$out/dif"; then
  echo "SAME airport ($(wc -c < "$out/hxn/hex.t3") bytes)"
else
  echo "DIFF $(head -1 "$out/dif")"
  exit 1
fi
