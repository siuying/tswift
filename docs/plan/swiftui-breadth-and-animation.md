# Plan — SwiftUI breadth (the three appliers) + animation

**Status:** proposed
**Date:** 2026-06-28
**Parent:** [`docs/plan/swiftui-usability-roadmap.md`](./swiftui-usability-roadmap.md)
(Plan C). This is the execution-level detail for widening the view/modifier
surface, plus a new **animation** workstream the user requested (animation was
backlogged in `swiftui-support.md`; this promotes it with a protocol extension).
**Architecture spec:** [`docs/plan/swiftui-support.md`](./swiftui-support.md)
(UIIR + patch protocol §3, token encoding §3.1).

---

## 1. The three appliers — exact extension points

Every item below is landed in **all three** appliers + a golden. These are the
concrete seams (verified against the code):

### 1a. Runtime — `crates/tswift-swiftui/`
- **New modifier (no/positional args):** add `modifier!(modifier_x, "x");` then a
  `("x", modifier_x)` row in `MODIFIER_FNS` (`src/lib.rs`). It appends a
  `_Modifier{name, value?}` record; `uiir::write_modifier` already serializes it.
- **New view:** `interp.register_free_fn("X", x_init)` in `install`; write
  `x_init` building a value via `view_value("X", fields)` (leaf) or
  `container_value("X", collect_children(ctx, args)?)` (container); add `"X"` to
  the `registered_keys` match so the coverage key `X.init` is emitted.
- **New token namespace** (e.g. `TextAlignment`, `Edge`, `ContentMode`,
  `Animation`): add a `struct` with `static let` token values to `PRELUDE`; add
  its type name to the allowlist in `token_of`; map it to a wire tag in
  `uiir::write_value`'s `namespace → tag` match (today: `Color→color`,
  `Font→textStyle`, `FontWeight→weight`; default `token`).
- **Golden:** `tests/swiftui-fixtures/<x>.swift` + `.uiir.json` (+ `.events.json`/
  `.patches.json` if interactive); regenerate `frameworks/swiftui/registered_keys.txt`.

### 1b. Web — `web/swiftui-canvas/src/`
- **New view kind:** add a `case "X"` in `element()` (`apply-patch.ts`) returning
  a DOM node; handle its args in `applyArgs`.
- **New modifier:** add a `case "x"` in `applyModifiers` (`modifier-css.ts`)
  writing inline style; extend the token tables (`TEXT_STYLE_SIZE`, `FONT_WEIGHT`,
  `COLOR`) or add a new one for a new token namespace.

### 1c. iOS — `ios/UiirRenderer/Sources/UiirRenderer/`
- **New view kind:** add a `case "X"` in `ViewFactory.build()`.
- **New modifier:** add a `case "x"` in `ModifierApply.applyOne()`.
- **New token:** extend the `Tokens.swift` tables (`font`/`weight`/`color`) and/or
  add `UiirValue` resolver helpers (`asColor`/`asLength` pattern).

> **Lockstep is the dominant cost.** A missing applier shows up as Layer-D drift.
> Land all three in one change per item.

---

## 2. Breadth batches (ordered)

Cheapest-and-most-universal first: modifiers that touch *every* view before new
node kinds; high-frequency containers before niche; compositing last.

### Batch C1 — Text & universal styling modifiers ⭐ (no new node kinds)
| modifier | runtime | web | iOS |
|---|---|---|---|
| `bold` `italic` `underline` `strikethrough` | `modifier!` (null value) | font-weight/style/text-decoration | `.bold()`/`.italic()`/… |
| `opacity(_)` | `modifier!` (numeric) | `opacity` | `.opacity` |
| `foregroundStyle(_)` | reuse Color token | `color` | `.foregroundStyle` (Color) |
| `tint(_)` | reuse Color token | `accent-color`/`color` | `.tint` |
| `lineLimit(_)` | numeric | `-webkit-line-clamp` | `.lineLimit` |
| `multilineTextAlignment(_)` | **new `TextAlignment` token** | `text-align` | `.multilineTextAlignment` |
| `textCase(_)` | **new token** | `text-transform` | `.textCase` |

*Pure modifier work; immediately lifts fidelity of every existing fixture.*

### Batch C2 — Layout modifiers & container args ⭐
- Extend `frame` → `minWidth/maxWidth/minHeight/maxHeight/alignment` (incl.
  `.infinity` → web `100%`/flex-grow, iOS `.infinity`). The runtime already has
  `frame`; widen the args object and the host readers.
- `padding(.horizontal/.vertical/.edges, _)` — **new `Edge`/`Edge.Set` token** +
  optional length; web → directional padding, iOS → `.padding(edges, len)`.
  ⚠️ `Edge.horizontal`/`.vertical` collide with C3's `Axis.horizontal`/`.vertical`
  and `Edge.leading`/`.trailing` with C1's `TextAlignment`; bare leading-dot
  forms are ambiguous under untyped builtin params, so these need typed prelude
  shims (or qualified `Edge.horizontal`) — same blocker as `alignment:`/`.infinity`
  (deferred, issue #189).
- Stack `spacing:` and `alignment:` args on `VStack`/`HStack`/`ZStack`
  (`x_init` reads the labeled args → fields; web flex `gap`/`align-items`, iOS
  native stack params).
- `Spacer(minLength:)`, `offset(x:y:)`.

*Top reason snippets look wrong; unblocks realistic screens.*

### Batch C3 — Structural containers ⭐
`Group` (transparent passthrough; iOS already falls through to children — make it
explicit) · `Divider` · `ScrollView` (`.vertical` first, then axes →
web `overflow:auto` + flex direction, iOS `ScrollView`).

### Batch C4 — Visual decoration (compositing) modifiers
`background(_ view)` (beyond color — a node hosting a node; `write_value` already
serializes a nested view) · `overlay(_ view, alignment:)` · `clipShape` ·
`clipped` · `border` · `shadow`. Web = positioned wrapper / box-shadow; iOS =
native. *Needs node-in-modifier compositing; after layout is solid.*

### Batch C5 — Content views
`Label(_, systemImage:)` · `Image(systemName:)` (web: **SF-Symbol → icon-set/SVG
table**, an accepted drift surface like colors; iOS: native `Image(systemName:)`)
· `Image(_ name)` placeholder · `ProgressView()` + `ProgressView(value:)`.

### Batch C6 — Lazy stacks, grids, Form
`LazyVStack`/`LazyHStack` (render like stacks) · `Grid`/`GridRow` ·
`LazyVGrid`/`LazyHGrid` (+ `GridItem`) · `Form` (styled `List`). *Most
layout-heavy; web CSS-grid + iOS `Grid`.*

### Batch C7 — Control styling & accessibility no-ops
`buttonStyle` (`.bordered`/`.borderedProminent`/`.plain`) · `listStyle` ·
`pickerStyle` · `textFieldStyle` · `disabled` · accept-and-drop
`accessibilityLabel`/`accessibilityHint`/… so snippets using them still render.

**Exit (C):** the fixture gallery + a chosen Apple-tutorial set render unedited on
both hosts; wire `coverage.py`'s `verified` state to golden-backed members.

---

## 3. Animation workstream

Animation was out of scope in `swiftui-support.md` (deferred to a backlog ADR).
The user wants it. It is a **protocol inflection** — the discrete request→response
patch model (§3.3) gains *animation metadata* — so it gets its **own ADR**
(propose `docs/adr/0009-swiftui-animation-as-patch-metadata.md`).

### 3.1 Core principle: the runtime never interpolates per-frame
The runtime stays declarative: it records *animation intent* as modifier values
and tags the patches a state change produces. The **host** plays the animation —
web via **CSS transitions/keyframes**, iOS via **real SwiftUI `.animation`/
`withAnimation`** (the native host *is* SwiftUI, so this is near-identity). This
keeps Rust out of the render loop (matches the plan's backlog stance).

### 3.2 The three animation surfaces & their wire encoding

1. **Implicit — `.animation(_ curve, value:)` / `.animation(_)`.** A view modifier
   `{name:"animation", value:{curve:"easeInOut", duration:0.3, …}}`.
   - *Web:* set a persistent CSS `transition` on the element so later
     `setModifiers`/`setArgs` style changes tween.
   - *iOS:* apply `.animation(Tokens.animation(...), value:)` in `ModifierApply`.

2. **Explicit — `withAnimation(_ curve) { stateMutation }`.** A prelude free
   function. During `dispatch`, it records the active animation in a
   session-visible slot; the session **tags the resulting patch batch** with
   `anim: {curve, duration, …}` and clears the slot. The diff engine threads an
   optional `anim` onto each emitted patch (or a batch-level envelope).
   - *Web:* apply the batch inside a transition (set transition, then apply
     patches, then clear), so mounted/changed nodes tween.
   - *iOS:* wrap `RenderModel.apply(_:)` in `withAnimation(Tokens.animation(...))`
     when the batch carries `anim`.

3. **Transitions — `.transition(_)` on insert/remove.** A modifier
   `{name:"transition", value:{kind:"slide"|"opacity"|"scale"|"move", edge?:…}}`.
   The host honors it on `insert`/`remove` patches for that node.
   - *Web:* enter/leave CSS keyframes/classes; delay actual DOM removal until the
     leave animation ends (needs a small async removal in `apply-patch.ts`).
   - *iOS:* `.transition(...)` on the node (SwiftUI plays it when the node enters/
     leaves an animated container update).

### 3.3 New token namespace: `Animation`
Add to `PRELUDE`:
```swift
struct Animation {
    let token: String; let duration: Double?; let delay: Double?
    let repeatCount: Int?; let autoreverses: Bool?
    static let `default`  = Animation(token:"default", ...)
    static let linear     = Animation(token:"linear", ...)
    static let easeIn     = Animation(token:"easeIn", ...)
    static let easeOut    = Animation(token:"easeOut", ...)
    static let easeInOut  = Animation(token:"easeInOut", ...)
    static let spring     = Animation(token:"spring", ...)
    static func easeInOut(duration: Double) -> Animation { ... }
    static func linear(duration: Double) -> Animation { ... }
    static func spring(response: Double, dampingFraction: Double) -> Animation { ... }
    func delay(_:) -> Animation; func repeatCount(_:autoreverses:) -> Animation
    func repeatForever(autoreverses:) -> Animation; func speed(_:) -> Animation
}
func withAnimation<R>(_ a: Animation = .default, _ body: () -> R) -> R { ... }
```
- `withAnimation` is a registered free fn (not pure prelude) so it can write the
  active-animation slot the session reads. (Or a prelude shim that sets a
  well-known global the session inspects — decide in the ADR.)
- Serialize `Animation` as `{curve, duration?, delay?, repeatCount?, autoreverses?}`
  via a dedicated branch in `uiir::write_value` (not the bare `{$,name}` form,
  since it carries parameters).
- **Token → host curve tables** (mirror the existing Tokens pattern):
  - *Web:* curve → `transition-timing-function` (`easeInOut`→`ease-in-out`,
    `linear`→`linear`, `spring`→a `cubic-bezier` approximation) + duration (ms).
  - *iOS:* `Tokens.animation(_) -> SwiftUI.Animation` (`.easeInOut(duration:)`,
    `.spring(response:dampingFraction:)`, `.repeatCount`, …).

### 3.4 Animatable-value modifiers (prereqs, fold into C2/C4)
`rotationEffect(_ angle)`, `scaleEffect(_)`, `offset`, `opacity` are just
modifiers; they only *animate* when combined with §3.2. Land them as plain
modifiers in C2/C4; animation makes their changes tween for free once §3.2 ships.
Needs an `Angle` token (`.degrees`/`.radians`).

### 3.5 Patch-protocol extension (the ADR's core)
- Add an **optional** `anim` field to patch ops (default absent → today's
  behavior, fully backward compatible; existing goldens unchanged).
- `insert`/`remove` consult the node's `transition` modifier.
- Diff engine (`crates/tswift-swiftui/src/diff.rs`): accept an optional
  "active animation" for the batch and stamp it onto produced patches.
- Session (`session.rs` / wasm `swiftui.rs` / ffi `swiftui.rs`): expose/read the
  active-animation slot set by `withAnimation`, attach to the dispatch result,
  clear after.

### 3.6 Animation phases (each shippable, golden-gated)
- **AN0 (ADR + tokens):** write ADR-0009; add `Animation`/`Angle` tokens +
  `uiir` encoding; no behavior yet. Existing goldens stay byte-identical.
- **AN1 implicit `.animation`:** modifier in all three appliers; web persistent
  CSS transition, iOS `.animation(_,value:)`. Golden: a counter with
  `.animation` produces the modifier; Layer D shows the tween.
- **AN2 explicit `withAnimation`:** session active-animation slot + diff `anim`
  stamping + both hosts apply the batch animated. Patch golden asserts the `anim`
  field on the tap's patch stream.
- **AN3 `.transition`:** insert/remove transitions; web deferred-removal, iOS
  `.transition`. Patch golden for a `ForEach` add/remove carrying transition.
- **AN4 richness:** `repeatForever`/`delay`/`speed`/spring params; `rotationEffect`/
  `scaleEffect` animated; `matchedGeometryEffect` explicitly deferred.

---

## 4. Sequencing

1. **C1 → C2** first (universal styling + layout) — they make the website
   playground (Plan A) and iOS app (Plan B) render real content.
2. **C3** (Group/Divider/ScrollView) next — ubiquitous.
3. **AN0 → AN1** can start right after C1 (implicit `.animation` is just a
   modifier); **AN2** after C3 (needs the dispatch/diff plumbing settled);
   **AN3** after C6 (transitions are most useful with `ForEach`/insert-remove).
4. **C4 → C5 → C6 → C7** as the compositing/content/grid/styling tail.

Animation's `anim` field is **additive and optional**, so it never blocks or
breaks the breadth batches or the existing goldens.

---

## 5. Definition of done (per item)
Registered (Layer A) · UIIR golden (B) · patch golden if interactive/animated (C)
· wired in **both** host appliers · `registered_keys.txt` regenerated · surfaced
in the gallery. Animation additionally: ADR-0009 updated, `anim`/transition path
covered by a patch golden, Layer D shows the motion on both hosts.

---

## 6. Open questions
- **Token tags:** one generic `{"$":"token","ns":…,"name":…}` for new namespaces,
  or a per-namespace tag (current style)? (Lean per-namespace for host clarity.)
- **`withAnimation` seam:** registered free fn writing a session slot, vs a
  prelude shim setting a global the session reads — resolve in ADR-0009.
- **Spring fidelity:** how closely to approximate SwiftUI springs in CSS
  (`cubic-bezier` vs a generated keyframe); accepted web-vs-iOS drift.
- **SF-Symbol web source** (C5): icon font vs bundled SVG set.
- **Transition removal on web:** the async "play leave, then remove" change to
  `apply-patch.ts` — keep the diff engine unaware (host-only concern)?
