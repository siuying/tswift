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

Two levels of detail:

* **list** (default) — every targeted section with its implemented/verified/
  total counts, plus an overall roll-up. Add ``--all`` to include sections with
  no registry coverage yet.
* **detail** — pass a section name to break it down member-by-member, grouping
  each member under ``verified`` / ``implemented`` / ``missing``.

Usage:
    python3 tools/stdlib-inventory/coverage.py                 # list sections
    python3 tools/stdlib-inventory/coverage.py --all           # incl. 0% sections
    python3 tools/stdlib-inventory/coverage.py Array           # detail one section
    python3 tools/stdlib-inventory/coverage.py "free functions"
"""

from __future__ import annotations

import argparse
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

# Members whose runtime behaviour is implemented in qswift-core rather than the
# qswift-std registry. They are still stdlib surface area and should count when
# executable fixtures exercise them.
CORE_MEMBERS = {
    "Array": {"+", "+=", "=="},
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
    """Identifiers/operators used in executing CLI fixtures."""
    tokens: set[str] = set()
    member_re = re.compile(r"\.([A-Za-z_][A-Za-z0-9_]*)")
    call_re = re.compile(r"\b([A-Za-z_][A-Za-z0-9_]*)\s*\(")
    operator_re = re.compile(r"(?<![=!<>+\-*/%&|^])(?:\+=|==|\+)(?![=+])")
    for swift in FIXTURES.glob("*.swift"):
        src = swift.read_text()
        tokens.update(member_re.findall(src))
        tokens.update(call_re.findall(src))
        tokens.update(operator_re.findall(src))
    return tokens


FREE_SECTION = "(free functions)"


class Coverage:
    """Loaded inventory + registry + fixture signals, with classification."""

    def __init__(self):
        self.free_inv, self.types_inv = parse_inventory(INVENTORY.read_text())
        self.free_reg, self.by_type_reg, self.seq_algos = load_keys()
        self.used = fixture_tokens()

    def member_state(self, section: str, member: str) -> str:
        """Classify one member as 'missing' | 'implemented' | 'verified'."""
        if section == FREE_SECTION:
            registered = member in self.free_reg
        else:
            registered = (
                member in self.by_type_reg.get(section, set())
                or member in CORE_MEMBERS.get(section, set())
                or (section in SEQUENCE_TYPES and member in self.seq_algos)
                or (section == "Optional" and member in {"map", "flatMap"})
            )
        if not registered:
            return "missing"
        return "verified" if member in self.used else "implemented"

    def members(self, section: str) -> set[str]:
        if section == FREE_SECTION:
            return self.free_inv
        return self.types_inv.get(section, set())

    def classify(self, section: str) -> dict[str, list[str]]:
        """Group a section's members by state, each list sorted."""
        groups: dict[str, list[str]] = {"verified": [], "implemented": [], "missing": []}
        for m in self.members(section):
            groups[self.member_state(section, m)].append(m)
        for v in groups.values():
            v.sort()
        return groups

    def counts(self, section: str) -> tuple[int, int, int]:
        """Return (implemented, verified, total) for a section."""
        g = self.classify(section)
        verif = len(g["verified"])
        impl = verif + len(g["implemented"])
        total = impl + len(g["missing"])
        return impl, verif, total

    def targeted_sections(self, include_all: bool) -> list[str]:
        """Type sections to list. Targeted = has registry/sequence coverage."""
        if include_all:
            return sorted(s for s in self.types_inv if not s.startswith("_"))
        touched = set(self.by_type_reg) | (SEQUENCE_TYPES & set(self.types_inv))
        return sorted(s for s in touched if self.types_inv.get(s))

    def resolve(self, name: str) -> str | None:
        """Map a user-supplied name to a canonical section, case-insensitively."""
        key = name.strip().lower()
        if key in {"free", "free functions", "(free functions)", "free function"}:
            return FREE_SECTION
        for section in self.types_inv:
            if section.lower() == key:
                return section
        return None


def cmd_list(cov: Coverage, include_all: bool) -> int:
    print("# Stdlib coverage — sections\n")
    print(f"{'section':<22} {'impl':>6} {'verif':>6} {'total':>6}  {'%verified':>9}")
    print("-" * 58)
    tot_impl = tot_verif = tot_total = 0
    for section in cov.targeted_sections(include_all):
        impl, verif, total = cov.counts(section)
        if not total:
            continue
        tot_impl += impl
        tot_verif += verif
        tot_total += total
        pct = 100 * verif / total if total else 0
        print(f"{section:<22} {impl:>6} {verif:>6} {total:>6}  {pct:>8.1f}%")

    f_impl, f_verif, f_total = cov.counts(FREE_SECTION)
    print("-" * 58)
    print(f"{FREE_SECTION:<22} {f_impl:>6} {f_verif:>6} {f_total:>6}")

    g_total = tot_total + f_total
    g_impl = tot_impl + f_impl
    g_verif = tot_verif + f_verif
    print("\n## Overall (targeted sections + free functions)")
    print(f"implemented: {g_impl}/{g_total} ({100*g_impl/g_total:.1f}%)")
    print(f"verified:    {g_verif}/{g_total} ({100*g_verif/g_total:.1f}%)")
    print(
        f"\ninventory totals: {len(cov.types_inv)} types, "
        f"{len(cov.free_inv)} free functions"
    )
    print("\nRun `coverage.py <section>` for a member-by-member breakdown.")
    return 0


def cmd_detail(cov: Coverage, section: str) -> int:
    impl, verif, total = cov.counts(section)
    groups = cov.classify(section)
    print(f"# Stdlib coverage — {section}\n")
    pct = 100 * verif / total if total else 0
    print(f"implemented: {impl}/{total}   verified: {verif}/{total} ({pct:.1f}%)\n")
    for state in ("verified", "implemented", "missing"):
        items = groups[state]
        print(f"## {state} ({len(items)})")
        for m in items:
            print(f"  - {m}")
        print()
    return 0


def main() -> int:
    if not INVENTORY.exists() or not KEYS.exists():
        print("missing inventory or registered_keys.txt", file=sys.stderr)
        return 1

    parser = argparse.ArgumentParser(
        description="Stdlib coverage: list sections, or detail one section.",
    )
    parser.add_argument(
        "section",
        nargs="?",
        help="section name to detail (e.g. Array, String, 'free functions')",
    )
    parser.add_argument(
        "--all",
        action="store_true",
        help="in list mode, include sections with no coverage yet",
    )
    args = parser.parse_args()

    cov = Coverage()

    if args.section is None:
        return cmd_list(cov, include_all=args.all)

    section = cov.resolve(args.section)
    if section is None:
        print(f"unknown section: {args.section!r}", file=sys.stderr)
        candidates = [
            s
            for s in cov.types_inv
            if not s.startswith("_") and args.section.lower() in s.lower()
        ]
        if candidates:
            print("did you mean: " + ", ".join(sorted(candidates)[:10]), file=sys.stderr)
        else:
            print("run without arguments to list sections", file=sys.stderr)
        return 2
    return cmd_detail(cov, section)


if __name__ == "__main__":
    raise SystemExit(main())
