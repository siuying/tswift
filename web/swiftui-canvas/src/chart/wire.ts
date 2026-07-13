// chart/wire.ts — decode a Chart UiirNode ONCE into a typed ChartSpec.
// Legacy Display-string PlottableValue parsing lives only in
// `parsePlottableFromLegacyDisplay` (the compatibility adapter).

import type { Modifier, UiirNode, UiirValue } from "../uiir-types.js";

/** Deterministic categorical series palette (iOS-system-ish blues/greens). */
export const SERIES_PALETTE = [
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

export const DEFAULT_MARK_COLOR = "#007aff";

export type MarkKind =
  | "BarMark"
  | "LineMark"
  | "PointMark"
  | "AreaMark"
  | "RuleMark"
  | "RectangleMark"
  | "SectorMark";

const MARK_KINDS: ReadonlySet<string> = new Set([
  "BarMark",
  "LineMark",
  "PointMark",
  "AreaMark",
  "RuleMark",
  "RectangleMark",
  "SectorMark",
]);

/**
 * PlottableValue: label + declared value (string | number on the wire).
 * `raw` / `num` / `isNumeric` are the render-facing projection.
 */
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

export interface MarkSpec {
  kind: MarkKind;
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

export interface AxisSpec {
  hidden: boolean;
  showGrid: boolean;
  showTick: boolean;
  showLabel: boolean;
  title?: string;
}

export interface ChartSpec {
  marks: MarkSpec[];
  seriesOrder: string[];
  seriesColor: ReadonlyMap<string, string>;
  xAxis: AxisSpec;
  yAxis: AxisSpec;
  legendHidden: boolean;
}

// ── Wire → UiirValue narrowing (no `as unknown` casts) ──────────────────────

function isPlainObject(v: unknown): v is { [key: string]: unknown } {
  return typeof v === "object" && v !== null && !Array.isArray(v);
}

/** Coerce a raw JSON arg into UiirValue when the shape is valid. */
function coerceUiirValue(v: unknown): UiirValue | undefined {
  if (v === null) return null;
  if (typeof v === "number" || typeof v === "string" || typeof v === "boolean") return v;
  if (Array.isArray(v)) {
    const out: UiirValue[] = [];
    for (const item of v) {
      const c = coerceUiirValue(item);
      if (c === undefined) return undefined;
      out.push(c);
    }
    return out;
  }
  if (isPlainObject(v)) {
    const out: { [key: string]: UiirValue } = {};
    for (const [k, val] of Object.entries(v)) {
      const c = coerceUiirValue(val);
      if (c === undefined) return undefined;
      out[k] = c;
    }
    return out;
  }
  return undefined;
}

/**
 * Normalize a UIIR object value to a string-keyed record so field access
 * does not need union-narrowing against the token shape `{ $; name }`.
 */
function uiirObjectFields(
  value: Exclude<UiirValue, null | number | string | boolean | UiirValue[]>,
): { [key: string]: UiirValue } {
  const out: { [key: string]: UiirValue } = {};
  for (const [k, val] of Object.entries(value)) {
    const c = coerceUiirValue(val);
    if (c !== undefined) out[k] = c;
  }
  return out;
}

function argAsUiirValue(args: Record<string, unknown>, key: string): UiirValue | undefined {
  if (!(key in args)) return undefined;
  return coerceUiirValue(args[key]);
}

// ── PlottableValue parsing ──────────────────────────────────────────────────

/**
 * Legacy Display-string adapter: `PlottableValue(label: L, value: V)`.
 * degraded: cannot distinguish String("3") from Int(3) when unquoted.
 */
function parsePlottableFromLegacyDisplay(input: string): Plottable | undefined {
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

/**
 * Parse a Charts PlottableValue arg.
 *
 * Preferred wire form: `{"$":"plottable","label":…,"value":…}` — JSON string
 * vs number preserves declared type (String categories stay strings).
 * Legacy Display strings still accepted via `parsePlottableFromLegacyDisplay`.
 */
export function parsePlottable(input: UiirValue | undefined): Plottable | undefined {
  if (input === undefined || input === null) return undefined;

  // Structured form: {"$":"plottable","label":L,"value":V}
  if (typeof input === "object" && !Array.isArray(input)) {
    const o = uiirObjectFields(input);
    if (o.$ === "plottable") {
      const labelField = o.label;
      const label = typeof labelField === "string" ? labelField : String(labelField ?? "");
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

  if (typeof input === "string") {
    return parsePlottableFromLegacyDisplay(input);
  }
  return undefined;
}

/** Accept raw wire args (unknown) and decode a plottable field. */
function parsePlottableArg(args: Record<string, unknown>, key: string): Plottable | undefined {
  return parsePlottable(argAsUiirValue(args, key));
}

// ── Modifier helpers ────────────────────────────────────────────────────────

function isVisibilityHidden(value: UiirValue | undefined): boolean {
  if (value === undefined || value === null) return false;
  if (typeof value === "string") {
    return /Visibility\(token:\s*hidden\)/i.test(value) || value === "hidden";
  }
  if (typeof value === "object" && !Array.isArray(value)) {
    const o = uiirObjectFields(value);
    if (o.$ === "visibility" && o.name === "hidden") return true;
    if (typeof o.name === "string" && o.name === "hidden") return true;
  }
  return false;
}

function colorFromValue(value: UiirValue): string | undefined {
  if (!value || typeof value !== "object" || Array.isArray(value)) return undefined;
  const o = uiirObjectFields(value);
  if (o.$ === "color" && typeof o.name === "string") {
    return NAMED_COLOR[o.name] ?? o.name;
  }
  return undefined;
}

/** Series key from `.foregroundStyle(by:)` / `.symbol(by:)` / `.position(by:)`. */
function seriesFromModifiers(mods: Modifier[]): string | undefined {
  for (const name of ["foregroundStyle", "symbol", "position"] as const) {
    const m = mods.find((x) => x.name === name);
    if (!m || m.value === null || m.value === undefined) continue;
    if (typeof m.value !== "object" || Array.isArray(m.value)) continue;
    const o = uiirObjectFields(m.value);
    if (!("by" in o)) continue;
    const pv = parsePlottable(o.by);
    if (pv) return pv.raw;
  }
  return undefined;
}

function solidColorFromModifiers(mods: Modifier[]): string | undefined {
  for (const name of ["foregroundStyle", "foregroundColor", "tint"] as const) {
    const m = mods.find((x) => x.name === name);
    if (!m) continue;
    // `{ by: ... }` is series encoding, not a solid color.
    if (m.value && typeof m.value === "object" && !Array.isArray(m.value)) {
      const o = uiirObjectFields(m.value);
      if ("by" in o) continue;
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
    const o = uiirObjectFields(m.value);
    if (typeof o.verbatim === "string") return o.verbatim;
    if (o.kind === "Text" && o.args && typeof o.args === "object" && !Array.isArray(o.args)) {
      const args = uiirObjectFields(o.args);
      if (typeof args.verbatim === "string") return args.verbatim;
    }
  }
  return undefined;
}

/** Minimal tree shape for axis builder content (grid/tick/label probes). */
interface AxisContentTree {
  kind: string;
  children: AxisContentTree[];
}

function asAxisContentTree(value: UiirValue): AxisContentTree | undefined {
  if (!value || typeof value !== "object" || Array.isArray(value)) return undefined;
  const o = uiirObjectFields(value);
  if (typeof o.kind !== "string") return undefined;
  const children: AxisContentTree[] = [];
  if (Array.isArray(o.children)) {
    for (const c of o.children) {
      const n = asAxisContentTree(c);
      if (n) children.push(n);
    }
  }
  return { kind: o.kind, children };
}

function axisContentTree(mods: Modifier[], name: string): AxisContentTree | undefined {
  const m = mods.find((x) => x.name === name);
  if (!m || m.value === null || m.value === undefined) return undefined;
  return asAxisContentTree(m.value);
}

function axisWants(node: AxisContentTree | undefined, kind: string): boolean {
  if (!node) return true; // default axes when no builder
  if (node.kind === kind) return true;
  return node.children.some((c) => c.kind === kind || axisWants(c, kind));
}

function lineWidthFromMods(mods: Modifier[]): number {
  const m = mods.find((x) => x.name === "lineStyle");
  if (!m) return 2;
  if (typeof m.value === "number") return m.value;
  if (m.value && typeof m.value === "object" && !Array.isArray(m.value)) {
    const o = uiirObjectFields(m.value);
    if (typeof o.lineWidth === "number") return o.lineWidth;
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

function collectMarkNodes(node: UiirNode): UiirNode[] {
  const out: UiirNode[] = [];
  for (const c of node.children) {
    if (MARK_KINDS.has(c.kind)) out.push(c);
    else if (c.children.length) out.push(...collectMarkNodes(c));
  }
  return out;
}

function isMarkKind(kind: string): kind is MarkKind {
  return MARK_KINDS.has(kind);
}

function toMarkSpec(mark: UiirNode, seriesColor: Map<string, string>): MarkSpec | undefined {
  if (!isMarkKind(mark.kind)) return undefined;
  const mods = mark.modifiers;
  const series = seriesFromModifiers(mods);
  const solid = solidColorFromModifiers(mods);
  const color =
    solid ??
    (series ? (seriesColor.get(series) ?? DEFAULT_MARK_COLOR) : DEFAULT_MARK_COLOR);
  return {
    kind: mark.kind,
    x: parsePlottableArg(mark.args, "x"),
    y: parsePlottableArg(mark.args, "y"),
    xStart: parsePlottableArg(mark.args, "xStart"),
    xEnd: parsePlottableArg(mark.args, "xEnd"),
    yStart: parsePlottableArg(mark.args, "yStart"),
    yEnd: parsePlottableArg(mark.args, "yEnd"),
    angle: parsePlottableArg(mark.args, "angle"),
    // RectangleMark UIIR shape is x/y/width/height (not xStart/xEnd/…).
    width: numberArg(mark.args, "width"),
    height: numberArg(mark.args, "height"),
    series,
    color,
    opacity: numberMod(mods, "opacity") ?? 1,
    cornerRadius: numberMod(mods, "cornerRadius") ?? 0,
    symbolSize: numberMod(mods, "symbolSize") ?? 64,
    lineWidth: lineWidthFromMods(mods),
    innerRadius: numberArg(mark.args, "innerRadius"),
  };
}

function decodeAxis(
  mods: Modifier[],
  axisMod: string,
  labelMod: string,
): AxisSpec {
  const hidden = isVisibilityHidden(mods.find((m) => m.name === axisMod)?.value);
  const content = axisContentTree(mods, axisMod);
  return {
    hidden,
    showGrid: !hidden && axisWants(content, "AxisGridLine"),
    showTick: !hidden && axisWants(content, "AxisTick"),
    showLabel: !hidden && axisWants(content, "AxisValueLabel"),
    title: stringAxisLabel(mods, labelMod),
  };
}

/**
 * Decode a Chart `UiirNode` into a fully typed `ChartSpec`.
 * All wire / legacy-display / modifier interpretation happens here once.
 */
export function decodeChartSpec(node: UiirNode): ChartSpec {
  const markNodes = collectMarkNodes(node);
  const seriesOrder: string[] = [];
  for (const m of markNodes) {
    const s = seriesFromModifiers(m.modifiers);
    if (s !== undefined && !seriesOrder.includes(s)) seriesOrder.push(s);
  }
  const seriesColor = new Map<string, string>();
  seriesOrder.forEach((s, i) => {
    seriesColor.set(s, SERIES_PALETTE[i % SERIES_PALETTE.length]!);
  });

  const marks: MarkSpec[] = [];
  for (const m of markNodes) {
    const spec = toMarkSpec(m, seriesColor);
    if (spec) marks.push(spec);
  }

  const mods = node.modifiers;
  return {
    marks,
    seriesOrder,
    seriesColor,
    xAxis: decodeAxis(mods, "chartXAxis", "chartXAxisLabel"),
    yAxis: decodeAxis(mods, "chartYAxis", "chartYAxisLabel"),
    legendHidden: isVisibilityHidden(mods.find((m) => m.name === "chartLegend")?.value),
  };
}
