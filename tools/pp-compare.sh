#!/bin/bash
# Preprocess source files with Hexicon and with the reference compiler's
# test program, and report the files whose output differs. With --tokens,
# compare the tokens the two tokenizers find instead of the preprocessed
# text.
#
# usage: pp-compare.sh [--tokens] [-I dir]... file...
#
# The reference's system headers are always on the include path. A file's
# own folder is searched for the files it includes in quotes. The outputs
# and their differences are left in build/pp-compare.
#
# TADS_RUNNER says where the reference's sources are; the default is
# ../_tads/tads-runner beside the repository.

root=$(cd "$(dirname "$0")/.." && pwd)
runner=${TADS_RUNNER:-$root/../_tads/tads-runner}
ref=$root/build/reference/bin/test_tok.exe
ref_mode=(-P)
hex_mode=preprocess
if [ "$1" = --tokens ]; then
  # A driver of our own, built against the reference's sources.
  ref=$root/build/reference/bin/tok_dump.exe
  ref_mode=()
  hex_mode=tokens
  shift
fi
hex=$root/target/release/hexicon
out=$root/build/pp-compare
mkdir -p "$out"

native() { if command -v cygpath > /dev/null; then cygpath -w "$1"; else echo "$1"; fi; }

inc=("-I" "$(native "$runner/tads3/include")")
while [ "$1" = -I ]; do
  inc+=("-I" "$(native "$2")")
  shift 2
done

# The reference's test program defines these before it starts.
defs=(-D _MSC_VER=1100 -D _WIN32=1 -D _M_IX86=500 -D __STDC__=0
      -D _INTEGRAL_MAX_BITS=64 -D __cplusplus=1)

pass=0
fail=0
for file in "$@"; do
  name=$(echo "$file" | sed 's|[/\\:]|_|g')
  src=$(native "$file")
  ref_inc=()
  for ((i = 0; i < ${#inc[@]}; i += 2)); do ref_inc+=("-I${inc[i + 1]}"); done
  timeout 60 "$ref" "${ref_inc[@]}" "${ref_mode[@]}" "$src" 2>&1 < /dev/null | tr -d '\r' > "$out/$name.ref"
  timeout 60 "$hex" $hex_mode --test "${inc[@]}" "${defs[@]}" "$src" 2> "$out/$name.err" \
    | tr -d '\r' > "$out/$name.hex"
  if diff "$out/$name.hex" "$out/$name.ref" > "$out/$name.dif"; then
    pass=$((pass + 1))
    rm -f "$out/$name.dif"
  else
    fail=$((fail + 1))
    echo "DIFF $file ($(grep -c '^[<>]' "$out/$name.dif") lines)"
  fi
done
echo "same=$pass different=$fail"
