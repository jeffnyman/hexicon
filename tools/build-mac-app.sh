#!/bin/bash
# Build Hexicon's player for macOS as an app: build/mac/Hexicon.app, which
# opens .hxg files from the Finder, and build/mac/Hexicon-<version>.dmg, a
# disk image with the app and a link to Applications, to hand to others.
#
# This needs what tools/build-app.sh needs, on a Mac, with Xcode's command
# line tools. UNIVERSAL=1 builds one program for Apple silicon and for
# Intel Macs both, which needs both targets:
#
#   rustup target add aarch64-apple-darwin x86_64-apple-darwin
#
# The app is signed for this Mac alone ("ad hoc"), which is enough to run
# it here. A Mac that downloads it will refuse it until it is signed with
# a Developer ID and notarized by Apple, which is for a release.
#
# usage: build-mac-app.sh

set -e
root=$(cd "$(dirname "$0")/.." && pwd)
cd "$root"

if [ "$(uname)" != Darwin ]; then
    echo "build-mac-app.sh: this builds a Mac app, on a Mac" >&2
    exit 1
fi

tools/build-web.sh
target=${CARGO_TARGET_DIR:-target}
if [ "${UNIVERSAL:-}" = 1 ]; then
    cargo build --release -p hexicon-app --target aarch64-apple-darwin
    cargo build --release -p hexicon-app --target x86_64-apple-darwin
    program=$(mktemp -d)/hexicon-player
    lipo -create -output "$program" \
        "$target/aarch64-apple-darwin/release/hexicon-player" \
        "$target/x86_64-apple-darwin/release/hexicon-player"
else
    cargo build --release -p hexicon-app
    program=$target/release/hexicon-player
fi

version=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
app=build/mac/Hexicon.app
rm -rf "$app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
cp "$program" "$app/Contents/MacOS/hexicon-player"
cp crates/hexicon-app/icon/hexicon.icns "$app/Contents/Resources/"
sed "s/@VERSION@/$version/g" tools/mac/Info.plist > "$app/Contents/Info.plist"
printf 'APPL????' > "$app/Contents/PkgInfo"
plutil -lint "$app/Contents/Info.plist"
codesign --force --sign - "$app"

# The disk image: the app, and Applications to drag it to.
staging=$(mktemp -d)
cp -R "$app" "$staging/"
ln -s /Applications "$staging/Applications"
image=build/mac/Hexicon-$version.dmg
rm -f "$image"
hdiutil create -quiet -volname Hexicon -srcfolder "$staging" -format UDZO "$image"
rm -rf "$staging"

ls -ld "$app" "$image"
lipo -archs "$app/Contents/MacOS/hexicon-player"
