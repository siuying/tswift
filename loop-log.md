# Feature Checklist Implementation Log

**Goal**: implement all features from docs/swift-runtime/feature-checklist.md
**Signal**: `cargo test --workspace` → all green + per-feature fixtures

## Iterations

| # | commit | feature | status | notes |
| - | ------ | ------- | ------ | ----- |
| 1 | done | Failable initializers `init?`/`init!` | keep | return nil → Optional.none in struct & class init |

## Blocked (needs human input)
See docs/swift-runtime/blocked-features.md
