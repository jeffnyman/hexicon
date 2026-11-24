#!/bin/bash
# Convert the libraries and the games to Hexicon with `hexicon convert`,
# and check the round trip: each game, built from its converted sources
# and converted libraries on Hexicon's own system library (the one built
# in), has to play the same as the original built on TADS 3's. Both are
# built by Hexicon, and preinitialized.
#
# On Windows, run this from Git Bash.
#
# usage: convert-compare.sh
#
# The games are Airport, on adv3Lite, and Return to Ditch Day, on adv3:
# Ditch Day through its walkthrough, and Airport through a short script.
# The images are not compared: the two system libraries are not the
# same, and nor are the names the programs use.
#
# Environment:
#   HEXICON      the hexicon executable (default: target/release/hexicon)
#   TADS_RUNNER  the reference's sources (default: ../_tads/tads-runner)
#   AIRPORT_SRC  Airport's sources (default: build/corpus/airport)
#   ADV3LITE     the adv3Lite library (default: build/corpus/adv3lite)
#   DITCH3_SRC   Return to Ditch Day's sources
#                (default: ../_tads/return-to-ditch-day-src)
#   DITCH3_WALK  its walkthrough, as a command script
#                (default: ../hexicon-playground/scripts/DitchDay.walkthrough.txt)

root=$(cd "$(dirname "$0")/../.." && pwd)
runner=${TADS_RUNNER:-$root/../_tads/tads-runner}
hex=${HEXICON:-$root/target/release/hexicon}
out=$root/build/convert-compare
native() { cygpath -w "$1" 2>/dev/null || echo "$1"; }
LIB=$(native "$runner/tads3/lib")
INC=$(native "$runner/tads3/include")
adv3lite=${ADV3LITE:-$root/build/corpus/adv3lite}
walk=${DITCH3_WALK:-$root/../hexicon-playground/scripts/DitchDay.walkthrough.txt}

rm -rf "$out"
mkdir -p "$out/libs"
"$hex" convert "$runner/tads3/lib/adv3" "$out/libs/adv3" 2> "$out/adv3.convert" || exit 1
"$hex" convert "$adv3lite" "$out/libs/adv3lite" 2> "$out/adv3lite.convert" || exit 1

# Airport's script: commands that look around, move, and talk.
{
  echo '<eventscript>'
  for c in look inventory "x me" "examine all" n s e w u d "take all" "drop all" \
      "talk to man" "ask about plane" hint score "full score" wait undo again about \
      credits help "x floor" "go to gate" sit stand i verbose brief quit y; do
    echo "<line>$c"
  done
} > "$out/airport.cmd"

pass=0
fail=0
check() {
  if [ "$1" = 0 ]; then pass=$((pass + 1)); echo "SAME $2"; else fail=$((fail + 1)); echo "DIFF $2"; fi
}

for game in airport ditch3; do
  case "$game" in
    airport)
      src=${AIRPORT_SRC:-$root/build/corpus/airport}
      orig_libs=(-Fs "$(native "$adv3lite")")
      conv_libs=(-Fs "$(native "$out/libs/adv3lite")")
      script=$out/airport.cmd;;
    ditch3)
      src=${DITCH3_SRC:-$root/../_tads/return-to-ditch-day-src}
      orig_libs=()
      conv_libs=(-Fs "$(native "$out/libs")")
      script=$walk;;
  esac
  orig=$out/$game-orig
  conv=$out/$game
  mkdir -p "$orig"
  cp -r "$src"/. "$orig/"
  rm -f "$orig"/*.t3
  "$hex" convert "$src" "$conv" 2> "$out/$game.convert" || exit 1
  rm -f "$conv"/*.t3
  # The images are named here.
  sed -i '/^-o /d' "$orig/$game.t3m"
  sed -i '/^image = /d' "$conv/hexicon.toml"
  # The converted game is checked for properties that are given values
  # but never read, which the original is not, and Ditch Day has one.
  # Its warning must not stop the build that is compared; it is shown
  # below.
  sed -i 's/^warnings-as-errors = true$/warnings-as-errors = false/' "$conv/hexicon.toml"

  ( cd "$orig" && "$hex" build -cs cp1252 -FL "$LIB" -FI "$INC" "${orig_libs[@]}" \
      -o pre.t3 -f "$game.t3m" ) > "$out/$game-orig.pre.msg" 2>&1 < /dev/null
  # The converted game: no -FL or -FI, so on Hexicon's system library.
  ( cd "$conv" && "$hex" build -cs cp1252 "${conv_libs[@]}" -o pre.t3 ) \
    > "$out/$game.pre.msg" 2>&1 < /dev/null
  grep -h "nothing reads it" "$out/$game.pre.msg" | sed "s/^/  $game: /"
  if [ ! -f "$conv/pre.t3" ]; then
    fail=$((fail + 1)); echo "DIFF $game: the converted game did not build; see $out/$game.pre.msg"
    continue
  fi
  if [ ! -f "$script" ]; then
    echo "SKIP $game play (no script at $script)"
    continue
  fi
  for d in "$orig" "$conv"; do
    ( cd "$d" && timeout 900 "$hex" run --plain --norand -I "$(native "$script")" pre.t3 ) \
      > "$d/play.txt" 2>&1 < /dev/null
  done
  diff "$orig/play.txt" "$conv/play.txt" > "$out/$game.play.dif"
  check $? "$game play ($(wc -l < "$conv/play.txt") lines)"
done
echo "same=$pass different=$fail"
[ "$fail" = 0 ]
