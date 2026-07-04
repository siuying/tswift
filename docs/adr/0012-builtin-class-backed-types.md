# ADR-0012: Builtin class-backed types — reference semantics for Foundation classes

- **Status:** Accepted
- **Date:** 2026-07-04
- **Supersedes:** known limitation "URLSessionDataTask value semantics (deferred)" in ADR-0011
- **Context slice:** Foundation formatters, coders, networking (URLSession / URLSessionDataTask / Progress)
- **Builds on:** ADR-0011 (event-driven HTTP transport), ADR-0005 (cooperative executor)
- **Drives:** `docs/plan/builtin-class-backed-types.md` phases 0–5

## Context

Several Foundation types that are **classes** in real Swift were backed by
`SwiftValue::Struct` in the runtime.  Struct backing forces value semantics:

- Mutations write back only to the bound variable via `Outcome::receiver` — so
  `let formatter = DateFormatter()` then mutating properties was rejected or
  silently wrong.
- Aliases and closure captures do not observe mutations — `task.progress` on a
  captured copy is stale.
- `===` identity and shared live state (e.g. `task.progress` shared with a
  delegate) are impossible.

The infrastructure for reference-typed objects already existed:
`SwiftValue::Object(Rc<RefCell<ClassObj>>)` has full reference semantics, and
`set_object_field` already mutates shared storage in place via the `RefCell`.
The gaps were in the *dispatch seam*: `BuiltinReceiver::of` had no `Object`
arm, and `try_class_instance_method` errored on class-def-less Objects instead
of falling through to the builtin-intrinsic layer.

## Decision

**Migrate Foundation class types to `SwiftValue::Object`-backed values, using
a three-part interpreter seam (the "ClassDef-less fall-through") as the
enabling infrastructure.**

### The dispatch seam (Phase 0)

Three changes make class-backed builtins reach the existing intrinsic layer
without touching user-class dispatch:

1. **`BuiltinReceiver::of` Object arm** (`stdlib.rs`): when the value is
   `SwiftValue::Object`, classify by `class_name` via the existing
   `from_type_name` look-up.  Reachable only once a builtin mints Objects;
   zero behavior change while no builtin does so.

2. **`try_class_instance_method` fall-through** (`dispatch.rs`): when the
   receiver is an Object whose `class_name` has no user `ClassDef` *and* maps
   to a `BuiltinReceiver`, return `Ok(None)` to fall through to the builtin
   intrinsic layer.  User classes own a `ClassDef` so the guard is skipped —
   they keep shadowing builtins unchanged.

3. **`read_object_member` builtin property registry** (`storage.rs`): for
   class-def-less Objects, after raw field and user-computed getter, consult
   the builtin property registry (same precedence as the Struct path in
   `eval_member`).  Raw ClassObj field wins first; then user getter; then
   builtin registry.  The gate keeps user classes on their existing path.

`render_description` (`nominal.rs`) similarly falls back to the builtin
`description` property for class-def-less Objects, then to the struct-form
`ClassName(field: value, …)` Display — identical to the prior struct rendering
so golden fixtures are unaffected.

`mutating: false` is the correct registration for all builtin-class methods:
`set_object_field` mutates the `ClassObj` in place through the `Rc`; no struct
write-back (`Outcome::receiver`) is needed or wanted.  `let` bindings are
therefore legal for all migrated types, matching real Swift.

### Inventory of migrated types (Phases 1–4)

| Type | Real Swift | Migrated | Notes |
|---|---|---|---|
| `URLSessionDataTask` | class | ✅ Phase 1 | `let task` legal; `===` holds; Progress object shared with delegate |
| `Progress` | class | ✅ Phase 1 | shared `Rc` with its task; aliases observe `fractionCompleted` updates |
| `DateFormatter` | class | ✅ Phase 2 | `let f = DateFormatter(); f.dateFormat = "…"` works |
| `ISO8601DateFormatter` | class | ✅ Phase 2 | same as DateFormatter |
| `NumberFormatter` | class | ✅ Phase 2 | same pattern |
| `URLSessionConfiguration` | class | ✅ Phase 3 | `.default` / `.ephemeral` return fresh Objects per qualified access |
| `URLSession` | class | ✅ Phase 3 | `shared` is identity-stable (`===` holds); `init` copies configuration |
| `JSONEncoder` | class | ✅ Phase 4 | `let encoder = JSONEncoder(); encoder.outputFormatting = .prettyPrinted` works |
| `JSONDecoder` | class | ✅ Phase 4 | same pattern |
| `PropertyListEncoder` | class | ✅ Phase 4 | same pattern |

### Types deliberately kept as `SwiftValue::Struct`

These are value types in real Swift or are observationally equivalent to
struct-backed at current fidelity:

`Date`, `Calendar`, `Decimal`, `Data`, `URL`, `URLRequest`, `URLComponents`,
`URLQueryItem`, `IndexPath`, `IndexSet`, `UUID`, `Measurement`,
`DateComponents`.

### Deferred: URLResponse / HTTPURLResponse

`URLResponse` and `HTTPURLResponse` are **immutable** after construction in
practice — no property setter is needed.  The only gap versus real Swift is
`===` identity (response objects from separate requests are not `===`-equal,
matching real Swift; but two aliases of the *same* response object could not
be `===`-compared).  This divergence is harmless for all known scripts.

Tripwire: lift deferral if a script needs response identity (`===` on
`URLResponse` / `HTTPURLResponse`), or if a setter is needed post-construction.

### Session init copies configuration

`URLSession(configuration:)` snapshots all configuration fields into a
independent new Object (matching Foundation's documented contract — post-init
mutations to the original config must not affect the session).

### `.default` / `.ephemeral` shorthand limitation

`URLSessionConfiguration.default` accessed via the **qualified** form returns
a fresh Object per call (factory path through `static_method`).  The `.default`
**shorthand** form resolves through `resolve_implicit_static` (statics map
only) and returns the same pre-allocated Object.

Tripwire: lift if a user reports `.default` shorthand mutation leaking between
`let` bindings — fix requires `resolve_implicit_static` to also consult
`static_methods`.

## Consequences

**Positive**

- `let formatter = DateFormatter()` + property mutation now works as in real
  Swift — the most common Foundation formatter pattern.
- `let task = session.dataTask(…)` is legal; delegate callbacks receive the
  live task Object; aliases of `task.progress` observe updated fractions.
- `URLSession.shared === URLSession.shared` holds.
- Session init copies configuration — post-init config changes are isolated.
- No user-visible behavior change for code that already worked (the dispatch
  seam is behavior-preserving; user classes keep shadowing builtins).
- All existing golden fixtures pass unchanged (ClassDef-less Object Display
  renders identically to the prior struct form).

**Costs and accepted limits**

- **`PropertySetterFn` registry not extended to Objects.**  The setter registry
  (`Fn(container, new) -> container`) returns a replacement value — it fits
  struct copy-on-write semantics but not in-place Object mutation.  Builtin
  class setters are implemented as intrinsics on the Object receiver instead.
  Revisit if a builtin Object needs a validating setter.

- **`.default` shorthand returns a shared mutable Object** (pre-allocated at
  registration time).  The session-init copy contract means
  `URLSession(configuration: .default)` always creates independent sessions,
  so the primary mutation use-case is unaffected.  Documented as a tripwire
  above.

- **`PropertyListDecoder` not migrated.**  No script-visible property setter
  exists for `PropertyListDecoder` in the current implementation; migration
  deferred until a property-set use-case is filed.

## Notes

- Phases 0–4 implementation notes (Rc::clone patterns, borrow discipline,
  dual-registration for statics) live in `notes.md` (coder-to-coder
  scratchpad) and `docs/plan/builtin-class-backed-types.md`.
- The `read_coder_field` / `read_formatter_field` / `read_nf_field` dual-mode
  helpers follow the same pattern: accept `&SwiftValue`, handle both Struct and
  Object arms, drop all `RefCell` borrows before returning — borrow discipline
  ensures no borrow held across `call_closure` / `run_event_driver` calls.
