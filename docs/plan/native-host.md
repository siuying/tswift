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

## Open branches (still to grill)

- **cbindgen** as a dev/one-shot tool vs build-dependency (not in `Cargo.lock`;
  offline — may need vendoring; confirm with user).
- **xcframework packaging**: device + simulator + arch slices; SwiftPM
  `binaryTarget` vs build script; where the generated header lives.
- **`TSwiftUI` session driver**: how it drives a live render session against
  `UiirRenderer`'s patch applier (which already consumes UIIR + patch JSON).
- **Two example apps**: CodeSandbox-style split view (TSwiftCore);
  Playground-style preview (TSwiftUI).
