#!/bin/bash
# Run preinitialization on images with Hexicon and with the reference's
# t3pre, and compare the images each writes and what each program said.
#
# On Windows, run this from Git Bash.
#
# usage: preinit-compare.sh image.t3...
#
# Each image is preinitialized in a folder of its own under
# build/preinit-compare, by both, under the same name, as t3pre gives
# the program the name of the new image as an argument.
#
# Environment:
#   HEXICON   the hexicon executable (default: target/release/hexicon)

root=$(cd "$(dirname "$0")/../.." && pwd)
hex=${HEXICON:-$root/target/release/hexicon}
ref=$root/build/reference/bin/t3pre.exe
out=$root/build/preinit-compare
pass=0
fail=0
for image in "$@"; do
  name=$(basename "$image" .t3)
  d=$out/$name
  rm -rf "$d"
  mkdir -p "$d/ref" "$d/hex"
  cp "$image" "$d/in.t3"
  ( cd "$d/ref" && timeout 300 "$ref" ../in.t3 pre.t3 ) < /dev/null 2>&1 | tr -d '\r' > "$d/ref.out"
  ( cd "$d/hex" && timeout 300 "$hex" preinit --unspecified ../hex.skip ../in.t3 pre.t3 ) \
    < /dev/null 2>&1 | tr -d '\r' > "$d/hex.out"
  if ! diff "$d/hex.out" "$d/ref.out" > "$d/outdif"; then
    fail=$((fail + 1))
    echo "DIFF $name: output ($(grep -c '^[<>]' "$d/outdif") lines)"
  elif [ ! -s "$d/ref/pre.t3" ] && [ ! -s "$d/hex/pre.t3" ]; then
    pass=$((pass + 1))
  elif [ ! -s "$d/ref/pre.t3" ] || [ ! -s "$d/hex/pre.t3" ]; then
    fail=$((fail + 1))
    echo "DIFF $name: only one wrote an image"
  elif python "$root/tools/image-diff.py" "$d/hex/pre.t3" "$d/ref/pre.t3" "$d/hex.skip" \
      > "$d/dif"; then
    pass=$((pass + 1))
  else
    fail=$((fail + 1))
    echo "DIFF $name: $(head -1 "$d/dif")"
  fi
done
echo "same=$pass different=$fail"
[ "$fail" = 0 ]
