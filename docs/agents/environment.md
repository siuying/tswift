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

- Assume **no network access to crates.io** during a task unless verified.
  Adding a dependency that isn't already in `Cargo.lock` may fail to fetch —
  test with a `--dry-run` `cargo add` (or a `static.crates.io` download)
  before relying on it.
  - Example: an arbitrary/unpinned `serde_json` version is *not* guaranteed
    available in offline sessions (only whatever's already resolved in
    `Cargo.lock`'s dependency graph is cached locally) — the `Codable` JSON
    layer is the hand-written `crates/tswift-core/src/json.rs` for most of the
    workspace for this reason. `tswift-testing` is the sanctioned exception
    (see below): its exact locked `serde`/`serde_json` versions are already
    vendored transitively and pinned in the workspace manifest.
- Prefer a small self-contained module over a new dependency. If a crate is
  genuinely required, confirm it's already vendored / in the lockfile first.
  - Sanctioned exception: `ureq` (rustls HTTPS) in **tswift-cli only**, backing
    `--allow-network` (ADR-0010).

### JSON policy: serde vs the runtime `json` layer

Two JSON layers coexist, split by *what* is being encoded — not by crate:

- **`serde`/`serde_json` — static Rust wire types.** When the JSON shape is a
  fixed Rust `struct`/`enum` (a wire contract, not program data), derive
  `serde::Serialize`/`Deserialize` and go through `serde_json`. Current
  adopters: `tswift-testing` (`wire.rs` — `TestDescriptor`/`RunReport`/
  `RunOptions`) and `tswift-frontend` (`symbols::Symbol`, the `dump_json` AST
  snapshot). Pin any migration behind a schema-stability test that compares
  the parsed output against the pre-serde bytes before switching. Both crates
  use the exact `serde`/`serde_json` versions already vendored transitively
  (via `criterion`) and pinned in the workspace manifest — a genuinely
  new/unvendored `serde` pull (e.g. a different major) is still subject to the
  no-network rule above.
- **`crates/tswift-core/src/json.rs` — Swift-semantics / dynamic values.** The
  hand-written `Json` model implements Swift `JSONEncoder`/`JSONDecoder`
  behaviour for *interpreted-program* values, and stays the layer for any
  JSON whose shape is driven by runtime `SwiftValue`s (the SwiftUI UIIR trees
  in `tswift-swiftui::uiir`/`diff`, the host-bridge FFI arg/reply envelopes in
  `tswift-cli`, the interpreter's `Codable` coding, `tswift-core::http`'s
  transport codecs whose decoders need domain-specific `URLError` messages).
  Don't route these through serde: the shape is dynamic and/or the bespoke
  error text is part of the contract.

## Tooling notes

- The `edit` tool takes `oldText`/`newText` only — do **not** emit a stray
  `newText2`; it's rejected. Merge nearby changes into one edit instead.
- `requesting-code-review` is not installed; do a self-review against
  `scripts/presubmit` and the feature checklist.
- To inspect how msf parses a construct, use `tswift dump <file>` rather
  than writing throwaway AST-walker test modules. See
  `docs/research/msf-ast-cheatsheet.md`.

## Build/codegen

- `crates/msf/build.rs` generates `NodeKind` from
  `vendor/msf/generated/ast_kinds.h`; `crates/msf-sys/build.rs` compiles the
  vendored C and runs bindgen. Both require the **msf submodule checked out**
  (`git submodule update --init`).
