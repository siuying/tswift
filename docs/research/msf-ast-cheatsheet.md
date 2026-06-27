# msf AST cheat-sheet (gotchas you will otherwise rediscover)

A living reference for working against msf's typed AST from the Rust side. Every
entry here cost at least one analyze-fail or wrong-output debugging cycle to
learn the first time. **Add to it whenever msf surprises you.**

## The fast way to inspect the AST

Don't hand-write a recursive walker. Use the built-in dump:

```bash
qswift dump path/to/snippet.swift          # kind, text, line, type, modifiers
qswift dump --json path/to/snippet.swift   # structured, for tooling
```

The text form prints one node per line: `Kind "text" L<line> :<ResolvedType>
[mods]`. Diagnostics (errors/warnings) go to stderr, so you still see them even
when analysis half-fails. To pin a construct's parse shape in a test, drop a
`crates/tswift-cli/tests/fixtures/ast/<name>.swift` + `<name>.ast` pair
(regenerate with `qswift dump`).

msf itself also exposes `msf_dump_text` / `msf_dump_json` / `msf_dump_sexpr` in
`vendor/msf/include/msf.h` if you ever need the frontend's own format.

## Node kinds

- `NodeKind` is **generated** from `vendor/msf/generated/ast_kinds.h` by
  `crates/msf/build.rs`. Every msf kind is already a named variant — there is no
  reason to match `Other(N)` for a known kind. `Other(u32)` only fires for a
  kind newer than the checked-out submodule.
- `AST_LET_DECL` and `AST_VAR_DECL` are distinct kinds (`LetDecl` / `VarDecl`)
  even though msf's own name string for both is `"var_decl"`.
- `IdentExpr` is msf's `unresolved_decl_ref_expr`; `MemberExpr` is
  `unresolved_dot_expr`. Names are resolved later, not at the node kind level.

## Where information actually lives

- **Resolved types collapse fixed-width integers to `Int`.** A `let x: Int8`
  node's *resolved type* (`Node::type_name`) reads `Int`, **not** `Int8`. To get
  the written width, read the `TypeIdent` annotation child's text instead. This
  bites every integer-width feature.
- **`modifiers` is a bitmask, and bits are reused across kinds.** Use
  `Node::modifier_names()` for the unambiguous global bits. Bit 22 means
  `weak`-capture on a closure capture but `borrowing` on a parameter and
  `testable` on an import — always qualify by node kind for the overlapping bits
  (see the `MOD_*` table in `msf.h` §9).
- **Some keywords are *not* in `modifiers`** and must be recovered from the
  token stream: argument labels, loop/`break` labels, and parameter ownership
  (`inout` shows up as a `TypeInout` child / `InoutExpr`, not a modifier).
- **`throws` appears two ways:** as the `MOD_THROWS` bit on the `FuncDecl` *and*
  as a `ThrowsClause` child node.

## Things msf resolves or rejects at parse time

- **`#if` / `#elseif` / `#else` / `#endif` are evaluated by the frontend.** Only
  the *active* branch survives into the AST — the runtime never sees the
  inactive ones. `#warning` / `#error` are emitted as diagnostics and leave no
  runtime node.
- **msf rejects unknown precedence groups** in `precedencegroup`/operator decls
  (e.g. referring to `AdditionPrecedence` it doesn't define) — analysis fails.
- **msf rejects stdlib protocols it doesn't model** in constraints/annotations,
  e.g. `Sequence` in a generic constraint, and **doesn't know `Set`** as a type
  name. Avoid them in fixtures, or expect an analyze error.

## Magic literals & attributes

- `#file` / `#line` / `#function` / `#column` are `MacroExpansion` nodes whose
  `text()` is the bare word (`"file"`, `"line"`, …). Get the line number from
  `Node::line()`.
- `@main`, `@propertyWrapper`, etc. are `Attribute` child nodes of the
  decl, with `text()` holding the attribute name (`"main"`,
  `"propertyWrapper"`). They are siblings of the decl's `Block`, not inside it.

## Token/line accessors

- `Node::text()` copies immediately because msf's `token_text` returns a
  thread-local buffer the next call overwrites.
- `Node::line()` reads the node's first token line from the result-owned token
  array (1-based).
