# ADR-0009: SwiftUI animation — host-played, carried as optional patch metadata

- **Status:** Proposed
- **Date:** 2026-06-28
- **Context slice:** SwiftUI rendering (framework layer) — animation
- **Target platform:** iOS — the SwiftUI surface is extracted from the iPhoneSimulator SDK; the web host is the secondary renderer
- **Builds on:** ADR-0006 (render host: runtime-evaluated host-neutral UIIR + Rust diff engine + thin patch-applier hosts)
- **Drives:** `docs/plan/swiftui-breadth-and-animation.md` §3 (the staged AN0–AN4 animation work)

## Context

ADR-0006 deferred animation to "a backlog ADR" and the `swiftui-support.md`
roadmap listed `withAnimation` / transitions / `matchedGeometryEffect` as
explicitly out of scope. Animation is now wanted: a live preview that doesn't
animate feels broken (a counter snaps, a list add pops in), and animation is a
defining part of SwiftUI's feel.

Animation is the inflection ADR-0006 warned about: the render host's contract is a
**discrete request→response patch stream** (§3.3) — `dispatch(event) → Patch[]` —
and patches are *instantaneous state replacements* (`setText`, `setModifiers`,
`insert`, `remove`, …). Animation is inherently *temporal*, so it does not fit the
existing protocol as-is.

The forces:

- **No per-frame work in Rust.** The runtime is a tree-walking interpreter behind
  a serialized boundary (wasm/C-ABI). Driving 60fps interpolation across that
  boundary — re-evaluating `body` per frame, diffing, and shipping a patch stream
  each frame — would be slow, power-hungry, and architecturally wrong. The runtime
  should stay declarative.
- **Two hosts can already animate natively, for free.** The web host owns the DOM
  and CSS — CSS transitions/keyframes are a hardware-accelerated, declarative
  animation engine. The iOS host *is* SwiftUI — `withAnimation`, `.animation`,
  and `.transition` are right there. Re-deriving an interpolator in Rust would
  duplicate what both hosts already do better.
- **Backward compatibility is mandatory.** Tiers v1–5 ship with committed UIIR and
  patch goldens (`tests/swiftui-fixtures/*.{uiir,patches}.json`). Any protocol
  change that perturbs a non-animated fixture's bytes would invalidate the whole
  gate.
- **Three distinct SwiftUI animation surfaces** must be expressible: implicit
  (`.animation(_:value:)`), explicit (`withAnimation { … }`), and transitions
  (`.transition(_)` on insert/remove) — they map to different points in the
  render/diff/patch loop.

## Decision

**The runtime records animation *intent*; the host *plays* it. Intent crosses the
boundary as optional, additive metadata on the existing UIIR and patch stream —
never as per-frame data.** Six decisions:

1. **Host-played, declarative animation; the runtime never interpolates.** The
   runtime emits *what* changed and *how it should be animated* (a curve token +
   parameters), not intermediate frames. The web host plays it with CSS
   transitions/keyframes; the iOS host plays it with real SwiftUI
   `.animation`/`withAnimation`/`.transition`. This keeps Rust out of the render
   loop and makes the native host a near-identity mapping (consistent with
   ADR-0006 decision 2).

2. **`Animation` and `Angle` are semantic tokens** (ADR-0006 decision 2 /
   plan §3.1 encoding). Prelude structs (`Animation`, `Angle`) carry a curve name
   plus parameters; `.easeInOut`, `.spring(response:dampingFraction:)`,
   `.degrees(_)` resolve to lightweight token values the host interprets. Hosts
   own the curve→engine tables (web: `transition-timing-function` + duration ms,
   with a `cubic-bezier`/keyframe approximation for springs; iOS:
   `SwiftUI.Animation`). **Spring/curve fidelity drift between web and iOS is
   accepted** (same stance as color/font tokens in ADR-0006).

3. **Implicit `.animation` is an ordinary modifier.** `.animation(_ curve,
   value:)` / `.animation(_)` appends a `_Modifier{name:"animation",
   value:{curve, duration?, delay?, repeatCount?, autoreverses?}}` record — the
   established modifier path (no new mechanism). The host reads it once and arms a
   *persistent* transition on that node so subsequent `setModifiers`/`setArgs`
   changes tween. (`value:` identity is not threaded across the boundary in v1; an
   armed transition animates any animatable change — an accepted simplification.)

4. **Explicit `withAnimation` tags the patch batch via an `anim` field.**
   `withAnimation(_ curve) { stateMutation }` is a prelude/runtime seam that
   records the active animation in a **session-scoped slot** for the duration of
   the closure. After the event's `body` re-evaluation, the diff engine stamps an
   **optional `anim` field** onto the patches it produces for that batch; the
   session clears the slot. Hosts apply an `anim`-tagged batch *inside* an
   animation transaction (web: arm a transition around the `applyPatches` call;
   iOS: `withAnimation { renderModel.apply(patches) }`).

5. **`.transition(_)` is a modifier honored on `insert`/`remove`.** A
   `_Modifier{name:"transition", value:{kind:"slide"|"opacity"|"scale"|"move",
   edge?}}` record is read by the host when the node enters or leaves. Web plays a
   CSS enter/leave animation and **defers the actual DOM removal** until the leave
   animation finishes (a host-only concern in `apply-patch.ts`; the diff engine
   stays unaware). iOS applies `.transition(...)` so SwiftUI plays it during the
   animated container update.

6. **The patch-protocol change is purely additive and optional.** The `anim`
   field is absent by default. The Rust `Patch` enum (`crates/tswift-swiftui/src/
   diff.rs`) gains an optional per-batch animation that `to_json` emits **only
   when present**, so every existing non-animated golden stays **byte-identical**.
   `transition` is just another entry in the already-serialized modifier list.
   No existing op is renamed or reshaped.

### Wire format (the additive surface)

- **Token value:** `Animation` → `{"$":"animation","curve":"easeInOut","duration":0.3,"delay":0,"repeatCount":1,"autoreverses":false}` (a dedicated `write_value` branch — it carries parameters, unlike the bare `{"$":tag,"name":…}` token form). `Angle` → `{"$":"angle","degrees":45}`.
- **Implicit modifier:** `{"name":"animation","value":<Animation token>}` in a node's ordered `modifiers`.
- **Transition modifier:** `{"name":"transition","value":{"kind":"slide","edge":"trailing"}}`.
- **Explicit batch tag:** patches produced under `withAnimation` carry an optional `"anim":<Animation token>` (e.g. `{"op":"setText","id":"0.0","text":"1","anim":{"$":"animation","curve":"easeInOut","duration":0.3}}`). Encoding emits it only when set.

## Consequences

- **Good:** zero per-frame Rust work; both hosts use their native, accelerated
  animation engines; the native host stays near-identity; the change is additive,
  so all v1–5 goldens remain valid; animation can land incrementally (AN0–AN4)
  without blocking the breadth batches (Plan C), since `anim` is optional.
- **Cost / accepted limits:**
  - **`value:`-scoped implicit animation is coarsened** — an armed `.animation`
    node tweens any animatable change, not only changes to the named `value`.
    Acceptable for preview fidelity; revisit if a fixture needs it.
  - **Spring/curve drift** between CSS and SwiftUI is accepted (per ADR-0006's
    token drift stance); Layer D surfaces it as an artifact, not a gate.
  - **Transition removal needs async host bookkeeping** on web (delay DOM removal
    until the leave animation ends) — isolated in `apply-patch.ts`.
  - **No interruptible/physics-accurate spring continuation** across rapid events
    (the runtime doesn't model in-flight velocity); the host's own engine handles
    re-targeting as best it can.
- **`withAnimation` seam:** it must reach session state set during `dispatch`.
  Resolve the exact mechanism in implementation (AN2): a **registered free
  function** that writes a session-visible active-animation slot is preferred over
  a prelude global, keeping the slot owned by the session that reads it.
- **Still deferred (separate backlog):** `matchedGeometryEffect`,
  `PhaseAnimator`/`KeyframeAnimator`, continuous-gesture-driven animation, and
  `TimelineView` — they need either geometry round-trips (cf. `GeometryReader`)
  or the async push channel (ADR-0005) and are out of scope here.

## Notes

- `unsafe` confinement (ADR-0001) is preserved: animation is safe-Rust token
  values + an optional field on the safe-Rust diff engine; all temporal work is in
  the hosts.
- Staged plan, per-applier extension points, and the AN0–AN4 phasing live in
  `docs/plan/swiftui-breadth-and-animation.md` §3.
- Implementation touch points: `crates/tswift-swiftui/src/{lib.rs (PRELUDE tokens,
  `withAnimation`, `.animation`/`.transition` modifiers, `token_of`), uiir.rs
  (`write_value` animation/angle branches), diff.rs (optional `anim` on the batch
  + `to_json`), session.rs (active-animation slot)}`; `crates/tswift-wasm/src/
  swiftui.rs` + `crates/tswift-ffi/src/swiftui.rs` (thread the slot through
  dispatch); `web/swiftui-canvas/src/{apply-patch.ts (anim transactions, deferred
  transition removal), modifier-css.ts (curve tables)}`; `ios/UiirRenderer/
  Sources/UiirRenderer/{ModifierApply.swift, ViewFactory.swift, Tokens.swift,
  RenderModel.swift (withAnimation wrap)}`.
