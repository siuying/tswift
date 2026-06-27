# framework inventory & coverage tooling

Generalized API surface tooling for the Swift stdlib, Foundation, and future
frameworks. See `docs/plan/framework-support.md`.

## Extract an inventory

```sh
python3 tools/framework-inventory/extract.py --framework foundation \
  > frameworks/foundation/inventory.md
python3 tools/framework-inventory/extract.py --framework stdlib \
  > docs/swift-runtime/stdlib-inventory.md
```

Framework source paths live in `tools/framework-inventory/frameworks.toml` and
resolve through the pinned Swift toolchain or `xcrun --show-sdk-path`.

## Refresh registry keys

Each runtime crate dumps its live registry:

```sh
cargo test -p qswift-std dump_registered_keys
cargo test -p qswift-foundation dump_registered_keys
```

## Report coverage

```sh
python3 tools/framework-inventory/coverage.py --framework foundation
python3 tools/framework-inventory/coverage.py --framework foundation Data
python3 tools/framework-inventory/coverage.py --framework stdlib Array
```

Coverage states are `missing`, `implemented`, and `verified`. Verified means the
member is registered and mentioned by a tagged executing CLI golden fixture
(e.g. `crates/qswift-cli/tests/fixtures/foundation_*.swift`).

`tools/stdlib-inventory/{extract,coverage}.py` remain as compatibility shims.
