# `<swiftui-canvas>` — the web render host

A dependency-free Web Component that renders the tswift SwiftUI **UIIR** and
applies **patch streams** from the Rust diff engine. No React, no vdom — the
runtime owns reconciliation; this host is a thin `applyPatch` over a
`Map<nodeId, HTMLElement>` inside a Shadow DOM (see
`docs/adr/0006-swiftui-render-host.md`).

## Package layout

```text
web/swiftui-canvas/
  package.json
  tsconfig.json
  src/
    index.ts
    canvas.ts
    apply-patch.ts
    modifier-css.ts
  scripts/
    validate.mjs
  example/
    index.html
    src/
      main.ts
      styles.css
```

## Modules

- `src/modifier-css.ts` — the SwiftUI-modifier → CSS design system. Resolves
  semantic tokens (`{"$":"color","name":"blue"}` → `#007aff`) host-side.
  iOS-vs-web drift lives here by design.
- `src/apply-patch.ts` — `PatchApplier`: owns the id→element map, lowers UIIR
  concepts to DOM primitives, and applies `mount`/`insert`/`remove`/`replace`/
  `setText`/`setModifiers`/`setArgs`.
- `src/canvas.ts` — the `<swiftui-canvas>` custom element (Shadow DOM for CSS
  isolation). Emits a `swiftui-event` CustomEvent on interaction.
- `src/index.ts` — package exports for the host element, patch applier, and
  protocol types.

## Wiring

```ts
import "@tswift/swiftui-canvas/canvas";

const canvas = document.querySelector("swiftui-canvas")!;
canvas.mount(await renderTree());                  // initial UIIR tree
canvas.addEventListener("swiftui-event", async (e) => {
  const patches = await dispatch(e.detail);        // runtime → patch stream
  canvas.applyPatches(patches);
});
```

The driver (`renderTree`/`dispatch`) is transport-agnostic: in the browser it is
the wasm `SwiftUISession`; offline it is `tswift swiftui render|dispatch`.

## Development

```sh
cd web/swiftui-canvas
npm install
npm run build
npm run validate
npm run dev          # opens the editor + preview example on port 4322
```

`npm run validate` loads the committed Layer B/C goldens and asserts they conform
to the protocol this host consumes (no browser needed).
