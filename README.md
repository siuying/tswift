# quick-swift

A lightweight **Swift runtime** in Rust. It parses Swift with
[`msf`](https://github.com/toprakdeviren/msf) (a C frontend: lex → parse → sema)
and implements all runtime *behaviour* — language semantics and the standard
library — in safe Rust over that typed AST.

> Status: **walking skeleton** (R0). The thinnest end-to-end path is wired up:
> `quick-swift run hello.swift` where `hello.swift` is `print(42)` prints `42`.

## Quick start

```sh
git submodule update --init        # fetch vendor/msf
cargo build
echo 'print(42)' > hello.swift
cargo run -p quick-swift-cli -- run hello.swift   # => 42
```

`bindgen` needs `libclang` at build time (preinstalled with Xcode CLT on macOS;
`apt-get install libclang-dev` on Debian/Ubuntu).

## Workspace layout

| Crate | Role | `unsafe`? |
|---|---|---|
| `crates/msf-sys` | Raw FFI to msf (`build.rs`: `cc` + `bindgen` + `stub.c`) | yes (generated) |
| `crates/msf` | Safe wrapper: `Analysis`, `Node`, `NodeKind` — owns AST lifetime | confined here |
| `crates/quick-swift-core` | Evaluator spine: `SwiftValue`, `eval`, native seam | no |
| `crates/quick-swift-std` | Native standard library (`print` for now) | no |
| `crates/quick-swift-cli` | The `quick-swift` binary | no |

The FFI lifetime invariant (the AST lives exactly as long as its analysis) is
enforced by the borrow checker: `Node<'a>` borrows its `Analysis`. See
[`docs/adr/0001`](docs/adr/0001-ffi-strategy-and-crate-architecture.md) and the
[implementation plan](docs/plan/swift-runtime-implementation-plan.md).

## Testing

```sh
cargo test            # unit + golden-fixture tests
```

Golden fixtures live in `crates/quick-swift-cli/tests/fixtures/` as
`<name>.swift` + `<name>.expected` pairs; the harness runs the CLI and diffs
stdout. Add a feature → add a pair, no code changes.
