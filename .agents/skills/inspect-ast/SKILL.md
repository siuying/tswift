---
name: inspect-ast
description: When user or agent need to construct a swift AST, see how the Rust frontend parses a construct — kind, text, line, resolved type, and decoded modifiers.
---

To **Inspecting the AST**: run `tswift dump <file.swift>` (or `--json`) to see how the Rust frontend parses a construct — kind, text, line, resolved type, and decoded modifiers. Don't hand-write AST walkers. Pin parse shapes with `tests/fixtures/ast/*.swift` + `*.ast` snapshots.
