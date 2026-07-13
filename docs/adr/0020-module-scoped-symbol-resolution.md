# ADR-0020: Module-scoped symbol resolution — honor `import`, resolve same-name members by receiver module

- **Status:** Accepted
- **Date:** 2026-07-13
- **Context slice:** interpreter symbol resolution / framework registry
- **Builds on:** ADR-0006 (SwiftUI render host, the name-only struct-method seam), ADR-0019 (Charts reuses that seam)
- **Drives:** `docs/plan/module-system.md`

## Context

Frameworks (`Swift` stdlib, `Foundation`, `SwiftData`, `SwiftUI`, `Charts`) are
installed into one interpreter by their `install()` functions. Registration is
**flat and global**: `register_free_fn`, `register_struct_method`, … drop symbols
into name-keyed tables with no module tag. `import` is parsed into an `ImportDecl`
and then **ignored** (`interp.rs`: `ImportDecl => Ok(Void) // hoisted/ignored`).

Two tables back method dispatch:

- `intrinsics: (BuiltinReceiver, name)` — **type-scoped** (Int/String/Array…).
- `struct_methods: name` — **name-only, global**: the SwiftUI view-modifier seam,
  applied to *any* struct receiver by name (`dispatch.rs::try_struct_receiver_method`).

The name-only seam is where the problem bites. SwiftUI and Charts both register a
`foregroundStyle` (and `cornerRadius`/`opacity`/`offset`/…) modifier. In real
Swift these are **distinct declarations** — protocol-extension methods on `View`
vs `ChartContent` — chosen by the receiver's static type and the imported modules.
In tswift they collapse to one global name, so the **last-installed** framework's
handler silently wins for all receivers. Today the handlers are byte-identical, so
there is no observable bug — only order-dependent coupling and future-drift risk.

The receiver's concrete type name (`BarMark`, `Text`, …) **is** available at
dispatch; the seam just ignores it.

## Decision

Give the interpreter a lightweight **module** notion and resolve the name-only
seam by the receiver's owning module, honoring `import` as a filter.

1. **Module-tagged registration (who owns a symbol).** Add a `current_module`
   scope to the registry: `install()` brackets its registrations with
   `interp.module("Charts", |i| { … })` (or `begin_module`/`end_module`). Every
   symbol registered inside is stamped with that `ModuleId`; constructors also
   populate a `type_module: TypeName → ModuleId` map. `Swift` (stdlib) is the
   always-present base module. No `register_*` signature changes.

2. **Receiver-module-scoped struct-method dispatch (prevents the conflict).**
   `struct_methods` becomes name → per-module candidates. At dispatch of
   `recv.m(...)`, resolve the receiver's module via `type_module[recv.type_name]`
   and pick the candidate owned by that module (or a module it re-exports), else
   the base/shared handler. `BarMark.foregroundStyle` → Charts', `Text` → SwiftUI's.
   Deterministic and install-order-independent. SwiftUI and Charts each keep their
   own `foregroundStyle`; neither clobbers the other.

3. **`import` awareness, named tiers (honor imports without breaking snippets).**
   The host collects the program's imported modules (from `ImportDecl`) and passes
   the set to the interpreter.
   - **Lenient (default):** every installed module is resolvable; imports act only
     as a tie-breaker when receiver-module resolution is ambiguous. This preserves
     today's behavior for the **36/40 fixtures that omit imports**.
   - **Strict (target end-state):** a symbol from module `M` is resolvable only if
     `M` is imported; otherwise a "cannot find … in scope" diagnostic. Reached by
     first migrating every first-party Swift source to add the `import`s it needs
     (a behavior-preserving codemod — imports are no-ops while gating is off), then
     flipping gating on. `Swift` (stdlib) is always implicitly imported.

### Faithfulness (target end-state)

- **Faithful:** module-scoped resolution of same-name members by receiver;
  install-order independence; `import` required for framework-symbol visibility
  (stdlib implicit), matching real Swift.
- **Migration tiers:** lenient resolution (Phase C) is an *intermediate* state that
  keeps the tree green while the fixture codemod (Phase D1) runs; strict gating
  (Phase D2) is the end-state, not a deferred maybe.

## Consequences

- **Good:** deletes the order-dependent global-clobber class of bug; `SwiftUI` and
  `Charts` (and any future frameworks) can share member names faithfully; the
  registry gains an honest module boundary that later features (import-gating,
  diagnostics, per-module docs/coverage) build on.
- **Cost / risk:** `struct_methods` gains a per-module dimension and dispatch does
  one `type_module` lookup; the migration must land behavior-preserving (Phase A/B)
  before any visibility change (Phase C/D). The `type_module` map assumes globally
  unique type names (already true in this runtime).
- **Out of scope (tripwires for a follow-up ADR):** submodule / `@_exported import`
  re-export graphs beyond a simple depends-on list; per-file (vs per-program) import
  scoping; sema-level (pre-run) import diagnostics with fix-its.

## Notes

- Phasing, signals, and slices live in `docs/plan/module-system.md`.
- Only 4/40 SwiftUI fixtures import anything (all `SwiftData`+`SwiftUI`); 0 Charts
  fixtures import — the empirical basis for the lenient default.
