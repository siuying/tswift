#!/usr/bin/env python3
"""Five-state stdlib coverage report.

Coverage is a pure join over canonical sets of *semantic* stdlib keys
(`print`, `Array.append`, `Optional.map`, `Sequence.map`, …):

* **inventory**   — every member declared in ``stdlib-inventory.md``.
* **registered**  — what the qswift-std registry actually wires up.
* **exercised**   — what a *passing* golden fixture actually dispatched.

Each inventory member is classified, type-scoped (no global token matching):

* **core**        — an operator, `subscript`, or `init`. These are evaluated by
  the interpreter core (`ops::binary`, `eval_subscript`, the constructor path),
  never by the std registry, so they can never be "registered". Counting them as
  registry *missing* understates real coverage; they get their own bucket.
* **out-of-scope** — past the runtime's scope ceiling (`is_out_of_scope`): unsafe
  / pointer / memory APIs and reflection hooks the runtime will never implement.
  Excluded from the denominator so the percentage reflects the real target
  (``docs/plan/stdlib-support.md`` §7.2).
* **missing**     — a method/property the registry does not wire up.
* **implemented** — registered but never exercised by a passing fixture.
* **verified**    — registered *and* exercised by a passing fixture.

The five buckets partition every inventory member, so per type
``core + out-of-scope + missing + implemented + verified == total``. Two headline
numbers, both over the *in-scope* total ``(total - out-of-scope)``:
``%covered = (core + implemented + verified) / in_scope`` (everything the runtime
handles, registry or core) and ``%verified = verified / in_scope`` (the
behaviourally-proven subset). The seam boundary — core owns
operators/subscripts/inits; the std registry owns methods, computed properties,
and free functions — is documented in ``docs/plan/stdlib-support.md`` §4.2.

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
# Swift operator characters, including `?` and `.` so `??`, `...`, and `..<`
# parse as their operator token rather than bleeding into a following name.
_OP_CHARS = r"-+*/<>=!%&|^~?."
_NAME_RE = re.compile(rf"`?(?P<name>[A-Za-z_][A-Za-z0-9_]*|[{_OP_CHARS}]+)")
# A semantic key made entirely of operator characters (`+`, `&<<`, `??`, `...`).
_OPERATOR_RE = re.compile(rf"^[{_OP_CHARS}]+$")


# Members the runtime deliberately does not implement (the plan's scope ceiling,
# `docs/plan/stdlib-support.md` §7.2). Two narrow, mechanical categories:
#   1. unsafe / pointer / memory APIs, and
#   2. reflection / debugging hooks.
# Everything else the runtime skips — the index/iterator model, the utf views,
# bit-pattern accessors — stays `missing` (a visible, deferred capability), so
# this allowlist is intentionally tight. Exact names plus a few prefixes.
_OUT_OF_SCOPE_EXACT = frozenset({
    # reflection / debugging hooks
    "customMirror", "customPlaygroundQuickLook",
    # raw memory / spans
    "span", "mutableSpan", "utf8Span",
    # unsafe bit reinterpretation
    "bitPattern", "unsafeBitCast", "unsafeDowncast",
    # C strings / closures over unsafe storage
    "withCString", "withUTF8", "withMutableCharacters",
    # va_list / lifetime
    "getVaList", "withVaList", "withExtendedLifetime", "extendLifetime",
})
_OUT_OF_SCOPE_PREFIXES = ("withUnsafe", "withContiguous")


def is_out_of_scope(member: str) -> bool:
    """True if `member` is past the runtime's scope ceiling (unsafe/reflection).

    Out-of-scope members are excluded from the coverage denominator so the
    percentage reflects the real target rather than APIs the runtime will never
    implement. Deliberately narrow: only unsafe/pointer/memory APIs and
    reflection hooks. Bit-pattern accessors (`exponentBitPattern`), the index
    model (`startIndex`), and the utf views stay in scope as `missing`.
    """
    if member in _OUT_OF_SCOPE_EXACT:
        return True
    return member.startswith(_OUT_OF_SCOPE_PREFIXES)


def is_core_member(member: str) -> bool:
    """True if `member` is handled by interpreter core, not the std registry.

    Operators (`+`, `==`, `&<<`), `subscript`, and `init` are evaluated by
    core eval / the constructor path and can never appear in the registry.
    """
    return member in ("subscript", "init") or bool(_OPERATOR_RE.match(member))


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


def classify(
    ty: str,
    member: str,
    reg: tuple[set[str], dict[str, set[str]], set[str]],
    ex: tuple[set[str], dict[str, set[str]], set[str]],
) -> str:
    """Assign one member to a bucket: core/oos/verified/implemented/missing."""
    if is_core_member(member):
        return "core"
    if is_out_of_scope(member):
        return "out-of-scope"
    _, reg_by_type, reg_seq = reg
    _, ex_by_type, ex_seq = ex

    def has(by_type: dict[str, set[str]], seq: set[str]) -> bool:
        if member in by_type.get(ty, ()):
            return True
        return ty in SEQUENCE_TYPES and member in seq

    if not has(reg_by_type, reg_seq):
        return "missing"
    return "verified" if has(ex_by_type, ex_seq) else "implemented"


def compute_report(
    free_inv: set[str],
    types_inv: dict[str, set[str]],
    reg: tuple[set[str], dict[str, set[str]], set[str]],
    ex: tuple[set[str], dict[str, set[str]], set[str]],
    report_types: list[str],
) -> dict:
    """Join inventory against registry/exercise sets into a report structure.

    `reg`/`ex` are `(free, by_type, seq_algos)` triples as returned by
    `load_keys`. `report_types` bounds which types appear (and which count
    toward the overall denominator) — typically the registry-touched types.
    """
    free_reg, _, _ = reg
    free_ex, _, _ = ex
    types: dict[str, dict[str, int]] = {}
    for ty in report_types:
        members = types_inv.get(ty, set())
        if not members:
            continue
        counts = {"core": 0, "oos": 0, "missing": 0, "impl": 0, "verif": 0}
        for m in members:
            state = classify(ty, m, reg, ex)
            bucket = {
                "implemented": "impl",
                "verified": "verif",
                "out-of-scope": "oos",
            }.get(state, state)
            counts[bucket] += 1
        counts["total"] = len(members)
        types[ty] = counts

    # Free functions can be operators too (`==`, `??`, `...`, `~=`): those are
    # core-eval, the same as type-scoped operators. Only named free functions
    # are the registry's responsibility.
    in_scope = [
        f for f in free_inv if not is_core_member(f) and not is_out_of_scope(f)
    ]
    f_core = sum(1 for f in free_inv if is_core_member(f))
    f_oos = sum(1 for f in free_inv if not is_core_member(f) and is_out_of_scope(f))
    f_impl = sum(1 for f in in_scope if f in free_reg and f not in free_ex)
    f_verif = sum(1 for f in in_scope if f in free_reg and f in free_ex)
    free = {
        "core": f_core,
        "oos": f_oos,
        "missing": len(in_scope) - f_impl - f_verif,
        "impl": f_impl,
        "verif": f_verif,
        "total": len(free_inv),
    }

    overall = {k: free[k] for k in ("core", "oos", "missing", "impl", "verif", "total")}
    for counts in types.values():
        for k in overall:
            overall[k] += counts[k]
    # Out-of-scope members are past the scope ceiling: exclude them from the
    # denominator so the percentage reflects the real target (§7.2).
    in_scope_total = (overall["total"] - overall["oos"]) or 1
    overall["pct_covered"] = 100 * (overall["core"] + overall["impl"] + overall["verif"]) / in_scope_total
    overall["pct_verified"] = 100 * overall["verif"] / in_scope_total
    return {"types": types, "free": free, "overall": overall, "order": report_types}


def format_report(report: dict, n_types: int) -> str:
    """Render a `compute_report` result as the textual coverage report."""
    types = report["types"]
    lines = ["# Stdlib coverage report", ""]
    header = (
        f"{'type':<20} {'core':>5} {'oos':>5} {'miss':>5} {'impl':>5} "
        f"{'verif':>5} {'total':>5}  {'%cov':>6} {'%ver':>6}"
    )
    lines.append(header)
    lines.append("-" * len(header))
    for ty in report["order"]:
        c = types.get(ty)
        if not c:
            continue
        covered = c["core"] + c["impl"] + c["verif"]
        in_scope = c["total"] - c["oos"]
        pct_cov = 100 * covered / in_scope if in_scope else 0
        pct_ver = 100 * c["verif"] / in_scope if in_scope else 0
        lines.append(
            f"{ty:<20} {c['core']:>5} {c['oos']:>5} {c['missing']:>5} {c['impl']:>5} "
            f"{c['verif']:>5} {c['total']:>5}  {pct_cov:>5.1f}% {pct_ver:>5.1f}%"
        )
    f = report["free"]
    lines.append("-" * len(header))
    lines.append(
        f"{'(free functions)':<20} {f['core']:>5} {f['oos']:>5} {f['missing']:>5} {f['impl']:>5} "
        f"{f['verif']:>5} {f['total']:>5}"
    )

    o = report["overall"]
    in_scope_total = o["total"] - o["oos"]
    lines.append("")
    lines.append("## Overall (targeted types + free functions, out-of-scope excluded)")
    lines.append(f"core (operators/subscripts/inits): {o['core']}/{in_scope_total}")
    lines.append(f"out-of-scope (unsafe/reflection, excluded): {o['oos']}/{o['total']}")
    lines.append(f"covered:  {o['core'] + o['impl'] + o['verif']}/{in_scope_total} ({o['pct_covered']:.1f}%)")
    lines.append(f"verified: {o['verif']}/{in_scope_total} ({o['pct_verified']:.1f}%)")
    lines.append("")
    lines.append(f"inventory totals: {n_types} types, {f['total']} free functions")
    return "\n".join(lines)


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
    reg = load_keys(REGISTERED)
    ex = load_keys(EXERCISED)

    # Report types the registry (or a sequence entry) touches.
    touched = sorted(set(reg[1]) | (SEQUENCE_TYPES & set(types_inv)))
    report = compute_report(free_inv, types_inv, reg, ex, touched)
    print(format_report(report, n_types=len(types_inv)))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
