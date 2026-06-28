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
      /* Default text-field base box. Kept in the stylesheet (not inline) so a
         theme rule or a user SwiftUI modifier overrides it without \`!important\`
         (inline modifier styles always beat these). */
      [data-kind="TextField"], [data-kind="SecureField"] {
        padding: 8px; border: 1px solid #ccc; border-radius: 6px; font: inherit;
      }

      /* ── iOS theme ──────────────────────────────────────────────
         Opt-in (\`theme="ios"\`) skin that restyles the control primitives to
         resemble iOS. Scoped under :host([theme="ios"]) so the default skin is
         untouched. Controls are matched by the \`data-kind\` PatchApplier sets
         on each element; the palette is held in --ios-* custom properties that
         adapt to the active appearance, and every rule is overridable by an
         inline SwiftUI modifier (no !important). */
      :host([theme="ios"]) {
        font-family: -apple-system, system-ui, sans-serif;
        --ios-tint: #007aff;
        --ios-switch-off: #e9e9ea;
        --ios-toggle-on: #34c759;
        --ios-track: #d1d1d6;
        --ios-field-border: #c6c6c8;
        --ios-fill: rgba(120, 120, 128, 0.12);
        --ios-fill-strong: rgba(120, 120, 128, 0.16);
      }
      /* iOS palette in dark: auto (no \`appearance\`) under a dark OS, or forced. */
      @media (prefers-color-scheme: dark) {
        :host([theme="ios"]:not([appearance="light"])) {
          --ios-tint: #0a84ff;
          --ios-switch-off: #39393d;
          --ios-toggle-on: #30d158;
          --ios-track: #48484a;
          --ios-field-border: #38383a;
          --ios-fill: rgba(120, 120, 128, 0.24);
          --ios-fill-strong: rgba(120, 120, 128, 0.32);
        }
      }
      :host([theme="ios"][appearance="dark"]) {
        --ios-tint: #0a84ff;
        --ios-switch-off: #39393d;
        --ios-toggle-on: #30d158;
        --ios-track: #48484a;
        --ios-field-border: #38383a;
        --ios-fill: rgba(120, 120, 128, 0.24);
        --ios-fill-strong: rgba(120, 120, 128, 0.32);
      }

      :host([theme="ios"]) [data-kind="Button"] {
        color: var(--ios-tint);
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
        background: var(--ios-switch-off);
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
      :host([theme="ios"]) [data-kind="Toggle"] input[type="checkbox"]:checked { background: var(--ios-toggle-on); }
      :host([theme="ios"]) [data-kind="Toggle"] input[type="checkbox"]:checked::after { transform: translateX(20px); }

      /* TextField / SecureField → rounded field. Higher specificity than the
         default base rule above; a user modifier (inline) still wins. */
      :host([theme="ios"]) [data-kind="TextField"],
      :host([theme="ios"]) [data-kind="SecureField"] {
        padding: 7px 11px;
        border: 0.5px solid var(--ios-field-border);
        border-radius: 10px;
        background: var(--swiftui-system-background);
        color: var(--swiftui-label);
        font-size: 17px;
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
          var(--ios-tint) var(--swiftui-slider-fill, 0%),
          var(--ios-track) var(--swiftui-slider-fill, 0%)
        );
      }
      :host([theme="ios"]) [data-kind="Slider"]::-moz-range-track {
        height: 4px;
        border-radius: 2px;
        background: linear-gradient(
          to right,
          var(--ios-tint) var(--swiftui-slider-fill, 0%),
          var(--ios-track) var(--swiftui-slider-fill, 0%)
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
        background: var(--ios-fill-strong);
      }
      :host([theme="ios"]) [data-kind="Stepper"] button:nth-of-type(1) {
        border-radius: 8px 0 0 8px;
        box-shadow: 1px 0 0 rgba(120, 120, 128, 0.3);
      }
      :host([theme="ios"]) [data-kind="Stepper"] button:nth-of-type(2) { border-radius: 0 8px 8px 0; }

      /* Picker → tinted menu-style control */
      :host([theme="ios"]) [data-kind="Picker"] {
        font-size: 15px;
        color: var(--ios-tint);
        background: var(--ios-fill);
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
