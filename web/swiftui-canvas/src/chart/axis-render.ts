// chart/axis-render.ts — axes, grid, labels, and HTML legend.

import type { ChartSpec } from "./wire.js";
import type { ChartLayout } from "./layout.js";
import { formatTick, niceTicks } from "./layout.js";
import { el } from "./svg.js";

const AXIS_STROKE = "#8e8e93";
const GRID_STROKE = "#e5e5ea";
const LABEL_FILL = "#3a3a3c";

/** Horizontal / vertical gridlines behind marks. */
export function renderGrid(svg: SVGElement, layout: ChartLayout, spec: ChartSpec): void {
  const { plotL, plotR, plotT, plotB, yScale, xScale, ymin, ymax } = layout;
  const { xAxis, yAxis } = spec;

  if (yAxis.showGrid && !yAxis.hidden) {
    const ticks = niceTicks(ymin, ymax, 4);
    for (const t of ticks) {
      const yy = yScale.map(t);
      svg.appendChild(
        el("line", {
          x1: plotL,
          x2: plotR,
          y1: yy,
          y2: yy,
          stroke: GRID_STROKE,
          "stroke-width": 1,
        }),
      );
    }
  }
  if (xAxis.showGrid && !xAxis.hidden && xScale.kind === "linear") {
    const ticks = niceTicks(xScale.domain[0], xScale.domain[1], 4);
    for (const t of ticks) {
      const xx = xScale.map(t);
      svg.appendChild(
        el("line", {
          x1: xx,
          x2: xx,
          y1: plotT,
          y2: plotB,
          stroke: GRID_STROKE,
          "stroke-width": 1,
        }),
      );
    }
  }
}

/** X/Y axis lines, ticks, value labels, and axis titles. */
export function renderAxes(svg: SVGElement, layout: ChartLayout, spec: ChartSpec): void {
  const {
    plotL,
    plotR,
    plotT,
    plotB,
    viewH,
    xScale,
    yScale,
    ymin,
    ymax,
  } = layout;
  const { xAxis, yAxis } = spec;

  if (!xAxis.hidden) {
    svg.appendChild(
      el("line", {
        x1: plotL,
        x2: plotR,
        y1: plotB,
        y2: plotB,
        stroke: AXIS_STROKE,
        "stroke-width": 1,
      }),
    );
    if (xScale.kind === "band") {
      xScale.categories.forEach((cat) => {
        const cx = xScale.map(cat);
        if (xAxis.showTick) {
          svg.appendChild(
            el("line", {
              x1: cx,
              x2: cx,
              y1: plotB,
              y2: plotB + 4,
              stroke: AXIS_STROKE,
              "stroke-width": 1,
            }),
          );
        }
        if (xAxis.showLabel) {
          const t = el("text", {
            x: cx,
            y: plotB + 16,
            "text-anchor": "middle",
            "font-size": 11,
            fill: LABEL_FILL,
            "font-family": "system-ui, -apple-system, sans-serif",
          });
          t.textContent = cat;
          svg.appendChild(t);
        }
      });
    } else {
      for (const tv of niceTicks(xScale.domain[0], xScale.domain[1], 4)) {
        const cx = xScale.map(tv);
        if (xAxis.showTick) {
          svg.appendChild(
            el("line", {
              x1: cx,
              x2: cx,
              y1: plotB,
              y2: plotB + 4,
              stroke: AXIS_STROKE,
              "stroke-width": 1,
            }),
          );
        }
        if (xAxis.showLabel) {
          const t = el("text", {
            x: cx,
            y: plotB + 16,
            "text-anchor": "middle",
            "font-size": 11,
            fill: LABEL_FILL,
            "font-family": "system-ui, -apple-system, sans-serif",
          });
          t.textContent = formatTick(tv);
          svg.appendChild(t);
        }
      }
    }
  }

  if (!yAxis.hidden) {
    svg.appendChild(
      el("line", {
        x1: plotL,
        x2: plotL,
        y1: plotT,
        y2: plotB,
        stroke: AXIS_STROKE,
        "stroke-width": 1,
      }),
    );
    for (const tv of niceTicks(ymin, ymax, 4)) {
      const cy = yScale.map(tv);
      if (yAxis.showTick) {
        svg.appendChild(
          el("line", {
            x1: plotL - 4,
            x2: plotL,
            y1: cy,
            y2: cy,
            stroke: AXIS_STROKE,
            "stroke-width": 1,
          }),
        );
      }
      if (yAxis.showLabel) {
        const t = el("text", {
          x: plotL - 8,
          y: cy + 3,
          "text-anchor": "end",
          "font-size": 11,
          fill: LABEL_FILL,
          "font-family": "system-ui, -apple-system, sans-serif",
        });
        t.textContent = formatTick(tv);
        svg.appendChild(t);
      }
    }
  }

  if (xAxis.title && !xAxis.hidden) {
    const t = el("text", {
      x: (plotL + plotR) / 2,
      y: viewH - 6,
      "text-anchor": "middle",
      "font-size": 12,
      "font-weight": 600,
      fill: LABEL_FILL,
      "font-family": "system-ui, -apple-system, sans-serif",
    });
    t.textContent = xAxis.title;
    svg.appendChild(t);
  }
  if (yAxis.title && !yAxis.hidden) {
    const t = el("text", {
      x: 14,
      y: (plotT + plotB) / 2,
      "text-anchor": "middle",
      "font-size": 12,
      "font-weight": 600,
      fill: LABEL_FILL,
      "font-family": "system-ui, -apple-system, sans-serif",
      transform: `rotate(-90 14 ${(plotT + plotB) / 2})`,
    });
    t.textContent = yAxis.title;
    svg.appendChild(t);
  }
}

/**
 * HTML legend above the SVG when series encodings are present.
 * Appends the legend element to `host` (before the SVG, per prior layout).
 */
export function renderLegend(host: HTMLElement, spec: ChartSpec): void {
  if (spec.legendHidden || spec.seriesOrder.length === 0) return;
  const legend = document.createElement("div");
  legend.className = "chart-legend";
  legend.style.cssText =
    "display:flex;flex-wrap:wrap;gap:10px 14px;justify-content:center;" +
    "font:12px system-ui,-apple-system,sans-serif;color:#3a3a3c;padding:4px 0 2px;";
  for (const s of spec.seriesOrder) {
    const item = document.createElement("span");
    item.style.cssText = "display:inline-flex;align-items:center;gap:5px;";
    const swatch = document.createElement("span");
    swatch.style.cssText = `display:inline-block;width:10px;height:10px;border-radius:2px;background:${spec.seriesColor.get(s)};`;
    const label = document.createElement("span");
    label.textContent = s;
    item.append(swatch, label);
    legend.appendChild(item);
  }
  host.appendChild(legend);
}
