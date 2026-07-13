// chart/layout.ts — pure domain / scale / layout math (no DOM).

import type { ChartSpec, MarkSpec, Plottable } from "./wire.js";

/** Fixed plot geometry so snapshots are stable across hosts. */
export const VIEW_W = 320;
export const VIEW_H = 220;
export const MARGIN = { top: 28, right: 16, bottom: 44, left: 48 };

export interface LinearScale {
  kind: "linear";
  domain: [number, number];
  range: [number, number];
  map(v: number): number;
}

export interface BandScale {
  kind: "band";
  categories: string[];
  range: [number, number];
  bandwidth: number;
  map(cat: string): number; // band center
  index(cat: string): number;
}

export type XScale = LinearScale | BandScale;

export interface ChartLayout {
  viewW: number;
  viewH: number;
  plotL: number;
  plotR: number;
  plotT: number;
  plotB: number;
  plotW: number;
  plotH: number;
  xScale: XScale;
  yScale: LinearScale;
  ymin: number;
  ymax: number;
  /** True when every collected x plottable was numeric (linear x path). */
  xAllNumeric: boolean;
  onlySectors: boolean;
  sectors: MarkSpec[];
  mapX(p: Plottable): number;
  mapY(p: Plottable): number;
}

function linearScale(domain: [number, number], range: [number, number]): LinearScale {
  const [d0, d1] = domain;
  const [r0, r1] = range;
  const span = d1 - d0 || 1;
  return {
    kind: "linear",
    domain,
    range,
    map(v: number): number {
      return r0 + ((v - d0) / span) * (r1 - r0);
    },
  };
}

function bandScale(categories: string[], range: [number, number]): BandScale {
  const [r0, r1] = range;
  const n = Math.max(categories.length, 1);
  const step = (r1 - r0) / n;
  const pad = step * 0.15;
  const bandwidth = Math.max(step - 2 * pad, 1);
  const indexOf = new Map(categories.map((c, i) => [c, i]));
  return {
    kind: "band",
    categories,
    range,
    bandwidth,
    index(cat: string): number {
      return indexOf.get(cat) ?? 0;
    },
    map(cat: string): number {
      const i = indexOf.get(cat) ?? 0;
      return r0 + i * step + step / 2;
    },
  };
}

/**
 * Build ordered unique x categories once from mark plottables.
 * Uses `raw` for categorical values; when every x is numeric but a band scale
 * is still required, uses String(num).
 */
function buildXCategories(marks: MarkSpec[], xAllNumeric: boolean): string[] {
  const cats: string[] = [];
  for (const d of marks) {
    for (const p of [d.x, d.xStart, d.xEnd]) {
      if (!p) continue;
      const key = p.isNumeric && xAllNumeric ? String(p.num) : p.raw;
      if (!cats.includes(key)) cats.push(key);
    }
  }
  return cats;
}

/** Pure layout from a decoded ChartSpec. No DOM access. */
export function computeLayout(spec: ChartSpec): ChartLayout {
  const data = spec.marks;

  const xCats: string[] = [];
  const xNums: number[] = [];
  const yNums: number[] = [];
  let xAllNumeric = true;

  const pushX = (p?: Plottable) => {
    if (!p) return;
    if (p.isNumeric) xNums.push(p.num);
    else {
      xAllNumeric = false;
      if (!xCats.includes(p.raw)) xCats.push(p.raw);
    }
  };
  const pushY = (p?: Plottable) => {
    if (!p) return;
    if (p.isNumeric) yNums.push(p.num);
    else yNums.push(0);
  };

  for (const d of data) {
    pushX(d.x);
    pushX(d.xStart);
    pushX(d.xEnd);
    pushY(d.y);
    pushY(d.yStart);
    pushY(d.yEnd);
  }

  const hasBarsOrAreas = data.some((d) => d.kind === "BarMark" || d.kind === "AreaMark");
  const sectors = data.filter((d) => d.kind === "SectorMark" && d.angle);
  const onlySectors = sectors.length > 0 && data.every((d) => d.kind === "SectorMark");

  const plotL = MARGIN.left;
  const plotR = VIEW_W - MARGIN.right;
  const plotT = MARGIN.top;
  const plotB = VIEW_H - MARGIN.bottom;
  const plotW = plotR - plotL;
  const plotH = plotB - plotT;

  let xScale: XScale;
  if (!xAllNumeric || xCats.length > 0) {
    const allCats = buildXCategories(data, xAllNumeric);
    xScale = bandScale(allCats.length ? allCats : ["?"], [plotL, plotR]);
  } else {
    let xmin = xNums.length ? Math.min(...xNums) : 0;
    let xmax = xNums.length ? Math.max(...xNums) : 1;
    if (xmin === xmax) {
      xmin -= 1;
      xmax += 1;
    }
    xScale = linearScale([xmin, xmax], [plotL, plotR]);
  }

  let ymin = yNums.length ? Math.min(...yNums) : 0;
  let ymax = yNums.length ? Math.max(...yNums) : 1;
  if (hasBarsOrAreas) {
    ymin = Math.min(0, ymin);
    ymax = Math.max(0, ymax);
  }
  if (ymin === ymax) {
    ymin = Math.min(0, ymin - 1);
    ymax = Math.max(1, ymax + 1);
  }
  // y grows downward in SVG — flip range.
  const yScale = linearScale([ymin, ymax], [plotB, plotT]);

  // Capture for mapX closure (band key selection matches prior renderer).
  const xCatsLen = xCats.length;

  const mapX = (p: Plottable): number => {
    if (xScale.kind === "band") {
      return xScale.map(
        p.isNumeric && xAllNumeric && xCatsLen === 0 ? String(p.num) : p.raw,
      );
    }
    return xScale.map(p.isNumeric ? p.num : 0);
  };
  const mapY = (p: Plottable): number => yScale.map(p.isNumeric ? p.num : 0);

  return {
    viewW: VIEW_W,
    viewH: VIEW_H,
    plotL,
    plotR,
    plotT,
    plotB,
    plotW,
    plotH,
    xScale,
    yScale,
    ymin,
    ymax,
    xAllNumeric,
    onlySectors,
    sectors,
    mapX,
    mapY,
  };
}

export function niceTicks(min: number, max: number, count: number): number[] {
  if (!Number.isFinite(min) || !Number.isFinite(max)) return [0];
  if (min === max) return [min];
  const span = max - min;
  const step0 = span / Math.max(count, 1);
  const mag = Math.pow(10, Math.floor(Math.log10(Math.abs(step0) || 1)));
  const err = step0 / mag;
  let step: number;
  if (err >= 5) step = 10 * mag;
  else if (err >= 2) step = 5 * mag;
  else if (err >= 1) step = 2 * mag;
  else step = mag;
  const start = Math.ceil(min / step) * step;
  const ticks: number[] = [];
  // Cap iterations for pathological domains.
  for (let v = start, i = 0; v <= max + step * 1e-9 && i < 32; v += step, i++) {
    // Avoid -0
    ticks.push(Math.abs(v) < 1e-12 ? 0 : Number(v.toPrecision(12)));
  }
  if (ticks.length === 0) ticks.push(min, max);
  return ticks;
}

export function formatTick(v: number): string {
  if (Number.isInteger(v)) return String(v);
  return String(Number(v.toPrecision(4)));
}
