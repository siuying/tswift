# Environment & commit conventions (read before committing or adding deps)

Practical constraints an agent hits in this repo. None are obvious from the code;
all cost time to rediscover.

## Commits & signing

- **Commit signing is unreliable here** (the 1Password SSH signing agent fails
  in non-interactive sessions). Commit with `--no-gpg-sign`:

  ```bash
  git commit --no-gpg-sign -m "feat(scope): subject"
  ```

- The git remote uses **HTTPS** (via the `gh` token), not SSH — SSH auth was
  also unreliable. Don't switch the remote back to SSH.
- Follow the conventional-commit format from `AGENTS.md`
  (`<type>(<scope>): <subject>`, imperative, ≤50 chars). No `Co-Authored-By` or
  "Generated with" trailers.
- Run `scripts/presubmit` (fmt + clippy + test) before every commit; it must be
  green.

## Dependencies / offline builds

- Assume **no network access to crates.io** during a task. Adding a dependency
  that isn't already in `Cargo.lock` will fail to fetch.
  - Example: `serde_json` is *not* available — the `Codable` JSON layer is the
    hand-written `crates/tswift-core/src/json.rs` for this reason.
- Prefer a small self-contained module over a new dependency. If a crate is
  genuinely required, confirm it's already vendored / in the lockfile first.

## Tooling notes

- The `edit` tool takes `oldText`/`newText` only — do **not** emit a stray
  `newText2`; it's rejected. Merge nearby changes into one edit instead.
- `request-code-review` is not installed; do a self-review against
  `scripts/presubmit` and the feature checklist.
- To inspect how msf parses a construct, use `tswift dump <file>` rather
  than writing throwaway AST-walker test modules. See
  `docs/research/msf-ast-cheatsheet.md`.

## Build/codegen

- `crates/msf/build.rs` generates `NodeKind` from
  `vendor/msf/generated/ast_kinds.h`; `crates/msf-sys/build.rs` compiles the
  vendored C and runs bindgen. Both require the **msf submodule checked out**
  (`git submodule update --init`).
