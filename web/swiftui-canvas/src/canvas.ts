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

      /* ── iOS theme ──────────────────────────────────────────────
         Opt-in (\`theme="ios"\`) skin that restyles the control primitives to
         resemble iOS. Scoped under :host([theme="ios"]) so the default skin —
         and its snapshot baselines — is untouched. Controls are matched by the
         \`data-kind\` PatchApplier sets on each element. A SwiftUI modifier
         (e.g. .background(_)) is applied inline and still wins, except where a
         field's base box needs !important to beat its inline default. */
      :host([theme="ios"]) { font-family: -apple-system, system-ui, sans-serif; }

      :host([theme="ios"]) [data-kind="Button"] {
        color: #007aff;
        font-size: 17px;
        padding: 4px 6px;
        border-radius: 8px;
      }
      :host([theme="ios"]) [data-kind="Button"]:active { opacity: 0.4; }

      /* Toggle → iOS switch */
      :host([theme="ios"]) [data-kind="Toggle"] { justify-content: space-between; width: 100%; }
      :host([theme="ios"]) [data-kind="Toggle"] input[type="checkbox"] {
        appearance: none;
        -webkit-appearance: none;
        position: relative;
        flex: none;
        width: 51px;
        height: 31px;
        border-radius: 31px;
        background: #e9e9ea;
        transition: background 0.2s ease;
        cursor: pointer;
      }
      :host([theme="ios"]) [data-kind="Toggle"] input[type="checkbox"]::after {
        content: "";
        position: absolute;
        top: 2px;
        left: 2px;
        width: 27px;
        height: 27px;
        border-radius: 50%;
        background: #ffffff;
        box-shadow: 0 1px 3px rgba(0, 0, 0, 0.3);
        transition: transform 0.2s ease;
      }
      :host([theme="ios"]) [data-kind="Toggle"] input[type="checkbox"]:checked { background: #34c759; }
      :host([theme="ios"]) [data-kind="Toggle"] input[type="checkbox"]:checked::after { transform: translateX(20px); }

      /* TextField / SecureField → rounded field (overrides the inline base box) */
      :host([theme="ios"]) [data-kind="TextField"],
      :host([theme="ios"]) [data-kind="SecureField"] {
        padding: 7px 11px !important;
        border: 0.5px solid #c6c6c8 !important;
        border-radius: 10px !important;
        background: var(--swiftui-system-background) !important;
        color: var(--swiftui-label) !important;
        font-size: 17px !important;
        outline: none;
      }

      /* Slider → thin track with a blue fill + white knob */
      :host([theme="ios"]) [data-kind="Slider"] {
        appearance: none;
        -webkit-appearance: none;
        height: 4px;
        border-radius: 2px;
        cursor: pointer;
        background: linear-gradient(
          to right,
          #007aff var(--swiftui-slider-fill, 0%),
          #d1d1d6 var(--swiftui-slider-fill, 0%)
        );
      }
      :host([theme="ios"]) [data-kind="Slider"]::-webkit-slider-thumb {
        -webkit-appearance: none;
        width: 27px;
        height: 27px;
        border-radius: 50%;
        background: #ffffff;
        box-shadow: 0 1px 4px rgba(0, 0, 0, 0.25), 0 0 1px rgba(0, 0, 0, 0.2);
        margin-top: -11.5px;
      }
      :host([theme="ios"]) [data-kind="Slider"]::-moz-range-thumb {
        width: 27px;
        height: 27px;
        border: none;
        border-radius: 50%;
        background: #ffffff;
        box-shadow: 0 1px 4px rgba(0, 0, 0, 0.25);
      }

      /* Stepper → joined −/+ segmented control */
      :host([theme="ios"]) [data-kind="Stepper"] { justify-content: space-between; width: 100%; }
      :host([theme="ios"]) [data-kind="Stepper"] button {
        width: 46px;
        height: 32px;
        font-size: 20px;
        color: var(--swiftui-label);
        background: rgba(120, 120, 128, 0.16);
      }
      :host([theme="ios"]) [data-kind="Stepper"] button:nth-of-type(1) {
        border-radius: 8px 0 0 8px;
        box-shadow: 1px 0 0 rgba(120, 120, 128, 0.3);
      }
      :host([theme="ios"]) [data-kind="Stepper"] button:nth-of-type(2) { border-radius: 0 8px 8px 0; }

      /* Picker → tinted menu-style control */
      :host([theme="ios"]) [data-kind="Picker"] {
        font-size: 15px;
        color: #007aff;
        background: rgba(120, 120, 128, 0.12);
        border: none;
        border-radius: 8px;
        padding: 6px 10px;
      }
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
