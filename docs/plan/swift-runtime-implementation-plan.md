# Swift Runtime — Implementation Plan

## Goal

Build a **lightweight Swift runtime**. It:

1. **Parses Swift using `msf`** (a C library) to produce a typed AST — we do **not**
   write a Swift parser/typechecker; msf does lexing → parsing → 3-pass sema.
2. **Implements the runtime in Rust**, consuming that AST via **FFI**, covering:
   - **(a) Language features** — the evaluator/semantics (values, control flow, types,
     generics, ARC, closures, errors, …).
   - **(b) Standard library** — the behaviour of `Int`/`String`/`Array`/`Dictionary`/
     `Optional`/protocols/etc. (msf gives type *shapes*, not behaviour).

**Why Rust:** its ownership model maps onto Swift's semantics unusually well —
`Rc`/`Arc` ≈ ARC, `Rc::make_mut` ≈ copy-on-write value semantics,
`rc::Weak` ≈ `weak`, native `checked_*`/`wrapping_*` ints ≈ Swift overflow/`&+`,
UTF-8 `String` ≈ Swift 5+ String backing. We get memory safety for free in the
evaluator and confine `unsafe` to the thin FFI layer that walks msf's AST.

**Guiding principle (from the VM studies):** *Get it running first, then make it fast.*
A typed tree-walking interpreter (R0–R5) before any bytecode VM (R6).

**Reads with this plan:**
- `docs/research/2026-06-24-implementing-a-swift-runtime-on-msf-ast.md` — architecture & rationale
- `docs/swift-runtime/feature-checklist.md` — full Swift 6.3 feature surface (tiers, FE/RT/phase)
- `docs/research/2026-06-24-msf-swift-frontend.md` — what the frontend gives us
- `docs/research/2026-06-24-minijs-...` & `...quickjs-ng...` — the proven VM recipe

---

## 1. Architecture: Rust over a C frontend

```
 Swift source
     │
     ▼
┌──────────────┐   FFI    ┌───────────────────────────────────────────┐
│ msf (C lib)  │ ───────▶ │ Rust runtime (quick-swift)                │
│ lex→parse→   │  raw     │  msf-sys → msf (safe) → core → std → cli    │
│ sema (typed  │  ptrs    │  language features + standard library      │
│ AST)         │          │  ARC=Rc/Arc · CoW=make_mut · safe eval      │
└──────────────┘          └───────────────────────────────────────────┘
```

`msf` stays a black box behind a typed AST. All *behaviour* lives in Rust.

### 1.1 Cargo workspace layout

```
quick-swift/                       # cargo workspace
├── Cargo.toml                     # [workspace] members
├── crates/
│   ├── msf-sys/                   # RAW FFI — unsafe, generated
│   │   ├── build.rs               # build msf .a + bindgen + link + module_stub_find
│   │   ├── wrapper.h              # #include <msf.h> for bindgen
│   │   ├── stub.c                 # provides module_stub_find (the backend seam)
│   │   └── src/lib.rs             # bindgen include!(); repr(C) ASTNode/TypeInfo/Token
│   ├── msf/                       # SAFE wrapper — zero unsafe leaks upward
│   │   └── src/lib.rs             # Analysis, Node, NodeKind(enum), Type, iterators,
│   │                              #   diagnostics; owns MSFResult lifetime (Drop)
│   ├── quick-swift-core/          # LANGUAGE FEATURES (the evaluator)
│   │   └── src/
│   │       ├── value.rs           # SwiftValue enum, ARC (Rc), CoW
│   │       ├── env.rs             # scope chain + interner
│   │       ├── interp.rs          # eval(node,env)->Completion dispatcher
│   │       ├── frame.rs           # call frames, defer, completion signals
│   │       ├── decl.rs            # declare-hoist pass (≈ msf sema pass 1)
│   │       ├── call.rs            # fn/method/init/enum-case/closure dispatch
│   │       ├── pattern.rs         # switch / if-case matching + binding
│   │       ├── lvalue.rs          # assignment lvalues + inout aliasing
│   │       ├── cast.rs            # is / as? / as! over types & hierarchy
│   │       ├── conformance.rs     # witness tables from msf ConformanceTable
│   │       └── generics.rs        # monomorphization via msf type_substitute
│   ├── quick-swift-std/           # STANDARD LIBRARY (native Rust builtins)
│   │   └── src/
│   │       ├── numeric.rs string.rs collection.rs optional.rs
│   │       ├── sequence.rs        # Sequence/Collection + map/filter/reduce
│   │       └── codable.rs         # (R5) Encodable/Decodable via serde_json
│   └── quick-swift-cli/           # binary: quick-swift run file.swift
└── tests/fixtures/                # *.swift + *.expected golden tests
```

### 1.2 The FFI boundary (msf-sys + msf)

**Build (`msf-sys/build.rs`):**
1. Compile msf into a static lib — either invoke its `Makefile` (`make release`) or
   compile its `src/**.c` + `generated/` via the **`cc`** crate (preferred: no make
   dependency, lets cargo cache). Add `-Iinclude -Igenerated -msimd128`-equivalent flags.
2. Compile `stub.c` (provides `module_stub_find`) into the same lib — this is the **one**
   backend symbol msf needs (verified: linking succeeds with only this stub; all other
   externals like `sema_import_module`, `ast_arena_*` are defined inside the archive).
3. Generate Rust bindings with **`bindgen`** from `wrapper.h` (`#include <msf.h>`).
   bindgen renders `ASTNode`/`TypeInfo`'s anonymous unions as Rust `union`s (unsafe access)
   and the `ASTNodeKind`/`TypeKind`/`OpKind`/`Keyword` enums as constants.
4. `println!("cargo:rustc-link-lib=static=MiniSwiftFrontend");` + search path.

**Safe wrapper (`msf` crate):** the *only* place that dereferences msf pointers. Exposes:
- `Analysis` — owns `*mut MSFResult`, `impl Drop { msf_result_free }`; `Send`-unsafe so it
  stays on one thread; keeps source alive (AST tokens point into it).
- `Node<'a>` — borrows `&'a Analysis`; `.kind() -> NodeKind` (a real Rust `enum` we map
  from `ASTNodeKind`), `.children() -> impl Iterator`, `.ty() -> Option<Type>`,
  `.int()/.float()/.bool()/.text()/.op()` typed payload accessors, `.modifiers()`.
- `Type<'a>` — `.kind() -> TypeKind`, `.inner()`, `.func()`, `.generic_args()`, etc.
- `Diagnostics` — `errors()` with messages + byte ranges.
- Read-only views of msf's `ConformanceTable` / `AssocTypeTable` for the conformance layer.

Everything above `msf-sys` is **safe Rust**. `unsafe` is confined to `msf-sys` + the
accessor methods in `msf`, each justified by the invariant "the AST is immutable and
lives as long as `Analysis`."

> Lifetime rule: `Analysis` owns the arena-allocated AST/`TypeInfo`. Never free nodes
> mid-run; `Node`/`Type` borrow `Analysis` so the borrow checker enforces this for us.

---

## 2. The core engine in Rust (built once in R0)

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
| Value-type copy on assign | `Clone` of an `Rc` + `Rc::make_mut` on mutation → **automatic CoW** |
| `mutating` / `inout` aliasing | `&mut` lvalue path / `Rc::make_mut`; uniqueness re-checked |
| `class` reference semantics + ARC | `Rc<RefCell<Object>>`; retain = clone, release = drop |
| Deterministic `deinit` | Rust `Drop` fires at `Rc` strong-count 0 (deterministic) |
| `weak` (zeroing) | `rc::Weak`; `.upgrade()` yields `None` after dealloc → Optional |
| `unowned` | `Weak` + trap (panic-as-Swift-fatalError) on `None` upgrade |
| **No cycle collection** | Rust `Rc` leaks cycles — **faithful**: Swift doesn't collect either |
| `isKnownUniquelyReferenced` | `Rc::strong_count(&rc) == 1` |
| Overflow trap `+` / wrapping `&+` | `i64::checked_add` (trap) / `wrapping_add` |
| `Int` width truncation | explicit per-`IntWidth` masking |

This is why Rust is the right host: **most of Swift's hard memory semantics are Rust's
native idioms**, not features we re-implement.

### 2.2 Spine pieces
- **`Interner` + `Scope` chain** (`env.rs`) — msf leaves identifiers unresolved
  (`unresolved_decl_ref_expr`), so we own lexical lookup (MiniJS `Env` model). Intern to
  `u32`, compare by id.
- **`eval(node, env) -> Completion`** (`interp.rs`) — `match node.kind()` dispatcher.
  `Completion` enum (`Normal/Return/Break/Continue/Throw/Fallthrough(value)`) unwinds
  without panics (design doc §8) — no `setjmp`/`longjmp` analogue needed.
- **`Frame`** (`frame.rs`) — decl, scope, `self`, parent, defer stack. Calls recurse the
  Rust call stack; deep recursion → a catchable error (guard with an explicit depth
  counter to avoid a real stack overflow).

**Spine acceptance:** run the design-doc sample (`add`/`for`/`print`) → prints `21`.

---

## 3. External dependencies & **Swift compatibility check**

Each crate below is chosen to match an exact Swift behaviour. ✅ good fit · ⚠️ caveat to manage.

### 3.1 Build / FFI
| Dependency | Use | Swift-compat note |
|---|---|---|
| **bindgen** | generate `msf.h` bindings | ✅ handles msf's unions/enums; pin msf commit |
| **cc** | compile msf `.c` + `stub.c` | ✅ build-time only |

### 3.2 Language-feature dependencies
| Need | Crate | Swift-compat note |
|---|---|---|
| Integer overflow/wrap | **std** (`checked_*`/`wrapping_*`) | ✅ exact: `+` traps, `&+` wraps |
| Int widths I8…U64 | **std** (`i8..i64`,`u8..u64`) | ✅ 1:1 |
| Float/Double math | **std** `f32`/`f64` | ✅ IEEE-754 like Swift |
| Float→String shortest round-trip | **ryu** | ⚠️ Swift uses SwiftDtoa; `ryu` is shortest-round-trip but **formatting differs** (exponent style). Wrap in a SwiftDtoa-mimicking formatter |
| ARC / weak / CoW | **std** `Rc`/`Arc`/`Weak`/`make_mut` | ✅ native fit (see §2.1) |
| Hashing (`Hashable`/Dictionary) | **std** `DefaultHasher` (SipHash-1-3) | ✅ Swift also SipHash-1-3 (seed differs; ordering non-deterministic in both — fine) |

### 3.3 Standard-library dependencies
| Swift type/behaviour | Crate | Swift-compat note |
|---|---|---|
| `String` UTF-8 backing | **std** `String` | ✅ Swift 5+ is UTF-8 |
| `Character` = extended grapheme cluster | **unicode-segmentation** (UAX #29) | ✅ matches Swift `Character`; ⚠️ pin Unicode version to Swift's |
| `String` ==/hash by canonical equivalence | **unicode-normalization** (NFC) | ✅ Swift compares canonical-equivalent; msf already vendors NFC for lexing |
| Case mapping (`uppercased`/`lowercased`) | **std** + **unicode-case-mapping** | ⚠️ std covers most; full Unicode special-casing may need the crate |
| `Array`/`ContiguousArray` (CoW) | **std** `Vec` + `Rc::make_mut` | ✅ value semantics + CoW exact |
| `Dictionary` | **std** `HashMap` | ✅ (unordered, like Swift); custom `Hashable` driven by interpreter |
| `Set` | **std** `HashSet` | ✅ |
| `Codable` / `JSONEncoder`/`Decoder` | **serde_json** | ⚠️ synthesis done in interpreter; serde_json is the JSON *format* layer. Match key order/float/date/key-strategies to JSONEncoder |
| Regex literals (`/.../`) | **fancy-regex** (backrefs/lookaround) or **regex** | ⚠️ Swift Regex dialect (ICU/Oniguruma-ish + DSL) ≠ Rust syntax exactly. `regex` lacks backrefs; `fancy-regex` adds them. **Partial compat — document supported subset** |
| `Decimal` (Foundation) | **rust_decimal** (if needed) | ⚠️ NSDecimal is base-10/38-digit — different model. **Scope out of MVP** |
| `Date`/`Calendar` (Foundation) | **time** or **chrono** (if needed) | ⚠️ Foundation date math is large. **Scope out of MVP** |
| Async executor (R6+) | **custom cooperative executor** (not tokio) | ⚠️ Swift structured concurrency (actors, child-task trees, cooperative cancellation) differs from tokio's work-stealing. Build a small Swift-faithful executor over the VM's suspendable frames |

**Dependency policy:** prefer **std** (it's the most Swift-aligned for memory/numerics).
Add a crate only when std can't match a Swift behaviour, and **record the exact
compatibility gap** in a per-feature note + fixture. Keep the dependency set lean
("lightweight runtime").

### 3.4 Known compatibility gaps to track explicitly
- **Float string formatting** — mirror SwiftDtoa output, not raw `ryu`/`std`.
- **Regex** — Swift Regex is a superset/different dialect; ship a documented subset.
- **Foundation** (`Decimal`, `Date`, `URL`, `Data` semantics) — out of MVP; revisit by demand.
- **Unicode version** — pin `unicode-segmentation`/`-normalization` to the Unicode
  version of the Swift release we target (grapheme breaking changes between versions).
- **Concurrency scheduling** — observable ordering may differ; aim for semantic, not
  scheduler-identical, behaviour.

---

## 4. Milestones (R0–R6+) — Rust deliverables & exit criteria

Each milestone = a slice of `feature-checklist.md`. Exit criteria are runnable fixtures.

### R0 — FFI bring-up + spine + arithmetic (weeks 1–4)
**Scope:** the whole FFI layer + Tier 0/1a + `print`.
**Build:** `msf-sys` (build.rs + bindgen + `stub.c`), `msf` safe wrapper with
`Analysis`/`Node`/`NodeKind`; `SwiftValue` + `Rc` plumbing; `env`/`interp`/`frame` spine;
`numeric.rs` (widths, overflow trap, `&+` wrap); CLI.
**Exit:** `quick-swift run sample.swift` prints `21`; arithmetic/`let`/`var`/string/overflow
fixtures pass. **FFI is fully working and safe-wrapped by end of R0.**

### R1 — Functions & control flow (weeks 5–7)
**Scope:** Tier 1b/1c, ranges, tuples, ternary, string interpolation (`msf_parse_expression`).
**Build:** `call.rs` (labels/defaults/variadics), `pattern.rs` v1, range/array iterators, asserts.
**Exit:** recursion, `switch` (Int/range/tuple), labeled break, interpolation, variadics.

### R2 — Value types (weeks 8–11)
**Scope:** Tier 2 (structs, enums incl. associated/raw, optionals, subscripts, properties),
`inout`, value semantics.
**Build:** struct/enum construction + memberwise init; `mutating`/`inout` via `lvalue.rs`
(`Rc::make_mut` + true aliasing); optionals + `if let`/`?.`/`!`/`??`; computed/observed/lazy
props; `pattern.rs` v2 (enum/optional patterns).
**Exit:** CoW verified (mutating a copy leaves original intact); associated-value matching;
mutating method updates caller's value.

### R3 — Reference types & memory (weeks 12–16)
**Scope:** Tier 3 + 3a (classes, ARC, inheritance, dynamic dispatch, 2-phase init,
weak/unowned, casting, closures + capture lists, @escaping/@autoclosure).
**Build:** `Object` over `Rc<RefCell>`; vtables; `Drop`-driven `deinit`; `Weak` for
`weak`; closures capturing `Rc` cells; `cast.rs`.
**Exit:** `deinit` fires deterministically; `weak` zeroes; downcasts; escaping-closure capture.

### R4 — Protocols, generics, extensions (weeks 17–24)
**Scope:** Tier 4 + Tier 6 core (opaque/any/metatype/`type(of:)`) + Tier 9a/9b + key Tier 10c.
**Build:** `conformance.rs` (witness tables from msf `ConformanceTable`/`AssocTypeTable`);
`generics.rs` (monomorphize via `type_substitute`, or frame-carried substitution);
existential boxes; operator/precedence resolution; Equatable/Hashable/Comparable synthesis;
`Sequence`/`Collection` + `map`/`filter`/`reduce`.
**Exit:** generic `Stack<T>`; protocol default impls + associated types; `Sequence`-driven
`for-in`; conditional conformance; custom operators; existential `any P` arrays.

### R5 — Errors, resources, modules, stdlib depth (weeks 25–30)
**Scope:** Tier 5 (throws/try/do-catch/rethrows/typed-throws/defer), property wrappers,
`Codable` (serde_json), `@main`, `#if`, `#file`/`#line`, multi-file modules, `Result`/`Set`/`Substring`.
**Build:** `Throw` on `Completion` + `do`/`catch` matching + `defer` LIFO; property-wrapper
desugaring; `codable.rs`; `#if` evaluation pass; `MSFModule` multi-file driving.
**Exit:** typed `catch`; `defer` on all paths; property-wrapper fixture; `Codable` round-trip; `@main`.

### R6 — Bytecode VM (perf; optional) (weeks 31–38)
**Scope:** Tier 6+ (key paths, ownership `consume`/`borrow`, `~Copyable`) + the VM.
**Build (MiniJS/QuickJS recipe in Rust):** AST→IR (basic blocks, virtual regs) → liveness +
graph-coloring regalloc → register bytecode (backpatch jumps) → `match`-dispatch VM loop;
stack-marker exceptions; **suspendable frames** (enables R6+).
**Exit:** all R0–R5 fixtures pass on VM path; speedup vs tree-walker; suspension primitive works.

### R6+ — Concurrency & macros (weeks 39+)
**Scope:** Tier 7 (async/await/Task/actor/@MainActor/Sendable/AsyncSequence), Tier 8 (macros,
`@resultBuilder`).
**Build:** custom cooperative executor over suspendable frames (async = "save registers,
remember pc"); actor serial executors (`Arc` + isolation); macro-expansion engine over the
AST (pre-interpretation transform); result-builder transform.
**Exit:** `async`/`await` round-trip; actor serialization; `withTaskGroup`; freestanding +
attached macro expand and run; a result-builder DSL.

---

## 5. Testing strategy (from day one)

1. **Golden fixtures (primary)** — `tests/fixtures/*.swift` + `*.expected`; a Rust
   `#[test]` harness (or `trybuild`-style) runs each via the CLI and diffs stdout. Every
   feature lands with ≥1 fixture.
2. **Differential testing vs real Swift** — where a `swiftc` toolchain exists, run the
   same fixture through real Swift and diff. This is the ground truth for "every exact
   Swift feature." Tag fixtures with min Swift version.
3. **Rust unit tests** — per crate: ARC counts (`Rc::strong_count`), CoW uniqueness,
   value-copy semantics, pattern matching, `type_substitute` integration, FFI accessors.
4. **msf corpus reuse** — msf's `tests/swift-fixtures/` + Swift corpus feed runtime fixtures.
5. **Sanitizers/stress** — `cargo +nightly miri` on the safe crates for UB; an ARC-stress
   mode; ASan on the C side (`msf-sys`) for the FFI build.

**Definition of done per checklist item:** parses (msf), runs, fixture passes, and (where
possible) matches real `swiftc` output.

---

## 6. Risk register & mitigations

| Risk | Impact | Mitigation |
|---|---|---|
| **FFI unsafety / msf pointer lifetimes** | High | Confine `unsafe` to `msf-sys`; borrow-checked `Node<'a>` ties AST to `Analysis`; Miri on safe crates |
| **bindgen on msf unions/anon structs** | Med | Verify generated bindings early (R0 week 1); add manual `repr(C)` shims if needed; pin msf commit |
| **Stdlib is unbounded** | Highest | Deliberate subset; std-first; expand by demand-driven fixtures |
| **Float/regex/Foundation compat gaps** | Med | Documented per §3.4; SwiftDtoa-mimicking float formatter; regex subset; Foundation out of MVP |
| **Value semantics / CoW correctness** | High | `Rc::make_mut` + uniqueness unit tests + ARC stress |
| **2-phase class init** | High | Follow Swift init rules exactly; fixture per rule |
| **Concurrency needs suspension** | High | Gate behind R6 VM; custom Swift-faithful executor, not tokio |
| **Macros need SwiftSyntax-equivalent** | High | Defer to R6+; focused AST-expansion engine, not a plugin host |
| **msf frontend gaps** (typed throws, packs, `#if`, macros) | Med | Track in checklist "FE gaps"; fix in msf before dependent runtime work |

---

## 7. Immediate next actions (first two weeks)

1. **Scaffold the workspace** — `cargo new` workspace + 5 crates. `msf-sys/build.rs`:
   compile msf (via `cc` over `src/**.c` + `generated/`, or shell its Makefile) + `stub.c`
   (`module_stub_find`), run **bindgen**, link `static=MiniSwiftFrontend`.
2. **Prove the FFI** — from Rust, call `msf_analyze` on a string, read `msf_error_count`,
   walk `msf_root` children, print `ast_kind_name`. This is the riskiest unknown — do it first.
3. **Safe wrapper** — `msf` crate: `Analysis` (Drop), `Node`/`NodeKind`, child iterator,
   typed payload accessors, diagnostics. Zero `unsafe` above this crate.
4. **R0 vertical slice** — `SwiftValue` + spine; literals → arithmetic → `print`; make the
   design-doc sample print `21`; stand up the fixture harness with it as fixture #1.
5. **CI** — `cargo test` (debug + ASan FFI build) + Miri on safe crates; convert
   `feature-checklist.md` rows into issues grouped by milestone R0–R6.

---

## 8. Definition of "complete" (north star)

Every row in `docs/swift-runtime/feature-checklist.md` is `[x]`: parsed by msf,
implemented in the Rust runtime, covered by a passing golden fixture, and — wherever a
Swift toolchain is available — **matching real `swiftc` output**, with any intentional
compatibility gaps (§3.4) explicitly documented. Tiers 0–5 deliver a runnable, faithful
**lightweight Swift**; Tier 6 makes it fast; Tiers 7–8 complete concurrency and macros.
```
quick-swift: Swift source → [msf C lib: lex/parse/sema] → typed AST → [Rust runtime] → execution
             language features + standard library, ARC via Rc, CoW via make_mut, safe by construction
```
