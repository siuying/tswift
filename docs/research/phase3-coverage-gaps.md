# Phase 3 coverage-gap survey

**Date:** 2026-07-18  
**Scope:** daily Swift + SwiftUI iOS-app usage under `tswift`; research only

## Executive readout

The runtime can already render a useful headless SwiftUI value tree, and its
targeted value-framework coverage is stronger than the raw full-SDK numbers
suggest. The main daily-app blockers are the app/scene boundary, core Swift
collection protocols, host-backed services (network/files), asynchronous view
lifecycle, and SwiftData model ergonomics. EventKit is not currently a gap.

Coverage was refreshed from the live registries with:

```text
cargo test -p tswift-std dump_registered_keys
cargo test -p tswift-foundation dump_registered_keys
```

| Framework | Implemented | Verified | Practical reading |
|---|---:|---:|---|
| Swift stdlib (targeted scope) | 428/524 = 81.7% | 428/524 = 81.7% | Good concrete containers/numbers, but protocol surface is largely absent. Full inventory denominator is 428/2,439 = 17.5%. |
| Foundation | 467/615 = 75.9% | 465/615 = 75.6% | Strong value/coding subset; networking and host persistence are the sharp edges. |
| SwiftUI | 481/657 = 73.2% | 464/657 = 70.6% | Counter-to-navigation slices work; app lifecycle, bindings/observation, geometry and async lifecycle remain limiting. |
| SwiftData | 27/114 = 23.7% | 25/114 = 21.9% | Stage-1 scalar CRUD works; real model identity, relationships and lifecycle ergonomics are incomplete. |
| Charts | 60/60 = 100.0% | 36/60 = 60.0% | Registry-complete for scoped 2D API, but most View-level behavior lacks fixtures and host rendering is separate. |
| EventKit (other tracked) | 224/224 = 100.0% | 224/224 = 100.0% | No immediate coverage blocker in its deliberately scoped surface. |

The stdlib full-inventory figure should not be compared directly with the
targeted figures: the scope manifest intentionally excludes hundreds of
low-priority/host-oriented types. A high registry percentage also does not
prove that a host can render or perform the operation.

## Smoke tests

I tried three app-shaped programs through `tswift run`:

| Program | Result | Gap exposed |
|---|---|---|
| Counter app: `@main`, `App`, `WindowGroup`, `@State`, `Button` | Failed: `CounterApp has no method main` | `App`/`Scene` lifecycle is explicitly out of scope, so a normal iOS app entry point cannot run through the general CLI. |
| List/navigation app: `@main`, `NavigationStack`, `List`, `ForEach`, `NavigationLink` | Failed: `ListApp has no method main` | Same lifecycle boundary masks otherwise-supported view slices. |
| SwiftData todo app: `@main`, `@Model`, `@Query`, `.modelContainer(for:)` | Failed: `TodoApp has no method main` | Same lifecycle failure occurs before SwiftData behavior is exercised. |

The corresponding existing root-view fixtures (without `@main`/`App`) run
successfully with exit code 0: `tests/swiftui-fixtures/counter.swift`,
`list.swift`, and `swiftdata-query-nested.swift`. This is a table blind spot:
the render/session slice is usable, but an ordinary app-shaped source file is
not runnable as an iOS app.

## Ranked backlog

Priority combines frequency in ordinary iOS apps, blocking power, and leverage
across frameworks. Labels: **(a)** implementable properly in the headless
runtime; **(b)** needs new runtime/host support; **(c)** document unsupported.

### 1. Establish an explicit App/Scene entry-point and host contract — L — **(b)/(c)**

Normal `@main struct MyApp: App` programs fail immediately because the CLI
evaluator looks for a `main` method on the app type. `App`, `Scene`, and
`WindowGroup` are intentionally excluded from SwiftUI coverage. A headless
runner needs either a host-owned entry point that extracts the root `View`, or
a clear `tswift run` rule saying it accepts root-view sources only. Full
window lifecycle, UIKit integration, and platform scenes should remain **(c)**
until a native host exists. This is highest leverage because all three smoke
tests stop here.

### 2. Implement core `Sequence`/`Collection` protocol operations — L — **(a)**

`Sequence` is 0/31 and `Collection` is 0/23 in the stdlib inventory. Daily
Swift and SwiftUI data flow depend on `map`, `filter`, `compactMap`, `forEach`,
`reduce`, `first`, `contains`, `sorted`, `enumerated`, `drop`, and `joined`;
`ForEach` is especially constrained without generic collection behavior. This
unlocks user-defined and framework-facing generic code better than isolated
concrete-container additions.

### 3. Finish SwiftData model identity, relationships, and common scalar types — L — **(a)/(b)**

`PersistentModel`, `Schema`, `SchemaProperty`, and `PersistentIdentifier` are
0% in the tracked inventory despite structural `@Model` discovery and scalar
CRUD working in fixtures. The implementation does not expose
`.persistentModelID`/`.id`, does not model relationships, and rejects `Data`
and `Date` columns. Implement identity and schema introspection first (**a**),
then relationship storage/querying and richer columns with SQLite/runtime work
(**b**). This turns a demo todo store into a viable app model layer.

### 4. Complete `Binding`, observation, and environment propagation — M — **(a)/(b)**

`Binding` is 4/13 implemented and only 2/13 verified; `Environment` is 0/2;
`StateObject`, `ObservedObject`, and `EnvironmentObject` are each 33.3%
verified. Missing collection/key-path bindings, transaction/animation
semantics, and invalidation propagation block forms, shared view models, and
settings screens. Value plumbing is **(a)**; automatic invalidation and host
event scheduling may require **(b)** session/runtime support.

### 5. Make async view lifecycle and network fetches host-correct — L — **(b)**

The common `.task {}` modifier is absent, and `Task {}` completion is not
drained before a dispatch re-render. `URLSession` is 2/5 implemented in the
tracked surface (20% verified); `bytes`, download APIs, and publisher APIs are
missing. `data(from:)` works only when an embedding installs a transport, and
the CLI SwiftUI path installs none. Add a mount-task/completion-to-rerender
protocol and make host transport capability explicit. Keep cancellation and
fetch timing honest per `docs/research/ios-hn-reader-feasibility.md`.

### 6. Add host services for iOS-compatible files, defaults, and errors — M — **(b)**

`FileManager` is 0/3 in the tracked Foundation inventory and richer operations
require a host service; `UserDefaults` and filesystem behavior are already
degraded host-backed implementations. Daily apps need persistence, documents,
caches, and settings, but iOS/WASM cannot simply use the CLI filesystem. Define
the host capability ABI, graceful unavailable behavior, and portable error/value
semantics. Keep symlinks, permissions, `FileHandle`, and Darwin-only details as
**(c)** until explicitly supported.

### 7. Close high-frequency Foundation data/time/network gaps — M — **(a)/(b)**

Foundation is weakest in `Data` (57.1%), `Date` (55.2%), `URLQueryItem`
(62.5%), and `URLSession` (20% verified). Omissions include `Data` byte/unsafe
byte access and contiguous storage, URLSession `bytes` and download forms, plus
locale/time-zone-sensitive date behavior. Data slicing, URL components, and
deterministic date operations are **(a)**; streaming, downloads, locale/time
zone fidelity, and transport policy are **(b)**.

### 8. Fill stdlib numeric/text protocols needed by decoded models — M — **(a)**

The targeted stdlib roll-up hides major generic gaps: all fixed-width integer
types other than `Int` are 0%, `Character` is 0/32, and Codable container
protocol sections are 0%. Concrete `Int`, `Double`, `String`, `Array`, and
`Dictionary` are fairly strong, but decoded API models and generic algorithms
frequently use `UInt*`, `Character`, `Result`, `OptionSet`, `Identifiable`, and
`Keyed/Unkeyed*CodingContainer`. Implement common protocol/conformance plumbing
in the headless runtime (**a**); leave raw pointer/ABI-only surfaces as **(c)**.

### 9. Finish everyday SwiftUI controls, navigation, geometry, and identity — L — **(a)/(b)**

`ForEach` is 25% verified, `Binding` 15.4%, `ScrollView` 20%, `Circle` and
`RoundedRectangle` 16.7%/12.5%, `NavigationLink` 33.3%, and
`NavigationSplitView`/`GeometryReader` are 0%. The existing UIIR can absorb
many value constructors and modifiers (**a**), while geometry round trips,
real navigation stacks, focus, and platform presentation need host event/layout
support (**b**). Prioritize keyed identity and binding-driven controls.

### 10. Verify Charts end-to-end and narrow its promise — M — **(b)/(c)**

Charts is 100% implemented in its scoped registry but only 60% verified; the
`View` section is 24% verified. Core marks are registered and fixtures exist,
but ChartProxy coordinate lookup, arbitrary gestures, and actual host plot/axis
rendering are not equivalent to a native chart. Add host snapshots and
selection tests (**b**). Document ChartProxy, 3D charts, and unsupported
gesture semantics as **(c)** until a plot-coordinate event contract exists.

## Framework-by-framework notes

**Swift stdlib.** Concrete containers, numbers, strings, and ranges are the
strongest area (roughly 76–94% in their sections). The missing protocol/generic
surface is the real blocker: `Sequence`, `Collection`, `Comparable`,
`Identifiable`, `Result`, `OptionSet`, fixed-width integers, `Character`, and
Codable container protocols. Unsafe pointers, SIMD, spans, and ABI-only APIs
should remain outside the daily-app target.

**Foundation.** JSON coding, URL requests, calendars, decimal arithmetic, and
most value primitives are usable. Highest-impact gaps are `Data` byte APIs,
locale/time-zone fidelity, URLSession streaming/download/publisher APIs, and
portable FileManager/UserDefaults host behavior. Existing scope notes already
document several intentional Darwin gaps; preserve those caveats rather than
adding inert shims.

**SwiftUI.** The modifier registry is broad (`View` 85.3% verified), but that
number overstates app readiness because lifecycle is excluded and key types are
shallow. Prioritize identity → bindings → observation → geometry/navigation →
async lifecycle. Rendering-only modifiers that cannot produce host events
should remain explicitly degraded.

**SwiftData.** Stage-1 CRUD is a sound foundation: container, context,
fetch/insert/delete/save, predicates, and `@Query` have working slices. The gap
is the model contract around that core—identity, relationships, schema
exposure, `Date`/`Data`, migrations, history, CloudKit, undo, and automatic
query invalidation. Migrations/history/CloudKit are **(c)** for the headless
tier until persistence/host ownership is decided.

**Charts.** Current coverage means “constructs UIIR,” not “renders a native
chart.” Core 2D marks are a reasonable headless target. Plot geometry,
arbitrary gestures, 3D APIs, and pixel-faithful output require host contracts
or should be documented unsupported.

**EventKit and other tracked frameworks.** EventKit is 100% implemented and
verified in its scoped inventory, so it should not displace this backlog.
UIKit, MapKit, AVFoundation, Photos, CoreLocation, HealthKit, and other modules
not present in `tools/framework-inventory/frameworks.toml` have no measured
runtime coverage in this phase; document them as unsupported (**c**) until each
gets a scope manifest and host-ownership plan.

## Evidence and limitations

- Commands: `python3 tools/stdlib-inventory/coverage.py --all` and
  `python3 tools/framework-inventory/coverage.py --framework <name>`.
- Smoke programs were temporary files under `/tmp`; no repository source or
  test fixture was changed.
- The three `tswift run` failures are the same first blocker, so they do not
  independently prove the deeper List or SwiftData gaps. Existing root
  fixtures confirm the failure is the app/scene boundary, not basic root-view
  parsing.
- “Daily usage frequency” is a qualitative weighting based on common app
  architecture patterns, not telemetry.
