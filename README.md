# tswift

A lightweight **Swift runtime** written in Rust.

`tswift` runs Swift source code without a Swift toolchain, LLVM, codegen, or
any C dependency. It parses Swift with a **frontend** (`tswift-lexer` →
`tswift-parser` → `tswift-sema`) and implements **runtime** (language
semantics *and* the standard library) in safe Rust on top of that AST.

```sh
echo 'print("hello, swift")' > hello.swift
cargo run -p tswift-cli -- run hello.swift   # => hello, swift
```

---

## What is it?

A tree-walking interpreter for Swift. The split of responsibilities is deliberate:

- **frontend** owns lexing, parsing, and semantic analysis: `tswift-lexer`
  → `tswift-ast` → `tswift-parser` → `tswift-sema`. Results are lowered through
  `tswift-frontend::compat` into the stable runtime-facing AST (`Analysis` /
  `Node` / `NodeKind`). No C, no LLVM, no `unsafe`.
- **tswift** owns the *runtime*:
  - **(a) Language features** — the evaluator/semantics: values, control flow, types,
    generics, ARC, closures, errors, concurrency, …
  - **(b) Standard library** — the *behaviour* of `Int` / `String` / `Array` /
    `Dictionary` / `Optional` / protocols / etc. The frontend gives us type
    *shapes*; Rust supplies the behaviour.

### Why Rust?

Rust's ownership model maps onto Swift's semantics unusually well, so most of Swift's
hard memory semantics become native Rust idioms rather than features we re-implement:

| Swift semantic | Rust mechanism |
|---|---|
| Value-type copy-on-write | `Rc::clone` + `Rc::make_mut` → automatic CoW |
| `class` reference semantics + ARC | `Rc<RefCell<Object>>`; retain = clone, release = drop |
| Deterministic `deinit` | Rust `Drop` fires at strong-count 0 |
| `weak` (zeroing) | `rc::Weak`; `.upgrade()` → `None` after dealloc |
| Overflow trap `+` / wrapping `&+` | `checked_add` (trap) / `wrapping_add` |
| UTF-8 `String` backing (Swift 5+) | Rust `String` |

We get memory safety for free in the evaluator and there is no `unsafe` code anywhere
in the stack (`tswift-frontend` is `#![forbid(unsafe_code)]`).

> **Status:** actively developed. Tiers 0–7 of the Swift feature surface (literals,
> control flow, value/reference types, ARC, protocols, generics, error handling,
> `Codable`, async/await, actors, task groups) are substantially implemented and
> covered by 53+ golden fixtures. Macros (Tier 8) and a bytecode VM for speed
> (Tier 6) are future work. See
> [`docs/swift-runtime/feature-checklist.md`](docs/swift-runtime/feature-checklist.md)
> for the per-feature status.

---

## How it works

```
 Swift source
     │
     ▼
┌────────────────────────────────┐
│ Frontend                       │
│  tswift-lexer                   │
│    → tswift-ast                 │
│    → tswift-parser              │
│    → tswift-sema                │
│    → tswift-frontend      │
│      (compat lowerer → AST)    │
└────────────────┬───────────────┘
                 │ Analysis / Node / NodeKind
                 ▼
┌────────────────────────────────┐
│ Runtime (tswift)               │
│  core → std → cli              │
│  language features +           │
│  standard library              │
│  ARC=Rc · CoW=make_mut         │
└────────────────────────────────┘
```

1. The CLI reads one or more `.swift` files (multiple files are concatenated into one
   module so cross-file references resolve).
2. `Analysis::analyze` runs the pure-Rust pipeline to produce a typed AST.
3. The interpreter walks the AST node-by-node (`eval(node, env) -> Completion`),
   resolving identifiers through its own lexical scope chain, evaluating expressions
   into `SwiftValue`s, and streaming `print` output to stdout.

### Workspace layout

| Crate | Role |
|---|---|
| [`crates/tswift-lexer`](crates/tswift-lexer) | Tokenizer for Swift source |
| [`crates/tswift-ast`](crates/tswift-ast) | AST node definitions |
| [`crates/tswift-parser`](crates/tswift-parser) | Recursive-descent parser |
| [`crates/tswift-sema`](crates/tswift-sema) | Semantic analysis / type resolution |
| [`crates/tswift-frontend`](crates/tswift-frontend) | Compat lowerer: drives the pipeline, exposes `Analysis`/`Node`/`NodeKind` to the runtime |
| [`crates/tswift-core`](crates/tswift-core) | Evaluator spine: `SwiftValue`, `env`, `interp`, operators, native seam |
| [`crates/tswift-std`](crates/tswift-std) | Native standard library builtins (e.g. `print`) |
| [`crates/tswift-cli`](crates/tswift-cli) | The `tswift` binary |

---

## Building & running

### Prerequisites

- **Rust** (stable, edition 2021) — install via [rustup](https://rustup.rs).

No C compiler, no `libclang`, no submodules required.

### Build

```sh
git clone https://github.com/siuying/tswift
cd tswift
cargo build
```

### Run a Swift file

```sh
echo 'print(42)' > hello.swift
cargo run -p tswift-cli -- run hello.swift     # => 42

# multiple files form a single module
cargo run -p tswift-cli -- run a.swift b.swift
```

Or install the binary and call it directly:

```sh
cargo install --path crates/tswift-cli
tswift run hello.swift
```

---

## Testing

```sh
cargo test          # unit tests + golden-fixture tests
./scripts/presubmit # the full pre-commit check
```

The primary test mechanism is **golden fixtures**. They live in
[`crates/tswift-cli/tests/fixtures/`](crates/tswift-cli/tests/fixtures) as
`<name>.swift` + `<name>.expected` pairs; the harness runs each through the CLI and
diffs stdout. Adding a feature means adding a fixture pair — no harness changes needed.
Where a `swiftc` toolchain is available, fixtures are validated against real Swift
output as the ground truth.

Per-crate Rust unit tests additionally cover ARC counts (`Rc::strong_count`), CoW
uniqueness, value-copy semantics, pattern matching, and the AST accessors.

---

## Documentation

- [`docs/plan/swift-runtime-implementation-plan.md`](docs/plan/swift-runtime-implementation-plan.md)
  — architecture, milestones (R0–R6+), dependency/Swift-compatibility notes.
- [`docs/swift-runtime/feature-checklist.md`](docs/swift-runtime/feature-checklist.md)
  — the full Swift 6.3 feature surface with per-feature frontend/runtime/phase status.
- [`docs/adr/`](docs/adr) — architectural decision records.
- [`docs/research/`](docs/research) — background research on the frontend and VM design.

---

## License

MIT.
