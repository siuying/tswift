# Plan: native embedding host (`tswift-ffi` → TSwiftCore / TSwiftUI)

Resumes the design tree deferred while the fragment leak was fixed (ADR-0007,
landed). See CONTEXT.md for the glossary (`tswift-ffi`, `Context`, `Render
session`, `Serialized boundary`, `String free contract`).

## Locked decisions

1. **One Rust crate `tswift-ffi`** — `extern "C"` entry points + cbindgen header,
   compiled as a `staticlib`, packaged as one `.xcframework`, fronted by two thin
   Swift façades (`TSwiftCore`, `TSwiftUI`). `TSwiftUI` reuses `ios/UiirRenderer`.

2. **`Context` = owned opaque handle** (`*mut`), not the wasm `thread_local`
   singleton. Owns `Analysis + Interpreter (+ render session)` as one reclaimable
   bundle, freed on Swift `deinit`. The QuickJS `JSContext` model.

3. **Serialized boundary.** Values crossing the C ABI are JSON strings (stdout,
   UIIR, patch stream). No live object references; no QuickJS-style object-graph
   API (deferred — no consumer; SwiftUI incrementality comes from the UIIR
   diff/patch stream, ADR-0006).

4. **String free contract.** Every returned `char*` is Rust-owned heap handed
   over as `*mut c_char`; the caller releases it with the single
   `tswift_string_free`. The Swift façade hides this in `defer`.

## C ABI surface (locked)

```c
// lifespan
TSwiftContext* tswift_context_new(void);
void           tswift_context_free(TSwiftContext*);

// one-shot run (TSwiftCore) — returns owned JSON; shares the Context so the
// fragment cache + installed stdlib persist across reuse.
char*          tswift_run(TSwiftContext*, const char* source);

// SwiftUI render session (TSwiftUI) — returns owned JSON (UIIR, then patches)
char*          tswift_swiftui_compile(TSwiftContext*, const char* source);
char*          tswift_swiftui_dispatch(TSwiftContext*, const char* event_json);

// universal string release
void           tswift_string_free(char*);
```

Mirrors `tswift-wasm`'s `run_swift` / `swiftui_compile` / `swiftui_dispatch`.

5. **Hand-written header, not cbindgen.** The surface is tiny (6 fns, one opaque
   type); a ~15-line `.h` is diff-reviewable with zero offline/vendoring cost
   (cbindgen is not in `Cargo.lock` and can't be `cargo install`ed offline). A
   drift check (a compile-time symbol check in the example app) keeps the `.h`
   and the Rust `extern "C"` signatures in sync. Escalate to a checked-in
   *generated* header only if the surface grows.

## Packaging (locked)

- **Slices**: iOS device (`aarch64-apple-ios`), iOS sim (fat: `aarch64` +
  `x86_64-apple-ios-sim`), macOS (fat: `aarch64` + `x86_64-apple-darwin`). Mac
  matters — example apps and `UiirRenderer` snapshot tests are macOS-hosted.
- **Header**: `crates/tswift-ffi/include/tswift_ffi.h` (single source of truth),
  copied into each xcframework slice's `Headers/` by the build script.
- **One SwiftPM package `ios/TSwift/`** exposing two products `TSwiftCore` and
  `TSwiftUI` over one private `TSwiftFFI` binary target. `TSwiftUI` depends on
  the existing `UiirRenderer`.
- **Local-override-else-pinned-remote `binaryTarget`.** `Package.swift` picks a
  git-ignored local `ios/TSwift/Artifacts/TSwiftFFI.xcframework` if present,
  else downloads the pinned released zip via `url` + `checksum` read from a
  committed `ios/TSwift/ffi.pin` (JSON: `{version, url, checksum}`).
- **Host**: a **GitHub Release asset** (not GitHub Packages — that registry does
  not host raw binaries). Pinned by a dedicated **`ffi-vN`** tag, bumped by the
  publish script, giving the binary ABI its own version line. See ADR-0008.
- **Two scripts**: `scripts/build-xcframework.sh` (cargo ×targets → `lipo` →
  `xcodebuild -create-xcframework` → local `Artifacts/`) and
  `scripts/publish-xcframework.sh` (zip → `swift package compute-checksum` →
  `gh release create/upload` → rewrite `ffi.pin`), the latter run on demand.
- Network note: the offline rule is **crates.io-only**; SwiftPM already fetches
  `swift-snapshot-testing`, so a remote `binaryTarget` is consistent.

## TSwiftUI session driver (locked)

The consume side already exists in `UiirRenderer` (`RenderModel.apply([Patch])`,
`Patch: Decodable`). Two pieces are added:

- **`PreviewSession`** — `@MainActor public final class ... : ObservableObject`
  in `TSwiftUI`. Owns a `Context` (`tswift_context_new` on init,
  `tswift_context_free` on deinit) and a `RenderModel`. `compile(source:)` →
  `tswift_swiftui_compile` → mount initial UIIR; `dispatch(id:event:value:)` →
  `tswift_swiftui_dispatch` → decode `[Patch]` → `renderModel.apply(...)`.
- **Event-out seam in `UiirRenderer`** — an `EventSink` injected via
  `@Environment`, **defaulting to a no-op** so existing snapshot tests stay
  byte-identical. Live mode injects a sink that calls `PreviewSession.dispatch`.
  Keeps **one** `ViewFactory` for both static snapshots and live previews
  (rejected: a parallel interactive renderer — duplicates the `build` switch).
- The dispatch event carries wasm's `(id, event, value)` triple (node id, event
  name e.g. `"tap"`, payload `""` for taps) as JSON, built by the façade.

## Example app + verification (locked)

- **One combined example app** (`examples/ios/`), two screens: a **Run** screen
  (TSwiftCore — editor + stdout/diagnostics) and a **Preview** screen (TSwiftUI
  — editor + live `PreviewSession`). One app proves both products link the one
  xcframework and smoke-tests the `binaryTarget` switch; halves project/signing
  boilerplate. (Rejected: two separate apps.)
- **The app is a demo, never the gate.** Regression signal lives in tests:
  - A `TSwiftUI` snapshot test drives `PreviewSession.compile` then `dispatch`
    and snapshots the resulting `RenderModel` tree (reuses the existing
    `swift-snapshot-testing` infra).
  - A Rust `#[test]` calls the `extern "C"` functions directly, asserting the
    returned JSON and that `tswift_string_free` is paired (no leak).

## Status

All design branches resolved. Distribution model captured in ADR-0008. Ready to
implement — see the task breakdown below.

---

# Implementation

Dependency-ordered vertical slices. Each task is independently committable and
leaves `scripts/presubmit` green (Rust tasks) or the Swift package building
(Swift tasks). The first Rust slices stand alone; Swift packaging waits on a
built xcframework.

## Overall checklist

- [ ] **T1** — Scaffold `tswift-ffi` crate: `Context` handle + `tswift_string_free`
- [ ] **T2** — `tswift_run` one-shot run entry point (TSwiftCore surface)
- [ ] **T3** — `tswift_swiftui_compile` / `tswift_swiftui_dispatch` (render session)
- [ ] **T4** — Hand-written `tswift_ffi.h` + ABI drift check
- [ ] **T5** — `scripts/build-xcframework.sh` (all slices → local `Artifacts/`)
- [ ] **T6** — `ios/TSwift` package skeleton + `TSwiftCore` façade over T2
- [ ] **T7** — `UiirRenderer` `EventSink` seam (no-op default, snapshot-safe)
- [ ] **T8** — `TSwiftUI` `PreviewSession` driver over T3 + T7
- [ ] **T9** — `scripts/publish-xcframework.sh` + `ffi.pin` pinned-release path
- [ ] **T10** — Combined example app (Run + Preview screens)
- [ ] **T11** — Verification: Rust FFI `#[test]` + `TSwiftUI` snapshot test

---

## T1 — Scaffold `tswift-ffi` crate (Context handle + string free)

**Goal.** A new `crates/tswift-ffi` `staticlib` exposing the lifespan-owning
`Context` handle and the universal string release, with the one sanctioned
`unsafe` seam confined here (ADR-0001). No run/render logic yet.

**Interfaces.**
```rust
// opaque to C; owns the reclaimable bundle (Interpreter + fragment cache, etc.)
pub struct Context { /* private */ }

#[no_mangle] pub extern "C" fn tswift_context_new() -> *mut Context;
#[no_mangle] pub unsafe extern "C" fn tswift_context_free(ctx: *mut Context);
#[no_mangle] pub unsafe extern "C" fn tswift_string_free(s: *mut c_char);
```
- `Cargo.toml`: `crate-type = ["staticlib", "rlib"]` (rlib so Rust `#[test]` can
  link it); depends on `tswift-core`/`-std`/`-foundation`/`-frontend`.
- A private `into_json_ptr(String) -> *mut c_char` / and null-safe free helper.

**Validation.**
- `cargo build -p tswift-ffi` produces a `.a`; `scripts/presubmit` green.
- Rust `#[test]`: `new` → non-null; `free` of that ptr; `free(null)` is a no-op;
  round-trip a `CString` through `into_json_ptr` + `tswift_string_free` under
  Miri-style discipline (manual: no double-free, asserted by a leak-counter
  test or just exercised paths).

## T2 — `tswift_run` one-shot run (TSwiftCore surface)

**Goal.** Port `tswift-wasm`'s `run_swift_impl` onto the C ABI: compile + run a
source string through a `Context`, return owned result JSON. Reuses the
Context's persistent interpreter/fragment cache across calls.

**Interfaces.**
```rust
#[no_mangle] pub unsafe extern "C" fn tswift_run(
    ctx: *mut Context, source: *const c_char,
) -> *mut c_char; // owned JSON; free with tswift_string_free
```
- JSON shape mirrors `run_swift` (`{ok, backend:"ffi", compile:{...}, run:{...}}`).
- Use `crates/tswift-core/src/json.rs` for escaping (no serde_json; offline).

**Validation.**
- Rust `#[test]`: run `print("hi")` → JSON with `run.stdout == "hi\n"`, `ok:true`;
  a compile error → `compile.ok:false` with the diagnostic; pointer freed.
- Reuse one `Context` across two runs → second succeeds (no stale state).

## T3 — SwiftUI render session FFI

**Goal.** Expose the stateful render session: compile to initial UIIR, then
route `(id, event, value)` events to a patch stream. Mirrors
`tswift-wasm/src/swiftui.rs` but session state lives in the owned `Context`,
not a `thread_local`.

**Interfaces.**
```rust
#[no_mangle] pub unsafe extern "C" fn tswift_swiftui_compile(
    ctx: *mut Context, source: *const c_char,
) -> *mut c_char; // owned JSON: initial UIIR
#[no_mangle] pub unsafe extern "C" fn tswift_swiftui_dispatch(
    ctx: *mut Context, event_json: *const c_char,
) -> *mut c_char; // owned JSON: patch stream
```
- `event_json` carries `{ "id", "event", "value" }`; decoded with core `json.rs`.
- `dispatch` before `compile` → a structured error JSON (mirror `dispatch_error`).

**Validation.**
- Rust `#[test]`: compile a counter view → UIIR contains the label; dispatch a
  `tap` on the button id → patch stream mutates the label (mirror
  `dispatch_taps_a_button_and_returns_a_patch_stream`); dispatch-without-compile
  → error JSON. Pointers freed.

## T4 — Hand-written header + drift check

**Goal.** A reviewable `crates/tswift-ffi/include/tswift_ffi.h` declaring the
opaque `TSwiftContext` and the 6 functions, plus a check that the `.h` and the
Rust `extern "C"` signatures cannot silently diverge.

**Interfaces.** `tswift_ffi.h`: `typedef struct TSwiftContext TSwiftContext;` +
the six prototypes from the locked C ABI surface.

**Validation.**
- Drift check: a Rust `#[test]` (or `build.rs` assertion) that references each
  `extern "C"` symbol so a rename breaks the build; plus a tiny C compile in
  `scripts/presubmit` (or the example app's link step in T10) that `#include`s
  the header and references all six symbols against the `.a`. Document which
  guard is authoritative.

## T5 — `scripts/build-xcframework.sh`

**Goal.** Produce the local `ios/TSwift/Artifacts/TSwiftFFI.xcframework` from the
three slice sets, header included.

**Interfaces.** `cargo build --release` for `aarch64-apple-ios`,
`{aarch64,x86_64}-apple-ios-sim`, `{aarch64,x86_64}-apple-darwin` → `lipo` the
fat sim/mac slices → `xcodebuild -create-xcframework` with `-headers
crates/tswift-ffi/include` → output under `ios/TSwift/Artifacts/` (git-ignored).

**Validation.**
- Script exits 0 and emits a `.xcframework` with device/sim/mac slices
  (`xcodebuild -create-xcframework` succeeds; `Info.plist` lists 3 libraries).
- `.gitignore` covers `ios/TSwift/Artifacts/`.

## T6 — `ios/TSwift` package + `TSwiftCore` façade

**Goal.** The SwiftPM package with the local-or-pinned `binaryTarget` switch and
the `TSwiftCore` product: a thin Swift API over `tswift_run` that hides pointer
lifetimes and the string free.

**Interfaces.**
```swift
public final class TSwiftContext { init(); deinit /* tswift_context_free */ }
public enum TSwiftCore {
  public struct RunResult { let ok: Bool; let stdout: String; let diagnostics: String }
  public static func run(_ source: String, in ctx: TSwiftContext = .init()) -> RunResult
}
```
- `Package.swift`: file-exists switch → `binaryTarget(path:)` else
  `binaryTarget(url:checksum:)` from `ffi.pin` (a placeholder pin until T9).
- `defer { tswift_string_free(ptr) }` around every returned pointer.

**Validation.**
- With a local xcframework present (T5), `swift build` succeeds and a
  `TSwiftCoreTests` test runs `print("hi")` → `RunResult.stdout == "hi\n"`.

## T7 — `UiirRenderer` EventSink seam

**Goal.** Add an event-out path to the existing renderer without disturbing
static snapshots: interactive nodes emit `(id, event, value)` to an injected
sink that defaults to a no-op.

**Interfaces.**
```swift
public struct UiirEvent { let id: String; let event: String; let value: String }
public protocol EventSink { func send(_ e: UiirEvent) }
// @Environment(\.uiirEventSink) default = no-op; ViewFactory routes Button/control
// actions through it instead of the current no-ops.
```

**Validation.**
- Existing `UiirRendererTests` snapshot suite stays **byte-identical** (default
  no-op sink) — the key regression gate.
- A new unit test installs a recording sink, renders a Button, simulates the
  action, asserts the recorded `UiirEvent`.

## T8 — `TSwiftUI` `PreviewSession` driver

**Goal.** The live preview driver tying T3 (FFI) + T7 (event seam) + the
existing `RenderModel` patch applier into one `ObservableObject`.

**Interfaces.**
```swift
@MainActor public final class PreviewSession: ObservableObject {
  @Published public private(set) var model: RenderModel
  public init()                  // owns a TSwiftContext
  public func compile(_ source: String)               // -> mount initial UIIR
  public func dispatch(id: String, event: String, value: String) // -> apply patches
}
// Provides an EventSink that calls dispatch; injected into the rendered tree.
```

**Validation.**
- `TSwiftUI` test: `compile` a counter view → `model.root` has the label;
  `dispatch` a tap → the label updates (drives FFI + patch apply end to end).

## T9 — `scripts/publish-xcframework.sh` + `ffi.pin`

**Goal.** The on-demand release path: zip the xcframework, checksum it, upload a
`ffi-vN` GitHub Release asset, rewrite `ffi.pin` so a clone without a local
build resolves the pinned remote (ADR-0008).

**Interfaces.** `scripts/publish-xcframework.sh` → `zip` → `swift package
compute-checksum` → `gh release create ffi-vN` + `gh release upload` → write
`ios/TSwift/ffi.pin` (`{version, url, checksum}`).

**Validation.**
- Dry-run produces a deterministic checksum and a well-formed `ffi.pin`.
- Removing the local `Artifacts/` and `swift build` resolves the pinned remote
  (manual/CI check, network).

## T10 — Combined example app

**Goal.** One Xcode app proving both products link the single xcframework: a Run
screen (TSwiftCore) and a Preview screen (TSwiftUI).

**Interfaces.** `examples/ios/` SwiftUI app, two screens; depends on the
`ios/TSwift` package products.

**Validation.**
- App builds against the local xcframework; Run screen shows stdout for a
  sample; Preview screen renders + responds to a tap. (Demo, not a CI gate.)

## T11 — Verification sweep

**Goal.** Lock the regression signals so future changes can't silently break the
boundary or the live path.

**Interfaces.** Consolidate: Rust FFI `#[test]`s (T1–T3) asserting JSON + paired
`tswift_string_free`; the `TSwiftUI` snapshot test driving `PreviewSession`.

**Validation.**
- `scripts/presubmit` green (Rust side); `swift test` green for `ios/TSwift`;
  `UiirRenderer` snapshots unchanged.
