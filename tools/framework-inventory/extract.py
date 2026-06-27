#!/usr/bin/env python3
"""Extract a framework's public Swift API surface from a `.swiftinterface`.

Usage:
  python3 tools/framework-inventory/extract.py --framework foundation
  python3 tools/framework-inventory/extract.py --framework stdlib
  python3 tools/framework-inventory/extract.py path/to/Module.swiftinterface

Framework mode resolves the interface through `frameworks.toml`. Path mode is
kept for the legacy stdlib-inventory shim.
"""
from __future__ import annotations

import argparse
import os
import re
import subprocess
import sys
import tomllib
from collections import defaultdict
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
MANIFEST = Path(__file__).with_name("frameworks.toml")

TYPE_RE = re.compile(
    r"\b(?:struct|enum|class|protocol|actor)\s+([A-Za-z_][A-Za-z0-9_]*)"
)
EXT_RE = re.compile(r"\bextension\s+(?:[A-Za-z_][A-Za-z0-9_]*\.)?([A-Za-z_][A-Za-z0-9_]*)")
MEMBER_RE = re.compile(
    r"\b(?:public|open)\b.*?\b(func|var|let|subscript|init)\b"
)
CASE_RE = re.compile(r"^\s*(?:@[A-Za-z_][A-Za-z0-9_]*(?:\([^)]*\))?\s+)*case\s+([A-Za-z_][A-Za-z0-9_]*)")
NAME_RE = re.compile(r"\b(?:func|var|let)\s+([A-Za-z_][A-Za-z0-9_]*)")


def load_manifest() -> dict:
    with MANIFEST.open("rb") as fh:
        return tomllib.load(fh)


def run(cmd: list[str]) -> str:
    return subprocess.check_output(cmd, text=True, stderr=subprocess.DEVNULL).strip()


def toolchain_root(desc: dict) -> Path:
    name = desc.get("toolchain", "swift-6.3.2-RELEASE")
    candidates = [
        Path.home() / "Library/Developer/Toolchains" / f"{name}.xctoolchain",
        Path("/Library/Developer/Toolchains") / f"{name}.xctoolchain",
        Path(run(["xcrun", "--toolchain", name, "--find", "swiftc"])).parents[2],
    ]
    for candidate in candidates:
        if candidate.exists():
            return candidate
    raise SystemExit(f"cannot find Swift toolchain {name!r}")


def sdk_root() -> Path:
    sdk = os.environ.get("SDKROOT")
    if sdk:
        return Path(sdk)
    return Path(run(["xcrun", "--show-sdk-path"]))


def resolve_interface(framework: str) -> Path:
    manifest = load_manifest()
    if framework not in manifest:
        names = ", ".join(sorted(manifest))
        raise SystemExit(f"unknown framework {framework!r}; known: {names}")
    desc = manifest[framework]
    kind = desc["kind"]
    if kind == "toolchain":
        base = toolchain_root(desc) / desc["relative_path"]
    elif kind == "sdk-framework":
        base = sdk_root() / desc["framework_path"]
    else:
        raise SystemExit(f"unsupported descriptor kind: {kind}")
    for candidate in desc.get("interface_candidates", []):
        path = base / candidate
        if path.exists():
            return path
    raise SystemExit(f"no .swiftinterface found for {framework!r} under {base}")


def member_signature(line: str) -> str:
    s = line.strip()
    s = re.sub(r"\s*\{.*$", "", s)
    # Drop common interface-only attributes, but keep access/static/mutating.
    s = re.sub(r"^(?:@[A-Za-z_][A-Za-z0-9_]*(?:\([^)]*\))?\s+)+", "", s)
    return re.sub(r"\s+", " ", s).strip()


def is_internal(sig: str, framework: str) -> bool:
    m = NAME_RE.search(sig)
    if m and m.group(1).startswith("_"):
        return True
    if "@_" in sig or "_bridge" in sig:
        return True
    if framework == "stdlib" and "ObjectiveC" in sig:
        return True
    # SDK interfaces contain many ObjC-imported declarations. Keep the public
    # Swift spelling, but drop private shim names and SPI fragments.
    if "__" in sig or " NS_SWIFT_NAME" in sig:
        return True
    return False


def extract(path: Path, framework: str | None) -> tuple[dict[str, set[str]], set[str]]:
    lines = path.read_text(encoding="utf-8").splitlines()
    members: dict[str, set[str]] = defaultdict(set)
    free_funcs: set[str] = set()
    depth = 0
    block_stack: list[tuple[int, str | None]] = []
    fw = framework or "custom"

    for line in lines:
        stripped = line.strip()
        current_type = block_stack[-1][1] if block_stack else None

        if current_type and depth == 1:
            cm = CASE_RE.match(line)
            if cm:
                members[current_type].add(f"case {cm.group(1)}")
            elif ("public" in stripped or "open" in stripped) and MEMBER_RE.search(stripped):
                sig = member_signature(stripped)
                if not is_internal(sig, fw):
                    members[current_type].add(sig)
        elif depth == 0 and "public func" in stripped and stripped.startswith(("public func", "@")):
            sig = member_signature(stripped)
            if not is_internal(sig, fw):
                free_funcs.add(sig)

        opens_block_type: str | None = None
        if "{" in line:
            ext = EXT_RE.search(line)
            typ = TYPE_RE.search(line)
            if ext:
                opens_block_type = ext.group(1)
            elif typ:
                opens_block_type = typ.group(1)

        for ch in line:
            if ch == "{":
                if depth == 0 and opens_block_type is not None:
                    block_stack.append((depth, opens_block_type))
                    opens_block_type = None
                else:
                    block_stack.append((depth, None))
                depth += 1
            elif ch == "}":
                depth = max(0, depth - 1)
                if block_stack:
                    block_stack.pop()
    return members, free_funcs


def emit(path: Path, framework: str | None, members: dict[str, set[str]], free_funcs: set[str]) -> None:
    title = framework or "Swift"
    print(f"# {title} API inventory (generated)")
    print()
    print(f"Source: `{path}`")
    print()
    print(f"- Types with members: **{len(members)}**")
    print(f"- Free functions: **{len(free_funcs)}**")
    print()
    print("> Generated by `tools/framework-inventory/extract.py`. Do not edit by hand.")
    print()
    print("## Free functions")
    print()
    for sig in sorted(free_funcs):
        print(f"- `{sig}`")
    print()
    for typ in sorted(members):
        sigs = sorted(members[typ])
        print(f"## {typ}  ({len(sigs)} members)")
        print()
        for sig in sigs:
            print(f"- `{sig}`")
        print()


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("path", nargs="?", help="explicit .swiftinterface path")
    parser.add_argument("--framework", "-f", help="framework descriptor name")
    args = parser.parse_args(argv)

    if args.framework:
        path = resolve_interface(args.framework.lower())
        framework = args.framework.lower()
    elif args.path:
        path = Path(args.path)
        framework = None
    else:
        parser.error("pass --framework or a .swiftinterface path")
    members, free_funcs = extract(path, framework)
    emit(path, framework, members, free_funcs)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
