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

/* ---- Host HTTP transport (URLSession support) — one-shot path ---------- */

/* The handler behind script `URLSession` requests (one-shot, synchronous).
 * Receives the request as a JSON string
 *   {"url","method","timeoutSeconds","headers":[[k,v]...],"bodyBase64"?}
 * plus an opaque in-flight `call` token. The handler MUST call
 * tswift_http_respond(call, response_json) exactly once BEFORE returning (it
 * may block internally, e.g. a semaphore around a real URLSession task).
 * Response JSON: {"status","headers":[[k,v]...],"bodyBase64"?} on success, or
 * {"error":"<URLError.Code case>","message"?} on transport failure. */
typedef void (*tswift_http_handler)(void *userdata,
                                    const char *request_json,
                                    void *call);

/* Register `handler` as the HTTP transport for scripts run through `ctx`;
 * `userdata` is passed through verbatim. NULL removes the handler (scripts
 * then see URLSession as unavailable). The handler must stay callable, and
 * `userdata` valid, until removed or the context is freed. */
void tswift_set_http_handler(TSwiftContext *ctx,
                             tswift_http_handler handler,
                             void *userdata);

/* Deliver the response for an in-flight handler call. Copies `response_json`
 * immediately; valid only during the handler invocation that received `call`. */
void tswift_http_respond(void *call, const char *response_json);

/* ---- Host HTTP transport (URLSession support) — streaming path (M6) ----- */

/* Called by Rust once per in-flight request. Must initiate the request and
 * return quickly (fire-and-forget). `request_json` is a NUL-terminated JSON
 * string (valid only during the call). `task_token` is an opaque token that
 * the host must pass to tswift_http_event for each event, keeping it alive
 * until the terminal event (done/error) has been consumed by the interpreter
 * (i.e. until tswift_http_cancel_fn has been called, or the script has
 * finished reading the response). */
typedef void (*tswift_http_start_fn)(void *userdata,
                                     const char *request_json,
                                     void *task_token);

/* Called by Rust to abort an in-flight request. After this call the host
 * should stop delivering events. Late calls to tswift_http_event before the
 * cancellation is processed are safe no-ops. */
typedef void (*tswift_http_cancel_fn)(void *userdata, void *task_token);

/* Register a streaming HTTP transport for scripts run through `ctx`.
 * Takes priority over the one-shot handler set by tswift_set_http_handler.
 * Pass a NULL start_fn to remove the streaming handler.
 * `start_fn` and `cancel_fn` must stay callable, and `userdata` valid, until
 * the handler is replaced/removed or the context is freed. */
void tswift_set_http_stream_handler(TSwiftContext *ctx,
                                    tswift_http_start_fn start_fn,
                                    tswift_http_cancel_fn cancel_fn,
                                    void *userdata);

/* Push one event for an in-flight streaming request. Callable from ANY thread.
 * `task_token` is the pointer passed to tswift_http_start_fn; it must not be
 * used after the terminal event has been consumed by the interpreter.
 * `event_json` is a NUL-terminated JSON string in one of these forms:
 *   {"event":"response","status":200,"headers":[["Content-Type","text/plain"]]}
 *   {"event":"chunk","bodyBase64":"aGk="}
 *   {"event":"done"}
 *   {"event":"error","code":"timedOut","message":"..."}
 * NULL or malformed event_json is treated as {"event":"error","code":"badServerResponse"}.
 * A push after the terminal event has already been pushed is a safe no-op. */
void tswift_http_event(void *task_token, const char *event_json);

/* ---- TSwiftUI: stateful render session --------------------------------- */

/* Compile a SwiftUI program and start a live render session, returning owned
 * UIIR-JSON. Replaces any prior session on `ctx`. */
char *tswift_swiftui_compile(TSwiftContext *ctx, const char *source);

/* Route an event (`{"id":..,"event":..,"value"?:..}`) into the live session,
 * returning an owned patch-stream JSON. */
char *tswift_swiftui_dispatch(TSwiftContext *ctx, const char *event_json);

/* Lint `source` and return owned diagnostics JSON
 * (`{"ok":bool,"diagnostics":[{"line","col","severity","message"}]}`) without
 * rendering. Stateless (no context). The editor's live error-feedback channel. */
char *tswift_diagnostics(const char *source);

/* ---- String release ---------------------------------------------------- */

/* Release a string returned by any function above. NULL is ignored. */
void tswift_string_free(char *s);

#ifdef __cplusplus
}
#endif

#endif /* TSWIFT_FFI_H */
