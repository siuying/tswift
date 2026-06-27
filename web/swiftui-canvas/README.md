# `<swiftui-canvas>` — the web render host

A dependency-free Web Component that renders the tswift SwiftUI **UIIR** and
applies **patch streams** from the Rust diff engine. No React, no vdom — the
runtime owns reconciliation; this host is a thin `applyPatch` over a
`Map<nodeId, HTMLElement>` inside a Shadow DOM (see
`docs/adr/0006-swiftui-render-host.md`).

## Modules

- `modifier-css.ts` — the SwiftUI-modifier → CSS design system. Resolves semantic
  tokens (`{"$":"color","name":"blue"}` → `#007aff`) host-side. iOS-vs-web drift
  lives here by design.
- `apply-patch.ts` — `PatchApplier`: owns the id→element map, lowers UIIR concepts
  to DOM primitives, and applies `mount`/`insert`/`remove`/`replace`/`setText`/
  `setModifiers`/`setArgs`.
- `canvas.ts` — the `<swiftui-canvas>` custom element (Shadow DOM for CSS
  isolation). Emits a `swiftui-event` CustomEvent on interaction.

## Wiring

```ts
const canvas = document.querySelector("swiftui-canvas")!;
canvas.mount(await renderTree());                  // initial UIIR tree
canvas.addEventListener("swiftui-event", async (e) => {
  const patches = await dispatch(e.detail);        // runtime → patch stream
  canvas.applyPatches(patches);
});
```

The driver (`renderTree`/`dispatch`) is transport-agnostic: in the browser it is
the wasm `SwiftUISession`; offline it is `tswift swiftui render|dispatch`.

## Offline check

`validate.mjs` loads the committed Layer B/C goldens and asserts they conform to
the protocol this host consumes (no browser needed):

```sh
node web/swiftui-canvas/validate.mjs
```
