# SwiftUI Sandbox

## Presets

- Editor presets live as real `.swift` files in `src/presets/`, ordered and
  labelled by `src/presets/manifest.mjs` (copies of `tests/swiftui-fixtures/`).
  The Astro page loads them through the Vite `?raw` glob in
  `src/presets/index.js`; `test/wasm-smoke.mjs` reads the same files via the
  manifest.
- To add/change a preset: drop/edit the `.swift` file and update `manifest.mjs`.
  Both the page menu and the smoke test pick it up automatically. Mark a preset
  `supported: false` to keep its source but hide it from the UI and smoke run.

## Rendering

- The DOM is rendered by the shared **`web/swiftui-canvas`** package — do **not**
  add a renderer here. `swiftUICompile` returns a UIIR tree fed to
  `<swiftui-canvas>.mount(...)`; `swiftUIDispatch` returns a patch stream fed to
  `.applyPatches(...)`. The page (`src/pages/index.astro`) is only editor + wasm
  glue + device chrome.
- When the runtime gains a new SwiftUI view kind or modifier, teach the
  **canvas** about it (`web/swiftui-canvas/src/apply-patch.ts` for the DOM
  primitive, `src/modifier-css.ts` for styling) so every surface benefits, then
  add a preset here to exercise it.
- The wasm entry points live in `crates/tswift-wasm/src/swiftui.rs`; the UIIR /
  patch wire format is defined by `crates/tswift-swiftui/src/{uiir,diff}.rs` and
  consumed by `web/swiftui-canvas/src/apply-patch.ts`.
