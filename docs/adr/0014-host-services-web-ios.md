# ADR-0014: Host-services web/iOS backing — wire contract and platform tiers

- **Status:** Accepted
- **Date:** 2026-07-11
- **Context slice:** `tswift.defaults.*` / `tswift.fs.*` host services (Foundation's
  `UserDefaults` / `FileManager`)
- **Builds on:** the host-native function bridge (`crates/tswift-core/src/host_bridge.rs`),
  `HostService`/`Capabilities` install-time gating (`crates/tswift-core/src/host_services.rs`),
  ADR-0005 (cooperative single-threaded executor)
- **Related:** `crates/tswift-foundation/src/user_defaults.rs`,
  `crates/tswift-foundation/src/file_manager.rs`, `crates/tswift-cli/src/defaults.rs`,
  `crates/tswift-cli/src/fs.rs`

## Context

Slices 1–3 landed `UserDefaults`/`FileManager` in `tswift-foundation`, layered
on two host services (`HostService::Defaults`/`HostService::FileSystem`) and
their `tswift.defaults.*`/`tswift.fs.*` wire functions, plus a native (CLI)
backing. This slice wires the same contract into the two embedding hosts that
had none yet: the wasm/JS playground and the iOS native embedding
(`tswift-ffi` → `TSwiftCore`).

Both hosts already had the *general* host-function/host-service plumbing
(`registerHostFunction`/`tswiftHost` on wasm; `tswift_register_host_fn`/
`tswift_declare_host_service` on iOS) from the host-bridge epic. What was
missing was (a) an actual implementation of the nine `tswift.fs.*` + three
`tswift.defaults.*` functions on each host, and (b) on wasm specifically, a
**pre-existing ordering bug** that silently broke every host function
resolved through the *default* handler (see "wasm ordering bug" below) —
`registerHostFunction`-based custom functions worked because that call site
always supplies an explicit handler, but `UserDefaults`/`FileManager`, whose
`tswift-foundation` registration passes `None` (see
"Foundation-side contract"), did not.

## The wire contract (unchanged by this slice — restated here for reference)

Three `tswift.defaults.*` functions, three-plus-nine `tswift.fs.*` functions.
Every embedding's backing MUST implement exactly this shape; it is the single
source of truth `crates/tswift-foundation` codes against, on every platform.

### `tswift.defaults.*`

| function | params | returns | throws |
|---|---|---|---|
| `tswift.defaults.set` | `key: String, value: String` | `Void` | no |
| `tswift.defaults.get` | `key: String` | `String?` | no |
| `tswift.defaults.remove` | `key: String` | `Void` | no |

`value`/the `get` reply's content is the JSON encoding of the stored Swift
value (`true`, `42`, `"hi"`, `["a","b"]`) — i.e. **double-encoded**: the outer
JSON shape required by the declared return type (`String?`) wraps an inner
JSON *text* that is itself the stored value's encoding. A backing that stores
the raw string verbatim (a `HashMap<String,String>`, a real `UserDefaults`
value, a `localStorage` entry) and hands it back unwrapped gets this for free,
because the host-function trampoline (`HostCallHandler::call`'s `Ok(json)` /
the JS shim's `JSON.stringify(string)` / Swift's `JSONSerialization.data(withJSONObject: String)`)
re-encodes a returned host-language string as a JSON string automatically.

### `tswift.fs.*`

| function | params | returns | throws |
|---|---|---|---|
| `tswift.fs.exists` | `path: String` | `Bool` | no |
| `tswift.fs.isDirectory` | `path: String` | `Bool` | no |
| `tswift.fs.read` | `path: String` | `String?` (base64) | no |
| `tswift.fs.list` | `path: String` | `[String]` | yes |
| `tswift.fs.mkdir` | `path: String, withIntermediateDirectories: Bool` | `Void` | yes |
| `tswift.fs.remove` | `path: String` | `Void` | yes |
| `tswift.fs.write` | `path: String, content: String (base64), atomically: Bool` | `Bool` | no |
| `tswift.fs.copy` | `from: String, to: String` | `Void` | yes |
| `tswift.fs.move` | `from: String, to: String` | `Void` | yes |

Binary content crosses the wire as base64 `String` (stage-1 host types have no
`Data`). A throwing op signals failure by returning `{"$thrown":"<message>"}`
from the handler (the interpreter turns this into a catchable
`HostError { message: String }`, per `crates/tswift-core/src/interp.rs`'s
`call_host_fn`) — never a host-side `Err`, which instead traps the whole run.

## Declaration: two independent opt-ins, on purpose

A framework API layered on a host service only calls the wire functions above
when **both**:

1. The embedding has declared the service available — `globalThis.tswiftHostServices`
   on wasm, `tswift_declare_host_service(ctx, "tswift.defaults" | "tswift.fs")`
   on iOS/native. This is a capability flag only (`Capabilities` in
   `crates/tswift-core/src/host_services.rs`); it never implies a handler
   exists.
2. Something actually answers the named `tswift.defaults.*`/`tswift.fs.*`
   calls — a JS `globalThis.tswiftHost` hook on wasm, or a per-function
   `registerHostFunction` callback on iOS/native.

Declaring (1) without (2) degrades every call to the same "not available"
diagnostic as declaring neither — this is deliberate: a host can advertise
future intent, or a script can be developed against a not-yet-wired platform,
without the framework code branching on it.

## Decision: platform tiers, named honestly

| Platform | `tswift.defaults.*` backing | `tswift.fs.*` backing | Tier |
|---|---|---|---|
| **native** (CLI) | in-process `HashMap`, optional `TSWIFT_DEFAULTS_FILE` persistence | real, unrooted `std::fs` | full — real storage |
| **iOS** (`tswift-ffi`/`TSwiftCore`) | real `UserDefaults.standard` (or a caller-supplied suite) | real `FileManager.default`, app sandbox container | full — real storage, same trust boundary as a hand-written Swift app |
| **web** (wasm playground) | `localStorage`, namespaced (`tswift:defaults:<key>`) | virtual flat-namespace fs; `localStorage`-backed when a value fits the quota, else in-memory only for that entry | **degraded** — see below |

The web tier is degraded and is documented as such, not presented as parity:

- The playground runs the wasm interpreter on the page's **main thread** (see
  `FullPlayground.astro`'s `initWasm()` — a plain dynamic `import()`, no
  `Worker`). The interpreter's host-call boundary is synchronous by design
  (mirrors ADR-0005/ADR-0010's constraint for HTTP). OPFS's synchronous access
  handle API (`createSyncAccessHandle`), the natural "real" browser
  filesystem primitive, is **worker-only** — calling it on the main thread
  throws. Since nothing here runs in a worker, that tier is unavailable.
  **Tripwire:** if the interpreter ever moves off the main thread (a worker,
  or a future async host-call seam), OPFS becomes reachable and should
  replace the `localStorage` virtual fs for `tswift.fs.*`; `tswift.defaults.*`
  can stay on `localStorage` regardless (already synchronous and adequate for
  a small key-value store).
- `localStorage` is small (~5MB/origin in most browsers) and UTF-16 string
  only. A write that doesn't fit falls back to **memory-only** storage for
  that one entry (still correct within the page's lifetime; does not survive
  reload) rather than failing the call or silently truncating data — a script
  that only cares about "does the value round-trip within this page session"
  sees no difference; a script that cares about reload persistence needs to
  know the quota exists, hence this table.

Implementation: `website/src/lib/tswift-host-services.js` (`installTSwiftHostServices()`),
wired into `FullPlayground.astro`/`MiniPlayground.astro` before the wasm
module's first run. Round-trip coverage: `website/test/wasm-smoke.mjs`
(checks 6+, "tswift.defaults/tswift.fs host services").

iOS implementation: `ios/TSwift/Sources/TSwiftCore/TSwiftFoundationHostServices.swift`
(`TSwiftContext.installFoundationHostServices(defaults:fileManager:)`), plus
`TSwiftContext.declareHostService(_:)` (new — the Swift wrapper over
`tswift_declare_host_service` was previously unexposed). Coverage:
`ios/TSwift/Tests/TSwiftCoreTests/TSwiftFoundationHostServicesTests.swift`.

## The wasm ordering bug this slice fixed

`crates/tswift-core/src/host_bridge.rs`'s `HostBridge::register` resolves a
`None` handler against the installed **default handler at registration time**
(`handler.or_else(|| self.default_handler.clone())`), not lazily per call. If
no default handler is installed yet, registration with `None` fails outright
(`Err`, silently discarded by `tswift-foundation`'s `let _ = interp.register_host_fn(...)`),
and `ctx.is_host_fn("tswift.defaults.set")` reports `false` forever after —
every `UserDefaults`/`FileManager` call then raises the "unavailable on this
platform" diagnostic, indistinguishable from a page that never declared the
service at all, **even when it did**.

`crates/tswift-wasm/src/lib.rs`'s `run_swift_impl` called
`tswift_foundation::install_with(&mut interp, …)` (which performs those `None`
registrations) *before* `platform::install_host_handler` (which installed the
default handler) — the wrong order. Fixed by splitting that function into
`install_host_call_handler` (sets the default handler, unconditional, now
called first) and `install_registered_host_fns` (registers
`registerHostFunction`-declared custom schemas, called after `install_with`,
unchanged behaviour for that path). `crates/tswift-ffi` never had this bug:
its per-function `registerHostFunction`/`tswift_register_host_fn` path always
supplies an explicit handler (`Some(handler)`), bypassing default-handler
resolution entirely, and `tswift-ffi` has no default-handler concept at all —
by design, every iOS host function (including `tswift.defaults.*`/
`tswift.fs.*`) is registered individually.

## Consequences

- Adding a third host service (`tswift.db.*`, per `HostService::ALL`) to wasm
  only needs its wire functions dispatched inside `tswiftHost` (or a sibling
  JS module composed the same way `tswift-host-services.js` composes with an
  existing hook) — the ordering fix removed a footgun that would otherwise
  have silently broken it too.
- iOS/native gain no equivalent footgun: every host function there is always
  explicitly registered, so there is no "forgot to set a default handler"
  failure mode to repeat.
- The web tier's documented limits (quota, no OPFS on the main thread) are a
  known, bounded gap — not a silent one. A future worker-hosted interpreter is
  the tracked way to close it for `tswift.fs.*`.
