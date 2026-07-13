// chart/mark-render.ts — mark painters + arc/path helpers.

import type { ChartSpec, MarkSpec, Plottable } from "./wire.js";
import { DEFAULT_MARK_COLOR, SERIES_PALETTE } from "./wire.js";
import type { ChartLayout, LinearScale, XScale } from "./layout.js";
import { el } from "./svg.js";

function plotFraction(xScale: LinearScale): number {
  return Math.abs(xScale.range[1] - xScale.range[0]) / 8;
}

function groupSeries(marks: MarkSpec[]): Map<string, MarkSpec[]> {
  const map = new Map<string, MarkSpec[]>();
  for (const d of marks) {
    const key = d.series ?? "";
    let arr = map.get(key);
    if (!arr) {
      arr = [];
      map.set(key, arr);
    }
    arr.push(d);
  }
  return map;
}

function sortByX(arr: MarkSpec[], mapX: (p: Plottable) => number): MarkSpec[] {
  return arr
    .filter((d) => d.x && d.y)
    .slice()
    .sort((a, b) => mapX(a.x!) - mapX(b.x!));
}

function drawBars(
  svg: SVGElement,
  bars: MarkSpec[],
  mapX: (p: Plottable) => number,
  mapY: (p: Plottable) => number,
  yScale: LinearScale,
  xScale: XScale,
  seriesOrder: string[],
): void {
  if (!bars.length) return;
  const y0 = yScale.map(0);
  // Group by x key for side-by-side series.
  const seriesCount = Math.max(seriesOrder.length, 1);

  for (const d of bars) {
    if (!d.x || !d.y) continue;
    const cx = mapX(d.x);
    const y = mapY(d.y);
    let bw: number;
    if (xScale.kind === "band") {
      bw = xScale.bandwidth;
      if (seriesCount > 1 && d.series) {
        const si = seriesOrder.indexOf(d.series);
        const slot = bw / seriesCount;
        const x = cx - bw / 2 + (si >= 0 ? si : 0) * slot + slot * 0.1;
        const w = slot * 0.8;
        const top = Math.min(y, y0);
        const h = Math.max(Math.abs(y0 - y), 1);
        svg.appendChild(
          el("rect", {
            x,
            y: top,
            width: w,
            height: h,
            fill: d.color,
            opacity: d.opacity,
            rx: d.cornerRadius,
            ry: d.cornerRadius,
          }),
        );
        continue;
      }
    } else {
      bw = Math.max(plotFraction(xScale) * 0.6, 8);
    }
    const x = cx - bw / 2;
    const top = Math.min(y, y0);
    const h = Math.max(Math.abs(y0 - y), 1);
    svg.appendChild(
      el("rect", {
        x,
        y: top,
        width: bw,
        height: h,
        fill: d.color,
        opacity: d.opacity,
        rx: d.cornerRadius,
        ry: d.cornerRadius,
      }),
    );
  }
}

function drawLines(
  svg: SVGElement,
  lines: MarkSpec[],
  mapX: (p: Plottable) => number,
  mapY: (p: Plottable) => number,
): void {
  for (const [, group] of groupSeries(lines)) {
    const pts = sortByX(group, mapX);
    if (pts.length === 0) continue;
    const d0 = pts[0]!;
    const points = pts.map((p) => `${mapX(p.x!)},${mapY(p.y!)}`).join(" ");
    // Catmull-Rom / smooth: still polyline for determinism; stroke style varies.
    svg.appendChild(
      el("polyline", {
        points,
        fill: "none",
        stroke: d0.color,
        "stroke-width": d0.lineWidth,
        "stroke-linejoin": "round",
        "stroke-linecap": "round",
        opacity: d0.opacity,
      }),
    );
  }
}

function drawAreas(
  svg: SVGElement,
  areas: MarkSpec[],
  mapX: (p: Plottable) => number,
  mapY: (p: Plottable) => number,
  y0: number,
): void {
  for (const [, group] of groupSeries(areas)) {
    const pts = sortByX(group, mapX);
    if (pts.length === 0) continue;
    const d0 = pts[0]!;
    const top = pts.map((p) => `${mapX(p.x!)},${mapY(p.y!)}`).join(" L ");
    const last = pts[pts.length - 1]!;
    const first = pts[0]!;
    const path = `M ${mapX(first.x!)},${y0} L ${top} L ${mapX(last.x!)},${y0} Z`;
    svg.appendChild(
      el("path", {
        d: path,
        fill: d0.color,
        opacity: d0.opacity * 0.35,
        stroke: "none",
      }),
    );
  }
}

function drawPoints(
  svg: SVGElement,
  points: MarkSpec[],
  mapX: (p: Plottable) => number,
  mapY: (p: Plottable) => number,
): void {
  for (const d of points) {
    if (!d.x || !d.y) continue;
    // Swift Charts symbolSize is area-ish; map to radius with a stable floor.
    const r = Math.max(2.5, Math.sqrt(Math.max(d.symbolSize, 1)) / 2.5);
    svg.appendChild(
      el("circle", {
        cx: mapX(d.x),
        cy: mapY(d.y),
        r,
        fill: d.color,
        opacity: d.opacity,
        stroke: "#ffffff",
        "stroke-width": 1,
      }),
    );
  }
}

function drawRules(
  svg: SVGElement,
  rules: MarkSpec[],
  mapX: (p: Plottable) => number,
  mapY: (p: Plottable) => number,
  plotL: number,
  plotR: number,
  plotT: number,
  plotB: number,
): void {
  for (const d of rules) {
    if (d.y && !d.x && !d.xStart) {
      const y = mapY(d.y);
      svg.appendChild(
        el("line", {
          x1: plotL,
          x2: plotR,
          y1: y,
          y2: y,
          stroke: d.color,
          "stroke-width": d.lineWidth,
          opacity: d.opacity,
          "stroke-dasharray": "4 3",
        }),
      );
    } else if (d.x && !d.y && !d.yStart) {
      const x = mapX(d.x);
      svg.appendChild(
        el("line", {
          x1: x,
          x2: x,
          y1: plotT,
          y2: plotB,
          stroke: d.color,
          "stroke-width": d.lineWidth,
          opacity: d.opacity,
          "stroke-dasharray": "4 3",
        }),
      );
    } else if (d.xStart && d.xEnd && d.y) {
      const y = mapY(d.y);
      svg.appendChild(
        el("line", {
          x1: mapX(d.xStart),
          x2: mapX(d.xEnd),
          y1: y,
          y2: y,
          stroke: d.color,
          "stroke-width": d.lineWidth,
          opacity: d.opacity,
        }),
      );
    } else if (d.yStart && d.yEnd && d.x) {
      const x = mapX(d.x);
      svg.appendChild(
        el("line", {
          x1: x,
          x2: x,
          y1: mapY(d.yStart),
          y2: mapY(d.yEnd),
          stroke: d.color,
          "stroke-width": d.lineWidth,
          opacity: d.opacity,
        }),
      );
    }
  }
}

function drawRectangles(
  svg: SVGElement,
  rects: MarkSpec[],
  mapX: (p: Plottable) => number,
  mapY: (p: Plottable) => number,
): void {
  for (const d of rects) {
    let x: number;
    let y: number;
    let w: number;
    let h: number;
    if (d.xStart && d.xEnd && d.yStart && d.yEnd) {
      // Interval form: xStart/xEnd/yStart/yEnd.
      const x1 = mapX(d.xStart);
      const x2 = mapX(d.xEnd);
      const y1 = mapY(d.yStart);
      const y2 = mapY(d.yEnd);
      x = Math.min(x1, x2);
      y = Math.min(y1, y2);
      w = Math.max(Math.abs(x2 - x1), 1);
      h = Math.max(Math.abs(y2 - y1), 1);
    } else if (d.x && d.y && d.width !== undefined && d.height !== undefined) {
      // UIIR form: x/y center (plottable) + width/height sizes.
      // Sizes are treated as plot pixels (stable, matches runtime CGFloat args).
      w = Math.max(Math.abs(d.width), 1);
      h = Math.max(Math.abs(d.height), 1);
      x = mapX(d.x) - w / 2;
      y = mapY(d.y) - h / 2;
    } else {
      continue;
    }
    svg.appendChild(
      el("rect", {
        x,
        y,
        width: w,
        height: h,
        fill: d.color,
        opacity: d.opacity * 0.5,
        rx: d.cornerRadius,
        ry: d.cornerRadius,
      }),
    );
  }
}

function arcPath(
  cx: number,
  cy: number,
  inner: number,
  outer: number,
  a0: number,
  a1: number,
): string {
  const delta = a1 - a0;
  // Full circle: a single A with identical endpoints is a no-op in SVG — use two
  // semicircles (or two half-ring paths for a donut).
  if (delta >= Math.PI * 2 - 1e-9 || Math.abs(delta) < 1e-9) {
    const oxR = cx + outer;
    const oxL = cx - outer;
    if (inner <= 0) {
      return [
        `M ${oxR},${cy}`,
        `A ${outer},${outer} 0 1 1 ${oxL},${cy}`,
        `A ${outer},${outer} 0 1 1 ${oxR},${cy}`,
        "Z",
      ].join(" ");
    }
    const ixR = cx + inner;
    const ixL = cx - inner;
    return [
      `M ${oxR},${cy}`,
      `A ${outer},${outer} 0 1 1 ${oxL},${cy}`,
      `A ${outer},${outer} 0 1 1 ${oxR},${cy}`,
      `L ${ixR},${cy}`,
      `A ${inner},${inner} 0 1 0 ${ixL},${cy}`,
      `A ${inner},${inner} 0 1 0 ${ixR},${cy}`,
      "Z",
    ].join(" ");
  }
  const large = delta > Math.PI ? 1 : 0;
  const ox0 = cx + outer * Math.cos(a0);
  const oy0 = cy + outer * Math.sin(a0);
  const ox1 = cx + outer * Math.cos(a1);
  const oy1 = cy + outer * Math.sin(a1);
  if (inner <= 0) {
    return [
      `M ${cx},${cy}`,
      `L ${ox0},${oy0}`,
      `A ${outer},${outer} 0 ${large} 1 ${ox1},${oy1}`,
      "Z",
    ].join(" ");
  }
  const ix0 = cx + inner * Math.cos(a0);
  const iy0 = cy + inner * Math.sin(a0);
  const ix1 = cx + inner * Math.cos(a1);
  const iy1 = cy + inner * Math.sin(a1);
  return [
    `M ${ox0},${oy0}`,
    `A ${outer},${outer} 0 ${large} 1 ${ox1},${oy1}`,
    `L ${ix1},${iy1}`,
    `A ${inner},${inner} 0 ${large} 0 ${ix0},${iy0}`,
    "Z",
  ].join(" ");
}

export function drawSectors(
  svg: SVGElement,
  sectors: MarkSpec[],
  plotL: number,
  plotT: number,
  plotW: number,
  plotH: number,
): void {
  const cx = plotL + plotW / 2;
  const cy = plotT + plotH / 2;
  const outer = Math.min(plotW, plotH) * 0.4;
  // A lone sector always spans a full circle so it paints visibly (SVG arcs with
  // identical endpoints draw nothing). Multiple sectors share 2π by angle weight.
  const total =
    sectors.length <= 1
      ? 1
      : sectors.reduce((s, d) => s + Math.abs(d.angle?.num ?? 0), 0) || 1;
  let a0 = -Math.PI / 2;
  sectors.forEach((d, i) => {
    const frac = sectors.length <= 1 ? 1 : Math.abs(d.angle?.num ?? 0) / total;
    const a1 = a0 + frac * Math.PI * 2;
    const inner =
      typeof d.innerRadius === "number"
        ? Math.min(outer * 0.9, Math.max(0, d.innerRadius))
        : 0;
    // When innerRadius looks like a pixel from Swift, scale vs outer heuristically.
    const ir = inner > 0 && inner < outer ? inner : inner >= outer ? outer * 0.5 : 0;
    const path = arcPath(cx, cy, ir, outer, a0, a1);
    svg.appendChild(
      el("path", {
        d: path,
        fill: d.color !== DEFAULT_MARK_COLOR ? d.color : SERIES_PALETTE[i % SERIES_PALETTE.length],
        opacity: d.opacity,
        stroke: "#ffffff",
        "stroke-width": 1,
      }),
    );
    a0 = a1;
  });
}

/**
 * Paint marks in stable order: area → bar → rule → rect → line → point → sector.
 */
export function renderMarks(svg: SVGElement, layout: ChartLayout, spec: ChartSpec): void {
  const { mapX, mapY, yScale, xScale, plotL, plotR, plotT, plotB, sectors } = layout;
  const data = spec.marks;
  const byKind = (k: string) => data.filter((d) => d.kind === k);

  drawAreas(svg, byKind("AreaMark"), mapX, mapY, yScale.map(0));
  drawBars(svg, byKind("BarMark"), mapX, mapY, yScale, xScale, spec.seriesOrder);
  drawRules(svg, byKind("RuleMark"), mapX, mapY, plotL, plotR, plotT, plotB);
  drawRectangles(svg, byKind("RectangleMark"), mapX, mapY);
  drawLines(svg, byKind("LineMark"), mapX, mapY);
  drawPoints(svg, byKind("PointMark"), mapX, mapY);
  if (sectors.length) drawSectors(svg, sectors, plotL, plotT, layout.plotW, layout.plotH);
}
