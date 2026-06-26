# AGENTS.md

## Folder Structure

- `creates` - quick-swift rust packages.
- `crates/qswift-lexer`, `crates/qswift-ast`, `crates/qswift-parser`, `crates/qswift-sema` - the **pure-Rust Swift frontend** pipeline. `crates/qswift-frontend` drives it and exposes the runtime-facing typed AST (`Analysis`/`Node`/`NodeKind`) via the compatibility lowerer in `src/compat.rs`. No C, no LLVM, no codegen — just the frontend. (The former vendored `msf` C frontend has been decommissioned; see `docs/plan/rust-frontend-compat-bridge.md`.)
- `docs/plan/swift-runtime-implementation-plan.md` - overall plan
- `docs/research` - research on the (now-removed) msf C frontend and the VM. `docs/research/msf-ast-cheatsheet.md` documents the runtime-facing AST contract the Rust compat lowerer reproduces — useful background before working against the AST.
- `docs/agents/environment.md` - commit/signing conventions, offline-build constraints, tooling notes. Read before committing or adding a dependency.

## Development

- **Git Commits**: Use conventional format: <type>(<scope>): <subject> where type = feat|fix|docs|style|refactor|test|chore|perf. Subject: 50 chars max, imperative mood ("add" not "added"), no period. For small changes: one-line commit only. For complex changes: add body explaining what/why (72-char lines) and reference issues. Keep commits atomic (one logical change) and self-explanatory. Split into multiple commits if addressing different concerns.
- No "Co-Authored-By: Claude" or "Generated with" line.
- Before commit, ensure all unit tests pass.
- Prefer self-documenting code over comments.

## Feature Checklist

Read `docs/swift-runtime/feature-checklist.md`. It is the feature checklist. 

When a feature is partially or fully implemented and fully verified, update the checklist item.

Every feature have a corresponding **Golden fixture** (`tests/swift-fixtures/*.swift`) in Swift, validated against the Rust frontend by `qswift-frontend`'s `golden_fixtures` test.

Every rust change should be fully covered in tests.

## Coverage

### Stdlib coverage

To measure how much of the Swift stdlib the runtime implements (overall and
per section), use the **`stdlib-coverage`** skill
(`.agents/skills/stdlib-coverage/SKILL.md`). It walks the top-down workflow:
refresh the registry snapshot, check overall coverage, then drill into a
section. Tooling details live in `tools/stdlib-inventory/README.md`.

## Agent skills

### Subagent

If running Pi Agent, when user request subagent, use the skill `subagent`.

### Issue tracker

Issues live in GitHub Issues; use the `gh` CLI. See `docs/agents/issue-tracker.md`.

### Triage labels

Default label vocabulary (`needs-triage`, `needs-info`, `ready-for-agent`, `ready-for-human`, `wontfix`). See `docs/agents/triage-labels.md`.

### Domain docs

Single-context repo — one `CONTEXT.md` + `docs/adr/` at the root. See `docs/agents/domain.md`.
