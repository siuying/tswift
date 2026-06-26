#!/usr/bin/env python3
"""Three-state stdlib coverage report.

Coverage is a pure join over three canonical sets of *semantic* stdlib keys
(`print`, `Array.append`, `Optional.map`, `Sequence.map`, …):

* **inventory**   — every member declared in ``stdlib-inventory.md``.
* **registered**  — what the qswift-std registry actually wires up.
* **exercised**   — what a *passing* golden fixture actually dispatched.

Each inventory member is classified by set membership, type-scoped (no global
token matching):

* **missing**     — not in the registry.
* **implemented** — registered but never exercised by a passing fixture.
* **verified**    — registered *and* exercised by a passing fixture.

The `registered`/`exercised` inputs are regenerated live by the golden harness
(they cannot drift from the code) into ``target/stdlib-coverage/``:

    cargo test -p qswift-cli --test golden stdlib_coverage_inputs

Then:

    python3 tools/stdlib-inventory/coverage.py
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
INVENTORY = ROOT / "docs/swift-runtime/stdlib-inventory.md"
COVERAGE_DIR = ROOT / "target/stdlib-coverage"
REGISTERED = COVERAGE_DIR / "registered.txt"
EXERCISED = COVERAGE_DIR / "exercised.txt"

# Inventory types that conform to Sequence/Collection: a `Sequence.<algo>`
# registry/exercise entry covers their algorithm members too. This is the one
# piece of domain knowledge the join needs, applied uniformly to both sets.
SEQUENCE_TYPES = {
    "Array", "ArraySlice", "ContiguousArray", "Set", "Dictionary",
    "String", "Substring", "Range", "ClosedRange", "CollectionOfOne",
    "EmptyCollection", "ReversedCollection",
}

# Member-declaration keywords we map to a semantic member name. `init`/`subscript`
# normalize to those literal names (collapsing all overloads); the others take
# the following identifier or operator token.
_KEYWORD_RE = re.compile(r"\b(func|var|let|init|subscript|case)\b")
_NAME_RE = re.compile(r"`?(?P<name>[A-Za-z_][A-Za-z0-9_]*|[-+*/<>=!%&|^~]+)")


def member_key(line: str) -> str | None:
    """Normalize one inventory bullet into a semantic member name.

    `public init()` -> `init`; `public subscript(i:)` -> `subscript`;
    `public func map<T>(...)` -> `map`; `static func + (...)` -> `+`.
    Lines without a recognized member keyword (typealias, …) are ignored.
    """
    m = _KEYWORD_RE.search(line)
    if not m:
        return None
    kw = m.group(1)
    if kw in ("init", "subscript"):
        return kw
    rest = line[m.end():]
    nm = _NAME_RE.search(rest)
    return nm.group("name") if nm else None


def parse_inventory(text: str) -> tuple[set[str], dict[str, set[str]]]:
    """Return (free_funcs, types[type -> set of semantic member names])."""
    free: set[str] = set()
    types: dict[str, set[str]] = {}
    current: str | None = None  # "__free__" for the free-functions section
    for line in text.splitlines():
        if line.startswith("## "):
            heading = line[3:].strip()
            if heading.lower().startswith("free function"):
                current = "__free__"
            else:
                current = heading.split("  ")[0].split(" (")[0].strip()
                types.setdefault(current, set())
            continue
        if not line.startswith("- `"):
            continue
        name = member_key(line)
        if not name:
            continue
        if current == "__free__":
            free.add(name)
        elif current:
            types[current].add(name)
    return free, types


def load_keys(path: Path) -> tuple[set[str], dict[str, set[str]], set[str]]:
    """Split a key file into (free, by_type[type -> members], sequence_algos)."""
    free: set[str] = set()
    by_type: dict[str, set[str]] = {}
    seq_algos: set[str] = set()
    for raw in path.read_text().splitlines():
        key = raw.strip()
        if not key:
            continue
        if "." not in key:
            free.add(key)
            continue
        ty, member = key.split(".", 1)
        if ty == "Sequence":
            seq_algos.add(member)
        else:
            by_type.setdefault(ty, set()).add(member)
    return free, by_type, seq_algos


def main() -> int:
    for required in (INVENTORY, REGISTERED, EXERCISED):
        if not required.exists():
            print(
                f"missing input: {required}\n"
                "regenerate with: "
                "cargo test -p qswift-cli --test golden stdlib_coverage_inputs",
                file=sys.stderr,
            )
            return 1

    free_inv, types_inv = parse_inventory(INVENTORY.read_text())
    free_reg, reg_by_type, reg_seq = load_keys(REGISTERED)
    free_ex, ex_by_type, ex_seq = load_keys(EXERCISED)

    def has(ty: str, member: str, by_type: dict[str, set[str]], seq: set[str]) -> bool:
        if member in by_type.get(ty, ()):
            return True
        return ty in SEQUENCE_TYPES and member in seq

    def state(ty: str, member: str) -> str:
        if not has(ty, member, reg_by_type, reg_seq):
            return "missing"
        if has(ty, member, ex_by_type, ex_seq):
            return "verified"
        return "implemented"

    # Per-type report, limited to types the registry (or a sequence entry) touches.
    touched = sorted(set(reg_by_type) | (SEQUENCE_TYPES & set(types_inv)))
    print("# Stdlib coverage report\n")
    print(f"{'type':<20} {'impl':>6} {'verif':>6} {'total':>6}  {'%verified':>9}")
    print("-" * 56)
    tot_impl = tot_verif = tot_total = 0
    for ty in touched:
        members = types_inv.get(ty, set())
        if not members:
            continue
        impl = verif = 0
        for m in members:
            s = state(ty, m)
            if s in ("implemented", "verified"):
                impl += 1
            if s == "verified":
                verif += 1
        total = len(members)
        tot_impl += impl
        tot_verif += verif
        tot_total += total
        pct = (100 * verif / total) if total else 0
        print(f"{ty:<20} {impl:>6} {verif:>6} {total:>6}  {pct:>8.1f}%")

    # Free functions.
    f_impl = sum(1 for f in free_inv if f in free_reg)
    f_verif = sum(1 for f in free_inv if f in free_reg and f in free_ex)
    print("-" * 56)
    print(f"{'(free functions)':<20} {f_impl:>6} {f_verif:>6} {len(free_inv):>6}")

    print("\n## Overall (targeted types + free functions)")
    g_total = tot_total + len(free_inv)
    g_impl = tot_impl + f_impl
    g_verif = tot_verif + f_verif
    print(f"implemented: {g_impl}/{g_total} ({100 * g_impl / g_total:.1f}%)")
    print(f"verified:    {g_verif}/{g_total} ({100 * g_verif / g_total:.1f}%)")
    print(f"\ninventory totals: {len(types_inv)} types, {len(free_inv)} free functions")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
