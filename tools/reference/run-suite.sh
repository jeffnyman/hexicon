#!/bin/bash
# Run the original TADS 3 test suite against the reference toolchain built by
# build.bat, and compare the output with the reference logs.
#
# This is a port of tads3/win32/test/long.bat and its helper scripts. The
# list of tests is read from long.bat itself, so it stays in step with the
# suite. The suite is used in place from the tads-sources checkout; it is
# copied into the work directory so the checkout is never written to.
#
# usage: run-suite.sh [work-dir]
#
# Environment:
#   TADS_SOURCES  checkout of github.com/tajmone/tads-sources
#   TADS_RUNNER   checkout of github.com/captain-mayhem/tads-runner
#   REFERENCE_BIN directory holding the reference executables

here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$here/../.." && pwd)
TADS_SOURCES=${TADS_SOURCES:-$root/../_tads/tads-sources}
TADS_RUNNER=${TADS_RUNNER:-$root/../_tads/tads-runner}
BIN=${REFERENCE_BIN:-$root/build/reference/bin}
W=${1:-$root/build/reference-suite}

SRC=$TADS_SOURCES/tads3
for need in "$SRC/win32/test/long.bat" "$TADS_RUNNER/tads3/lib" "$BIN/test_exec.exe"; do
  [ -e "$need" ] || { echo "missing: $need" >&2; exit 2; }
done

rm -rf "$W"
mkdir -p "$W/test" || exit 2
cp -r "$SRC/test/data" "$SRC/test/log" "$W/test/" || exit 2
mkdir "$W/test/out" || exit 2
cd "$W" || exit 2

# Neither checkout has the suite intact. tads-sources has the newest tests,
# but its text files were re-encoded to UTF-8 and stripped of trailing
# whitespace, and its binary and input fixtures are missing. tads-runner has
# an older suite with the original bytes. Start from tads-sources, then take
# tads-runner's copy of any file that is missing, or that differs only by
# encoding and whitespace.
overlay() { # subdirectory
  local from="$TADS_RUNNER/tads3/test/$1" to="test/$1" f name
  [ -d "$from" ] || return
  for f in "$from"/*; do
    [ -f "$f" ] || continue
    name=$(basename "$f")
    if [ ! -e "$to/$name" ]; then
      cp "$f" "$to/$name"
    elif ! cmp -s "$f" "$to/$name" && cmp -s <(loose < "$f") <(loose < "$to/$name"); then
      cp "$f" "$to/$name"
    fi
  done
}
loose() { perl -0pe 's/[\x80-\xff]+/?/g; s/[ \t\r]+(?=\n|\z)//g; s/\n*\z/\n/'; }
overlay data
overlay log

# Two fixtures are not intact in any known copy of the suite, but the
# expected logs pin down what they must have been.
#
# hashes.src survives only with LF line endings. The expected MD5 and
# SHA-256 values are those of the same text with CRLF line endings.
if [ -f test/data/hashes.src ] && ! grep -q $'\r' test/data/hashes.src; then
  perl -pi -e 's/\n/\r\n/' test/data/hashes.src
fi
# The three files the directory test copies are missing. The test only
# lists them by name and size, so stand-ins of the logged sizes will do.
standin() { # name size
  [ -e "test/data/$1" ] || head -c "$2" /dev/zero | tr '\0' 'x' > "test/data/$1"
}
standin dirtest-file-a.txt 2250
standin dirtest-file-b.txt 1281
standin dirtest-file-c.txt 1087

# The original scripts pass absolute native paths; do the same.
native() { cygpath -w "$1" 2>/dev/null || echo "$1"; }
ROOT=$(native "$W/test")
DAT=$(native "$W/test/data")
OUT=$(native "$W/test/out")
INC=$(native "$TADS_RUNNER/tads3/include")
LIB=$(native "$TADS_RUNNER/tads3/lib")
out=test/out
log=test/log

pass=0; fail=0; skip=0
: > results.txt

# Compare text logs ignoring carriage returns, the number of newlines at the
# end of the file, and the heap and hash table reports that only a debug
# build of the test programs prints. Binary files are compared exactly.
norm() {
  tr -d '\r' | perl -0pe '
    s/\n\(T3VM\) Memory blocks still in use:\n\nTotal blocks in use: 0\n//g;
    s/\n*hash table: total[^\n]*\nnumber of buckets over[^\n]*\nnumber of empty buckets[^\n]*\n/\n/g;
    s/\n*\z/\n/'
}
tdiff() { # test-name [file-name]
  local name=$1 f=${2:-$1.log} ok
  if [ ! -f "$out/$f" ]; then
    echo "FAIL $name (no output)" >> results.txt; fail=$((fail+1)); return
  fi
  if [ "${f##*.}" = bin ]; then
    cmp -s "$out/$f" "$log/$f"; ok=$?
  else
    diff <(norm < "$out/$f") <(norm < "$log/$f") > "$out/$name.dif" 2>&1; ok=$?
  fi
  if [ $ok = 0 ]; then
    echo "PASS $name" >> results.txt; pass=$((pass+1)); rm -f "$out/$name.dif"
  else
    echo "FAIL $name" >> results.txt; fail=$((fail+1))
  fi
}
skipt() { echo "SKIP $*" >> results.txt; skip=$((skip+1)); }

# Run a program with a time limit, appending its output to a log.
run() { # log-name program args...
  local l=$1 prog=$2; shift 2
  timeout 300 "$BIN/$prog.exe" "$@" >> "$out/$l.log" 2>&1 < /dev/null
}

make_image() { # log-name args...
  local l=$1; shift
  : > "$out/$l.log"
  run "$l" t3make -test -I"$DAT" -I"$INC" -FL "$LIB" -FI "$INC" -a -nobanner \
    -Fs "$DAT" -Fs "$LIB" -Fo "$OUT" -Fy "$OUT" "$@"
}

testpp() {
  : > "$out/$1.log"
  run "$1" test_tok -I"$INC" -I"$DAT" -P "$DAT\\$1.c"
  tdiff "$1"
}
testexec() {
  local cp=cp437
  [ "$1" = -cp ] && { cp=$2; shift 2; }
  : > "$out/$1.log"
  run "$1" test_prs_top -I"$INC" "$DAT\\$1.t" "$OUT\\$1.t3"
  run "$1" test_exec -cs $cp -norand "$OUT\\$1.t3"
  tdiff "$1"
}
testmake() {
  local cp=cp437 dorun=1 script=() mode=() pre=(-nopre)
  [ "$1" = -cp ] && { cp=$2; shift 2; }
  [ "$1" = -norun ] && { dorun=0; shift; }
  local scripted=0
  [ "$1" = -script ] && { scripted=1; shift; }
  case "$1" in
    -nodef) mode=(-nodef); shift;;
    -debug) mode=(-d); shift;;
    -pre) pre=(); shift;;
  esac
  local n=$1; shift
  [ $scripted = 1 ] && script=(-I "$DAT\\$n.in")
  make_image "$n" "${mode[@]}" "${pre[@]}" -o "$OUT\\$n.t3" "$@"
  [ $dorun = 1 ] && run "$n" test_exec -sd "$ROOT" "${script[@]}" -cs $cp -norand "$OUT\\$n.t3"
  tdiff "$n"
}
testrun() {
  local l=$1 img=$2; shift 2
  : > "$out/$l.log"
  run "$l" test_exec -sd "$ROOT" -cs cp437 -norand "$OUT\\$img.t3" "$@"
  tdiff "$l"
}
testrestore() {
  : > "$out/$1.log"
  run "$1" test_exec -cs cp437 -norand "$OUT\\$2.t3" restore
  tdiff "$1"
}
testpre() {
  : > "$out/$1.log"
  run "$1" test_prs_top -I"$INC" "$DAT\\$1.t" "$OUT\\$1.t3"
  run "$1" t3pre "$OUT\\$1.t3" "$OUT\\$1_pre.t3"
  run "$1" test_exec -cs cp437 -norand "$OUT\\$1_pre.t3"
  tdiff "$1"
}
testpack() {
  testmake -cp latin1 -pre "$1" "$1"
  tdiff "$1-bin" "$1.bin"
}
testres() {
  local n=$1 src=()
  while [ $# -gt 0 ] && [ "$1" != -res ]; do src+=("$1"); shift; done
  shift
  make_image "$n" -nopre -o "$OUT\\$n.t3" "${src[@]}"
  run "$n" t3res "$OUT\\$n.t3" "$@"
  run "$n" test_exec -cs cp437 -norand "$OUT\\$n.t3"
  tdiff "$n"
}
testsample() {
  local dir="$TADS_RUNNER/tads3/samples"
  [ "$1" = /Adv3 ] && shift
  local n=$1
  if ! cp "$dir/$n.t" "$out/$n.t" 2>/dev/null; then skipt "testsample $n (no sample source)"; return; fi
  : > "$out/$n.log"
  run "$n" t3make -test -FL "$LIB" -FI "$INC" -a -nobanner -DLANGUAGE=en_us -DMESSAGESTYLE=neu \
    -nopre -Fo "$OUT" -Fy "$OUT" -o "$OUT\\$n.t3" -lib system.tl -lib adv3/adv3.tl -source "$OUT\\$n.t"
  run "$n" t3run -plain -norand -csl us-ascii -i "$DAT\\$n.in" -l"$OUT\\${n}_run.log" "$OUT\\$n.t3" -norandomize
  tdiff "$n"
  tdiff "${n}_run"
}

# Return to Ditch Day walkthrough. The original harness lived in the game's
# own project directory and is not available, so this is a reconstruction:
# the game is built from source in QA test mode, and the command script is
# recovered from the prompts in the expected log. Only the play transcript
# is compared; the build listing differs because the source distribution
# has no gameinfo.txt resource to bundle.
testditch() { # script-name log-name
  local l=$2 src=${DITCH3_SRC:-$root/../_tads/return-to-ditch-day-src}
  if [ ! -f "$src/ditch3.t3m" ]; then skipt "testditch $* (no game source at $src)"; return; fi
  local d="$W/ditch3"
  mkdir -p "$d/obj" && cp "$src"/*.t "$src"/*.h "$d/" || { skipt "testditch $* (copy failed)"; return; }
  sed '/^-res/,$d' "$src/ditch3.t3m" > "$d/ditch3.t3m"
  # Every line of the log that starts with ">" is a command prompt, except
  # one line of game text.
  tr -d '\r' < "$log/$l.log" | grep '^>' | grep -v '^>>>' > "$d/walkthru.in"
  ( cd "$d" && timeout 600 "$BIN/t3make.exe" -test -a -nobanner -FL "$LIB" -FI "$INC" \
      -D QA_TEST_MODE -f ditch3.t3m > build.log 2>&1 < /dev/null )
  ( cd "$d" && timeout 600 "$BIN/test_exec.exe" -I walkthru.in -cs cp437 -norand ditch3.t3 \
      > run.log 2>&1 < /dev/null )
  # Drop the build listing from the expected log, and the form feeds the
  # stdio console prints for a screen clear, which the log does not have.
  if diff <(norm < "$d/run.log" | perl -0pe 's/\f\n//g; s/\A\n+//') \
          <(norm < "$log/$l.log" | perl -0pe 's/\A(?:\t[^\n]*\n)+//; s/\A\n+//') > "$out/$l.dif" 2>&1; then
    echo "PASS $l" >> results.txt; pass=$((pass+1)); rm -f "$out/$l.dif"
  else
    echo "FAIL $l" >> results.txt; fail=$((fail+1))
  fi
}

# The object test produces no log; it only has to run without crashing.
if timeout 300 "$BIN/test_obj.exe" > "$out/test_obj.out" 2>&1 < /dev/null; then
  echo "PASS test_obj" >> results.txt; pass=$((pass+1))
else
  echo "FAIL test_obj (exit $?)" >> results.txt; fail=$((fail+1))
fi

grep '^call %tstbat%' "$SRC/win32/test/long.bat" | tr -d '\r' | sed 's|^call %tstbat%.||' > commands.txt
while read -r cmd args; do
  # Arguments in long.bat are plain words, some with backslashes or quotes.
  eval "set -- ${args//\\/\\\\}"
  case $cmd in
    testpp|testexec|testmake|testrun|testrestore|testpre|testpack|testres|testsample|testditch) $cmd "$@";;
    *) skipt "$cmd $*";;
  esac
done < commands.txt

echo "pass=$pass fail=$fail skip=$skip"
grep -v '^PASS' results.txt
[ $fail = 0 ]
