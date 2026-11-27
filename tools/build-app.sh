#!/bin/bash
# Build Hexicon's player: the web page, and the program that shows it in a
# window, with the system's web view. The player is
# target/release/hexicon-player (hexicon-player.exe on Windows), or the
# same under CARGO_TARGET_DIR if it is set.
#
# This needs what tools/build-web.sh needs, and on Linux the WebKitGTK
# development files (webkit2gtk-4.1). On Windows, run it from Git Bash;
# WebView2 comes with Windows 10 and 11.
#
# usage: build-app.sh

set -e
root=$(cd "$(dirname "$0")/.." && pwd)
cd "$root"
tools/build-web.sh
cargo build --release -p hexicon-app
# Where cargo builds: target/, unless CARGO_TARGET_DIR says elsewhere.
ls -l "${CARGO_TARGET_DIR:-target}"/release/hexicon-player*
