# xcframework distribution: local-override else pinned GitHub Release asset

## Status

accepted

## Context

`tswift-ffi` compiles to a static binary that the `ios/TSwift` SwiftPM package
(`TSwiftCore` + `TSwiftUI`) links as a `binaryTarget`. We need every consumer —
local dev machines, fresh clones, and CI without a Rust toolchain — to obtain a
working `TSwiftFFI.xcframework`, while honouring two constraints: the offline
rule (no crates.io during a task) and not committing a large, churny multi-arch
binary blob to git.

## Decision

`Package.swift` selects the FFI binary at resolve time: if a git-ignored local
`ios/TSwift/Artifacts/TSwiftFFI.xcframework` exists (just built by
`scripts/build-xcframework.sh`), use `.binaryTarget(path:)`; otherwise download
a **pinned GitHub Release asset** via `.binaryTarget(url:checksum:)`, with `url`
and SHA-256 `checksum` read from a committed `ios/TSwift/ffi.pin`
(`{version, url, checksum}`). The artifact is versioned on its own `ffi-vN` tag,
bumped by `scripts/publish-xcframework.sh` (zip → `swift package
compute-checksum` → `gh release upload` → rewrite `ffi.pin`).

## Considered options

- **Committed `binaryTarget`** (blob in git): reproducible but heavy and churns
  the repo on every ABI change.
- **Build-script only** (always build locally, no remote): blocks fresh clones
  and CI that lack a Rust + Apple cross-compile toolchain.
- **GitHub Packages registry**: does not host raw binary assets; Releases do.

The local-or-pinned hybrid gives fast local iteration *and* toolchain-free
consumption, at the cost of a manual `ffi-vN` publish step and keeping `ffi.pin`
in sync — an acceptable trade since the boundary is offline-friendly (SwiftPM
already fetches `swift-snapshot-testing` over the network).
