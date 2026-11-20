#!/usr/bin/env bash
# Build a universal (Apple Silicon + Intel) app and put it in a .dmg. Run on a Mac.
#   rustup target add aarch64-apple-darwin x86_64-apple-darwin
#   scripts/package-macos.sh
# Output: dist/WeeklyScheduler-<version>-mac.dmg
set -euo pipefail
cd "$(dirname "$0")/.."
VERSION="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)"
APP_NAME="Weekly Assignment Scheduler"
export MACOSX_DEPLOYMENT_TARGET=11.0

if [ "${SKIP_BUILD:-}" != 1 ]; then
    cargo build --release --target aarch64-apple-darwin
    cargo build --release --target x86_64-apple-darwin
fi

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT
APP="$WORK/dmg/$APP_NAME.app"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources" dist
lipo -create -output "$APP/Contents/MacOS/WeeklyScheduler" \
    target/aarch64-apple-darwin/release/WeeklyScheduler \
    target/x86_64-apple-darwin/release/WeeklyScheduler
sed "s/@VERSION@/$VERSION/g" packaging/macos/Info.plist > "$APP/Contents/Info.plist"

# icon.icns from the 1024 px picture
SET="$WORK/icon.iconset"
mkdir -p "$SET"
for n in 16 32 128 256 512; do
    sips -z $n $n packaging/macos/icon-1024.png --out "$SET/icon_${n}x${n}.png" >/dev/null
    sips -z $((n * 2)) $((n * 2)) packaging/macos/icon-1024.png --out "$SET/icon_${n}x${n}@2x.png" >/dev/null
done
iconutil -c icns "$SET" -o "$APP/Contents/Resources/icon.icns"

# Not notarized yet: an ad-hoc signature keeps Apple Silicon happy.
codesign --force --deep --sign - "$APP"
codesign --verify --deep --strict "$APP"

cp packaging/macos/README.txt "$WORK/dmg/Read me.txt"
ln -s /Applications "$WORK/dmg/Applications"
rm -f "dist/WeeklyScheduler-$VERSION-mac.dmg"
hdiutil create -volname "$APP_NAME" -srcfolder "$WORK/dmg" -ov -format UDZO "dist/WeeklyScheduler-$VERSION-mac.dmg" >/dev/null
ls -la dist/
