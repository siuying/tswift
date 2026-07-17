# Blocked / Partial Features

Features that cannot be completed without a larger frontend change or a design
decision needing human input. Each entry records what works, what is missing,
and why it is blocked.

## Builtin-enum `init(rawValue:)` round-trip (EventKit + all framework enums)

**Works:** Every in-scope EventKit enum (`EKSpan`, `EKAuthorizationStatus`,
`EKWeekday`, …) is registered as a builtin enum: its cases resolve (leading-dot
and qualified spellings), `switch`/`==` work, and the `init` coverage key is
reachable — `EKType(rawValue:)` is dispatched without error.

**Missing:** `init(rawValue:)` returns `nil` instead of the matching case, and
`.rawValue` is unavailable, because builtin enums carry no raw values
(`register_builtin_enum` stores `raw: None`; see
`crates/tswift-core/src/interp.rs`). ObjC `NS_ENUM` raw values are also not plain
ordinals (e.g. `EKWeekday` is 1-based, `EKAuthorizationStatus.authorized`
aliases `.fullAccess == 3`), so ordinal indexing would be wrong.

**Why blocked:** Faithful round-trip needs a `register_builtin_enum` variant
that accepts explicit per-case raw values — a shared core change affecting every
framework's enum registration, out of scope for the EventKit slice. Tracked as a
cross-framework enhancement. The 16 EventKit enum `init` keys are therefore
counted **implemented** (registered/reachable) but **not verified** (no faithful
golden), which is the honest state.

