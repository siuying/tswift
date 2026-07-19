#!/usr/bin/env python3
"""Three-state framework coverage report.

Classifies every in-scope inventory member as:

* missing     — absent from the framework runtime registry;
* implemented — present in the registry; or
* verified    — present in the registry and mentioned by a tagged CLI golden
                fixture.

Usage:
  python3 tools/framework-inventory/coverage.py --framework foundation
  python3 tools/framework-inventory/coverage.py --framework foundation Data
  python3 tools/framework-inventory/coverage.py --framework stdlib Array
"""
from __future__ import annotations

import argparse
import json
import re
import sys
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
MANIFEST = Path(__file__).with_name("frameworks.toml")
FIXTURES = ROOT / "crates/tswift-cli/tests/fixtures"
FREE_SECTION = "(free functions)"

MEMBER_RE = re.compile(
    r"""(?P<kind>func|var|let|case)\s+
        `?
        (?P<name>[A-Za-z_][A-Za-z0-9_]*|[-+*/<>=!%&|^~]+)
    """,
    re.VERBOSE,
)

# --emit-json's 4-state status vocabulary is coarser than the CLI's registry
# states above: "verified" (registered + exercised by a golden fixture)
# collapses to "implemented", and "implemented" (registered but never proven
# by a fixture) collapses to "partial". No new truth is invented — this is
# purely a renaming for the website's consumption. See README.md.
JSON_STATUS = {
    "verified": "implemented",
    "implemented": "partial",
    "missing": "missing",
    "out_of_scope": "out_of_scope",
}


def root_path(value: str) -> Path:
    path = Path(value)
    return path if path.is_absolute() else ROOT / path


def load_manifest() -> dict:
    with MANIFEST.open("rb") as fh:
        return tomllib.load(fh)


def framework_desc(name: str) -> dict:
    manifest = load_manifest()
    if name not in manifest:
        known = ", ".join(sorted(manifest))
        raise SystemExit(f"unknown framework {name!r}; known: {known}")
    return manifest[name]


def parse_inventory(text: str) -> tuple[set[str], dict[str, set[str]]]:
    free, types, _, _ = parse_inventory_full(text)
    return free, types


def parse_inventory_full(
    text: str,
) -> tuple[set[str], dict[str, set[str]], dict[str, str], dict[str, dict[str, str]]]:
    """Like `parse_inventory`, plus a best-effort `member name -> kind`
    lookup (one representative kind per name; overloads collapse to the
    kind of their first occurrence). Used only by `--emit-json`."""
    free: set[str] = set()
    types: dict[str, set[str]] = {}
    free_kinds: dict[str, str] = {}
    type_kinds: dict[str, dict[str, str]] = {}
    current = None
    for line in text.splitlines():
        if line.startswith("## "):
            heading = line[3:].strip()
            if heading.lower().startswith("free function"):
                current = "__free__"
            else:
                current = heading.split("  ")[0].split(" (")[0].strip()
                types.setdefault(current, set())
                type_kinds.setdefault(current, {})
            continue
        if not line.startswith("- `"):
            continue
        result = extract_member_full(line)
        if not result:
            continue
        name, kind = result
        if current == "__free__":
            free.add(name)
            free_kinds.setdefault(name, kind)
        elif current:
            types[current].add(name)
            type_kinds[current].setdefault(name, kind)
    return free, types, free_kinds, type_kinds


def extract_member(line: str) -> str | None:
    result = extract_member_full(line)
    return result[0] if result else None


def extract_member_full(line: str) -> tuple[str, str] | None:
    if re.search(r"\binit[?!]?\b", line):
        return "init", "init"
    if re.search(r"\bsubscript\b", line):
        return "subscript", "subscript"
    m = MEMBER_RE.search(line)
    if not m:
        return None
    return m.group("name"), m.group("kind")


def load_keys(path: Path) -> tuple[set[str], dict[str, set[str]], set[str]]:
    free: set[str] = set()
    by_type: dict[str, set[str]] = {}
    seq_algos: set[str] = set()
    for raw in path.read_text().splitlines():
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


def load_scope(path: Path | None) -> dict:
    if not path or not path.exists():
        return {}
    with path.open("rb") as fh:
        return tomllib.load(fh)


def scoped_types(scope: dict) -> list[str]:
    result: list[str] = []
    for tier in scope.get("tier", []):
        for typ in tier.get("types", []):
            if typ not in result:
                result.append(typ)
    return result


def out_of_scope_types(scope: dict) -> dict[str, str]:
    excluded: dict[str, str] = {}
    for bucket, types in scope.get("out_of_scope", {}).items():
        for typ in types:
            excluded[typ] = bucket
    return excluded


def out_of_scope_members(scope: dict) -> dict[str, dict[str, str]]:
    """Per-type member exclusions: scope.toml `[out_of_scope_members]`.

    Shape (bucket → list of `"Type.member"` keys, same style as registered_keys):

        [out_of_scope_members]
        chart3d = ["View.chart3DPose", "View.chartZScale"]
        chart_a11y = ["ChartContent.accessibilityHidden"]

    Returns `{type: {member: bucket}}` so individual members of an in-scope
    type can leave the coverage denominator without dropping the whole type.
    """
    excluded: dict[str, dict[str, str]] = {}
    for bucket, keys in scope.get("out_of_scope_members", {}).items():
        if not isinstance(keys, list):
            continue
        for key in keys:
            if not isinstance(key, str) or "." not in key:
                continue
            typ, member = key.split(".", 1)
            if not typ or not member:
                continue
            excluded.setdefault(typ, {})[member] = bucket
    return excluded


def table_string_list(scope: dict, table: str, key: str) -> set[str]:
    raw = scope.get(table, {}).get(key, [])
    return set(raw) if isinstance(raw, list) else set()


# Binary/compound operator tokens credited when they appear *whitespace-
# delimited* in a fixture (` a + b `, ` x &<< 3 `). Requiring spaces on both
# sides is what keeps generics (`Set<Int>`), `inout` (`&x`), arrows (`->`),
# unary minus (`-5`), and `//` comments from registering as operator usage.
# Sorted longest-first so the alternation prefers `&<<`/`<=`/`+=` over `&`/`<`/`+`.
_OP_TOKENS = sorted(
    [
        "&<<=", "&>>=",
        "&<<", "&>>", "&&", "||", "===", "!==", "==", "!=", "<=", ">=", "<<", ">>",
        "&+", "&-", "&*", "+=", "-=", "*=", "/=", "%=", "&=", "|=", "^=",
        "~=",
        "+", "-", "*", "/", "%", "<", ">", "&", "|", "^",
    ],
    key=len,
    reverse=True,
)


def _strip_literals(src: str) -> str:
    """Blank out comments and string literals before operator scanning so that
    operators inside `"a < b"` or `// note +` are never counted."""
    src = re.sub(r'"""(?:.|\n)*?"""', '""', src)
    src = re.sub(r"//[^\n]*", " ", src)
    src = re.sub(r"/\*.*?\*/", " ", src, flags=re.S)
    src = re.sub(r'"(?:\\.|[^"\\\n])*"', '""', src)
    return src


def fixture_tokens(framework: str, prefix: str | None) -> set[str]:
    tokens: set[str] = set()
    member_re = re.compile(r"\.([A-Za-z_][A-Za-z0-9_]*)")
    call_re = re.compile(r"\b([A-Za-z_][A-Za-z0-9_]*)\s*\(")
    operator_re = re.compile(
        r"(?<=\s)(?:" + "|".join(re.escape(t) for t in _OP_TOKENS) + r")(?=\s)"
    )
    unary_not_re = re.compile(r"(?<![=!])!(?!=)")

    candidates: list[Path]
    fixture_dir = framework_desc(framework).get("fixture_dir")
    if fixture_dir:
        candidates = list(root_path(fixture_dir).glob("*.swift"))
    elif framework == "stdlib":
        candidates = list(FIXTURES.glob("*.swift"))
    else:
        candidates = []
        if prefix:
            candidates.extend(FIXTURES.glob(f"{prefix}*.swift"))
        subdir = FIXTURES / framework
        if subdir.exists():
            candidates.extend(subdir.glob("*.swift"))

    for swift in candidates:
        src = swift.read_text()
        code = _strip_literals(src)
        tokens.update(member_re.findall(src))
        tokens.update(call_re.findall(src))
        tokens.update(operator_re.findall(code))
        tokens.update(unary_not_re.findall(code))
    return tokens


class Coverage:
    def __init__(self, framework: str):
        self.framework = framework
        self.desc = framework_desc(framework)
        inventory = root_path(self.desc["inventory"])
        registry = root_path(self.desc["registry"])
        scope_path = root_path(self.desc["scope"]) if "scope" in self.desc else None
        if framework == "stdlib" and not registry.exists():
            registry = ROOT / "tools/stdlib-inventory/registered_keys.txt"
        if not inventory.exists() or not registry.exists():
            raise FileNotFoundError(
                f"missing inventory or registered keys for {framework}:\n"
                f"  inventory: {inventory}\n  registry:  {registry}"
            )
        (
            self.free_inv,
            self.types_inv,
            self.free_kinds,
            self.type_kinds,
        ) = parse_inventory_full(inventory.read_text())
        self.free_reg, self.by_type_reg, self.seq_algos = load_keys(registry)
        self.scope = load_scope(scope_path)
        self.used = fixture_tokens(framework, self.desc.get("fixture_prefix"))
        self._scoped = scoped_types(self.scope)
        self._excluded = out_of_scope_types(self.scope)
        self._excluded_members = out_of_scope_members(self.scope)

    def member_state(self, section: str, member: str) -> str:
        if section != FREE_SECTION and section in self._excluded:
            return "out_of_scope"
        if section != FREE_SECTION and member in self._excluded_members.get(section, {}):
            return "out_of_scope"
        if section == FREE_SECTION:
            registered = member in self.free_reg
        else:
            registered = (
                member in self.by_type_reg.get(section, set())
                or member in table_string_list(self.scope, "core_members", section)
                or (
                    section in table_string_list(self.scope, "coverage", "sequence_types")
                    and member in self.seq_algos
                )
                or (section == "Optional" and member in {"map", "flatMap"})
            )
        if not registered:
            return "missing"
        # KNOWN LIMITATION (see README "token-based verification"): fixture
        # tokens are not type-scoped, so a generic member name (init/body/map/
        # description/count/...) is credited "verified" as soon as ANY type
        # exercises that name in a fixture, not necessarily `section`. `init`
        # is partially guarded by also requiring the owning type name; the
        # bare-token path is an upper bound, whereas registry-gated
        # "implemented" is exact. Type-inferring tokens back to receivers would
        # tighten this but destabilizes calibrated baselines — deferred.
        used = member in self.used or (member == "init" and section in self.used)
        return "verified" if used else "implemented"

    def members(self, section: str) -> set[str]:
        if section == FREE_SECTION:
            return self.free_inv
        direct = self.types_inv.get(section, set())
        inherited_by_type = self.scope.get("coverage", {}).get("inherited_members", {})
        inherited = set(inherited_by_type.get(section, [])) if isinstance(inherited_by_type, dict) else set()
        return direct | inherited

    def classify(self, section: str) -> dict[str, list[str]]:
        groups = {"verified": [], "implemented": [], "missing": [], "out_of_scope": []}
        for member in self.members(section):
            groups[self.member_state(section, member)].append(member)
        for values in groups.values():
            values.sort()
        return groups

    def counts(self, section: str) -> tuple[int, int, int, int]:
        groups = self.classify(section)
        verif = len(groups["verified"])
        impl = verif + len(groups["implemented"])
        total = impl + len(groups["missing"])
        return impl, verif, total, len(groups["out_of_scope"])

    def targeted_sections(self, include_all: bool) -> list[str]:
        if include_all:
            if self._scoped:
                return [s for s in self._scoped if s in self.types_inv]
            return sorted(s for s in self.types_inv if not s.startswith("_"))
        if self._scoped:
            return [s for s in self._scoped if s in self.types_inv]
        touched = set(self.by_type_reg) | table_string_list(self.scope, "coverage", "sequence_types")
        return sorted(s for s in touched if self.types_inv.get(s))

    def resolve(self, name: str) -> str | None:
        key = name.strip().lower()
        if key in {"free", "free functions", "(free functions)", "free function"}:
            return FREE_SECTION
        for section in self.types_inv:
            if section.lower() == key:
                return section
        return None

    def include_free_functions(self) -> bool:
        return self.framework == "stdlib" or bool(
            self.scope.get("coverage", {}).get("include_free_functions", False)
        )

    def section_json(self, section: str) -> dict:
        """One section's `--emit-json` payload: members (name/kind/status[/notes])
        plus roll-up counts in the JSON_STATUS vocabulary."""
        groups = self.classify(section)
        kinds = self.free_kinds if section == FREE_SECTION else self.type_kinds.get(section, {})
        out_of_scope_bucket = self._excluded.get(section)
        member_buckets = self._excluded_members.get(section, {})
        members: list[dict] = []
        counts = {"implemented": 0, "partial": 0, "missing": 0, "out_of_scope": 0}
        for state, names in groups.items():
            status = JSON_STATUS[state]
            counts[status] += len(names)
            for name in names:
                entry = {"name": name, "kind": kinds.get(name, "unknown"), "status": status}
                if status == "out_of_scope":
                    bucket = out_of_scope_bucket or member_buckets.get(name)
                    if bucket:
                        entry["notes"] = f"out of scope: {bucket}"
                members.append(entry)
        members.sort(key=lambda m: m["name"])
        counts["total"] = counts["implemented"] + counts["partial"] + counts["missing"] + counts["out_of_scope"]
        display = FREE_SECTION if section == FREE_SECTION else section
        return {"name": display, "members": members, "counts": counts}

    def to_dict(self, include_all: bool = False) -> dict:
        """Full `--emit-json` payload for this framework: one entry per
        targeted section (plus free functions where the framework tracks
        them), and a `totals` roll-up in the same JSON_STATUS vocabulary
        `section_json`'s `counts` use. Status truth is entirely derived from
        the inventory/registry/scope.toml manifests already loaded by
        `Coverage.__init__` — nothing here invents data."""
        sections = [self.section_json(s) for s in self.targeted_sections(include_all)]
        if self.include_free_functions():
            free = self.section_json(FREE_SECTION)
            if free["counts"]["total"]:
                sections.append(free)
        totals = {"implemented": 0, "partial": 0, "missing": 0, "out_of_scope": 0, "total": 0}
        for section in sections:
            for key in totals:
                totals[key] += section["counts"][key]
        return {
            "framework": self.desc.get("display_name", self.framework),
            "sections": sections,
            "totals": totals,
        }


def cmd_list(cov: Coverage, include_all: bool) -> int:
    title = cov.desc.get("display_name", cov.framework)
    print(f"# {title} coverage — sections\n")
    print(f"{'section':<24} {'impl':>6} {'verif':>6} {'total':>6}  {'%verified':>9}")
    print("-" * 60)
    tot_impl = tot_verif = tot_total = excluded = 0
    for section in cov.targeted_sections(include_all):
        impl, verif, total, out = cov.counts(section)
        excluded += out
        if not total:
            continue
        tot_impl += impl
        tot_verif += verif
        tot_total += total
        pct = 100 * verif / total if total else 0
        print(f"{section:<24} {impl:>6} {verif:>6} {total:>6}  {pct:>8.1f}%")

    include_free = cov.include_free_functions()
    f_impl = f_verif = f_total = 0
    if include_free:
        f_impl, f_verif, f_total, _ = cov.counts(FREE_SECTION)
        if f_total:
            print("-" * 60)
            print(f"{FREE_SECTION:<24} {f_impl:>6} {f_verif:>6} {f_total:>6}")

    g_total = tot_total + f_total
    g_impl = tot_impl + f_impl
    g_verif = tot_verif + f_verif
    print("\n## Overall")
    if g_total:
        print(f"implemented: {g_impl}/{g_total} ({100*g_impl/g_total:.1f}%)")
        print(f"verified:    {g_verif}/{g_total} ({100*g_verif/g_total:.1f}%)")
    else:
        print("implemented: 0/0")
        print("verified:    0/0")
    if excluded:
        print(f"out of scope members excluded: {excluded}")
    print(f"\ninventory totals: {len(cov.types_inv)} types, {len(cov.free_inv)} free functions")
    print("\nRun `coverage.py --framework <name> <section>` for member detail.")
    return 0


def cmd_detail(cov: Coverage, section: str) -> int:
    title = cov.desc.get("display_name", cov.framework)
    impl, verif, total, out = cov.counts(section)
    groups = cov.classify(section)
    print(f"# {title} coverage — {section}\n")
    pct = 100 * verif / total if total else 0
    print(f"implemented: {impl}/{total}   verified: {verif}/{total} ({pct:.1f}%)")
    if out:
        print(f"out of scope: {out}")
    print()
    for state in ("verified", "implemented", "missing", "out_of_scope"):
        items = groups[state]
        if not items:
            continue
        print(f"## {state} ({len(items)})")
        for item in items:
            print(f"  - {item}")
        print()
    return 0


def cmd_emit_json(cov: Coverage, include_all: bool, out: Path | None) -> int:
    payload = cov.to_dict(include_all=include_all)
    text = json.dumps(payload, indent=2, sort_keys=False) + "\n"
    if out is None:
        sys.stdout.write(text)
    else:
        out.parent.mkdir(parents=True, exist_ok=True)
        out.write_text(text)
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(description="Framework coverage report")
    parser.add_argument("section", nargs="?", help="section/type name to detail")
    parser.add_argument("--framework", "-f", default="stdlib", help="framework descriptor name")
    parser.add_argument("--all", action="store_true", help="include all targeted sections")
    parser.add_argument(
        "--emit-json",
        action="store_true",
        help="emit machine-readable coverage JSON instead of the text report",
    )
    parser.add_argument(
        "--out",
        type=Path,
        default=None,
        help="write --emit-json output to this path instead of stdout",
    )
    args = parser.parse_args()

    try:
        cov = Coverage(args.framework.lower())
    except FileNotFoundError as e:
        print(str(e), file=sys.stderr)
        return 1

    if args.emit_json:
        return cmd_emit_json(cov, include_all=args.all, out=args.out)

    if args.section is None:
        return cmd_list(cov, include_all=args.all)

    section = cov.resolve(args.section)
    if section is None:
        print(f"unknown section: {args.section!r}", file=sys.stderr)
        candidates = [s for s in cov.types_inv if args.section.lower() in s.lower()]
        if candidates:
            print("did you mean: " + ", ".join(sorted(candidates)[:10]), file=sys.stderr)
        else:
            print("run without arguments to list sections", file=sys.stderr)
        return 2
    return cmd_detail(cov, section)


if __name__ == "__main__":
    raise SystemExit(main())
