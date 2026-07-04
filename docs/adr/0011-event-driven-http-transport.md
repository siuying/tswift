# ADR-0011: Event-driven HTTP transport — start / next_event / cancel seam

- **Status:** Proposed
- **Date:** 2026-07-04
- **Amends:** ADR-0010 (synchronous one-shot `perform` seam)
- **Context slice:** Foundation networking — delegates, cancellation, progress
- **Builds on:** ADR-0005 (cooperative single-threaded executor), ADR-0010
- **Drives:** `docs/plan/urlsession-event-transport.md` milestones M1–M8; `frameworks/foundation/scope.toml` tier F6 delegate/bytes/download members

## Context

ADR-0010 shipped `URLSession.data/upload` over a one-shot synchronous
`HttpTransport::perform`. That seam cannot express three Foundation features:

- **Delegate callbacks** (`urlSession(_:dataTask:didReceive:)`,
  `urlSession(_:task:didCompleteWithError:)`) — these fire *during* a request,
  not at its end.
- **Mid-flight cancellation** (`URLSessionTask.cancel()`, or `Task.cancel()`
  on the awaiting Swift task) — there is nothing to signal while `perform`
  blocks.
- **Progress** (`task.progress`, byte counters) — the body arrives whole under
  `perform`; streaming byte counts are not observable.

The force from ADR-0005 still holds: the executor is cooperative and
single-threaded. A "resumable top-level run surface" that could park the
main run loop on host I/O (option C) would require heavy machinery in every
embedding and none currently needs it. Option B — the interpreter owns a small
event loop *while a request is in flight*, keeping the outer run surface
synchronous and unchanged — is the minimum effective extension of ADR-0010.

## Decision

**The `HttpTransport` trait grows three methods (`start`, `next_event`,
`cancel`) that together form an interpreter-owned event loop per in-flight
request. The one-shot `perform` becomes a provided convenience built on
them. The top-level run surface is not changed.**

### Core seam (`crates/tswift-core/src/http.rs`)

```rust
/// Opaque per-transport in-flight request id.
pub struct HttpTaskHandle(pub u64);

pub enum HttpEvent {
    /// Status line + headers arrived. Exactly one, first.
    Response { status: i64, headers: Vec<(String, String)> },
    /// One body fragment. Zero or more.
    Chunk(Vec<u8>),
    /// Terminal: success.
    Done,
    /// Terminal: failure carrying a `URLError.Code` case name.
    Failed { code: String, message: String },
}

pub trait HttpTransport {
    /// Required. Perform `req` blocking (used by provided defaults).
    fn perform(&mut self, req: &HttpRequest) -> Result<HttpResponse, HttpError>;
    /// Provided default: calls `perform` eagerly and wraps result in
    /// `SingleShotEvents` stored in a thread-local queue.
    fn start(&mut self, req: &HttpRequest) -> Result<HttpTaskHandle, HttpError>;
    /// Provided default: pops the next event from the thread-local queue.
    /// After a terminal event the handle is dead; subsequent calls return
    /// `Failed { code: "badServerResponse" }` sentinel.
    fn next_event(&mut self, h: HttpTaskHandle) -> HttpEvent;
    /// Provided default: replaces the pending queue with a single
    /// `Failed { code: "cancelled" }` terminal so the **next** `next_event`
    /// call returns that event (not the badServerResponse sentinel).
    fn cancel(&mut self, h: HttpTaskHandle);
}
```

> **Deviation from sketch:** The M1 implementation keeps `perform` as the
> **required** method and makes `start / next_event / cancel` **provided
> defaults** (backed by a thread-local `SingleShotEvents` queue). This is the
> reverse of what the sketch above shows, but it is the correct inversion for
> backward compatibility: all existing one-shot backends compile unchanged
> because they already implement `perform`. Native streaming backends (M2+)
> override all four methods. `cancel`'s provided default honours the cancel
> contract by replacing the pending queue with `Failed{cancelled}` rather than
> removing the entry, ensuring the caller can always drain exactly one terminal
> event after cancellation.

Two compatibility helpers keep simple backends one screenful:

- **Provided `perform`** — drives `start / next_event* / Done` synchronously;
  used by callers that do not care about events (existing call-sites).
- **`SingleShotEvents`** — adapter that wraps a `Result<HttpResponse, HttpError>`
  into the canonical `Response → Chunk(body) → Done` / `Failed` sequence.
  All existing backends are migrated to this adapter mechanically (M2); their
  observable behaviour is unchanged.

**Handle-lifetime invariant:** every handle returned by `start` must be
either drained to its terminal event or `cancel`led *and then polled once*
(to consume the `Failed{cancelled}` terminal). Handles that are abandoned
without draining or cancelling accumulate indefinitely in the thread-local
backing store. The interpreter is responsible for enforcing this contract; it
must cancel-and-drain any in-flight handle when the owning task is torn down.

Event-order contract: `Response` first, then zero or more `Chunk`, then exactly
one terminal event (`Done` or `Failed`). A terminal event before `Response` is
treated as failure-before-headers and maps to `badServerResponse`. The
interpreter loop enforces this; lenient handling maps any malformed sequence to
`badServerResponse` rather than panicking.

### Wire contract — event JSON codec

Shared between FFI and wasm (lives next to the existing `encode_request_json`
in `tswift_core::http`):

```jsonc
{"event": "response", "status": 200, "headers": [["Content-Type", "text/plain"]]}
{"event": "chunk", "bodyBase64": "aGk="}
{"event": "done"}
{"event": "error", "code": "timedOut", "message": "…"}
```

`encode_event_json` / `decode_event_json` are added with round-trip unit tests.
The existing request JSON and one-shot response JSON are **unchanged** — old
hosts keep working.

### Interpreter-owned event loop (Foundation driver)

`StdContext` gains `http_start`, `http_next_event`, `http_cancel` mirroring
the trait, plus `current_task_cancelled() -> bool` that exposes the executor's
existing cooperative cancellation flag (the one behind `Task.isCancelled`).

The Foundation request driver shared by all entry points:

```
start → loop {
    e = next_event
    dispatch delegate callback for e  (call_closure into script)
    update task counters / progress
    if script called task.cancel() or current_task_cancelled() {
        transport.cancel; continue draining to terminal event
    }
} → build (Data, URLResponse) or throw URLError
```

New Foundation surface (out of scope for this ADR to fully specify; scoped
here as context for the seam decision):

- `URLSessionTask` / `URLSessionDataTask` objects with mutable
  `state`/`cancel()`/`resume()`/`progress.fractionCompleted`/byte counters,
  modelled on the existing class-instance machinery with interior mutability.
- `dataTask(with:completionHandler:)` — `resume()` runs the driver immediately
  (blocking) and invokes the completion handler `(Data?, URLResponse?, Error?)`
  via `call_closure`.
- Delegate protocols (`URLSessionDelegate`, `URLSessionTaskDelegate`,
  `URLSessionDataDelegate`) registered as runtime protocols; dispatched per
  event only if the script type implements the optional method.
- Async paths `data(from:)`/`data(for:)`/`upload(for:from:)` rerouted through
  the driver; `current_task_cancelled()` polled between events.

### FFI streaming contract (additive, §2.6 of the plan)

```c
typedef void (*tswift_http_start_fn)(void *userdata, const char *request_json,
                                     void *task);
typedef void (*tswift_http_cancel_fn)(void *userdata, void *task);

void tswift_set_http_stream_handler(tswift_context *ctx,
                                    tswift_http_start_fn start,
                                    tswift_http_cancel_fn cancel,
                                    void *userdata);

// Host pushes events from ANY thread; token dies after the terminal event is consumed.
void tswift_http_event(void *task, const char *event_json);
```

The Rust side allocates a per-task `Arc<(Mutex<VecDeque<HttpEvent>>, Condvar)>`;
the token handed to the host is a raw pointer to a clone (freed when the
terminal event is consumed by `next_event`). `next_event` blocks on the condvar
with `HttpRequest.timeout_seconds` as deadline — expiry synthesizes
`Failed { code: "timedOut" }` and calls the host `cancel`. This design lets
the host push straight from its native delegate queue **with no trampoline and
no re-entrant interpreter calls**; delegate dispatch into script always happens
on the interpreter thread between `next_event` returns. The existing one-shot
`tswift_set_http_handler` path is kept verbatim (wrapped in `SingleShotEvents`).

### wasm degraded tier (additive, §2.7 of the plan)

The wasm build is single-threaded and cannot block on the main thread, so
`next_event` cannot wait. The existing `tswiftHttp` hook is extended with a
batch response form:

```jsonc
{"events": [ {"event":"response", ...}, {"event":"chunk", ...}, {"event":"done"} ]}
```

`JsHttpTransport::start` calls `tswiftHttp` once and queues the decoded events;
`next_event` pops synchronously; `cancel` drops the remainder. The old scalar
response form is auto-wrapped via `SingleShotEvents` and continues to work.

Semantics are honestly documented: delegates and progress replay faithfully;
**cancellation stops delivery but cannot abort an already-completed fetch**.
True streaming/abort on wasm (SharedArrayBuffer + `Atomics.wait`, or async
`fetch` behind a resumable run surface) is a separate future concern.

## Consequences

**Positive**

- Delegate callbacks, mid-flight cancellation, and byte-level progress are now
  expressible within the existing cooperative single-threaded executor — no
  resumable run surface required.
- All existing backends migrate mechanically via `SingleShotEvents`; every
  existing golden fixture passes unchanged.
- The FFI push-from-any-thread contract eliminates the semaphore hack in the
  `TSwiftCore` façade and maps directly onto what a native `URLSessionDataDelegate`
  host queue does — no adapter layer, no re-entrant interpreter calls.
- The wasm batch form replays delegate ordering and progress values
  deterministically; page authors who do not need streaming keep the existing
  scalar hook unchanged.

**Costs and accepted limits**

- **wasm cancellation is delivery-only.** `cancel` stops the interpreter from
  processing further events but cannot abort the already-completed native fetch.
  This is documented as a degraded tier, not a bug. True abort requires either
  SharedArrayBuffer + `Atomics.wait` or an option-C resumable run surface — both
  deferred.
- **FFI token lifetime is a contract.** Hosts must treat the `void *task` token
  as dead after pushing a terminal event. Pushing after that is a safe no-op
  by the `Arc` design, but hosts should not rely on it. An explicit test covers
  the post-terminal push case.
- **`async let` interleaving explicitly deferred.** Two overlapping requests become
  *mechanically possible* with this seam (two `HttpTaskHandle`s alive at once), but
  interleaving their `next_event` polls requires executor suspension points that do
  not exist in the current cooperative executor. This is left as future
  evidence-driven work; the seam does not foreclose it.
- **Timeout source of truth.** `next_event`'s deadline uses
  `HttpRequest.timeout_seconds`; the native FFI host may also time out
  independently. Both paths converge on `Failed { code: "timedOut" }`, but the
  FFI test suite must cover the host-timeout-before-interpreter-timeout case.
- **Re-entrant `call_closure` from a builtin.** Delegate dispatch calls script
  while the session driver is on the stack. `call_closure` is already used by
  builtins; composability with ADR-0004 async frames needs an explicit integration
  test (M4).
- **Option C follow-up.** A resumable top-level run surface (true async `fetch`
  on wasm, concurrent `async let` requests) is tracked in
  `docs/plan/resumable-run-surface-followup.md` and explicitly out of scope here.

## Known limitations

### URLSessionDataTask value semantics (deferred)

`URLSessionDataTask` is backed by `SwiftValue::Struct` (`Rc<StructObj>`), not
by a class-instance / handle-registry object.  Mutations from `cancel()` /
`resume()` are written back only to the **bound variable** that last received
the `Outcome::receiver` write-back — exactly the same as any Swift struct.

This diverges from the Swift stdlib, where `URLSessionDataTask` is a
**reference type** (class).  The observable effect: an alias or a captured copy
of a task will not see state changes made on the original binding:

```swift
var task = session.dataTask(with: url) { ... }
let snapshot = task   // copies the struct
task.resume()         // writes .running back to `task`
// snapshot.state is still .suspended — value semantics, not reference
```

Fix: back the task through the class-instance / handle-registry machinery (the
pattern used by SwiftUI session objects in the interpreter).  This requires
non-trivial interpreter work and is deferred.  Until then:

- Scripts **must** bind tasks to `var`, not `let`.
- Scripts **must not** rely on alias observation of `state`, `progress`, or
  byte counters through a captured copy or `let` binding.
- The `state` field is always accurate for the binding that last received the
  `Outcome::receiver` write-back (i.e., the `var` that owns the task).

The limitation is documented in `crates/tswift-foundation/src/urlsession.rs`
(module-level doc comment) and in `notes.md`.

### `has_method_on` ignores protocol-extension default methods (known limitation)

`Interpreter::has_method_on` walks the class inheritance chain checking the
`method_overloads` and `methods` maps of each `ClassDef`.  Protocol-extension
*default* implementations are not indexed in these maps — the interpreter does
not yet parse or store protocol-extension bodies in the type table.  As a
result, a class that conforms to `URLSessionDataDelegate` or
`URLSessionTaskDelegate` but does **not** explicitly override an optional
delegate method will have `has_method_on` return `false` for that method, and
the callback will be silently skipped.

In the tswift runtime this is the *correct* observable behaviour (optional
Foundation delegate methods with no script override should be no-ops), but it
diverges from a world where protocol-extension defaults could have observable
side-effects.  Fixing this would require indexing protocol-extension bodies in
the type table — non-trivial, deferred.

### `overload_labels_match` vs `args_select_params` (intentional strictness)

`has_method_on` uses `overload_labels_match`, which requires an exact
arg-count match (`len ==`).  The call-dispatch path uses `args_select_params`,
which handles variadic spans and default-valued parameters.  The strictness in
`overload_labels_match` is intentional: Foundation's delegate probes are
constructed by `delegate_probe_args` with *exactly* the labels that will be
passed on the real call, so defaults and variadics never arise.  If a future
delegate method uses a default or variadic, the probe must supply the
representative arg count and this note must be updated.

## Notes

- The full milestone breakdown (M1–M8), backend change table, testing strategy,
  and open-question detail live in `docs/plan/urlsession-event-transport.md`.
- `download*`, `bytes(from:)` as `AsyncSequence`, publishers, and
  auth-challenge/redirect delegate methods remain unmodelled; their tier status
  in `frameworks/foundation/scope.toml` is unchanged until M8.
