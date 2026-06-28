# tswift

A lightweight, end-to-end Swift compiler and runtime in Rust: a pure-Rust
frontend (lex → parse → sema) feeding a tree-walking interpreter, with stdlib,
Foundation, and SwiftUI implemented as native Rust builtins. Hosts (web/wasm,
native/iOS) drive the runtime over a serialized boundary.

> **Current focus:** the **fragment leak** is resolved (ADR-0007, landed) — the
> per-interpolation `Analysis` leak is now bounded and reclaimed by the
> **fragment cache**. Next: design and build the native embedding host
> (`tswift-ffi` → `TSwiftCore`/`TSwiftUI`).

## Language

### Hosts & bindings

**tswift-ffi**:
The single Rust crate exposing the runtime to native callers as a C ABI
(`extern "C"` entry points + cbindgen-generated header), compiled as a
`staticlib` and packaged as one `.xcframework`. The native analogue of
`tswift-wasm`; serves both the one-shot runner and the stateful SwiftUI session.
_Avoid_: tswift-c, tswift-native, tswift-swift.

**Serialized boundary**:
The FFI contract that values crossing the C ABI are **serialized JSON strings**
(stdout, UIIR, patch stream) — never live object references. The VM owns its
object graph; the host receives copies. Mirrors `tswift-wasm`. A live-reference
(QuickJS/JSC-style) object API is deliberately *not* exposed — no consumer needs
it, and SwiftUI incrementality already comes from the UIIR diff/patch stream
(ADR-0006). It can be added later behind the same handle without breaking this
contract.

**String free contract**:
Every JSON string an entry point returns is a Rust-owned heap allocation handed
over as `*mut c_char`; the caller must release it with the single
`tswift_string_free`. The Swift façade hides this behind `defer`, so callers
never touch a raw pointer. (The VM owning the session does not eliminate this:
even QuickJS pairs `JS_ToCString` with `JS_FreeCString` — extracting a string to
C-land always incurs a release.)

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

**Context**:
The opaque pointer (`*mut`) the C ABI hands out as the lifespan-owning VM
handle — the native analogue of QuickJS's `JSContext` / WebKit's
`JSGlobalContextRef`. Created explicitly and freed on the Swift wrapper's
`deinit`. Unlike the wasm `thread_local` singleton, it owns its `Analysis` +
`Interpreter` (+ any render session) as one reclaimable self-referential bundle
(no `Box::leak`). Serves both the one-shot runner and the SwiftUI preview; a
host may reuse one Context across runs so the fragment cache and installed
stdlib persist. The `unsafe` for the bundle is confined to `tswift-ffi`,
preserving ADR-0001's FFI-only-unsafe rule.

**Render session**:
The narrower SwiftUI render-loop state (diff/patch driver) that lives *inside* a
Context; "session" always means this, never the VM handle itself.
