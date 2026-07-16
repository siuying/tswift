# ADR-0019: Charts render host — Charts content in the shared SwiftUI UIIR

- **Status:** Accepted
- **Date:** 2026-07-13
- **Context slice:** Charts (framework layer)
- **Target platform:** iOS simulator SDK `Charts.framework`; see `tools/framework-inventory/frameworks.toml` `[charts]`
- **Builds on:** ADR-0006 (SwiftUI render host), `docs/plan/framework-support.md` (framework descriptors)
- **Drives:** `docs/plan/charts-support.md` (the staged implementation plan)

## Context

Charts is a render-host framework, not value semantics: `Chart { ... }` evaluates
content and each host must turn that result into graphics. A separate chart tree,
diff engine, or host protocol would duplicate the SwiftUI render path even though
Charts is itself SwiftUI content and needs the same stateful evaluation and patch
flow.

The host needs to preserve plottable type. A category such as `"3"` is not the
numeric value `3`: the former uses a band scale, the latter a continuous scale.
It also needs honest cross-host fidelity: the web host paints SVG; the iOS host
can use Apple's native `Charts` framework, but not every modifier has a complete
host mapping yet.

## Decision

Build Charts as a framework layer over ADR-0006's existing UIIR and Rust diff
engine.

1. **`Chart` is a container view value; marks are `ChartContent` leaves.**
   `Chart { ... }` collects its result-builder children into the shared UIIR.
   `BarMark`, `LineMark`, `PointMark`, `AreaMark`, `RuleMark`, `RectangleMark`,
   `SectorMark`, and axis-content leaves are ordinary UIIR values, so existing
   mount, patch, and state behaviour applies without a chart-specific diff.

2. **Chart and mark modifiers use the shared ordered `_Modifier` records.**
   Mark modifiers attach to `ChartContent`; chart-level modifiers attach to the
   `Chart` view value. This preserves modifier ordering and lets hosts lower the
   same semantic records at their boundary.

3. **Plottable values use a structured wire form.**
   `PlottableValue.value` serializes as
   `{"$":"plottable","label":...,"value":...}`. The nested value is a JSON
   string or number, preserving categorical `String` versus numeric scale input;
   numeric-looking strings remain categories. Hosts accept the old display form
   only as a compatibility fallback.

4. **Hosts lower Charts independently.** The web host paints deterministic SVG
   inside the existing SwiftUI canvas. The iOS host lowers UIIR to native
   `import Charts` views. Native axes, scales, legend visibility, supported mark
   modifiers, foreground-style scales, and collected plot-area styling map to
   Charts where possible. Axis/legend builder forms are degraded on iOS;
   `chartXSelection` is best-effort (a constant initial binding, no write-back),
   and unexpandable `chartPlotStyle` content is a no-op. The web SVG renderer is
   a semantic approximation, not a pixel-identical native chart renderer.

### Fidelity boundary (explicitly accepted)

- **Faithful:** Charts content shares SwiftUI UIIR identity, Rust diffing, and
  patch application. Structured plottables retain categorical versus numeric
  semantics.
- **Degraded tiers:** web is SVG rather than native Charts; iOS has best-effort
  chart modifiers as listed above. These are named capability limits, not claims
  of native parity.

## Consequences

- **Good:** one evaluator, UIIR, and diff engine serve SwiftUI and Charts; a
  chart can participate in the existing web and iOS host pipeline without a
  parallel runtime.
- **Cost / risk:** host lowering remains deliberately platform-specific. SVG and
  native Charts will differ visually, and unsupported modifier forms must stay
  explicit degraded behaviour rather than silently acquiring new semantics.
- **Out of scope:** RealityKit/visionOS 3D Charts: `Chart3D*` types and
  `chart3D*`/`chartZ*` members. This ADR covers only the 2D Charts render host.

## Notes

- `frameworks/charts/inventory.md`, `frameworks/charts/scope.toml`, and
  `frameworks/charts/registered_keys.txt` are the scoped surface and coverage
  inputs.
- The shipped sequence, verification signals, and follow-up work live in
  `docs/plan/charts-support.md`.
