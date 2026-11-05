#!/usr/bin/env bash
# Runs inside the padd-ci container: tests on Linux, then release builds
# for Linux arm64, Linux x86_64 and Windows x86_64.
set -euo pipefail
cd /src
echo "::: linux tests"
cargo test --release --locked -q
for t in aarch64-unknown-linux-gnu x86_64-unknown-linux-gnu x86_64-pc-windows-gnu; do
    echo "::: build $t"
    cargo build --release --locked --target "$t"
done
mkdir -p /out
cp "$CARGO_TARGET_DIR/aarch64-unknown-linux-gnu/release/padd" /out/padd-linux-arm64
cp "$CARGO_TARGET_DIR/x86_64-unknown-linux-gnu/release/padd" /out/padd-linux-x86_64
cp "$CARGO_TARGET_DIR/x86_64-pc-windows-gnu/release/padd.exe" /out/padd-windows-x86_64.exe
ls -l /out
