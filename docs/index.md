# tswift documentation index

## Using tswift

- [Multi-file compilation](multi-file-compilation.md) — compiling file lists,
  directories, and Package.swift subsets as one module; diagnostic format and
  known limitations.

## Runtime and coverage

- [Feature checklist](swift-runtime/feature-checklist.md) — high-level runtime
  feature status.
- [Stdlib inventory](swift-runtime/stdlib-inventory.md) — the Swift standard
  library surface tracked for coverage.
- [Blocked features](swift-runtime/blocked-features.md)

## Design

- [Architectural decision records](adr/) — load-bearing decisions and
  invariants. Start with
  [ADR-0017 multi-file program input](adr/0017-multi-file-program-input.md),
  [ADR-0006 SwiftUI render host](adr/0006-swiftui-render-host.md), and
  [ADR-0014 host services](adr/0014-host-services-web-ios.md).
- [Implementation plan](plan/swift-runtime-implementation-plan.md)
- [Research notes](research/) — including
  [incremental compilation](research/incremental-compilation.md) and the
  [phase 3 coverage-gap survey](research/phase3-coverage-gaps.md).

## Contributing / agents

- [Environment and conventions](agents/environment.md) — commit rules, offline
  builds, presubmit.
- [Issue tracker workflow](agents/issue-tracker.md) and
  [triage labels](agents/triage-labels.md).
- [Domain docs](agents/domain.md)
