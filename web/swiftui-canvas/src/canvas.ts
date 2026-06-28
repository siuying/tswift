// canvas.ts — the `<swiftui-canvas>` custom element (plan §2, decision 5). A
// Shadow-DOM host that renders a UIIR tree and applies patch streams, isolated
// from host-page CSS. The element is transport-agnostic: a driver (wasm session
// or a fetch to the CLI) feeds it `mount(tree)` then `applyPatches(stream)` and
// listens for `swiftui-event`s to round-trip back to the runtime.

import { PatchApplier, mountPatch } from "./apply-patch.js";
import type { Patch, UiirNode } from "./apply-patch.js";

/** Detail of the `swiftui-event` CustomEvent dispatched on user interaction. */
export interface SwiftUIEventDetail {
  id: string;
  event: string;
  value: unknown;
}

export class SwiftUICanvas extends HTMLElement {
  private readonly mountPoint: HTMLDivElement;
  private readonly applier: PatchApplier;

  constructor() {
    super();
    const shadow = this.attachShadow({ mode: "open" });
    const style = document.createElement("style");
    // Encapsulated baseline: the SwiftUI-modifier→CSS system can't leak out and
    // host-page styles can't leak in.
    // Semantic colors (.primary/.secondary, default label, systemBackground)
    // are dynamic on iOS: they adapt to light/dark. We mirror that with CSS
    // custom properties overridden under `prefers-color-scheme: dark`, so the
    // same UIIR renders correctly in both appearances (and the dark/light
    // screenshots line up with the native ones).
    style.textContent = `
      :host {
        display: block;
        font-family: -apple-system, system-ui, sans-serif;
        --swiftui-label: #000000;
        --swiftui-label-secondary: rgba(60, 60, 67, 0.6);
        --swiftui-system-background: #ffffff;
        color: var(--swiftui-label);
        background: var(--swiftui-system-background);
      }
      @media (prefers-color-scheme: dark) {
        /* Auto appearance only: an explicit \`appearance\` attribute opts out and
           pins the scheme below, so a host can force a fixed device appearance
           regardless of the OS setting. */
        :host(:not([appearance])) {
          --swiftui-label: #ffffff;
          --swiftui-label-secondary: rgba(235, 235, 245, 0.6);
          --swiftui-system-background: #000000;
        }
      }
      /* Forced appearance. \`color-scheme\` also makes native controls
         (checkbox/select/range) render in the matching light/dark style. */
      :host([appearance="light"]) {
        color-scheme: light;
        --swiftui-label: #000000;
        --swiftui-label-secondary: rgba(60, 60, 67, 0.6);
        --swiftui-system-background: #ffffff;
      }
      :host([appearance="dark"]) {
        color-scheme: dark;
        --swiftui-label: #ffffff;
        --swiftui-label-secondary: rgba(235, 235, 245, 0.6);
        --swiftui-system-background: #000000;
      }
      .root { display: flex; justify-content: center; padding: 16px; }
      button { font: inherit; border: none; background: transparent; cursor: pointer; color: inherit; }
    `;
    this.mountPoint = document.createElement("div");
    this.mountPoint.className = "root";
    this.mountPoint.part.add("root");
    shadow.append(style, this.mountPoint);

    this.applier = new PatchApplier(this.mountPoint, (id, event, value) => {
      this.dispatchEvent(
        new CustomEvent<SwiftUIEventDetail>("swiftui-event", {
          detail: { id, event, value },
          bubbles: true,
          composed: true,
        }),
      );
    });
  }

  /** Render an initial UIIR tree (equivalent to applying a single mount). */
  mount(tree: UiirNode): void {
    this.applier.apply(mountPatch(tree));
  }

  /** Apply a patch stream from `tswift swiftui dispatch`. */
  applyPatches(patches: Patch[]): void {
    this.applier.apply(patches);
  }
}

if (typeof customElements !== "undefined" && !customElements.get("swiftui-canvas")) {
  customElements.define("swiftui-canvas", SwiftUICanvas);
}
