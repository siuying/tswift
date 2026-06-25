# quick-swift

A lightweight **Swift runtime** written in Rust.

`quick-swift` runs Swift source code without a Swift toolchain, LLVM, or codegen.
It parses Swift with [`msf`](https://github.com/toprakdeviren/msf) — a single-header
C frontend that does lexing → parsing → 3-pass semantic analysis and emits a fully
typed AST — then implements **all runtime behaviour** (language semantics *and* the
standard library) in safe Rust on top of that AST.

```sh
echo 'print("hello, swift")' > hello.swift
cargo run -p quick-swift-cli -- run hello.swift   # => hello, swift
```

---

## What is it?

A tree-walking interpreter for Swift. The split of responsibilities is deliberate:

- **msf (C library)** owns the *frontend*: lexing, parsing, and a 3-pass typechecker.
  We treat it as a black box that produces a typed AST. We do **not** write a Swift
  parser or typechecker.
- **quick-swift (Rust)** owns the *runtime*:
  - **(a) Language features** — the evaluator/semantics: values, control flow, types,
    generics, ARC, closures, errors, …
  - **(b) Standard library** — the *behaviour* of `Int` / `String` / `Array` /
    `Dictionary` / `Optional` / protocols / etc. msf gives us type *shapes*; Rust
    supplies the behaviour.

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

We get memory safety for free in the evaluator and confine `unsafe` to the thin FFI
layer that walks msf's AST.

> **Status:** actively developed, well past the initial skeleton. Tiers 0–5 of the
> Swift feature surface (literals, control flow, value/reference types, ARC, protocols,
> generics, error handling, `Codable`, …) are substantially implemented and covered by
> 47+ golden fixtures. Concurrency (Tier 7) and macros (Tier 8) and a bytecode VM for
> speed (Tier 6) are future work. See
> [`docs/swift-runtime/feature-checklist.md`](docs/swift-runtime/feature-checklist.md)
> for the per-feature status.

---

## How it works

```
 Swift source
     │
     ▼
┌──────────────┐   FFI    ┌───────────────────────────────────────────┐
│ msf (C lib)  │ ───────▶ │ Rust runtime (quick-swift)                │
│ lex→parse→   │  raw     │  msf-sys → msf (safe) → core → std → cli    │
│ sema (typed  │  ptrs    │  language features + standard library      │
│ AST)         │          │  ARC=Rc · CoW=make_mut · safe evaluation    │
└──────────────┘          └───────────────────────────────────────────┘
```

1. The CLI reads one or more `.swift` files (multiple files are concatenated into one
   module so cross-file references resolve).
2. `Analysis::analyze` calls into msf over FFI to produce a typed AST.
3. The interpreter walks the AST node-by-node (`eval(node, env) -> Completion`),
   resolving identifiers through its own lexical scope chain (msf leaves identifiers
   unresolved), evaluating expressions into `SwiftValue`s, and streaming `print` output
   to stdout.

### Workspace layout

| Crate | Role | `unsafe`? |
|---|---|---|
| [`crates/msf-sys`](crates/msf-sys) | Raw FFI to msf (`build.rs`: `cc` compiles msf + `stub.c`, `bindgen` generates bindings) | yes (generated) |
| [`crates/msf`](crates/msf) | Safe wrapper: `Analysis`, `Node`, `NodeKind` — owns the AST lifetime via `Drop` | confined here |
| [`crates/quick-swift-core`](crates/quick-swift-core) | Evaluator spine: `SwiftValue`, `env`, `interp`, operators, native seam | no |
| [`crates/quick-swift-std`](crates/quick-swift-std) | Native standard library builtins (e.g. `print`) | no |
| [`crates/quick-swift-cli`](crates/quick-swift-cli) | The `quick-swift` binary | no |

The key FFI invariant — *the AST lives exactly as long as its `Analysis`* — is enforced
by the borrow checker: `Node<'a>` borrows its `Analysis`, so nodes can never outlive the
arena that owns them. Everything above `msf-sys` is safe Rust. See
[`docs/adr/0001-ffi-strategy-and-crate-architecture.md`](docs/adr/0001-ffi-strategy-and-crate-architecture.md)
and the [implementation plan](docs/plan/swift-runtime-implementation-plan.md) for the
full architecture and rationale.

---

## Building & running

### Prerequisites

- **Rust** (stable, edition 2021) — install via [rustup](https://rustup.rs).
- **A C compiler** — to compile msf (Clang on macOS, GCC/Clang on Linux).
- **`libclang`** — `bindgen` needs it at build time.
  - macOS: preinstalled with the Xcode Command Line Tools (`xcode-select --install`).
  - Debian/Ubuntu: `apt-get install libclang-dev`.

### Build

```sh
git clone https://github.com/siuying/quick-swift
cd quick-swift
git submodule update --init     # fetch vendor/msf
cargo build
```

### Run a Swift file

```sh
echo 'print(42)' > hello.swift
cargo run -p quick-swift-cli -- run hello.swift     # => 42

# multiple files form a single module
cargo run -p quick-swift-cli -- run a.swift b.swift
```

Or install the binary and call it directly:

```sh
cargo install --path crates/quick-swift-cli
quick-swift run hello.swift
```

---

## Testing

```sh
cargo test          # unit tests + golden-fixture tests
./scripts/presubmit # the full pre-commit check
```

The primary test mechanism is **golden fixtures**. They live in
[`crates/quick-swift-cli/tests/fixtures/`](crates/quick-swift-cli/tests/fixtures) as
`<name>.swift` + `<name>.expected` pairs; the harness runs each through the CLI and
diffs stdout. Adding a feature means adding a fixture pair — no harness changes needed.
Where a `swiftc` toolchain is available, fixtures are validated against real Swift
output as the ground truth.

Per-crate Rust unit tests additionally cover ARC counts (`Rc::strong_count`), CoW
uniqueness, value-copy semantics, pattern matching, and the FFI accessors.

---

## Documentation

- [`docs/plan/swift-runtime-implementation-plan.md`](docs/plan/swift-runtime-implementation-plan.md)
  — architecture, milestones (R0–R6+), dependency/Swift-compatibility notes.
- [`docs/swift-runtime/feature-checklist.md`](docs/swift-runtime/feature-checklist.md)
  — the full Swift 6.3 feature surface with per-feature frontend/runtime/phase status.
- [`docs/adr/`](docs/adr) — architectural decision records.
- [`docs/research/`](docs/research) — background research on msf and VM design.

---

## License

MIT.
