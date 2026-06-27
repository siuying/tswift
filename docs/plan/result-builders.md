# Plan — Full Result Builders (`@resultBuilder`) via a Sema-time Transform

**Status:** proposed (supersedes the runtime-transform approach); sema seams landed
**Date:** 2026-06-27 (rev. after `Symbols` + `Pass` pipeline refactor)
**Reference toolchain:** Swift **6.3.2** (`swift-6.3.2-RELEASE`)
**Related:**
- `docs/swift-runtime/feature-checklist.md` — Tier 8 (`@resultBuilder` DSL transform, method synthesis)
- `docs/plan/swift-runtime-implementation-plan.md` — overall phasing (R6+)
- Reference: [SE-0289 *Result Builders*](https://github.com/swiftlang/swift-evolution/blob/main/proposals/0289-result-builders.md);
  swiftc `lib/Sema/BuilderTransform.cpp`

## 1. Decision

Move the result-builder transform from the **runtime interpreter** to
**compile time**, performed as an **AST→AST rewrite in `tswift-sema`** — the same
place and shape the real Swift compiler does it (`BuilderTransform.cpp`, during
type checking). After the rewrite, the builder is **erased**: the interpreter
sees ordinary `Builder.buildBlock(...)` method calls and needs no
builder-specific evaluation code.

This supersedes the previous plan, which kept the transform in
`crates/tswift-core/src/interp.rs` (`eval_result_builder_*`). That code, plus the
`result_builder` fields on `FuncDef`/closures/`Param`, will be **removed** once
the sema transform reaches parity (§8 migration).

## 2. Why move it

| | Runtime transform (today) | Sema transform (this plan) |
|---|---|---|
| Fidelity to swiftc | Divergent re-implementation | Same model: rewrite during type-check, erased after |
| Cost | Re-runs the whole transform **on every call** | Runs **once** at compile time |
| Diagnostics | Need a separate ad-hoc pass | Fall out of resolving the synthesized AST + a small validator |
| Control-flow code | Duplicates `for`/`if`/`switch`/`defer`/scope logic | Reuses the interpreter's existing statement evaluation |
| Type inference | Impossible (no types at runtime) | Synthesized nodes get resolved like any expression |
| Interpreter surface | `eval_result_builder_*` (~240 lines) + 3 struct fields | **deleted** |

The runtime version also can't ever see static types, so overload-by-type,
generic builders, and incompatible-component diagnostics are structurally out of
reach. Moving to sema unblocks them.

## 3. Target pipeline (the seams already exist)

The `Symbols` registry and `Pass` pipeline refactor put the two seams the
transform needs in place. We no longer build them — we plug into them:

```
source
  → tswift_parser::parse            → Ast (mutable arena)
  → tswift_sema::analyze(&mut ast)            [passes.rs]
        Symbols::collect(ast)                 [symbols.rs]
            · enums, func_returns  (existing)
            · result_builders      ← NEW field+getter (S1)
        pipeline():
            1. BuilderTransform pass           ← NEW; rewrites builder-bodied blocks (the work)
            2. Annotate pass        (existing); type-annotates the now-ordinary calls
  → compat::lower_node              → RuntimeAst (already transformed)
  → interp.run                      → evaluates ordinary method calls
```

Concretely, two insertions into the refactored crate:

1. **`Symbols`** (`crates/tswift-sema/src/symbols.rs`) — a new `result_builders`
   field + `result_builder(name) -> Option<&BuilderMethods>` getter, collected in
   the existing `Symbols::walk`. This is exactly the growth the module's doc
   anticipates ("a new field and getter on one registry").
2. **`BuilderTransform`** (new pass) — a `Pass` added to `passes::pipeline()`
   **before** `Annotate`. `Pass::run(&self, ast: &mut Ast, symbols: &Symbols)`
   already gives it the mutable arena and the registry it needs.

`analyze()` collects `Symbols` **once**, before any pass, so `BuilderTransform`
and `Annotate` share one registry. The transform only rewrites decl *bodies*
(adding `let`/call/member nodes); it never adds top-level decls, so collecting
Symbols up front stays correct. Annotation then runs over the rewritten tree —
mirroring swiftc, where the transform produces an expression that is then
type-checked.

## 4. The transform (faithful to SE-0289 / `BuilderTransform.cpp`)

The output of transforming a builder-bodied `Block` is **another `Block`**: a
sequence of synthesized statements that bind each component to a fresh
variable, ending in a single `return Builder.buildFinalResult(Builder.buildBlock(v0, v1, …))`.
Control flow stays as **real statements** (so `if`/`for`/`switch` bodies execute
normally); each produces a component value captured into a variable.

Lowering table (each `vN` is a fresh synthesized `let`/`var`):

```
source statement                target synthesized statements
──────────────────────────────────────────────────────────────────────
expr                            let vN = Builder.buildExpression(expr)
                                  (or `let vN = expr` if no buildExpression)
{ s1 … sk }                     transform each → v1…vk;
                                  let vBlock = Builder.buildBlock(v1, …, vk)
                                  (empty → Builder.buildBlock(); or partial-block fold, §4.1)
if c { A }                      var vN; if c { …A→vA; vN = buildEither?/buildOptional }
if c { A } else { B }           buildEither(first: vA) / buildEither(second: vB)
switch …                        nested buildEither tree over the cases
for x in xs where w { B }       var arr=[]; for x in xs where w { …B→vB; arr.append(vB) }
                                  let vN = Builder.buildArray(arr)
if #available(…) { A }          let vN = Builder.buildLimitedAvailability(vA)   (then buildOptional/Either)
let/var/func/type decl          left in place — NOT a component
return <expr>                   only legal as the sole statement (else diagnostic, §7)
outermost result                return Builder.buildFinalResult(vBlock)  (buildFinalResult optional)
```

All of these are constructed in the arena via `Ast::add` / `append_child` /
`set_arg_label`, moving user subexpressions with `clone_subtree`.

### 4.1 Method selection (the one place we diverge from swiftc)

swiftc selects builder methods through the **constraint solver** (bidirectional
overload ranking, type inference). `tswift-sema` is a single-pass, forward-only
annotator with **no solver**, so we select in three tiers, escalating only as
far as the available information allows:

- **Tier A — structural** (covers ~all real builders). Pick by name/label/arity
  from the builder's declared static-method set (`Symbols::result_builder`):
  - prefer the `buildPartialBlock(first:)` + `buildPartialBlock(accumulated:next:)`
    pair when **both** are declared; else fold with variadic `buildBlock`.
  - emit `buildExpression` / `buildOptional` / `buildLimitedAvailability` /
    `buildFinalResult` calls **only when** the builder declares them; else the
    value passes through.
  - `buildEither(first:)` / `(second:)` by label.
- **Tier B — sema-typed** (closes the common `buildExpression`-by-type case).
  When a method name has >1 overload separable only by parameter type, use the
  type the **`Annotate` pass would record** on the argument node to pick the
  matching overload at compile time. (The transform runs before `Annotate`, so
  this needs a small forward type probe on the argument expression — literals
  and bound identifiers, which is what sema already knows.)
- **Tier C — solver** (bidirectional, contextual, conformance-ranked):
  **out of scope** — no constraint solver.

**Ambiguity is diagnosed, never guessed.** When a builder declares overloads of
the same name+label+arity (separable only by type) and Tier B's forward type
does not uniquely match one parameter type, emit an S10 diagnostic
("ambiguous result-builder method") rather than picking arbitrarily. This
converts the one case we can't resolve from a silent wrong-output bug into a
clear error. (Note: the *current runtime* registry already collapses such
overloads silently — same `(builder, method, first_label)` key overwrites — so
this is a strict improvement, not a regression.)

The undecidable Tier-C cases (contextual/backward selection, conformance-only
overloads) are covered by a `frontend-gap` fixture, not mis-compiled.

## 5. Recognizing builder attributes

A builder attribute is a custom `@Name` attribute whose `Name` is a known
`@resultBuilder` type. `Symbols::collect` (which already walks nested scopes)
records the set of `@resultBuilder` type names and each one's declared static
methods. A builder body is then any of:

- `@Builder func` / `@Builder var`/`subscript` **getter** / explicit accessor.
- a **closure literal** passed to a parameter annotated `@Builder` — *contextual*
  inference. Handled by a new `Symbols` query for per-parameter builder
  attributes on a function's signature, then at a resolved call site
  transforming the matching closure-literal argument.
- a **nested/local** `@resultBuilder` declared inside a function/type — already
  covered, since `Symbols::walk` recurses into nested declarations.

## 6. Workstreams

Each lands with fixtures and a Tier 8 checklist update.

### S0 — AST-builder helpers (sema)
Small internal `astbuild` module in `tswift-sema` to synthesize arena nodes via
the existing `Ast` mutation API (`add`/`append_child`/`set_arg_label`/
`clone_subtree`): `static_call(builder, method, [labelled args])`,
`fresh_let(name, expr)`, `ident(name)`, `var_decl`, `assign`, `append_call`,
`block(stmts)`. Foundation for the transform pass; unit-tested directly on the
arena. Fresh names use a reserved `$build` prefix the lexer can't produce, so
synthesized bindings never collide with user names (§11).

### S1 — `result_builder` query on `Symbols` (sema)
Add a `result_builders: HashMap<String, BuilderMethods>` field + a
`result_builder(name) -> Option<&BuilderMethods>` getter to `symbols.rs`,
populated in `Symbols::walk`: detect a nominal decl carrying a `@resultBuilder`
attribute and record its declared static methods (name + first-param label +
arity). Also a `func_builder_params(name)` query for S8's per-parameter
attributes. Drives §4.1 selection. *(The pass seam itself already exists — this
is the only registry work.)*

### S2 — `BuilderTransform` pass: core block (sema)
Add the new pass to `passes::pipeline()` **before** `Annotate`. Rewrite a
builder-bodied `Block`: `buildExpression` per expression statement, `buildBlock`
fold (variadic **or** `buildPartialBlock`, §4.1), empty `buildBlock()`, trailing
`buildFinalResult`. Leave declarations in place. Targets `@Builder func` first.
Add a `Pass`-interface test (a builder body rewrites to the expected call tree)
alongside the existing `passes.rs` tests.
- Replaces the runtime path for the existing `result_builder.swift` fixture
  (kept green throughout).

### S3 — `if` / `else` / `if let` / optional (sema)
`buildEither(first:)`/`(second:)`; bare `if` → `buildOptional`; pattern
conditions (`if let`) bind then transform the then-branch.

### S4 — `switch` → nested `buildEither` tree (sema)
Balanced first/second tree over cases; reuse case patterns/`where`/bindings as
real `switch` statements that assign into the component var. `default` = final
`second`.

### S5 — `for` → `buildArray` (sema)
Accumulator-var rewrite (§4 table) with pattern bindings, `where`,
`break`/`continue`, labeled loops — all preserved as a real `for` statement.

### S6 — `if #available` → `buildLimitedAvailability` (sema)
Wrap the availability-branch component before the surrounding
`buildOptional`/`buildEither`.

### S7 — Declaration targets (sema)
Extend the `BuilderTransform` pass to: computed-property getters, subscript
getters, explicit `get` accessors, and nested/local builders. Recognize the
builder attribute on these decls (via `Symbols`) and transform their accessor
`Block`.

### S8 — Contextual closure builders (sema)
`Symbols::func_builder_params` (S1) → transform closure-literal arguments at
resolved call sites, including **trailing-closure** and **multi-closure** forms,
and **generic** builder parameters (`func f<C>(@B _: () -> C)`). Verify
composition with `@escaping` / `@autoclosure` / `inout`.

### S9 — `return` / `throws` / `async` / `guard` semantics (sema)
- `return` legal only as the sole statement (else S10 diagnostic).
- `throws`: synthesized calls inherit the body's throwing context (the rewrite
  is transparent to `Signal::Throw`).
- `guard` is **not** a component — left as control flow (early exit).
- `async`: components evaluated in order; gated on async-closure support.

### S10 — Diagnostics (sema)
Emitted while collecting/transforming, through the existing `Diagnostic`
channel:
- builder attribute on a **non-function-typed parameter**.
- `@resultBuilder` type **missing** `buildBlock` (and not providing the
  `buildPartialBlock` pair).
- invalid builder-method **signature** (non-`static`; `buildEither` lacking
  `first:`/`second:`).
- **unsupported statement** in a builder body (`while`/`repeat`; `return` mixed
  with components).
- **ambiguous** overloads / unresolvable type-only overload (the §4.1 gap).

### S11 — Interpreter cleanup (tswift-core)
Once S2–S9 reach parity behind the existing fixtures, **delete**
`eval_result_builder_*`, the `(builder, method, first_label)` registry, and the
`result_builder` fields on `FuncDef`/`ClosureDef`/`Param`. Net: the interpreter
loses ~240 lines and three fields; result builders become "just method calls."

### S12 — Test corpus
- **Frontend golden** (`tests/swift-fixtures/tier8-macros/`): one positive
  fixture per hook + per declaration target; `expected-error` fixtures for every
  S10 diagnostic; a `frontend-gap` fixture for the §4.1 type-only-overload
  limitation.
- **Runtime golden** (`crates/tswift-cli/tests/fixtures/`): `.swift` + `.expected`
  per lowering case (partial-block, final-result, limited-availability, switch,
  nested builder, computed property, generic/overloaded builder).
- **AST snapshots** (`fixtures/ast/`): pin the **post-transform** shape via
  `tswift dump` for a representative builder body — this is the highest-value
  regression guard for a sema rewrite.

## 7. Sequencing

```
S0 (astbuild) ─ S1 (Symbols query) ─ S2 (BuilderTransform: core block)
                                         ├─ S3 (if) ─ S4 (switch) ─ S6 (#available)
                                         └─ S5 (for)
S7 (decl targets) ── S8 (contextual closures)
S9 (return/throws/async/guard)
S10 (diagnostics) ── S12 (fixtures)   ← continuous, every S lands with fixtures
S11 (interpreter cleanup)             ← only after S2–S9 are green at parity
```

The pass seam and `Symbols` registry already exist, so S0+S1 are small and
unblock everything. S2–S9 all extend one `BuilderTransform` pass. S11 is last
and gated on parity to keep the existing fixtures green throughout.

## 8. Migration & rollback

- The sema transform is built **alongside** the runtime path. Keep both until S2–
  S9 pass the *existing* `result_builder.swift` runtime fixture and the Tier 8
  frontend fixture.
- A feature flag (env or a `resolve` parameter) can toggle the transform during
  development so a regression falls back to the runtime path. Removed with S11.
- Rollback is "don't run the new pass" — the runtime path stays until S11
  deletes it, so no point in the sequence is unshippable.

## 9. Divergences & limitations (explicit)

- **No constraint solver.** Method selection is Tier A (structural) + Tier B
  (forward sema type), never Tier C (bidirectional/contextual/conformance).
  Overloads decidable only by Tier C are **diagnosed** (§4.1), not guessed, and
  covered by a `frontend-gap` fixture — never miscompiled.
- **Generic builders** work to the extent the synthesized calls resolve through
  the interpreter's existing `infer_type_bindings`; we do not add full generic
  inference to sema.
- **`async`/`do`-as-expression** lowering is gated on those constructs being
  supported as value-producing forms elsewhere in the runtime.

## 10. Acceptance

- Every S-item: green golden fixture(s) (frontend + runtime) and a Tier 8
  checklist update (`docs/swift-runtime/feature-checklist.md` ~line 334 and
  ~367).
- After S11: `interp.rs` contains **no** `result_builder` code; `cargo test`
  (workspace) green, including `golden_fixtures`, CLI `golden`, and the new
  post-transform AST snapshot.
- No regression in the existing `result_builder.swift` runtime fixture at any
  point.

## 11. Open questions

- **Synthesized name collisions** — fresh bindings use a reserved `$build`
  prefix the lexer can't produce, so they can't shadow user names. Confirm the
  `Annotate` pass binds them without complaint (it runs after the rewrite).
  *(The placement question is settled: `BuilderTransform` is a whole-tree pass
  before `Annotate` in `pipeline()`.)*
- **`buildFinalResult` boundary** for computed-property/subscript targets (S7):
  applies at the accessor's result, per SE-0289 — verify against the spec's
  examples.
- **Variadic `buildBlock` vs `buildPartialBlock` fold** when a builder declares
  *both*: SE-0289 prefers `buildPartialBlock`; confirm our structural rule
  matches and add a fixture where both exist.
