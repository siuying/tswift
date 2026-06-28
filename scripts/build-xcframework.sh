#!/usr/bin/env bash
#
# Build the local TSwiftFFI.xcframework from the tswift-ffi staticlib.
#
# Produces a git-ignored artifact under ios/TSwift/Artifacts/ that the SwiftPM
# package picks up via its local-override `binaryTarget` (see ADR-0008 and
# docs/plan/native-host.md). Slices: iOS device, iOS simulator (fat arm64 +
# x86_64), macOS (fat arm64 + x86_64).
#
# Usage: scripts/build-xcframework.sh [--debug]
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
cd "$SCRIPT_DIR/.."

usage() { echo "Usage: $(basename "$0") [--debug]"; }

PROFILE="release"
PROFILE_FLAG="--release"
case "${1:-}" in
  "") ;;
  --debug) PROFILE="debug"; PROFILE_FLAG="" ;;
  -h | --help) usage; exit 0 ;;
  *) echo "error: unknown argument: $1" >&2; usage >&2; exit 2 ;;
esac
if [[ $# -gt 1 ]]; then
  echo "error: too many arguments" >&2; usage >&2; exit 2
fi

CRATE="tswift-ffi"
LIB="libtswift_ffi.a"
HEADERS="crates/tswift-ffi/include"
OUT_DIR="ios/TSwift/Artifacts"
XCFRAMEWORK="$OUT_DIR/TSwiftFFI.xcframework"

IOS="aarch64-apple-ios"
SIM_ARM="aarch64-apple-ios-sim"
# The Intel iOS *simulator* slice is plain `x86_64-apple-ios` (there is no
# `-sim` variant; iOS never ran on Intel devices).
SIM_X86="x86_64-apple-ios"
MAC_ARM="aarch64-apple-darwin"
MAC_X86="x86_64-apple-darwin"
ALL_TARGETS=("$IOS" "$SIM_ARM" "$SIM_X86" "$MAC_ARM" "$MAC_X86")

echo "==> Building $CRATE ($PROFILE) for ${#ALL_TARGETS[@]} targets"
for target in "${ALL_TARGETS[@]}"; do
  rustup target add "$target" >/dev/null
  echo "    - $target"
  cargo build -p "$CRATE" $PROFILE_FLAG --target "$target"
done

lib_path() { echo "target/$1/$PROFILE/$LIB"; }

STAGE="$(mktemp -d)"
trap 'rm -rf "$STAGE"' EXIT

echo "==> Lipo'ing fat simulator and macOS slices"
# Keep the `lib` prefix (SwiftPM rejects static libraries without it); make the
# names unique by staging each fat slice in its own directory.
mkdir -p "$STAGE/sim" "$STAGE/mac"
lipo -create "$(lib_path "$SIM_ARM")" "$(lib_path "$SIM_X86")" \
  -output "$STAGE/sim/$LIB"
lipo -create "$(lib_path "$MAC_ARM")" "$(lib_path "$MAC_X86")" \
  -output "$STAGE/mac/$LIB"

echo "==> Creating $XCFRAMEWORK"
rm -rf "$XCFRAMEWORK"
mkdir -p "$OUT_DIR"
xcodebuild -create-xcframework \
  -library "$(lib_path "$IOS")" -headers "$HEADERS" \
  -library "$STAGE/sim/$LIB" -headers "$HEADERS" \
  -library "$STAGE/mac/$LIB" -headers "$HEADERS" \
  -output "$XCFRAMEWORK"

echo "==> Done: $XCFRAMEWORK"
