#!/bin/bash
# Build Hexicon for web browsers: compile the VM to WebAssembly and
# generate the JavaScript that loads it, into web/pkg.
#
# This needs the wasm32-unknown-unknown target (rustup target add
# wasm32-unknown-unknown) and the wasm-bindgen command-line tool, in the
# same version as the wasm-bindgen crate in Cargo.lock (cargo install
# wasm-bindgen-cli --version <version>).
#
# On Windows, run this from Git Bash, or run build-web.ps1 in PowerShell.
#
# usage: build-web.sh [target]
#
# The target is what the JavaScript is for: "web" (the default) for a page
# to load as a module, or "nodejs" for running under Node, which puts the
# result in build/web-nodejs instead.

set -e
root=$(cd "$(dirname "$0")/.." && pwd)
cd "$root"
target=${1:-web}
# Where cargo builds: target/, unless CARGO_TARGET_DIR says elsewhere.
built=${CARGO_TARGET_DIR:-target}
out=web/pkg
[ "$target" = web ] || out=build/web-$target

cargo build --release --target wasm32-unknown-unknown -p hexicon-wasm
wasm-bindgen --target "$target" --out-dir "$out" \
  "$built/wasm32-unknown-unknown/release/hexicon_wasm.wasm"
ls -l "$out"
