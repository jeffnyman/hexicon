#!/bin/bash
# Build the programs of the TADS 3 test suite with Hexicon and with the
# reference compiler, as the suite builds them, and compare the images
# and the messages.
#
# usage: suite-compare.sh [test-name...]
#
# This needs the work directory that tools/reference/run-suite.sh leaves
# behind, for the list of tests and their sources. Every program is
# built without preinitialization (-nopre), which Hexicon does not do
# yet, and without the suite's -test option, which only changes how
# file names are shown. Programs built for the debugger (-d) are
# passed over.
#
# Results go to build/suite-compare: for each test, what each compiler
# said, the two images, and their differences.
#
# Environment:
#   HEXICON      the hexicon executable (default: target/release/hexicon)
#   TADS_RUNNER  the reference's sources (default: ../_tads/tads-runner)
#   SUITE        the suite work directory (default: build/reference-suite)

root=$(cd "$(dirname "$0")/../.." && pwd)
runner=${TADS_RUNNER:-$root/../_tads/tads-runner}
hex=${HEXICON:-$root/target/release/hexicon}
ref=$root/build/reference/bin/t3make.exe
W=${SUITE:-$root/build/reference-suite}
out=$root/build/suite-compare
mkdir -p "$out/obj"
native() { cygpath -w "$1" 2>/dev/null || echo "$1"; }
DAT=$(native "$W/test/data")
LIB=$(native "$runner/tads3/lib")
INC=$(native "$runner/tads3/include")
common=(-I"$DAT" -I"$INC" -FL "$LIB" -FI "$INC" -Fs "$DAT" -Fs "$LIB")
only=" $* "
pass=0
fail=0
skip=0
while read -r kind args; do
  [ "$kind" = testmake ] || continue
  set -- $args
  mode=()
  [ "$1" = -cp ] && shift 2
  [ "$1" = -norun ] && shift
  [ "$1" = -script ] && shift
  case "$1" in
    -nodef) mode=(-nodef); shift;;
    -debug) skip=$((skip + 1)); continue;;
    -pre) shift;;
  esac
  n=$1
  shift
  [ "$only" = "  " ] || [[ "$only" == *" $n "* ]] || continue
  rm -f "$out/$n.ref.t3" "$out/$n.hex.t3" "$out"/obj/*
  ( cd "$out" && timeout 120 "$ref" "${common[@]}" -a -nobanner -Fo obj -Fy obj \
      "${mode[@]}" -nopre -o "$n.ref.t3" "$@" ) 2>&1 < /dev/null | tr -d '\r' \
    | grep -v -e $'^\t' -e '^Errors: ' -e '^Warnings: ' \
    | sed 's|in object file ".*[\\/]\([^\\/"]*\.t3o\)"|in object file "\1"|' > "$out/$n.ref.msg"
  ( cd "$out" && timeout 120 "$hex" build "${common[@]}" -cs cp1252 "${mode[@]}" -nopre \
      -o "$n.hex.t3" --unspecified "$n.skip" "$@" ) 2>&1 < /dev/null | tr -d '\r' > "$out/$n.hex.msg"
  if ! diff "$out/$n.hex.msg" "$out/$n.ref.msg" > "$out/$n.msgdif"; then
    fail=$((fail + 1))
    echo "DIFF $n: messages ($(grep -c '^[<>]' "$out/$n.msgdif") lines)"
  elif [ ! -f "$out/$n.ref.t3" ] && [ ! -f "$out/$n.hex.t3" ]; then
    pass=$((pass + 1))
  elif [ ! -f "$out/$n.ref.t3" ] || [ ! -f "$out/$n.hex.t3" ]; then
    fail=$((fail + 1))
    echo "DIFF $n: only one compiler wrote an image"
  elif python "$root/tools/image-diff.py" "$out/$n.hex.t3" "$out/$n.ref.t3" "$out/$n.skip" \
      > "$out/$n.dif"; then
    pass=$((pass + 1))
    rm -f "$out/$n.dif"
  else
    fail=$((fail + 1))
    echo "DIFF $n: $(head -1 "$out/$n.dif")"
  fi
done < "$W/commands.txt"
echo "same=$pass different=$fail passed-over=$skip"
[ "$fail" = 0 ]
