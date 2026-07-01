# Swift frontend golden fixtures

Repo-owned Swift source fixtures that specify what the tswift frontend
(`tswift-lexer` → `tswift-ast` → `tswift-parser` → `tswift-sema`, exposed through
`tswift-frontend`) must accept, reject, and how it must diagnose.

These are **our own** fixtures, authored to track the feature checklist
(`docs/swift-runtime/feature-checklist.md`). They are deliberately *not* a copy
of any upstream corpus — they live in this repo so we can evolve them freely.

## Two kinds of tests (and when to use which)

The repo has **two independent fixture corpora**. They validate different
layers of the pipeline — don't confuse them:

| | **Frontend golden fixtures** (this dir) | **Runtime golden fixtures** |
|---|---|---|
| Location | `tests/swift-fixtures/` | `crates/tswift-cli/tests/fixtures/` |
| Harness | `tswift-frontend/tests/golden_fixtures.rs` | `tswift-cli/tests/golden.rs` |
| What runs | `Analysis::analyze()` — lex → parse → sema only | `tswift run` — the full evaluator (`tswift-core` + `tswift-std`) |
| What's asserted | **Diagnostics** match inline directives | Program **stdout** matches a `.expected` sibling, byte-for-byte |
| Executes code? | No | Yes |
| Add a case by | Dropping in a `.swift` with directives | Dropping in a `.swift` + `.expected` pair |

**Use a frontend fixture when** you're validating *what the compiler accepts,
rejects, or how it diagnoses* — parser coverage, type-checking, error messages.
Negative cases (`expected-error`) live here.

**Use a runtime fixture when** you're validating *what a program produces when
executed* — evaluator semantics, stdlib behaviour, printed output. These must be
valid, runnable Swift with deterministic output.

**Use both when** you add a feature end-to-end: a frontend fixture proves the
construct is accepted/diagnosed correctly, and a runtime fixture proves it
evaluates to the right result. A construct accepted by the frontend but not yet
handled by the evaluator is exactly the gap a runtime fixture catches.

The runtime corpus also has two extra flavors (see
`crates/tswift-cli/tests/golden.rs`): `fixtures/multifile/<case>/` directories
for cross-file modules, and `fixtures/ast/<name>.swift` + `.ast` snapshots that
pin the typed-AST shape via `tswift dump`.

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
| `// oracle-gap: <reason>` | file | Valid Swift the (now-decommissioned) **C msf oracle** could not parse/type. The golden harness validates these as positives — the pure-Rust frontend must accept them with zero diagnostics. `<reason>` documents the historical gap. |
| `// frontend-gap: <reason>` | file | Valid Swift the **Rust frontend** cannot yet handle. Skipped by the harness so the corpus passes CI while the limitation is open; `<reason>` explains what is missing. |

Rules:

- A file is either positive (`expected-no-diagnostics`) or negative (one or more
  `expected-error`) — not both.
- Every positive fixture should be valid Swift 6 per TSPL.
- `oracle-gap` files are positive (validated for zero diagnostics like
  `expected-no-diagnostics`); the tag is historical documentation only.
- `frontend-gap` files are skipped entirely — use sparingly, with a reason, and
  remove the tag when the frontend catches up.

## How fixtures are consumed

- **Golden harness** — analyze each file, assert the directives hold.
- **Differential harness** — for non-`oracle-gap` files, assert the pure-Rust
  backend and the C oracle agree on diagnostics (line + category) and on a
  normalised AST dump.

See `docs/plan/replace-msf-with-rust-frontend.md` §5–6.

## Coverage

The C msf oracle is decommissioned; every fixture (including `oracle-gap`-tagged
ones) is validated against the pure-Rust frontend by
`tswift-frontend/tests/golden_fixtures.rs`. The harness prints the checked and
skipped counts on every run — only `frontend-gap` files are skipped, and each
one names the frontend limitation it is waiting on.

The `oracle-gap` tags remain as historical documentation of what the C msf could
not handle (regex literals, typed throws, key paths, concurrency, macros,
unicode operators, …) — all of which the Rust frontend now accepts.
