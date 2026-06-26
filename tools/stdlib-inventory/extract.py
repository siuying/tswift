#!/usr/bin/env python3
"""Extract the Swift standard-library public API surface from a `.swiftinterface`.

Goal 1 of the stdlib-support plan: produce a machine-generated inventory of every
public type and member the runtime may need to implement, grouped by type, so the
hand-written Tier 10 checklist has a complete companion that cannot silently drift
from the reference toolchain.

Reference (goal 2): Swift 6.3.2 stdlib interface, e.g.
  ~/Library/Developer/Toolchains/swift-6.3.2-RELEASE.xctoolchain/usr/lib/swift/\
    macosx/Swift.swiftmodule/arm64-apple-macos.swiftinterface

Usage:
  python3 extract.py <path-to-Swift.swiftinterface> > stdlib-inventory.md

The extractor is intentionally simple: it tracks brace depth, recognises top-level
`struct/enum/class/protocol` declarations and `extension <Type>` blocks, and
collects the public members declared at depth 1 inside them. It is a *surface*
extractor, not a full parser — good enough to enumerate names + signatures.
"""
from __future__ import annotations

import re
import sys
from collections import defaultdict

# A type declaration that opens a fresh namespace.
TYPE_RE = re.compile(
    r"\b(?:struct|enum|class|protocol|actor)\s+([A-Za-z_][A-Za-z0-9_]*)"
)
# `extension Swift.Array : ...` or `extension Array {`
EXT_RE = re.compile(r"\bextension\s+(?:Swift\.)?([A-Za-z_][A-Za-z0-9_]*)")
# Public members we care about (declared at brace depth 1 of a type/extension).
MEMBER_RE = re.compile(
    r"\b(?:public|open)\b.*?\b(func|var|let|subscript|init)\b"
)
CASE_RE = re.compile(r"^\s*(?:@\w+\s+)*case\s+([A-Za-z_][A-Za-z0-9_]*)")


def strip_attrs(line: str) -> str:
    return line.strip()


def member_signature(line: str) -> str:
    """Trim a member declaration to a readable signature (drop bodies/attrs noise)."""
    s = line.strip()
    # Drop trailing ` {` opening an accessor/body and anything after.
    s = re.sub(r"\s*\{.*$", "", s)
    # Collapse leading attributes (@inlinable @frozen ...) but keep modifiers.
    s = re.sub(r"^(?:@\w+(?:\([^)]*\))?\s+)+", "", s)
    return s.strip()


# Match the declared identifier of a func/var/let so we can filter internals.
NAME_RE = re.compile(r"\b(?:func|var|let)\s+([A-Za-z_][A-Za-z0-9_]*)")


def is_internal(sig: str) -> bool:
    """Underscore-prefixed members and ObjC-bridging shims are runtime internals,
    not user-facing stdlib API — exclude them from the inventory."""
    m = NAME_RE.search(sig)
    if m and m.group(1).startswith("_"):
        return True
    if "_bridge" in sig or "ObjectiveC" in sig or "@_" in sig:
        return True
    return False


def main(path: str) -> None:
    with open(path, encoding="utf-8") as fh:
        lines = fh.readlines()

    # type name -> set of member signatures
    members: dict[str, set[str]] = defaultdict(set)
    free_funcs: set[str] = set()

    depth = 0
    # stack of (open_depth, type_name | None) for the block we're inside
    block_stack: list[tuple[int, str | None]] = []

    for raw in lines:
        line = raw.rstrip("\n")
        stripped = line.strip()

        current_type = block_stack[-1][1] if block_stack else None

        # Member detection at depth 1 inside a named block.
        if current_type and depth == 1:
            cm = CASE_RE.match(line)
            if cm:
                members[current_type].add(f"case {cm.group(1)}")
            elif ("public" in stripped or "open" in stripped) and MEMBER_RE.search(stripped):
                sig = member_signature(stripped)
                if not is_internal(sig):
                    members[current_type].add(sig)
        # Free (top-level) public functions.
        elif depth == 0 and "public func" in stripped and stripped.startswith(("public func", "@")):
            sig = member_signature(stripped)
            if not is_internal(sig):
                free_funcs.add(sig)

        # Does this line open a new type/extension block at the current depth?
        opens_block_type: str | None = None
        if "{" in line:
            ext = EXT_RE.search(line)
            typ = TYPE_RE.search(line)
            if ext:
                opens_block_type = ext.group(1)
            elif typ:
                opens_block_type = typ.group(1)

        # Update brace depth, pushing/popping block frames as we cross braces.
        for ch in line:
            if ch == "{":
                if depth == 0 and opens_block_type is not None:
                    block_stack.append((depth, opens_block_type))
                    opens_block_type = None  # only the first brace opens it
                else:
                    block_stack.append((depth, None))
                depth += 1
            elif ch == "}":
                depth = max(0, depth - 1)
                if block_stack:
                    block_stack.pop()

    # Emit markdown.
    print("# Swift stdlib API inventory (generated)")
    print()
    print(f"Source: `{path}`")
    print()
    print(f"- Types with members: **{len(members)}**")
    print(f"- Free functions: **{len(free_funcs)}**")
    print()
    print("> Generated by `tools/stdlib-inventory/extract.py`. Do not edit by hand.")
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


if __name__ == "__main__":
    if len(sys.argv) != 2:
        sys.exit("usage: extract.py <Swift.swiftinterface>")
    main(sys.argv[1])
