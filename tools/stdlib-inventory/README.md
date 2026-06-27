# stdlib inventory & coverage tooling

This directory is kept for backward compatibility. The generalized tooling now
lives in `tools/framework-inventory/` and is driven by framework descriptors.

Existing commands still work:

```sh
F=~/Library/Developer/Toolchains/swift-6.3.2-RELEASE.xctoolchain/usr/lib/swift/macosx/Swift.swiftmodule/arm64-apple-macos.swiftinterface
python3 tools/stdlib-inventory/extract.py "$F" > docs/swift-runtime/stdlib-inventory.md
python3 tools/stdlib-inventory/coverage.py
python3 tools/stdlib-inventory/coverage.py Array
```

Preferred commands:

```sh
python3 tools/framework-inventory/extract.py --framework stdlib \
  > docs/swift-runtime/stdlib-inventory.md
python3 tools/framework-inventory/coverage.py --framework stdlib
```

Registry keys are now mirrored to `frameworks/stdlib/registered_keys.txt` by
`cargo test -p tswift-std dump_registered_keys`; the legacy
`tools/stdlib-inventory/registered_keys.txt` file is also refreshed.
