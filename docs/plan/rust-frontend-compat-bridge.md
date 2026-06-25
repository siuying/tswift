# Plan — Bridge the Rust Frontend / Runtime AST Compatibility Gap

**Status:** proposed  
**Date:** 2026-06-25  
**Context:** follow-up to the #36 cutover attempt  
**Related:**
- `docs/plan/replace-msf-with-rust-frontend.md`
- `docs/research/msf-ast-cheatsheet.md`
- GitHub issues #36 and #37

## 1. Problem statement

The #36 experiment showed that replacing the `qswift-frontend` backend with
`qswift-parser` + `qswift-sema` is not blocked mainly by parse acceptance. It is blocked
by **runtime-facing AST contract compatibility**.

The runtime currently consumes the frontend through a small surface:

- `Analysis::analyze`, `root`, `diagnostics`, `is_ok`
- `Node::kind`, `children`, `text`, `int`, `float`, `bool`, `line`
- semantic helpers such as `decl_name`, `op_text`, `type_name`, `arg_label`,
  `param_info`, `case_info`, `var_accessors`, `modifiers`, `modifier_names`,
  `jump_label`, `loop_label`, `ownership`, `is_async_let`
- `NodeKind` names and dump shape used by runtime code and AST snapshots

The Rust AST is cleaner than the msf AST, but the runtime was written against msf's
**effective tree contract**: node kinds, child ordering, payload conventions, modifier
bits, synthesized semantic nodes, and type annotations. The attempted direct adapter
forced every mismatch to leak into `qswift-core`, producing a broad set of failing
runtime tests.

## 2. Decision

Do **not** keep patching the runtime for Rust AST differences. Instead, introduce one
deep compatibility module inside `qswift-frontend`:

```text
qswift-lexer/parser/sema clean AST
          │
          ▼
qswift-frontend::compat lowerer
          │
          ▼
RuntimeAst: msf-compatible runtime-facing tree
          │
          ▼
Analysis / Node / NodeKind public facade
          │
          ▼
qswift-core / qswift-std unchanged
```

The seam remains `qswift-frontend::{Analysis, Node, NodeKind}`. The new compat
lowerer hides the messy shape conversion behind that seam.

## 3. Non-goals

- Do not rewrite `qswift-core` to understand both msf and Rust AST shapes.
- Do not delete `msf-sys` until the Rust backend passes the full workspace test suite.
- Do not make `qswift_ast::NodeKind` a clone of msf. Keep `qswift_ast` clean and lower
  into a separate runtime-facing representation.
- Do not skip or weaken runtime behavior fixtures as the final solution. Temporary
  `#[ignore]`/allow-lists are acceptable only on a WIP branch and must be tracked.

## 4. Target design

### 4.1 Add `RuntimeAst`

Add an internal module, e.g. `crates/qswift-frontend/src/compat/`, containing:

```rust
pub(crate) struct RuntimeAst {
    nodes: Vec<RuntimeNode>,
    diagnostics: Vec<Diagnostic>,
    source: SourceMap,
}

pub(crate) struct RuntimeNode {
    kind: NodeKind,
    text: Option<String>,
    line: u32,
    col: u32,
    ty: Option<String>,
    modifier_bits: u32,
    flags: RuntimeFlags,
    payload: RuntimePayload,
    first_child: Option<NodeId>,
    next_sibling: Option<NodeId>,
}
```

`Analysis` should own either the C result or a `RuntimeAst` depending on backend feature.
`Node<'a>` should become a thin cursor over the selected backing store. Callers should
not learn which backing store produced the tree.

### 4.2 Keep two node vocabularies

- `qswift_ast::NodeKind`: clean parser/sema IR.
- `qswift_frontend::NodeKind`: stable runtime-facing compatibility vocabulary.

The compat lowerer maps from the former into the latter. This lets us improve the Rust
frontend without breaking runtime callers.

### 4.3 Define the compatibility contract explicitly

Create `crates/qswift-frontend/src/compat/contract.rs` or a markdown spec listing,
for every runtime-facing `NodeKind`:

- expected payload (`text`, `op_text`, `decl_name`, `type_name`, labels)
- child order
- type annotation behavior
- modifier/flag behavior
- known runtime consumers in `qswift-core`
- one AST snapshot fixture that pins the shape

This turns undocumented msf behavior into our owned contract.

## 5. TDD strategy

Bridge the gap fixture-first. For each failing feature, add or update an AST snapshot
before implementing the lowerer rule.

### Test layers

1. **Compat unit tests** in `qswift-frontend`:
   - parse Rust AST
   - run lowerer
   - assert exact runtime-facing dump shape

2. **Differential tests while C msf exists**:
   - compare C-backed `Node::dump()` to Rust-backed compat dump
   - allow-list intentional differences only with comments and issue links

3. **Runtime golden tests**:
   - no fixture is considered fixed until the corresponding `qswift-core` or
     `qswift-cli` runtime test passes on the Rust backend

4. **Full presubmit gate**:
   - `cargo test --workspace`
   - `scripts/presubmit` before PR

## 6. Implementation phases

### Phase 0 — Freeze the public seam

Goal: make the interface stable before changing internals.

Tasks:

- Keep `qswift-frontend::{Analysis, Node, NodeKind}` as the only runtime-facing
  interface.
- Promote any remaining `NodeKind::Other(n)` runtime matches into named variants.
- Add a `frontend-backend` test helper that can run the same fixture against C and Rust
  backends.
- Add a manifest of runtime-facing AST snapshots under
  `crates/qswift-cli/tests/fixtures/ast/` or a new
  `crates/qswift-frontend/tests/fixtures/compat/` directory.

Exit criteria:

- C backend still green.
- The compat contract has snapshots for the constructs already relied on by runtime
  fixtures.

### Phase 1 — Build the minimal `RuntimeAst` skeleton

Goal: make the Rust backend produce a tree through the same cursor methods without C.

Tasks:

- Introduce `RuntimeAst`, `RuntimeNode`, `RuntimePayload`, and `RuntimeFlags`.
- Implement `Analysis::analyze_rust` behind a feature flag; keep C as default.
- Implement `Node` methods by delegating to a backend enum:

```rust
enum AnalysisBacking {
    C(*mut msf_sys::MSFResult),
    Rust(RuntimeAst),
}
```

- Lower only the walking-skeleton constructs first: source file, block, literals,
  identifiers, binary/assign/call expressions, let/var, function declarations,
  return, if/for/while/switch basics.

Exit criteria:

- Simple fixtures (`hello`, `arithmetic`, `functions_recursion`, `control_flow`) pass
  on the Rust backend without runtime changes.

### Phase 2 — Nominal/type-shape parity

Goal: fix the highest-leverage mismatch: declarations and type metadata.

Tasks:

- Lower nominal declarations into msf-compatible shapes:
  - `StructDecl`, `ClassDecl`, `EnumDecl`, `ProtocolDecl`, `ExtensionDecl`
  - `Conformance` wrapper nodes for inherited protocols/types
  - body as `Block` with the expected text/line behavior
- Lower type nodes consistently:
  - `TypeIdent`, optional/IUO forms, function types, tuple types, metatypes,
    generic arguments
  - attach resolved type names to declaration and expression nodes expected by runtime
- Preserve declaration names directly on `LetDecl`/`VarDecl` when runtime expects them,
  even if the clean Rust AST uses pattern children.

Exit criteria:

- AST snapshots for `struct_codable.swift` and nominal fixtures match the runtime-facing
  contract.
- Runtime fixtures for structs/enums/protocol shell cases pass on Rust backend.

### Phase 3 — Modifiers, attributes, and semantic flags

Goal: restore metadata currently encoded in msf-specific fields.

Tasks:

- Define our own modifier bit layout in `qswift-frontend`; stop depending on C enum
  values.
- Lower access/modifier keywords:
  - `mutating`, `static`, `class`, `final`, `override`, `weak`, `unowned`, `lazy`,
    `indirect`, `throws`, `async`, `rethrows`, access control
- Lower attributes:
  - `@main`, `@propertyWrapper`, `@discardableResult`, `@escaping`, `@autoclosure`,
    `@MainActor`, `@unknown`, etc.
- Implement helper parity:
  - `modifiers`, `modifier_names`, `ownership`, `is_async_let`, `param_info`,
    `var_accessors`

Exit criteria:

- `main_entry`, `property_wrapper`, `struct_static_lazy`, `deinit_weak`, async-let
  metadata fixtures pass on Rust backend.

### Phase 4 — Pattern lowering parity

Goal: isolate Swift's pattern complexity in the compat lowerer, not the runtime.

Tasks:

- Lower all runtime-observed patterns into stable runtime-facing nodes:
  - binding patterns (`let`, `var`, wildcard)
  - tuple patterns
  - enum case patterns (`.some(let x)`, `.none`, `Enum.case(...)`)
  - optional patterns (`x?`)
  - cast/is patterns
  - range/value patterns
  - `where` clauses and fallthrough markers in switch cases
- Preserve child order used by `qswift-core::match_pattern`.

Exit criteria:

- `switch_patterns`, `enum_matching`, `optionals`, `indirect_enum`, and tuple-pattern
  runtime tests pass on Rust backend.

### Phase 5 — Effects and executable wrappers

Goal: make runtime execution see the same wrappers around effects and directives.

Tasks:

- Lower `try`, `try?`, `try!`, `throws`, `throw`, `do/catch`, and `defer` exactly as the
  runtime expects.
- Lower `async`, `await`, `Task`, `Task.detached`, task groups, `actor`, and
  `for await` metadata used by the runtime.
- Lower compiler directives:
  - `#if`/`#elseif`/`#else` executable branch wrappers
  - `#file`, `#line`, `#column`, `#function`
  - `#warning`, `#error`

Exit criteria:

- `try_variants`, `errors`, `defer_order`, `conditional_compilation`, and concurrency
  runtime fixtures pass on Rust backend.

### Phase 6 — Calls, labels, accessors, and operators

Goal: remove the remaining expression-shape divergences.

Tasks:

- Normalize constructor calls and function calls:
  - callee shape
  - argument label payload
  - default arguments
  - variadics
  - `inout` argument node
- Normalize property/subscript access:
  - getter/setter/willSet/didSet accessors
  - static type member access
  - optional chaining and force unwrap
- Normalize operator declarations and precedence groups:
  - custom infix/prefix/postfix declarations
  - precedence lookup for parser and runtime dump shape

Exit criteria:

- `struct_observers`, `subscripts`, `func_labels_defaults`, `func_variadic`,
  `custom_operator`, and `super_init` fixtures pass on Rust backend.

### Phase 7 — Flip the default and decommission C

Goal: remove msf only after evidence says the compatibility bridge is complete.

Tasks:

- Run full matrix:
  - C backend tests
  - Rust backend tests
  - differential compat snapshots
  - `cargo test --workspace`
  - `scripts/presubmit`
- Flip default frontend backend to Rust.
- Keep C oracle behind an opt-in feature for one soak PR if feasible.
- Delete `crates/msf-sys`, `vendor/msf`, `.gitmodules`, C build scripts, `bindgen`, and
  `cc` only after the default Rust suite is green.

Exit criteria:

- No `msf-sys` dependency remains.
- `cargo test --workspace` passes without a C toolchain.
- #36 can close.
- #37 can start from a Rust-default frontend.

## 7. Suggested issue split

1. **Compat bridge skeleton** — add `RuntimeAst` and backend enum, no cutover.
2. **Nominal/type lowering** — conformances, declaration shapes, type metadata.
3. **Modifier/attribute lowering** — modifiers, property wrappers, `@main`, async flags.
4. **Pattern lowering** — switch/optional/enum/tuple patterns.
5. **Effects/directives lowering** — try/throw/defer/async/#if.
6. **Calls/accessors/operators lowering** — labels, inout, observers, custom operators.
7. **Rust-default cutover** — flip default, delete C only after green presubmit.

This split prevents #36 from being another big-bang PR.

## 8. Definition of done for the bridge

The bridge is complete when:

- `qswift-core` has no Rust-vs-msf AST special cases.
- `qswift-frontend` owns the runtime-facing `NodeKind` contract.
- All runtime fixtures pass on the Rust backend.
- AST snapshots document the compatibility shapes we intentionally expose.
- `cargo test --workspace` and `scripts/presubmit` pass without C tooling.
- The feature checklist is updated for the verified Rust frontend cutover row.

## 9. Immediate next action

Start with Phase 0 + Phase 1 in a small PR:

1. Add `RuntimeAst` behind a `rust-backend` feature.
2. Keep the C backend as default.
3. Port only enough lowering for `hello.swift`, `arithmetic.swift`, and
   `functions_recursion.swift`.
4. Add compat snapshots for those fixtures.
5. Do not delete `msf-sys` yet.

That PR proves the seam and testing strategy without betting the entire cutover on one
large change.
