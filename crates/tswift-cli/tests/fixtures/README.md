# Runtime golden fixtures

Repo-owned Swift programs that specify what the quick-swift **runtime** (the
`tswift-core` evaluator + `tswift-std` builtins, driven by `qswift run`) must
*produce when executed*. The harness is `crates/tswift-cli/tests/golden.rs`.

This is the runtime counterpart to the **frontend** golden corpus in
`tests/swift-fixtures/`. See that directory's README for the full comparison of
the two test types and when to use each. In short:

- **Frontend fixtures** (`tests/swift-fixtures/`) assert *diagnostics* — what the
  compiler accepts, rejects, and how it errors. They never execute code.
- **Runtime fixtures** (here) assert *output* — they run the program and diff
  stdout. They must be valid, runnable Swift with deterministic output.

Add a feature end-to-end with both: a frontend fixture for acceptance/diagnosis,
a runtime fixture for the evaluated result.

## Layout

| Shape | What it is | Asserted against |
|---|---|---|
| `<name>.swift` + `<name>.expected` | A single program | `qswift run <name>.swift` stdout matches `.expected`, byte-for-byte |
| `multifile/<case>/*.swift` + `expected.txt` | A multi-file module | All `.swift` files (sorted) passed to one `qswift run`; stdout matches `expected.txt` |
| `ast/<name>.swift` + `<name>.ast` | An AST snapshot | `qswift dump <name>.swift` stdout matches `.ast`, byte-for-byte |

## Adding a fixture

Drop in the files — no code changes needed; the harness discovers them.

- Keep output **deterministic** (no timestamps, addresses, or unordered-set
  iteration order).
- Every `.swift` must have its sibling (`.expected`, `expected.txt`, or `.ast`),
  or the harness fails.
