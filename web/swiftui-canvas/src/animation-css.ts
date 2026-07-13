// animation-css.ts — SwiftUI `.animation` / `.transition` → CSS mapping.
//
// The web insight (plan Slice 6): SwiftUI's `.animation(_:value:)` makes the
// *next* property change tween. On the web that is exactly CSS `transition`:
// set `transition` on the element now, and when the following render's
// `setModifiers` mutates an inline style property, the browser animates it.
// This module owns the ANIM/TRANS → CSS timing-string translation so
// `modifier-css.ts` stays a flat switch.

import type { UiirValue } from "./uiir-types.js";

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

/** Inject the shared repeat keyframes once per tree root (guarded by id).
 * Repeating CSS `transition` is impossible, so a repeating SwiftUI animation
 * degrades to a generic opacity pulse (see `applyAnimation`); this sheet
 * defines it.
 *
 * CRITICAL: `@keyframes` names resolve *within the tree that contains the
 * animated element*. The canvas renders inside a shadow root, so keyframes
 * injected into `document.head` are invisible to it and the animation silently
 * no-ops. We therefore inject into `el`'s own root node (its `ShadowRoot`, or
 * the document when unhosted). */
function ensureKeyframes(el: HTMLElement): void {
  if (typeof document === "undefined") return;
  // The shadow root (or document) that scopes this element's keyframe lookup.
  // Guard with `querySelector` (present on both `Document` and `ShadowRoot`;
  // `getElementById` is not reliably exposed on `ShadowRoot` in WebKit).
  const root = el.getRootNode() as Document | ShadowRoot;
  if (root.querySelector(`#${KEYFRAME_STYLE_ID}`)) return;
  const style = document.createElement("style");
  style.id = KEYFRAME_STYLE_ID;
  // A property-agnostic pulse: the only repeat we can express generically on an
  // arbitrary element without knowing which property SwiftUI intends to cycle.
  style.textContent =
    "@keyframes swiftui-pulse{0%{opacity:1}50%{opacity:0.4}100%{opacity:1}}";
  // Append to the tree root itself (shadow root) or the document head.
  const host = root instanceof Document ? (root.head ?? root.documentElement) : root;
  host.appendChild(style);
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
    ensureKeyframes(el);
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
 * A transition's visible effect fires on insert/remove, which is
 * `apply-patch`'s mount/unmount hook — not the modifier pass. Here we only
 * *record* the decoded transition on `el.dataset.transition`; the insert/remove
 * handlers in `apply-patch` consume it via `playTransitionEnter` /
 * `playTransitionLeave` to actually tween the node in and out. Never crashes on
 * unknown/missing types.
 */
export function applyTransition(el: HTMLElement, value: UiirValue): void {
  const trans = asTrans(value);
  if (!trans) return; // unknown shape — no-op, never crash.

  // Record the raw transition so apply-patch's insert/remove can drive it.
  try {
    el.dataset.transition = JSON.stringify(value);
  } catch {
    // Non-serializable (shouldn't happen for UIIR) — skip silently.
  }
}

/** Default insert/remove tween duration (seconds) and curve. A `.transition`
 * carries no timing of its own — the ambient `withAnimation` supplies it — so
 * we use SwiftUI's default ease for the common case. */
const TRANSITION_DURATION = 0.35;
const TRANSITION_TIMING = "ease-in-out";

/** The offscreen/hidden style for a transition's active edge (`insertion` =
 * where an inserted view starts, `removal` = where a removed view ends). Only
 * the properties the transition animates are set; the rest stay at their
 * natural value. Recurses through `combined`/`asymmetric`. */
function hiddenStyle(
  trans: TransObject,
  phase: "insertion" | "removal",
  acc: { opacity?: string; transform: string[] } = { transform: [] },
): { opacity?: string; transform: string[] } {
  switch (trans.type) {
    case "opacity":
      acc.opacity = "0";
      break;
    case "scale": {
      const s = typeof trans.scale === "number" ? trans.scale : 0.0001;
      acc.transform.push(`scale(${s})`);
      break;
    }
    case "slide":
      // Slide: in from the leading edge, out toward the trailing edge.
      acc.transform.push(phase === "insertion" ? "translateX(-100%)" : "translateX(100%)");
      break;
    case "move": {
      // Move: the view sits fully off toward `edge` at the hidden extreme.
      const edge = typeof trans.edge === "string" ? trans.edge : "leading";
      acc.transform.push(MOVE_OFFSET[edge] ?? "translateY(100%)");
      break;
    }
    case "offset": {
      const x = typeof trans.x === "number" ? trans.x : 0;
      const y = typeof trans.y === "number" ? trans.y : 0;
      acc.transform.push(`translate(${x}px, ${y}px)`);
      break;
    }
    case "combined": {
      const list = Array.isArray(trans.transitions) ? trans.transitions : [];
      for (const t of list) {
        const inner = asTrans(t as UiirValue);
        if (inner) hiddenStyle(inner, phase, acc);
      }
      break;
    }
    case "asymmetric": {
      // Insertion uses the `insertion` leg; removal uses the `removal` leg.
      const inner = asTrans(trans[phase] as UiirValue);
      if (inner) hiddenStyle(inner, phase, acc);
      break;
    }
    default:
      break; // identity / unknown → no offset.
  }
  return acc;
}

/** `.move(edge:)` → the transform that parks the view fully off that edge. */
const MOVE_OFFSET: Record<string, string> = {
  top: "translateY(-100%)",
  bottom: "translateY(100%)",
  leading: "translateX(-100%)",
  trailing: "translateX(100%)",
};

/** Read + parse the transition recorded on `el.dataset.transition`, if any. */
function readTransition(el: HTMLElement): TransObject | null {
  const raw = el.dataset.transition;
  if (!raw) return null;
  try {
    return asTrans(JSON.parse(raw) as UiirValue);
  } catch {
    return null;
  }
}

/** Compose a hidden-state style string pair (opacity + transform) for `phase`. */
function hiddenFor(trans: TransObject, phase: "insertion" | "removal") {
  const h = hiddenStyle(trans, phase);
  return {
    opacity: h.opacity,
    transform: h.transform.length ? h.transform.join(" ") : undefined,
  };
}

/**
 * Animate `el` *in* per its recorded `.transition` (called from `apply-patch`'s
 * `insert`). The node is built in its final state; we snap it to the transition's
 * hidden start, force a reflow, then tween back to the natural state. No-op when
 * the node carries no transition, so plain inserts stay instant.
 */
export function playTransitionEnter(el: HTMLElement): void {
  const trans = readTransition(el);
  if (!trans) return;
  const from = hiddenFor(trans, "insertion");
  if (from.opacity === undefined && from.transform === undefined) return;

  // Natural (target) values the tween lands on — whatever the modifiers set.
  const toOpacity = el.style.opacity || "1";
  const toTransform = el.style.transform || "none";

  // Snap to the hidden start with transitions disabled, then force layout so the
  // browser registers the start before we arm the tween.
  el.style.transition = "none";
  if (from.opacity !== undefined) el.style.opacity = from.opacity;
  if (from.transform !== undefined) el.style.transform = from.transform;
  void el.offsetWidth; // reflow

  el.style.transition = `opacity ${TRANSITION_DURATION}s ${TRANSITION_TIMING}, transform ${TRANSITION_DURATION}s ${TRANSITION_TIMING}`;
  requestAnimationFrame(() => {
    el.style.opacity = toOpacity;
    el.style.transform = toTransform;
  });
}

/**
 * Animate `el` *out* per its recorded `.transition`, invoking `done()` (the
 * actual DOM removal) once the tween finishes. When the node carries no
 * transition, `done()` runs synchronously so plain removes stay instant.
 */
export function playTransitionLeave(el: HTMLElement, done: () => void): void {
  const trans = readTransition(el);
  const to = trans ? hiddenFor(trans, "removal") : { opacity: undefined, transform: undefined };
  if (!trans || (to.opacity === undefined && to.transform === undefined)) {
    done();
    return;
  }

  let finished = false;
  const finish = () => {
    if (finished) return;
    finished = true;
    done();
  };

  el.style.transition = `opacity ${TRANSITION_DURATION}s ${TRANSITION_TIMING}, transform ${TRANSITION_DURATION}s ${TRANSITION_TIMING}`;
  el.addEventListener("transitionend", finish, { once: true });
  // Safety net: if `transitionend` never fires (e.g. no visual change), remove
  // after the nominal duration anyway.
  setTimeout(finish, TRANSITION_DURATION * 1000 + 80);
  requestAnimationFrame(() => {
    if (to.opacity !== undefined) el.style.opacity = to.opacity;
    if (to.transform !== undefined) el.style.transform = to.transform;
  });
}

