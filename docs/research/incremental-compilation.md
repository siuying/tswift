# Incremental compilation for the tswift runtime — investigation

Status: investigation only (no implementation)
Date: 2026-07-11

## Framing: what "incremental" can even mean here

tswift is an **interpreter with a pure-Rust frontend**, not an AOT compiler.
There is no codegen, object file, or linked artifact — the only durable product
of the frontend is an `Analysis` (the type-resolved parse AST) that a
tree-walking interpreter executes fresh every run (ADR-0018). So "incremental
compilation" cannot mean the classic swiftc/LLVM sense (per-file object files,
module partitioning, driver-level dependency graphs). For this runtime it can
only mean one of three concrete things:

1. **Frontend result reuse across submissions** — skip lex/parse/sema when the
   input (or part of it) is byte-identical to a previous run. This already
   exists in a coarse, whole-program form (ADR-0018, wasm only).
2. **Per-file frontend caching with cross-file invalidation** — cache each
   file's lex/parse (and ideally sema) result, and re-run only the parts a
   given edit touches, merging the rest from cache. This is the "true"
   incremental ask and the subject of this doc.
3. **Session/runtime reuse** — keep the interpreter, installed stdlib, host
   services, and fragment cache warm across runs so only the *changed* program
   re-analyzes. Partially exists (FFI `Context`, wasm session).

The load-bearing question for (2): **can the frontend produce and merge
per-file `Analysis` fragments, or is a compilation unit irreducibly one AST?**

## Current architecture — what the code actually does

Verified in code, not from memory:

- **One source, one AST, one sema pass.** `tswift_frontend::Analysis::analyze`
  calls `tswift_parser::parse(&str) -> Ast` then `tswift_sema::analyze(&mut Ast)`.
  The parser produces exactly one `source_file` root; there is no multi-AST
  merge API (`crates/tswift-frontend/src/lib.rs`, ADR-0017).
- **Multi-file is concatenation, not linking.** `Analysis::analyze_program`
  concatenates the ordered `SourceFile`s once into a single combined source,
  keeps a `FileSpan { start_line, path }` table, and remaps every diagnostic
  back to `(path, local_line)` (ADR-0017). There is exactly **one**
  lex/parse/sema pass over the whole concatenation — no per-file frontend
  boundary survives into the AST.
- **Sema is whole-unit by construction.** `tswift_sema::analyze` first does
  `Symbols::collect(ast)` over the entire AST (gathering every type/func/global
  declaration into one registry), then runs each `Pass` over the whole tree
  against that shared registry (`crates/tswift-sema/src/passes.rs`). Name and
  type resolution in file `b` reads declarations collected from file `a`
  through this single registry. There is no per-file symbol table and no
  dependency edge recorded between a use and the declaration it resolved to.
- **The interpreter needs `Node<'static>` into a stable arena.** `Interpreter::run`
  takes `&'static Analysis`; a SwiftUI session holds those cursors across
  dispatch calls, so the AST must outlive each run. Ownership is either a
  bounded `Box::leak` (CLI, one-shot), or an `Rc<Analysis>` retained via
  `run_retaining` (wasm cache), or the FFI `Context` bundle (CONTEXT.md).
- **Warm-start `Analysis` cache already exists (wasm).** `analysis_cache.rs`
  keys a `DefaultHasher` digest of length-prefixed full program bytes
  (entry-mode tag + ordered per-file `(path, source)` pairs), confirms hits by
  full byte comparison, owns each `Analysis` behind an `Rc`, and frees on LRU
  eviction (N=4). **It is whole-program:** any change to any file is a cache
  miss that re-analyzes the entire concatenation. CLI is process-per-run and
  intentionally uncached; FFI is unwired but the pattern is available.
- **Fragment cache is a different axis.** `interp/fragment_cache.rs` caches
  interpolation-fragment analyses *within/across a session* (ADR-0007); it is
  runtime, not frontend, caching and does not partition the program.

### What this means for incremental

The current whole-program cache gives the *warm re-run* win (edit nothing, run
again → free). What it does **not** give is the *edit-one-file* win: in a
Studio project of N files, editing one file invalidates the whole-program key
and re-analyzes all N. True per-file incremental would need the frontend to
(a) analyze files independently enough to cache each, and (b) know which cached
results a given edit invalidates. Neither exists today, and the concatenation
model actively erases the file boundary before sema ever runs.

Measured context (ADR-0018): analysis is **not** the bottleneck. Execution is
~90%+ of every cold run; total install + all analysis is under ~1.6 ms even for
a 1000-line program (~0.88 ms user analysis at 594 lines). The warm-start cache
already elides that ~0.3–1.6 ms on re-runs. So incremental frontend caching is
optimizing a slice of the ~5% of wall time that is not execution.

## Options (with effort estimates)

### Option A — Do nothing beyond the existing whole-program cache

Keep ADR-0018's whole-program `Analysis` cache; optionally wire the same
pattern into the FFI `Context` (a host that resubmits identical input). No
per-file granularity.

- **Effort:** ~0 (already shipped on wasm); FFI wiring ~0.5 day if a host needs
  it.
- **Win:** warm re-run of *unchanged* input is free. Zero win on edit-one-file.
- **Risk:** none. Behavior-preserving.

### Option B — Per-file parse cache, whole-unit re-sema

Cache each file's **lex+parse** `Ast` fragment keyed by that file's bytes. On a
multi-file run, reuse unchanged files' parsed fragments and re-parse only edited
files, then still run **one** sema pass over the merged tree.

- **Requires:** an AST-merge/splice capability the parser does not have today
  (splice N `source_file` roots into one arena, preserving per-node provenance
  for diagnostics). ADR-0017 already names this as "a much larger change
  touching the AST arena and every `Node::line()` consumer."
- **Effort:** ~1–2 weeks. New arena-merge API in `tswift-ast`, provenance
  tracking, parser cache keyed per file, and re-validation that every
  `Node::line()`/diagnostic-remap consumer still works.
- **Win:** saves **parse** time for unchanged files only. But parse is the
  cheap half; sema still re-runs whole-unit (`Symbols::collect` + passes over
  the full tree). Given analysis is already sub-ms–~1 ms total, the realized
  saving is a fraction of a millisecond — below the ADR-0018 measurement noise
  floor.
- **Risk:** high complexity for a saving that likely does not clear the median
  benchmark bar. Introduces a second source of truth for the AST arena.

### Option C — Per-file sema with a cross-file dependency graph (true incremental)

Give sema a per-file symbol table, record use→decl dependency edges, and on an
edit re-run `Symbols::collect` + passes only for the edited file and its
dependents (transitively). This is the swiftc "incremental" analogue adapted to
a tree-walker.

- **Requires:** re-architecting `tswift-sema` from whole-unit
  (`Symbols::collect(ast)` over everything, passes over everything) into a
  resumable/linkable form with a persisted symbol graph and invalidation. Plus
  the Option B arena-merge (sema fragments must compose). ADR-0018 already
  rejected a related "separable prelude `Analysis`" idea for exactly this
  reason: "sema must resolve user types against prelude declarations in one
  pass … a large, risky re-architecture."
- **Effort:** ~4–8 weeks, touching the deepest, least-forgiving layer (sema +
  AST arena) and every diagnostic/`Node::line()` consumer.
- **Win:** on a large multi-file project, editing one file re-analyzes only that
  file + dependents instead of all N. But the absolute frontend cost being
  optimized is ~1 ms total today; even a 10× project would put whole-unit
  re-sema at ~10 ms against a ~250 ms execution wall.
- **Risk:** very high. Rewrites the correctness-critical resolver; large blast
  radius; optimizes a non-bottleneck.

### Option D — Session/runtime reuse (the lever with real headroom)

Orthogonal to frontend caching: keep the **interpreter + installed
stdlib/Foundation/SwiftUI + host services + fragment cache** warm across runs,
re-analyzing only the submitted program. The FFI `Context` (CONTEXT.md) and the
wasm session already own a reusable bundle; a host that reuses one `Context`
across runs keeps the fragment cache and installed stdlib persistent.

- **Effort:** ~1–3 days to document + wire an explicit "reuse session" path in
  FFI and measure it; the ownership model already exists.
- **Win:** avoids re-`install`ing builtins (~0.30 ms, flat) and re-warming the
  fragment cache per run. Small, but this is the only cost *outside* the
  execution wall that recurs every run and is cheaply reclaimable.
- **Risk:** low–moderate. Session state must be reset correctly between runs
  (finalizers, host handles) to stay behavior-preserving.

## Recommendation

**Do not build per-file incremental frontend caching (Options B/C) now.** The
evidence points the other way:

1. **It optimizes a non-bottleneck.** Execution is ~90%+ of wall time
   (ADR-0018). Total analysis is sub-ms–~1 ms; the whole-program warm cache
   already elides it on re-runs. Per-file granularity chases a fraction of the
   remaining ~5%.
2. **It fights the architecture at its least-forgiving layer.** The
   concatenation model (ADR-0017) and whole-unit sema (`Symbols::collect` +
   passes over one tree) mean per-file reuse requires an AST-arena merge plus a
   resumable resolver — precisely the "large, risky re-architecture" ADR-0018
   already rejected for the prelude-snapshot idea.
3. **The realistic interactive workload is already served.** Studio's live
   error feedback uses `swiftDiagnostics`/`swiftDiagnosticsModule` (cheap,
   whole-unit) and re-runs reuse the whole-program cache. Editing a file *should*
   re-analyze — that is where new diagnostics come from — and doing so costs
   ~1 ms.

**Instead:**

- **Keep Option A.** Optionally extend the ADR-0018 cache pattern to the FFI
  `Context` if a native host proves a repeated-identical-submission workload
  (cheap, contained, behavior-preserving).
- **Prefer Option D when a host needs "keep it warm."** Session/runtime reuse is
  the only lever outside the execution wall that recurs every run and is cheaply
  reclaimable; the ownership bundle already exists.
- **If execution ever needs to get faster, that is a different project** (a
  bytecode/threaded-interpreter step, ADR-0018's standing tripwire), not
  incremental frontend caching.

### Tripwire — when to reopen per-file incremental (Option C)

Revisit only if **all** of these become true, measured on the committed
benchmark (not assumed):

- A real project workload has **many files** (tens+) and **frontend analysis
  becomes a measurable median cost** (e.g. >20% of wall time), i.e. execution is
  no longer the wall — which today it overwhelmingly is.
- Editing a single file in that workload demonstrably re-analyzes enough
  unchanged code to matter to an interactive user (>~50 ms perceptible lag
  attributable to sema, not execution).
- The team is prepared to re-architect `tswift-sema` into a resumable,
  per-file, dependency-tracked resolver and add an AST-arena merge — with the
  correctness re-validation that entails.

Until then, whole-program warm-start caching (Option A) + optional session
reuse (Option D) is the correct, honest tier.
