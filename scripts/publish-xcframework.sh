#!/usr/bin/env bash
#
# Publish the TSwiftFFI.xcframework as a GitHub Release asset and update the
# committed pin (ADR-0008). On-demand — not part of any build.
#
#   scripts/publish-xcframework.sh [--dry-run] [--tag ffi-vN]
#
# --dry-run computes the checksum and prints the proposed ffi.pin without
# touching GitHub or the committed ios/TSwift/ffi.pin (writes dist/ffi.pin).
#
# A real run requires an authenticated `gh` and network access; it creates the
# release, uploads the zip, and rewrites ios/TSwift/ffi.pin — commit that file
# to publish the pin.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
cd "$SCRIPT_DIR/.."

usage() { echo "Usage: $(basename "$0") [--dry-run] [--tag <ffi-vN>]"; }

DRY_RUN=0
TAG=""
while [[ $# -gt 0 ]]; do
  case "$1" in
    --dry-run) DRY_RUN=1; shift ;;
    --tag)
      if [[ $# -lt 2 || "$2" == -* ]]; then
        echo "error: --tag requires a value" >&2; usage >&2; exit 2
      fi
      TAG="$2"; shift 2 ;;
    -h | --help) usage; exit 0 ;;
    *) echo "error: unknown argument: $1" >&2; usage >&2; exit 2 ;;
  esac
done

XCFRAMEWORK="ios/TSwift/Artifacts/TSwiftFFI.xcframework"
PIN="ios/TSwift/ffi.pin"
DIST="$PWD/dist"
ZIP="$DIST/TSwiftFFI.xcframework.zip"

# The same repo must back both the upload and the pin URL. Prefer an explicit
# override, else derive from `gh` on a real run (network); dry-run uses a
# display-only fallback so it needs no `gh`.
if [[ -n "${TSWIFT_REPO:-}" ]]; then
  REPO="$TSWIFT_REPO"
elif [[ "$DRY_RUN" == "1" ]]; then
  REPO="siuying/tswift"
else
  REPO="$(gh repo view --json nameWithOwner -q .nameWithOwner)"
fi

if [[ ! -d "$XCFRAMEWORK" ]]; then
  echo "==> No local xcframework; building it first"
  ./scripts/build-xcframework.sh
fi

mkdir -p "$DIST"
echo "==> Zipping $XCFRAMEWORK"
rm -f "$ZIP"
# `ditto` preserves the xcframework bundle layout (symlinks, structure).
( cd "$(dirname "$XCFRAMEWORK")" && ditto -c -k --keepParent "$(basename "$XCFRAMEWORK")" "$ZIP" )

echo "==> Computing checksum"
CHECKSUM="$(swift package --package-path ios/TSwift compute-checksum "$ZIP")"

if [[ -z "$TAG" ]]; then
  TAG="$(date +ffi-v%Y%m%d%H%M%S)"
fi
URL="https://github.com/$REPO/releases/download/$TAG/TSwiftFFI.xcframework.zip"

PIN_JSON="$(printf '{\n  "version": "%s",\n  "url": "%s",\n  "checksum": "%s"\n}\n' \
  "$TAG" "$URL" "$CHECKSUM")"

if [[ "$DRY_RUN" == "1" ]]; then
  echo "==> [dry-run] tag=$TAG"
  echo "==> [dry-run] checksum=$CHECKSUM"
  printf '%s\n' "$PIN_JSON" > "$DIST/ffi.pin"
  echo "==> [dry-run] proposed pin written to dist/ffi.pin (committed $PIN unchanged):"
  cat "$DIST/ffi.pin"
  exit 0
fi

if gh release view "$TAG" --repo "$REPO" >/dev/null 2>&1; then
  echo "error: release $TAG already exists in $REPO; choose a new --tag" >&2
  exit 1
fi

echo "==> Creating GitHub release $TAG in $REPO and uploading asset"
gh release create "$TAG" "$ZIP" \
  --repo "$REPO" \
  --title "TSwiftFFI $TAG" \
  --notes "Prebuilt TSwiftFFI.xcframework for $TAG."

echo "==> Updating $PIN"
printf '%s\n' "$PIN_JSON" > "$PIN"
echo "==> Done. Commit $PIN to publish the pin."
