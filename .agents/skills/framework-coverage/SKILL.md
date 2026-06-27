---
name: framework-coverage
description: Understand how much of a Swift framework runtime implements, by framework and type. Use when the user asks about Foundation/SwiftUI/stdlib coverage, what APIs are implemented/missing/verified, or which framework APIs to build next.
---

Use the top-down coverage workflow:

1. Refresh the framework registry snapshot from the live runtime crate:
   - stdlib: `cargo test -p tswift-std dump_registered_keys`
   - Foundation: `cargo test -p tswift-foundation dump_registered_keys`
2. Check the roll-up:
   - `python3 tools/framework-inventory/coverage.py --framework foundation`
   - `python3 tools/framework-inventory/coverage.py --framework stdlib`
3. Drill into one type/section:
   - `python3 tools/framework-inventory/coverage.py --framework foundation Data`

Artifacts:
- Descriptors: `tools/framework-inventory/frameworks.toml`
- Scope manifests: `frameworks/<name>/scope.toml`
- Inventories: `frameworks/<name>/inventory.md` (`docs/swift-runtime/stdlib-inventory.md` for stdlib)
- Registry dumps: `frameworks/<name>/registered_keys.txt`
- Tagged CLI fixtures: `crates/tswift-cli/tests/fixtures/<framework>_*.swift`
