#!/usr/bin/env python3
"""Three-state stdlib coverage report.

Cross-references the generated API inventory (``stdlib-inventory.md``) against
two signals to classify every inventory member:

* **missing**     — not in the qswift-std registry.
* **implemented** — present in the registry (declared coverage).
* **verified**    — in the registry *and* exercised by a passing CLI golden
                    fixture (behavioural coverage).

The registry signal comes from ``registered_keys.txt`` (regenerate with
``cargo test -p qswift-std dump_registered_keys`` — it reads the live registry,
so it cannot drift). The fixture signal is read from the *executing* CLI golden
fixtures under ``crates/qswift-cli/tests/fixtures`` (not the frontend-only
``tests/swift-fixtures``).

Usage:
    python3 tools/stdlib-inventory/coverage.py
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
INVENTORY = ROOT / "docs/swift-runtime/stdlib-inventory.md"
KEYS = Path(__file__).resolve().parent / "registered_keys.txt"
FIXTURES = ROOT / "crates/qswift-cli/tests/fixtures"

# Inventory types that conform to Sequence/Collection, so a `Sequence.<algo>`
# registry entry covers their algorithm members too.
SEQUENCE_TYPES = {
    "Array", "ArraySlice", "ContiguousArray", "Set", "Dictionary",
    "String", "Substring", "Range", "ClosedRange", "CollectionOfOne",
    "EmptyCollection", "ReversedCollection",
}

MEMBER_RE = re.compile(
    r"""(?:func|var|let|init|subscript|case)\s+   # member keyword
        `?                                          # optional backtick
        (?P<name>[A-Za-z_][A-Za-z0-9_]*|[-+*/<>=!%&|^~]+)  # identifier or operator
    """,
    re.VERBOSE,
)


def parse_inventory(text: str):
    """Return (free_funcs:set, types:dict[str, set[str]])."""
    free: set[str] = set()
    types: dict[str, set[str]] = {}
    current = None  # None means the "Free functions" section
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
        name = extract_member(line)
        if not name:
            continue
        if current == "__free__":
            free.add(name)
        elif current:
            types[current].add(name)
    return free, types


def extract_member(line: str) -> str | None:
    m = MEMBER_RE.search(line)
    return m.group("name") if m else None


def load_keys():
    free: set[str] = set()
    by_type: dict[str, set[str]] = {}
    seq_algos: set[str] = set()
    for raw in KEYS.read_text().splitlines():
        key = raw.strip()
        if not key:
            continue
        if "." not in key:
            free.add(key)
        else:
            ty, member = key.split(".", 1)
            if ty == "Sequence":
                seq_algos.add(member)
            else:
                by_type.setdefault(ty, set()).add(member)
    return free, by_type, seq_algos


def fixture_tokens() -> set[str]:
    """Identifiers used in executing CLI fixtures (member + call names)."""
    tokens: set[str] = set()
    member_re = re.compile(r"\.([A-Za-z_][A-Za-z0-9_]*)")
    call_re = re.compile(r"\b([A-Za-z_][A-Za-z0-9_]*)\s*\(")
    for swift in FIXTURES.glob("*.swift"):
        src = swift.read_text()
        tokens.update(member_re.findall(src))
        tokens.update(call_re.findall(src))
    return tokens


def main() -> int:
    if not INVENTORY.exists() or not KEYS.exists():
        print("missing inventory or registered_keys.txt", file=sys.stderr)
        return 1

    free_inv, types_inv = parse_inventory(INVENTORY.read_text())
    free_reg, by_type_reg, seq_algos = load_keys()
    used = fixture_tokens()

    def state(ty: str, member: str) -> str:
        registered = (
            member in by_type_reg.get(ty, set())
            or (ty in SEQUENCE_TYPES and member in seq_algos)
            or (ty == "Optional" and member in {"map", "flatMap"})
        )
        if not registered:
            return "missing"
        return "verified" if member in used else "implemented"

    # Per-type report, limited to types we touch (registry or sequence types).
    touched = sorted(set(by_type_reg) | (SEQUENCE_TYPES & set(types_inv)))
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
    f_verif = sum(1 for f in free_inv if f in free_reg and f in used)
    print("-" * 56)
    print(f"{'(free functions)':<20} {f_impl:>6} {f_verif:>6} {len(free_inv):>6}")

    print("\n## Overall (targeted types + free functions)")
    g_total = tot_total + len(free_inv)
    g_impl = tot_impl + f_impl
    g_verif = tot_verif + f_verif
    print(f"implemented: {g_impl}/{g_total} ({100*g_impl/g_total:.1f}%)")
    print(f"verified:    {g_verif}/{g_total} ({100*g_verif/g_total:.1f}%)")
    print(f"\ninventory totals: {len(types_inv)} types, {len(free_inv)} free functions")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
