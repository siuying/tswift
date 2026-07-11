# ADR-0015: `tswift.db.*` host-service wire (SQL over the host bridge)

- **Status:** Accepted
- **Date:** 2026-07-11
- **Context slice:** Slice 7 — the `tswift.db` host service
  ([`crates/tswift-core/src/host_services.rs`], already declared) gets its
  first real wire contract and a native (CLI) backing. No SwiftData
  Swift-facing surface (`@Model`, `ModelContext`, …) ships in this slice —
  that is future work layered on top.
- **Builds on:** the host-native function bridge
  (`crates/tswift-core/src/host_bridge.rs`), `HostService`/`Capabilities`
  install-time gating (`crates/tswift-core/src/host_services.rs`), ADR-0005
  (cooperative single-threaded executor), ADR-0014 (the `tswift.defaults.*`/
  `tswift.fs.*` wire contract this one mirrors).
- **Related:** `crates/tswift-swiftdata/src/db.rs` (op names, signatures,
  tagged-value codec), `crates/tswift-cli/src/db.rs` (native backing),
  `crates/tswift-cli/src/sqlite_ffi.rs` (the SQLite FFI binding),
  `crates/tswift-cli/build.rs` (linking).

## Context

`HostService::Database` (`tswift.db` namespace) has existed since the
host-services epic landed, but nothing implemented it — every embedding
already claims to back it (`Capabilities::all()`), but no host function was
ever registered under that namespace, so any caller would have hit "host fn
not registered", not a clean capability diagnostic. This slice fixes that:
defines the wire, ships a `tswift-swiftdata` crate that declares it, and
backs it natively in the CLI with real SQLite.

The eventual consumer is a SwiftData-shaped Swift API (`@Model`,
`ModelContext`, `#Predicate`, …) in a later slice. That API needs a shared,
host-agnostic way to run SQL against a local database — exactly the shape
`tswift.defaults.*`/`tswift.fs.*` already established for key-value storage
and the filesystem. This ADR is that shape for SQL.

## Decision: seven host functions, stage-1 typed, tagged-value payloads

Mirrors ADR-0014's contract pattern: every function is declared with
stage-1 fixed types (`Int`, `String`, `Void` — see `host_bridge::TypeExpr`);
anything that doesn't fit that vocabulary (heterogeneous SQL values, result
rows) travels as a `String` carrying a JSON document, decoded a second time
by the caller — precisely how `tswift.defaults.get`'s `String?` carries an
arbitrary stored value's JSON encoding.

| function | params | returns | throws |
|---|---|---|---|
| `tswift.db.open` | `path: String` | `Int` (handle) | yes |
| `tswift.db.close` | `handle: Int` | `Void` | yes |
| `tswift.db.execute` | `handle: Int, sql: String, params: String` | `String` | yes |
| `tswift.db.query` | `handle: Int, sql: String, params: String` | `String` | yes |
| `tswift.db.begin` | `handle: Int` | `Void` | yes |
| `tswift.db.commit` | `handle: Int` | `Void` | yes |
| `tswift.db.rollback` | `handle: Int` | `Void` | yes |

- `open(path)` opens (creating if absent) the database at `path` — including
  SQLite's special `:memory:` path for an ephemeral database — and returns an
  opaque, process-local `Int` handle. There is no implicit "current database";
  every subsequent call names its handle explicitly, matching how
  `FileManager` calls always name their path explicitly rather than assuming
  a cwd.
- `execute`'s `params` is the JSON encoding of a [`DbValue`] array bound
  positionally (`?` placeholders, 1-based, in array order). Its reply is a
  JSON object `{"rowsAffected": Int, "lastInsertRowid": Int}` — the two
  pieces of post-execute state SQL callers most commonly need
  (`sqlite3_changes`/`sqlite3_last_insert_rowid`), encoded (as a `String`
  wire value) once, rather than as two more host functions.
- `query`'s `params` is the same shape; its reply is a JSON array of
  column-name-keyed objects (each row, in column order — a JSON array of
  `[name, value]` pairs internally, encoded as an object since result column
  names are not guaranteed unique but are effectively always treated as
  such by SQL consumers; see "Rejected: array-of-arrays rows" below for why
  this is a `Vec<(String, DbValue)>`, not a `HashMap`, on the Rust side).

## Decision: tagged JSON encoding for SQL values (`DbValue`)

SQLite's storage classes are `NULL`, `INTEGER`, `REAL`, `TEXT`, `BLOB`. Plain
JSON can represent three of those directly, but:

- A JSON number can't distinguish `REAL 5.0` from `INTEGER 5` on the wire —
  encoding a bound `Double` param as bare `5` would let SQLite (or a
  round-trip back through this wire) silently treat it as an `INTEGER`.
- `BLOB` has no native JSON representation at all.

So every `DbValue` — both directions, bind params and result-row values —
is a single-key **tagged** JSON object: `{"<tag>": <payload>}`.

| Swift value shape | tag | payload |
|---|---|---|
| `nil` | `null` | `null` (payload present but unused) |
| `Int`/`Int64` | `int` | JSON number (integer) |
| `Double`/`Float` | `real` | JSON number (may be integral, e.g. `5.0`) |
| `String` | `text` | JSON string |
| `Data`/`[UInt8]` | `blob` | JSON string, base64 (`tswift_core::base64`, the same codec `tswift.fs.read`/`.write` already use for binary content) |

Implemented once, symmetrically, in `tswift-swiftdata::db::DbValue` — the
same module both the future SwiftData layer and every platform's native
backing import, so there is exactly one encoder/decoder for this shape in
the whole workspace, not one per platform.

#### `real` non-finite and signed-zero values

JSON has no literal for `NaN`/`±inf`, and a bare `-0` re-parses as the
integer `0` (dropping both the sign and the `real` distinction). Encoding a
`real` as a bare `Json::Double` for those cases would therefore emit invalid
JSON (`nan`/`inf`) or silently lose the sign of negative zero. So a `real`
whose value is `NaN`, `+inf`, `-inf`, or `-0` encodes as a **tagged string**
payload instead of a number: `{"real":"nan"}`, `{"real":"inf"}`,
`{"real":"-inf"}`, `{"real":"-0"}`. Every other (finite, non-negative-zero)
`real` stays a bare JSON number. Decoding accepts a JSON number *or* one of
those sentinel strings, so the codec round-trips every `f64` class
losslessly, negative zero's sign bit included.

This keeps the *wire codec* lossless on its own terms. Note that SQLite's
*storage* layer is separately lossy here: binding a `NaN` parameter is stored
as `NULL` (SQLite has no NaN storage class), so a `Real(NaN)` bound and then
read back returns `Null`. That is SQLite's documented semantic, downstream of
and independent from this wire codec — not something the codec papers over.

#### Duplicate result-column names

SQLite does not guarantee result column names are unique (`SELECT a, a`, a
join projecting two `id` columns, …). Since `query`'s reply is a JSON object
per row, duplicate names would produce duplicate JSON keys — legal but
ambiguous (a downstream map keeps only one). `encode_rows` therefore
**disambiguates** repeated names within a row by suffixing the second and
later occurrences with `_1`, `_2`, … (`a`, `a_1`, `a_2`), keeping every
column addressable and column order intact; the first occurrence keeps its
bare name. Pinned by `duplicate_column_names_are_disambiguated`
(`tswift-swiftdata`) and `duplicate_result_columns_are_disambiguated`
(`tswift-cli`).

#### Non-UTF-8 `TEXT`

A `TEXT` column whose bytes are not valid UTF-8 is surfaced as a structured
(catchable) `$thrown` error, not silently lossy-decoded into replacement
characters (which would corrupt the value with no signal). A lossless
blob-style representation of malformed text is intentionally out of scope for
v1: well-formed UTF-8 is the norm, and a clean error beats silent corruption.
Pinned by `sqlite_ffi.rs`'s `non_utf8_text_surfaces_structured_error`.

#### Empty / comment-only SQL

`sqlite3_prepare_v2` returns `SQLITE_OK` with a **null** statement pointer
when the SQL compiles to no statement (empty, all-whitespace, or
comment-only). Dereferencing that null in `step`/`column_*` would be UB, so a
null statement is treated as a completed no-op: `execute` reports 0 rows
affected, `query` returns an empty result set — matching the `sqlite3` CLI's
behavior of quietly accepting empty input. Binding a parameter into a no-op
statement is a structured error (there are no placeholders to bind). Pinned
by `empty_and_comment_only_sql_is_a_noop_statement` (`sqlite_ffi.rs`) and
`empty_sql_is_a_noop` (`tswift-cli`).

### Rejected: widen stage-1 `TypeExpr` instead of tagging

`host_bridge::TypeExpr` could grow a `oneOf`/`any` variant so `params` could
be declared as `[DbValue]` natively instead of a `String` blob. Rejected:
that widens the *general* host-bridge type vocabulary for one framework's
need, when the existing "smuggle a heterogeneous document through a `String`
and JSON-decode it a second time" pattern (already established by
`tswift.defaults.*`) works, is already proven, and keeps `host_bridge`
framework-agnostic per the crate's own module doc ("stage 1: fixed-type
functions only").

### Rejected: array-of-arrays row encoding

Encoding each row as a bare JSON array of tagged values (positional, no
column names inline) would be smaller on the wire, but forces every
consumer to carry the column-name list separately and index into it
correctly — exactly the class of off-by-one bug SwiftData's future
`@Model`-to-column mapping most wants to avoid. Column-name-keyed objects
cost more bytes but make every row self-describing, which matters more here
than wire size (this is a local SQLite call, not a network request).

## Decision: transactions are three sequential ops, not one atomic batch

Considered two shapes for `tswift.db.transaction`:

1. **Sequential `begin`/`commit`/`rollback` ops** (what shipped) — three
   ordinary host-function calls against the same handle, exactly like any
   other multi-step host interaction (e.g. `open` then `execute` then
   `close`).
2. **One atomic "batch" op** — a single host call carrying a list of
   `(sql, params)` statements, executed as one transaction host-side, with
   one combined reply.

Chose (1). The host bridge is **synchronous** and the interpreter is a
**single-threaded cooperative executor** (ADR-0005): between a `begin` call
returning and the matching `commit`/`rollback` call being made, no other
Swift code can run *on this interpreter* and touch the same handle — there
is no concurrent writer that could interleave with an in-flight transaction
the way there would be across OS threads or async tasks. That's exactly the
property that makes a multi-statement transaction expressible as ordinary
sequential calls sound: nothing can observe the database in a
partially-applied state from *this* interpreter between `begin` and
`commit`, and cross-process/cross-connection isolation is SQLite's own job
(its normal transaction semantics), not this wire's.

A batch op would need to invent its own miniature multi-statement wire
shape (nested SQL+params documents) for a benefit that only matters under
concurrent access this bridge doesn't have — added complexity with no
matching problem. If a future slice needs genuine batching for performance
(fewer round-trips), that can be added as an *additional* op without
touching this one; nothing about shipping `begin`/`commit`/`rollback` now
forecloses that.

**Tripwire:** if a platform ever backs `tswift.db.*` with a connection that
*is* genuinely concurrently shared (e.g. a shared worker/service-worker-backed
wasm SQLite with multiple interpreter instances), sequential begin/commit
calls stop being sufficient — revisit then, not speculatively now.

## Decision: native CLI backing links system `libsqlite3` via a minimal hand-written FFI

No crates.io dependency is available offline (`docs/agents/environment.md`),
and `rusqlite`/`libsqlite3-sys` are not in `Cargo.lock`. Two remaining
options:

1. **Hand-written FFI to the system SQLite** (what shipped,
   `crates/tswift-cli/src/sqlite_ffi.rs` + `crates/tswift-cli/build.rs`).
   macOS and Linux both ship a system `libsqlite3` as part of the base OS —
   `build.rs` emits `cargo:rustc-link-lib=dylib=sqlite3` on those two target
   OSes and nothing elsewhere. The FFI surface is deliberately tiny: open,
   close, prepare, bind (null/int64/double/text/blob), step, the five column
   readers, finalize, plus `changes`/`last_insert_rowid`/`errmsg` — exactly
   what `db.rs` needs, no more.
2. **An in-memory hand-rolled SQL-subset engine.** Rejected outright per the
   task brief and on its own merits: SQL (even a "useful subset") is a large
   surface (parsing, planning, indexes, type affinity, transactions) to
   reimplement and keep correct, for a worse result than option 1 (an
   in-memory engine still wouldn't give real file-backed persistence, which
   `tswift.db.open`'s `path` argument promises).

Confirmed linking works on this workstation (macOS, Xcode SDK ships
`libsqlite3.tbd`) and the full FFI round-trip (open/create table/insert every
`DbValue` tag/query/transactions/structured errors) is covered by
`sqlite_ffi.rs`'s own unit tests plus `db.rs`'s wire-level tests. Crucially,
**no crate outside `tswift-cli` links or calls into SQLite** —
`tswift-core`, `tswift-swiftdata`, `tswift-wasm`, and `tswift-ffi` (iOS) stay
pure Rust and never pull in a native dependency; a wasm or iOS backing for
`tswift.db.*` is a future slice's problem, entirely decoupled from this one
by the wire contract above.

## Decision: errors are a structured `code: message` string in `$thrown`

Every `tswift.db.*` function `throws`; a SQL/handle failure crosses as a
`{"$thrown": "<message>"}` payload (`host_bridge`'s existing sentinel), which
`Interpreter::call_host_fn` turns into a catchable
`HostError { message: String }` — the same shape `tswift.fs.*` already uses
(see `fs.rs`'s `Self::thrown`). The native CLI backing formats `message` as
`"SQLite error <extended-code>: <sqlite3_errmsg text>"`
(`db.rs`'s `thrown_sqlite`), so the structured code is present in the
message text a `catch` block can inspect, without inventing a second
`HostError`-shaped struct with a numeric `code` field only this one service
would use — consistent with `tswift.fs.*`'s own code-in-message convention
(e.g. its `couldn't remove "…": <io error>` messages), not a new pattern.

## Decision: handle lifecycle

Numeric (`i64`) handles minted by an ascending, process-wide atomic counter,
stored in a `Mutex<HashMap<i64, Connection>>` owned by `db::DbHandler`.
`close` removes the entry (dropping and thereby closing the `Connection`).
Any operation against a handle that was never opened, or was already closed,
is a structured `$thrown` error (`"handle N is not open"`), never a panic or
a silent no-op — covered by `db.rs`'s `invalid_handle_is_thrown_not_panicked`
and `double_close_is_thrown` tests. There is no host-triggered close-on-context-teardown
in this slice (no `Drop`/lifecycle hook fires when an `Interpreter` is
dropped) — `DbHandler`'s own `Drop` (via `HashMap<_, Connection>`'s field
drop) closes every still-open connection when the handler itself is dropped
at process exit, which is the CLI's actual teardown moment (`tswift run` is
one-shot per process); an explicit host-triggered per-context teardown hook
is deferred to whichever future slice needs a *long-lived* interpreter (a
REPL, an embedding that runs multiple programs per process) where leaking
open handles across runs would actually matter.

## Consequences

- `crates/tswift-swiftdata` exists as a new workspace member: the shared,
  host-agnostic substrate (op names, signatures, `DbValue` codec) a future
  SwiftData Swift-facing layer builds on, and any future platform backing
  (wasm, iOS) implements against — without duplicating the codec per
  platform the way it would if each platform's backing hand-rolled its own
  tagging scheme.
- `tswift-cli` gains its first native (non-Rust) linked dependency: system
  `libsqlite3`. This is intentionally scoped to `tswift-cli` only — the
  `docs/agents/environment.md` constraint ("keep it out of
  `tswift-core`/`tswift-ffi`/`tswift-wasm`") is satisfied by construction:
  the FFI module and the `build.rs` link directive both live under
  `crates/tswift-cli/`.
- wasm and iOS backings for `tswift.db.*` are out of scope for this slice
  (mirroring how ADR-0014 shipped `tswift.defaults.*`/`tswift.fs.*` for the
  CLI in an earlier slice before wiring wasm/iOS in ADR-0014 itself) — a
  natural follow-up once a SwiftData Swift-facing layer exists to actually
  exercise them end-to-end.
