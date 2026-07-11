#!/usr/bin/env bash
#
# Regenerate website/src/data/coverage/*.json from the framework-inventory
# manifests (inventory.md / registered_keys.txt / scope.toml). Checked-in
# output — see tools/framework-inventory/generate_website_json.py for why.
#
# Usage:
#   scripts/generate-coverage-json.sh          # write the files
#   scripts/generate-coverage-json.sh --check  # drift check only (no writes)
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
ROOT="$(cd "$SCRIPT_DIR/.." && pwd -P)"
cd "$ROOT"

exec python3 tools/framework-inventory/generate_website_json.py "$@"
