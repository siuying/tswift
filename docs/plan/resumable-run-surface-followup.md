# Follow-up note — resumable run surface ("option C", full run loop with main)

**Status:** deferred — do not start without a driving feature (see triggers)
**Date:** 2026-07-04
**Related:**
- `docs/plan/urlsession-event-transport.md` — the event seam ("option B") shipped instead
- `docs/adr/0005-cooperative-concurrency-executor.md` — the constraint C would reverse
- `docs/adr/0010-http-transport-seam.md` — first flagged the "pending protocol" idea

## What C is

Make the run surface **resumable**: `tswift_run` / `runSwift` may return
`pending` with unfinished tasks parked on host I/O; the host pumps events
(`tswift_deliver_event` / a JS callback) and the interpreter resumes
continuations across calls. Equivalently: the interpreter gets a real
`RunLoop.main` / main-queue that the *host* drives.

This reverses ADR-0005's core rule ("a continuation must resume before the run
call returns") and changes **every** embedding's contract plus the golden
fixture harness (fixtures are deterministic because a run is one synchronous
call). It is a full ADR + multi-milestone effort, not an increment on B.

## Triggers — write the ADR when any of these is actually wanted

- `Timer` / `RunLoop.main` / `DispatchQueue.main.async{After}` with real
  scheduling semantics.
- **True async networking on wasm** — plain `fetch` without
  SharedArrayBuffer/COOP/COEP, with real mid-flight abort. This is the only
  path that fixes wasm properly; option B's wasm tier is batch-replay only.
- `URLSession.bytes(from:)` as a genuine incremental `AsyncSequence`, or
  Combine publishers with host-driven delivery.
- Overlapping `async let` network requests (needs executor suspension points
  at transport waits; B's seam already permits it mechanically).
- A host (iOS app / web page) that must not block its UI thread during script
  network calls.

## What carries over from B (deliberately)

- The `HttpEvent` vocabulary and event JSON wire codec — unchanged.
- The `start / cancel` transport halves and the FFI `tswift_http_event`
  push-from-any-thread entry point — unchanged.
- Only "who blocks" moves: B blocks a Rust condvar inside `next_event`; C
  parks the Swift task and returns to the host. Foundation's driver loop and
  delegate dispatch are untouched.

## Sketch of the work (for future estimation, not commitment)

1. ADR-0012(+): resumable executor — task parking, wake tokens, persistence of
   coroutine frames (ADR-0004) across host returns.
2. New run surface: `tswift_run_start` → `{done|pending}`,
   `tswift_deliver_event`, `tswift_run_drain`; wasm equivalents returning to
   the JS event loop; CLI gains an internal epoll/park loop so its UX is
   unchanged.
3. Fixture harness: scripted event schedules (virtual clock) to keep goldens
   deterministic.
4. `RunLoop`/`Timer`/`DispatchQueue.main` modelled on the new loop.
