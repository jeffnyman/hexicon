# Build Hexicon for web browsers: compile the VM to WebAssembly and
# generate the JavaScript that loads it, into web/pkg. This is
# build-web.sh for PowerShell; the two do the same thing.
#
# This needs the wasm32-unknown-unknown target (rustup target add
# wasm32-unknown-unknown) and the wasm-bindgen command-line tool, in the
# same version as the wasm-bindgen crate in Cargo.lock (cargo install
# wasm-bindgen-cli --version <version>).
#
# usage: build-web.ps1 [target]
#
# The target is what the JavaScript is for: "web" (the default) for a page
# to load as a module, or "nodejs" for running under Node, which puts the
# result in build/web-nodejs instead.

param([string]$Target = "web")

$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
$out = if ($Target -eq "web") { "web/pkg" } else { "build/web-$Target" }

Push-Location $root
try {
    cargo build --release --target wasm32-unknown-unknown -p hexicon-wasm
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
    # Where cargo builds: target/, unless CARGO_TARGET_DIR says elsewhere.
    $built = if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { "target" }
    wasm-bindgen --target $Target --out-dir $out `
        "$built/wasm32-unknown-unknown/release/hexicon_wasm.wasm"
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
    Get-ChildItem $out
}
finally {
    Pop-Location
}
