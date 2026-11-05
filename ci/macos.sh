#!/usr/bin/env bash
# Runs natively on the Mac mini: lint, test, and build a universal Padd.app.
set -euo pipefail
export PATH="$HOME/.cargo/bin:$PATH" CARGO_TERM_COLOR=never
export CARGO_TARGET_DIR="$HOME/ci/padd/target-macos" MACOSX_DEPLOYMENT_TARGET=11.0
cd "$(dirname "$0")/.."
OUT="$HOME/ci/padd/out"; mkdir -p "$OUT"
VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)

rustup target add aarch64-apple-darwin x86_64-apple-darwin >/dev/null 2>&1
echo "::: fmt";    cargo fmt --check
echo "::: clippy"; cargo clippy --release --locked -q -- -D warnings
echo "::: tests";  cargo test --release --locked -q
for t in aarch64-apple-darwin x86_64-apple-darwin; do
    echo "::: build $t"
    cargo build --release --locked --target "$t"
done

echo "::: bundle"
APP="$OUT/Padd.app"
rm -rf "$APP"; mkdir -p "$APP/Contents/MacOS"
lipo -create -output "$APP/Contents/MacOS/padd" \
    "$CARGO_TARGET_DIR/aarch64-apple-darwin/release/padd" \
    "$CARGO_TARGET_DIR/x86_64-apple-darwin/release/padd"
sed "s/@VERSION@/$VERSION/g" ci/Info.plist > "$APP/Contents/Info.plist"
mkdir -p "$APP/Contents/Resources"
ICONSET=$(mktemp -d)/padd.iconset; mkdir -p "$ICONSET"
for s in 16 32 128 256 512; do
    sips -z $s $s assets/padd-1024.png --out "$ICONSET/icon_${s}x${s}.png" >/dev/null
    sips -z $((s*2)) $((s*2)) assets/padd-1024.png --out "$ICONSET/icon_${s}x${s}@2x.png" >/dev/null
done
iconutil -c icns "$ICONSET" -o "$APP/Contents/Resources/padd.icns"
codesign --force --sign - "$APP"   # ad-hoc signature (required on Apple Silicon)
rm -f "$OUT/padd-macos-universal.zip"
(cd "$OUT" && ditto -c -k --keepParent Padd.app padd-macos-universal.zip)
lipo -info "$APP/Contents/MacOS/padd"
ls -l "$OUT"
