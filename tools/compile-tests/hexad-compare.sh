#!/bin/bash
# Build adv3Lite's games on adv3Lite and on Hexad (lib/hexad), and check
# that they are the same games: Airport, from its project file, and the
# exercises of adv3Lite's manual (Learning TADS 3 with adv3Lite), each a
# game in one file. Both are built by Hexicon.
#
# On Windows, run this from Git Bash.
#
# usage: hexad-compare.sh
#
# Hexad is in Hexicon's language and includes hexicon.h, so a game on it
# has more names in its tables of symbols and macros than one on
# adv3Lite. adv3Lite looks through those tables when it is
# preinitialized, so the objects it makes then are numbered differently,
# and a game can then behave differently where it depends on that order
# (Exercise 15 has random messages, picked in an order that does). So
# each game is checked twice, and both times byte for byte:
#
#   - without preinitialization, against adv3Lite as it is;
#   - with it, against adv3Lite with `#include <hexicon.h>` in the same
#     files and on the same lines as Hexad has it, but none of Hexad's
#     renames: a program with the same names and macros, module for
#     module, so that only the spelling of the calls differs.
#
# Environment:
#   HEXICON      the hexicon executable (default: target/release/hexicon)
#   TADS_RUNNER  the reference's sources (default: ../_tads/tads-runner)
#   AIRPORT_SRC  Airport's sources (default: build/corpus/airport)
#   ADV3LITE     the adv3Lite library (default: build/corpus/adv3lite)

root=$(cd "$(dirname "$0")/../.." && pwd)
runner=${TADS_RUNNER:-$root/../_tads/tads-runner}
hex=${HEXICON:-$root/target/release/hexicon}
out=$root/build/hexad-compare
native() { cygpath -w "$1" 2>/dev/null || echo "$1"; }
LIB=$(native "$runner/tads3/lib")
INC=$(native "$runner/tads3/include")
adv3lite=${ADV3LITE:-$root/build/corpus/adv3lite}
HEXAD=$(native "$root/lib")

rm -rf "$out"
mkdir -p "$out/with-include"
# adv3Lite with hexicon.h included where Hexad includes it.
python - "$adv3lite" "$root/lib/hexad" "$out/with-include/adv3Lite" <<'EOF'
import os, shutil, sys
lite, hexad, dest = sys.argv[1:]
shutil.copytree(lite, dest, ignore=shutil.ignore_patterns('docs'))
added = 0
for folder, _, files in os.walk(hexad):
    for name in files:
        if not name.endswith(('.hxn', '.h')):
            continue
        rel = os.path.relpath(os.path.join(folder, name), hexad)
        orig = os.path.join(dest, rel[:-4] + '.t' if rel.endswith('.hxn') else rel)
        new_lines = open(os.path.join(folder, name), 'rb').read().split(b'\n')
        old = open(orig, 'rb').read()
        if b'<hexicon.h>' in old:
            continue
        for i, line in enumerate(new_lines):
            if line.strip() == b'#include <hexicon.h>':
                old_lines = old.split(b'\n')
                old_lines.insert(i, line)
                open(orig, 'wb').write(b'\n'.join(old_lines))
                added += 1
                break
print(f'hexicon.h included in {added} files of adv3Lite', file=sys.stderr)
EOF

pass=0
fail=0
result() {
  if [ "$1" = 0 ]; then pass=$((pass + 1)); echo "SAME $2"; else fail=$((fail + 1)); echo "DIFF $2"; fi
}

# Build the game in folder $1 from project file $2, as $3 (nopre or
# pre), on the adv3Lite in folder $4 and on Hexad, and compare.
build_both() {
  local d=$1 t3m=$2 kind=$3 lite=$4 label=$5 pre=-pre
  [ "$kind" = nopre ] && pre=-nopre
  sed -e '/^-o /d' -e "s/^-pre\$/$pre/" "$d/$t3m" > "$d/lite-$kind.t3m"
  sed -e '/^-o /d' -e "s/^-pre\$/$pre/" -e 's/^-lib adv3Lite$/-lib hexad/' "$d/$t3m" \
    > "$d/hexad-$kind.t3m"
  ( cd "$d" && "$hex" build -cs cp1252 -FL "$LIB" -FI "$INC" -Fs "$(native "$lite")" \
      -o "lite-$kind.t3" --unspecified "lite-$kind.skip" -f "lite-$kind.t3m" ) 2>&1 < /dev/null \
    | tr -d '\r' | grep -v $'^\t' > "$d/lite-$kind.msg"
  ( cd "$d" && "$hex" build -cs cp1252 -FL "$LIB" -FI "$INC" -Fs "$HEXAD" \
      -o "hexad-$kind.t3" -f "hexad-$kind.t3m" ) 2>&1 < /dev/null \
    | tr -d '\r' | grep -v $'^\t' > "$d/hexad-$kind.msg"
  local same=0
  if ! diff "$d/lite-$kind.msg" "$d/hexad-$kind.msg" > "$d/$kind.msgdif"; then
    same=1
  elif ! python "$root/tools/image-diff.py" "$d/hexad-$kind.t3" "$d/lite-$kind.t3" \
      "$d/lite-$kind.skip" > "$d/$kind.dif" 2>&1; then
    same=1
  fi
  result "$same" "$label $6"
}

compare() {
  build_both "$1" "$2" nopre "$adv3lite" "$3" "without preinitialization"
  build_both "$1" "$2" pre "$out/with-include/adv3Lite" "$3" "preinitialized"
}

# Airport, from its own project file.
mkdir -p "$out/airport"
cp -r "${AIRPORT_SRC:-$root/build/corpus/airport}"/. "$out/airport/"
rm -f "$out"/airport/*.t3
compare "$out/airport" airport.t3m airport

# The exercises, each with a project file of its own.
for src in "$adv3lite"/docs/manual/docs/learning/*.t; do
  name=$(basename "$src" .t)
  d=$out/${name// /_}
  mkdir -p "$d"
  cp "$src" "$d/"
  printf -- '-pre\n-D LANGUAGE=english\n-lib system\n-lib adv3Lite\n-source "%s"\n' "$name" > "$d/game.t3m"
  compare "$d" game.t3m "$name"
done
echo "same=$pass different=$fail"
[ "$fail" = 0 ]
