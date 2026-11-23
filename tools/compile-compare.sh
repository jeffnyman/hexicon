#!/bin/bash
# Compile source files with Hexicon and with the reference compiler, and
# report the files on which the two differ.
#
# On Windows, run this from Git Bash.
#
# usage: compile-compare.sh [-I dir]... [--lib] [--debug] file...
#
# Each file is compiled as a whole program on its own, without the
# reference's default modules, so it needs a _main function. With
# --lib, the system library's _main comes first, as it does for the
# reference by default, and libraries such as system.tl are found in
# the system library. With --debug, both build for debugging (-d). A
# folder in place of a file is a program of
# several source files: the .t files in it, put together in the order
# of their names; or, if it has a project file (.t3m), what that says.
# The two
# agree on a file if they report the same problems and, when there are
# no errors, write the same image. The images and what each compiler
# said are left in build/compile-compare.
#
# TADS_RUNNER says where the reference's sources are; the default is
# ../_tads/tads-runner beside the repository. HEXICON says which
# Hexicon to use, and COMPARE_OUT where to leave things, so that two
# comparisons can run at once.

root=$(cd "$(dirname "$0")/.." && pwd)
runner=${TADS_RUNNER:-$root/../_tads/tads-runner}
ref=$root/build/reference/bin/t3make.exe
hex=${HEXICON:-$root/target/release/hexicon}
out=${COMPARE_OUT:-$root/build/compile-compare}
mkdir -p "$out/obj"
tab=$(printf '\t')
# The reference takes a source file that does not say what character set
# it is in to be in the system's own. This is the one on Windows set up
# for English; set CHARSET for another.
charset=${CHARSET:-cp1252}

native() { if command -v cygpath > /dev/null; then cygpath -w "$1"; else echo "$1"; fi; }

inc=("-I" "$(native "$runner/tads3/include")")
while [ "$1" = -I ]; do
  inc+=("-I" "$(native "$2")")
  shift 2
done
# Where the system library and its headers are, for both compilers.
sys=("-FL" "$(native "$runner/tads3/lib")" "-FI" "$(native "$runner/tads3/include")")
nodef=(-nodef)
if [ "$1" = --lib ]; then
  nodef=()
  shift
fi
if [ "$1" = --debug ]; then
  nodef+=(-d)
  shift
fi

pass=0
fail=0
for file in "$@"; do
  name=$(echo "$file" | sed 's|[/\\:]|_|g')
  src=()
  if [ -d "$file" ] && ls "$file"/*.t3m > /dev/null 2>&1; then
    for part in "$file"/*.t3m; do src=(-f "$(native "$part")"); break; done
  elif [ -d "$file" ]; then
    for part in "$file"/*.t; do src+=("$(native "$part")"); done
  else
    src=("$(native "$file")")
  fi
  rm -f "$out/$name.ref.t3" "$out/$name.hex.t3" "$out"/obj/*
  timeout 60 "$ref" "${nodef[@]}" -nopre -nobanner -a "${inc[@]}" "${sys[@]}" \
    -Fo "$(native "$out/obj")" -Fy "$(native "$out/obj")" \
    -o "$(native "$out/$name.ref.t3")" "${src[@]}" > "$out/$name.ref.log" 2>&1 < /dev/null
  timeout 60 "$hex" build "${nodef[@]}" -nopre "${inc[@]}" "${sys[@]}" -cs "$charset" \
    -o "$(native "$out/$name.hex.t3")" \
    --unspecified "$(native "$out/$name.skip")" "${src[@]}" 2>&1 \
    | tr -d '\r' > "$out/$name.hex.msg"
  # What the reference says besides the problems it finds: the steps of
  # the build, each on a line starting with a tab, and the totals. A
  # message that names the object file the reference wrote gives the
  # folder it is in; Hexicon, which writes none, gives the name alone.
  tr -d '\r' < "$out/$name.ref.log" \
    | grep -v -e "^$tab" -e '^Errors: ' -e '^Warnings: ' \
    | sed 's|in object file ".*[\\/]\([^\\/"]*\.t3o\)"|in object file "\1"|' \
    > "$out/$name.ref.msg"
  if ! diff "$out/$name.hex.msg" "$out/$name.ref.msg" > "$out/$name.msgdif"; then
    fail=$((fail + 1))
    echo "DIFF $file: messages ($(grep -c '^[<>]' "$out/$name.msgdif") lines)"
  elif [ ! -f "$out/$name.ref.t3" ] && [ ! -f "$out/$name.hex.t3" ]; then
    pass=$((pass + 1))
  elif [ ! -f "$out/$name.ref.t3" ]; then
    fail=$((fail + 1))
    echo "DIFF $file: only Hexicon compiled it"
  elif [ ! -f "$out/$name.hex.t3" ]; then
    fail=$((fail + 1))
    echo "DIFF $file: only the reference compiled it"
  elif python "$root/tools/image-diff.py" "$out/$name.hex.t3" "$out/$name.ref.t3" \
      "$out/$name.skip" > "$out/$name.dif"; then
    pass=$((pass + 1))
    rm -f "$out/$name.dif"
  else
    fail=$((fail + 1))
    echo "DIFF $file: $(head -1 "$out/$name.dif")"
  fi
done
echo "same=$pass different=$fail"
[ "$fail" = 0 ]
