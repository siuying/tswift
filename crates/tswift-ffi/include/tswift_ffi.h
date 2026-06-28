/*
 * tswift-ffi — C ABI for the native embedding host (TSwiftCore / TSwiftUI).
 *
 * Hand-written and kept in lockstep with the Rust `extern "C"` definitions in
 * `src/lib.rs` (see `docs/plan/native-host.md`, decision 5). The Rust test
 * `c_abi_signatures_match_header` guards the Rust side; the example app's link
 * step (T10) is the authoritative cross-language check.
 *
 * Boundary contract: every `char *` returned here is a Rust-owned, NUL-
 * terminated, UTF-8 JSON string. The caller MUST release each one exactly once
 * with `tswift_string_free`. NULL pointers are accepted and ignored by the
 * `*_free` functions.
 */
#ifndef TSWIFT_FFI_H
#define TSWIFT_FFI_H

#ifdef __cplusplus
extern "C" {
#endif

/* Opaque lifespan-owning VM handle (the QuickJS `JSContext` analogue). */
typedef struct TSwiftContext TSwiftContext;

/* ---- Lifespan ---------------------------------------------------------- */

/* Allocate a new context. Never returns NULL. Free with tswift_context_free. */
TSwiftContext *tswift_context_new(void);

/* Free a context from tswift_context_new. NULL is accepted and ignored.
 * Must be called exactly once per context. */
void tswift_context_free(TSwiftContext *ctx);

/* ---- TSwiftCore: one-shot compile + run -------------------------------- */

/* Compile and run `source`, returning an owned result-JSON string. */
char *tswift_run(TSwiftContext *ctx, const char *source);

/* ---- TSwiftUI: stateful render session --------------------------------- */

/* Compile a SwiftUI program and start a live render session, returning owned
 * UIIR-JSON. Replaces any prior session on `ctx`. */
char *tswift_swiftui_compile(TSwiftContext *ctx, const char *source);

/* Route an event (`{"id":..,"event":..,"value"?:..}`) into the live session,
 * returning an owned patch-stream JSON. */
char *tswift_swiftui_dispatch(TSwiftContext *ctx, const char *event_json);

/* ---- String release ---------------------------------------------------- */

/* Release a string returned by any function above. NULL is ignored. */
void tswift_string_free(char *s);

#ifdef __cplusplus
}
#endif

#endif /* TSWIFT_FFI_H */
