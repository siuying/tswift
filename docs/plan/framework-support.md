# Plan — Framework Support (Foundation, SwiftUI, …)

**Status:** proposal
**Date:** 2026-06-27
**Reference toolchain / SDK:** Swift **6.3.2** (`swift-6.3.2-RELEASE`) + matching macOS SDK
**Related:**
- `docs/plan/stdlib-support.md` — the proven prototype this generalizes
- `tools/stdlib-inventory/` — `extract.py` / `coverage.py` / `registered_keys.txt`
- `docs/swift-runtime/feature-checklist.md` — hand-written feature tiers
- `crates/qswift-std` — the stdlib registry (the template for per-framework crates)

## 1. Problem statement

The runtime has a frontend + a stdlib runtime. The next layer of real Swift
programs depends on **closed-source frameworks** — first Foundation
(`Data`/`URL`/`Date`/`UUID`/`JSONEncoder`…), later SwiftUI. Unlike the stdlib,
these are not pure Swift we can read; their *behaviour* must be reimplemented in
Rust and validated against real `swiftc`.

We already have a measurement loop that works for **one** framework (the stdlib).
It is hardcoded to the stdlib in three places:

- `extract.py` resolves only `Swift.swiftinterface`.
- `coverage.py` hardcodes paths, the `SEQUENCE_TYPES`/`CORE_MEMBERS` rules, and
  the out-of-scope buckets.
- `registered_keys.txt` is dumped from the single `qswift-std` registry.

To build a roadmap that extends to Foundation and beyond, we generalize this loop
into a **framework-parameterized** system. The four requirements below map 1:1 to
four deliverables.

## 2. Requirement 1 — reliably fetch the full framework surface

**Finding (verified on this machine):** Foundation and SwiftUI ship the *same*
`.swiftinterface` format as the stdlib, in the SDK rather than the toolchain:

| Framework  | Interface source | Lines |
|------------|------------------|------:|
| stdlib     | `<toolchain>/usr/lib/swift/macosx/Swift.swiftmodule/arm64-apple-macos.swiftinterface` | 53,580 |
| Foundation | `<sdk>/System/Library/Frameworks/Foundation.framework/Versions/Current/Modules/Foundation.swiftmodule/arm64e-apple-macos.swiftinterface` | 24,249 |
| SwiftUI    | `<sdk>/System/Library/Frameworks/SwiftUI.framework/Versions/Current/Modules/SwiftUI.swiftmodule/arm64e-apple-macos.swiftinterface` | 27,131 |

So the **same brace-depth surface extractor works unchanged** in principle. The
generalization is to stop hardcoding the source and resolve it from a manifest:

```
tools/framework-inventory/        # renamed/generalized from stdlib-inventory
  extract.py                      # now: extract.py --framework Foundation
  coverage.py                     # now: coverage.py --framework Foundation [section]
  frameworks.toml                 # source resolution + scope (req 2), see below
frameworks/
  stdlib/        inventory.md  registered_keys.txt  scope.toml
  foundation/    inventory.md  registered_keys.txt  scope.toml
  swiftui/       inventory.md  registered_keys.txt  scope.toml
```

`frameworks.toml` resolves the interface path per framework using `xcrun
--show-sdk-path` and the toolchain pin, so regeneration is one command:

```sh
python3 tools/framework-inventory/extract.py --framework foundation \
  > frameworks/foundation/inventory.md
```

**Extractor caveats to harden (vs the stdlib path):** Foundation/SwiftUI
interfaces carry more `@available`, `@MainActor`, property wrappers
(`@State`/`@Binding`), result builders (`@ViewBuilder`), and ObjC-imported
declarations. `extract.py`'s `is_internal` filter and member regex need a
framework-aware ruleset (the only real code change in req 1).

## 3. Requirement 2 — define what is in scope (declarative)

Today "scope" is **implicit and hardcoded** in `coverage.py` (`SEQUENCE_TYPES`,
`CORE_MEMBERS`, the §7.2 out-of-scope unsafe/reflection buckets). For multiple
frameworks this must become a **declarative, reviewable manifest** —
`frameworks/<name>/scope.toml`:

```toml
# frameworks/foundation/scope.toml
[meta]
framework = "Foundation"
reference = "swiftc 6.3.2 / MacOSX SDK"
stance    = "mirror swift-corelibs-foundation semantics; document Darwin gaps"

# Types we intend to implement, in priority order (the roadmap spine).
[[tier]]
id = "F1"
title = "Value primitives"
types = ["Data", "UUID", "IndexPath", "IndexSet"]

[[tier]]
id = "F2"
title = "URL + components"
types = ["URL", "URLComponents", "URLQueryItem"]

[[tier]]
id = "F3"
title = "Date & time"
types = ["Date", "TimeInterval", "Calendar", "DateComponents", "DateFormatter", "ISO8601DateFormatter"]

[[tier]]
id = "F4"
title = "Numbers & formatting"
types = ["Decimal", "NumberFormatter", "Measurement"]

[[tier]]
id = "F5"
title = "Coding"
types = ["JSONEncoder", "JSONDecoder", "PropertyListEncoder"]

# Explicitly excluded, with rationale — keeps the denominator honest.
[out_of_scope]
"filesystem"   = ["FileManager", "FileHandle"]            # host I/O, sandbox
"networking"   = ["URLSession", "URLRequest", "Host"]     # async + sockets
"objc-runtime" = ["NSObject", "NSCoding", "Bundle"]       # ObjC bridging
"distributed"  = ["NSXPCConnection", "NSUserActivity"]
```

This single file is the answer to "what is in scope": it drives the roadmap
ordering **and** the coverage denominator. `coverage.py` reads it instead of
hardcoding category allowlists. Out-of-scope types are dropped from the total so
the percentage reflects the real target (the principle already resolved in
stdlib-support §7.1: *capability-driven, not pure-inventory %*).

## 4. Requirement 3 — script to track implemented & validated

The three-state model is already correct and reused verbatim:

- **missing** — not in the framework's registry.
- **implemented** — in the registry (declared coverage).
- **verified** — in the registry *and* exercised by a passing CLI golden fixture.

Generalizations to `coverage.py`:

1. **Per-framework registry signal.** Each framework gets its own runtime crate
   (`qswift-foundation`, later `qswift-swiftui`) exposing
   `registered_keys() -> Vec<String>`, dumped by a `dump_registered_keys` test to
   `frameworks/<name>/registered_keys.txt` — exactly the `qswift-std` pattern
   (`crates/qswift-std/src/lib.rs:44`). Cannot drift; reads the live registry.
2. **Per-framework fixtures.** Reuse the executing CLI golden harness
   (`crates/qswift-cli/tests/fixtures/*.swift` + `.expected`). Tag framework
   fixtures with a subdir or prefix so the `verified` signal is scoped.
3. **Scope-aware denominator.** Read `scope.toml`; classify out-of-scope members
   into a fifth bucket excluded from the total.
4. **Same UX.** `coverage.py --framework foundation` (overall roll-up) and
   `coverage.py --framework foundation Date` (member detail).

The `stdlib-coverage` skill generalizes to a `framework-coverage` skill with the
same top-down workflow (refresh snapshot → overall → drill into a section).

## 5. Requirement 4 — generalize into an extensible structure

The unifying abstraction is a **Framework Descriptor** — five fields that make a
framework a plug-in to the same loop:

| Field | stdlib | Foundation | SwiftUI |
|-------|--------|-----------|---------|
| **interface source** | toolchain `Swift.swiftinterface` | SDK `Foundation.swiftinterface` | SDK `SwiftUI.swiftinterface` |
| **scope manifest** | `frameworks/stdlib/scope.toml` | `…/foundation/scope.toml` | `…/swiftui/scope.toml` |
| **runtime crate** | `qswift-std` | `qswift-foundation` | `qswift-swiftui` |
| **registry dump** | `registered_keys.txt` | `registered_keys.txt` | `registered_keys.txt` |
| **fixtures** | CLI goldens | CLI goldens (tagged) | CLI goldens (tagged) |

Everything else — extractor, coverage classifier, three-state model, skill
workflow, "done = dispatched + CLI fixture + frontend fixture + matches swiftc" —
is **shared and framework-agnostic**. Adding a framework becomes:

1. Add a row to `frameworks.toml` (source resolver).
2. Write `frameworks/<name>/scope.toml` (the roadmap + denominator).
3. `extract.py --framework <name>` → `inventory.md`.
4. Scaffold `qswift-<name>` crate with `registered_keys()` + `dump_*` test.
5. Implement tier-by-tier, each slice = registry entries + CLI fixture +
   frontend fixture + coverage/checklist update.

**SwiftUI caveat (why the descriptor is enough but not trivial):** SwiftUI's
surface is `@ViewBuilder`/result-builder + property-wrapper driven and needs a
*render/diff host*, not value semantics. The descriptor still measures it
correctly; the runtime work is larger and gated behind its own ADR. The
measurement loop should land **before** any SwiftUI runtime work so scope is
visible from day one.

## 6. Deliverables

- [ ] Generalize `tools/stdlib-inventory/` → `tools/framework-inventory/` with
      `--framework`; keep a stdlib shim for back-compat.
- [ ] `frameworks.toml` source resolver (toolchain + `xcrun` SDK path).
- [ ] Framework-aware `extract.py` filter ruleset (availability/wrappers/builders).
- [ ] `frameworks/<name>/scope.toml` schema + `frameworks/foundation/scope.toml`
      (F1–F5 above) as the first roadmap.
- [ ] `coverage.py --framework` with scope-aware denominator + out-of-scope bucket.
- [ ] `qswift-foundation` crate skeleton with `registered_keys()` + dump test.
- [ ] `framework-coverage` skill (generalized from `stdlib-coverage`).
- [ ] Foundation tier F1 (`Data`/`UUID`) as the end-to-end proof slice.

## 7. Open decisions (for grilling before build)

- **Reference for behaviour.** Darwin Foundation vs open
  swift-corelibs-foundation — proposal: mirror corelibs semantics, document
  Darwin-only gaps the way float/regex gaps are documented (stdlib-support §3.3).
- **One registry crate per framework vs a namespaced shared registry.** Proposal:
  one crate per framework (clean ownership, mirrors `qswift-std`), keys namespaced
  by receiver type as today.
- **Fixture tagging convention** for the per-framework `verified` signal
  (subdir vs filename prefix).
- **Scope manifest format** (TOML proposed; could be YAML/JSON to match existing
  tooling).
- **SDK pin & portability** — interface paths are macOS/Xcode-specific; CI on
  Linux would use swift-corelibs interfaces. Record the pin like `.swift-version`.
