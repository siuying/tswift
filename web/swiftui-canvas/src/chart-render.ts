// chart-render.ts — SVG renderer for Charts UIIR nodes (Chart + marks).
// Deterministic: stable series ordering, fixed viewBox/margins, no randomness.

import type { Modifier, UiirValue } from "./modifier-css.js";

/** Minimal UIIR node shape used by the chart painter (matches apply-patch). */
export interface ChartNode {
  kind: string;
  args: Record<string, unknown>;
  modifiers: Modifier[];
  children: ChartNode[];
}

const SVG_NS = "http://www.w3.org/2000/svg";

/** Fixed plot geometry so snapshots are stable across hosts. */
const VIEW_W = 320;
const VIEW_H = 220;
const MARGIN = { top: 28, right: 16, bottom: 44, left: 48 };

/** Deterministic categorical series palette (iOS-system-ish blues/greens). */
const SERIES_PALETTE = [
  "#007aff",
  "#ff3b30",
  "#34c759",
  "#ff9500",
  "#af52de",
  "#5856d6",
  "#ff2d55",
  "#30b0c7",
  "#a2845e",
  "#8e8e93",
];

const NAMED_COLOR: Record<string, string> = {
  primary: "#000000",
  secondary: "#8e8e93",
  white: "#ffffff",
  black: "#000000",
  red: "#ff3b30",
  orange: "#ff9500",
  yellow: "#ffcc00",
  green: "#34c759",
  mint: "#00c7be",
  teal: "#30b0c7",
  cyan: "#32ade6",
  blue: "#007aff",
  indigo: "#5856d6",
  purple: "#af52de",
  pink: "#ff2d55",
  brown: "#a2845e",
  gray: "#8e8e93",
  clear: "transparent",
};

const DEFAULT_MARK_COLOR = "#007aff";
const AXIS_STROKE = "#8e8e93";
const GRID_STROKE = "#e5e5ea";
const LABEL_FILL = "#3a3a3c";

// ── PlottableValue parsing ──────────────────────────────────────────────────

export interface Plottable {
  label: string;
  /** Canonical string form of the value (for category keys / series ids). */
  raw: string;
  /** Numeric value when `isNumeric`; otherwise NaN. */
  num: number;
  /**
   * True only when the declared plottable value is a JSON number.
   * A String `"3"` stays categorical (band scale), not linear.
   */
  isNumeric: boolean;
}

/**
 * Parse a Charts PlottableValue arg.
 *
 * Preferred wire form: `{"$":"plottable","label":…,"value":…}` — JSON string
 * vs number preserves declared type (String categories stay strings).
 * Legacy Display strings `PlottableValue(label: L, value: V)` still accepted.
 */
export function parsePlottable(input: unknown): Plottable | undefined {
  // Structured form: {"$":"plottable","label":L,"value":V}
  if (input && typeof input === "object" && !Array.isArray(input)) {
    const o = input as Record<string, unknown>;
    if (o.$ === "plottable") {
      const label = typeof o.label === "string" ? o.label : String(o.label ?? "");
      const value = o.value;
      if (typeof value === "number" && Number.isFinite(value)) {
        return { label, raw: String(value), num: value, isNumeric: true };
      }
      if (typeof value === "string") {
        // Declared String stays categorical even when numeric-looking.
        return { label, raw: value, num: NaN, isNumeric: false };
      }
      if (value === null || value === undefined) {
        return { label, raw: "", num: NaN, isNumeric: false };
      }
      if (typeof value === "boolean") {
        return { label, raw: value ? "true" : "false", num: NaN, isNumeric: false };
      }
      const raw = String(value);
      return { label, raw, num: NaN, isNumeric: false };
    }
  }

  // Legacy Display string: `PlottableValue(label: Name, value: A)`.
  // degraded: cannot distinguish String("3") from Int(3) when unquoted.
  if (typeof input !== "string") return undefined;
  const m = /^PlottableValue\(label:\s*(.*?),\s*value:\s*(.*)\)\s*$/.exec(input);
  if (!m) return undefined;
  const label = m[1] ?? "";
  let raw = (m[2] ?? "").trim();
  // Quoted string → always categorical.
  if (raw.length >= 2 && raw.startsWith('"') && raw.endsWith('"')) {
    raw = raw.slice(1, -1).replace(/\\"/g, '"').replace(/\\\\/g, "\\");
    return { label, raw, num: NaN, isNumeric: false };
  }
  const num = Number(raw);
  const isNumeric = raw !== "" && Number.isFinite(num);
  return { label, raw, num: isNumeric ? num : NaN, isNumeric };
}

// ── Modifier helpers ────────────────────────────────────────────────────────

function isVisibilityHidden(value: UiirValue): boolean {
  if (typeof value === "string") {
    return /Visibility\(token:\s*hidden\)/i.test(value) || value === "hidden";
  }
  if (value && typeof value === "object" && !Array.isArray(value)) {
    const o = value as Record<string, unknown>;
    if (o.$ === "visibility" && o.name === "hidden") return true;
    if (typeof o.name === "string" && o.name === "hidden") return true;
  }
  return false;
}

function colorFromValue(value: UiirValue): string | undefined {
  if (!value || typeof value !== "object" || Array.isArray(value)) return undefined;
  const o = value as Record<string, unknown>;
  if (o.$ === "color" && typeof o.name === "string") {
    return NAMED_COLOR[o.name] ?? o.name;
  }
  return undefined;
}

/** Series key from `.foregroundStyle(by:)` / `.symbol(by:)` / `.position(by:)`. */
function seriesFromModifiers(mods: Modifier[]): string | undefined {
  for (const name of ["foregroundStyle", "symbol", "position"] as const) {
    const m = mods.find((x) => x.name === name);
    if (!m || !m.value || typeof m.value !== "object" || Array.isArray(m.value)) continue;
    const by = (m.value as Record<string, unknown>).by;
    const pv = parsePlottable(by);
    if (pv) return pv.raw;
  }
  return undefined;
}

function solidColorFromModifiers(mods: Modifier[]): string | undefined {
  for (const name of ["foregroundStyle", "foregroundColor", "tint"] as const) {
    const m = mods.find((x) => x.name === name);
    if (!m) continue;
    // `{ by: ... }` is series encoding, not a solid color.
    if (m.value && typeof m.value === "object" && !Array.isArray(m.value) && "by" in m.value) {
      continue;
    }
    const c = colorFromValue(m.value);
    if (c) return c;
  }
  return undefined;
}

function numberMod(mods: Modifier[], name: string): number | undefined {
  const m = mods.find((x) => x.name === name);
  return typeof m?.value === "number" ? m.value : undefined;
}

function stringAxisLabel(mods: Modifier[], name: string): string | undefined {
  const m = mods.find((x) => x.name === name);
  if (!m) return undefined;
  if (typeof m.value === "string") return m.value;
  // Builder form may nest a Text node under `value`.
  if (m.value && typeof m.value === "object" && !Array.isArray(m.value)) {
    const v = m.value as Record<string, unknown>;
    if (typeof v.verbatim === "string") return v.verbatim;
    if (v.kind === "Text" && v.args && typeof v.args === "object") {
      const args = v.args as Record<string, unknown>;
      if (typeof args.verbatim === "string") return args.verbatim;
    }
  }
  return undefined;
}

function axisContentNode(mods: Modifier[], name: string): ChartNode | undefined {
  const m = mods.find((x) => x.name === name);
  if (!m || !m.value || typeof m.value !== "object" || Array.isArray(m.value)) return undefined;
  const v = m.value as Record<string, unknown>;
  if (typeof v.kind === "string") return m.value as unknown as ChartNode;
  return undefined;
}

function axisWants(node: ChartNode | undefined, kind: string): boolean {
  if (!node) return true; // default axes when no builder
  if (node.kind === kind) return true;
  return node.children.some((c) => c.kind === kind || axisWants(c, kind));
}

// ── Scales ──────────────────────────────────────────────────────────────────

interface LinearScale {
  kind: "linear";
  domain: [number, number];
  range: [number, number];
  map(v: number): number;
}

interface BandScale {
  kind: "band";
  categories: string[];
  range: [number, number];
  bandwidth: number;
  map(cat: string): number; // band center
  index(cat: string): number;
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

// ── Mark data ───────────────────────────────────────────────────────────────

interface Datum {
  mark: ChartNode;
  kind: string;
  x?: Plottable;
  y?: Plottable;
  xStart?: Plottable;
  xEnd?: Plottable;
  yStart?: Plottable;
  yEnd?: Plottable;
  angle?: Plottable;
  /** RectangleMark pixel/domain size from UIIR `width`/`height` args. */
  width?: number;
  height?: number;
  series?: string;
  color: string;
  opacity: number;
  cornerRadius: number;
  symbolSize: number;
  lineWidth: number;
  innerRadius?: number;
}

const MARK_KINDS = new Set([
  "BarMark",
  "LineMark",
  "PointMark",
  "AreaMark",
  "RuleMark",
  "RectangleMark",
  "SectorMark",
]);

function collectMarks(node: ChartNode): ChartNode[] {
  const out: ChartNode[] = [];
  for (const c of node.children) {
    if (MARK_KINDS.has(c.kind)) out.push(c);
    else if (c.children.length) out.push(...collectMarks(c));
  }
  return out;
}

function lineWidthFromMods(mods: Modifier[]): number {
  const m = mods.find((x) => x.name === "lineStyle");
  if (!m) return 2;
  if (typeof m.value === "number") return m.value;
  if (m.value && typeof m.value === "object" && !Array.isArray(m.value)) {
    const o = m.value as Record<string, unknown>;
    if (typeof o.lineWidth === "number") return o.lineWidth;
    // StrokeStyle Display string fallback
  }
  if (typeof m.value === "string") {
    const mm = /lineWidth:\s*([0-9.]+)/.exec(m.value);
    if (mm) return Number(mm[1]);
  }
  return 2;
}

function numberArg(args: Record<string, unknown>, key: string): number | undefined {
  const v = args[key];
  if (typeof v === "number" && Number.isFinite(v)) return v;
  if (typeof v === "string" && v.trim() !== "") {
    const n = Number(v);
    if (Number.isFinite(n)) return n;
  }
  return undefined;
}

function toDatum(mark: ChartNode, seriesColor: Map<string, string>): Datum {
  const mods = mark.modifiers;
  const series = seriesFromModifiers(mods);
  const solid = solidColorFromModifiers(mods);
  const color =
    solid ??
    (series ? (seriesColor.get(series) ?? DEFAULT_MARK_COLOR) : DEFAULT_MARK_COLOR);
  const opacity = numberMod(mods, "opacity") ?? 1;
  const cornerRadius = numberMod(mods, "cornerRadius") ?? 0;
  const symbolSize = numberMod(mods, "symbolSize") ?? 64;
  return {
    mark,
    kind: mark.kind,
    x: parsePlottable(mark.args.x),
    y: parsePlottable(mark.args.y),
    xStart: parsePlottable(mark.args.xStart),
    xEnd: parsePlottable(mark.args.xEnd),
    yStart: parsePlottable(mark.args.yStart),
    yEnd: parsePlottable(mark.args.yEnd),
    angle: parsePlottable(mark.args.angle),
    // RectangleMark UIIR shape is x/y/width/height (not xStart/xEnd/…).
    width: numberArg(mark.args, "width"),
    height: numberArg(mark.args, "height"),
    series,
    color,
    opacity,
    cornerRadius,
    symbolSize,
    lineWidth: lineWidthFromMods(mods),
    innerRadius: numberArg(mark.args, "innerRadius"),
  };
}

// ── SVG helpers ─────────────────────────────────────────────────────────────

function el<K extends keyof SVGElementTagNameMap>(
  tag: K,
  attrs: Record<string, string | number | undefined> = {},
): SVGElementTagNameMap[K] {
  const node = document.createElementNS(SVG_NS, tag);
  for (const [k, v] of Object.entries(attrs)) {
    if (v === undefined) continue;
    node.setAttribute(k, String(v));
  }
  return node;
}

function niceTicks(min: number, max: number, count: number): number[] {
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

function formatTick(v: number): string {
  if (Number.isInteger(v)) return String(v);
  return String(Number(v.toPrecision(4)));
}

// ── Public API ──────────────────────────────────────────────────────────────

/**
 * Paint a Chart UIIR node into `host` as an inline SVG (+ optional legend).
 * Replaces any previous chart content under the host (keeps host itself).
 */
export function renderChart(host: HTMLElement, node: ChartNode): void {
  // Clear prior chart chrome only.
  host.querySelectorAll(":scope > .chart-svg, :scope > .chart-legend").forEach((n) => n.remove());

  host.style.display = host.style.display || "flex";
  host.style.flexDirection = host.style.flexDirection || "column";
  host.style.alignItems = host.style.alignItems || "stretch";
  host.style.boxSizing = "border-box";
  if (!host.style.minWidth) host.style.minWidth = `${VIEW_W}px`;
  if (!host.style.minHeight) host.style.minHeight = `${VIEW_H}px`;

  const marks = collectMarks(node).map((m) => m); // preserve document order
  const seriesOrder: string[] = [];
  for (const m of marks) {
    const s = seriesFromModifiers(m.modifiers);
    if (s !== undefined && !seriesOrder.includes(s)) seriesOrder.push(s);
  }
  const seriesColor = new Map<string, string>();
  seriesOrder.forEach((s, i) => {
    seriesColor.set(s, SERIES_PALETTE[i % SERIES_PALETTE.length]!);
  });

  const data = marks.map((m) => toDatum(m, seriesColor));
  const mods = node.modifiers;

  const xHidden =
    isVisibilityHidden(mods.find((m) => m.name === "chartXAxis")?.value ?? null) ||
    false;
  const yHidden =
    isVisibilityHidden(mods.find((m) => m.name === "chartYAxis")?.value ?? null) ||
    false;
  const legendHidden =
    isVisibilityHidden(mods.find((m) => m.name === "chartLegend")?.value ?? null) ||
    false;

  const xAxisNode = axisContentNode(mods, "chartXAxis");
  const yAxisNode = axisContentNode(mods, "chartYAxis");
  const xShowGrid = !xHidden && axisWants(xAxisNode, "AxisGridLine");
  const yShowGrid = !yHidden && axisWants(yAxisNode, "AxisGridLine");
  const xShowTick = !xHidden && axisWants(xAxisNode, "AxisTick");
  const yShowTick = !yHidden && axisWants(yAxisNode, "AxisTick");
  const xShowLabel = !xHidden && axisWants(xAxisNode, "AxisValueLabel");
  const yShowLabel = !yHidden && axisWants(yAxisNode, "AxisValueLabel");

  const xAxisTitle = stringAxisLabel(mods, "chartXAxisLabel");
  const yAxisTitle = stringAxisLabel(mods, "chartYAxisLabel");

  // Domain collection
  const xCats: string[] = [];
  const xNums: number[] = [];
  const yNums: number[] = [];
  let xAllNumeric = true;
  let hasXY = false;

  const pushX = (p?: Plottable) => {
    if (!p) return;
    hasXY = true;
    if (p.isNumeric) xNums.push(p.num);
    else {
      xAllNumeric = false;
      if (!xCats.includes(p.raw)) xCats.push(p.raw);
    }
  };
  const pushY = (p?: Plottable) => {
    if (!p) return;
    hasXY = true;
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

  let xScale: LinearScale | BandScale;
  if (!xAllNumeric || xCats.length > 0) {
    // Categorical: use ordered unique raw strings (include numeric raws if mixed).
    const cats = xCats.length
      ? xCats
      : xNums.map(String).filter((c, i, a) => a.indexOf(c) === i);
    // Also fold numeric-as-string categories from marks that were numeric-only
    // when we still want band scale for bar charts with string x.
    const allCats: string[] = [];
    for (const d of data) {
      for (const p of [d.x, d.xStart, d.xEnd]) {
        if (!p) continue;
        const key = p.isNumeric && xAllNumeric ? String(p.num) : p.raw;
        if (!allCats.includes(key)) allCats.push(key);
      }
    }
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

  const mapX = (p: Plottable): number => {
    if (xScale.kind === "band") {
      return xScale.map(p.isNumeric && xAllNumeric && xCats.length === 0 ? String(p.num) : p.raw);
    }
    return xScale.map(p.isNumeric ? p.num : 0);
  };
  const mapY = (p: Plottable): number => yScale.map(p.isNumeric ? p.num : 0);

  const svg = el("svg", {
    class: "chart-svg",
    viewBox: `0 0 ${VIEW_W} ${VIEW_H}`,
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
      x: plotL,
      y: plotT,
      width: plotW,
      height: plotH,
      fill: "transparent",
    }),
  );

  // ── Pie / donut (SectorMark-only charts) ──────────────────────────────────
  if (onlySectors) {
    drawSectors(svg, sectors, plotL, plotT, plotW, plotH);
  } else {
    // Gridlines
    if (yShowGrid && !yHidden) {
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
    if (xShowGrid && !xHidden && xScale.kind === "linear") {
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

    // Marks — paint order: area → bar → rule → rect → line → point → sector overlay
    const byKind = (k: string) => data.filter((d) => d.kind === k);
    drawAreas(svg, byKind("AreaMark"), mapX, mapY, yScale.map(0));
    drawBars(svg, byKind("BarMark"), mapX, mapY, yScale, xScale, seriesOrder);
    drawRules(svg, byKind("RuleMark"), mapX, mapY, plotL, plotR, plotT, plotB);
    drawRectangles(svg, byKind("RectangleMark"), mapX, mapY, xScale, yScale);
    drawLines(svg, byKind("LineMark"), mapX, mapY);
    drawPoints(svg, byKind("PointMark"), mapX, mapY);
    if (sectors.length) drawSectors(svg, sectors, plotL, plotT, plotW, plotH);

    // Axes
    if (!xHidden) {
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
          if (xShowTick) {
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
          if (xShowLabel) {
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
          if (xShowTick) {
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
          if (xShowLabel) {
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

    if (!yHidden) {
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
        if (yShowTick) {
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
        if (yShowLabel) {
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

    if (xAxisTitle && !xHidden) {
      const t = el("text", {
        x: (plotL + plotR) / 2,
        y: VIEW_H - 6,
        "text-anchor": "middle",
        "font-size": 12,
        "font-weight": 600,
        fill: LABEL_FILL,
        "font-family": "system-ui, -apple-system, sans-serif",
      });
      t.textContent = xAxisTitle;
      svg.appendChild(t);
    }
    if (yAxisTitle && !yHidden) {
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
      t.textContent = yAxisTitle;
      svg.appendChild(t);
    }
  }

  // Legend (HTML, above SVG when series present)
  if (!legendHidden && seriesOrder.length > 0) {
    const legend = document.createElement("div");
    legend.className = "chart-legend";
    legend.style.cssText =
      "display:flex;flex-wrap:wrap;gap:10px 14px;justify-content:center;" +
      "font:12px system-ui,-apple-system,sans-serif;color:#3a3a3c;padding:4px 0 2px;";
    for (const s of seriesOrder) {
      const item = document.createElement("span");
      item.style.cssText = "display:inline-flex;align-items:center;gap:5px;";
      const swatch = document.createElement("span");
      swatch.style.cssText = `display:inline-block;width:10px;height:10px;border-radius:2px;background:${seriesColor.get(s)};`;
      const label = document.createElement("span");
      label.textContent = s;
      item.append(swatch, label);
      legend.appendChild(item);
    }
    host.appendChild(legend);
  }

  host.appendChild(svg);

  // Silence unused when no xy data (empty chart still shows axes box).
  void hasXY;
}

// ── Mark drawers ────────────────────────────────────────────────────────────

function drawBars(
  svg: SVGElement,
  bars: Datum[],
  mapX: (p: Plottable) => number,
  mapY: (p: Plottable) => number,
  yScale: LinearScale,
  xScale: LinearScale | BandScale,
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

function plotFraction(xScale: LinearScale): number {
  return Math.abs(xScale.range[1] - xScale.range[0]) / 8;
}

function groupSeries(marks: Datum[]): Map<string, Datum[]> {
  const map = new Map<string, Datum[]>();
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

function sortByX(arr: Datum[], mapX: (p: Plottable) => number): Datum[] {
  return arr
    .filter((d) => d.x && d.y)
    .slice()
    .sort((a, b) => mapX(a.x!) - mapX(b.x!));
}

function drawLines(
  svg: SVGElement,
  lines: Datum[],
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
  areas: Datum[],
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
  points: Datum[],
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
  rules: Datum[],
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
  rects: Datum[],
  mapX: (p: Plottable) => number,
  mapY: (p: Plottable) => number,
  _xScale: LinearScale | BandScale,
  _yScale: LinearScale,
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

function drawSectors(
  svg: SVGElement,
  sectors: Datum[],
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
    const frac =
      sectors.length <= 1 ? 1 : Math.abs(d.angle?.num ?? 0) / total;
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
