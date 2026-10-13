#!/usr/bin/env bash
set -euo pipefail

ROOT=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
ARIA2_URL=https://github.com/abcfy2/aria2-static-build/releases/download/1.37.0/aria2-x86_64-linux-musl_static.zip
ARIA2_SHA256=e0a09b12ef67f35f8a8e4fdddbec851d235b7c31da549d0578bff459032b499a
FFMPEG_URL=https://github.com/BtbN/FFmpeg-Builds/releases/download/latest/ffmpeg-n9.0-latest-linux64-lgpl-9.0.tar.xz
FFMPEG_SHA256=1a88f2305c14c208aa4f252298cefa0b0c30485f20bebe181055ef50eb2a54f5

fail() { printf 'ERROR: %s\n' "$*" >&2; exit 1; }
for command in curl sha256sum tar unzip install; do
    command -v "$command" >/dev/null || fail "$command is required"
done
[[ $(uname -s) == Linux && $(uname -m) == x86_64 ]] || fail 'This script downloads Linux x86_64 tools.'

TEMP_DIR=$(mktemp -d /tmp/muxsift-linux-tools.XXXXXX)
trap 'rm -rf -- "$TEMP_DIR"' EXIT
ARIA2_ARCHIVE=$TEMP_DIR/aria2.zip
FFMPEG_ARCHIVE=$TEMP_DIR/ffmpeg.tar.xz

curl -fL --retry 3 --connect-timeout 15 "$ARIA2_URL" -o "$ARIA2_ARCHIVE"
printf '%s  %s\n' "$ARIA2_SHA256" "$ARIA2_ARCHIVE" | sha256sum -c -
mkdir -p "$TEMP_DIR/aria2"
unzip -q "$ARIA2_ARCHIVE" -d "$TEMP_DIR/aria2"

curl -fL --retry 3 --connect-timeout 15 "$FFMPEG_URL" -o "$FFMPEG_ARCHIVE"
printf '%s  %s\n' "$FFMPEG_SHA256" "$FFMPEG_ARCHIVE" | sha256sum -c -
mkdir -p "$TEMP_DIR/ffmpeg"
tar -xJf "$FFMPEG_ARCHIVE" -C "$TEMP_DIR/ffmpeg" --strip-components=1

mkdir -p "$ROOT/third_party/aria2-linux" "$ROOT/third_party/ffmpeg-linux"
install -m 0755 "$TEMP_DIR/aria2/aria2c" "$ROOT/third_party/aria2-linux/aria2c"
install -m 0755 "$TEMP_DIR/ffmpeg/bin/ffmpeg" "$ROOT/third_party/ffmpeg-linux/ffmpeg"
install -m 0644 "$TEMP_DIR/ffmpeg/LICENSE.txt" "$ROOT/third_party/ffmpeg-linux/LICENSE"

printf 'Installed aria2c and FFmpeg under %s/third_party.\n' "$ROOT"
printf 'Run ./scripts/package-linux.sh to build the release archive.\n'
