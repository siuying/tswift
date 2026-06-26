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

## 2. Coverage inputs — `target/stdlib-coverage/`

Two *semantic* key sets, regenerated live by the golden harness so they cannot
drift from the code (and are not checked in):

- `registered.txt` — every symbol the `qswift-std` registry wires up, read from
  the live registry via `qswift_std::registered_keys()`.
- `exercised.txt` — every symbol actually dispatched by a *passing* golden
  fixture, captured through the interpreter's behavioural-coverage hook.

```sh
cargo test -p qswift-cli --test golden stdlib_coverage_inputs
```

Keys are semantic, not receiver-dispatch: `print` (free function),
`Array.append` (method/property), `Sequence.map` (algorithm layer), or
`Optional.map` (one symbol even though it dispatches on several receiver kinds).

## 3. Coverage report — `coverage.py`

A pure join over three semantic key sets — inventory (`stdlib-inventory.md`),
registered, and exercised — assigning each inventory member one of three states,
type-scoped (no global token matching):

- **missing** — not in the registry,
- **implemented** — registered but not exercised by a passing fixture,
- **verified** — registered *and* exercised by a passing fixture.

```sh
cargo test -p qswift-cli --test golden stdlib_coverage_inputs  # refresh inputs
python3 tools/stdlib-inventory/coverage.py
```

Prints per-type implemented/verified/total counts, a free-function line, and an
overall percentage across the targeted types plus free functions.
