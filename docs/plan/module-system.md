# Plan — Module-scoped symbol resolution (`import` + receiver-module dispatch)

**Status:** proposed
**Decision record:** `docs/adr/0020-module-scoped-symbol-resolution.md`

Honor `import` and resolve same-name members (e.g. `foregroundStyle` on `View` vs
`ChartContent`) by the receiver's owning module, eliminating the name-only global
struct-method seam's order-dependent clobbering. Migration is phased so the
infrastructure lands behavior-preserving and separately from any visibility change.

## Verification signals

- `cargo test --workspace` (esp. `tswift-swiftui`, `tswift-charts`, `tswift-cli` goldens)
- `cargo test -p tswift-cli --test swiftui_goldens` (UIIR wire output byte-identical)
- web playwright snapshots + iOS UiirRenderer snapshots (host output unchanged)
- a new order-independence test: swiftui-then-charts and charts-then-swiftui install
  orders yield identical dispatch + wire output for shared modifier names.

## Phases

### Phase A — module-tagging infrastructure (behavior-preserving)
- Add `ModuleId` + a `current_module` scope to the registry; `Interpreter::module(name, |i| …)`
  brackets an `install()`; entries stamped with the module; build `type_module: TypeName → ModuleId`.
- Wrap each framework `install()` in its module scope (`Swift`, `Foundation`, `SwiftData`,
  `SwiftUI`, `Charts`). stdlib = base module, always present.
- `struct_methods` unchanged in dispatch (still last-wins). NO behavior change.
- Signal: full suite green, wire output identical.

### Phase B — receiver-module-scoped struct-method dispatch (behavior-preserving)
- `struct_methods`: name → per-module candidates. Dispatch resolves the receiver's
  module via `type_module` and selects that module's handler, else a base/shared one.
- SwiftUI and Charts each keep their own shared-name modifiers; remove the ad-hoc
  duplicate-and-reregister hack in `tswift-charts` (the R4 concern) — now each module
  simply owns its members.
- Behavior-preserving because shared-name handlers are identical today; verified by
  goldens + host snapshots + the new order-independence test.
- Signal: full suite green + order-independence test green.

### Phase C — `import` awareness, lenient (behavior-preserving default)
- Host collects imported modules from `ImportDecl`; pass the set to the interpreter.
- Lenient resolution: all installed modules resolvable; imports only tie-break
  ambiguous receiver-module resolution. 36/40 import-less fixtures keep working.
- Signal: full suite green (no fixture edits needed).

### Phase D1 — import codemod across first-party Swift sources (behavior-preserving)
- Add the needed `import`s to every first-party `.swift` that references a framework
  symbol (Foundation / SwiftUI / SwiftData / Charts). `Swift` stdlib stays implicit.
- Surfaces: `tests/swiftui-fixtures/` (~36 need `import SwiftUI`; 5 chart fixtures also
  `import Charts`), `crates/tswift-cli/tests/fixtures/` (Foundation users; ~69 already
  import), the SwiftUI/Charts render PRELUDE, `examples/`, website presets, iOS example
  apps. Detect used symbols per file → prepend missing imports.
- Imports are no-ops while gating is off, so goldens/`.expected`/snapshots stay
  byte-identical. Regenerate nothing.
- Signal: full suite green with imports added, zero output diffs.

### Phase D2 — flip strict import-gating on (BEHAVIOR CHANGE, end-state)
- Resolution rejects a framework-module symbol whose module is not imported →
  clear "cannot find 'X' in scope" error (stdlib always implicit). Lenient path removed.
- Fixtures already migrated in D1, so the tree stays green; add a few negative tests
  (using a SwiftUI symbol without `import SwiftUI` fails).
- Signal: full suite green + negative-gating tests green.

## Backlog / tripwires
- `@_exported import` / re-export graphs beyond a simple depends-on list.
- Per-file (vs per-program) import scoping.
- Sema-level (pre-run) import diagnostics with fix-its.
