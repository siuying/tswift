#!/usr/bin/env bash
#
# Run the full native-host verification sweep: the Rust FFI tests, the TSwift
# package tests (macOS), the UiirRenderer snapshot tests (iOS simulator), and a
# build of the example app (iOS simulator). See docs/plan/native-host.md (T11).
#
# Override the simulator with TSWIFT_SIM. Builds the local xcframework first if
# it is missing (the Swift halves link against it).
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
cd "$SCRIPT_DIR/.."

SIM="${TSWIFT_SIM:-platform=iOS Simulator,name=iPhone 16 Pro,OS=18.5}"

if [[ ! -d ios/TSwift/Artifacts/TSwiftFFI.xcframework ]]; then
  echo "==> Building local xcframework (missing)"
  ./scripts/build-xcframework.sh
fi

echo "==> [1/4] Rust FFI tests"
cargo test -p tswift-ffi

echo "==> [2/4] TSwift package tests (macOS)"
( cd ios/TSwift && swift test )

echo "==> [3/4] UiirRenderer tests (iOS simulator)"
( cd ios/UiirRenderer && xcodebuild test -scheme UiirRenderer -destination "$SIM" )

echo "==> [4/4] Example app build (iOS simulator)"
( cd examples/ios && xcodegen generate >/dev/null \
  && xcodebuild build -project TSwiftExample.xcodeproj -scheme TSwiftExample -destination "$SIM" )

echo "==> Native host verification complete"
