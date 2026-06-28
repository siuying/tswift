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
  | UiirValue[]
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

/** `.multilineTextAlignment(.center)` → CSS `text-align`. */
const TEXT_ALIGN: Record<string, string> = {
  leading: "left",
  center: "center",
  trailing: "right",
};

/** `.textCase(.uppercase)` → CSS `text-transform`. */
const TEXT_TRANSFORM: Record<string, string> = {
  uppercase: "uppercase",
  lowercase: "lowercase",
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

/** Map a nested shape descriptor (a UIIR node value) to a CSS `border-radius`
 * for `.clipShape(...)`. Returns undefined for non-rounded shapes. */
function shapeClipRadius(value: UiirValue): string | undefined {
  if (!value || typeof value !== "object" || !("kind" in value)) return undefined;
  const node = value as unknown as { kind: string; args?: Record<string, UiirValue> };
  switch (node.kind) {
    case "Circle":
    case "Ellipse":
      return "50%";
    case "Capsule":
      return "9999px";
    case "RoundedRectangle": {
      const r = node.args?.cornerRadius;
      return typeof r === "number" ? `${r}px` : "8px";
    }
    case "Rectangle":
      return "0";
    default:
      return undefined;
  }
}

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

/** Append a `text-decoration-line` keyword without dropping existing ones, so
 * `.underline().strikethrough()` yields both lines (matching SwiftUI). */
function addDecoration(el: HTMLElement, line: string): void {
  const current = el.style.textDecorationLine;
  el.style.textDecorationLine = current ? `${current} ${line}` : line;
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
      // C1 — text & universal styling modifiers.
      case "bold": {
        el.style.fontWeight = "700";
        break;
      }
      case "italic": {
        el.style.fontStyle = "italic";
        break;
      }
      case "underline": {
        addDecoration(el, "underline");
        break;
      }
      case "strikethrough": {
        addDecoration(el, "line-through");
        break;
      }
      case "opacity": {
        if (typeof value === "number") el.style.opacity = String(value);
        break;
      }
      case "foregroundStyle": {
        const c = cssColor(value);
        if (c) el.style.color = c;
        break;
      }
      case "tint": {
        const c = cssColor(value);
        if (c) el.style.accentColor = c;
        break;
      }
      case "lineLimit": {
        // The `-webkit-box` clamp requires overriding `display`, which would
        // destroy a container's flex/grid layout. SwiftUI's `lineLimit` only
        // affects text rendering, so restrict the clamp to text nodes; on
        // containers it is a no-op (descendant inheritance is deferred).
        if (typeof value === "number" && el.dataset.kind === "Text") {
          el.style.display = "-webkit-box";
          el.style.setProperty("-webkit-box-orient", "vertical");
          el.style.setProperty("-webkit-line-clamp", String(value));
          el.style.overflow = "hidden";
        }
        break;
      }
      case "multilineTextAlignment": {
        if (isToken(value, "textAlign")) {
          el.style.textAlign = TEXT_ALIGN[value.name] ?? "left";
        }
        break;
      }
      case "textCase": {
        if (isToken(value, "textCase")) {
          el.style.textTransform = TEXT_TRANSFORM[value.name] ?? "none";
        }
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
          const f = value as Record<string, UiirValue>;
          const w = cssLength(f.width);
          const h = cssLength(f.height);
          if (w) el.style.width = w;
          if (h) el.style.height = h;
          // Numeric min/max bounds (C2). `.infinity` is deferred (issue #189).
          const minW = cssLength(f.minWidth);
          const maxW = cssLength(f.maxWidth);
          const minH = cssLength(f.minHeight);
          const maxH = cssLength(f.maxHeight);
          if (minW) el.style.minWidth = minW;
          if (maxW) el.style.maxWidth = maxW;
          if (minH) el.style.minHeight = minH;
          if (maxH) el.style.maxHeight = maxH;
        }
        break;
      }
      case "offset": {
        if (value && typeof value === "object" && !("$" in value)) {
          const o = value as Record<string, UiirValue>;
          const x = typeof o.x === "number" ? o.x : 0;
          const y = typeof o.y === "number" ? o.y : 0;
          el.style.transform = `translate(${x}px, ${y}px)`;
        }
        break;
      }
      // C4 — visual decoration.
      case "clipped": {
        el.style.overflow = "hidden";
        break;
      }
      case "clipShape": {
        // The value is a nested shape descriptor; map its kind to a clip.
        const r = shapeClipRadius(value);
        if (r) el.style.borderRadius = r;
        el.style.overflow = "hidden";
        break;
      }
      case "border": {
        // `{ value: <color token>, width: n }` (width defaults to 1).
        const o = (value && typeof value === "object" ? value : {}) as Record<string, UiirValue>;
        const c = cssColor(o.value ?? value) ?? "currentColor";
        const w = typeof o.width === "number" ? o.width : 1;
        el.style.border = `${w}px solid ${c}`;
        break;
      }
      case "shadow": {
        // `{ color?, radius, x?, y? }` -> CSS box-shadow (blur = radius).
        const o = (value && typeof value === "object" ? value : {}) as Record<string, UiirValue>;
        const radius = typeof o.radius === "number" ? o.radius : 0;
        const x = typeof o.x === "number" ? o.x : 0;
        const y = typeof o.y === "number" ? o.y : 0;
        const c = cssColor(o.color) ?? "rgba(0,0,0,0.33)";
        el.style.boxShadow = `${x}px ${y}px ${radius}px ${c}`;
        break;
      }
      default:
        // Unknown modifier — ignored (forward-compatible with new tiers).
        break;
    }
  }
}
