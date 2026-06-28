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

- `src/renderer.js` turns the UIIR JSON (from `swiftUICompile` /
  `swiftUIDispatch`) into DOM. When the runtime gains a new SwiftUI view kind or
  modifier, add a `case` here (and a CSS rule in `src/styles/global.css`) — an
  unhandled kind renders as `⟨Kind⟩`.
- The wasm entry points live in `crates/tswift-wasm/src/swiftui.rs`; the UIIR
  wire format is defined by `crates/tswift-swiftui/src/uiir.rs`.
