# Multi-file compilation

`tswift` compiles multiple Swift source files as **one module**, mirroring
`swiftc`'s single-module behavior: types, extensions, and functions declared in
one file are visible from every other file without imports.

## Usage

```sh
# Explicit file list (compiled in the given order)
tswift run main.swift models.swift views.swift

# Directory of *.swift files (sorted deterministically; main.swift is the entry)
tswift run Sources/

# Swift package subset: a directory with a root Package.swift
tswift run MyApp/ [--target <name>]
```

The same program-input model backs every entrypoint — CLI, wasm
(`runSwiftModule`), and iOS FFI (`tswift_run_module`) — via
`Analysis::analyze_program(&[SourceFile])`.

## Semantics

- **One module.** All files form a single compilation unit. Cross-file
  references (a type in `models.swift` used from `views.swift`, extensions in a
  separate file, `@main` in one file with views in others) resolve without any
  import statements.
- **Top-level code rule.** Only `main.swift` (by basename) — or the sole file
  of a single-file program — may contain top-level executable statements. A
  top-level statement in any other file is a compile error pointing at that
  file. Declarations are allowed everywhere.
- **Deterministic order.** Explicit file arguments keep their given order;
  directory mode sorts `*.swift` files by name.

## Diagnostics

Compile errors (lexer, parser, sema) report the **owning file**, with
swiftc-style formatting, a caret snippet, and a nonzero exit code:

```text
views.swift:12:7: error: cannot convert value of type 'String' to specified type 'Int'
let n: Int = "hi"
      ^
```

Columns are counted in Unicode code points and tabs are preserved in the
snippet line. In multi-file builds each diagnostic is remapped to its file and
file-local line, so an error in the third file never blames the first.

## Known limitations

These are documented tripwires, not silent gaps (see ADR-0017):

- **Runtime errors** still report combined-module line numbers (the interpreter
  walks the merged AST). Compile-time diagnostics are per-file accurate.
- **File-private isolation is not modelled.** Concatenation flattens all files
  into one scope; two files declaring the same private symbol collide.
- **Undefined-variable is a runtime error** without a source location — sema
  does not yet perform full name resolution (ADR-0020 scope).
- **Parser is first-fail.** Multiple errors in one run are collected where the
  pipeline allows, but the parser stops at its first syntax error.

## Design background

- [ADR-0017 — multi-file program input](adr/0017-multi-file-program-input.md):
  concatenation with a per-file line-offset table, and why a true multi-AST
  merge was deferred.
- [ADR-0020 — module-scoped symbol resolution](adr/0020-module-scoped-symbol-resolution.md)
- [Incremental compilation research](research/incremental-compilation.md):
  frontend time is <5% of wall time, so per-file caching is intentionally not
  implemented; a whole-program warm-start cache exists (ADR-0018).
