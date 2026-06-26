# Swift Runtime — Implementation Plan

## Goal

Build a **lightweight Swift runtime**. It:

1. **Parses Swift using a pure-Rust frontend** (`qswift-lexer` → `qswift-ast` →
   `qswift-parser` → `qswift-sema` → `qswift-frontend::compat`) to produce a
   typed AST — no C compiler, no FFI, no `unsafe`.
2. **Implements the runtime in Rust**, consuming that AST through the stable
   `Analysis` / `Node` / `NodeKind` API, covering:
   - **(a) Language features** — the evaluator/semantics (values, control flow, types,
     generics, ARC, closures, errors, concurrency, …).
   - **(b) Standard library** — the behaviour of `Int`/`String`/`Array`/`Dictionary`/
     `Optional`/protocols/etc. (the frontend gives type *shapes*, not behaviour).

**Why Rust:** its ownership model maps onto Swift's semantics unusually well —
`Rc`/`Arc` ≈ ARC, `Rc::make_mut` ≈ copy-on-write value semantics,
`rc::Weak` ≈ `weak`, native `checked_*`/`wrapping_*` ints ≈ Swift overflow/`&+`,
UTF-8 `String` ≈ Swift 5+ String backing. We get memory safety for free, and there
is no `unsafe` anywhere in the stack.

**Guiding principle:** *Get it running first, then make it fast.*
A typed tree-walking interpreter (R0–R5) before any bytecode VM (R6).

**Reads with this plan:**
- `docs/swift-runtime/feature-checklist.md` — full Swift 6.3 feature surface (tiers, FE/RT/phase)
- `docs/adr/0002-bytecode-vm-vs-tree-walker.md` — go/no-go for the R6 VM
- `docs/adr/0004-suspendable-frames-via-stackful-coroutines.md` — suspension primitive
- `docs/adr/0005-cooperative-concurrency-executor.md` — concurrency executor design
- `docs/research/2026-06-24-implementing-a-swift-runtime-on-msf-ast.md` — original architecture & rationale

---

## 1. Architecture: pure-Rust pipeline

```
 Swift source
     │
     ▼
┌────────────────────────────────┐
│ pure-Rust frontend             │
│  qswift-lexer                   │
│    → qswift-ast                 │
│    → qswift-parser              │
│    → qswift-sema                │
│    → qswift-frontend      │
│      (compat lowerer → AST)    │
└────────────────┬───────────────┘
                 │ Analysis / Node / NodeKind
                 ▼
┌────────────────────────────────┐
│ Rust runtime (quick-swift)     │
│  core → std → cli              │
│  language features +           │
│  standard library              │
│  ARC=Rc · CoW=make_mut         │
└────────────────────────────────┘
```

The frontend is a black box behind the `Analysis` / `Node` / `NodeKind` API. All
*behaviour* lives in the runtime.

### 1.1 Cargo workspace layout

```
quick-swift/                       # cargo workspace
├── Cargo.toml                     # [workspace] members
├── crates/
│   ├── qswift-lexer/               # tokenizer for Swift source
│   ├── qswift-ast/                 # AST node definitions
│   ├── qswift-parser/              # recursive-descent parser
│   ├── qswift-sema/                # semantic analysis / type resolution
│   ├── qswift-frontend/      # compat lowerer: drives the pipeline,
│   │   └── src/                   # exposes Analysis/Node/NodeKind to the runtime
│   │       ├── compat.rs          # RuntimeAst arena + lowering from sema output
│   │       ├── kind.rs            # NodeKind enum (runtime-facing)
│   │       └── lib.rs             # Analysis::analyze / Node<'a> / AnalyzeError
│   ├── qswift-core/          # LANGUAGE FEATURES (the evaluator)
│   │   └── src/
│   │       ├── value.rs           # SwiftValue enum, ARC (Rc), CoW
│   │       ├── env.rs             # scope chain + interner
│   │       ├── interp.rs          # eval(node,env)->Completion dispatcher
│   │       ├── ops.rs             # operator dispatch
│   │       ├── suspend.rs         # stackful-coroutine suspension primitive (ADR-0004)
│   │       └── lib.rs
│   ├── qswift-std/           # STANDARD LIBRARY (native Rust builtins)
│   │   └── src/
│   │       └── lib.rs             # print, numeric, string, collection, optional, …
│   └── qswift-cli/           # binary: qswift run file.swift
│       └── tests/fixtures/        # *.swift + *.expected golden tests (53+)
```

### 1.2 The frontend API

`qswift-frontend` is `#![forbid(unsafe_code)]` and the sole runtime-facing
seam onto the Swift frontend. It exposes three types:

- **`Analysis`** — owns the typed AST in a `RuntimeAst` arena; `impl Drop` disposes
  it. `Analysis::analyze(source, filename) -> Result<Analysis, AnalyzeError>` drives
  the full pipeline.
- **`Node<'a>`** — a cheap cursor that borrows `&'a Analysis`. `.kind() -> NodeKind`,
  `.children() -> impl Iterator<Item=Node<'a>>`, `.text()`, `.line()`, `.col()`,
  and other payload accessors. Nodes can never outlive their `Analysis` — the
  borrow checker enforces this.
- **`NodeKind`** — a Rust `enum` covering every AST node the runtime needs to
  pattern-match on.

Everything above `qswift-frontend` is safe Rust with no FFI.

---

## 2. The core engine in Rust

### 2.1 `SwiftValue` — value model that leverages Rust ownership

```rust
pub enum SwiftValue {
    Void,
    Bool(bool),
    Int(i64, IntWidth),        // width: I8..I64/U8..U64 for trap/wrap correctness
    Double(f64), Float(f32),
    Str(SwiftString),          // UTF-8 + grapheme view (see std)
    Struct(Rc<StructData>),    // VALUE type: Rc + make_mut == copy-on-write
    Enum(Rc<EnumData>),        // tag + associated payload
    Class(Rc<RefCell<Object>>),// REFERENCE type: shared, ARC = Rc
    Closure(Rc<Closure>),
    Optional(Option<Box<SwiftValue>>),
    Array(Rc<Vec<SwiftValue>>),       // CoW via Rc::make_mut
    Dict(Rc<HashMap<HashableKey, SwiftValue>>),
    Set(Rc<HashSet<HashableKey>>),
    Metatype(TypeId),
}
```

**How Rust maps Swift semantics (the core insight):**

| Swift semantic | Rust mechanism |
|---|---|
| Value-type copy on assign | `Clone` of an `Rc` + `Rc::make_mut` on mutation → automatic CoW |
| `mutating` / `inout` aliasing | `&mut` lvalue path / `Rc::make_mut`; uniqueness re-checked |
| `class` reference semantics + ARC | `Rc<RefCell<Object>>`; retain = clone, release = drop |
| Deterministic `deinit` | Rust `Drop` fires at `Rc` strong-count 0 (deterministic) |
| `weak` (zeroing) | `rc::Weak`; `.upgrade()` yields `None` after dealloc → Optional |
| `unowned` | `Weak` + trap (panic-as-Swift-fatalError) on `None` upgrade |
| No cycle collection | Rust `Rc` leaks cycles — faithful: Swift doesn't collect either |
| `isKnownUniquelyReferenced` | `Rc::strong_count(&rc) == 1` |
| Overflow trap `+` / wrapping `&+` | `i64::checked_add` (trap) / `wrapping_add` |
| `Int` width truncation | explicit per-`IntWidth` masking |

### 2.2 Spine pieces
- **`Interner` + `Scope` chain** (`env.rs`) — owns lexical lookup (identifiers may be
  unresolved in the AST). Interns to `u32`, compare by id.
- **`eval(node, env) -> Completion`** (`interp.rs`) — `match node.kind()` dispatcher.
  `Completion` enum (`Normal/Return/Break/Continue/Throw/Fallthrough(value)`) unwinds
  without panics — no `setjmp`/`longjmp` analogue needed.
- **Suspension** (`suspend.rs`) — stackful-coroutine primitive via `corosensei`
  (ADR-0004). Each `Task` / `async let` child / `withTaskGroup` child runs on its own
  native stack; the tree-walker suspends at `await` and hands control back to the
  cooperative executor. Chosen over CPS or state-machine transforms because it requires
  no changes to the recursive `eval` structure.

### 2.3 Concurrency executor (ADR-0005)
A **custom single-threaded cooperative executor** — not tokio — models Swift's
structured concurrency:

- Tasks are `corosensei` coroutines. The scheduler loop is the only place that
  resumes them; coroutines suspend back to the loop with `Await(id)` / `Yield`.
- `await` on an already-complete value is a no-op. Suspension only happens when
  awaiting a task handle that has not finished.
- Single thread → actors serialize for free; no data races by construction.
- Cancellation is a cooperative `isCancelled` flag propagated to structured children.
- **Fidelity boundary:** results and structure are faithful; interleaving order may
  differ from Apple's multi-threaded executor (see ADR-0005 for the explicit boundary).

---

## 3. External dependencies & Swift compatibility

### 3.1 Language-feature dependencies
| Need | Crate | Swift-compat note |
|---|---|---|
| Integer overflow/wrap | **std** (`checked_*`/`wrapping_*`) | ✅ exact: `+` traps, `&+` wraps |
| Int widths I8…U64 | **std** (`i8..i64`,`u8..u64`) | ✅ 1:1 |
| Float/Double math | **std** `f32`/`f64` | ✅ IEEE-754 like Swift |
| Float→String shortest round-trip | **ryu** | ⚠️ Swift uses SwiftDtoa; `ryu` is shortest-round-trip but formatting may differ. Wrap in a SwiftDtoa-mimicking formatter |
| ARC / weak / CoW | **std** `Rc`/`Weak`/`make_mut` | ✅ native fit |
| Hashing (`Hashable`/Dictionary) | **std** `DefaultHasher` (SipHash-1-3) | ✅ Swift also SipHash-1-3 (seed differs; non-deterministic ordering in both — fine) |
| Suspendable tasks | **corosensei** | ✅ stackful coroutines; one native stack per live task |

### 3.2 Standard-library dependencies
| Swift type/behaviour | Crate | Swift-compat note |
|---|---|---|
| `String` UTF-8 backing | **std** `String` | ✅ Swift 5+ is UTF-8 |
| `Character` = extended grapheme cluster | **unicode-segmentation** (UAX #29) | ✅ matches Swift `Character`; ⚠️ pin Unicode version to target Swift release |
| `String` ==/hash by canonical equivalence | **unicode-normalization** (NFC) | ✅ Swift compares canonical-equivalent |
| Case mapping (`uppercased`/`lowercased`) | **std** + **unicode-case-mapping** | ⚠️ std covers most; full Unicode special-casing may need the crate |
| `Array`/`ContiguousArray` (CoW) | **std** `Vec` + `Rc::make_mut` | ✅ value semantics + CoW exact |
| `Dictionary` | **std** `HashMap` | ✅ unordered, like Swift |
| `Set` | **std** `HashSet` | ✅ |
| `Codable` / `JSONEncoder`/`Decoder` | **serde_json** | ⚠️ synthesis done in interpreter; serde_json is the JSON format layer. Match key order/float/date/key-strategies to JSONEncoder |
| Regex literals (`/.../`) | **fancy-regex** or **regex** | ⚠️ Swift Regex dialect ≠ Rust syntax; `fancy-regex` adds backrefs. **Partial compat — document supported subset** |
| `Decimal` (Foundation) | **rust_decimal** (if needed) | ⚠️ NSDecimal is base-10/38-digit. **Scope out of MVP** |
| `Date`/`Calendar` (Foundation) | **time** or **chrono** (if needed) | ⚠️ Foundation date math is large. **Scope out of MVP** |

**Dependency policy:** prefer **std**. Add a crate only when std can't match a Swift
behaviour, and **record the exact compatibility gap** in a per-feature note + fixture.

### 3.3 Known compatibility gaps to track explicitly
- **Float string formatting** — mirror SwiftDtoa output, not raw `ryu`/`std`.
- **Regex** — Swift Regex is a superset/different dialect; ship a documented subset.
- **Foundation** (`Decimal`, `Date`, `URL`, `Data`) — out of MVP; revisit by demand.
- **Unicode version** — pin `unicode-segmentation`/`-normalization` to the Unicode
  version of the Swift release we target.
- **Concurrency scheduling** — observable ordering may differ from Apple's executor
  (ADR-0005 fidelity boundary); aim for semantic, not scheduler-identical, behaviour.

---

## 4. Milestones (R0–R6+) — status & exit criteria

### ✅ R0 — Frontend bring-up + spine + arithmetic
**Scope:** pure-Rust frontend + Tier 0/1a + `print`.
**Status:** complete. `Analysis::analyze` drives the full pipeline; `SwiftValue` + `Rc`
plumbing; `env`/`interp`/`frame` spine; `numeric.rs` (widths, overflow trap, `&+` wrap); CLI.
**Exit verified:** `qswift run sample.swift` prints correct output; arithmetic / `let` /
`var` / string / overflow fixtures pass.

### ✅ R1 — Functions & control flow
**Scope:** Tier 1b/1c, ranges, tuples, ternary, string interpolation.
**Status:** complete. Argument labels/defaults/variadics; `call.rs`; `pattern.rs` v1;
range/array iterators; asserts.
**Exit verified:** recursion, `switch` (Int/range/tuple), labeled break, interpolation, variadics.

### ✅ R2 — Value types
**Scope:** Tier 2 (structs, enums incl. associated/raw, optionals, subscripts, properties),
`inout`, value semantics.
**Status:** complete. Struct/enum construction + memberwise init; `mutating`/`inout` via
`lvalue.rs` (`Rc::make_mut` + true aliasing); optionals + `if let`/`?.`/`!`/`??`;
computed/observed/lazy props; `pattern.rs` v2 (enum/optional patterns).
**Exit verified:** CoW verified; associated-value matching; mutating method updates caller's value.

### ✅ R3 — Reference types & memory
**Scope:** Tier 3 + 3a (classes, ARC, inheritance, dynamic dispatch, 2-phase init,
weak/unowned, casting, closures + capture lists, `@escaping`/`@autoclosure`).
**Status:** complete. `Object` over `Rc<RefCell>`; vtables; `Drop`-driven `deinit`; `Weak` for
`weak`; closures capturing `Rc` cells; `cast.rs`.
**Exit verified:** `deinit` fires deterministically; `weak` zeroes; downcasts; escaping-closure capture.

### ✅ R4 — Protocols, generics, extensions
**Scope:** Tier 4 + Tier 6 core (opaque/any/metatype/`type(of:)`) + Tier 9a/9b + key Tier 10c.
**Status:** complete. `conformance.rs` (witness tables); `generics.rs` (monomorphization /
frame-carried substitution); existential boxes; operator/precedence resolution;
Equatable/Hashable/Comparable synthesis; `Sequence`/`Collection` + `map`/`filter`/`reduce`.
**Exit verified:** generic `Stack<T>`; protocol default impls + associated types; `Sequence`-driven
`for-in`; conditional conformance; custom operators; existential `any P` arrays.

### ✅ R5 — Errors, resources, modules, stdlib depth
**Scope:** Tier 5 (throws/try/do-catch/rethrows/typed-throws/defer), property wrappers,
`Codable`, `@main`, `#if`, `#file`/`#line`, multi-file modules, `Result`/`Set`/`Substring`.
**Status:** complete. `Throw` on `Completion` + `do`/`catch` matching + `defer` LIFO;
property-wrapper desugaring; `codable.rs`; `#if` evaluation pass; multi-file driving.
**Exit verified:** typed `catch`; `defer` on all paths; property-wrapper fixture; `Codable` round-trip; `@main`.

### ✅ R6+ — Concurrency (Tier 7)
**Scope:** `async`/`await`, `async let`, `Task`/`Task.detached`, `withTaskGroup`,
`actor` isolation, `@MainActor`/global actors, `Sendable`, `AsyncSequence`/`for await`.
**Status:** substantially complete (issue #12). Cooperative single-threaded executor over
`corosensei` stackful coroutines (ADR-0004, ADR-0005). Suspension primitive lives in
`crates/qswift-core/src/suspend.rs`.
**Gaps:** preemptive ordering / `Task.yield` interleaving; `withCheckedContinuation`;
strict-concurrency diagnostics. Documented in ADR-0005 fidelity boundary.
**Exit verified:** `async`/`await` round-trip; actor serialization; `withTaskGroup`; custom `AsyncSequence`.

### 🔲 R6 — Bytecode VM (perf; go/no-go pending)
**Scope:** Throughput optimisation — AST → IR (basic blocks, virtual registers) → liveness +
graph-colouring regalloc → register bytecode → `match`-dispatch VM loop.
**Status:** not started. The suspension
primitive (#12) was decoupled from this milestone and implemented independently via
`corosensei`.
**Go criteria (all must hold):**
- Benchmark suite shows the tree-walker is a material bottleneck for realistic workloads.
- Expected speedup justifies maintaining two execution engines at feature parity.
- Team bandwidth can sustain the parity tax.

**Scope add-ons** (if VM is approved): key paths `\Root.path`, `consume`/`borrow`,
`~Copyable`/`~Escapable`.

### 🔲 R7 — Macros & metaprogramming (Tier 8)
**Scope:** freestanding macros `#macro`, attached macros `@Macro`, `@resultBuilder` DSL
transform, macro-expansion engine over the AST.
**Status:** not started. Heavy: real macros need a macro-expansion pass before evaluation.
**Prerequisite:** Tier 8 FE gaps (frontend parsing of macro declarations + `@attached` roles)
must be closed first.

---

## 5. Testing strategy

1. **Golden fixtures (primary)** — `crates/qswift-cli/tests/fixtures/*.swift` +
   `*.expected` (53+ pairs). A Rust `#[test]` harness runs each via the CLI and diffs
   stdout. Every feature lands with ≥1 fixture.
2. **Differential testing vs real Swift** — where a `swiftc` toolchain exists, run the
   same fixture through real Swift and diff. Tag fixtures with min Swift version. This is
   the ground truth for "exact Swift feature."
3. **Rust unit tests** — per crate: ARC counts (`Rc::strong_count`), CoW uniqueness,
   value-copy semantics, pattern matching, AST accessors.
4. **Sanitizers/stress** — `cargo +nightly miri` on all crates for UB; ARC-stress test
   mode to surface cycle / deinit ordering issues.

**Definition of done per checklist item:** parses (frontend), runs, fixture passes, and
(where possible) matches real `swiftc` output.

---

## 6. Risk register & mitigations

| Risk | Impact | Mitigation |
|---|---|---|
| **Stdlib is unbounded** | Highest | Deliberate subset; std-first; expand by demand-driven fixtures |
| **Float/regex/Foundation compat gaps** | Med | Documented in §3.3; SwiftDtoa-mimicking float formatter; regex subset; Foundation out of MVP |
| **Value semantics / CoW correctness** | High | `Rc::make_mut` + uniqueness unit tests + ARC stress |
| **2-phase class init** | High | Follow Swift init rules exactly; fixture per rule |
| **Concurrency scheduling fidelity** | Med | Documented ADR-0005 fidelity boundary; fixtures avoid asserting on interleaving order |
| **VM parity tax** (if go) | High | Full R0–R5 golden-fixture harness gates the VM; ship only after byte-identical stdout |
| **Macros need SwiftSyntax-equivalent** | High | Deferred to R7; focused AST-expansion engine (not a plugin host) |
| **Frontend gaps** (typed throws, packs, `#if`, macros) | Med | Track in checklist "FE gaps"; close in frontend before dependent runtime work |
| **Unicode version drift** | Low | Pin `unicode-segmentation`/`-normalization` to target Swift release; CI fixture catches regressions |

---

## 7. Definition of "complete" (north star)

Every row in `docs/swift-runtime/feature-checklist.md` is `[x]`: parsed by the
pure-Rust frontend, implemented in the Rust runtime, covered by a passing golden
fixture, and — wherever a Swift toolchain is available — **matching real `swiftc`
output**, with any intentional compatibility gaps (§3.3) explicitly documented.
Tiers 0–5 deliver a runnable, faithful **lightweight Swift**; Tier 7 adds
structured concurrency; Tier 6 (VM) makes it fast if benchmarks justify the cost;
Tier 8 completes macros.

```
quick-swift: Swift source
  → [pure-Rust frontend: lex/parse/sema]
  → typed AST (Analysis/Node/NodeKind)
  → [Rust runtime: language features + standard library]
  → execution
  ARC via Rc · CoW via make_mut · safe by construction · no unsafe
```
