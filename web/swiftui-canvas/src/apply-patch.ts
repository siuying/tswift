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
  | { op: "setArgs"; id: string; args: Record<string, unknown> };

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
        const ref = parent.children[patch.index] ?? null;
        parent.insertBefore(el, ref);
        break;
      }
      case "remove": {
        const el = this.nodes.get(patch.id);
        el?.remove();
        this.forget(patch.id);
        break;
      }
      case "replace": {
        const old = this.nodes.get(patch.id);
        if (!old) return;
        const el = this.build(patch.node);
        old.replaceWith(el);
        this.forget(patch.id);
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
        if (el) this.applyArgs(el, el.dataset.kind ?? "", patch.args);
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
    el.dataset.baseStyle = el.style.cssText;
    this.applyArgs(el, node.kind, node.args);
    applyModifiers(el, node.modifiers);
    this.nodes.set(node.id, el);
    for (const child of node.children) el.appendChild(this.build(child));
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
      case "Text":
      default:
        return document.createElement("span");
    }
  }

  /** Reflect constructor args onto the host node (text/title). */
  private applyArgs(el: HTMLElement, kind: string, args: Record<string, unknown>): void {
    if (kind === "Text" && typeof args.verbatim === "string") {
      el.textContent = args.verbatim;
    } else if (kind === "Button" && typeof args.title === "string") {
      el.textContent = args.title;
    }
  }
}

/** Build the initial mount patch from a rendered UIIR tree. */
export function mountPatch(tree: UiirNode): Patch[] {
  return [{ op: "mount", node: tree }];
}
