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
cargo test -p tswift-std dump_registered_keys
cargo test -p tswift-foundation dump_registered_keys
```

## Report coverage

```sh
python3 tools/framework-inventory/coverage.py --framework foundation
python3 tools/framework-inventory/coverage.py --framework foundation Data
python3 tools/framework-inventory/coverage.py --framework stdlib Array
```

Coverage states are `missing`, `implemented`, and `verified`. Verified means the
member is registered and mentioned by a tagged executing CLI golden fixture
(e.g. `crates/tswift-cli/tests/fixtures/foundation_*.swift`).

### Known limitation: token-based verification is not type-scoped

`implemented` is gated by the registry, whose keys *are* type-scoped
(`Type.member`). `verified`, however, is decided by scanning fixture **source
tokens**: a member counts as verified when its bare name appears as a
member-access (`.name`), call (`name(`), or operator token in any in-scope
fixture. Those tokens carry no receiver type, so a generic member name
(`init`, `body`, `map`, `description`, `count`, `hash`, …) can be credited to
*every* registered type that declares it as soon as **any** unrelated type
exercises that name in a fixture. `init` is partially guarded (it also accepts
the owning type name appearing in a fixture), but the bare-token path still
applies.

Consequently `verified` counts for common members are optimistic: they prove
the runtime executes *a* call spelled that way, not that the specific type was
exercised. Tightening this would require type-inferring each fixture token back
to a receiver type (so `verified` could demand a type-scoped match like the
registry provides) — deferred because it destabilizes the calibrated per-slice
baselines many fixtures were tuned against. Treat generic-member `verified`
numbers as an upper bound; `implemented` (registry-gated) is exact.

`tools/stdlib-inventory/{extract,coverage}.py` remain as compatibility shims.

## Emit machine-readable coverage JSON

```sh
python3 tools/framework-inventory/coverage.py --framework foundation --emit-json
python3 tools/framework-inventory/coverage.py --framework foundation --emit-json --out /tmp/foundation.json
```

Shape (per framework):

```jsonc
{
  "framework": "Foundation",
  "sections": [
    {
      "name": "Data",
      "members": [
        { "name": "append", "kind": "func", "status": "implemented" },
        { "name": "bytes", "kind": "var", "status": "missing" }
        // "notes" is present only on out_of_scope members, e.g.
        // { "name": "NSObject", "kind": "...", "status": "out_of_scope",
        //   "notes": "out of scope: objc_runtime" }
      ],
      "counts": { "implemented": 20, "partial": 0, "missing": 15, "out_of_scope": 0, "total": 35 }
    }
    // ... one entry per section in scope.toml's [[tier]] types (or, for
    // stdlib, the registry-touched types); free functions appended as a
    // "(free functions)" section when the framework tracks them.
  ],
  "totals": { "implemented": 335, "partial": 32, "missing": 248, "out_of_scope": 0, "total": 615 }
}
```

`status` is a 4-state renaming of the text report's states (`JSON_STATUS` in
`coverage.py`): the text report's `verified` (registered *and* exercised by a
golden fixture) becomes `implemented`; its `implemented` (registered but
never proven by a fixture) becomes `partial`; `missing`/`out_of_scope` are
unchanged. No new truth is invented — status is always derived from the
inventory/registry/scope.toml manifests already used by the text report.
`kind` (`func`/`var`/`let`/`case`/`init`/`subscript`) is a best-effort read of
the inventory line's declaration keyword (one representative kind per member
name; overloads collapse to their first occurrence's kind) — inventory.md has
no richer per-member metadata than that today, so this is section/member-level
granularity, not full per-overload signatures.

### Regenerate the website's coverage data

```sh
scripts/generate-coverage-json.sh          # write website/src/data/coverage/*.json + index.json
scripts/generate-coverage-json.sh --check  # drift check only (used by scripts/validate web)
```

The output is **checked in** (`website/src/data/coverage/`), same convention
as `inventory.md`/`registered_keys.txt`: the website build has no access to
the Swift toolchain/SDK `extract.py` needs and must stay buildable offline.
`coverage.py --emit-json` itself only reads already-checked-in manifests, so
regeneration works anywhere the repo is checked out, without a toolchain.
Run it after `extract.py` or `dump_registered_keys` refreshes a
framework's inventory/registry, or after editing a `scope.toml`.

Tests: `python3 tools/framework-inventory/test_coverage_json.py` (schema +
cross-checks against the real manifests, no test-framework dependency).
