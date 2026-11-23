#!/bin/bash
# Build adv3Lite's games on adv3Lite and on Hexad (lib/hexad), and check
# that the images are the same: Airport, from its project file, and the
# exercises of adv3Lite's manual (Learning TADS 3 with adv3Lite), each a
# game in one file. Both are built by Hexicon, preinitialized.
#
# On Windows, run this from Git Bash.
#
# usage: hexad-compare.sh
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
LITE=$(native "$adv3lite")
HEXAD=$(native "$root/lib")

rm -rf "$out"
mkdir -p "$out"
pass=0
fail=0

# Build the game in folder $1 from the project file $2 on adv3Lite
# (lite.t3) and on Hexad (hexad.t3), and compare the two.
compare() {
  local d=$1 t3m=$2 label=$3
  sed -e '/^-o /d' "$d/$t3m" > "$d/lite.t3m"
  sed -e '/^-o /d' -e 's/^-lib adv3Lite$/-lib hexad/' "$d/$t3m" > "$d/hexad.t3m"
  ( cd "$d" && "$hex" build -cs cp1252 -FL "$LIB" -FI "$INC" -Fs "$LITE" \
      -o lite.t3 --unspecified lite.skip -f lite.t3m ) 2>&1 < /dev/null | tr -d '\r' \
    | grep -v $'^\t' > "$d/lite.msg"
  ( cd "$d" && "$hex" build -cs cp1252 -FL "$LIB" -FI "$INC" -Fs "$HEXAD" \
      -o hexad.t3 -f hexad.t3m ) 2>&1 < /dev/null | tr -d '\r' \
    | grep -v $'^\t' > "$d/hexad.msg"
  if ! diff "$d/lite.msg" "$d/hexad.msg" > "$d/msgdif"; then
    fail=$((fail + 1)); echo "DIFF $label: messages"
  elif [ ! -f "$d/lite.t3" ] || [ ! -f "$d/hexad.t3" ]; then
    fail=$((fail + 1)); echo "DIFF $label: an image is missing"
  elif python "$root/tools/image-diff.py" "$d/hexad.t3" "$d/lite.t3" "$d/lite.skip" > "$d/dif"; then
    pass=$((pass + 1)); echo "SAME $label ($(wc -c < "$d/hexad.t3") bytes)"
  else
    fail=$((fail + 1)); echo "DIFF $label: $(head -1 "$d/dif")"
  fi
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
