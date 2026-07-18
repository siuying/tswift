# Plan — Charts Support

**Status:** landed
**Date:** 2026-07-13
**Reference SDK:** iOS Simulator `Charts.framework`; see
[`frameworks/charts/inventory.md`](../../frameworks/charts/inventory.md).
**Related:**

- [ADR-0019](../adr/0019-charts-render-host.md) — shared-UIIR Charts render host
- [SwiftUI support plan](swiftui-support.md) — the UIIR/diff/host foundation
- [Framework support plan](framework-support.md) — inventory and coverage loop

---

## Shipped slices

| Slice | Status | Delivered |
|---|---|---|
| Scaffold | Landed | `tswift-charts`, `Chart`, `BarMark`, and plottable values on shared UIIR. |
| Core marks | Landed | Line, point, area, rule, rectangle, sector, and axis-content leaves. |
| Mark modifiers | Landed | `ChartContent` modifiers append shared ordered `_Modifier` records. |
| Chart modifiers | Landed | Axes, scales, legend, selection, and chart-content builder forms. |
| Web host | Landed | Deterministic SVG rendering and dynamic chart patches in the SwiftUI canvas. |
| iOS host | Landed | Native `import Charts` lowering in `UiirRenderer`. |
| Coverage breadth | Landed | 58/60 in-scope members implemented (96.7%); only 3D/RealityKit chart surface excluded. |

## Verification signals

- `tswift-charts` unit tests cover runtime constructors, modifiers, and UIIR
  plottable encoding.
- SwiftUI UIIR and patch goldens cover chart fixtures.
- Web Playwright snapshots cover SVG output and dynamic patches.
- iOS `UiirRenderer` snapshots cover native Charts lowering.
- `framework-inventory` measures the scoped Charts surface against
  [`frameworks/charts/inventory.md`](../../frameworks/charts/inventory.md).

## Backlog

- 3D Charts and RealityKit/visionOS: `Chart3D*`, `chart3D*`, and `chartZ*`.
- Verify the supported Charts surface against real `swiftc` where available.
- Improve degraded modifier tiers: iOS axis/legend builder forms,
  `chartXSelection` write-back, unexpandable `chartPlotStyle`, and host mappings
  for `chartBackground`, `chartOverlay`, and axis-style modifiers. `Chart.body`
  now exposes the materialized Chart UIIR; `chartGesture` supports the
  headless TapGesture/LongPressGesture subset. ChartProxy geometry/value lookup
  and arbitrary Gesture families remain deferred until hosts provide plot
  coordinates and gesture payloads.
