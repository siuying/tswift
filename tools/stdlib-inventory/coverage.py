#!/usr/bin/env python3
"""Backward-compatible shim for tools/framework-inventory/coverage.py."""
from __future__ import annotations

import runpy
import sys
from pathlib import Path

TARGET = Path(__file__).resolve().parents[1] / "framework-inventory" / "coverage.py"

if __name__ == "__main__":
    if "--framework" not in sys.argv and "-f" not in sys.argv:
        sys.argv.insert(1, "--framework")
        sys.argv.insert(2, "stdlib")
    sys.argv[0] = str(TARGET)
    runpy.run_path(str(TARGET), run_name="__main__")
