# Plan — Layer D: Web Screenshot Harness (Playwright)

**Status:** implemented
**Date:** 2026-06-28
**Parent:** `docs/plan/swiftui-support.md` §5.4 (Layer D)
**Sibling:** `docs/plan/layer-d-ios-renderer.md` (native half)

---

## 1. Goal

The web half of the Layer D web↔native perceptual diff. For each UIIR fixture,
drive the real `<swiftui-canvas>` element through the **same loop** as the iOS
harness and screenshot every step:

```
for each fixture:
  load fixture.uiir.json
  canvas.mount(tree)
  screenshot                          ← initial render
  for each patch_set in fixture.patches.json:
    canvas.applyPatches(patch_set)
    screenshot
```

The screenshots are committed baselines; reruns assert against them. The
side-by-side comparison with `ios/UiirRenderer` baselines is the actual Layer D
artifact (perceptual-diff tooling is a follow-up).

Non-gating — pixel parity between WebKit and SwiftUI is unattainable; this is
confidence + drift surfacing only.

---

## 2. Decisions

1. **Playwright + WebKit.** WebKit is the closest browser engine to iOS
   Safari/SwiftUI (same `-apple-system` font stack), so the web baseline is the
   most directly comparable to the native one. Single `webkit` project.

2. **Imperative harness page, not the example app.** `tests/harness/` is a
   minimal Vite entry: a 320-wide dark `#stage` (matching the iOS
   `RenderHostView`), a single `<swiftui-canvas>`, and a `main.ts` that exposes
   `window.harness.{mount, applyPatches}`. No Swift parsing, no editor — fixtures
   arrive pre-built as JSON. Keeps the screenshot surface deterministic.

3. **Fixtures read from the repo, not copied.** The spec reads
   `tests/swiftui-fixtures/*.uiir.json` / `*.patches.json` via Node `fs` and
   feeds them into the page with `page.evaluate`. Single-sourced with the iOS
   harness.

4. **Deterministic surface + device/appearance matrix.** Four WebKit projects
   pair a viewport with a color scheme: `iphone` (390×844 @3x) and `ipad`
   (834×1194 @2x, portrait) × `light`/`dark`. Sizes/scale match the iOS
   `ViewImageConfig` presets. `workers: 1`, `fullyParallel: false`. Screenshots
   clip to the `#canvas` element, which fills the device width and supplies its
   own adaptive `systemBackground` (semantic `.primary`/`.secondary` adapt via
   CSS variables under `prefers-color-scheme`). Baselines are suffixed by
   project, e.g. `counter-0-initial-iphone-dark-darwin.png`.

5. **Baselines via Git LFS.** `.gitattributes` routes
   `web/swiftui-canvas/tests/**/*-snapshots/**/*.png` through LFS, same policy as
   the iOS PNGs.

---

## 3. Layout

```
web/swiftui-canvas/
  playwright.config.ts                # webkit project + vite webServer
  tests/
    harness/
      index.html                      # 320-wide dark stage + <swiftui-canvas>
      main.ts                         # exposes window.harness.{mount,applyPatches}
    snapshot.spec.ts                  # the mount→patch→screenshot loop
    snapshot.spec.ts-snapshots/       # committed PNG baselines (LFS)
```

---

## 4. Running locally

```sh
cd web/swiftui-canvas
npm install                           # @playwright/test is a devDependency
npm run test:snapshot                 # assert against committed baselines
npm run test:snapshot:update          # re-record baselines
```

The Vite dev server (`tests/harness`, port 4323) is started automatically by
Playwright's `webServer`.

---

## 5. Mirror points (keep web == native loop)

| iOS (`ios/UiirRenderer`) | Web (`web/swiftui-canvas`) |
|---|---|
| `FixtureLoader` walks to `tests/swiftui-fixtures/` | spec reads same dir via `fs` |
| `RenderModel.apply(patchStep)` | `canvas.applyPatches(patchStep)` |
| `RenderHostView` 320-wide dark host | `#stage` 320-wide dark host |
| snapshot per initial + per patch step | `toHaveScreenshot` per step |
| baseline names `name-0-initial`, `name-N` | same naming |

---

## 6. Out of scope (follow-ups)

- **Perceptual-diff tooling** (`odiff`/`pixelmatch` side-by-side web vs native).
- **CI job** (macOS runner for iOS + Linux runner for web, artifact upload).
- Token/CSS drift fixes surfaced by the diff (e.g. Button label visibility) —
  tracked separately; the harness only *surfaces* drift, it doesn't fix it.

---

## 7. Definition of done (this plan)

1. `npx playwright test` mounts every fixture, replays its patches, screenshots
   each step. ✅
2. 22 baselines committed under `tests/snapshot.spec.ts-snapshots/` (LFS). ✅
3. Re-run asserts green against committed baselines. ✅
4. Loop + surface mirror the iOS harness (§5). ✅
