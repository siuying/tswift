# ADR 0017: Multi-file program input (concatenation with per-file line mapping)

Status: accepted
Date: 2026-07-11

## Context

Slice 11 needs a shared program-input model — an ordered `[{ path, source }]`
compilation unit — feeding every entrypoint (CLI, wasm, iOS FFI). Studio (the
multi-file editor foundation) depends on it. Two questions had to be answered
before designing (risk R1):

1. Does `tswift-sema` model a compilation unit as one source, or can `Analysis`
   already take multiple sources?
2. If it assumes a single unit, what is the cheapest correct v1?

## R1 investigation (verified in code, not from memory)

`tswift_frontend::Analysis::analyze(source, filename)` drives a single pipeline:
`tswift_parser::parse(&str) -> Ast` then `tswift_sema::analyze(&mut Ast)`. Both
operate on **one** `tswift_ast::Ast` produced from **one** source string. There
is no multi-AST merge API, and the parser produces exactly one `source_file`
root. So: **sema assumes a single compilation unit.** A true multi-AST merge
(parse each file, splice roots, track per-node file provenance) would be a much
larger change touching the AST arena and every `Node::line()` consumer.

Pre-existing entrypoints already "supported" multiple files by **concatenating**
sources (`CLI run <files>`, wasm `runSwiftModule`, FFI `tswift_run_module`) — but
they reported every diagnostic against the *first* file's path and used
*combined* line numbers, so a diagnostic in the second file pointed at the wrong
file and line.

## Decision

Take the blessed fallback: **concatenate the ordered files once into a single
combined source, with a per-file line-offset table**, and remap results back to
`(path, local_line)`. This is a degraded-but-correct v1:

- One lex/parse/analyze pass over the concatenation — no per-file re-lexing
  (honours the performance constraint).
- A `FileSpan { start_line, path }` table records where each file begins in the
  combined source. Every diagnostic's combined line is mapped to its owning file
  and file-local line.
- `Diagnostic` gains an additive `file: Option<String>` field. Single-file
  `analyze` sets it to the analyzed filename; existing callers that read only
  `line`/`col`/`message` are unaffected.
- New API: `Analysis::analyze_program(&[SourceFile])`, plus a public
  `SourceFile { path, source }` — the ordered, deterministic program-input unit
  shared by all entrypoints.
- Swift's top-level-code rule is enforced: only `main.swift` (by basename), or a
  single-file program, may contain top-level executable statements. A top-level
  statement in any other file yields an error diagnostic pointing at that file.
  Declarations (types, funcs, globals, imports, …) are allowed everywhere.

## Consequences

- Compile diagnostics now report the correct `file:line:col` across files.
- **Known degradation (tripwire):** *runtime* errors still surface the combined
  line via `interp.set_filename(entry)`, because the interpreter walks the merged
  AST whose `Node::line()` is a combined line. Acceptable for v1 (the slice
  targets compile-time diagnostics). Reopen for a true multi-AST/provenance model
  if runtime stack traces must be per-file accurate, or if two files sharing a
  private symbol name must stay isolated (concatenation flattens them into one
  scope — Swift's real file-private/internal boundaries are not modelled).
- Deterministic order: `tswift run <dir>` sorts `*.swift` files; explicit file
  args keep their given order. `main.swift` is the entry.
- Additive across the ABI: `tswift_run_module` / `runSwiftModule` /
  `TSwiftModule` already existed and were re-routed through `analyze_program`;
  single-source `tswift_run` / `runSwift` / `analyze` are untouched.
