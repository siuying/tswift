#!/usr/bin/env python3
"""Backward-compatible shim for tools/framework-inventory/extract.py."""
from __future__ import annotations

import runpy
import sys
from pathlib import Path

TARGET = Path(__file__).resolve().parents[1] / "framework-inventory" / "extract.py"

if __name__ == "__main__":
    sys.argv[0] = str(TARGET)
    runpy.run_path(str(TARGET), run_name="__main__")
