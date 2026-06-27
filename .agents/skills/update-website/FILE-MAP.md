# Website File Map

## Coverage numbers to update

| What changed | Table file | CoverageBar in table | CoverageBar in dashboard |
|---|---|---|---|
| Language Tier 0–9 | `src/pages/status/language.mdx` | none (no bars per tier in that file) | `src/pages/status/index.astro` — 10 tier bars + "Language" status card |
| Stdlib | `src/pages/status/stdlib.mdx` | `<CoverageBar label="Standard Library" .../>` + per-section bars | `src/pages/status/index.astro` — "Standard Library" bar + status card |
| Foundation | `src/pages/status/foundation.mdx` | `<CoverageBar label="Foundation" .../>` | `src/pages/status/index.astro` — "Foundation" bar + status card |

## Key files

```
website/
├── public/wasm/
│   ├── tswift_wasm.js          ← WASM JS module (update with new runtime)
│   └── tswift_wasm_bg.wasm     ← WASM binary
│
├── src/pages/
│   ├── index.astro             ← Home: hero, mini-playground, coverage bars
│   ├── playground.astro        ← Full playground
│   │
│   ├── how-it-works/
│   │   ├── index.mdx           ← Architecture overview (Mermaid pipeline)
│   │   ├── frontend.mdx        ← Lexer → parser → sema → compat lowerer
│   │   ├── runtime.mdx         ← Evaluator, SwiftValue, ARC/CoW (Mermaid)
│   │   └── libraries.mdx       ← Stdlib / Foundation / SwiftUI roadmap
│   │
│   └── status/
│       ├── index.astro         ← Dashboard: status cards + tier bars
│       ├── language.mdx        ← Tier 0–9 feature tables  ← EDIT HERE for language
│       ├── stdlib.mdx          ← Stdlib tables             ← EDIT HERE for stdlib
│       ├── foundation.mdx      ← Foundation tables         ← EDIT HERE for Foundation
│       └── swiftui.mdx         ← SwiftUI roadmap
│
├── src/components/
│   ├── CoverageBar.astro       ← Props: label, done, total, href?
│   ├── Mermaid.astro           ← Wraps <pre class="mermaid">
│   ├── MiniPlayground.astro    ← Home demo (preset codes inline)
│   └── FullPlayground.astro    ← Playground page (all 10 presets inline)
│
└── src/layouts/
    ├── Base.astro              ← HTML shell, Nav, Footer, Mermaid CDN loader
    └── Doc.astro               ← Sidebar layout for how-it-works + status pages
```

## Dashboard status card numbers (index.astro)

The status cards show a big percentage (`pct-big`) and a prose description.
Both must be updated manually — the card does **not** read from the CoverageBar components.

```astro
<!-- example: update "81%" and "138 of 171" to match real counts -->
<div class="pct-big high">81%</div>
<p>Tiers 0–7 · 138 of 171 features</p>
```

CSS class for the colour: `high` (≥80%, green) · `mid` (50–79%, yellow) · `low` (<50%, red) · `zero` (0%, grey).

## Preset code (playground)

Preset Swift snippets are inlined directly in `MiniPlayground.astro` and `FullPlayground.astro`.
To add/change a preset, edit the `PRESETS` / `ALL_PRESETS` array in the relevant component.
Use `\\(` for Swift string interpolation inside JS template literals.
