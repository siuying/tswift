// Layer D web harness driver. Exposes a minimal imperative API on `window` so
// the Playwright spec can drive the real <swiftui-canvas> element through the
// same `mount(tree)` → `applyPatches(step)` loop as the iOS renderer harness
// (ios/UiirRenderer). No editor, no Swift parsing — fixtures arrive pre-built.

import "../../src/canvas.ts";
import type { Patch, UiirNode } from "../../src/index.ts";

interface SwiftUICanvasElement extends HTMLElement {
  mount(tree: UiirNode): void;
  applyPatches(patches: Patch[]): void;
}

declare global {
  interface Window {
    harness: {
      mount(tree: UiirNode): void;
      applyPatches(patches: Patch[]): void;
    };
    harnessReady: boolean;
  }
}

const canvas = document.querySelector<SwiftUICanvasElement>("#canvas");
if (!canvas) throw new Error("harness: #canvas element missing");

window.harness = {
  mount: (tree) => canvas.mount(tree),
  applyPatches: (patches) => canvas.applyPatches(patches),
};
window.harnessReady = true;
