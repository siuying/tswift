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
- **Hard rules** (details in `docs/agents/environment.md`):
  - Run `scripts/presubmit` (fmt + clippy + test) before every commit; it must be green.
  - Commit with `git commit --no-gpg-sign` (the signing agent fails in non-interactive sessions).
  - Assume no network access to crates.io: never add a dependency that isn't already in `Cargo.lock`. Prefer a small self-contained module.
- **Git Commits**: Use conventional format: <type>(<scope>): <subject> where type = feat|fix|docs|style|refactor|test|chore|perf. Subject: 50 chars max, imperative mood ("add" not "added"), no period. For small changes: one-line commit only. For complex changes: add body explaining what/why (72-char lines) and reference issues. Keep commits atomic (one logical change) and self-explanatory. Split into multiple commits if addressing different concerns.
- No "Co-Authored-By: Claude" or "Generated with" line.

## Coverage

### Stdlib coverage

To measure how much of the Swift stdlib the runtime implements (overall and
per section), use the **`stdlib-coverage`** skill.

## Agent skills

### Subagent

If running Pi Agent, when user request subagent, use the skill (`/subagent`).
If running Codex/Claude Code, when user request Pi subagent, use the skill (`/pi-subagent`).

### Autoloop

When user request autoloop, survey, plan and confirm first before enter the loop.

### Issue tracker & triage

Issues live in GitHub Issues; use the `gh` CLI. See `docs/agents/issue-tracker.md`. Label vocabulary (`needs-triage`, `needs-info`, `ready-for-agent`, `ready-for-human`, `wontfix`): see `docs/agents/triage-labels.md`.

### Inspecting the AST

When constructing or inspecting Swift AST shapes, use the **`inspect-ast`** skill (`tswift dump <file.swift>`) instead of hand-reading parser code.

### Domain docs

Single-context repo — one `CONTEXT.md` + `docs/adr/` at the root. See `docs/agents/domain.md`.
