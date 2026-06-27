# Plan — Unify the Frontend Parse AST and the Runtime AST

**Status:** complete
**Date:** 2026-06-27
**Supersedes the bridge in:** `docs/plan/rust-frontend-compat-bridge.md`

## 1. Problem

There are two AST vocabularies in the codebase:

1. **`tswift_ast::Ast` / `NodeKind`** — the clean *parse AST*, built by
   `tswift-parser` and annotated by `tswift-sema`.
2. **The runtime AST** — `RuntimeAst` / a *different* `NodeKind` (in
   `tswift-frontend/src/kind.rs`), the shape `tswift-core` executes against.

Between them sits `tswift-frontend/src/compat.rs` (the "compat lowerer", ~730
lines): a structural translation inherited from the decommissioned **msf** C
frontend. It hoists names, synthesizes wrapper nodes (`Block`, `Conformance`,
`OptionalBinding`, `CaseCondition`, `EnumElementDecl`), packs modifier bits, and
plants sentinels (the `for await` `"await"` text trick). msf is gone, so this
"compatibility" layer is compatible with a ghost.

## 2. Decision

Collapse the two vocabularies into one and delete the structural lowering.

- **(A) Clean AST wins, runtime adapts.** `tswift-frontend`'s `Node`/`NodeKind`
  become a thin cursor directly over `tswift_ast::Ast`. `tswift-core` is taught
  to read the clean shapes. `compat.rs` dies.
- **(i) Incremental, structural-quirks-first.** Kill one reshape per commit,
  keeping the full suite green between commits. Collapse the enum and delete the
  arena only once nothing structural remains.
- **(b) Keep `tswift_ast` minimal.** When a runtime kind is more specific than
  the clean kind, the runtime recovers the specifics from text/children. Enrich
  `tswift_ast` only with an explicitly flagged justification.

## 3. Verification signal

`cargo test --workspace` — **baseline: 444 unit tests + 156 golden behavioral
fixtures (`tswift-cli/tests/golden.rs`) + frontend AST snapshot fixtures
(`tswift-frontend/tests/golden_fixtures.rs`)**. Must stay green after every step.

## 4. Steps

Each box = one atomic commit; full suite green before the next.

- [x] 1. **`for` hoist + await sentinel** (`lower_for`) — runtime reads the loop
      pattern child and the `async` modifier directly; delete the `"await"`
      sentinel + `token_text_offset(1)` path. *(Single eval site — loop
      validator.)*
- [x] 2. **binding name hoist** (`lower_binding`) — runtime reads the
      `NamePattern`/`WildcardPattern` child instead of the decl's hoisted text.
- [x] 3. **nominal reshape** (`lower_nominal`) — `register_*` read members and
      inherited `TypeRef`s directly; stop synthesizing the `Block` wrapper and
      `Conformance`/`TypeIdent` nodes. *(Split into 3a conformances→TypeIdent,
      3b drop Block wrapper.)*
- [x] 4. **conditional bindings** (`lower_conditional` / `lower_optional_binding`)
      — `eval_cond_list` reads the `LetDecl`/`VarDecl` + pattern; delete
      `OptionalBinding`/`CaseCondition` synthesis.
- [x] 5. **case clause** (`lower_case_clause`) — runtime reads the `WhereClause`
      child and `default` marker directly; drop `case_info` synthesis.
- [x] 6. **enum case** (`lower_enum_case`) — runtime reads the flat
      `EnumCaseDecl` + `TypeRef` children; delete the `EnumElementDecl`/`Param`
      nesting.
- [x] 7. **`#if` splice** (`lower_child_list`) — runtime expands the
      `CompilerDirective` `#if` wrapper at each decl/member/statement site; stop
      splicing in the lowerer. *(Broadest — every child-list site; done last
      among the structural steps.)*
- [x] 8. **enum collapse** — re-export `tswift_ast::NodeKind` as the frontend
      `NodeKind`, rewrite `tswift-core` match arms to the clean names, delete
      `map_kind`. Modifier bitfield stays (payload encoding, not structural).
- [x] 9. **arena deletion** — `Node` becomes a cursor straight over
      `tswift_ast`; delete `RuntimeAst`/`RuntimeNode`; rename the module away
      from "compat".

## 5. Progress log

- 2026-06-27 — plan written; baseline green (444 + goldens).
- 2026-06-27 — swapped steps 1↔2: `#if` splice touches every child-list site
  (top-level/members/block), `for` hoist is a single eval site, so `for` goes
  first as the lower-risk loop validator.
- 2026-06-27 — reordered `#if` splice to step 7 (broadest blast radius — every
  child-list site — so it runs after the localized quirks, per simple→complex).
- 2026-06-27 — step 9 done (39bfbd1). Deleted the `RuntimeAst` arena and
  `compat.rs`; `Node` is a thin cursor over `tswift_ast`, accessors compute on
  the fly, decoders in `decode.rs`. Codex flagged labeled-`for` `text()` drift +
  stale docs (Important/Minor) — both fixed. Updated AGENTS.md. 447 green.
  **All 9 steps complete: the compat lowering is gone; one AST, one vocabulary.**
- 2026-06-27 — step 8 done (91ecf64). Deleted `kind.rs`/`map_kind`; frontend
  `NodeKind` is now `tswift_ast::NodeKind`. Renamed all match arms; removed dead
  kinds (Conformance/OptionalChain/ParenExpr/Type{Optional,Inout,Func,Tuple}).
  Codex flagged the `precedence_group_decl` JSON-name break (Important) — fixed.
  447 green.
- 2026-06-27 — step 7 done (e9e0850). `#if` stays as `MacroExpansion("if")`;
  runtime `expand_directives` flattens it at hoist/eval_seq/member loops/closure
  bodies. Parser rejects `#if` between switch cases, so that path is moot. Codex
  flagged the closure-body gap (Important) — fixed + regression test. 447 green.
- 2026-06-27 — step 6 done. Enum cases read as flat `EnumCaseDecl(name)` with
  `TypeIdent` payload children / raw-value expr; deleted `lower_enum_case`.
  Verified int/str raw values + assoc destructuring. Codex: Yes. 446 green.
- 2026-06-27 — step 5 done (e4ab227). `case_info()` reads the `"default"` text
  and the `WhereClause` child directly; `case_parts()` skips the guard. Removed
  the synthesized fields + `lower_case_clause` + dead `cursor()`. Codex: Yes.
  446 green.
- 2026-06-27 — step 4 done (80dfb58). Condition bindings stay as `LetDecl`/
  `VarDecl`; `eval_cond_list` has one unified arm (optional-unwrap vs refutable
  match). Codex flagged 2 Important issues (subject self-selection; wildcard
  nil-check) — both fixed + regression tests added. 446 green.
- 2026-06-27 — step 3b done (bfac697). Removed the synthesized nominal `Block`;
  members are direct children. 5 register sites iterate direct children. Codex:
  no issues. 444 green.
- 2026-06-27 — step 3a done (38d7cf8). Conformances kept as plain `TypeIdent`
  children; runtime readers filter `TypeIdent`. Codex: Yes. 444 green.
- 2026-06-27 — step 2 done. `lower_binding` keeps the binding pattern as a child;
  `decl_name()` reads the name from the `PatternValueBinding`/`PatternWildcard`
  child. Excluded pattern nodes from `is_value_node` so the re-added child is
  never mistaken for an initializer/member default; simplified `is_expr`.
  Regenerated 3 `.ast` snapshots. Codex review: no Critical/Important. 444 green.
- 2026-06-27 — step 1 done. `lower_for` keeps the binding pattern as a child and
  records the `async` modifier; `eval_for` reads the binding from the pattern
  child and detects `for await` via `Node::is_async()`. Deleted the `"await"`
  sentinel, `for_await_binding`, and `token_text_offset`. 444 green, 0 warnings.
</content>
</invoke>
