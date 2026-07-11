# ADR-0016: SwiftData core surface (`@Model`, `ModelContainer`, `ModelContext`)

- **Status:** Accepted
- **Date:** 2026-07-11
- **Context slice:** Slice 9 — the Swift-facing SwiftData core surface layered
  on the `tswift.db.*` wire from ADR-0015. Ships `@Model` schema derivation,
  `ModelContainer`/`ModelConfiguration`, and `ModelContext` with
  `insert`/`delete`/`save`. `fetch`/`#Predicate`/relationships are deferred.
- **Builds on:** ADR-0015 (the `tswift.db.*` host-service wire + tagged-value
  codec), ADR-0012 (builtin class-backed types), ADR-0014 (host-service
  capability gating), ADR-0005 (single-threaded executor).
- **Related:** `crates/tswift-swiftdata/src/model.rs` (the whole surface),
  `crates/tswift-core/src/stdlib.rs` (`StdContext::nominal_type_info`, the
  generic type-introspection seam), `crates/tswift-cli/tests/fixtures/
  swiftdata_model_crud.swift` (end-to-end fixture).

## Context

The SwiftData surface must (a) learn a user `@Model` class's stored properties
to derive a table schema, and (b) intercept `ModelContainer`/`ModelContext`
construction and mutation, all *without* teaching the generic evaluator spine
anything SwiftData-specific (project rule: framework logic stays out of
`tswift-core`). Real Apple SwiftData is the oracle for observable semantics.

## Decisions

### `@Model` is discovered generically, not macro-expanded

Attributes already reach the runtime on the class declaration
(`NodeKind::Attribute` children, e.g. `@main`/`@dynamicMemberLookup`). Rather
than teach `tswift-swiftdata` to parse AST, we added **one generic seam** to
`tswift-core`: `StdContext::nominal_type_info(type_name) -> Option<NominalTypeInfo>`
returning a type's declaration attributes (with `@` stripped) and its stored
properties (name + declared type). Core assigns these strings no framework
meaning — it is the same "expose a generic capability, let the framework
interpret it" pattern as `BuiltinReceiver::register_extension`. `@Model`
detection is then just `info.attributes.contains("Model")` inside
`tswift-swiftdata`. (`ClassDef` gained an `attributes: Vec<String>` field,
populated at hoist time from the same `Attribute` children `@main` already
reads.)

### Schema: table per class, rowid is the persistent identifier

Each `@Model` class maps to `CREATE TABLE IF NOT EXISTS "<ClassName>"` with one
column per stored property. The implicit SQLite `rowid` is the primary key and
**is** the object's persistent identifier — no separate identifier column is
synthesized (rowid already provides stable, monotonic identity). Type mapping
(stage-1 codec types only):

| Swift type                          | SQLite column |
| ----------------------------------- | ------------- |
| `Int`/`Bool` (+ sized int variants) | `INTEGER`     |
| `Double`/`Float`/`CGFloat`          | `REAL`        |
| `String`                            | `TEXT`        |

A non-optional property gets `NOT NULL`; `T?` allows `NULL`. **`Data` and
`Date` are deferred**: this runtime has no primitive `SwiftValue` for either
(they are Foundation-backed structs), so supporting them would couple
`tswift-swiftdata` to Foundation's value shapes. A `@Model` declaring an
unsupported property type raises a clear, catchable error at
`ModelContainer(for:)` time rather than silently dropping the column.

### `ModelContainer` / `ModelContext` are builtin extension receivers

`ModelContainer`, `ModelConfiguration`, and `ModelContext` are **not** user
nominal types; they are registered natively:

- `ModelContainer(for:)`, `ModelConfiguration(...)`, `ModelContext(_:)` are
  free-function initializers (`register_free_fn`), returning class-backed
  builtin `Object`s (ADR-0012) tagged with those class names.
- `container.mainContext` is a builtin property; `context.insert/delete/save`
  are builtin intrinsics, dispatched through `register_extension` receivers.

Native change-tracking state (the db handle, derived schema, and the
inserted/tracked/deleted sets holding live `Rc<RefCell<ClassObj>>` references)
cannot live inside a `SwiftValue`, so it is held in a **thread-local registry**
keyed by a small integer id the Swift-facing objects carry as a hidden
`__cid`/`__ctxid` field. Safe under ADR-0005 (single-threaded interpreter),
same pattern as core's `http.rs` pending-map.

### Store path: the runtime passes a name, the host decides its meaning

`ModelConfiguration(isStoredInMemoryOnly: true)` selects `":memory:"`;
otherwise the store name is the configuration's `name` (default
`"default.store"`). The runtime only passes that string to `tswift.db.open`;
what it means on disk (a file, a `localStorage` kvvfs slot, a sandbox path) is
the host's business — exactly as ADR-0015 established.

### `save()` is transactional; autosave is OFF

`save()` computes the change plan (inserts, dirty updates by snapshot diff,
deletes), then, if non-empty, brackets the flush in `tswift.db.begin` …
`tswift.db.commit`, rolling back (`tswift.db.rollback`) on any host error and
leaving the tracking sets unchanged. Statements are batched (one INSERT/UPDATE/
DELETE per object, never per-property); dirty detection is a per-object column
snapshot comparison (no `SELECT`-then-`UPDATE`). Inserts capture
`lastInsertRowid`; UPDATE/DELETE address rows by `rowid`. Re-inserting an
already-tracked object is idempotent (identity dedup by `Rc::ptr_eq`).

**Deviation — autosave is OFF.** Real SwiftData's `mainContext` autosaves on
run-loop ticks. This runtime has no run loop to hang autosave off, so callers
must call `save()` explicitly. `autosaveEnabled` is not modelled. Documented
rather than faked.

### Deferred (clean seams left)

- `fetch(_:)` / `#Predicate` / `FetchDescriptor`, relationships (`@Relationship`),
  `@Attribute(.unique)`, cascade deletes.
- A Swift-visible `.persistentModelID` accessor on the model instance (member
  access on a user class routes through its `ClassDef`, not this crate's
  builtin dispatch; the rowid is tracked internally for identity today).
- `Data`/`Date` columns (need a value-shape decision with Foundation).

The schema and open connection already exist, so `fetch` is an additive next
step over the same wire.
