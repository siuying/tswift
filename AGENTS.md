# AGENTS.md

This project aimed to build a end-to-end, lightweight Swift compiler and runtime in Rust.

## Folder Structure

- `crates` - tswift rust packages.
- `crates/tswift-lexer`, `crates/tswift-ast`, `crates/tswift-parser`, `crates/tswift-sema` - the **pure-Rust Swift frontend** pipeline. `crates/tswift-frontend` drives it and exposes the runtime-facing typed AST (`Analysis`/`Node`/`NodeKind`), where `Node` is a thin cursor straight over the `tswift_ast` parse AST (one shared `NodeKind` vocabulary; `src/decode.rs` decodes modifier/literal payloads). No C, no LLVM, no codegen — just the frontend. (The former vendored `msf` C frontend has been decommissioned, and the compatibility lowerer it required has been removed; see `docs/plan/unify-frontend-runtime-ast.md`.)

## Notable Documents

- `README.md` - overview of the project.
- `docs/swift-runtime/feature-checklist.md` - high level checklist of features we want to implement.
- `docs/swift-runtime/stdlib-inventory.md` - complete standard library interface of Swift.
- `docs/plan/swift-runtime-implementation-plan.md` - overall plan
- `docs/research` - research on the (now-removed) msf C frontend and the VM. `docs/research/msf-ast-cheatsheet.md` documents the historical runtime-facing AST contract — useful background, though the runtime now consumes the `tswift_ast` parse AST directly.
- `docs/agents/environment.md` - commit/signing conventions, offline-build constraints, tooling notes. Read before committing or adding a dependency.

## Development

- Read `CODING_STANDARD.md` before writing code.
- **Git Commits**: Use conventional format: <type>(<scope>): <subject> where type = feat|fix|docs|style|refactor|test|chore|perf. Subject: 50 chars max, imperative mood ("add" not "added"), no period. For small changes: one-line commit only. For complex changes: add body explaining what/why (72-char lines) and reference issues. Keep commits atomic (one logical change) and self-explanatory. Split into multiple commits if addressing different concerns.
- No "Co-Authored-By: Claude" or "Generated with" line.

## Coverage

### Stdlib coverage

To measure how much of the Swift stdlib the runtime implements (overall and
per section), use the **`stdlib-coverage`** skill.

## Agent skills

### Subagent

If running Pi Agent, when user request subagent, use the skill `subagent`.

### Issue tracker

Issues live in GitHub Issues; use the `gh` CLI. See `docs/agents/issue-tracker.md`.

### Triage labels

Default label vocabulary (`needs-triage`, `needs-info`, `ready-for-agent`, `ready-for-human`, `wontfix`). See `docs/agents/triage-labels.md`.

### Domain docs

Single-context repo — one `CONTEXT.md` + `docs/adr/` at the root. See `docs/agents/domain.md`.
