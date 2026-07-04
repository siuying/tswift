# Plan — URLSession event-driven transport (delegates, cancellation, progress)

**Status:** proposed
**Date:** 2026-07-04
**Related:**
- `docs/adr/0010-http-transport-seam.md` — the synchronous one-shot seam this evolves
- `docs/adr/0005-cooperative-concurrency-executor.md` — the executor constraint we stay inside
- `docs/plan/resumable-run-surface-followup.md` — the deferred "option C" follow-up
- `frameworks/foundation/scope.toml` — tier F6 notes (delegate/bytes/download unmodelled)

## 1. Problem statement

ADR-0010 shipped `URLSession.data/upload` over a one-shot synchronous
`HttpTransport::perform`. That shape cannot express:

- **delegate callbacks** (`urlSession(_:dataTask:didReceive:)`,
  `urlSession(_:task:didCompleteWithError:)`) — events *during* a request;
- **mid-flight cancellation** (`URLSessionTask.cancel()`, `Task.cancel()` on
  the awaiting Swift task) — nothing to signal while `perform` blocks;
- **progress** (`task.progress`, byte counters) — the body arrives whole.

Decision (this plan = "option B"): the interpreter owns a small **event loop
that runs while a request is in flight**. The transport seam becomes
`start / next_event / cancel`; the top-level run surface (`tswift_run`,
`runSwift`) stays synchronous and unchanged. We explicitly do **not** expose a
resumable "main run loop" to hosts — that is option C, see the follow-up note.

## 2. Design

### 2.1 Core seam (`crates/tswift-core/src/http.rs`)

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
    fn start(&mut self, req: &HttpRequest) -> Result<HttpTaskHandle, HttpError>;
    /// Block until the next event for `h`. After `Done`/`Failed`, `h` is dead.
    fn next_event(&mut self, h: HttpTaskHandle) -> HttpEvent;
    /// Best-effort abort. Transport must still deliver a terminal event
    /// (normally `Failed { code: "cancelled" }`) if `next_event` is called again.
    fn cancel(&mut self, h: HttpTaskHandle);
}
```

Compatibility helpers in core so simple backends stay one screenful:

- provided method `perform(&mut self, req) -> Result<HttpResponse, HttpError>`
  that drives the three methods (used by callers that don't care about events);
- `SingleShotEvents`: adapter turning a `Result<HttpResponse, HttpError>` into
  the canonical `Response → Chunk(body) → Done` / `Failed` sequence, for
  backends with no native streaming.

Event-order contract (enforced by the interpreter loop, tolerated leniently):
`Response` first, then `Chunk*`, then exactly one terminal event. A terminal
event before `Response` is a failure-before-headers. Anything malformed maps to
`badServerResponse`.

### 2.2 Wire contract (shared FFI/wasm codec, next to `encode_request_json`)

```jsonc
{"event": "response", "status": 200, "headers": [["Content-Type", "text/plain"]]}
{"event": "chunk", "bodyBase64": "aGk="}
{"event": "done"}
{"event": "error", "code": "timedOut", "message": "…"}
```

Add `encode_event_json` / `decode_event_json` in `tswift_core::http` with
round-trip unit tests. The existing request JSON and one-shot response JSON are
unchanged — old hosts keep working.

### 2.3 Interpreter plumbing (`crates/tswift-core`)

- `StdContext` (stdlib.rs) gains `http_start`, `http_next_event`, `http_cancel`
  mirroring the trait (keep `perform_http` as a convenience built on them).
- `StdContext` gains `current_task_cancelled(&self) -> bool` exposing the
  executor's existing cooperative cancellation flag (the one behind
  `Task.isCancelled`), so Foundation can poll it between events.
- Interpreter impl at `interp.rs` (`set_http_transport` unchanged; the impl
  block near line 4564 forwards the new methods).

### 2.4 Foundation surface (`crates/tswift-foundation/src/urlsession.rs`)

The request loop moves out of `perform` into a driver shared by all entry
points:

```
start → loop {
    e = next_event
    dispatch delegate callback for e (call_closure into script)
    update task counters/progress
    if script called task.cancel() or current_task_cancelled() {
        transport.cancel; continue draining to terminal event
    }
} → build (Data, URLResponse) or throw URLError
```

New/changed script API:

- **`URLSessionTask` / `URLSessionDataTask`** object with mutable state
  (`state`: `.suspended/.running/.canceling/.completed`, `cancel()`,
  `resume()`, `progress.fractionCompleted`, `countOfBytesReceived`,
  `countOfBytesExpectedToReceive` from `Content-Length`). Needs interior
  mutability — model on the existing class-instance machinery, not
  `Rc<StructObj>`.
- **`dataTask(with: URL|URLRequest, completionHandler:)`** — under the
  cooperative executor, `resume()` runs the driver immediately (blocking) and
  invokes the completion handler `(Data?, URLResponse?, Error?)` via
  `call_closure`. `cancel()` before `resume()` completes with
  `URLError(.cancelled)` without touching the transport.
- **Delegate protocols** registered as runtime protocols:
  `URLSessionDelegate`, `URLSessionTaskDelegate`
  (`urlSession(_:task:didCompleteWithError:)`), `URLSessionDataDelegate`
  (`urlSession(_:dataTask:didReceive:completionHandler:)` for the response —
  honour `.allow`/`.cancel` — and `urlSession(_:dataTask:didReceive:)` for
  chunks). Stored via `URLSession(configuration:delegate:delegateQueue:)`
  (`delegateQueue` accepted and ignored — single-threaded). Each callback is
  dispatched only if the script type implements it (they are optional in
  Foundation).
- **Async paths** `data(from:)/data(for:)/upload(for:from:)` reroute through
  the driver; they check `current_task_cancelled()` between events and throw
  `URLError(.cancelled)` — this is the `Task { }.cancel()` integration.

Out of scope (unchanged in `scope.toml` notes): `download*`, `bytes(from:)`
as an `AsyncSequence`, publishers, auth-challenge/redirect delegate methods.

### 2.5 Backends

| Backend | Change |
|---|---|
| **Mock** (`tswift-core` + `tswift-cli/src/httpmock.rs`) | `MockRoute` gains optional scripted `chunks` (route JSON `"chunksBase64": ["...", ...]`) and optional mid-stream `"failAfterChunks"` error — enables deterministic delegate/progress/cancel fixtures. Default remains single-chunk via `SingleShotEvents`. |
| **CLI ureq** (`tswift-cli/src/nethttp.rs`) | Stream the body reader in fixed-size chunks (~64 KiB) instead of `read_to_end`; `cancel` drops the reader. Real mid-flight cancellation and progress. |
| **FFI** (`tswift-ffi/src/http.rs`) | See §2.6. Existing `tswift_set_http_handler` one-shot path kept verbatim (wrapped in `SingleShotEvents`). |
| **wasm** (`tswift-wasm/src/lib.rs`) | See §2.7. Existing `tswiftHttp` sync hook kept. |

### 2.6 FFI contract (additive)

```c
// Registered together; start must return quickly (fire the native request).
typedef void (*tswift_http_start_fn)(void *userdata, const char *request_json,
                                     void *task /* opaque token */);
typedef void (*tswift_http_cancel_fn)(void *userdata, void *task);

void tswift_set_http_stream_handler(tswift_context *ctx,
                                    tswift_http_start_fn start,
                                    tswift_http_cancel_fn cancel,
                                    void *userdata);

// Host pushes events — callable from ANY thread, 0..n times, ending with a
// terminal "done"/"error" event. The token dies after the terminal event.
void tswift_http_event(void *task, const char *event_json);
```

Rust side: per-task `Arc<(Mutex<VecDeque<HttpEvent>>, Condvar)>`; the token
handed to the host is a raw pointer to an `Arc` clone (freed when the terminal
event is *consumed* by `next_event`). `next_event` blocks on the condvar with
the request timeout as deadline → synthesize `Failed { code: "timedOut" }` and
call the host `cancel`.

Why this beats the "emit re-entrancy" alternative: the host can push straight
from its native `URLSessionDataDelegate` queue with **no trampoline and no
re-entrant interpreter calls** — delegate dispatch into script always happens
on the interpreter thread between `next_event` returns. The `TSwiftCore`
façade's semaphore hack is replaced by plain event pushes. Cancellation is a
normal outbound `cancel(userdata, token)` call between events.

ABI drift-guard test in `tswift-ffi/src/lib.rs` extended for the three new
symbols; C header + façade docs updated (`docs/plan/native-host.md`).

### 2.7 wasm contract (additive, degraded tier)

The wasm build is single-threaded and cannot block on the main thread, so
`next_event` cannot wait. Extend the **existing** hook's response JSON with a
batch form:

```jsonc
{"events": [ {"event":"response", ...}, {"event":"chunk", ...}, {"event":"done"} ]}
```

`JsHttpTransport::start` calls `tswiftHttp` once and queues the decoded
events; `next_event` pops; `cancel` drops the remainder. Semantics documented
honestly: delegates and progress replay faithfully; cancellation stops
*delivery* but cannot abort the already-completed fetch. The old scalar
response form still works (auto-wrapped via `SingleShotEvents`).

True streaming/abort on wasm (SharedArrayBuffer + `Atomics.wait`, or async
`fetch` behind a resumable run surface) is deferred — see the option-C note.

## 3. Milestones

Each milestone is presubmit-green and independently committable.

- **M0 — ADR.** Write ADR-0011 "event-driven HTTP transport" amending
  ADR-0010 (decision = this plan §2; consequences include the wasm degraded
  tier and the C follow-up pointer).
- **M1 — core seam.** `HttpEvent`, trait `start/next_event/cancel` + provided
  `perform`, `SingleShotEvents`, event JSON codec, `MockHttpTransport` chunked
  routes. Unit tests incl. codec round-trip and event-order lenience.
- **M2 — interpreter plumbing.** `StdContext::{http_start,http_next_event,
  http_cancel,current_task_cancelled}`; interpreter impls; migrate existing
  transports mechanically (mock/ureq/ffi-one-shot/wasm wrap in
  `SingleShotEvents`). Behaviour identical; all existing golden fixtures pass.
- **M3 — Foundation driver + URLSessionTask.** Event-loop driver;
  `URLSessionTask` object; `dataTask(with:completionHandler:)` +
  `resume/cancel/state/progress/counters`; async paths rerouted with
  `Task`-cancellation checks. Golden fixtures: completion-handler flow,
  pre-flight cancel, `Task.cancel()` → thrown `URLError(.cancelled)`.
- **M4 — delegates.** Protocol registration, session `delegate:` init,
  per-event dispatch, `.allow`/`.cancel` disposition. Golden fixtures with
  chunked mock routes: callback ordering, progress accumulation, mid-stream
  scripted failure, delegate-initiated cancel.
- **M5 — CLI streaming.** Chunked `ureq` reads + cancel-by-drop. Manual
  `--allow-network` smoke test (not in CI).
- **M6 — FFI streaming.** `tswift_http_event` + stream-handler registration,
  queue/condvar, timeout synthesis, token lifetime tests (incl. a
  push-from-another-thread test), ABI drift guard, header/façade docs.
- **M7 — wasm batch events.** `{"events":[...]}` decoding in
  `JsHttpTransport`, docs for page authors.
- **M8 — bookkeeping.** `frameworks/foundation/scope.toml` (delegate/task
  members move to modelled; notes updated), `docs/swift-runtime/feature-checklist.md`,
  website sync (`update-website` skill).

## 4. Testing strategy

- Unit tests per crate as listed in milestones (`scripts/presubmit` green
  before every commit).
- Golden fixtures under the existing `<name>.swift` + `<name>.http.json`
  scheme (`crates/tswift-cli/tests/golden.rs`); chunked routes make delegate
  ordering, progress values, and cancellation deterministic and offline.
- FFI: pure-Rust tests calling the `extern "C"` surface directly (pattern in
  `tswift-ffi/src/http.rs` tests today), plus one multi-thread push test.

## 5. Risks / open questions

- **Task-object mutability**: `URLSessionTask` needs interior mutability;
  confirm the class-instance machinery supports builtin-defined classes, else
  add a small handle registry in the interpreter (pattern: SwiftUI session
  objects).
- **Re-entrant `call_closure` from a builtin**: delegate dispatch calls script
  while `session_data` is on the stack. `call_closure` is already used by
  builtins (e.g. `sorted(by:)`); verify it composes with `async` frames
  (ADR-0004 coroutine frames).
- **FFI token lifetime**: host pushing after the terminal event, or after
  timeout-cancel, must be a safe no-op — the `Arc` design covers it; needs an
  explicit test.
- **Timeout source of truth**: `next_event` deadline uses
  `HttpRequest.timeout_seconds`; hosts may also time out natively — both paths
  must converge on `timedOut`.
- **`async let` interleaving** (two requests overlapping on the wire) becomes
  *possible* with this seam but requires executor suspension points at
  `next_event` — explicitly out of scope here; note in ADR-0011 as future
  evidence-driven work.
