---
date: 2026-06-24
title: "Implementing a Swift Runtime on top of msf's Typed AST"
status: design
tags: [research, design, swift, runtime, interpreter, vm, msf, backend]
references:
  - docs/research/2026-06-24-msf-swift-frontend.md
  - docs/research/2026-06-24-minijs-building-a-vm-for-javascript.md
  - docs/research/2026-06-24-quickjs-ng-vm-architecture.md
inputs_verified_against: ".checkout/msf @ 0f92a4a (libMiniSwiftFrontend.a built locally)"
---

# Implementing a Swift Runtime on top of msf's Typed AST

**Goal.** Take the **typed, immutable AST** that `msf` produces (lexer → parser →
3-pass sema) and build the *missing back third* of a Swift compiler: a thing that
**runs** the code. msf explicitly ships **no IR, no codegen, no runtime** — its
README says so, and the `msf.h` "Backend ABI" (§9–16) is the documented seam where a
backend attaches. This document is the concrete plan for that backend, grounded in
three prior studies: the **msf frontend** research (what we get for free), and the
**MiniJS** and **QuickJS-NG** VM studies (the proven recipe for building a runtime).

This is a *design*, validated against a locally-built `libMiniSwiftFrontend.a` and
real AST dumps — not yet an implementation.

---

## 0. What msf hands us (verified)

Built the lib (`make release`), linked `tests/stubs.c` (provides the
`module_stub_find` / `sema_import_module` seam the backend is expected to fill), and
dumped real ASTs. For:

```swift
func add(_ a: Int, _ b: Int) -> Int { return a + b * 2 }
var total = 0
for i in 0..<3 { total = total + add(i, 4) }
print(total)
```

msf gives (abridged JSON from `msf_dump_json`):

```
source_file
  func_decl: add
    parameter: a → type_ident: Int
    parameter: b → type_ident: Int
    type_ident: Int                      (return type)
    brace_stmt → return_stmt → binary_expr +
        unresolved_decl_ref_expr: a
        binary_expr * (b, integer_literal_expr 2)
  var_decl: total → integer_literal_expr 0
  for_each_stmt
    parameter: i
    binary_expr ..< (0, 3)
    brace_stmt → … assign_expr = (total, total + add(i,4))
  expr_stmt → call_expr (print, total)
```

Key facts this establishes about the **input contract**:

1. **It is a tree of `ASTNode`** (`msf.h:566`). Walk `first_child` / `next_sibling`;
   switch on `kind` (`ASTNodeKind`, ~140 kinds); read the kind-specific `data` union
   (`integer.ival`, `flt.fval`, `boolean.bval`, `binary.op_tok`, `var.*`, `func.name_tok`,
   `call.resolved_callee_decl`, …).
2. **Every node carries a resolved `TypeInfo *type`** after `msf_analyze()`
   (`msf.h:576`). Use `type_kind_of()` to switch (it canonicalises the builtin
   singletons). This is the single biggest gift versus MiniJS/QuickJS, which are
   untyped — **we know every expression's static type**, so we can monomorphise
   arithmetic, pick witness tables, and skip most runtime type juggling.
3. **Names arrive *unresolved* in the dump** (`unresolved_decl_ref_expr`,
   `unresolved_dot_expr`). Sema resolves *types*; it does **not** rewrite every
   identifier into a slot index. `call.resolved_callee_decl` is populated for calls,
   but in general **the runtime still owns lexical name→binding resolution** — exactly
   like MiniJS's `Env` scope chain. Plan for an environment/scope model, do not assume
   pre-resolved slots.
4. **Literals are pre-decoded**: `integer_literal_expr` carries `data.integer.ival`
   (int64), floats `data.flt.fval`, bools `data.boolean.bval`. No re-lexing needed.
5. **Operators**: `binary_expr` / `assign_expr` store the operator token; read via the
   token (`OpKind op_kind` on the `Token`, `msf.h:175`) — `OP_ADD_ASSIGN`, `OP_RANGE_EXCL`,
   `OP_NIL_COAL`, etc. are enumerated.

Richer constructs (verified from a second dump):

| Swift | msf AST shape |
|---|---|
| `struct Point { var x; func dist() }` | `struct_decl` → `brace_stmt` → `var_decl` / `func_decl` |
| `enum Shape { case rect(Int,Int) }` | `enum_decl` → `enum_case_decl` → `enum_element_decl: rect` → `parameter` |
| `.rect(2, 5)` | `call_expr` → `unresolved_dot_expr: .rect` + arg children |
| `[1,2,3].map { $0 * 2 }` | `call_expr` → (`unresolved_dot_expr: .map` over `array_expr`) + `closure_expr` |
| `var maybe: Int? = nil` | `var_decl` → `type_optional`→`type_ident: Int`, `nil_literal_expr` |
| `if let v = maybe { … }` | `if_stmt` → `optional_binding_cond: v` → `brace_stmt` |

So method calls are `call_expr(callee = dot_expr(receiver, member), args…)`; enum
construction looks identical to a call on a dot expression; closures are first-class
child nodes with `$0…` implicit params. **All the runtime-relevant structure is
present and typed.**

---

## 1. The strategy: tree-walker first, bytecode VM later

Both prior VM studies converge on the same build-order lesson:

> **MiniJS** shipped a tree-walking evaluator (M0–M6) *then* added a register
> bytecode VM. **QuickJS-NG**'s "pragmatic build order" is the same skeleton.
> *"Get it running first, then make it fast."*

For Swift on msf the case for starting with a **direct AST tree-walking interpreter**
is even stronger, because:

- The AST is already **typed** — we skip the hardest part of a dynamic VM (figuring
  out types at runtime). A tree-walker can read `node->type` and dispatch directly.
- msf's AST is **immutable and arena-owned** — safe to keep pointers into it and
  re-walk subtrees (loop bodies, closure bodies) without copying.
- It gets us to "runs a Swift program" in the shortest path, which is what unlocks
  everything else (the editor demo, test corpus, etc.).

**Phase plan** (mirrors MiniJS M0→M6 then VM):

| Phase | Deliverable |
|---|---|
| **R0** | Value model + Env + tree-walk arithmetic/print → run the `add`/`for` sample |
| **R1** | Functions, control flow (`if`/`guard`/`while`/`for-in`/`switch`), ranges |
| **R2** | Value types (`struct`/`enum`), methods, `mutating`, optionals, `if let` |
| **R3** | Reference types (`class`/`actor`), ARC, closures + capture |
| **R4** | Protocols + witness tables, generics (monomorphise via msf's substitution) |
| **R5** | Error handling (`throws`/`try`/`do-catch`), a minimal stdlib |
| **R6 (opt.)** | Lower hot AST → IR → register bytecode VM (the MiniJS/QuickJS path) |

Ship R0–R5 as a tree-walker. Only build R6 if/when throughput matters.

---

## 2. Value representation

Adapt QuickJS-NG §1 and MiniJS §1 — a **tagged union** — but specialise for Swift's
type system. Swift's crucial difference from JS: **value types vs reference types**.

```c
typedef enum {
  SV_VOID, SV_BOOL, SV_INT, SV_DOUBLE,          // unboxed scalars
  SV_STRING,                                     // ref-counted immutable buffer
  SV_STRUCT,                                     // value type: inline/boxed fields, COPY on assign
  SV_ENUM,                                       // tag + associated payload
  SV_CLASS,                                      // reference type: pointer, ARC, SHARE on assign
  SV_CLOSURE,                                    // fn template + captured env
  SV_OPTIONAL,                                   // .none / .some(SwiftValue)
  SV_ARRAY, SV_DICT, SV_SET,                     // stdlib collections (CoW)
  SV_METATYPE,                                   // T.self for dynamic dispatch / witness lookup
} SwiftValueTag;

typedef struct SwiftValue {
  SwiftValueTag tag;
  union {
    bool      b;
    int64_t   i;            // also Int8..UInt64 — width tracked via static type
    double    d;
    SVString *str;          // refcounted
    SVStruct *st;           // value semantics enforced at assignment, not by sharing
    SVEnum   *en;
    SVObject *obj;          // class instance, ARC header
    SVClosure*clo;
    SVArray  *arr;          // CoW backing
    SVTypeMeta *meta;
  } u;
} SwiftValue;
```

**Lessons applied:**
- *(QuickJS §1)* keep small ints / bool / void **unboxed** — most values are these.
  Because msf already tells us the static type, we know at *compile/walk* time whether
  a slot is `Int`, so we can keep it a raw `int64_t` and never box.
- *(QuickJS §1)* encode ownership discipline. Swift makes this explicit: **value types
  copy, reference types share.** Model `=` and argument passing as: `SV_STRUCT`/`SV_ENUM`
  → deep-ish copy (with CoW for the big ones); `SV_CLASS` → retain (bump ARC count).
- **Integer width**: msf distinguishes `TY_INT8…TY_UINT64`. Store them all in `int64_t`
  but carry the `TypeKind` so `&+` (wrapping, `OP_WRAP_ADD`) vs `+` (trapping overflow)
  and truncation behave correctly. This is a Swift-specific obligation JS engines don't have.

---

## 3. Memory management: ARC, not a tracing GC

This is the **single biggest divergence** from both reference VMs and the most
Swift-defining decision.

- MiniJS uses a **conservative mark-sweep GC** (scans the C stack for roots).
- QuickJS-NG uses **refcounting + a cycle collector**.

Swift's actual semantics are **ARC (Automatic Reference Counting)**:
- **Value types** (`struct`, `enum`, tuples, most stdlib types) are **not** heap-managed
  at all in the language model — they're copied. In the runtime, large value types get a
  CoW buffer with its *own* refcount, but semantically there's no sharing.
- **Reference types** (`class`, `actor`, closures capturing references) are
  **retain/release** counted. Each `SVObject` carries an ARC header
  (`uint32_t strong; uint32_t unowned; weak-ref side table`).

**Recommendation:** implement **deterministic refcounting** (QuickJS §9's first half)
— it matches Swift semantics exactly and gives predictable deinit timing (Swift
guarantees `deinit` runs promptly). **Defer the cycle collector.** Swift the *language*
does **not** collect reference cycles (that's why `weak`/`unowned` exist), so a faithful
runtime is allowed to leak cycles too. We honour `weak` (zeroing via a side table) and
`unowned` (no retain) using msf's `MOD_WEAK` / `MOD_UNOWNED` / `MOD_CAPTURE_WEAK` /
`MOD_CAPTURE_UNOWNED` modifier bits (`msf.h:1397`, `:1432`). This is *simpler* than
QuickJS's trial-deletion collector and *more correct* for Swift.

> Takeaway: Swift's ownership model means we get away with **refcounting only** — the
> hardest part of QuickJS's GC (the cycle collector) is explicitly not Swift's job.

---

## 4. Environments, scopes, and name binding

Because msf leaves identifiers as `unresolved_decl_ref_expr` (§0.3), we reuse
**MiniJS's `Env` model** (its §7 / `src/interp/env.c`): a runtime scope chain.

- A `Scope` maps interned name → `SwiftValue` (or a slot for `inout`).
- Lexical lookup walks parent scopes; top of chain is the module/global scope holding
  top-level `func`/`struct`/`enum`/`class` decls and the stdlib.
- **Pre-pass (declare):** before executing a `brace_stmt`/source file, hoist its
  declarations into the scope so forward references work — this mirrors **msf's own
  sema Pass 1 "Declare"** (frontend research §3) and JS function hoisting.
- **Interning:** intern all identifiers to integers and compare by id (QuickJS §3,
  "atoms"). msf already interns names internally; we keep our own intern pool keyed off
  the token text (`token_text(src, tok)`).

For performance later (R6), a resolver pass can convert names → (depth, slot) indices
once, the way QuickJS resolves locals at compile time — but **not needed for R0–R5**.

---

## 5. Mapping msf AST kinds → runtime behaviour

The interpreter core is one `eval(node, env)` dispatcher — a `switch (node->kind)`,
exactly QuickJS §5 / MiniJS §6 in shape. Concrete mapping:

### 5.1 Expressions

| `ASTNodeKind` | Runtime action |
|---|---|
| `AST_INTEGER_LITERAL` | `SV_INT` from `data.integer.ival`, width from `node->type` |
| `AST_FLOAT_LITERAL` / `_BOOL_` / `_STRING_` / `_NIL_` | direct from `data.*` / make `.none` |
| `AST_IDENT_EXPR` (`unresolved_decl_ref`) | `env_lookup(intern(name))` |
| `AST_BINARY_EXPR` | eval both children, dispatch on `OpKind` + operand `TypeKind` |
| `AST_ASSIGN_EXPR` | eval rhs; **store** to lvalue (copy if value type) |
| `AST_UNARY_EXPR` / `AST_FORCE_UNWRAP` | negate / `!` / unwrap optional (trap on nil) |
| `AST_CALL_EXPR` | §6 — function / method / initializer / enum-case dispatch |
| `AST_MEMBER_EXPR` (`unresolved_dot`) | property get, or bound-method, or enum case ref |
| `AST_CLOSURE_EXPR` | capture env → `SV_CLOSURE` (open/closed upvalues, §7) |
| `AST_OPTIONAL_CHAIN` / `AST_NIL_COAL`(`??`) | short-circuit on `.none` |
| `AST_TERNARY_EXPR` / `AST_IF_EXPR` | branch |
| `AST_TRY_EXPR` / `AST_AWAIT_EXPR` | propagate error / suspend (R5 / async later) |
| `AST_ARRAY_LITERAL` / `AST_DICT_LITERAL` / `AST_TUPLE_EXPR` | build collection / tuple |
| `AST_CAST_EXPR` (`as`,`as?`,`is`) | use `node->type` + conformance table |

### 5.2 Statements

| Kind | Action |
|---|---|
| `AST_BLOCK` (`brace_stmt`) | new scope, declare-hoist, eval children |
| `AST_RETURN_STMT` / `AST_THROW_STMT` | non-local exit (§8) |
| `AST_IF_STMT` / `AST_GUARD_STMT` | eval cond incl. `AST_OPTIONAL_BINDING` (`if let`) |
| `AST_WHILE_STMT` / `AST_REPEAT_STMT` | loop |
| `AST_FOR_STMT` (`for_each_stmt`) | get iterator from the sequence; `for i in 0..<3` → range iterator |
| `AST_SWITCH_STMT` + `AST_CASE_CLAUSE` | pattern match (§5.3); `where` via `cas.where_expr` |
| `AST_BREAK` / `AST_CONTINUE` / `AST_FALLTHROUGH` | loop/switch control via signals |
| `AST_DEFER_STMT` | push closure onto scope's defer list; run on scope exit (LIFO) |
| `AST_EXPR_STMT` | eval, discard |

### 5.3 Pattern matching (`switch`, `if case`, `for case`)

msf gives pattern nodes directly: `AST_PATTERN_ENUM`, `_TUPLE`, `_VALUE_BINDING`,
`_WILDCARD`, `_RANGE`, `_GUARD`, `AST_OPTIONAL_BINDING`. A `match(pattern, value, env)`
routine returns bool and **binds** names into `env` on success — e.g.
`case .rect(let w, let h)` matches an `SV_ENUM` with tag `rect`, binds `w`,`h` from the
payload. This is the structural counterpart to QuickJS's destructuring; Swift just gives
us explicit pattern nodes so there's nothing to infer.

---

## 6. Calls: the dispatch hub

`AST_CALL_EXPR`'s first child is the callee; remaining children are args (verified:
`call_expr(print, total)`; `call_expr(dot(arr,.map), closure)`; `call_expr(.rect,2,5)`).
Resolve the callee shape:

1. **Free function** — callee is an ident resolving to an `AST_FUNC_DECL`.
   `call.resolved_callee_decl` may already point at it; otherwise look up in env. Bind
   params (respecting argument labels via `arg_label_tok`), push a frame, eval body.
2. **Method** — callee is `dot(receiver, name)`. Eval `receiver`; find the method on its
   type (struct/class/enum/extension). Bind `self` (a **copy** for value types unless the
   method is `MOD_MUTATING`, in which case `self` is `inout`; for classes `self` is the
   reference). This is where msf's `MOD_MUTATING` / `MOD_STATIC` bits drive semantics.
3. **Initializer** — callee is a type name (`Point(x:3,y:4)`) → allocate the struct/class,
   run the `AST_INIT_DECL`, apply memberwise default if synthesized.
4. **Enum case with payload** — callee is `dot(.rect)` whose type is the enum → construct
   `SV_ENUM{ tag=rect, payload=[2,5] }`. (No call frame; it's a value constructor.)
5. **Closure value** — callee evaluates to `SV_CLOSURE` → invoke with captured env.
6. **Native builtin** — `print`, `Array.map`, operators-as-functions → C function pointer
   (MiniJS §7 `NativeFn` pattern). The entire stdlib starts as native C functions
   registered into the global scope.

**Calls recurse the C stack** (MiniJS §6, QuickJS `OP_call`): `eval` of a call invokes
`eval` on the body. Simple; deep Swift recursion → catchable stack-overflow error, not a
crash. Save a "current node" pointer in each frame before nested calls so backtraces and
(later) the cycle-debug tooling work (QuickJS writes `cur_pc` before re-entry).

**Frame** (tree-walk version):

```c
typedef struct Frame {
  ASTNode  *decl;        // the func/closure being run
  Scope    *scope;       // params + locals (the Env chain head)
  SwiftValue self;       // for methods; inout-aliased for mutating value methods
  struct Frame *parent;  // call stack + GC/backtrace roots
  DeferList defers;      // run LIFO on exit
} Frame;
```

---

## 7. Closures and capture

Swift closures capture by **reference** by default (like JS), with `[weak self]` /
`[unowned self]` capture lists (msf: `AST_CLOSURE_CAPTURE` + `MOD_CAPTURE_*`). Reuse
**QuickJS §7 open/closed upvalues** / MiniJS env capture:

- On `AST_CLOSURE_EXPR`, snapshot the defining `Scope` chain into the `SV_CLOSURE`.
- Captured *variables* (not values) stay live; mutations are visible to closure and
  outer scope (shared cell), matching Swift.
- `[weak x]` → store a weak ref (zeroing); `[unowned x]` → borrow without retain. These
  come straight from msf's capture modifier bits.
- `$0,$1` implicit params (verified in the `.map` dump) and trailing-closure syntax are
  already desugared into `closure_expr` children — nothing special to parse.

---

## 8. Control flow exits: signals (not setjmp, at first)

MiniJS and QuickJS use `setjmp`/`longjmp` + a try-frame stack for exceptions. For a
**tree-walker**, the cleaner first implementation is a **completion-signal** returned up
the `eval` call chain (no `longjmp`):

```c
typedef enum { CO_NORMAL, CO_RETURN, CO_BREAK, CO_CONTINUE,
               CO_THROW, CO_FALLTHROUGH } CompletionKind;
typedef struct { CompletionKind kind; SwiftValue value; /*label*/ } Completion;
```

Every `eval` returns a `Completion`; block/loop evaluators check it and unwind,
**running `defer` blocks** as scopes pop. `throws`/`try`/`do-catch` (msf: `MOD_THROWS`,
`AST_TRY_EXPR`, `AST_DO_STMT`, `AST_CATCH_CLAUSE`) ride the same `CO_THROW` channel; a
`do` with `catch` clauses pattern-matches the thrown error value. This is simpler to get
right than stack-marker unwinding and avoids `longjmp`/`defer` interaction bugs. If we
build the **R6 bytecode VM**, *then* switch to QuickJS §6's stack-marker scheme
(`CATCH_OFFSET` on the operand stack), which also powers `defer`/iterator-close in one
pass.

---

## 9. Types, protocols, generics — leaning on msf

This is where having a **typed** AST pays off massively versus the untyped JS engines.

- **Monomorphic arithmetic / property access** (QuickJS §5's "80% of JIT-less speed"):
  because `node->type` is known, we don't probe values at runtime — we emit/eval the
  `Int`-path or `Double`-path directly. No `js_add_slow` fallback maze.
- **Protocols → witness tables.** msf already computes conformances: the
  **ConformanceTable** (§14) answers `conformance_table_has(type, protocol)`, and the
  **AssocTypeTable** (§15) resolves associated types. The backend builds a witness table
  per (type, protocol) by collecting the methods msf's sema matched, then dynamic protocol
  dispatch is a table lookup keyed by `SV_METATYPE`. This is the documented backend seam.
- **Generics → substitution.** Rather than runtime type erasure, **monomorphise**: msf
  exposes `type_substitute(generic, &sub, &arena)` (§12) and `TypeSubstitution`. At a
  generic call site we read the concrete type args off `node->type`
  (`TY_GENERIC_INST.generic.args`), build a `TypeSubstitution`, and specialise the body
  — or, in the tree-walker, simply carry the substitution in the frame so `T` resolves to
  `Int` during the walk. Constraints (`where T: Equatable`) are checked against the
  conformance table.
- **`as?` / `is`** use `type_kind_of` + the conformance table directly.

> Net: msf's §10–§16 Backend ABI (type arena, substitution, conformance + assoc-type
> tables) is *exactly* a protocol/generic runtime support library handed to us. We do not
> re-derive conformances; we consume them.

---

## 10. The standard library problem (the real work)

The frontend research's blunt conclusion: msf has **no runtime, no stdlib**. Neither do
we, initially. The pragmatic path (MiniJS §7: "the entire stdlib is native C functions"):

- **Tier 1 (R0–R2):** native C implementations of `print`, `Int`/`Double`/`Bool`/`String`
  operators, `Range`/`ClosedRange` iteration, `Array`/`Dictionary`/`Set` with **CoW**,
  `Optional`, `String` (UTF-8, since msf already vendors NFC). Register them in the global
  scope as `SV_CLOSURE`-wrapped native fns.
- **Tier 2 (R4–R5):** `Sequence`/`Collection`/`IteratorProtocol` as *real protocols* with
  native witness tables so `map`/`filter`/`reduce`/`for-in` work generically; `Equatable`/
  `Comparable`/`Hashable`/`Codable` synthesis (msf marks the conformances; we provide the
  witnesses).
- msf's **baked SDK vocabulary** (`sdk_vocab*.h`, frontend research §1) tells us the
  *shape* of SDK types for name resolution, but **not their behaviour** — behaviour is on
  us. Scope the supported surface deliberately (this is the "person-years piece"
  miniswift kept private — we choose a small, useful subset).

---

## 11. Build order (concrete, merging both VM studies)

Synthesising MiniJS's "tree-walk → VM" and QuickJS's 10-step list, specialised for Swift:

1. **Value model** — tagged union, unboxed `Int/Double/Bool/Void`; value-vs-reference tag.
2. **Intern pool** for identifiers (atoms).
3. **Env / Scope chain** + declare-hoist pass (mirrors msf sema Pass 1).
4. **`eval(node, env)` tree-walker**: literals → arithmetic → `print` → **run the
   `add`/`for` sample** (R0 done).
5. **Functions + control flow + ranges + pattern `switch`** (R1).
6. **Value types**: struct/enum construction, methods, `mutating` (inout self),
   optionals, `if let`/`guard let` (R2).
7. **Reference types + ARC refcounting + `weak`/`unowned`** + closures with open/closed
   upvalues (R3). *No cycle collector — Swift doesn't either.*
8. **Protocols (witness tables from msf's ConformanceTable) + generics (monomorphise via
   `type_substitute`)** (R4).
9. **Error handling via completion signals + `defer`** + Tier-2 stdlib (R5).
10. **(Optional R6)** AST → IR (basic blocks, virtual regs) → liveness + graph-colouring
    reg alloc → fixed-width register bytecode → `switch`/computed-goto VM loop; move
    exceptions to QuickJS stack-marker scheme. This is the MiniJS §3–§6 / QuickJS §4–§8
    machinery, applied only once correctness is proven.

> **"Get it running first, then make it fast"** — both studies. msf's typed AST lets us
> skip straight to a *correct, typed* tree-walker; the bytecode VM is a later throughput
> optimisation, not a prerequisite.

---

## 12. Integration seam with msf (practical)

- Link `libMiniSwiftFrontend.a`, `#include <msf.h>` only.
- **Provide the backend stubs msf expects**: `module_stub_find` and the
  `sema_import_module` vocabulary hook (currently `tests/stubs.c` returns NULL). This is
  the literal "frontend/backend seam." For real stdlib name resolution, feed msf a
  vocabulary (`msf_analyze_with_vocab` / `MSFModule`) describing our supported stdlib
  surface so references resolve and `node->type` is populated for our types.
- Drive: `MSFResult *r = msf_analyze(src, file)`; bail/report via `msf_error_*` if
  `msf_error_count(r) > 0`; else `interpret(msf_root(r), msf_tokens(r), source)`.
- **Lifetime**: the AST + `TypeInfo` are arena-owned by `r` and immutable — keep `r`
  alive for the whole run; never free nodes mid-execution. Re-walking loop/closure bodies
  is just re-visiting stable pointers.
- **String interpolation**: use `msf_parse_expression` (§16) to parse the `\( … )`
  fragments on demand instead of shipping our own parser.

---

## 13. What's hard / open questions

1. **Stdlib scope.** The behaviour of `String`, `Array`, `Dictionary`, `Codable`,
   concurrency — unbounded. Must pick a deliberate subset (this is *the* cost driver).
2. **Concurrency** (`async`/`await`/`actor`/`Task`). msf parses and marks it
   (`MOD_ASYNC`, `AST_ACTOR_DECL`, `AST_AWAIT_EXPR`, `MOD_MAIN_ACTOR`), but a runtime
   needs a scheduler/executor. MiniJS does async via "save VM frame + microtask queue"
   (§6) — feasible, but only with the R6 VM (suspending a C-stack tree-walker is hard).
   Recommend: **defer async to post-R6**; a coroutine needs saved interpreter state, which
   the register VM gives cheaply (MiniJS: "don't free the registers, remember the pc").
3. **Overflow/trap semantics.** Faithful `Int` overflow traps and `&+` wrapping need
   per-width handling — Swift-specific, no JS analogue.
4. **`inout` aliasing & exclusivity.** Value-type `inout` params and `mutating self` need
   true lvalue aliasing (a slot pointer), not a copy — design the lvalue path early.
5. **CoW correctness.** `Array`/`Dict`/`String` value semantics with shared buffers +
   uniqueness check on mutation (`isKnownUniquelyReferenced`) — get the refcount-on-write
   right or value semantics silently break.
6. **Deinit timing.** ARC must run `deinit` deterministically at refcount 0 (Swift
   guarantees this) — easy with refcounting, impossible with pure tracing GC. Validates
   the §3 "refcount-only" choice.

---

## 14. One-paragraph summary

msf gives us a **typed, immutable, arena-owned Swift AST** plus a **protocol/generic
support library** (conformance table, assoc-type table, `type_substitute`) and a
documented **backend seam** — i.e. everything *except* the thing that runs code. The
proven recipe from MiniJS and QuickJS-NG is: **value model → intern → env → typed
tree-walker → (later) bytecode VM.** Swift's twists are **value-vs-reference semantics**
and **ARC** — and the happy news is ARC means we implement **refcounting only and skip
the cycle collector** (Swift doesn't collect cycles either). Start with a tree-walker
that reads `node->type` to stay monomorphic, lean on msf's tables for protocols/generics,
implement the stdlib as native C functions over a deliberately-scoped surface, and run
R0–R5 before deciding whether the R6 register VM is worth it. **Get it running first.**
