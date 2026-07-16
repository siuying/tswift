// chart-render.ts — compose decode → layout → axis/mark painters into SVG.
// Deterministic: stable series ordering, fixed viewBox/margins, no randomness.

import type { UiirNode } from "./uiir-types.js";
import { decodeChartSpec, parsePlottable } from "./chart/wire.js";
import type { Plottable } from "./chart/wire.js";
import { computeLayout, VIEW_H, VIEW_W } from "./chart/layout.js";
import { renderAxes, renderGrid, renderLegend } from "./chart/axis-render.js";
import { drawSectors, renderMarks } from "./chart/mark-render.js";
import { el } from "./chart/svg.js";

export type { Plottable };
export { parsePlottable };

/**
 * Paint a Chart UIIR node into `host` as an inline SVG (+ optional legend).
 * Replaces any previous chart content under the host (keeps host itself).
 */
export function renderChart(host: HTMLElement, node: UiirNode): void {
  // Clear prior chart chrome only.
  host.querySelectorAll(":scope > .chart-svg, :scope > .chart-legend").forEach((n) => n.remove());

  host.style.display = host.style.display || "flex";
  host.style.flexDirection = host.style.flexDirection || "column";
  host.style.alignItems = host.style.alignItems || "stretch";
  host.style.boxSizing = "border-box";
  if (!host.style.minWidth) host.style.minWidth = `${VIEW_W}px`;
  if (!host.style.minHeight) host.style.minHeight = `${VIEW_H}px`;

  const spec = decodeChartSpec(node);
  const layout = computeLayout(spec);

  const svg = el("svg", {
    class: "chart-svg",
    viewBox: `0 0 ${layout.viewW} ${layout.viewH}`,
    width: "100%",
    height: "100%",
    preserveAspectRatio: "xMidYMid meet",
    role: "img",
  });
  // Ensure the class is also a property for querySelector (SVG class attribute).
  svg.setAttribute("class", "chart-svg");
  svg.style.display = "block";
  svg.style.flex = "1 1 auto";
  svg.style.minHeight = "0";

  // Background plot area (subtle)
  svg.appendChild(
    el("rect", {
      x: layout.plotL,
      y: layout.plotT,
      width: layout.plotW,
      height: layout.plotH,
      fill: "transparent",
    }),
  );

  // ── Pie / donut (SectorMark-only charts) ──────────────────────────────────
  if (layout.onlySectors) {
    drawSectors(svg, layout.sectors, layout.plotL, layout.plotT, layout.plotW, layout.plotH);
  } else {
    renderGrid(svg, layout, spec);
    // Marks — paint order: area → bar → rule → rect → line → point → sector overlay
    renderMarks(svg, layout, spec);
    renderAxes(svg, layout, spec);
  }

  // Legend (HTML, above SVG when series present)
  renderLegend(host, spec);
  host.appendChild(svg);
}
