# tswift

A lightweight, end-to-end Swift compiler and runtime in Rust: a pure-Rust
frontend (lex → parse → sema) feeding a tree-walking interpreter, with stdlib,
Foundation, and SwiftUI implemented as native Rust builtins. Hosts (web/wasm,
native/iOS) drive the runtime over a serialized boundary.

> **Current focus (pivot):** before embedding the runtime as a long-running
> native host (`TSwiftCore`/`TSwiftUI`), resolve the **fragment leak** — the
> per-interpolation `Analysis` leak that ADR-0003 deferred — so a live session
> doesn't grow memory on every render. The Swift-library work resumes once the
> leak is bounded and reclaimed.

## Language

### Hosts & bindings

**tswift-ffi**:
The single Rust crate exposing the runtime to native callers as a C ABI
(`extern "C"` entry points + cbindgen-generated header), compiled as a
`staticlib` and packaged as one `.xcframework`. The native analogue of
`tswift-wasm`; serves both the one-shot runner and the stateful SwiftUI session.
_Avoid_: tswift-c, tswift-native, tswift-swift.

**TSwiftCore**:
The Swift package façade over `tswift-ffi`'s run surface: compile Swift to AST,
include the standard library, and run a program. Thin wrapper, not a second
binary.

**TSwiftUI**:
The Swift package façade over `tswift-ffi`'s SwiftUI session surface: manage a
live render session and preview SwiftUI code. Reuses `UiirRenderer` for the
actual view rendering.

**UiirRenderer**:
The existing iOS package that turns UIIR + a patch stream into real SwiftUI
views (a thin patch-applier). The render half that `TSwiftUI` builds on; not a
session driver.

**UIIR**:
The host-neutral, semantic SwiftUI intermediate representation (SwiftUI
*concepts* like `VStack`/`Text`/`.font`, never lowered to host primitives)
emitted by the runtime and consumed by hosts. See ADR-0006.

**Fragment leak**:
The per-interpolation `Analysis` leak: evaluating `"\(expr)"` re-analyzes `expr`
into a fresh `Analysis` and `Box::leak`s it (`eval_interpolation`,
interp.rs:7233) so its nodes are `Node<'static>` (ADR-0003). Bounded and
harmless for the run-once CLI; an unbounded leak for a long-running host that
re-renders. The thing the pivot fixes.

**Fragment cache**:
The fix (ADR-0007): an interpreter-owned, append-only, source-keyed store of
interpolation-fragment analyses (`interp/fragment_cache.rs`). A repeated fragment
is analyzed once; `Drop` reclaims the whole cache with the session — so memory is
bounded within *and* across sessions. Never evicts (a live `Node<'static>` points
into it). The one sanctioned `unsafe` seam in the core.

**Session handle**:
An opaque pointer (`*mut`) the C ABI hands out for a live SwiftUI render
session, created from source and freed explicitly. Unlike the wasm
`thread_local` singleton, it owns its `Analysis` + `Interpreter` + `Session` as
one reclaimable self-referential bundle (no `Box::leak`), freed on the Swift
wrapper's `deinit`. The `unsafe` for the bundle is confined to `tswift-ffi`,
preserving ADR-0001's FFI-only-unsafe rule.
