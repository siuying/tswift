# Swift Runtime — Implementation Plan

**What we're building:** the missing *back third* of a Swift compiler — a runtime that
**runs** Swift by consuming the typed, immutable AST that `msf` produces
(lexer → parser → 3-pass sema). Tree-walking interpreter first; optional bytecode VM later.

**Reads with this plan:**
- `docs/research/2026-06-24-implementing-a-swift-runtime-on-msf-ast.md` — architecture & rationale
- `docs/swift-runtime/feature-checklist.md` — the full Swift 6.3 feature surface (tiers, FE/RT/phase)
- `docs/research/2026-06-24-msf-swift-frontend.md` — what the frontend gives us
- `docs/research/2026-06-24-minijs-building-a-vm-for-javascript.md` & `...quickjs-ng...` — the proven VM recipe

**Guiding principle (from both VM studies):** *Get it running first, then make it fast.*
Correctness via a typed tree-walker (R0–R5) before any bytecode VM (R6).

---

## 1. Project setup & conventions

### 1.1 Language, layout, build
- **Language:** C11 (matches msf; links its `libMiniSwiftFrontend.a` directly).
- **Repository:** a new sibling project, working name **`swiftrun`** (the runtime).
  Depends on msf via `#include <msf.h>` + static lib — no source coupling.

```
swiftrun/
├── Makefile                 # debug/release/asan/test; links libMiniSwiftFrontend.a
├── include/
│   └── swiftrun.h           # public API: swiftrun_run_file(), error reporting
├── src/
│   ├── main.c               # CLI entry: parse args, drive msf, interpret
│   ├── driver.c             # msf glue: analyze → check errors → interpret
│   ├── backend_stubs.c      # provides module_stub_find / sema_import_module seam
│   ├── value/               # SwiftValue model, ARC, CoW buffers
│   │   ├── value.c/.h       # tagged union, retain/release, copy semantics
│   │   ├── string.c         # UTF-8 String/Character/Substring
│   │   └── collection.c     # Array/Dictionary/Set (CoW backings)
│   ├── runtime/
│   │   ├── interp.c/.h      # eval(node, env) tree-walker — the core
│   │   ├── env.c/.h         # Scope chain + intern pool (atoms)
│   │   ├── frame.c/.h       # call frames, defer lists, completion signals
│   │   ├── decl.c           # declare-hoist pass (mirrors msf sema pass 1)
│   │   ├── call.c           # call dispatch hub (fn/method/init/enum-case/closure)
│   │   ├── pattern.c        # match(pattern,value,env) for switch/if-case
│   │   ├── lvalue.c         # lvalue resolution for assignment + inout aliasing
│   │   ├── types.c          # type_kind dispatch, casts (is/as?/as!)
│   │   ├── conformance.c    # witness tables from msf ConformanceTable
│   │   └── generics.c       # monomorphization via msf type_substitute
│   ├── stdlib/              # native C builtins registered into global scope
│   │   ├── builtins.c       # print/assert/precondition/fatalError/min/max/…
│   │   ├── numeric.c        # Int/UInt widths, Float/Double, overflow/wrap ops
│   │   ├── sequence.c       # Sequence/Collection + map/filter/reduce/sorted
│   │   └── codable.c        # (R5) Encodable/Decodable
│   └── ir/                  # (R6, optional) AST→IR→bytecode VM
│       ├── ir.c             # basic blocks, virtual regs
│       ├── regalloc.c       # liveness + graph coloring
│       ├── bytecode.c       # lowering + jump patching
│       └── vm.c             # switch/computed-goto interpreter loop
└── tests/
    ├── fixtures/            # *.swift + *.expected (golden stdout)
    ├── unit/                # C unit tests per module
    └── run_fixtures.c       # harness: run fixture, diff stdout vs expected
```

### 1.2 Coding conventions
- Mirror msf style: single public header, arena allocation, intern + compare by id.
- **Never mutate or free msf's AST/TypeInfo** — they're arena-owned by `MSFResult`,
  immutable, and must outlive execution. Keep one `MSFResult*` alive per run.
- All allocation goes through a runtime arena + an ARC heap; one teardown at exit.
- `-O0 -g` for debug builds so the conservative-stack assumptions (if any) hold; ASan target.

### 1.3 The msf integration seam (do this in week 1)
msf expects the backend to provide two symbols (today stubbed in `tests/stubs.c`):
- `module_stub_find(name)` — SDK module table lookup (return NULL → no SDK).
- `sema_import_module(ctx, name)` — feed import vocabularies.

`backend_stubs.c` starts by returning NULL (like the test stub), so `import` resolves
to nothing and we rely on `msf_analyze` populating `node->type` for built-ins. Later we
feed a **vocabulary describing our supported stdlib** (`msf_analyze_with_vocab` /
`MSFModule`) so references to our `Array`, `String`, etc. resolve and get typed.

**Driver flow** (`driver.c`):
```c
MSFResult *r = msf_analyze(source, filename);
if (msf_error_count(r) > 0) { report_errors(r); return EX_DATAERR; }
Interp in; interp_init(&in, r);
int code = interp_run(&in, msf_root(r));   // walks top-level decls + statements
interp_dispose(&in); msf_result_free(r);
```

---

## 2. The core engine (build once, in R0; everything else plugs in)

These four pieces are the spine; every feature is added by extending them, not rewriting.

1. **`SwiftValue`** (`value/value.c`) — tagged union (see design doc §2). Unboxed
   `Void/Bool/Int/Double`; ref-counted `String`; `Struct`/`Enum` value types (copy on
   assign); `Class` reference type (ARC); `Closure`; `Optional`; `Array/Dict/Set` (CoW).
   Functions: `sv_retain`, `sv_release`, `sv_copy` (value-type deep/CoW copy), `sv_eq`.

2. **`Env`/`Scope`** (`runtime/env.c`) — runtime scope chain (MiniJS model). Intern pool
   maps identifier text → id; scopes map id → `SwiftValue` slot. `env_lookup` walks parents.

3. **`eval(node, env) -> Completion`** (`runtime/interp.c`) — the `switch (node->kind)`
   dispatcher. Returns a `Completion { kind, value }` so `return`/`break`/`continue`/
   `throw`/`fallthrough` unwind without `longjmp` (design doc §8).

4. **`Frame`** (`runtime/frame.c`) — per-call: decl, scope, `self`, parent (call stack +
   roots), defer list. Calls recurse the C stack; deep recursion → catchable overflow error.

**Acceptance for the spine:** run the design-doc sample (`add`/`for`/`print`) → prints `21`.

---

## 3. Milestones (R0–R6) with concrete deliverables & exit criteria

Each milestone = a slice of `feature-checklist.md`. "Exit criteria" are runnable fixtures.

### R0 — Spine + arithmetic (weeks 1–3)
**Scope:** Tier 0 (lexical/literals), Tier 1a (bindings, arithmetic, compound assign),
`print`, integer/double/bool/string values.
**Build:** the four spine pieces; `numeric.c` (Int/Double ops, overflow trap, `&+` wrap);
declare-hoist pass; `driver.c` + CLI.
**Exit:** fixtures for arithmetic, `let`/`var`, string literals, `print`, overflow trap,
wrapping ops all pass golden stdout.

### R1 — Functions & control flow (weeks 4–6)
**Scope:** Tier 1b (functions, labels, defaults, variadics, function types as values),
Tier 1c (if/guard/while/repeat/for-in/switch/break/continue/fallthrough/labeled),
ranges, tuples, ternary, string interpolation (via `msf_parse_expression`).
**Build:** `call.c` (free functions, labels, defaults, variadics); `pattern.c` v1
(value/range/tuple/wildcard); range + array iterators; `assert`/`precondition`/`fatalError`.
**Exit:** recursion (factorial/fib), `switch` over Ints/ranges/tuples, labeled break,
interpolated strings, variadic `sum(_:)`, basic `Array` literal + `for-in`.

### R2 — Value types (weeks 7–10)
**Scope:** Tier 2 entirely (structs, enums incl. associated & raw values, optionals,
subscripts, properties incl. computed/observers/lazy/static), `inout`, value semantics.
**Build:** struct/enum construction + memberwise init; `mutating`/`inout` via `lvalue.c`
(true aliasing, **not** copy); optional model + `if let`/`guard let`/`?.`/`!`/`??`;
computed props (get/set), `willSet`/`didSet`, `lazy`; pattern.c v2 (enum + optional patterns).
**Exit:** `Point`/`Shape` fixtures; mutating method mutates caller's value; enum with
associated values pattern-matched; optional chaining; copy-on-assign verified (mutating a
copy leaves original intact).

### R3 — Reference types & memory (weeks 11–15)
**Scope:** Tier 3 (classes, ARC, inheritance, override, dynamic dispatch, super,
designated/convenience/required/failable init, 2-phase init, weak/unowned, identity,
casting) + Tier 3a (closures, capture lists, @escaping, @autoclosure).
**Build:** `SVObject` ARC header (strong/unowned counts + weak side table); vtables for
dynamic dispatch; 2-phase init protocol; `deinit` at refcount 0; closures with open/closed
upvalues + capture lists honoring `MOD_CAPTURE_WEAK/UNOWNED`; `is`/`as?`/`as!` over class hierarchy.
**Exit:** inheritance + override fixtures; `deinit` fires deterministically; `weak`
reference zeroes on dealloc; retain-cycle leaks (correct — Swift does too); escaping closure
captures and mutates outer var; downcast fixtures.

### R4 — Protocols, generics, extensions (weeks 16–22)
**Scope:** Tier 4 (protocols + witness tables, associated types, composition, default impls,
existential `any`, conditional conformance, synthesized Equatable/Hashable/Comparable;
generics + constraints + where + monomorphization; extensions), Tier 6 core (opaque `some`,
`any`, metatypes, `type(of:)`), Tier 9a/9b (access control, custom operators/precedence),
key Tier 10c protocols (Sequence/Collection/Equatable/Hashable + map/filter/reduce).
**Build:** `conformance.c` (build witness tables from msf `ConformanceTable`/`AssocTypeTable`);
`generics.c` (monomorphize via `type_substitute`, or carry substitution in frame); protocol
existential boxes; operator/precedencegroup resolution; Equatable/Hashable synthesis.
**Exit:** generic `Stack<T>`; protocol with default impl + associated type; `Sequence`
conformance drives `for-in`; conditional conformance; custom operator with precedence;
`[T].map/filter/reduce`; existential `any Shape` array.

### R5 — Errors, resources, modules, stdlib depth (weeks 23–28)
**Scope:** Tier 5 (Error/throws/throw/do-catch/try/try?/try!/rethrows/typed-throws/defer),
property wrappers, `Codable` synthesis, `@main`, `#if` conditional compilation,
`#file`/`#line`/`#function`, multi-file modules, deeper stdlib (`Result`, `Set`, `Substring`).
**Build:** error channel on `Completion` (`CO_THROW`) + `do`/`catch` pattern matching +
`defer` LIFO; `@propertyWrapper` desugaring + projected values; `codable.c`; `#if`
evaluation pass (likely pre-sema); `MSFModule` multi-file driving.
**Exit:** throwing function caught by typed `catch`; `defer` runs on all exit paths;
property wrapper fixture; `Codable` round-trip (encode→decode); `@main`; multi-file program.

### R6 — Bytecode VM (perf; optional, only if throughput matters) (weeks 29–36)
**Scope:** Tier 6+ advanced (key paths, ownership `consume`/`borrow`, `~Copyable`),
plus the VM itself. Prerequisite for concurrency & macros.
**Build (the MiniJS/QuickJS recipe):** AST→IR (basic blocks, virtual regs) →
liveness + graph-coloring regalloc → fixed-width register bytecode (backpatch jumps) →
`switch`-then-computed-goto VM loop; move exceptions to stack-marker scheme (QuickJS §6);
save/restore frame state to enable suspension.
**Exit:** all R0–R5 fixtures pass on the VM path; measurable speedup vs tree-walker;
generator/suspension primitive works (enables R6+).

### R6+ — Concurrency & macros (weeks 37+)
**Scope:** Tier 7 (async/await/Task/TaskGroup/actor/@MainActor/Sendable/AsyncSequence),
Tier 8 (freestanding + attached macros, `@resultBuilder`), remaining Tier 6+ items.
**Build:** cooperative scheduler + executors over the VM's suspendable frames (async =
"save registers, remember pc" — MiniJS §6); actor serial executors; macro-expansion engine
over the AST (pre-interpretation transform); result-builder transform.
**Exit:** `async`/`await` round-trip; actor serializes mutations; `withTaskGroup` parallel
fixture; a `#freestanding` macro and an `@attached` macro expand and run; a result-builder DSL.

---

## 4. Testing strategy (built from day one)

**1. Golden fixture tests (primary).** `tests/fixtures/*.swift` + `*.expected`; harness
runs each through `swiftrun`, diffs stdout. Every feature lands with ≥1 fixture. This is
the regression spine.

**2. Differential testing against real Swift.** Where a `swiftc`/Swift toolchain is
available, run the *same* fixture through real Swift and diff — the ground truth for
"every exact Swift feature." Mark fixtures with the minimum Swift version.

**3. C unit tests** (`tests/unit/`) for tricky internals: ARC retain/release counts,
CoW uniqueness, value-copy semantics, pattern matching, `type_substitute` integration.

**4. msf's own corpus.** Reuse msf's `tests/swift-fixtures/` and Swift corpus — anything
that parses cleanly is a candidate runtime fixture.

**5. Sanitizers + stress.** ASan build in CI; an ARC stress mode (release on every alloc)
to flush missed retains/releases, analogous to MiniJS's `MINIJS_GC_STRESS`.

**Definition of done per checklist item:** parses (FE), runs, fixture passes, and (where
possible) matches real `swiftc` output.

---

## 5. Risk register & mitigations

| Risk | Impact | Mitigation |
|---|---|---|
| **Stdlib is unbounded** (Tier 10 behaviour) | Highest | Scope a deliberate subset; native C builtins; expand by demand-driven fixtures |
| **Value semantics / CoW bugs** | High | Unit tests on copy + `isKnownUniquelyReferenced`; ARC stress mode |
| **2-phase class init correctness** | High | Follow Swift's init rules precisely; dedicated fixtures per rule |
| **Frontend gaps** (typed throws, packs, macros, `#if`) | Med | Track in checklist's "FE gaps"; fix in msf *before* the dependent runtime work |
| **Concurrency needs suspension** | High | Gate behind R6 VM; don't attempt on tree-walker (can't suspend C stack) |
| **Macros need SwiftSyntax-equivalent** | High | Defer to R6+; build a focused AST-expansion engine, not full plugin host |
| **Monomorphization blow-up** | Med | Start with frame-carried substitution (lazy) before specializing bodies |
| **msf AST/ABI drift** | Low | Pin msf commit; the Backend ABI (§9–16) is documented/stable |

---

## 6. Immediate next actions (first two weeks)

1. **Scaffold `swiftrun/`** — Makefile linking `libMiniSwiftFrontend.a`, `swiftrun.h`,
   `main.c`, `driver.c`, `backend_stubs.c` (NULL stubs). Get `msf_analyze` → error report
   working end to end (no interpretation yet).
2. **Implement the spine** — `value.c` (scalars + retain/release no-ops for now),
   `env.c` (intern + scope chain), `frame.c` (Completion), `interp.c` skeleton dispatch.
3. **R0 vertical slice** — literals → arithmetic → `print`; make the design-doc sample
   print `21`. Stand up the fixture harness with that as fixture #1.
4. **Wire CI** — debug + ASan builds, run fixtures + unit tests on every commit.
5. **Open the tracking board** — turn `feature-checklist.md` rows into issues, grouped by
   milestone R0–R6; the checklist is the single source of truth for progress.

---

## 7. Definition of "complete" (project north star)

Every row in `docs/swift-runtime/feature-checklist.md` is `[x]`: parsed by msf,
implemented in `swiftrun`, covered by a passing golden fixture, and — wherever a Swift
toolchain is available — **matching real `swiftc` output**. Tiers 0–5 deliver a runnable,
faithful Swift; Tier 6 makes it fast; Tiers 7–8 complete concurrency and macros for full
Swift 6.3 coverage.
