// modifier-css.ts — the SwiftUI-modifier → CSS design system (host-side token
// resolution, plan §3.1). The runtime emits *semantic* tokens
// (`{"$":"color","name":"white"}`); the web host owns the token→CSS tables, so
// iOS-vs-web color/typography drift lives here by design.

/** A UIIR modifier value: a tagged-union token, a scalar, or an object. */
export type UiirValue =
  | null
  | number
  | string
  | boolean
  | { $: string; name: string }
  | { [key: string]: UiirValue };

export interface Modifier {
  name: string;
  value: UiirValue;
}

/** `.font(.largeTitle)` text styles → CSS `font-size` (pt → px, 1:1). */
const TEXT_STYLE_SIZE: Record<string, string> = {
  largeTitle: "34px",
  title: "28px",
  title2: "22px",
  title3: "20px",
  headline: "17px",
  subheadline: "15px",
  body: "17px",
  callout: "16px",
  caption: "12px",
  caption2: "11px",
  footnote: "13px",
};

/** `.fontWeight(.bold)` weights → CSS `font-weight`. */
const FONT_WEIGHT: Record<string, string> = {
  ultraLight: "100",
  thin: "200",
  light: "300",
  regular: "400",
  medium: "500",
  semibold: "600",
  bold: "700",
  heavy: "800",
  black: "900",
};

/** Named SwiftUI colors → CSS colors (approximate iOS system palette). */
const COLOR: Record<string, string> = {
  // Dynamic system colors resolve to CSS variables that adapt to light/dark
  // (defined in canvas.ts). Fixed colors below are appearance-independent,
  // matching SwiftUI (`.white` is always white; `.primary` adapts).
  primary: "var(--swiftui-label)",
  secondary: "var(--swiftui-label-secondary)",
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

function isToken(value: UiirValue, tag: string): value is { $: string; name: string } {
  return Boolean(
    value &&
      typeof value === "object" &&
      "$" in value &&
      value.$ === tag &&
      "name" in value &&
      typeof value.name === "string",
  );
}

/** Resolve a color token (or raw rgba) to a CSS color string. */
function cssColor(value: UiirValue): string | undefined {
  if (isToken(value, "color")) {
    return COLOR[value.name] ?? value.name;
  }
  return undefined;
}

/** Length values arrive numeric (px) — coerce to a CSS length. */
function cssLength(value: UiirValue): string | undefined {
  if (typeof value === "number") return `${value}px`;
  return undefined;
}

/**
 * Apply an ordered modifier list to `el`'s inline style. Order matters
 * (`.padding().background()` ≠ `.background().padding()`); later writes win,
 * matching the runtime's whole-list `setModifiers` semantics.
 */
export function applyModifiers(el: HTMLElement, modifiers: Modifier[]): void {
  // Reset the styles this system owns so a whole-list replace is idempotent.
  el.style.cssText = el.dataset.baseStyle ?? "";

  for (const { name, value } of modifiers) {
    switch (name) {
      case "font": {
        if (isToken(value, "textStyle")) {
          el.style.fontSize = TEXT_STYLE_SIZE[value.name] ?? "17px";
        }
        break;
      }
      case "fontWeight": {
        if (isToken(value, "weight")) {
          el.style.fontWeight = FONT_WEIGHT[value.name] ?? "400";
        }
        break;
      }
      case "foregroundColor": {
        const c = cssColor(value);
        if (c) el.style.color = c;
        break;
      }
      case "background": {
        const c = cssColor(value);
        if (c) el.style.backgroundColor = c;
        break;
      }
      case "fill": {
        // A shape's fill drives `currentColor` (shape backgrounds use it).
        const c = cssColor(value);
        if (c) el.style.color = c;
        break;
      }
      case "cornerRadius": {
        const r = cssLength(value);
        if (r) el.style.borderRadius = r;
        break;
      }
      case "padding": {
        const p = cssLength(value);
        el.style.padding = p ?? "16px";
        break;
      }
      case "frame": {
        if (value && typeof value === "object" && !("$" in value)) {
          const w = cssLength((value as Record<string, UiirValue>).width);
          const h = cssLength((value as Record<string, UiirValue>).height);
          if (w) el.style.width = w;
          if (h) el.style.height = h;
        }
        break;
      }
      default:
        // Unknown modifier — ignored (forward-compatible with new tiers).
        break;
    }
  }
}
