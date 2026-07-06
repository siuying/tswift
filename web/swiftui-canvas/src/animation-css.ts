// animation-css.ts — SwiftUI `.animation` / `.transition` → CSS mapping.
//
// The web insight (plan Slice 6): SwiftUI's `.animation(_:value:)` makes the
// *next* property change tween. On the web that is exactly CSS `transition`:
// set `transition` on the element now, and when the following render's
// `setModifiers` mutates an inline style property, the browser animates it.
// This module owns the ANIM/TRANS → CSS timing-string translation so
// `modifier-css.ts` stays a flat switch.

import type { UiirValue } from "./modifier-css.js";

/** A decoded SwiftUI `Animation` object (the `{"$":"animation",…}` payload). */
interface AnimObject {
  $: string;
  kind?: string;
  duration?: number;
  delay?: number;
  speed?: number;
  repeat?: string | number;
  autoreverses?: boolean;
}

/** kind → CSS `transition-timing-function`. The spring family has no native CSS
 * timing function, so it degrades to a hand-tuned `cubic-bezier` approximation
 * (a real spring would need Web Animations / JS; out of scope for v1). */
const TIMING_FUNCTION: Record<string, string> = {
  linear: "linear",
  easeIn: "ease-in",
  easeOut: "ease-out",
  easeInOut: "ease-in-out",
  // Springy overshoot: ends above 1 then settles (visible bounce past target).
  spring: "cubic-bezier(0.5, 1.25, 0.75, 1.25)",
  // `smooth` is a critically-damped, no-overshoot spring → ease-in-out is close.
  smooth: "ease-in-out",
  // `snappy`/`bouncy` overshoot more aggressively than `spring`.
  snappy: "cubic-bezier(0.4, 1.4, 0.6, 1.2)",
  bouncy: "cubic-bezier(0.34, 1.56, 0.64, 1.0)",
};

/** kind → default duration (seconds) when the ANIM omits `duration`. Bare
 * curves have no duration in the UIIR; these mirror SwiftUI's conceptual
 * defaults (easeInOut≈0.35, linear≈0.25, spring family≈0.5). */
const DEFAULT_DURATION: Record<string, number> = {
  linear: 0.25,
  easeIn: 0.35,
  easeOut: 0.35,
  easeInOut: 0.35,
  spring: 0.5,
  smooth: 0.5,
  snappy: 0.5,
  bouncy: 0.5,
};

/** Narrow a UIIR value to the tagged `Animation` object (guards every field). */
function asAnim(value: UiirValue): AnimObject | null {
  if (!value || typeof value !== "object" || Array.isArray(value)) return null;
  const v = value as Record<string, unknown>;
  if (v.$ !== "animation") return null;
  return v as unknown as AnimObject;
}

/** The id of the injected `<style>` sheet holding the repeat keyframes. */
const KEYFRAME_STYLE_ID = "swiftui-canvas-anim-keyframes";

/** Inject the shared repeat keyframes once (guarded by id). Repeating CSS
 * `transition` is impossible, so a repeating SwiftUI animation degrades to a
 * generic opacity pulse (see `applyAnimation`); this sheet defines it. */
function ensureKeyframes(): void {
  if (typeof document === "undefined") return;
  if (document.getElementById(KEYFRAME_STYLE_ID)) return;
  const style = document.createElement("style");
  style.id = KEYFRAME_STYLE_ID;
  // A property-agnostic pulse: the only repeat we can express generically on an
  // arbitrary element without knowing which property SwiftUI intends to cycle.
  style.textContent =
    "@keyframes swiftui-pulse{0%{opacity:1}50%{opacity:0.4}100%{opacity:1}}";
  (document.head ?? document.documentElement).appendChild(style);
}

/**
 * Apply a `.animation` modifier value to `el` (plan Slice 6).
 *
 * The modifier value is `{animation: ANIM|null, value?: observed}`. We only use
 * the `animation` field here — the observed `value` is the runtime's re-render
 * trigger, already reflected as changed inline styles by the time this runs.
 *
 *  - `animation: null` → clear the transition (SwiftUI "disable animation").
 *  - non-repeating ANIM → set `el.style.transition = "all <dur>s <fn> <delay>s"`
 *    so the next `setModifiers` style change tweens.
 *  - repeating ANIM (`repeat:"forever"`/`<int>`) → CSS `transition` can't repeat,
 *    so degrade to an injected opacity-pulse `@keyframes` on `el.style.animation`.
 *    (A faithful "repeat this property change" needs the target property, which
 *    the ANIM doesn't carry — documented degraded tier.)
 */
export function applyAnimation(el: HTMLElement, value: UiirValue): void {
  // Unwrap the `{animation, value}` envelope; tolerate a bare ANIM too.
  let animField: UiirValue = value;
  if (value && typeof value === "object" && !Array.isArray(value) && "animation" in value) {
    animField = (value as Record<string, UiirValue>).animation;
  }

  // `animation: null` (or absent) → disable: clear any prior tween/pulse.
  if (animField === null || animField === undefined) {
    el.style.transition = "";
    el.style.animation = "";
    return;
  }

  const anim = asAnim(animField);
  if (!anim) {
    // Unknown shape — never crash; leave styles untouched.
    return;
  }

  const kind = typeof anim.kind === "string" ? anim.kind : "easeInOut";
  const timing = TIMING_FUNCTION[kind] ?? "ease-in-out";

  let duration = typeof anim.duration === "number" ? anim.duration : DEFAULT_DURATION[kind] ?? 0.35;
  if (typeof anim.speed === "number" && anim.speed > 0) duration /= anim.speed;
  const delay = typeof anim.delay === "number" ? anim.delay : 0;

  const hasRepeat =
    anim.repeat === "forever" || (typeof anim.repeat === "number" && anim.repeat > 0);

  if (hasRepeat) {
    // DEGRADED TIER: CSS `transition` cannot repeat and the ANIM doesn't name a
    // target property, so a repeating animation maps to a generic opacity pulse
    // (covers the common spinner/pulse idiom; arbitrary-property cycling is not
    // expressible on the web without the property name).
    ensureKeyframes();
    const iterations = anim.repeat === "forever" ? "infinite" : String(anim.repeat);
    const direction = anim.autoreverses === false ? "normal" : "alternate";
    el.style.transition = "";
    el.style.animation = `swiftui-pulse ${duration}s ${timing} ${delay}s ${iterations} ${direction}`;
    return;
  }

  // The common case: arm a transition so the next style change tweens.
  el.style.animation = "";
  el.style.transition = `all ${duration}s ${timing} ${delay}s`;
}

/** A decoded `AnyTransition` object (the `{"$":"transition",…}` payload). */
interface TransObject {
  $: string;
  type?: string;
  [key: string]: unknown;
}

/** Narrow a UIIR value to the tagged `AnyTransition` object. */
function asTrans(value: UiirValue): TransObject | null {
  if (!value || typeof value !== "object" || Array.isArray(value)) return null;
  const v = value as Record<string, unknown>;
  if (v.$ !== "transition") return null;
  return v as unknown as TransObject;
}

/**
 * Apply a `.transition` modifier value to `el` (plan Slice 6).
 *
 * DEGRADED TIER: a transition's visible effect fires on insert/remove, which
 * is `apply-patch`'s mount/unmount hook — not the modifier pass. Full insert/
 * remove animation is out of scope for this slice, so we do the cheap, safe
 * thing: arm a `transition` covering the property the transition animates
 * (opacity / transform) so any subsequent style change tweens, and record the
 * decoded transition on `el.dataset.transition` for a future apply-patch mount/
 * unmount integration to consume. Never crashes on unknown/missing types.
 */
export function applyTransition(el: HTMLElement, value: UiirValue): void {
  const trans = asTrans(value);
  if (!trans) return; // unknown shape — no-op, never crash.

  // Record the raw transition so apply-patch can drive mount/unmount later.
  try {
    el.dataset.transition = JSON.stringify(value);
  } catch {
    // Non-serializable (shouldn't happen for UIIR) — skip silently.
  }

  // Arm a transition on the property the effect touches, so a same-node style
  // change (opacity/transform) animates even before the mount/unmount hook lands.
  const props = transitionProps(trans);
  if (props.size > 0) {
    el.style.transition = [...props].map((p) => `${p} 0.35s ease-in-out`).join(", ");
  }
}

/** Which CSS properties a transition's visible effect touches (best-effort,
 * recursing through `combined`/`asymmetric`). Used only to arm a transition. */
function transitionProps(trans: TransObject, acc = new Set<string>()): Set<string> {
  switch (trans.type) {
    case "opacity":
      acc.add("opacity");
      break;
    case "slide":
    case "move":
    case "push":
    case "offset":
    case "scale":
      acc.add("transform");
      break;
    case "combined": {
      const list = Array.isArray(trans.transitions) ? trans.transitions : [];
      for (const t of list) {
        const inner = asTrans(t as UiirValue);
        if (inner) transitionProps(inner, acc);
      }
      break;
    }
    case "asymmetric": {
      for (const key of ["insertion", "removal"]) {
        const inner = asTrans(trans[key] as UiirValue);
        if (inner) transitionProps(inner, acc);
      }
      break;
    }
    default:
      break; // identity / unknown → no property armed.
  }
  return acc;
}
