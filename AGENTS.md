# AGENTS.md

## Folder Structure

- `creates` - quick-swift rust packages.
- `vendor/msf` - Mini Swift Frontend. A single-header C library that takes Swift source code and produces a fully typed abstract syntax tree. No LLVM, no codegen, no runtime — just the frontend.
- `docs/plan/swift-runtime-implementation-plan.md` - overall plan
- `docs/research` - research on msf and VM

## Development

- **Git Commits**: Use conventional format: <type>(<scope>): <subject> where type = feat|fix|docs|style|refactor|test|chore|perf. Subject: 50 chars max, imperative mood ("add" not "added"), no period. For small changes: one-line commit only. For complex changes: add body explaining what/why (72-char lines) and reference issues. Keep commits atomic (one logical change) and self-explanatory. Split into multiple commits if addressing different concerns.
- No "Co-Authored-By: Claude" or "Generated with" line.
- Before commit, ensure all unit tests pass.

## Feature Checklist

Read `docs/swift-runtime/feature-checklist.md`. It is the feature checklist. 

When a feature is partially or fully implemented and fully verified, update the checklist item.

Every feature have a corresponding **Golden fixtures** (tests/fixtures/*.swift) in swift.

Every rust change should be fully covered in tests.

## Agent skills

### Issue tracker

Issues live in GitHub Issues; use the `gh` CLI. See `docs/agents/issue-tracker.md`.

### Triage labels

Default label vocabulary (`needs-triage`, `needs-info`, `ready-for-agent`, `ready-for-human`, `wontfix`). See `docs/agents/triage-labels.md`.

### Domain docs

Single-context repo — one `CONTEXT.md` + `docs/adr/` at the root. See `docs/agents/domain.md`.
