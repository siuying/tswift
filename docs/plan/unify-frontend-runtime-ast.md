# Plan — Unify the Frontend Parse AST and the Runtime AST

**Status:** in progress
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
      validator; reordered ahead of `#if` which touches every child-list site.)*
- [ ] 2. **`#if` splice** (`lower_child_list`) — runtime expands the
      `CompilerDirective` `#if` wrapper at each decl/member/statement site; stop
      splicing in the lowerer.
- [ ] 3. **binding name hoist** (`lower_binding`) — runtime reads the
      `NamePattern`/`WildcardPattern` child instead of the decl's hoisted text.
- [ ] 4. **nominal reshape** (`lower_nominal`) — `register_*` read members and
      inherited `TypeRef`s directly; stop synthesizing the `Block` wrapper and
      `Conformance`/`TypeIdent` nodes.
- [ ] 5. **conditional bindings** (`lower_conditional` / `lower_optional_binding`)
      — `eval_cond_list` reads the `LetDecl`/`VarDecl` + pattern; delete
      `OptionalBinding`/`CaseCondition` synthesis.
- [ ] 6. **case clause** (`lower_case_clause`) — runtime reads the `WhereClause`
      child and `default` marker directly; drop `case_info` synthesis.
- [ ] 7. **enum case** (`lower_enum_case`) — runtime reads the flat
      `EnumCaseDecl` + `TypeRef` children; delete the `EnumElementDecl`/`Param`
      nesting.
- [ ] 8. **enum collapse** — re-export `tswift_ast::NodeKind` as the frontend
      `NodeKind`, rewrite `tswift-core` match arms to the clean names, delete
      `map_kind`. Modifier bitfield stays (payload encoding, not structural).
- [ ] 9. **arena deletion** — `Node` becomes a cursor straight over
      `tswift_ast`; delete `RuntimeAst`/`RuntimeNode`; rename the module away
      from "compat".

## 5. Progress log

- 2026-06-27 — plan written; baseline green (444 + goldens).
- 2026-06-27 — swapped steps 1↔2: `#if` splice touches every child-list site
  (top-level/members/block), `for` hoist is a single eval site, so `for` goes
  first as the lower-risk loop validator.
- 2026-06-27 — step 1 done. `lower_for` keeps the binding pattern as a child and
  records the `async` modifier; `eval_for` reads the binding from the pattern
  child and detects `for await` via `Node::is_async()`. Deleted the `"await"`
  sentinel, `for_await_binding`, and `token_text_offset`. 444 green, 0 warnings.
</content>
</invoke>
