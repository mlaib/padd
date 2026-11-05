#!/usr/bin/env bash
# Padd CI, executed on the Mac mini.
#   macOS (native):  fmt, clippy -D warnings, tests, universal Padd.app
#   Docker (Linux):  tests, Linux arm64 + x86_64, Windows x86_64
# Artifacts are copied back to ./dist/.
# Usage: ci/ci.sh [macos|linux|all]   (default: all)
set -euo pipefail
WHAT=${1:-all}
ROOT=$(cd "$(dirname "$0")/.." && pwd)
CONNECT=${MACMINI_CONNECT:-$HOME/.config/macmini/connect.sh}
# shellcheck disable=SC1090
source "$CONNECT"   # provides mm, mmd, mscp and $MM_ASKPASS
mfetch() { SSH_ASKPASS="$MM_ASKPASS" SSH_ASKPASS_REQUIRE=force DISPLAY=:0 \
           setsid -w scp -o ConnectTimeout=35 "macmini:$1" "$2" < /dev/null; }

echo "::: upload sources"
TGZ=$(mktemp --suffix=.tgz)
tar czf "$TGZ" -C "$ROOT" --exclude=./target --exclude=./dist --exclude=./.git .
mm 'mkdir -p ~/ci/padd'
mscp "$TGZ" ci/padd/src.tgz
rm -f "$TGZ"
mm 'rm -rf ~/ci/padd/src ~/ci/padd/out && mkdir -p ~/ci/padd/src ~/ci/padd/out && tar xzf ~/ci/padd/src.tgz -C ~/ci/padd/src 2>/dev/null'

if [[ $WHAT == all || $WHAT == macos ]]; then
    echo "::: macOS"
    mm 'bash ~/ci/padd/src/ci/macos.sh'
fi
if [[ $WHAT == all || $WHAT == linux ]]; then
    echo "::: docker image"
    mmd 'docker build -q -t padd-ci ~/ci/padd/src/ci'
    echo "::: linux + windows"
    mmd 'docker run --rm -v ~/ci/padd/src:/src -v ~/ci/padd/out:/out \
        -v padd-cargo-registry:/usr/local/cargo/registry -v padd-target:/target \
        -e CARGO_TARGET_DIR=/target padd-ci bash /src/ci/linux-windows.sh'
fi

echo "::: fetch artifacts"
mkdir -p "$ROOT/dist"
for f in padd-macos-universal.zip padd-linux-arm64 padd-linux-x86_64 padd-windows-x86_64.exe; do
    mfetch "ci/padd/out/$f" "$ROOT/dist/$f" 2>/dev/null && echo "  dist/$f" || true
done
ls -l "$ROOT/dist"
