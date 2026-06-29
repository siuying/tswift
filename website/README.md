# tswift website

The public-facing website for the tswift project, built with [Astro](https://astro.build).

## Pages

| Path | Description |
|------|-------------|
| `/` | Home — hero, live mini-playground, architecture overview, status summary |
| `/playground` | Full interactive playground (editor + output, all presets) |
| `/how-it-works` | Architecture overview with Mermaid pipeline diagram |
| `/how-it-works/frontend` | Frontend pipeline — lexer, parser, sema, compat lowerer |
| `/how-it-works/runtime` | Runtime evaluator — SwiftValue, ARC, CoW, closures, concurrency |
| `/how-it-works/libraries` | Standard library, Foundation, SwiftUI roadmap |
| `/status` | Status dashboard with coverage bars |
| `/status/language` | Language features (Tiers 0–9) from the feature checklist |
| `/status/stdlib` | Standard library coverage |
| `/status/foundation` | Foundation framework coverage |
| `/status/swiftui` | SwiftUI roadmap and blockers |

## Tech stack

- **Astro 7** — static site generator
- **MDX** — content pages (`@astrojs/mdx`) — easy to update
- **Mermaid** — architecture diagrams (CDN, no build step)
- **tswift WASM** — browser runtime in `/public/wasm/`

## Development

```sh
cd website
npm install
npm run dev     # dev server on :4321
npm run build   # production build → dist/
npm run preview # preview the production build
```

## Updating coverage numbers

Coverage numbers live in:

- `src/pages/status/index.astro` — dashboard `<CoverageBar>` props (`done`, `total`)
- `src/pages/status/language.mdx` — per-feature tables
- `src/pages/status/stdlib.mdx` — stdlib tables
- `src/pages/status/foundation.mdx` — Foundation tables

Each `<CoverageBar>` component takes `done` and `total` counts and renders
the percentage bar automatically.

## Updating the WASM runtime

When the tswift runtime is updated, rebuild from `crates/tswift-wasm` (which
exports `runSwift` plus the SwiftUI host entry points `swiftUICompile` /
`swiftUIDispatch` that drive the live `<swiftui-canvas>` preview):

```sh
cd ../crates/tswift-wasm   # build the wasm
wasm-pack build --target web --out-dir ../../website/public/wasm --out-name tswift_wasm
```

The SwiftUI preview pane imports the shared render host straight from the
sibling package source (`web/swiftui-canvas/src/canvas.ts`); `astro.config.mjs`
widens Vite's `fs.allow` to the repo root so that import resolves.

## Adding Mermaid diagrams

In any MDX page:

```mdx
import Mermaid from '../../components/Mermaid.astro';

<Mermaid caption="Optional caption">
{`
flowchart LR
  A --> B --> C
`}
</Mermaid>
```

Mermaid is loaded from CDN only when a `.mermaid` element exists on the page.
