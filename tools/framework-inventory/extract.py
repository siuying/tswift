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
import json
import os
import re
import subprocess
import sys
import tempfile
import tomllib
from collections import defaultdict
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
MANIFEST = Path(__file__).with_name("frameworks.toml")

TYPE_RE = re.compile(
    r"\b(?:struct|enum|class|protocol|actor)\s+([A-Za-z_][A-Za-z0-9_]*)"
)
# Module-qualified extensions: SwiftUI uses `SwiftUI.View` (dot); newer Charts
# interfaces use `Charts::PlottableValue` (double-colon). Capture the final type.
EXT_RE = re.compile(
    r"\bextension\s+(?:[A-Za-z_][A-Za-z0-9_]*(?:::|\.))*([A-Za-z_][A-Za-z0-9_]*)"
)
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


def sdk_root(sdk_name: str | None = None) -> Path:
    sdk = os.environ.get("SDKROOT")
    if sdk:
        return Path(sdk)
    cmd = ["xcrun"]
    if sdk_name:
        cmd += ["--sdk", sdk_name]
    cmd.append("--show-sdk-path")
    return Path(run(cmd))


def find_tool(name: str) -> str:
    """Locate a Swift toolchain binary (`swift-symbolgraph-extract`).

    `xcrun -f` is blocked in some sandboxes, so probe the default Xcode
    toolchain directly before falling back to `xcrun`."""
    candidates = [
        Path(
            "/Applications/Xcode-beta.app/Contents/Developer/Toolchains/"
            f"XcodeDefault.xctoolchain/usr/bin/{name}"
        ),
        Path(
            "/Applications/Xcode.app/Contents/Developer/Toolchains/"
            f"XcodeDefault.xctoolchain/usr/bin/{name}"
        ),
    ]
    for candidate in candidates:
        if candidate.exists():
            return str(candidate)
    try:
        return run(["xcrun", "-f", name])
    except Exception:
        raise SystemExit(f"cannot find {name!r}")


def sdk_ios_version(sdk: Path) -> str:
    """Extract the iOS version baked into the simulator SDK path
    (`.../iPhoneSimulator27.0.sdk` -> `27.0`)."""
    m = re.search(r"iPhoneSimulator([0-9]+(?:\.[0-9]+)?)\.sdk", str(sdk))
    return m.group(1) if m else "26.0"


# Symbol-graph symbol kinds -> the Swift keyword `extract_member`/coverage.py
# expects to see leading the emitted signature.
_SG_KIND = {
    "swift.property": "var",
    "swift.type.property": "var",
    "swift.var": "var",
    "swift.method": "func",
    "swift.type.method": "func",
    "swift.func": "func",
    "swift.func.op": "func",
    "swift.init": "init",
    "swift.enum.case": "case",
    "swift.subscript": "subscript",
}
_SG_TYPE_KINDS = {"swift.class", "swift.enum", "swift.struct", "swift.protocol"}


def extract_symbolgraph(desc: dict) -> tuple[dict[str, set[str]], set[str], list[str]]:
    """Extract an ObjC framework's Swift-imported surface via the symbol graph.

    ObjC frameworks (EventKit) ship an almost-empty `.swiftinterface`; the real
    API is bridged from Clang headers by the importer. `swift-symbolgraph-extract`
    emits that faithful Swift-imported view (real Swift names, arg labels,
    omit-needless-words applied) as JSON, which we fold into the same per-type
    inventory the `.swiftinterface` path produces."""
    module = desc["module"]
    sdk = sdk_root(desc.get("sdk"))
    version = sdk_ios_version(sdk)
    target = desc.get("target", f"arm64-apple-ios{version}-simulator")
    tool = find_tool("swift-symbolgraph-extract")
    with tempfile.TemporaryDirectory() as tmp:
        run(
            [
                tool,
                "-module-name", module,
                "-sdk", str(sdk),
                "-target", target,
                "-minimum-access-level", "public",
                "-output-dir", tmp,
            ]
        )
        graphs = sorted(Path(tmp).glob(f"{module}*.symbols.json"))
        if not graphs:
            raise SystemExit(f"no symbol graph emitted for {module!r}")
        members: dict[str, set[str]] = defaultdict(set)
        free: set[str] = set()
        types: set[str] = set()
        for graph in graphs:
            data = json.loads(graph.read_text(encoding="utf-8"))
            for sym in data.get("symbols", []):
                kind = sym["kind"]["identifier"]
                pc = sym.get("pathComponents", [])
                if kind in _SG_TYPE_KINDS and len(pc) == 1:
                    types.add(pc[0])
                    continue
                keyword = _SG_KIND.get(kind)
                if not keyword or not pc:
                    continue
                name = pc[-1]
                if len(pc) == 1:
                    if keyword == "func":
                        free.add(f"public func {name}")
                    continue
                container = pc[-2]
                if keyword == "case":
                    members[container].add(f"case {name}")
                elif keyword == "init":
                    members[container].add(f"public {name}")
                else:
                    members[container].add(f"public {keyword} {name}")
    for typ in types:
        members.setdefault(typ, set())
    provenance = [f"symbol graph: module {module} / sdk {sdk} / target {target}"]
    return members, free, provenance


def resolve_interfaces(framework: str) -> list[Path]:
    """Resolve every .swiftinterface a framework spans.

    A framework may re-export sibling modules (SwiftUI `@_exported import`s
    SwiftUICore, where the layout primitives live). `additional_framework_paths`
    lists those extra modules; all resolve under the same SDK/toolchain root and
    are merged into one inventory.
    """
    manifest = load_manifest()
    if framework not in manifest:
        names = ", ".join(sorted(manifest))
        raise SystemExit(f"unknown framework {framework!r}; known: {names}")
    desc = manifest[framework]
    kind = desc["kind"]
    candidates = desc.get("interface_candidates", [])

    def find(base: Path) -> Path:
        for candidate in candidates:
            path = base / candidate
            if path.exists():
                return path
        raise SystemExit(f"no .swiftinterface found for {framework!r} under {base}")

    if kind == "toolchain":
        root = toolchain_root(desc)
        bases = [root / desc["relative_path"]]
    elif kind == "sdk-framework":
        root = sdk_root(desc.get("sdk"))
        bases = [root / desc["framework_path"]]
        bases += [root / extra for extra in desc.get("additional_framework_paths", [])]
    else:
        raise SystemExit(f"unsupported descriptor kind: {kind}")
    return [find(base) for base in bases]


def member_signature(line: str) -> str:
    s = line.strip()
    s = re.sub(r"\s*\{.*$", "", s)
    # Drop leading attributes, but keep access/static/mutating. Handles
    # module-qualified attribute names (`@_Concurrency.MainActor`) and arg
    # lists (`@_typeEraser(AnyView)`) that pervade SDK interfaces.
    s = re.sub(r"^(?:@[\w.]+(?:\([^)]*\))?\s+)+", "", s)
    return re.sub(r"\s+", " ", s).strip()


def is_internal(raw: str, sig: str, framework: str) -> bool:
    m = NAME_RE.search(sig)
    if m and m.group(1).startswith("_"):
        return True
    # SPI / private-implementation attributes mark non-public-stable surface.
    # Public decorations (`@_Concurrency.MainActor`, `@preconcurrency`,
    # `@_typeEraser`) are stripped by member_signature and must NOT count as
    # internal — on iOS SwiftUI they adorn nearly every public member.
    if "@_spi" in raw or "_bridge" in raw:
        return True
    if framework == "stdlib" and "ObjectiveC" in sig:
        return True
    # SDK interfaces contain many ObjC-imported declarations. Keep the public
    # Swift spelling, but drop private shim names and SPI fragments.
    if "__" in sig or " NS_SWIFT_NAME" in sig:
        return True
    return False


def extract(paths: list[Path], framework: str | None) -> tuple[dict[str, set[str]], set[str]]:
    lines: list[str] = []
    for path in paths:
        lines.extend(path.read_text(encoding="utf-8").splitlines())
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
                if not is_internal(stripped, sig, fw):
                    members[current_type].add(sig)
        elif depth == 0 and "public func" in stripped and stripped.startswith(("public func", "@")):
            sig = member_signature(stripped)
            if not is_internal(stripped, sig, fw):
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


def emit(sources: list[str], framework: str | None, members: dict[str, set[str]], free_funcs: set[str]) -> None:
    title = framework or "Swift"
    print(f"# {title} API inventory (generated)")
    print()
    if len(sources) == 1:
        print(f"Source: `{sources[0]}`")
    else:
        print("Sources:")
        for source in sources:
            print(f"- `{source}`")
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
        framework = args.framework.lower()
        desc = load_manifest().get(framework, {})
        if desc.get("kind") == "objc-framework":
            members, free_funcs, sources = extract_symbolgraph(desc)
            emit(sources, framework, members, free_funcs)
            return 0
        paths = resolve_interfaces(framework)
    elif args.path:
        paths = [Path(args.path)]
        framework = None
    else:
        parser.error("pass --framework or a .swiftinterface path")
    members, free_funcs = extract(paths, framework)
    emit([str(p) for p in paths], framework, members, free_funcs)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
