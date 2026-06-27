// apply-patch.ts — the thin patch applier (plan §3.2/§4). It owns a
// `Map<nodeId, HTMLElement>` and mutates the host tree from the Rust diff
// engine's keyed patch stream. No vdom, no reconciler — the runtime already
// did the diffing.

import { applyModifiers } from "./modifier-css.js";
import type { Modifier } from "./modifier-css.js";

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
        this.root.appendChild(this.build(patch.node));
        break;
      }
      case "insert": {
        const parent = this.nodes.get(patch.parentId);
        if (!parent) return;
        const el = this.build(patch.node);
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
        const el = this.build(patch.node);
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
        if (el) applyModifiers(el, patch.modifiers);
        break;
      }
      case "setArgs": {
        const el = this.nodes.get(patch.id);
        if (el) {
          this.applyArgs(el, el.dataset.kind ?? "", patch.args);
          // Keep arg-derived styling in the base so a later `setModifiers`
          // reset preserves it.
          el.dataset.baseStyle = el.style.cssText;
        }
        break;
      }
    }
  }

  /** Drop `id` and any descendant ids from the node map. */
  private forget(id: string): void {
    const prefix = `${id}.`;
    for (const key of [...this.nodes.keys()]) {
      if (key === id || key.startsWith(prefix)) this.nodes.delete(key);
    }
  }

  /** Build a DOM subtree for a UIIR node and register it (and descendants). */
  private build(node: UiirNode): HTMLElement {
    const el = this.element(node);
    el.dataset.kind = node.kind;
    this.applyArgs(el, node.kind, node.args);
    // Capture the base style *after* args so arg-derived styling (e.g. a
    // RoundedRectangle's corner radius) survives the idempotent reset that
    // `applyModifiers`/`setModifiers` performs.
    el.dataset.baseStyle = el.style.cssText;
    applyModifiers(el, node.modifiers);
    this.nodes.set(node.id, el);
    for (const child of node.children) {
      const childEl = this.build(child);
      // ZStack overlays its children: place each in the same grid cell.
      if (el.dataset.zstack === "1") childEl.style.gridArea = "1 / 1";
      el.appendChild(childEl);
    }
    return el;
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
      case "ForEach": {
        // A transparent group: its keyed rows lay out as if direct children of
        // the surrounding container.
        const el = document.createElement("div");
        el.style.cssText = "display:contents;";
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
      case "Text":
      default:
        return document.createElement("span");
    }
  }

  /** Reflect constructor args onto the host node (text/title/checked state). */
  private applyArgs(el: HTMLElement, kind: string, args: Record<string, unknown>): void {
    if (kind === "Text" && typeof args.verbatim === "string") {
      el.textContent = args.verbatim;
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
