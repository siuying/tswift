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
  publish script, giving the binary ABI its own version line.
- **Two scripts**: `scripts/build-xcframework.sh` (cargo ×targets → `lipo` →
  `xcodebuild -create-xcframework` → local `Artifacts/`) and
  `scripts/publish-xcframework.sh` (zip → `swift package compute-checksum` →
  `gh release create/upload` → rewrite `ffi.pin`), the latter run on demand.
- Network note: the offline rule is **crates.io-only**; SwiftPM already fetches
  `swift-snapshot-testing`, so a remote `binaryTarget` is consistent.

## Open branches (still to grill)

- **`TSwiftUI` session driver**: how it drives a live render session against
  `UiirRenderer`'s patch applier (which already consumes UIIR + patch JSON).
- **Two example apps**: CodeSandbox-style split view (TSwiftCore);
  Playground-style preview (TSwiftUI).
