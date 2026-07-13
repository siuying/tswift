// modifier-css.ts — the SwiftUI-modifier → CSS design system (host-side token
// resolution, plan §3.1). The runtime emits *semantic* tokens
// (`{"$":"color","name":"white"}`); the web host owns the token→CSS tables, so
// iOS-vs-web color/typography drift lives here by design.

import { applyAnimation, applyTransition } from "./animation-css.js";
import type { Modifier, UiirValue } from "./uiir-types.js";

export type { Modifier, UiirValue } from "./uiir-types.js";

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

/** Apply directional padding (`.padding(.horizontal, 8)`). A missing length
 * uses SwiftUI's default system padding (16px). */
function applyEdgePadding(el: HTMLElement, edge: string, length: string | undefined): void {
  const sides = EDGE_SIDES[edge] ?? EDGE_SIDES.all;
  const len = length ?? "16px";
  for (const side of sides) {
    el.style[`padding${side}` as "paddingTop"] = len;
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

/** Length values arrive numeric (px) — coerce to a CSS length. A non-finite
 * length (`.frame(maxWidth: .infinity)`) serializes as the `{"$":"infinity"}`
 * sentinel and maps to `100%` (fill the available cross-axis). */
function cssLength(value: UiirValue): string | undefined {
  if (typeof value === "number") return `${value}px`;
  if (value && typeof value === "object" && "$" in value && value.$ === "infinity") {
    return "100%";
  }
  return undefined;
}

/** SwiftUI 2-D `Alignment` → CSS flexbox `{ justify, align }` so a frame
 * positions its content on *both* axes (parity with iOS's native
 * `.frame(_, alignment:)`), not just horizontally. `justifyContent` is the
 * horizontal axis, `alignItems` the vertical (the element is laid out as a
 * single-row flex container). Baselines approximate to centered. */
export const FRAME_ALIGN: Record<string, { justify: string; align: string }> = {
  center: { justify: "center", align: "center" },
  leading: { justify: "flex-start", align: "center" },
  trailing: { justify: "flex-end", align: "center" },
  top: { justify: "center", align: "flex-start" },
  bottom: { justify: "center", align: "flex-end" },
  topLeading: { justify: "flex-start", align: "flex-start" },
  topTrailing: { justify: "flex-end", align: "flex-start" },
  bottomLeading: { justify: "flex-start", align: "flex-end" },
  bottomTrailing: { justify: "flex-end", align: "flex-end" },
  leadingFirstTextBaseline: { justify: "flex-start", align: "center" },
  centerFirstTextBaseline: { justify: "center", align: "center" },
  trailingFirstTextBaseline: { justify: "flex-end", align: "center" },
};

/** SwiftUI `Edge.Set` token → the CSS box sides it expands to. */
const EDGE_SIDES: Record<string, ("Top" | "Right" | "Bottom" | "Left")[]> = {
  top: ["Top"],
  bottom: ["Bottom"],
  leading: ["Left"],
  trailing: ["Right"],
  horizontal: ["Left", "Right"],
  vertical: ["Top", "Bottom"],
  all: ["Top", "Right", "Bottom", "Left"],
};

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
        // Three shapes: `.padding()` / `.padding(8)` (uniform), and the
        // directional `.padding(.horizontal, 8)` → `{ value: <edge token>,
        // value1: <length?> }` (issue #203).
        if (isToken(value, "edge")) {
          applyEdgePadding(el, value.name, undefined);
        } else if (value && typeof value === "object" && !("$" in value) && "value" in value) {
          const o = value as Record<string, UiirValue>;
          if (isToken(o.value, "edge")) {
            applyEdgePadding(el, o.value.name, cssLength(o.value1) ?? "16px");
          } else {
            el.style.padding = cssLength(o.value) ?? "16px";
          }
        } else {
          el.style.padding = cssLength(value) ?? "16px";
        }
        break;
      }
      case "frame": {
        if (value && typeof value === "object" && !("$" in value)) {
          const f = value as Record<string, UiirValue>;
          const w = cssLength(f.width);
          const h = cssLength(f.height);
          if (w) el.style.width = w;
          if (h) el.style.height = h;
          // Numeric min/max bounds plus `.infinity` (→ `100%`, issue #203).
          const minW = cssLength(f.minWidth);
          const maxW = cssLength(f.maxWidth);
          const minH = cssLength(f.minHeight);
          const maxH = cssLength(f.maxHeight);
          if (minW) el.style.minWidth = minW;
          if (maxW) {
            el.style.maxWidth = maxW;
            // `maxWidth: .infinity` fills the available width (the common
            // full-width idiom); widen the box so content alignment is visible.
            if (maxW === "100%") el.style.width = "100%";
          }
          if (minH) el.style.minHeight = minH;
          if (maxH) el.style.maxHeight = maxH;
          // Content alignment within the frame, on both axes via flexbox so it
          // matches iOS's native `frame(_, alignment:)` (issue #203).
          if (isToken(f.alignment, "align")) {
            const a = FRAME_ALIGN[f.alignment.name];
            if (a) {
              el.style.display = "flex";
              el.style.justifyContent = a.justify;
              el.style.alignItems = a.align;
            }
          }
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
      // C7 — control styling + disabled. Accessibility modifiers fall through to
      // the default (accepted-and-ignored on the web).
      case "buttonStyle": {
        if (isToken(value, "style")) {
          // A prominent/bordered/plain button skin.
          if (value.name === "borderedProminent") {
            el.style.background = "var(--swiftui-tint, #007aff)";
            el.style.color = "#fff";
            el.style.border = "none";
            el.style.padding = "7px 14px";
            el.style.borderRadius = "8px";
          } else if (value.name === "bordered") {
            el.style.background = "rgba(0,122,255,0.12)";
            el.style.color = "var(--swiftui-tint, #007aff)";
            el.style.border = "none";
            el.style.padding = "7px 14px";
            el.style.borderRadius = "8px";
          } else if (value.name === "plain") {
            el.style.background = "none";
            el.style.border = "none";
            el.style.color = "var(--swiftui-tint, #007aff)";
            el.style.padding = "0";
          }
        }
        break;
      }
      case "listStyle": {
        if (isToken(value, "style") && value.name === "plain") {
          // Plain lists drop the grouped card chrome.
          el.style.border = "none";
          el.style.borderRadius = "0";
        }
        break;
      }
      case "disabled": {
        if (value === true) {
          el.style.opacity = "0.4";
          el.style.pointerEvents = "none";
        }
        break;
      }
      // pickerStyle / textFieldStyle: accepted; the default control skin is kept.
      case "pickerStyle":
      case "textFieldStyle":
        break;
      // Tier 2 — scale / aspect / layout / z-order / navigation.
      case "scaledToFit": {
        // On image elements use `object-fit: contain`; on all elements set the
        // display hint so the host can stretch or contain the content.
        if (el.dataset.kind === "Image" || el.tagName === "IMG") {
          (el as HTMLImageElement).style.objectFit = "contain";
        }
        el.style.maxWidth = "100%";
        el.style.maxHeight = "100%";
        break;
      }
      case "scaledToFill": {
        if (el.dataset.kind === "Image" || el.tagName === "IMG") {
          (el as HTMLImageElement).style.objectFit = "cover";
        }
        el.style.width = "100%";
        el.style.height = "100%";
        break;
      }
      case "aspectRatio": {
        // `{ value: <ratio>, contentMode: <token> }` or bare ratio token.
        const o = (value && typeof value === "object" ? value : {}) as Record<string, UiirValue>;
        const ratio = typeof o.value === "number" ? o.value : typeof value === "number" ? value : undefined;
        const mode = isToken(o.contentMode, "contentMode") ? o.contentMode.name : undefined;
        if (ratio !== undefined && ratio > 0) {
          el.style.aspectRatio = String(ratio);
        }
        if (el.dataset.kind === "Image" || el.tagName === "IMG") {
          (el as HTMLImageElement).style.objectFit = mode === "fill" ? "cover" : "contain";
        }
        break;
      }
      case "fixedSize": {
        // No args: both axes fixed. `{ horizontal: bool, vertical: bool }`.
        const o = (value && typeof value === "object" ? value : {}) as Record<string, UiirValue>;
        const hasArgs = typeof o.horizontal === "boolean" || typeof o.vertical === "boolean";
        const h = hasArgs ? o.horizontal !== false : true;
        const v = hasArgs ? o.vertical !== false : true;
        if (h) { el.style.width = "fit-content"; el.style.flexShrink = "0"; }
        if (v) { el.style.height = "fit-content"; el.style.flexShrink = "0"; }
        break;
      }
      case "layoutPriority": {
        // Higher priority → higher flex-grow (proportional allocation).
        if (typeof value === "number") {
          el.style.flexGrow = String(Math.max(0, value));
        }
        break;
      }
      case "zIndex": {
        if (typeof value === "number") {
          el.style.position = "relative";
          el.style.zIndex = String(value);
        }
        break;
      }
      case "navigationTitle": {
        // Record-only for now; hosts render it when NavigationStack lands.
        // Store as a data attribute so a NavigationStack host can read it.
        if (typeof value === "string") {
          el.dataset.navigationTitle = value;
        }
        break;
      }
      case "resizable": {
        // Mark the image as resizable so later scale modifiers take effect.
        el.style.width = "100%";
        el.style.height = "auto";
        break;
      }
      // Animation — arm a CSS transition so the *next* render's style change
      // tweens (SwiftUI `.animation(_:value:)` semantics on the web). Repeating
      // animations degrade to an injected opacity pulse; `null` disables.
      case "animation": {
        applyAnimation(el, value);
        break;
      }
      // Transition — record the insert/remove effect and arm the relevant
      // property transition. Full mount/unmount animation lives in apply-patch
      // (out of scope here); this is the cheap, non-crashing v1.
      case "transition": {
        applyTransition(el, value);
        break;
      }
      default:
        // Unknown modifier — ignored (forward-compatible with new tiers).
        break;
    }
  }
}
