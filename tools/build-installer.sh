#!/bin/bash
# Build Hexicon's installer for Windows: the player (with the web page, as
# tools/build-app.sh builds it), the compiler, and the installer around
# them, build/installer/hexicon-<version>-setup.exe.
#
# This needs what tools/build-app.sh needs, and Inno Setup 6
# (winget install JRSoftware.InnoSetup). Run it from Git Bash.
#
# usage: build-installer.sh

set -e
root=$(cd "$(dirname "$0")/.." && pwd)
cd "$root"

# Inno Setup's compiler: on the PATH, or where its installer puts it, for
# one user or for all.
iscc=$(command -v ISCC.exe || command -v iscc || true)
for candidate in \
    "${LOCALAPPDATA:-}/Programs/Inno Setup 6/ISCC.exe" \
    "/c/Program Files (x86)/Inno Setup 6/ISCC.exe" \
    "/c/Program Files/Inno Setup 6/ISCC.exe"; do
    if [ -z "$iscc" ] && [ -f "$candidate" ]; then
        iscc=$candidate
    fi
done
if [ -z "$iscc" ]; then
    echo "build-installer.sh: Inno Setup 6 is not installed (winget install JRSoftware.InnoSetup)" >&2
    exit 1
fi

tools/build-app.sh
cargo build --release -p hexicon-cli

version=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
bin=$(cygpath -w "$(cd "${CARGO_TARGET_DIR:-target}/release" && pwd)")
# Git Bash would take ISCC's /D switches for paths and change them.
MSYS2_ARG_CONV_EXCL="*" "$iscc" -Q "/DAppVersion=$version" "/DBinDir=$bin" \
    tools/installer/hexicon.iss
ls -l build/installer/hexicon-"$version"-setup.exe
