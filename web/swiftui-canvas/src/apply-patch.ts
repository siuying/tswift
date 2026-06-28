// apply-patch.ts — the thin patch applier (plan §3.2/§4). It owns a
// `Map<nodeId, HTMLElement>` and mutates the host tree from the Rust diff
// engine's keyed patch stream. No vdom, no reconciler — the runtime already
// did the diffing.

import { applyModifiers } from "./modifier-css.js";
import type { Modifier } from "./modifier-css.js";
import { sfGlyph } from "./sf-symbols.js";

/** A UIIR node from `tswift swiftui render` / patch payloads. */
export interface UiirNode {
  id: string;
  kind: string;
  args: Record<string, unknown>;
  modifiers: Modifier[];
  children: UiirNode[];
}

/** A patch op from `tswift swiftui dispatch`. */
export type Patch =
  | { op: "mount"; node: UiirNode }
  | { op: "insert"; parentId: string; index: number; node: UiirNode }
  | { op: "remove"; id: string }
  | { op: "replace"; id: string; node: UiirNode }
  | { op: "setText"; id: string; text: string }
  | { op: "setModifiers"; id: string; modifiers: Modifier[] }
  | { op: "setArgs"; id: string; args: Record<string, unknown> }
  | { op: "move"; parentId: string; id: string; index: number };

/** How a host node reports an event back to the runtime. */
export type EventSink = (id: string, event: string, value: unknown) => void;

/**
 * Applies a patch stream into a root element, keeping an id→element map. One
 * instance per mounted `<swiftui-canvas>`.
 */
export class PatchApplier {
  private nodes = new Map<string, HTMLElement>();
  /** Last-applied modifier list per node, so `setArgs` can rebuild a node's
   * base style from scratch and re-apply modifiers without ever capturing
   * modifier CSS into the base. */
  private mods = new Map<string, Modifier[]>();

  constructor(
    private readonly root: HTMLElement,
    private readonly emit: EventSink,
  ) {}

  /** Apply an ordered batch of patches. */
  apply(patches: Patch[]): void {
    for (const patch of patches) this.applyOne(patch);
  }

  private applyOne(patch: Patch): void {
    switch (patch.op) {
      case "mount": {
        this.root.replaceChildren();
        this.nodes.clear();
        this.mods.clear();
        this.root.appendChild(this.build(patch.node));
        break;
      }
      case "insert": {
        const parent = this.nodes.get(patch.parentId);
        if (!parent) return;
        // A child inserted under a Picker is an <option>, not a nested view.
        const el =
          parent instanceof HTMLSelectElement
            ? this.buildOption(patch.node)
            : this.build(patch.node);
        // A child inserted under a ZStack must overlap like the others.
        if (parent.dataset.zstack === "1") el.style.gridArea = "1 / 1";
        parent.insertBefore(el, patchRef(parent, patch.index));
        break;
      }
      case "remove": {
        const el = this.nodes.get(patch.id);
        el?.remove();
        this.forget(patch.id);
        break;
      }
      case "move": {
        // Keyed reorder: relocate the existing element (preserving its DOM
        // node and any host state) to index `index`. The target is computed
        // among the *other* children so it is correct whether the element
        // moves left or right (insertBefore implicitly removes it first).
        const parent = this.nodes.get(patch.parentId);
        const el = this.nodes.get(patch.id);
        if (!parent || !el) return;
        const ref = patchRef(parent, patch.index, el);
        if (ref !== el) parent.insertBefore(el, ref);
        break;
      }
      case "replace": {
        const old = this.nodes.get(patch.id);
        if (!old) return;
        // Forget the old subtree's ids *before* building the replacement, which
        // re-registers the same ids — otherwise `forget` would delete the new
        // entries and later patches/events for the subtree become no-ops.
        this.forget(patch.id);
        // Replacing a Picker option keeps it an <option>.
        const el =
          old instanceof HTMLOptionElement
            ? this.buildOption(patch.node)
            : this.build(patch.node);
        old.replaceWith(el);
        break;
      }
      case "setText": {
        const el = this.nodes.get(patch.id);
        if (el) el.textContent = patch.text;
        break;
      }
      case "setModifiers": {
        const el = this.nodes.get(patch.id);
        if (!el) break;
        // A Picker option's identity is its `tag` value, not CSS styling.
        if (el instanceof HTMLOptionElement) {
          const tag = patch.modifiers.find((m) => m.name === "tag");
          if (tag && (typeof tag.value === "string" || typeof tag.value === "number")) {
            el.value = String(tag.value);
          }
        } else {
          applyModifiers(el, patch.modifiers);
          this.mods.set(patch.id, patch.modifiers);
        }
        break;
      }
      case "setArgs": {
        const el = this.nodes.get(patch.id);
        if (el) {
          // Rebuild from the pristine intrinsic style so arg-owned styling
          // (stack `gap`, Spacer `flex-basis`, …) is total: removed args revert
          // to their defaults instead of leaking. Then recompute the base and
          // re-apply the remembered modifiers — never capturing modifier CSS.
          el.style.cssText = el.dataset.intrinsicStyle ?? "";
          this.applyArgs(el, el.dataset.kind ?? "", patch.args);
          el.dataset.baseStyle = el.style.cssText;
          applyModifiers(el, this.mods.get(patch.id) ?? []);
        }
        break;
      }
    }
  }

  /** Drop `id` and any descendant ids from the node map. */
  private forget(id: string): void {
    const prefix = `${id}.`;
    for (const key of [...this.nodes.keys()]) {
      if (key === id || key.startsWith(prefix)) {
        this.nodes.delete(key);
        this.mods.delete(key);
      }
    }
  }

  /** Build a DOM subtree for a UIIR node and register it (and descendants). */
  private build(node: UiirNode): HTMLElement {
    const el = this.element(node);
    el.dataset.kind = node.kind;
    // Pristine style straight from `element()` — no args, no modifiers — so
    // `setArgs` can revert arg-owned style to defaults.
    el.dataset.intrinsicStyle = el.style.cssText;
    this.applyArgs(el, node.kind, node.args);
    // Capture the base style *after* args so arg-derived styling (e.g. a
    // RoundedRectangle's corner radius) survives the idempotent reset that
    // `applyModifiers`/`setModifiers` performs.
    el.dataset.baseStyle = el.style.cssText;
    applyModifiers(el, node.modifiers);
    this.mods.set(node.id, node.modifiers);
    this.nodes.set(node.id, el);
    if (node.kind === "Picker" && el instanceof HTMLSelectElement) {
      // A Picker's tagged children become <option>s, not nested views.
      for (const child of node.children) {
        el.appendChild(this.buildOption(child));
      }
      if (typeof node.args.selection === "string") el.value = node.args.selection;
    } else {
      for (const child of node.children) {
        const childEl = this.build(child);
        // ZStack overlays its children: place each in the same grid cell.
        if (el.dataset.zstack === "1") childEl.style.gridArea = "1 / 1";
        el.appendChild(childEl);
      }
    }
    return el;
  }

  /** Build a Picker `<option>` from a tagged child view (label + tag value). */
  private buildOption(child: UiirNode): HTMLOptionElement {
    const opt = document.createElement("option");
    const tag = child.modifiers.find((m) => m.name === "tag");
    if (tag && (typeof tag.value === "string" || typeof tag.value === "number")) {
      opt.value = String(tag.value);
    } else {
      // Untagged option: an empty value, so selecting it can't write the label
      // text into the binding.
      opt.value = "";
    }
    if (typeof child.args.verbatim === "string") opt.textContent = child.args.verbatim;
    this.nodes.set(child.id, opt);
    return opt;
  }

  /** Create the host primitive for a SwiftUI concept (the lowering boundary). */
  private element(node: UiirNode): HTMLElement {
    switch (node.kind) {
      case "VStack": {
        const el = document.createElement("div");
        el.style.cssText = "display:flex;flex-direction:column;align-items:center;gap:8px;";
        return el;
      }
      case "HStack": {
        const el = document.createElement("div");
        el.style.cssText = "display:flex;flex-direction:row;align-items:center;gap:8px;";
        return el;
      }
      case "ZStack": {
        // Depth container: children overlap, centered, back-to-front.
        const el = document.createElement("div");
        el.style.cssText = "display:grid;place-items:center;";
        el.dataset.zstack = "1";
        return el;
      }
      case "ForEach":
      case "Group": {
        // A transparent group: its children lay out as if direct children of
        // the surrounding container (no box of its own).
        const el = document.createElement("div");
        el.style.cssText = "display:contents;";
        return el;
      }
      case "Divider": {
        // A thin rule; stretches across the container's cross axis.
        const el = document.createElement("div");
        el.style.cssText =
          "flex:0 0 auto;align-self:stretch;min-width:1px;min-height:1px;background:var(--swiftui-separator, #e0e0e0);";
        return el;
      }
      case "ScrollView": {
        // A scroll container. Default axis is vertical; `axes` arg may switch it.
        const el = document.createElement("div");
        el.style.cssText = "display:flex;flex-direction:column;overflow-y:auto;overflow-x:hidden;";
        return el;
      }
      case "List": {
        // A vertically scrolling list of rows.
        const el = document.createElement("div");
        el.style.cssText =
          "display:flex;flex-direction:column;overflow-y:auto;border:1px solid #e0e0e0;border-radius:8px;";
        return el;
      }
      case "Section": {
        // A grouped block; its `header` arg renders as a caption above rows.
        const el = document.createElement("div");
        el.style.cssText = "display:flex;flex-direction:column;";
        return el;
      }
      case "Circle":
      case "Ellipse":
      case "Rectangle":
      case "RoundedRectangle":
      case "Capsule": {
        const el = document.createElement("div");
        el.style.cssText = `${shapeRadius(node)}background:currentColor;width:40px;height:40px;`;
        return el;
      }
      case "Spacer": {
        const el = document.createElement("div");
        el.style.cssText = "flex:1 1 auto;";
        return el;
      }
      case "Label": {
        // An icon + title row.
        const el = document.createElement("span");
        el.style.cssText = "display:inline-flex;align-items:center;gap:6px;";
        const icon = document.createElement("span");
        icon.className = "label-icon";
        const text = document.createElement("span");
        text.className = "label-text";
        el.append(icon, text);
        return el;
      }
      case "Image": {
        const el = document.createElement("span");
        el.style.cssText = "display:inline-flex;align-items:center;justify-content:center;";
        return el;
      }
      case "ProgressView": {
        // Native <progress>: determinate when `value` is set, else indeterminate.
        const el = document.createElement("progress");
        el.max = 1;
        return el;
      }
      case "Button": {
        const el = document.createElement("button");
        el.addEventListener("click", () => this.emit(node.id, "tap", null));
        return el;
      }
      case "Toggle": {
        // A labelled switch: a checkbox emits `set` with its new boolean.
        const el = document.createElement("label");
        el.style.cssText = "display:flex;flex-direction:row;align-items:center;gap:8px;";
        const input = document.createElement("input");
        input.type = "checkbox";
        input.addEventListener("change", () => this.emit(node.id, "set", input.checked));
        const text = document.createElement("span");
        el.append(input, text);
        return el;
      }
      case "TextField":
      case "SecureField": {
        // A text input emitting `set` with its string value on each edit. The
        // base box (padding/border) lives in the stylesheet, not inline, so a
        // theme or a user modifier can override it without `!important`.
        const input = document.createElement("input");
        input.type = node.kind === "SecureField" ? "password" : "text";
        input.addEventListener("input", () =>
          this.emit(node.id, "set", input.value),
        );
        return input;
      }
      case "Slider": {
        // A range input emitting `set` with its numeric value as the user drags.
        const input = document.createElement("input");
        input.type = "range";
        input.addEventListener("input", () =>
          this.emit(node.id, "set", Number(input.value)),
        );
        return input;
      }
      case "Stepper": {
        // A label plus -/+ buttons; each click computes the clamped next value
        // from the node's args and emits `set` with it.
        const el = document.createElement("div");
        el.style.cssText = "display:flex;flex-direction:row;align-items:center;gap:8px;";
        const label = document.createElement("span");
        label.className = "stepper-label";
        const dec = document.createElement("button");
        dec.textContent = "\u2212";
        const inc = document.createElement("button");
        inc.textContent = "+";
        const bump = (dir: number) => {
          // The current value/step/bounds live on the element's dataset, kept
          // fresh by `applyArgs`. We optimistically advance the dataset before
          // emitting so a rapid second click computes from the new value (not
          // the stale one that a not-yet-applied `setArgs` would carry); the
          // runtime's authoritative `setArgs` reconciles afterward.
          const cur = Number(el.dataset.value ?? "0");
          const step = Number(el.dataset.step ?? "1");
          let next = cur + dir * step;
          if (el.dataset.lowerBound !== undefined)
            next = Math.max(next, Number(el.dataset.lowerBound));
          if (el.dataset.upperBound !== undefined)
            next = Math.min(next, Number(el.dataset.upperBound));
          if (next !== cur) {
            el.dataset.value = String(next);
            this.emit(node.id, "set", next);
          }
        };
        dec.addEventListener("click", () => bump(-1));
        inc.addEventListener("click", () => bump(1));
        el.append(label, dec, inc);
        return el;
      }
      case "Picker": {
        // A choice control: a <select> whose options are the tagged children.
        // It emits `set` with the chosen option's tag value.
        const sel = document.createElement("select");
        sel.addEventListener("change", () => this.emit(node.id, "set", sel.value));
        return sel;
      }
      case "Text":
      default:
        return document.createElement("span");
    }
  }

  /** Reflect constructor args onto the host node (text/title/checked state). */
  private applyArgs(el: HTMLElement, kind: string, args: Record<string, unknown>): void {
    if (kind === "Text" && typeof args.verbatim === "string") {
      el.textContent = args.verbatim;
    } else if (kind === "VStack" || kind === "HStack" || kind === "ZStack") {
      // `spacing:` overrides the default inter-child gap (C2). ZStack ignores it.
      if (typeof args.spacing === "number" && kind !== "ZStack") {
        el.style.gap = `${args.spacing}px`;
      }
    } else if (kind === "Spacer") {
      // `minLength:` is the spacer's minimum length along the stack axis (C2).
      if (typeof args.minLength === "number") {
        el.style.flexBasis = `${args.minLength}px`;
      }
    } else if (kind === "Label") {
      const icon = el.querySelector(".label-icon");
      const text = el.querySelector(".label-text");
      if (icon) icon.textContent = typeof args.systemImage === "string" ? sfGlyph(args.systemImage) : "";
      if (text) text.textContent = typeof args.title === "string" ? args.title : "";
    } else if (kind === "Image") {
      if (typeof args.systemName === "string") {
        el.textContent = sfGlyph(args.systemName);
        el.removeAttribute("title"); // clear any stale asset-name tooltip
      } else if (typeof args.name === "string") {
        // No asset bundle on the web; show a placeholder labelled by name.
        el.textContent = "\u{1F5BC}"; // 🖼
        el.title = args.name;
      } else {
        el.textContent = "";
        el.removeAttribute("title");
      }
    } else if (kind === "ProgressView" && el instanceof HTMLProgressElement) {
      if (typeof args.value === "number") {
        el.max = typeof args.total === "number" ? args.total : 1;
        el.value = args.value;
      } else {
        el.removeAttribute("value"); // indeterminate
      }
    } else if (kind === "ScrollView") {
      // `axes` switches the scroll direction (C3); default is vertical.
      const axis = args.axes as { name?: string } | undefined;
      if (axis?.name === "horizontal") {
        el.style.flexDirection = "row";
        el.style.overflowX = "auto";
        el.style.overflowY = "hidden";
      }
    } else if (kind === "Button" && typeof args.title === "string") {
      el.textContent = args.title;
    } else if (kind === "RoundedRectangle" && typeof args.cornerRadius === "number") {
      el.style.borderRadius = `${args.cornerRadius}px`;
    } else if (kind === "Section") {
      // The header caption is a synthetic child, kept out of the patch-addressed
      // child list (see `patchRef`). Add/update it, or remove it if the arg is
      // gone, without disturbing row positions.
      let head = el.querySelector(":scope > .section-header") as HTMLElement | null;
      if (typeof args.header === "string") {
        if (!head) {
          head = document.createElement("div");
          head.className = "section-header";
          head.style.cssText =
            "font-size:13px;font-weight:600;text-transform:uppercase;color:#6b6b6b;padding:8px 12px;";
          el.prepend(head);
        }
        head.textContent = args.header;
      } else {
        head?.remove();
      }
    } else if (kind === "Toggle") {
      const input = el.querySelector("input");
      const label = el.querySelector("span");
      if (input instanceof HTMLInputElement && typeof args.isOn === "boolean") {
        input.checked = args.isOn;
      }
      if (label && typeof args.title === "string") {
        label.textContent = args.title;
      }
    } else if (kind === "TextField" || kind === "SecureField") {
      if (el instanceof HTMLInputElement) {
        // Only overwrite when the model and DOM disagree, so we don't fight the
        // user's caret mid-edit on the echo back.
        if (typeof args.text === "string" && el.value !== args.text) {
          el.value = args.text;
        }
        if (typeof args.title === "string") el.placeholder = args.title;
      }
    } else if (kind === "Slider" && el instanceof HTMLInputElement) {
      if (typeof args.lowerBound === "number") el.min = String(args.lowerBound);
      if (typeof args.upperBound === "number") el.max = String(args.upperBound);
      // Always set step: `"any"` restores continuous dragging when a previous
      // `step:` is removed, instead of leaving the browser default of `1`.
      el.step = typeof args.step === "number" ? String(args.step) : "any";
      if (typeof args.value === "number" && Number(el.value) !== args.value) {
        el.value = String(args.value);
      }
      // Expose the value as a 0–100% of the range so a theme can paint a filled
      // track (e.g. iOS). Theme-agnostic and unused by the default skin, so it
      // does not affect the default rendering.
      const lo = Number(el.min || "0");
      const hi = Number(el.max || "1");
      const cur = Number(el.value || "0");
      const pct = hi > lo ? ((cur - lo) / (hi - lo)) * 100 : 0;
      el.style.setProperty("--swiftui-slider-fill", `${pct}%`);
    } else if (kind === "Picker" && el instanceof HTMLSelectElement) {
      // Options are built in `build`; here we only reflect the active tag.
      if (typeof args.selection === "string") el.value = args.selection;
    } else if (kind === "Stepper") {
      // Stash the live value/step/bounds for the button handlers; reflect label.
      el.dataset.value = String(typeof args.value === "number" ? args.value : 0);
      el.dataset.step = String(typeof args.step === "number" ? args.step : 1);
      if (typeof args.lowerBound === "number") el.dataset.lowerBound = String(args.lowerBound);
      else delete el.dataset.lowerBound;
      if (typeof args.upperBound === "number") el.dataset.upperBound = String(args.upperBound);
      else delete el.dataset.upperBound;
      const label = el.querySelector(".stepper-label");
      if (label && typeof args.title === "string") {
        label.textContent =
          typeof args.value === "number" ? `${args.title}: ${args.value}` : args.title;
      }
    }
  }
}

/**
 * The DOM node before which a positional `insert`/`move` at logical `index`
 * should land, addressing only patch-managed children and skipping synthetic
 * nodes (a `Section`'s `.section-header`) and an optionally excluded element
 * (the node being moved). Returns `null` to append at the end.
 */
function patchRef(
  parent: HTMLElement,
  index: number,
  exclude?: Element,
): Node | null {
  const addressable = Array.from(parent.children).filter(
    (c) => c !== exclude && !c.classList.contains("section-header"),
  );
  return addressable[index] ?? null;
}

/** The intrinsic corner radius for a shape primitive, as a CSS declaration. */
function shapeRadius(node: UiirNode): string {
  switch (node.kind) {
    case "Circle":
    case "Ellipse":
      return "border-radius:50%;";
    case "Capsule":
      return "border-radius:9999px;";
    default:
      return "";
  }
}

/** Build the initial mount patch from a rendered UIIR tree. */
export function mountPatch(tree: UiirNode): Patch[] {
  return [{ op: "mount", node: tree }];
}
