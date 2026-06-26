# Web Sandbox

## Presets

- Editor presets live as real `.swift` files in `src/presets/`, ordered and
  labelled by `src/presets/manifest.mjs`. The Astro page loads them through the
  Vite `?raw` glob in `src/presets/index.js`; the `wasm-smoke.mjs` test reads
  the same files via the manifest.
- To add or change a preset: drop/edit the `.swift` file and update
  `manifest.mjs`. Both the page and the smoke test pick it up automatically — no
  edits to `src/pages/index.astro` needed.
- `wasm-smoke.mjs` runs **every** supported preset through the compiled wasm, so
  a new preset is covered as soon as it is in the manifest. Mark a preset
  `supported: false` in the manifest to keep its source on disk but hide it from
  the UI and the smoke run.
