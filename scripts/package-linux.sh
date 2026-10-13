#!/usr/bin/env bash
set -euo pipefail

ROOT=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
PACKAGE_NAME=${PACKAGE_NAME:-MuxSift-linux-x86_64}
OUT="$ROOT/dist/$PACKAGE_NAME"
ARCHIVE="$OUT.tar.gz"
CHECKSUM="$ARCHIVE.sha256"
TARGET_DIR=${CARGO_TARGET_DIR:-$ROOT/target}
if [[ $TARGET_DIR != /* ]]; then
    TARGET_DIR="$ROOT/$TARGET_DIR"
fi

fail() { printf 'ERROR: %s\n' "$*" >&2; exit 1; }
command -v readelf >/dev/null || fail 'readelf is required (install binutils).'
command -v tar >/dev/null || fail 'tar is required.'
[[ $(uname -s) == Linux && $(uname -m) == x86_64 ]] || fail 'Build this package on Linux x86_64.'

# Supply independently obtained Linux executables and the notices for those exact builds.
ARIA2_SOURCE=${ARIA2_SOURCE:-$ROOT/third_party/aria2-linux/aria2c}
FFMPEG_SOURCE=${FFMPEG_SOURCE:-$ROOT/third_party/ffmpeg-linux/ffmpeg}
ARIA2_LICENSE=${ARIA2_LICENSE:-$ROOT/third_party/aria2-linux/COPYING}
FFMPEG_LICENSE=${FFMPEG_LICENSE:-$ROOT/third_party/ffmpeg-linux/LICENSE}
ARIA2_SOURCE_INFO=${ARIA2_SOURCE_INFO:-$ROOT/third_party/aria2-linux/SOURCE.txt}
FFMPEG_SOURCE_INFO=${FFMPEG_SOURCE_INFO:-$ROOT/third_party/ffmpeg-linux/SOURCE.txt}

for item in "$ARIA2_SOURCE" "$FFMPEG_SOURCE" "$ARIA2_LICENSE" "$FFMPEG_LICENSE" "$ARIA2_SOURCE_INFO" "$FFMPEG_SOURCE_INFO"; do
    [[ -f $item ]] || fail "Missing $item. See the Linux packaging section in README.md."
done

check_binary() {
    local binary=$1 name=$2 needed
    [[ -x $binary ]] || fail "$name is not executable: $binary"
    readelf -h "$binary" | grep -q 'Machine:.*Advanced Micro Devices X86-64' || fail "$name is not an x86_64 ELF binary: $binary"
    # A portable release may use the base Ubuntu C runtime, but not host multimedia libraries.
    while IFS= read -r needed; do
        case "$needed" in
            libc.so.6|libm.so.6|libmvec.so.1|libpthread.so.0|libdl.so.2|librt.so.1|libgcc_s.so.1|ld-linux-x86-64.so.2) ;;
            *) fail "$name depends on $needed. Provide a portable build with its non-system libraries linked in." ;;
        esac
    done < <(readelf -d "$binary" | sed -n 's/.*Shared library: \[\([^]]*\)\].*/\1/p')
    if [[ $name == ffmpeg ]]; then
        "$binary" -version >/dev/null 2>&1 || fail "$name cannot run on this build host: $binary"
    else
        "$binary" --version >/dev/null 2>&1 || fail "$name cannot run on this build host: $binary"
    fi
}
check_binary "$ARIA2_SOURCE" aria2c
check_binary "$FFMPEG_SOURCE" ffmpeg

if [[ ${SKIP_BUILD:-0} != 1 ]]; then
    command -v cargo >/dev/null || fail 'cargo is required to build the release binary.'
    (cd "$ROOT" && cargo build --release --locked -p muxsift-cli)
fi
CLI="$TARGET_DIR/release/muxsift"
[[ -x $CLI ]] || fail "Missing release binary: $CLI"
check_binary "$CLI" muxsift

# Refuse to overwrite an earlier package. Use a fresh output location for each build.
[[ ! -e $OUT && ! -e $ARCHIVE && ! -e $CHECKSUM ]] || fail "Output already exists: $OUT, $ARCHIVE, or $CHECKSUM"
mkdir -p "$OUT/release/bin" "$OUT/release/libexec" "$OUT/release/licenses"
cp "$CLI" "$OUT/release/muxsift"
cp "$ARIA2_SOURCE" "$OUT/release/bin/aria2c"
cp "$FFMPEG_SOURCE" "$OUT/release/bin/ffmpeg"
cp "$ARIA2_LICENSE" "$OUT/release/licenses/aria2-LICENSE"
if [[ -f $ROOT/third_party/aria2-linux/LICENSE.OpenSSL ]]; then
    cp "$ROOT/third_party/aria2-linux/LICENSE.OpenSSL" "$OUT/release/licenses/aria2-OpenSSL-LICENSE"
fi
cp "$FFMPEG_LICENSE" "$OUT/release/licenses/ffmpeg-LICENSE"
cp "$ARIA2_SOURCE_INFO" "$OUT/release/licenses/aria2-SOURCE.txt"
cp "$FFMPEG_SOURCE_INFO" "$OUT/release/licenses/ffmpeg-SOURCE.txt"
cp "$ROOT/distribution/start-tui.sh" "$OUT/release/start-tui.sh"
cp "$ROOT/scripts/buzz-asr.py" "$OUT/release/libexec/buzz-asr.py"
cp "$ROOT/distribution/THIRD-PARTY-NOTICES.txt" "$OUT/release/THIRD-PARTY-NOTICES.txt"
cp "$ROOT/README.md" "$ROOT/USER-GUIDE.md" "$ROOT/USER-GUIDE.zh-CN.md" "$ROOT/distribution/config.example.json" "$OUT/release/"
chmod 755 "$OUT/release/muxsift" "$OUT/release/bin/aria2c" "$OUT/release/bin/ffmpeg" "$OUT/release/start-tui.sh" "$OUT/release/libexec/buzz-asr.py"
tar -C "$OUT" -czf "$ARCHIVE" release
(
    cd "$(dirname -- "$ARCHIVE")"
    sha256sum "$(basename -- "$ARCHIVE")" > "$(basename -- "$CHECKSUM")"
)
printf 'Package: %s\nArchive: %s\nChecksum: %s\n' "$OUT/release" "$ARCHIVE" "$CHECKSUM"
