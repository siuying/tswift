# Builtin class-backed types (reference semantics for Foundation classes)

**Status:** planned — supersedes the "URLSessionDataTask value semantics"
known limitation in ADR-0011.

## Problem

Several Foundation types that are **classes** in real Swift are faked as
`SwiftValue::Struct` in the runtime. Struct backing forces value semantics:
mutations write back to the bound variable via `Outcome::receiver`, so

- bindings must be `var` (real-world Swift uses `let formatter = DateFormatter()`
  then mutates properties — currently rejected or silently wrong);
- aliases and closure captures do not observe mutations;
- `===` identity and live shared state (e.g. `task.progress`) are impossible.

## What already exists (no new value model needed)

- `SwiftValue::Object(Rc<RefCell<ClassObj>>)` — full reference semantics,
  `weak`/`unowned`, identity.
- Member **reads** on Objects work without a user `ClassDef`
  (`interp.rs` reads `ClassObj` fields directly).
- Member **writes** on Objects mutate shared storage via `set_object_field`
  (`storage.rs:set_named_member`) — legal on `let` bindings, as in Swift.

## The actual gaps (interpreter dispatch, `tswift-core`)

1. `BuiltinReceiver::of` (`stdlib.rs:409`) has no `SwiftValue::Object` arm —
   builtin intrinsics are unreachable from Object receivers.
2. `try_class_instance_method` (`dispatch.rs:1250`) intercepts every Object
   receiver and errors when there is no user `ClassDef`, instead of falling
   through to the builtin-intrinsic layer.
3. `read_object_member` does not consult the builtin property registry
   (needed for computed getters like `NumberFormatter.numberStyle`).
4. `ClassObj` is not exported from `tswift-core`'s lib.rs.
5. Display/`print` formatting of ClassDef-less Objects needs checking against
   current struct rendering (golden-test risk).

## Inventory

| Type | Real Swift | Today | Action |
|---|---|---|---|
| `URLSessionDataTask` | class | Struct + var-only caveat | migrate (Phase 1) |
| `Progress` | class | Struct snapshot field | migrate, share with task (Phase 1) |
| `DateFormatter` | class | Struct | migrate (Phase 2) |
| `ISO8601DateFormatter` | class | Struct | migrate (Phase 2) |
| `NumberFormatter` | class | Struct | migrate (Phase 2) |
| `URLSessionConfiguration` | class | Struct | migrate (Phase 3) |
| `URLSession` | class | Struct | migrate (Phase 3) |
| `JSONEncoder`/`JSONDecoder` | class | Struct (core `coding.rs`) | migrate (Phase 4) |
| `PropertyListEncoder`/`Decoder` | class | Struct (core) | migrate (Phase 4) |
| `URLResponse`/`HTTPURLResponse` | class (immutable) | Struct | defer — observationally fine except `===`; tripwire: a script needs response identity |
| `Date`, `Calendar`, `Decimal`, `Data`, `URL(Request/Components)`, `IndexPath/Set`, `UUID`, `Measurement`, `DateComponents` | structs | Struct | correct as-is |

## Phases (each lands separately, presubmit green)

### Phase 0 — interpreter seam (behavior-preserving)

- Export `ClassObj` from `tswift-core`.
- `BuiltinReceiver::of`: add `Object(o)` arm → `from_type_name(&o.borrow().class_name)`.
- `try_class_instance_method`: return `Ok(None)` (fall through to builtin
  layer) when the class has **no user ClassDef** and the name maps to a
  `BuiltinReceiver`. User classes keep shadowing builtins.
- `read_object_member`: for ClassDef-less Objects, consult the builtin
  property registry (same precedence as the Struct path), then raw fields.
- Verify Display/print of ClassDef-less Objects matches struct rendering.
- Unit tests: fall-through dispatch, user-class shadowing, property getter.
- No builtin constructs Objects yet → zero behavior change.

### Phase 1 — URLSessionDataTask + Progress (`tswift-foundation/urlsession.rs`)

- `task_value` returns `SwiftValue::Object`; `cancel`/`resume` mutate via
  `RefCell`, registered `mutating: false` (no write-back → `let` legal).
- `progress` field holds a shared `Progress` Object updated in place —
  aliases of `task.progress` observe updates (fixes a second latent divergence).
- Delegate callbacks receive the **live** task object (mid-flight state/counters
  observable — matches Foundation).
- Tests: `let task` + `resume()`; alias/closure capture observes state; `===`;
  existing delegate fixtures stay green.

### Phase 2 — formatters (`formatter.rs`, `numberformatter.rs`)

- Constructors return Objects; property getters accept Object receivers.
- Headline fidelity win: `let f = DateFormatter(); f.dateFormat = "..."` works.

### Phase 3 — URLSession + URLSessionConfiguration

- Config: `let config = URLSessionConfiguration.default; config.X = ...` works.
  `.default`/`.ephemeral` return a fresh object per access (matches Foundation).
- Session init **copies** the configuration (Foundation-documented behavior —
  post-init config mutations must not affect the session).
- `URLSession.shared`: intern one Object per interpreter so `===` holds.

### Phase 4 — coders (`tswift-core/interp/coding.rs`)

- `JSONEncoder`/`JSONDecoder`/`PropertyListEncoder`/`Decoder` Object-backed;
  `let encoder = JSONEncoder(); encoder.outputFormatting = ...` works.

### Phase 5 — docs & site

- New ADR: builtin class-backed types (dispatch seam + inventory); mark the
  ADR-0011 limitation superseded.
- Update `urlsession.rs` module doc, `frameworks/foundation/scope.toml`
  (`urlsession_task_semantics`), website `status/foundation.mdx` — remove
  "must be `var`" caveats (via update-website skill).

## Risks

- **Golden output diffs**: `print(formatter)`/`print(task)` rendering may change
  — audit CLI fixture expectations per phase.
- **Foundation code cloning `StructObj`**: resume/cancel paths rebuild structs;
  rewrite to interior mutation carefully (no stale snapshots passed to
  delegates).
- **Other `BuiltinReceiver::of` call sites** now seeing Objects: only reachable
  once a builtin constructs Objects; covered per-phase by that type's tests.
- **Session copies config** semantics easy to get wrong — test explicitly.
