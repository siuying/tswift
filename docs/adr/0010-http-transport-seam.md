# ADR-0010: URLSession over a synchronous, embedding-owned HTTP transport seam

- **Status:** Accepted — amended by ADR-0011
- **Date:** 2026-07-04
- **Context slice:** Foundation networking (`URLRequest`/`URLResponse`/`URLError`/`URLSession`) and every embedding (CLI, ffi, wasm)
- **Builds on:** ADR-0005 (cooperative single-threaded executor), the serialized-boundary contract (CONTEXT.md)
- **Drives:** `frameworks/foundation/scope.toml` tier F6; the golden-fixture mock routes (`<name>.http.json`)

## Context

Scripts want `URLSession` (`try await URLSession.shared.data(from:)`), but the
runtime runs inside very different hosts: an offline test harness, a CLI on a
developer machine, an iOS/macOS app embedding `tswift-ffi`, and a browser page
running the wasm build. Each has a different "right" way to reach the network
— and some (fixtures, sandboxed embeds) must never reach it at all.

Two forces shape the design:

1. **The executor is cooperative and single-threaded (ADR-0005).** A
   `with*Continuation` must be resumed before it returns; there is no
   scheduler that can park a task on host I/O and yield to an event loop.
   An "async" transport would require a resumable top-level run surface in
   every host — heavy machinery no current embedding needs.
2. **Capability, not policy, belongs to the embedding.** Whether the network
   is reachable — and through what stack (rustls, the platform's URLSession,
   a browser's fetch) — is the host's decision, exactly like the serialized
   boundary keeps object ownership host-side.

## Decision

One **synchronous** trait in `tswift-core::http`:

```rust
pub trait HttpTransport {
    fn perform(&mut self, req: &HttpRequest) -> Result<HttpResponse, HttpError>;
}
```

installed per-interpreter (`Interpreter::set_http_transport`) and reached by
Foundation through `StdContext::perform_http`. `URLSession.data/upload`
lowers `URLRequest` into an `HttpRequest`, performs one blocking call, and
builds the `(Data, URLResponse)` tuple.

The embedding matrix (each backend ~a screenful):

| Embedding | Backend | Wire |
|---|---|---|
| tests / golden fixtures | `MockHttpTransport` route table (`TSWIFT_HTTP_MOCK`, sibling `<name>.http.json`) | native structs |
| CLI `--allow-network` | `ureq` + rustls, blocking | native structs |
| `tswift-ffi` hosts | host handler registered via `tswift_set_http_handler`; must call `tswift_http_respond` before returning (may block internally — the `TSwiftCore` façade wraps a real `URLSession` task in a semaphore) | request/response JSON, bodies base64 |
| wasm | synchronous `globalThis.tswiftHttp(requestJson)` hook | same JSON (codec shared in `tswift_core::http`) |

Error taxonomy is part of the seam: transport failures carry a **`URLError.Code`
case name** (`"timedOut"`, `"cannotFindHost"`, …) and surface to scripts as
thrown `URLError` values; non-2xx statuses are *responses*, not errors,
matching Foundation. A **missing transport is an interpreter error**
(`unsupported construct`), not a Swift-catchable `URLError` — a sandboxed
script must not be able to mistake "no network capability" for "network down".

## Consequences

- Fixtures stay deterministic and offline by construction: the mock answers
  from a route table and unrouted requests fail like an unknown host.
- A blocking `perform` occupies the interpreting thread; concurrent `async
  let` requests serialize. Acceptable for scripts; revisit only with evidence.
- The wasm hook must be synchronous (scripted answers, sync XHR, or
  `Atomics.wait`+`SharedArrayBuffer` worker bridges). If true async fetch is
  ever required, the run surface needs a resumable "pending" protocol — a
  separate ADR; this seam's `HttpError`/wire contract would not change.
- `ureq` is the repo's first networking dependency (CLI only; core/ffi/wasm
  stay dependency-free). The offline-build rule in
  `docs/agents/environment.md` is amended accordingly.
- Delegate-, publisher-, download- and bytes-based `URLSession` members remain
  unmodelled (documented in `frameworks/foundation/scope.toml`).
