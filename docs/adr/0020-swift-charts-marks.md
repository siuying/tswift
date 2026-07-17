# ADR-0020: Swift Charts marks as SwiftUI view nodes

- **Status:** Accepted
- **Date:** 2026-07-17
- **Context slice:** Standing up Swift Charts as a tracked framework — the
  `Chart` container and the core marks (`BarMark`, `LineMark`, `PointMark`,
  `AreaMark`, `RuleMark`, `RectangleMark`, `SectorMark`).
- **Builds on:** ADR-0006 (render-host architecture), the SwiftUI view/UIIR
  pipeline (`view_value` leaf nodes, `container_value`, the modifier chain).

## Context

Swift Charts renders declarative data visualizations. Its public surface is a
`Chart { … }` view whose `@ChartContentBuilder` body produces *marks* — data
points mapped onto x/y/angle scales. In real Charts, a mark carries a
`PlottableValue` per channel, the framework infers scale domains/ranges, lays
out axes and legends, and draws using host geometry.

tswift already renders SwiftUI views to a serializable UIIR node tree consumed
by a web/iOS host. A mark is, structurally, a SwiftUI view: a leaf record with
constructor args. The weakest sufficient requirement to "support Charts" is
therefore *not* a new rendering subsystem — it is registering mark constructors
that produce ordinary `view_value` leaf nodes under a `Chart` container node,
exactly like `Text` under a `VStack`.

## Decision

**Charts marks are SwiftUI views.** They live in
`crates/tswift-swiftui/src/charts.rs` and register through `crate::install` into
the same interpreter/session/UIIR pipeline as native views. No separate crate,
no separate render path, no new patch ops.

- **`Chart`** is a container view (`container_value("Chart", children)`). Two
  forms: static `Chart { marks }` (`collect_children`) and data-driven
  `Chart(data, id:) { d in mark }` (`keyed_rows`, sugar for a keyed `ForEach`
  of marks — one mark subtree per element, keys stable for the diff).
- **Marks** (`BarMark`/`LineMark`/`PointMark`/`AreaMark`/`RuleMark`/
  `RectangleMark`/`SectorMark`) are leaf `view_value` nodes. Each records the
  plotted channels it was constructed with (`x`, `y`, `xStart`, `xEnd`,
  `yStart`, `yEnd`, `series`, `angle`, `width`, `height`, `innerRadius`,
  `outerRadius`, `angularInset`, `stacking`) as node args; unrecognized labels
  are dropped rather than mis-serialized.
- **Channel values** are prelude Swift types (SwiftUI `PRELUDE`), following the
  `GridItem` precedent:
  - `PlottableValue<Value>.value(_ label:_ value:)` → serialized
    `{"$":"plottable","label":…,"value":…}`. The value is stored dynamically
    (Double/Int/String flow through unchanged).
  - `MarkDimension.automatic/.fixed(_)/.ratio(_)/.inset(_)` → serialized
    `{"$":"markDimension","kind":…,"value":…}`.
  - `MarkStackingMethod` and `InterpolationMethod` are token structs (the
    `token_of` allowlist), serialized `{"$":"markStacking"|"interpolationMethod",
    "name":…}`.
- **Leading-dot resolution.** Mark inits are registered with typed params
  (`x: PlottableValue`, `width: MarkDimension`, `stacking: MarkStackingMethod`)
  so `x: .value(…)`, `width: .fixed(20)`, `stacking: .center` resolve against
  the right namespace — the issue #203 typed-param pattern.

## Fidelity tier — named honestly

**Stage-1 is "channels-recorded, host-drawn."** The runtime performs **no**
scale-domain/range inference, axis or legend layout, data binning/aggregation,
or mark stacking. It records what each mark was given and hands the tree to the
host, which owns all geometry and drawing. This is the same degraded-tier
honesty as the faux-vs-native presentation/navigation contracts.

Deliberately out of scope this slice (see `frameworks/charts/scope.toml`
`[out_of_scope]`): axis/legend/scale customization, `ChartProxy` geometry
read-back, scrollable/zoomable charts, 3-D charts, plot annotations, symbol
shapes, and the vectorized plot-content fast paths — all host-geometry- or
data-pipeline-heavy surfaces with no UIIR contract yet.

## Consequences

- Marks compose with the existing modifier pipeline (`.foregroundStyle`,
  `.opacity`, …) for free — they are view nodes.
- Coverage is tracked as a first-class `charts` framework
  (`tools/framework-inventory/frameworks.toml`, `frameworks/charts/`), with its
  own inventory (generated from the iOS SDK `.swiftinterface`), scope manifest,
  and a `registered_keys.txt` dumped from `charts::registered_keys`.
- **Tripwire to reopen:** the first fixture that needs a resolved axis, a legend,
  a `ChartProxy` gesture read-back, or true mark stacking. That is a genuine new
  host-geometry capability tier, not a modifier addition — it earns its own ADR.
