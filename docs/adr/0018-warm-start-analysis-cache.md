# ADR 0018: Warm-start `Analysis` cache (runtime caching, not compilation)

Status: accepted
Date: 2026-07-11

## Honest naming

tswift is an **interpreter** with a **pure-Rust frontend** (lexer → parser →
sema). There is **no ahead-of-time codegen**: nothing is "compiled to wasm".
The runtime tier is precisely:

> **interpreter + warm-start caching** — the Swift frontend produces an
> `Analysis` (the type-resolved parse AST) which a tree-walking interpreter
> then executes. The only durable artifact reused across runs is that
> `Analysis`; execution is always fresh.

Slice 17 adds warm-start caching. It is **not** compilation, incremental or
otherwise, and must never be described as such in docs, UI copy, or code.

## Context

The web entry points (`runSwift`, `runSwiftModule`, `swiftUICompile`,
`swiftUICompileModule`) re-analyze the full program on every call. Interactive
surfaces resubmit byte-identical source constantly: a Studio "Run" pressed
twice, an embed iframe refresh, a SwiftUI recompile after a no-op edit. Each of
those repaid the entire lex/parse/sema cost for a result already computed.

## Decision

Cache the frontend `Analysis` keyed by the full program input, in the wasm
crate (`crates/tswift-wasm/src/analysis_cache.rs`). A re-submission of the same
source reuses the prior `Analysis` and skips lex/parse/sema; the interpreter is
still constructed and run fresh, so all program side effects (stdout, host
calls, SwiftUI render) are byte-for-byte unchanged. **The cache is invisible
except as reduced latency.**

Key design points:

- **Key = full program bytes**, length-prefixed so concatenation can't alias
  (`"1:" + filename + source` for single-file; ordered `path`/`source` pairs
  for modules). A `DefaultHasher` digest indexes the entry; every hit is
  confirmed by a **full byte comparison** of the stored key, so a hash
  collision degrades to a re-analyze, never a wrong-`Analysis` hit. No new
  dependency (std hasher only).
- **Real ownership via `Rc`, freed on eviction.** The cache owns each
  `Analysis` behind an `Rc<Analysis>` and hands out `Rc` clones. A miss
  analyzes once; an evicted entry drops the cache's `Rc`, and the backing AST
  is reclaimed as soon as no interpreter still holds a clone. The cache does
  **not** `Box::leak`, so its retained memory is bounded by `CACHE_CAP`
  regardless of how many *distinct* programs are submitted over the process
  lifetime. (The earlier draft leaked a `&'static Analysis` per miss and merely
  orphaned the leak on eviction — unbounded growth on unique submissions.)
- **The `'static` interpreter contract, satisfied without leaking.** The
  tree-walking interpreter is built on `Node<'static>` cursors into the AST
  (`Interpreter::run` takes `&'static Analysis`), and a SwiftUI session holds
  those cursors across dispatch calls, so the AST genuinely must outlive each
  run. Callers therefore pass their `Rc<Analysis>` to
  `Interpreter::run_retaining`, which retains the `Rc` for the interpreter's
  lifetime (the same bounded-ownership model as `FragmentCache`). The single
  `unsafe` deriving `'static` from the retained `Rc` lives in `tswift-core`;
  `tswift-wasm` stays `#![forbid(unsafe_code)]`.
- **Bounded (LRU, N=4).** Only the four most-recent distinct programs are kept
  warm; eviction actually frees (real ownership), so total cache memory is
  bounded by `CACHE_CAP`, not by distinct sources seen.
- **Entry-mode + per-file keying.** Key material carries an entry-mode tag
  (`run` single/multi, SwiftUI compile single/multi) and, for multi-file
  inputs, the ordered per-file `(path, source)` pairs — each length-prefixed —
  so a module `[a, b]` can never alias the single source `a + b`, nor can a
  `run` submission alias a SwiftUI one (which prepends a different prelude).

## Per-platform applicability (named honestly)

- **wasm (web): benefits.** The wasm module is a long-lived in-process instance
  (Studio, playground, embed), so an in-memory cache survives across calls.
  This is the target of the slice.
- **CLI (native): does not benefit, intentionally skipped.** `tswift run` is
  process-per-invocation; an in-memory cache never sees a second call in the
  same process, so it would be pure overhead. The CLI is left unchanged.
- **iOS FFI:** not wired in this slice; the same cache pattern could be adopted
  behind the C ABI later if a host proves a repeated-submission workload.

## Measurements

Measured natively through the real `run_swift_impl` path (the wasm entry point
compiled for the host; see the `#[ignore]`d `bench_warm_start` in
`crates/tswift-wasm/src/lib.rs`). The benchmark emits exactly these two program
sizes and reports the **median** wall time over 51 samples; `warm` is a re-run
of byte-identical source (cache hit), `cold` re-analyzes a freshly-unique
source each sample (cache miss). Numbers below are a representative run on an
Apple M3 Max, `--release`:

| program           | cold run | warm run | saved                |
| ----------------- | -------- | -------- | -------------------- |
| 162-line program  | ~7.6 ms  | ~7.2 ms  | ~0.3–0.5 ms (~4–7%)  |
| 594-line program  | ~27.3 ms | ~26.0 ms | ~1.1–1.6 ms (~4–6%)  |

Absolute run times are machine-dependent (the tree-walking interpreter
dominates end-to-end time). The reproducible invariant is the **`saved`
delta**: it tracks the elided lex/parse/sema cost — a few tenths of a
millisecond at ~160 lines, ~1–1.5 ms at ~600 lines — and grows with program
size. The delta is small and shows run-to-run variance, so warm-start caching
is honestly a latency trim on re-runs, not a step change. To reproduce:
`cargo test -p tswift-wasm --release bench_warm_start -- --ignored --nocapture`.

## Consequences

- Behavior-preserving: no output, diagnostic, or ordering change; only latency.
- Memory bounded by `CACHE_CAP` retained programs and *actually freed* on
  eviction (real `Rc` ownership) — no permanent leak, even on an unbounded
  stream of unique submissions.
- If a future slice wants larger wins it must target the interpreter/framework
  install path, not analysis — this ADR is the tripwire recording that analysis
  is no longer the place to look.
