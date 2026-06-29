# Plan — SwiftUI Usability Roadmap (website playground · iOS app · breadth)

**Status:** proposed
**Date:** 2026-06-28
**Baselined against:** `origin/main` (local HEAD is behind — see §0).
**Supersedes (roadmap only):** the §7 tier roadmap in
[`docs/plan/swiftui-support.md`](./swiftui-support.md), which stays the
authoritative **architecture** spec. The native-host work this roadmap builds on
is specified in [`docs/plan/native-host.md`](./native-host.md) (done) and
[`docs/adr/0008`](../adr/0008-xcframework-distribution-local-or-pinned-release.md).
**Related substrate (all on `origin/main`):**
- `crates/tswift-wasm/src/swiftui.rs` — `swiftUICompile` / `swiftUIDispatch`
- `crates/tswift-ffi/` — C ABI (`tswift_swiftui_compile` / `_dispatch`)
- `web/swiftui-canvas/` — the DOM applier; `prototype/swiftui-sandbox/` — web PoC
- `ios/TSwift/` (`TSwiftCore` + `TSwiftUI/PreviewSession`), `ios/UiirRenderer/`
  (`ViewFactory` / `ModifierApply` / `EventSink`), `examples/ios/` (xcodegen app)

---

## 0. Repo state note (read first)

This plan is baselined on `origin/main`, which is **ahead of the current local
HEAD**. Before executing, fast-forward/merge `origin/main` so the FFI crate, the
`ios/TSwift` package, the example app, and the expanded scripts are present
locally. Everything in §1 "Done" refers to `origin/main`.

---

## 1. Where we are

**Runtime + verification (done, Tiers v1–5).** `crates/tswift-swiftui` (~3.2k
LOC): View protocol, `@State`, `@ViewBuilder` shim, render→diff→patch with keyed
reconciliation. **20 views**, **9 modifiers** + `environmentObject`. Layers A/B/C
goldens green (`crates/tswift-cli/tests/swiftui_goldens.rs`). Observation
prelude-only; `@Environment(\.keyPath)` deferred.

**Web substrate (done, but only in the prototype).** `swiftUICompile`/
`swiftUIDispatch` wasm entry points + the shared `web/swiftui-canvas` element +
`prototype/swiftui-sandbox` (a working live editor/preview with a `wasm-smoke.mjs`
gate). **Not yet in the production `website/`.**

**Native substrate (done).** `docs/plan/native-host.md` T1–T11 all landed:
`crates/tswift-ffi` C ABI, `scripts/build-xcframework.sh` /
`publish-xcframework.sh`, `ios/TSwift` (`TSwiftCore`, `TSwiftUI/PreviewSession`),
`ios/UiirRenderer` `EventSink` seam, the pinned `ffi-v1` release (ADR-0008), and
`examples/ios/` — an **xcodegen** app whose `PreviewView` is already a live,
interactive SwiftUI playground (editor → `compile` → `RenderHostView` →
event sink → `dispatch` → patch apply).

**The three appliers (the "locksteps").** Every view/modifier must be taught to:
1. **Runtime** — `crates/tswift-swiftui/src/lib.rs` (view init / modifier fn) +
   `uiir.rs` (serialization) + a golden.
2. **Web** — `web/swiftui-canvas/src/apply-patch.ts` (DOM primitive for new view
   kinds) + `modifier-css.ts` (styling/tokens).
3. **iOS** — `ios/UiirRenderer/Sources/UiirRenderer/ViewFactory.swift` (new kind)
   + `ModifierApply.swift` (new modifier) + `Tokens.swift` (color/font tokens).

### 1.1 The gaps that map to the three asks

- **#1 Website playground** — the live preview exists only in
  `prototype/swiftui-sandbox`; the shipped site still does text-only `runSwift`.
- **#2 iOS app** — a working xcodegen app exists, but it's an `examples/` demo
  using `TextEditor`, not a product app with a real code editor (**Runestone**).
- **#3 Breadth** — still 20 views / 9 modifiers; most pasted SwiftUI won't render.

---

## 2. Plan A — Integrate the real playground into the website

**Goal.** The shipped `website/` playground renders and interacts with live
SwiftUI, driven by the *real* wasm session (no regex fakes), reusing
`web/swiftui-canvas` and the prototype's glue. Then retire the prototype.

**Why it's small.** The runtime, the wasm boundary, the canvas, and the glue all
exist and are tested in the prototype; this is integration + productization.

**Tasks (ordered).**

1. **Ship a SwiftUI-capable wasm build to the site.** The published
   `website/public/wasm/tswift_wasm.js` currently exports only `runSwift`
   (stale). Rebuild from `crates/tswift-wasm` (the crate already has
   `swiftui.rs`) so `swiftUICompile`/`swiftUIDispatch` are exported; update the
   `website/README.md` "Updating the WASM runtime" step to point at
   `crates/tswift-wasm` (not the removed `prototype/web-sandbox`).
2. **Add `@tswift/swiftui-canvas` to the site build.** Import the canvas element
   (source import like the prototype, or the package's `dist`) into the
   playground bundle so `<swiftui-canvas>` is defined.
3. **Detect SwiftUI mode.** In `website/src/components/FullPlayground.astro`,
   after a successful compile, branch: if the program declares a root `View`
   (use `swiftUICompile`'s `{ok, root, tree}` envelope — `root != null`),
   enter preview mode; else keep the text path.
4. **Wire the preview pane.** Add a third output tab (alongside stdout/compile):
   a `<swiftui-canvas>` host. On compile → `canvas.mount(res.tree)`; on the
   element's `swiftui-event` → `swiftUIDispatch(id, event, value)` →
   `canvas.applyPatches(res.patches)`. Lift this verbatim from
   `prototype/swiftui-sandbox/src/pages/index.astro`.
5. **Layout decision (Open Q §6).** Default: **show both** — keep the console for
   `print`, add a "Preview" tab that becomes the default when a `View` is found.
6. **Seed the gallery.** Surface the existing fixtures (`tests/swiftui-fixtures/`,
   already mirrored as `prototype/.../presets/`) as playground presets so the
   feature is discoverable; add a "SwiftUI" preset group.
7. **Gate it.** Move `prototype/.../test/wasm-smoke.mjs` into the website (or CI)
   so the *real compiled wasm* session is asserted on the production surface; add
   one Playwright pass driving `<swiftui-canvas>` end to end.
8. **Retire the prototype.** Per `prototype/swiftui-sandbox/NOTES.md`, delete it
   once parity lands; fold any device-chrome CSS worth keeping into the site.

**Exit / done.** A visitor edits a `View` on the production site, sees a live
device-framed preview, and taps/toggles/types to drive `@State`; the wasm smoke
test + a Playwright smoke run gate it; the prototype is gone.

---

## 3. Plan B — iOS app: edit (Runestone) + live SwiftUI preview

**Goal.** A real iOS **app** (its own xcodegen project) where the user edits
Swift in a **Runestone** code editor and sees a live, interactive SwiftUI preview
— Playground-style. The runtime bridge (`PreviewSession` over `tswift-ffi`) and
the renderer (`UiirRenderer`) are reused unchanged.

**Why it's mostly UI.** `native-host.md` already shipped the hard parts: the FFI
session, the patch applier, the event sink, and a proof-of-life xcodegen app
(`examples/ios`). This plan elevates that into a product and swaps the editor.

**Decisions to take.**
- **Promote `examples/ios` → a product app, or add `apps/ios-playground/`?**
  Recommend a **new app target** (e.g. `apps/TSwiftPlayground/` with its own
  `project.yml`) so `examples/ios` stays a minimal link-smoke demo and the
  product app can grow (multi-file, sharing) without bloating the example.
- **Runestone dependency.** Add `github.com/simonbs/Runestone` (SwiftPM). The
  offline rule is **crates.io-only** (ADR-0008 note); SwiftPM already fetches
  `swift-snapshot-testing`/the pinned xcframework, so a SwiftPM UI dep is
  consistent. Pin a version in `project.yml`'s `packages:`.
- **Syntax highlighting.** Runestone needs a Tree-sitter language. There is no
  Swift `tree-sitter` bundled; options: (a) `tree-sitter-swift` via
  `TreeSitterSwiftRunestone` if available, else (b) ship a minimal regex/none
  highlighter first and add Tree-sitter later. Treat highlighting as a follow-up,
  not a blocker.

**Tasks (ordered).**

1. **Scaffold the app project.** `apps/TSwiftPlayground/project.yml` (xcodegen):
   `type: application`, iOS 16, depends on `TSwift` (`TSwiftCore`+`TSwiftUI`),
   `UiirRenderer`, and `Runestone`. `xcodegen generate` → `.xcodeproj`
   (git-ignored; generated from `project.yml`).
2. **Editor component.** Wrap Runestone's `TextView` in a
   `UIViewRepresentable` (`CodeEditor(text:)`): monospaced theme, line numbers,
   no autocorrect/autocapitalize, a debounce on text change.
3. **Compose the playground screen.** Reuse `examples/ios/Sources/PreviewView`'s
   logic but replace `TextEditor` with `CodeEditor`: a split (editor top / preview
   bottom, or side-by-side on iPad). Drive `@StateObject PreviewSession`:
   debounced `session.compile(source)` on edit; `RenderHostView(model:)` +
   `.uiirEventSink(session.makeEventSink())`; show `session.lastError` inline.
4. **Live recompile UX.** Debounce (~250 ms) so typing recompiles smoothly;
   preserve preview interaction state between recompiles (the `RenderModel`
   patch-in-place already preserves focus/scroll within an event; a *recompile*
   currently rebuilds `model` — acceptable v1, note it).
5. **Sample gallery / presets.** Bundle the `tests/swiftui-fixtures` `.swift`
   files as starter snippets (a picker), matching the website gallery.
6. **App chrome.** Navigation title, a Run/Preview toggle if keeping the Run
   screen, error banner, and a "share/export" stub (deferred). Launch screen +
   bundle id via `project.yml` settings (mirror `examples/ios`).
7. **CI/build gate.** `xcodegen generate` + `xcodebuild build` (sim) in a
   macOS-only, non-blocking job (mirrors the Layer-D job). The regression signal
   stays in the `TSwiftUI`/`UiirRenderer` tests, not the app (per native-host.md
   "the app is a demo, never the gate").

**Exit / done.** An installable iOS app: edit Swift in Runestone, get a live
interactive SwiftUI preview from the tswift runtime, pick from a sample gallery.

**Sequencing inside B.** 1→2→3 is the critical path to "pixels with Runestone";
4–7 are polish. If `tree-sitter-swift` proves fiddly, ship steps 1–3 with plain
text and add highlighting after.

---

## 4. Plan C — Widen the three appliers (breadth) + animation, in order

> **Execution detail:** [`docs/plan/swiftui-breadth-and-animation.md`](./swiftui-breadth-and-animation.md)
> has the exact per-applier extension points, per-item tables, and the
> **animation** workstream (promoted from the backlog; needs ADR-0009 for the
> patch-metadata extension). This section is the summary.

**Goal.** Grow the view/modifier surface until the fixture gallery + common
Apple-tutorial screens render unedited — landing each item in **all three
appliers** (§1.1) with a UIIR golden (and a patch golden if interactive) — and
add **animation** as optional, backward-compatible patch metadata.

**The lockstep recipe (per item).**
1. Runtime: register the view init / modifier fn (`lib.rs`), emit it (`uiir.rs`).
2. Web: DOM primitive (`apply-patch.ts`) and/or CSS mapping (`modifier-css.ts`).
3. iOS: `ViewFactory` case (new kind) and/or `ModifierApply` case; `Tokens.swift`
   for any new color/font token.
4. Add a fixture (`tests/swiftui-fixtures/<x>.swift` + `.uiir.json`, + events/
   patches if interactive); regenerate `registered_keys.txt`; bump coverage.
5. Verify Layer D renders on both hosts (drift shows here).

**Ordering principle.** Cheapest-and-most-universal first: modifiers that touch
*every* view before new node kinds; the highest-frequency containers before
niche ones; styling/compositing that needs host work last.

### Batch C1 — Text & universal styling modifiers (no new node kinds) ⭐
`bold` · `italic` · `underline` · `strikethrough` · `opacity` ·
`foregroundStyle` (accepts the `foregroundColor` token set) · `tint` ·
`lineLimit` · `multilineTextAlignment` · `textCase`.
*Why first:* pure modifier work, applies to existing views, immediately lifts the
fidelity of every current fixture; lowest risk, no host primitives.

### Batch C2 — Layout modifiers & container arguments ⭐
Extend `frame` → `minWidth/maxWidth/minHeight/maxHeight/alignment` (incl.
`.infinity`) · `padding(.horizontal/.vertical/.edges, _)` · stack `spacing:` and
`alignment:` args (VStack/HStack/ZStack) · `Spacer(minLength:)` · `offset`.
*Why second:* layout is the top reason snippets look wrong; unblocks realistic
screens. Web = flex tweaks; iOS = native frame/padding.

### Batch C3 — Structural containers: Group · Divider · ScrollView ⭐
`Group` (transparent passthrough) · `Divider` · `ScrollView` (vertical first,
then `.horizontal` / axes).
*Why third:* `ScrollView` is ubiquitous; `Group`/`Divider` are trivial and
common. New node kinds, so all three appliers grow a case.

### Batch C4 — Visual decoration modifiers
`background(_ view)` (beyond color) · `overlay(_ view, alignment:)` ·
`clipShape` · `clipped` · `border` · `shadow`.
*Why fourth:* needs compositing (a node hosting another node) in both hosts —
more involved; do after layout is solid.

### Batch C5 — Content views: Label · Image · ProgressView
`Label(_, systemImage:)` · `Image(systemName:)` (web: SF-Symbol→icon-set/SVG
table, an accepted drift surface like colors; iOS: native) · `Image(_ name)`
placeholder · `ProgressView()` indeterminate + `ProgressView(value:)`.
*Why fifth:* `Image` needs a symbol-mapping table on the web host (drift), so it
trails the cheaper wins.

### Batch C6 — Lazy stacks, grids, Form
`LazyVStack`/`LazyHStack` (render like stacks) · `Grid`/`GridRow` ·
`LazyVGrid`/`LazyHGrid` (+ `GridItem`) · `Form` (a styled `List`).
*Why sixth:* grids need CSS-grid mapping + iOS `Grid`; most layout-heavy.

### Batch C7 — Control styling & accessibility no-ops
`buttonStyle` (`.bordered`/`.borderedProminent`/`.plain`) · `listStyle` ·
`pickerStyle` · `textFieldStyle` · `disabled` · accept-and-drop
`accessibilityLabel`/`accessibilityHint`/… (so snippets using them still render).
*Why last:* polish; `disabled` and accessibility no-ops keep real snippets from
failing even before styling is perfect.

### Animation (promoted from backlog) — see the detail doc
Animation is the host-plays-it model: the runtime never interpolates per-frame;
it records intent (`.animation` modifier, `withAnimation` batch tag,
`.transition`) and hosts play it (web CSS transitions, iOS real SwiftUI). It adds
an **optional `anim` field** to patches (backward compatible — existing goldens
unchanged) and gets **ADR-0009**. Phases: AN0 tokens/ADR → AN1 implicit
`.animation` → AN2 explicit `withAnimation` → AN3 `.transition` → AN4 richness.
AN0–AN1 can start right after C1; AN2 after C3; AN3 after C6.

**Exit / done for C.** Gate on outcome, not coverage %: the fixture gallery plus
a chosen set of Apple-tutorial views render unedited on **both** hosts, with
basic animations playing. Coverage % rises as a side effect; also wire
`coverage.py`'s unused `verified` state to golden-backed members so the headline
reflects reality.

---

## 5. Sequencing across A / B / C

- **A first** (days): smallest lift, biggest visible win; unblocks public demos.
- **C runs continuously** and feeds both hosts; start **C1–C2 immediately** —
  they make A's website preview and B's iOS app render real content.
- **B in parallel after C1–C3** so the app launches against a runtime that can
  render more than a counter. B's bridge is done, so B is gated mainly on Runestone
  integration + app polish, independent of C.
- **Deferred tiers** (own ADRs, post-A/B/C): navigation & presentation
  (`NavigationStack`/`TabView`/`.sheet` — needs the portal/detached-root patch
  op), async (`.task`/`AsyncImage` — needs the `on_patch`/`pump` seam, ADR-0005),
  `GeometryReader`, and Layer-D perceptual diff CI.

---

## 6. Open questions

- **A:** preview *replaces* the console or sits beside it? (Recommend: both, with
  Preview auto-selected when a root `View` is detected.)
- **B:** which Swift Tree-sitter grammar for Runestone highlighting, and is the
  product app a promotion of `examples/ios` or a new `apps/` target? (Recommend:
  new target.) Recompile-on-edit vs explicit Run button for the live loop.
- **C:** stop condition for breadth (which tutorial set is the bar); the SF-Symbol
  web mapping source (icon font vs bundled SVG set).
- **Cross:** when navigation lands, the exact portal patch-op encoding (carried
  from `swiftui-support.md` §11).

---

## 7. Deliverables checklist

**Plan A — website**
- [x] Rebuild site wasm with `swiftUICompile`/`swiftUIDispatch`; fix README step.
- [x] `<swiftui-canvas>` in the site bundle; SwiftUI-mode detection in playground.
- [x] Preview pane: mount on compile, route `swiftui-event` → dispatch → patches.
- [x] SwiftUI preset gallery (Counter/Toggle/List/Profile).
- [x] wasm smoke gate on production (CI `website-wasm-smoke`); delete
      `prototype/swiftui-sandbox`. (Playwright deferred — no browser in CI yet.)

**Plan B — iOS app**
- [ ] `apps/TSwiftPlayground/project.yml` (xcodegen) + Runestone dependency.
- [ ] `CodeEditor` Runestone `UIViewRepresentable`.
- [ ] Playground screen: debounced `PreviewSession.compile` + `RenderHostView` +
      event sink + inline errors.
- [ ] Sample gallery; app chrome; non-gating `xcodebuild` CI.

**Plan C — breadth (each batch: 3 appliers + goldens + registered_keys)**
- [ ] C1 text/styling modifiers · [ ] C2 layout modifiers/args · [ ] C3 Group/
      Divider/ScrollView · [ ] C4 decoration · [ ] C5 Label/Image/ProgressView ·
      [ ] C6 lazy/grids/Form · [ ] C7 control styling + a11y no-ops.
- [ ] Wire `coverage.py` `verified` to golden-backed members.

**Animation (ADR-0009; optional `anim` patch field)**
- [ ] AN0 ADR + `Animation`/`Angle` tokens + `uiir` encoding (no behavior).
- [ ] AN1 implicit `.animation` · [ ] AN2 explicit `withAnimation` (session slot
      + diff stamping) · [ ] AN3 `.transition` (insert/remove) · [ ] AN4 richness
      (`repeatForever`/`delay`/spring, `rotationEffect`/`scaleEffect`).

**Cross-cutting**
- [ ] Merge `origin/main` into the working branch (§0).
- [ ] `@Environment(\.keyPath)` (property-wrapper arguments).
- [ ] `update-website` syncs coverage + gallery after each batch.
</content>
