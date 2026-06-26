# stdlib inventory & coverage tooling

Three artifacts keep standard-library coverage measurable against the reference
toolchain (Swift 6.3.2). See `docs/plan/stdlib-support.md` §2 and §4.2.

## 1. API inventory — `extract.py`

Extracts the public surface from the reference `.swiftinterface` into
`docs/swift-runtime/stdlib-inventory.md` (192 types, 99 free functions).

```sh
F=~/Library/Developer/Toolchains/swift-6.3.2-RELEASE.xctoolchain/usr/lib/swift/macosx/Swift.swiftmodule/arm64-apple-macos.swiftinterface
python3 tools/stdlib-inventory/extract.py "$F" > docs/swift-runtime/stdlib-inventory.md
```

## 2. Registry keys — `registered_keys.txt`

The authoritative set of entries registered by `qswift-std`, read from the live
registry so it cannot drift from the registration code. Regenerate with:

```sh
cargo test -p qswift-std dump_registered_keys
```

This runs `qswift_std::registered_keys()` and writes the sorted keys here. Keys
are `print` (free function), `Array.append` (method/property intrinsic), or
`Sequence.map` (algorithm-layer member).

## 3. Coverage report — `coverage.py`

Cross-references the inventory against two signals and assigns each member one of
three states:

- **missing** — not in the registry,
- **implemented** — in the registry,
- **verified** — in the registry *and* used by an executing CLI golden fixture
  (`crates/qswift-cli/tests/fixtures/*.swift`), i.e. behaviourally proven.

Two levels of detail:

```sh
# List every targeted section with impl/verified/total counts + overall %.
python3 tools/stdlib-inventory/coverage.py
python3 tools/stdlib-inventory/coverage.py --all      # incl. 0%-coverage sections

# Detail one section, member-by-member (verified / implemented / missing).
python3 tools/stdlib-inventory/coverage.py Array
python3 tools/stdlib-inventory/coverage.py "free functions"
```

Section names match the inventory headings (case-insensitive); an unknown name
prints close matches.
