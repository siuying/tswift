#!/usr/bin/env python3
"""Regenerate the website's machine-readable coverage JSON.

Writes one `<framework>.json` per entry in `frameworks.toml` (via
`coverage.py --emit-json`) plus an `index.json` totals summary, into
`website/src/data/coverage/`. These files are **checked in** (same
convention as the generated `inventory.md`/`registered_keys.txt` manifests
this tool already produces) because the website build has no access to the
Swift toolchain/SDK that `extract.py` needs, and must stay buildable offline.
`coverage.py` itself only reads already-checked-in manifests (inventory.md,
registered_keys.txt, scope.toml) — no toolchain required — so this script is
safe to run anywhere the repo is checked out.

Usage:
  python3 tools/framework-inventory/generate_website_json.py
  python3 tools/framework-inventory/generate_website_json.py --check   # drift check, no writes

`--check` regenerates into memory and diffs against the checked-in files,
exiting non-zero (and printing which files are stale) without touching disk.
Wired into `scripts/validate web`.
"""
from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

from coverage import Coverage, load_manifest  # noqa: E402

ROOT = Path(__file__).resolve().parents[2]
OUT_DIR = ROOT / "website/src/data/coverage"


def render(framework: str) -> str:
    cov = Coverage(framework)
    payload = cov.to_dict(include_all=False)
    return json.dumps(payload, indent=2) + "\n"


def render_index(frameworks: list[str], rendered: dict[str, str]) -> str:
    entries = []
    for name in frameworks:
        payload = json.loads(rendered[name])
        entries.append(
            {
                "framework": name,
                "display_name": payload["framework"],
                "totals": payload["totals"],
            }
        )
    return json.dumps({"frameworks": entries}, indent=2) + "\n"


def build_all() -> dict[str, str]:
    frameworks = sorted(load_manifest())
    rendered = {name: render(name) for name in frameworks}
    rendered["index"] = render_index(frameworks, rendered)
    return rendered


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--check",
        action="store_true",
        help="fail if checked-in JSON is stale instead of writing it",
    )
    args = parser.parse_args()

    rendered = build_all()

    if args.check:
        stale = []
        for name, text in rendered.items():
            path = OUT_DIR / f"{name}.json"
            if not path.exists() or path.read_text() != text:
                stale.append(path)
        if stale:
            print("Coverage JSON is stale (run generate_website_json.py to refresh):", file=sys.stderr)
            for path in stale:
                print(f"  {path.relative_to(ROOT)}", file=sys.stderr)
            return 1
        print("Coverage JSON is up to date.")
        return 0

    OUT_DIR.mkdir(parents=True, exist_ok=True)
    for name, text in rendered.items():
        (OUT_DIR / f"{name}.json").write_text(text)
    print(f"Wrote {len(rendered)} files to {OUT_DIR.relative_to(ROOT)}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
