# ADR-0001: FFI strategy and crate architecture for quick-swift

- **Status:** Accepted
- **Date:** 2026-06-24
- **Context slice:** Walking skeleton (issue #1)

## Context

quick-swift is a lightweight Swift runtime that consumes a typed AST from `msf`
(a C11 frontend: lex → parse → 3-pass sema) over FFI and implements all runtime
*behaviour* in Rust. The walking skeleton is the slice where the FFI strategy
and crate split are ratified before the rest of the runtime is built on top.

Two questions had to be settled:

1. **How do we build and bind `msf`?** It ships as ~50 `.c` files behind a single
   public header (`include/msf.h`) plus a `generated/` data-table directory. It
   has one unresolved backend symbol, `module_stub_find`, that a host must
   provide.
2. **How do we keep `unsafe` from leaking into the evaluator?** msf hands out raw
   pointers into an arena-allocated AST that lives only as long as its
   `MSFResult`. Mis-handling lifetimes is the highest FFI risk.

## Decision

### Build & bind via `cc` + `bindgen` in `msf-sys/build.rs`

- Compile every `vendor/msf/src/**/*.c` plus a local `stub.c` into
  `libMiniSwiftFrontend.a` using the **`cc`** crate (no dependency on msf's
  Makefile; cargo caches the build). Include paths mirror the Makefile:
  `include/`, `generated/`, `src/`, `src/unicode/include`, `src/unicode/src`.
- `stub.c` provides the single backend seam, `module_stub_find`, as a null
  implementation (returns `NULL`) plus the `extern MODULE_STUBS` definition the
  header requires. quick-swift drives msf with bare source and no `.msfvocab`,
  so imports simply resolve to nothing for now.
- Generate bindings with **`bindgen`** from `wrapper.h` (`#include <msf.h>`),
  using `EnumVariation::ModuleConsts` so each msf enum becomes a module of
  typed constants (e.g. `ASTNodeKind::AST_CALL_EXPR`). The anonymous
  `ASTNode`/`TypeInfo` unions bindgen emits are **verified usable** — the safe
  wrapper reads `data.integer.ival` etc. directly; no manual `repr(C)` shim was
  needed.
- `msf` is statically linked via `cc`'s emitted `rustc-link-lib=static`.

### Five-crate split with `unsafe` confined to `msf-sys` + `msf`

```
msf-sys  (raw FFI, generated, all unsafe)
  └─ msf  (safe wrapper — the ONLY place msf pointers are dereferenced)
       └─ tswift-core  (SwiftValue + eval spine, safe)
            ├─ tswift-std  (native builtins, safe)
            └─ tswift-cli  (binary)
```

The lifetime invariant — *the AST lives exactly as long as its analysis* — is
encoded in the type system: `Analysis` owns `*mut MSFResult` and frees it on
`Drop`; every `Node<'a>` borrows the `Analysis`, so the borrow checker forbids
using a node past the analysis it came from. Above the `msf` crate there is
**zero `unsafe`** (enforced by review and verified by grep in the skeleton).

## Consequences

- **Good:** no `make`/Emscripten dependency; cargo owns the whole build; the
  riskiest unknown (FFI lifetimes, union bindings) is retired in the skeleton;
  the evaluator is ordinary safe Rust.
- **Cost:** `bindgen` requires `libclang` at build time (handled in CI). The full
  msf C build is recompiled when its sources change (mitigated by cargo cache +
  `rerun-if-changed`).
- **Pinned:** msf is a git submodule under `vendor/msf`; bindings track that
  commit. Bumping msf may require regenerating/reviewing bindings.

## Notes

- `NodeKind` is a real Rust enum mapped from `ASTNodeKind`, with an `Other(u32)`
  catch-all so unmapped kinds round-trip losslessly and matches stay exhaustive.
  New milestones promote `Other` values to named variants.
