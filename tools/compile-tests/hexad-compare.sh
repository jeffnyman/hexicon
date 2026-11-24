#!/bin/bash
# Build adv3Lite's games on adv3Lite and on Hexad (lib/hexad), play them
# both from the same command script, and check that they differ only as
# Hexad means them to: Airport, from its project file, and the exercises
# of adv3Lite's manual (Learning TADS 3 with adv3Lite), each a game in
# one file. Both are built by Hexicon, preinitialized, and played with
# --norand, so that each plays the same on every run.
#
# On Windows, run this from Git Bash.
#
# usage: hexad-compare.sh [--accept]
#
# For each game, tools/compile-tests/hexad/ has:
#   NAME.txt    the command script: VERSION and CREDITS, then what
#               tools/explore.py chose on adv3Lite (30 rounds, seed 1)
#   NAME.diff   how the transcript on Hexad differs from the one on
#               adv3Lite (diff's output, adv3Lite first); no file means
#               they are the same
# --accept writes the differences found as the new NAME.diff files, for
# a change to Hexad that is meant to change what games do.
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
expect=$root/tools/compile-tests/hexad
native() { cygpath -w "$1" 2>/dev/null || echo "$1"; }
LIB=$(native "$runner/tads3/lib")
INC=$(native "$runner/tads3/include")
adv3lite=${ADV3LITE:-$root/build/corpus/adv3lite}
LITE=$(native "$adv3lite")
HEXAD=$(native "$root/lib")
accept=0
[ "$1" = --accept ] && accept=1

rm -rf "$out"
mkdir -p "$out"
pass=0
fail=0

# Build the game in folder $1 from project file $2 on adv3Lite and on
# Hexad, play both from the script for $3, and compare.
compare() {
  local d=$1 t3m=$2 name=$3
  sed -e '/^-o /d' "$d/$t3m" > "$d/lite.t3m"
  sed -e '/^-o /d' -e 's/^-lib adv3Lite$/-lib hexad/' "$d/$t3m" > "$d/hexad.t3m"
  ( cd "$d" && "$hex" build -cs cp1252 -FL "$LIB" -FI "$INC" -Fs "$LITE" \
      -o lite.t3 -f lite.t3m ) 2>&1 < /dev/null | tr -d '\r' | grep -v $'^\t' > "$d/lite.msg"
  ( cd "$d" && "$hex" build -cs cp1252 -FL "$LIB" -FI "$INC" -Fs "$HEXAD" \
      -o hexad.t3 --unspecified hexad.skip -f hexad.t3m ) 2>&1 < /dev/null \
    | tr -d '\r' | grep -v $'^\t' > "$d/hexad.msg"
  if ! diff "$d/lite.msg" "$d/hexad.msg" > "$d/msg.diff"; then
    fail=$((fail + 1)); echo "DIFF $name: the builds' messages"; return
  fi
  if [ ! -f "$d/lite.t3" ] || [ ! -f "$d/hexad.t3" ]; then
    fail=$((fail + 1)); echo "DIFF $name: an image is missing"; return
  fi
  if [ ! -f "$expect/$name.txt" ]; then
    fail=$((fail + 1)); echo "DIFF $name: no command script, $expect/$name.txt"; return
  fi
  local script
  script=$(native "$expect/$name.txt")
  for lib in lite hexad; do
    ( cd "$d" && "$hex" run --norand --plain --nomore -I "$script" "$lib.t3" ) \
      < /dev/null 2>&1 | tr -d '\r' > "$d/$lib.out"
  done
  diff "$d/lite.out" "$d/hexad.out" > "$d/play.diff"
  if [ "$accept" = 1 ]; then
    if [ -s "$d/play.diff" ]; then cp "$d/play.diff" "$expect/$name.diff"; else rm -f "$expect/$name.diff"; fi
  fi
  local want=/dev/null
  [ -f "$expect/$name.diff" ] && want=$expect/$name.diff
  if cmp -s "$d/play.diff" "$want"; then
    pass=$((pass + 1))
    if [ -s "$d/play.diff" ]; then
      echo "SAME $name ($(wc -l < "$d/hexad.out") lines, differing as expected)"
    else
      echo "SAME $name ($(wc -l < "$d/hexad.out") lines)"
    fi
  else
    fail=$((fail + 1)); echo "DIFF $name: see $d/play.diff"
  fi
}

# Airport, from its own project file.
mkdir -p "$out/airport"
cp -r "${AIRPORT_SRC:-$root/build/corpus/airport}"/. "$out/airport/"
rm -f "$out"/airport/*.t3
compare "$out/airport" airport.t3m airport

# Airport again, in Hexicon: its sources as .hxn files that import
# Hexad (`import hexad;`) where they included its header. That is the
# same program, so the image must be the same as the one above.
mkdir -p "$out/airport-hxn"
for src in "$out"/airport/*.t; do
  sed 's/^#include "advlite.h"/import hexad;/' "$src" \
    > "$out/airport-hxn/$(basename "$src" .t).hxn"
done
# A manifest, which names each module by its source's name without the
# extension, as the project file does.
cat > "$out/airport-hxn/hexicon.toml" <<'TOML'
[game]
libraries = ["system", "hexad"]
sources = ["start", "gatearea", "plane", "npcs"]

[defines]
LANGUAGE = "english"

[build]
preinit = true
charset = "cp1252"
TOML
( cd "$out/airport-hxn" && "$hex" build -FL "$LIB" -FI "$INC" -Fs "$HEXAD" \
    -o hexad.t3 ) > "$out/airport-hxn/hexad.msg" 2>&1 < /dev/null
if [ "$(grep -l '^import hexad;' "$out"/airport-hxn/*.hxn | wc -l)" != 4 ]; then
  fail=$((fail + 1)); echo "DIFF airport, importing Hexad: not every source imports it"
elif python "$root/tools/image-diff.py" "$out/airport-hxn/hexad.t3" "$out/airport/hexad.t3" \
    "$out/airport/hexad.skip" > "$out/airport-hxn/dif" 2>&1; then
  pass=$((pass + 1)); echo "SAME airport, importing Hexad: the same image"
else
  fail=$((fail + 1)); echo "DIFF airport, importing Hexad: $(head -1 "$out/airport-hxn/dif")"
fi

# Airport again, with Hexicon's own system library and headers, which
# are built into it, in place of the reference's (no -FL or -FI): the
# same files, so the same image.
mkdir -p "$out/airport-own"
cp "$out"/airport/*.t "$out/airport/hexad.t3m" "$out/airport-own/"
( cd "$out/airport-own" && "$hex" build -cs cp1252 -Fs "$HEXAD" -o hexad.t3 -f hexad.t3m ) \
  > "$out/airport-own/hexad.msg" 2>&1 < /dev/null
if grep -qi "tads-runner\|_tads" "$out/airport-own/hexad.msg"; then
  fail=$((fail + 1)); echo "DIFF airport, on Hexicon's system library: a reference file was read"
elif python "$root/tools/image-diff.py" "$out/airport-own/hexad.t3" "$out/airport/hexad.t3" \
    "$out/airport/hexad.skip" > "$out/airport-own/dif" 2>&1; then
  pass=$((pass + 1)); echo "SAME airport, on Hexicon's system library: the same image"
else
  fail=$((fail + 1)); echo "DIFF airport, on Hexicon's system library: $(head -1 "$out/airport-own/dif")"
fi

# The exercises, each with a project file of its own.
for src in "$adv3lite"/docs/manual/docs/learning/*.t; do
  name=$(basename "$src" .t)
  name=${name// /_}
  d=$out/$name
  mkdir -p "$d"
  cp "$src" "$d/"
  printf -- '-pre\n-D LANGUAGE=english\n-lib system\n-lib adv3Lite\n-source "%s"\n' \
    "$(basename "$src" .t)" > "$d/game.t3m"
  compare "$d" game.t3m "$name"
done
echo "same=$pass different=$fail"
[ "$fail" = 0 ]
