#!/bin/bash
# Build the games with Hexicon and with the reference compiler, each
# from its own project file, and compare the images and the messages.
#
# usage: game-compare.sh [game...]
#
# The games are airport (adv3Lite's tutorial game) and ditch3 (Return to
# Ditch Day, on adv3). Each is copied to build/game-compare/<game>, and
# its project file changed in three ways for the comparison: the
# resources are left out (-res), as Hexicon does not bundle them yet;
# the program is not run before the image is written (-nopre), as
# Hexicon does not do that yet; and the image is named by this script.
#
# Environment:
#   HEXICON      the hexicon executable (default: target/release/hexicon)
#   TADS_RUNNER  the reference's sources (default: ../_tads/tads-runner)
#   AIRPORT_SRC  Airport's sources (default: build/corpus/airport)
#   ADV3LITE     the adv3Lite library (default: build/corpus/adv3lite)
#   DITCH3_SRC   Return to Ditch Day's sources
#                (default: ../_tads/return-to-ditch-day-src)

root=$(cd "$(dirname "$0")/../.." && pwd)
runner=${TADS_RUNNER:-$root/../_tads/tads-runner}
hex=${HEXICON:-$root/target/release/hexicon}
ref=$root/build/reference/bin/t3make.exe
out=$root/build/game-compare
native() { cygpath -w "$1" 2>/dev/null || echo "$1"; }
LIB=$(native "$runner/tads3/lib")
INC=$(native "$runner/tads3/include")
games=("$@")
[ ${#games[@]} = 0 ] && games=(airport ditch3)
pass=0
fail=0
for game in "${games[@]}"; do
  extra=()
  case "$game" in
    airport)
      src=${AIRPORT_SRC:-$root/build/corpus/airport}
      extra=(-Fs "$(native "${ADV3LITE:-$root/build/corpus/adv3lite}")");;
    ditch3) src=${DITCH3_SRC:-$root/../_tads/return-to-ditch-day-src};;
    *) echo "unknown game: $game"; fail=$((fail + 1)); continue;;
  esac
  if [ ! -f "$src/$game.t3m" ]; then
    echo "SKIP $game (no project file at $src)"
    continue
  fi
  d=$out/$game
  rm -rf "$d"
  mkdir -p "$d/obj"
  cp "$src"/*.t "$d/"
  cp "$src"/*.h "$d/" 2>/dev/null
  sed -e '/^-res/,$d' -e 's/^-pre$/-nopre/' -e '/^-o /d' "$src/$game.t3m" > "$d/$game.t3m"
  ( cd "$d" && timeout 600 "$ref" -nobanner -a -FL "$LIB" -FI "$INC" "${extra[@]}" \
      -o ref.t3 -f "$game.t3m" ) 2>&1 < /dev/null | tr -d '\r' \
    | grep -v -e $'^\t' -e '^Errors: ' -e '^Warnings: ' > "$d/ref.msg"
  ( cd "$d" && timeout 600 "$hex" build -cs cp1252 -FL "$LIB" -FI "$INC" "${extra[@]}" \
      -o hex.t3 --unspecified hex.skip -f "$game.t3m" ) 2>&1 < /dev/null | tr -d '\r' \
    | grep -v $'^\t' > "$d/hex.msg"
  if ! diff "$d/hex.msg" "$d/ref.msg" > "$d/msgdif"; then
    fail=$((fail + 1))
    echo "DIFF $game: messages ($(grep -c '^[<>]' "$d/msgdif") lines)"
  elif [ ! -f "$d/ref.t3" ] || [ ! -f "$d/hex.t3" ]; then
    fail=$((fail + 1))
    echo "DIFF $game: an image is missing"
  elif python "$root/tools/image-diff.py" "$d/hex.t3" "$d/ref.t3" "$d/hex.skip" > "$d/dif"; then
    pass=$((pass + 1))
    echo "SAME $game ($(wc -c < "$d/hex.t3") bytes)"
  else
    fail=$((fail + 1))
    echo "DIFF $game: $(head -1 "$d/dif")"
  fi
done
echo "same=$pass different=$fail"
[ "$fail" = 0 ]
