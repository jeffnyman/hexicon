#!/bin/bash
# Run the programs of the TADS 3 test suite under Hexicon and compare the
# output with what the reference interpreter prints for the same image.
#
# This needs the work directory that tools/reference/run-suite.sh leaves
# behind: it holds the compiled images and the list of tests.
#
# usage: run.sh [test-name...]
#
# With test names, only those tests run. Results go to build/conformance:
# results.txt lists each test as PASS, FAIL, or SKIP, and for each test there is
# the expected output, the actual output, and their differences.
#
# Environment:
#   HEXICON        the hexicon executable (default: target/release/hexicon)
#   REFERENCE_BIN  directory holding the reference executables
#   SUITE          the suite work directory (default: build/reference-suite)

here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$here/../.." && pwd)
HEXICON=${HEXICON:-$root/target/release/hexicon}
BIN=${REFERENCE_BIN:-$root/build/reference/bin}
W=${SUITE:-$root/build/reference-suite}
R=$root/build/conformance

for need in "$HEXICON" "$HEXICON.exe"; do [ -x "$need" ] && HEXICON=$need && break; done
for need in "$HEXICON" "$BIN/test_exec.exe" "$W/commands.txt"; do
  [ -e "$need" ] || { echo "missing: $need" >&2; exit 2; }
done

only=" $* "
mkdir -p "$R" || exit 2
cd "$W" || exit 2

native() { cygpath -w "$1" 2>/dev/null || echo "$1"; }
ROOT=$(native "$W/test")
DAT=$(native "$W/test/data")
OUT=$(native "$W/test/out")

pass=0; fail=0; skip=0
: > "$R/results.txt"

# Compare text ignoring carriage returns, the number of newlines at the end,
# and the heap report that only a debug build of the reference prints.
norm() {
  tr -d '\r' | perl -0pe '
    s/\n\(T3VM\) Memory blocks still in use:\n\nTotal blocks in use: 0\n//g;
    s/\n*\z/\n/'
}

# Run one image under both interpreters and compare.
check() { # test-name script-or-empty image args...
  local name=$1 script=$2 image=$3; shift 3
  [ "$only" = "  " ] || [[ "$only" == *" $name "* ]] || return
  # Some tests only check that the compiler rejects a program.
  if [ ! -f "test/out/$image.t3" ]; then
    echo "SKIP $name (no image)" >> "$R/results.txt"; skip=$((skip+1)); return
  fi
  local ref=() hex=()
  if [ -n "$script" ]; then ref=(-I "$script"); hex=(-I "$script"); fi
  timeout 120 "$BIN/test_exec.exe" -sd "$ROOT" "${ref[@]}" -cs utf8 -norand \
    "$OUT\\$image.t3" "$@" < /dev/null 2>&1 | norm > "$R/$name.expected"
  timeout 60 "$HEXICON" run --norand --sandbox "$ROOT" "${hex[@]}" "$OUT\\$image.t3" "$@" \
    < /dev/null 2>&1 \
    | norm > "$R/$name.actual"
  if diff "$R/$name.actual" "$R/$name.expected" > "$R/$name.dif" 2>&1; then
    echo "PASS $name" >> "$R/results.txt"; pass=$((pass+1)); rm -f "$R/$name.dif"
  else
    local why
    why=$(grep -o 'not implemented in Hexicon: .*' "$R/$name.actual" | head -1)
    echo "FAIL $name${why:+ ($why)}" >> "$R/results.txt"; fail=$((fail+1))
  fi
}

testexec() {
  [ "$1" = -cp ] && shift 2
  check "$1" "" "$1"
}
testmake() {
  local scripted=0
  [ "$1" = -cp ] && shift 2
  [ "$1" = -norun ] && return
  [ "$1" = -script ] && { scripted=1; shift; }
  case "$1" in -nodef|-debug|-pre) shift;; esac
  if [ $scripted = 1 ]; then check "$1" "$DAT\\$1.in" "$1"; else check "$1" "" "$1"; fi
}
testrun() {
  local l=$1 img=$2; shift 2
  check "$l" "" "$img" "$@"
}
testrestore() { check "$1" "" "$2" restore; }
testpre() { check "$1" "" "$1_pre"; }
testpack() { testmake -cp latin1 -pre "$1" "$1"; }
testres() { check "$1" "" "$1"; }

while read -r cmd args; do
  eval "set -- ${args//\\/\\\\}"
  case $cmd in
    testexec|testmake|testrun|testrestore|testpre|testpack|testres) $cmd "$@";;
  esac
done < commands.txt

echo "pass=$pass fail=$fail skip=$skip"
[ $fail = 0 ]
