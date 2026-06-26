# Swift frontend golden fixtures

Repo-owned Swift source fixtures that specify what the quick-swift frontend
(`qswift-lexer` → `qswift-ast` → `qswift-parser` → `qswift-sema`, exposed through
`qswift-frontend`) must accept, reject, and how it must diagnose.

These are **our own** fixtures, authored to track the feature checklist
(`docs/swift-runtime/feature-checklist.md`). They are deliberately *not* a copy
of any upstream corpus — they live in this repo so we can evolve them freely.

## Layout

One directory per checklist tier; one or more `.swift` files per feature group:

```
tests/swift-fixtures/
├── tier0-lexical/            # literals, operators, identifiers, comments
├── tier1-imperative/         # bindings, functions, control flow, switch
├── tier2-value-types/        # struct, enum, optional, subscript, properties
├── tier3-reference-arc/      # class, ARC, init, casting, closures
├── tier4-protocols-generics/ # protocols, generics, extensions, synthesis
├── tier5-errors-modules/     # throws/try/defer, property wrappers, #if, @main
├── tier6-advanced-types/     # some/any, metatypes, key paths
├── tier7-concurrency/        # async/await/actor (frontend gap, F8+)
├── tier8-macros/             # macros / result builders (frontend gap, F8+)
├── tier9-attributes-operators/ # access control, custom operators, attrs, directives
└── tier10-stdlib/            # stdlib surface (collections, strings, core)
```

## Directive language

A fixture is a normal Swift file annotated with comment directives the harness
reads:

| Directive | Scope | Meaning |
|---|---|---|
| `// expected-no-diagnostics` | file | The whole file must analyze with **zero** diagnostics. Put it on the first line. |
| `// expected-error{{substring}}` | line | The line it appears on must produce a diagnostic whose message **contains** `substring`. Matching is an asymmetric substring test, so exact compiler wording can differ across backends. |
| `// oracle-gap: <reason>` | file | Valid Swift that the vendored **C msf oracle** cannot parse/type. The differential harness **skips** the oracle for these files; the pure-Rust frontend's job is to accept them. `<reason>` explains the gap. |

Rules:

- A file is either positive (`expected-no-diagnostics`) or negative (one or more
  `expected-error`) — not both.
- Every positive fixture should be valid Swift 6 per TSPL.
- `oracle-gap` files are positive (no diagnostics expected) but excluded from the
  C-oracle differential comparison.

## How fixtures are consumed

- **Golden harness** — analyze each file, assert the directives hold.
- **Differential harness** — for non-`oracle-gap` files, assert the pure-Rust
  backend and the C oracle agree on diagnostics (line + category) and on a
  normalised AST dump.

See `docs/plan/replace-msf-with-rust-frontend.md` §5–6.

## Coverage and oracle gaps

The corpus has **43 fixtures** across all 11 tiers: 30 positive, 4 negative, and 9
`oracle-gap`. Every positive and negative fixture was validated against the current
C msf oracle (positives produce zero diagnostics; negatives produce the expected one).

The 9 `oracle-gap` files are valid Swift that the C msf cannot handle — concrete
targets the pure-Rust frontend must do better. They are excluded from the C-oracle
differential:

| Fixture | Gap the C msf has |
|---|---|
| `tier0-lexical/regex_literals.swift` | does not lex regex literals `/.../`, `#/.../#` |
| `tier1-imperative/for_case_optional.swift` | rejects the `?` optional pattern in `for case` |
| `tier4-protocols-generics/generics.swift` | does not resolve a generic parameter (`Element`) in its type's member signatures |
| `tier4-protocols-generics/extensions.swift` | does not resolve a protocol associated type (`Collection.Element`) in an extension |
| `tier5-errors-modules/typed_throws.swift` | does not parse Swift 6 typed throws `throws(E)` |
| `tier6-advanced-types/key_paths.swift` | does not parse key-path expressions `\Root.path` |
| `tier7-concurrency/concurrency.swift` | does not fully parse/type async/await/actor (F8+) |
| `tier8-macros/macros.swift` | does not parse macro declarations or result-builder transforms (F8+) |
| `tier9-attributes-operators/unicode_operators.swift` | does not lex unicode operator characters (`√`, `°`) |

> The C msf is also more permissive than Swift in places (e.g. it does not enforce
> missing-`try`, undeclared-symbol use, or many type mismatches at top level), so the
> negative fixtures deliberately target rules msf **does** detect (assign-to-`let`,
> stored property in an extension, unterminated string) — keeping them differential-safe
> until the Rust frontend tightens these up.
