# Plan — SwiftUI Support (render SwiftUI to a host without native compilation)

**Status:** proposed
**Date:** 2026-06-27
**Reference toolchain / SDK:** Swift **6.3.2** (`swift-6.3.2-RELEASE`) + **iPhoneSimulator SDK** — iOS is the target platform, so the SwiftUI surface (`frameworks/swiftui/inventory.md`, and the denominator `registered_keys.txt` is measured against) is extracted from the iOS Simulator SDK, not macOS. The `[swiftui]` descriptor in `tools/framework-inventory/frameworks.toml` resolves it via `xcrun --sdk iphonesimulator --show-sdk-path` with `arm64-apple-ios-simulator.swiftinterface`.
**Related:**
- `docs/plan/framework-support.md` — the framework-parameterized inventory/coverage loop this extends
- `tools/framework-inventory/` — `extract.py` / `coverage.py` / `frameworks.toml` (the surface-coverage tooling we reuse)
- `frameworks/swiftui/scope.toml` — the SwiftUI scope manifest (to be authored, see §6)
- `docs/adr/0005-cooperative-concurrency-executor.md` — the async executor Tier 6 depends on
- `crates/tswift-wasm` — the existing one-shot `runSwift` boundary we promote to a stateful session
- `crates/tswift-core/src/value.rs` — `SwiftValue` (view values reuse `Struct`; no core change)

---

## 1. Problem statement

We want to **render SwiftUI in a browser (and later a native host) without a Swift
toolchain, LLVM, or native compilation** — so a user can edit a SwiftUI `View` on
the web and get a live, interactive preview, and so the same view tree can later
drive a real native renderer.

The existing runtime already parses and *runs* Swift (frontend → typed AST →
tree-walking interpreter, with stdlib/Foundation behaviour in Rust). SwiftUI is
the next framework layer, but unlike Foundation it is **not value semantics** — it
needs a *render/diff host*: instantiate a `View`, run `body`, produce a view tree,
react to events by mutating `@State`, re-evaluate, and patch the host. This plan
specifies that host, the wire protocol, the crate/runtime work, the staged
roadmap, and — most importantly (§5) — an **autonomous verification loop** so a
completed slice can be checked without a human and without an Apple toolchain.

This plan is the strategic spec. The two architectural inflections it introduces
(the async push channel, the `GeometryReader` layout round-trip) each get their
own ADR when their tier lands.

---

## 2. Architecture (resolved)

The design is **runtime-evaluated, host-neutral UIIR with the diff engine in Rust
and dependency-free hosts** — settled decision-by-decision; rationale in §9.

```
 Swift source ──▶ frontend (lexer→parser→sema) ──▶ typed AST
                                                     │
                    tswift-core interpreter ◀────────┘
                    + tswift-swiftui builtins
                                                     │
                 persistent render-node tree (owns @State, keyed by identity)
                                                     │
                       evaluate body  ──▶  UIIR view-value tree
                                                     │
                      Rust diff engine  ──▶  keyed PATCH stream
                                                     │
                ┌────────────────────────┴────────────────────────┐
        web host: <swiftui-canvas>                     native host (later):
        Shadow-DOM custom element                      UIView/SwiftUI applier
        h() + applyPatch + modifier→CSS                same patch protocol
                │  ▲                                            ▲
            events │  │ patches                                 │
                ▼  │                                            │
            wasm SwiftUISession  ──────── onPatch push ─────────┘
```

Core invariants (each a §9 decision):

- **Host-neutral semantic UIIR.** Nodes are SwiftUI *concepts* (`VStack`, `Text`,
  `.font(.largeTitle)`), never pre-lowered to DOM. Lowering concept → host
  primitive happens **only in the host adapter**. This is what makes a native
  host nearly an identity mapping.
- **Stateful in-browser wasm session.** `@State` lives in live Rust memory and
  never leaves it; only the view tree and patches cross the boundary.
- **No React.** The Rust runtime owns the single diff engine; both hosts are thin
  `applyPatch` appliers. (Matches the "no React, no Web Components" product
  identity.)
- **No `tswift-core` changes for view *values*.** Views are `SwiftValue::Struct`
  with a `type_name` and a flat ordered `_modifiers` field (CoW). The one core
  touch is the `@State` storage hook (§4).

---

## 3. The UIIR + patch protocol (wire format)

Both hosts consume this; it is the stable contract.

### 3.1 Node shape

```json
{
  "id": "0.1.2",
  "kind": "Text",
  "args": { "verbatim": "0" },
  "modifiers": [
    { "name": "font",            "value": { "$": "textStyle", "name": "largeTitle" } },
    { "name": "fontWeight",      "value": { "$": "weight",    "name": "bold" } },
    { "name": "foregroundColor", "value": { "$": "color",     "name": "white" } }
  ],
  "children": []
}
```

- **`id`** — stable structural-path identity (§4). Used by both diffing and event routing.
- **`modifiers`** — **ordered** (`.padding().background()` ≠ `.background().padding()`).
- **Value encoding is a tagged union: semantic token OR explicit value.**
  `.font(.largeTitle)` → `{"$":"textStyle","name":"largeTitle"}` (not `34px`);
  `.foregroundColor(.indigo)` → `{"$":"color","name":"indigo"}` (not `#5856D6`);
  but `Color(red:…)` → `{"$":"color","rgba":[…]}` and `.frame(width:56)` → numeric.
  **Hosts resolve tokens** — the web host owns a `largeTitle → CSS` table; the
  native host maps `largeTitle → .largeTitle`. This is the accepted source of
  iOS-vs-web color/font drift.

### 3.2 Patch operations (v1, replace-heavy)

```
mount(node)                    // initial render = insert subtree from empty
insert(parentId, index, node)  // node carries its full subtree
remove(id)
replace(id, node)              // coarse fallback when kind changes
setText(id, text)              // Text fast-path
setModifiers(id, modifiers[])  // replace the WHOLE ordered list (not per-modifier)
setArgs(id, argsDelta)         // constructor-arg changes
```

`move(parentId, from, to)` and keyed reconciliation arrive in **Tier 3** with
`ForEach`. Whole-list `setModifiers` is deliberate: modifier lists are short and
order-sensitive, so whole-list replacement avoids reorder-diff complexity at the
cost of a few redundant style writes.

### 3.3 Event protocol (host → runtime)

```json
{ "id": "0.1.2", "event": "tap",    "value": null }
{ "id": "0.3.0", "event": "change", "value": "hello" }
```

The session resolves `id → the live closure/binding` in the persistent render
tree, invokes it, mutates `@State`, re-evaluates, and returns the resulting patch
list. One round-trip per discrete event.

### 3.4 Session API (the wasm boundary)

Promote `tswift-wasm` from one-shot `runSwift` to a stateful handle:

```rust
SwiftUISession::new(source) -> Result<Session, Diagnostics>
session.render() -> Patch[]            // initial mount
session.dispatch(event) -> Patch[]     // discrete event → patches
session.on_patch(cb)                   // runtime-initiated pushes (Tier 6, async)
session.pump()                         // advance the async executor (Tier 6)
```

The native host instantiates the same object through a C ABI instead of
wasm-bindgen. `on_patch`/`pump` are inert until Tier 6.

---

## 4. The one runtime hook: `@State` persistent storage

Everything else is additive builtins + a Rust tree-walk. The single load-bearing
core change is `@State`:

- A view `struct` is **recreated on every `body` evaluation** (value type), so
  field-stored property-wrapper state would reset each render.
- The interpreter must hold a **persistent render-node tree** that owns each
  view's `@State` storage, keyed by **structural identity** (the Nth child of a
  given view keeps its state). `body` evaluation is recomputed and disposable;
  the state store persists.
- `@State` reads/writes resolve to this identity-keyed store, **not** the struct
  field. This is the only place SwiftUI reaches into interpreter behaviour, and
  it is required even by the v1 Counter slice.

Explicit `id`-based identity (for `ForEach` rows) is added in Tier 3; v1/Tier 2
use structural identity only.

### 4.1 What v1 actually shipped (revision)

The v1 slice reaches the same observable behaviour **without** a `tswift-core`
hook, so the "single load-bearing core change" above is **not** what landed:

- `@State` is a prelude `@propertyWrapper struct State<Value>` backed by a
  **shared reference box** (`_StateBox`, a class). Because the box is a reference
  type, every copy of the view struct (and every closure that captured it) shares
  one cell, so `count += 1` in a `Button` action is visible on the next render.
- The `Session` instantiates the **root view once** and reuses that instance
  across `render`/`dispatch`, so the disposable-`body` reset problem never
  arises for the root. This is the structural-identity guarantee **for the root
  only**.
- **Bounded limitation:** a *child* view constructed inside `body` is recreated
  each render, so its `@State` would reset. v1 has no view composition (Tier 2),
  so no fixture hits this; the identity-keyed render-node store of §4 is deferred
  to whenever child `@State` / `ForEach` rows land (Tier 3), and is the real
  reason that store still earns its place in the roadmap.
- **Tier 5 observation (revision):** `ObservableObject`/`@Published`/
  `@StateObject`/`@ObservedObject` also land **prelude-only**, with no engine
  change. `@Published` is a transparent wrapper; the object is a class
  (reference), so interior mutations (`model.x = …`, `model.method()`) persist
  and the full re-render reflects them. A parent's `@StateObject` passed to a
  child's `@ObservedObject` is one shared reference, so a child mutation updates
  both views. **Bounded limits:** (1) the same root-only structural-identity
  rule applies — a *nested* custom view's inline `@StateObject` is re-created
  each render (resets), the same deferral as child `@State` above; (2)
  reassigning the whole object (`model = Model()`) from an action does **not**
  persist (it rebinds a value-type copy, not the reused root instance) — mutate
  through the reference instead.
- The sanctioned core seams that *did* land are generic, not SwiftUI-specific:
  `register_struct_method` (the view-modifier dispatch fallback) and
  `eval_block_values` (the `@ViewBuilder` shim). Two caveats, accepted for v1:
  the modifier fallback matches **any** struct receiver by method name (only
  active when SwiftUI is installed; user `View` composition isn't in v1 anyway),
  and leading-dot tokens shared across namespaces (`.black` is both `Color` and
  `FontWeight`) are ambiguous without contextual typing — write the qualified
  form (`Color.black`).

---

## 5. Autonomous verification loop (the core requirement)

The whole point: **a completed slice is verifiable headless, deterministically,
in CI, with no browser and no Apple toolchain.** Verification has four layers;
the first three are autonomous and the third is the interactive gate.

### 5.1 Layer A — Surface coverage (reuse framework-inventory verbatim)

SwiftUI is registered as a framework descriptor, exactly like Foundation:

- `tools/framework-inventory/frameworks.toml` already resolves the SwiftUI
  `.swiftinterface` from the SDK.
- `frameworks/swiftui/scope.toml` declares the in-scope view/modifier surface
  (the roadmap spine + the coverage denominator; out-of-scope types excluded).
- `tswift-swiftui` exposes `registered_keys()`, dumped by a `dump_registered_keys`
  test to `frameworks/swiftui/registered_keys.txt` (cannot drift; reads the live
  registry).
- `python3 tools/framework-inventory/coverage.py --framework swiftui [Type]`
  reports the three states: **missing / implemented / verified**.

This answers "what fraction of the targeted SwiftUI surface is wired up."

### 5.2 Layer B — UIIR golden snapshots (the deterministic gate) ⭐

This is the SwiftUI-specific gate and the primary autonomous signal. Each fixture
renders to a **canonical UIIR JSON** that is committed and asserted exactly —
same mechanism as the existing `golden_fixtures` test.

```
tests/swiftui-fixtures/
  counter.swift             # the source
  counter.uiir.json         # committed canonical UIIR golden (the gate)
```

Driven by a CLI subcommand so it is scriptable and reused by the harness:

```sh
tswift swiftui render tests/swiftui-fixtures/counter.swift   # → canonical UIIR JSON
```

A Rust harness (`crates/tswift-swiftui/tests/uiir_goldens.rs`) walks every
`*.swift`, renders it, and asserts byte-equality against `*.uiir.json`
(`UPDATE_GOLDEN=1` to regenerate). Deterministic, offline, no browser — this is
what an agent runs to know "this Swift produced exactly this semantic tree."

### 5.3 Layer C — Interaction (patch-stream) goldens (the interactive gate) ⭐

Layer B proves the *static* tree; this proves the **event → mutate → re-render →
patch** loop autonomously, still with no browser. A fixture may carry a scripted
event sequence and an expected patch stream:

```
tests/swiftui-fixtures/
  counter.swift
  counter.events.json       # [ {"id":"0.2.1","event":"tap"}, {"id":"0.2.0","event":"tap"} ]
  counter.patches.json      # committed expected patch stream per event (golden)
```

```sh
tswift swiftui dispatch tests/swiftui-fixtures/counter.swift counter.events.json
# → JSON array of patch lists, one per event
```

The harness replays events through `SwiftUISession::dispatch` and asserts the
patch stream equals the golden. This is the autonomous proof that `@State`,
identity, and the diff engine work — the thing a browser screenshot can't certify
deterministically.

### 5.4 Layer D — Native-vs-web screenshot diff (non-blocking artifact)

A **macOS-only, non-gating** CI job renders each fixture two ways — native
(`swiftc` + a tiny SwiftUI snapshot harness in the simulator) and web (UIIR →
`<swiftui-canvas>` → Playwright headless screenshot) — and publishes a
**side-by-side perceptual diff as an artifact**. It catches "tree is right but the
CSS mapping looks wrong," which Layers B/C cannot see. It is explicitly **not a
gate** (system fonts/antialiasing/dynamic-type make pixel-parity unattainable),
and it doubles as the reference render when the native host is built.

### 5.5 The "done" definition for any SwiftUI slice

A view/modifier/feature is **verified** (and may be checked off) when **all** hold:

1. Registered in `tswift-swiftui` (`registered_keys()` — Layer A).
2. Exercised by a passing **UIIR golden** (Layer B).
3. If interactive, exercised by a passing **patch-stream golden** (Layer C).
4. Its frontend constructs parse + type-check cleanly (existing
   `golden_fixtures` discipline).

Layers A–C run offline in CI and are what an autonomous agent uses to self-verify.
Layer D is human-reviewed confidence only.

---

## 6. Crate & repo layout

```
crates/tswift-swiftui/              # NEW — SwiftUI primitives as Rust builtins
  src/lib.rs                        #   install(&mut Interpreter); registered_keys()
  src/views/  (text, stack, button, shape, …)
  src/modifiers/                    #   font, color, frame, padding, background, …
  src/uiir.rs                       #   view-value tree → canonical UIIR JSON
  src/diff.rs                       #   keyed diff → patch stream
  tests/uiir_goldens.rs             #   Layer B harness
  tests/patch_goldens.rs            #   Layer C harness

crates/tswift-wasm/                 # promote: add SwiftUISession (render/dispatch/on_patch/pump)
crates/tswift-cli/                  # add `swiftui render` / `swiftui dispatch` subcommands

frameworks/swiftui/                 # NEW — framework descriptor (Layer A)
  scope.toml  inventory.md  registered_keys.txt

tests/swiftui-fixtures/             # NEW — *.swift + *.uiir.json (+ *.events/*.patches.json)

web/swiftui-canvas/                 # NEW — dependency-free host package
  package.json  tsconfig.json
  src/
    canvas.ts                       #   <swiftui-canvas> custom element + Shadow DOM
    apply-patch.ts                  #   Map<nodeId,Element> + applyPatch
    modifier-css.ts                 #   SwiftUI-modifier → CSS design system
  example/                          #   editor + preview demo
```

The Studio chrome (editor, tabs) is **not** in conflict with the canvas: it uses
**CodeMirror 6** + the existing **Astro** site and never touches the UIIR/patch
path.

---

## 7. Roadmap (staged, each tier = one new mechanism)

### v1 — the **Counter** slice (prove the whole loop)
- Runtime: `struct: View`, `var body: some View`, **`@State` persistent store
  (§4)**, the narrow `@ViewBuilder` shim (`buildBlock` + fixed arities,
  `if`/`if-else`), tap → mutate → re-eval → patch.
- Views: `Text` (literal + `\(interp)`), `Button(label){action}`, `VStack`,
  `HStack`, `Spacer`.
- Modifiers: `.font`, `.fontWeight`, `.foregroundColor`, `.frame(width:height:)`,
  `.background`, `.cornerRadius`.
- Host: `<swiftui-canvas>` rendering it. Verification: Counter UIIR golden +
  patch-stream golden + one Layer-D artifact.
- **Resist widening v1** — the value is a *complete* loop, not breadth.

### Tier 2 — the remaining three demo tabs (each adds one concept)
- **Greeting** (`toggle · ternary`): `Bool @State`, ternary in `body`,
  `buildEither`. Use `Toggle(isOn:)` against own `@State` (no real `@Binding`).
- **Stack** (`vstack · shapes`): `ZStack`, `Circle`/`RoundedRectangle`/`Rectangle`,
  `.fill`, `.frame` on shapes.
- **Profile** (`composition`): custom sub-`View`s composed, params passed down.

### Tier 3 — Dynamic collections & identity
`ForEach(_, id:)`, `Identifiable`, `List`, `Section`. **Unlocks** explicit keyed
identity + the **`move` patch op** + the **keyed diff** (the reconciliation work
deferred in §3.2). Hardest pure-reconciler work; element-identity preservation on
reorder.

### Tier 4 — Bindings & input controls
Real `@Binding`, `TextField`, `Toggle(isOn:)`, `Slider`, `Stepper`, `Picker`.
**Unlocks** two-way data flow + **focus/caret/IME preservation** in the web host
(the reason §3.2 uses keyed `setProp`, not `innerHTML` replace). Controlled-input
discipline (runtime is source of truth).

### Tier 5 — Observation & shared model state
`ObservableObject`/`@Published`, `@StateObject`/`@ObservedObject`, `@Observable`,
`@Environment`/`@EnvironmentObject`. **Unlocks** reference-type models + DI down
the tree + **fine-grained invalidation**. A real observation subsystem; couples to
the macro engine for `@Observable`.

### Tier 6 — Task & async ⚠️ *inflection — blocked on language async*
`.task`, `async/await` in views, `AsyncSequence`, `.refreshable`, loading/error
states. **Blocked-on:** the in-flight cooperative concurrency executor
(`docs/adr/0005`). **Requires** two protocol changes:
1. **Push channel** (`session.on_patch`) — runtime emits patches *unprompted* when
   an awaited task resolves (breaks the request→response model of §3.3).
2. **Executor pump** (`session.pump`) — single-threaded wasm must advance
   suspended tasks on the JS microtask/event loop.
   **Coordination ask:** the async executor must expose a **host-drivable pump**
   and not assume a native multi-threaded runtime.
Gets its own ADR.

### Tier 7 — Layout & geometry ⚠️ *inflection*
`ScrollView`, `LazyVStack`/`LazyHStack`/`Grid`/`LazyVGrid`, alignment guides,
`.fixedSize`, `.layoutPriority`, custom `Layout`, **`GeometryReader`**. Most layout
maps cleanly to CSS flex/grid — **but `GeometryReader` breaks the one-way flow:**
the host must **measure layout and feed sizes back into the runtime mid-render**, a
host→runtime round-trip we otherwise avoid. Kept in scope but isolated behind its
own ADR; if it proves too costly it can be declared permanently out of scope
(many previews never need it).

### Tier 8 — Navigation & presentation
`NavigationStack`/`NavigationLink`, `.sheet`, `.alert`, `.confirmationDialog`,
`.popover`, `TabView`, `.fullScreenCover`. **Unlocks** multiple screens + **out-of-
tree presentation surfaces** (overlays escape parent/child flow → the patch
protocol needs a "portal"/detached-root notion). Placed after layout because
list-/scroll-heavy screens want Tier 7 primitives.

### Backlog (deferred indefinitely, own ADR)
Animation (`withAnimation`, transitions, `matchedGeometryEffect`), gestures
(`DragGesture`, long-press, `@GestureState`), graphics (`Canvas`/`Path`,
gradients, shadows), `TimelineView`. Animations would likely be **declarative CSS
transitions driven by patch metadata** (keep Rust out of per-frame work);
continuous gestures need a streaming event channel, not the discrete protocol of
§3.3.

---

## 8. Risks & architectural inflections

- **Reconciliation correctness (Tier 3+).** We own keyed diffing and element-
  identity preservation that React would have given us. Bounded at preview scale;
  the §5.3 patch goldens are the safety net.
- **The async push/pump inflection (Tier 6).** Converts the clean request→response
  flow into runtime-initiated pushes; fully gated on language-level async
  (`docs/adr/0005`).
- **The `GeometryReader` round-trip (Tier 7).** The one feature that needs
  host→runtime layout feedback; isolated behind an ADR and droppable.
- **Result-builder generality (cross-cutting).** v1 ships a narrow `ViewBuilder`
  shim; real fixtures drive hardening the generic `@resultBuilder` implementation
  (checklist still `[~]`).
- **`@Observable` ↔ macro engine (Tier 5).** Depends on macro-expansion support
  the feature checklist lists as outstanding.

---

## 9. Decisions taken (grilled, with rationale)

1. **Runtime-evaluated UIIR, not static AST→JSX.** `@State` + `Button` actions
   require *executing* Swift per interaction; static transpilation would
   re-implement an evaluator in generated JS.
2. **Host-neutral semantic UIIR.** Enables a near-identity native host
   (React-DOM-vs-React-Native split); lowering lives in the adapter only.
3. **Stateful in-browser wasm session.** `@State` stays in live Rust memory
   (where ARC/value semantics already work); nothing serialized but tree+patches.
4. **No React; one Rust diff engine; thin `applyPatch` hosts.** We need a Rust
   diff engine for the native host anyway — build it once; both hosts become
   symmetric; matches the "no React" identity. Cost consciously accepted: we own
   reconciliation correctness.
5. **Shadow-DOM `<swiftui-canvas>` custom element.** Solves CSS isolation between
   the rendered SwiftUI and the host page, and makes the preview portable
   (the original "mount on any HTML document" goal). Shadow DOM is *encapsulation*,
   not diffing — the patch applier still does the work.
6. **No vdom/reactive framework on the canvas.** Preact/Lit/Solid ship their own
   reconciler → a literal conflict with the Rust diff engine. Canvas = `h()` +
   applier + CSS design system. CodeMirror+Astro only for the surrounding chrome.
7. **Views = `SwiftValue::Struct`; modifiers = flat CoW `_modifiers` field;
   `@State` = the one core hook.** No `tswift-core` change for view values; the
   view-value tree *is* the UIIR.
8. **Semantic-token value encoding (tagged union).** Pushes `.largeTitle`/`.indigo`
   resolution into hosts → cheap native parity; accepted iOS-vs-web drift.
9. **Whole-list `setModifiers` patches.** Simpler than per-modifier reorder
   diffing; modifier lists are short.
10. **Verification = deterministic UIIR + patch goldens (gate), screenshot diff
    (artifact).** Autonomous, offline, no Apple toolchain on the critical path.
11. **v1 = Counter only.** Thinnest complete loop; breadth is additive.
12. **Tier order 3→4→5→6→7→8**, async before layout/navigation, animation/gestures/
    graphics deferred. (User-directed reordering.)

---

## 10. Deliverables

- [x] `frameworks/swiftui/scope.toml` (in-scope view/modifier surface = roadmap +
      denominator) + regenerated `inventory.md` via `extract.py --framework swiftui`.
- [x] `crates/tswift-swiftui` skeleton: `install()`, `registered_keys()`,
      `dump_registered_keys` → `frameworks/swiftui/registered_keys.txt`.
- [x] `coverage.py --framework swiftui` reporting missing/implemented/verified.
- [x] `SwiftValue::Struct`-based view values + flat `_modifiers`; UIIR serializer
      (`uiir.rs`) emitting the §3.1 canonical JSON.
- [x] **`@State` persistence** — achieved without a `tswift-core` hook: a prelude
      `@propertyWrapper struct State<Value>` backed by a shared reference box,
      with the `Session` reusing one root instance across renders (§4 revised).
      The sanctioned core seam ended up being the generic `register_struct_method`
      (view-modifier dispatch) + `eval_block_values` (`@ViewBuilder`) instead.
- [x] Narrow `@ViewBuilder` shim (§7 v1) — `eval_block_values` collects each
      result-builder statement; containers filter to view values.
- [x] Rust diff engine (`diff.rs`) emitting the §3.2 patch ops (replace-heavy,
      `setText` fast-path).
- [~] `tswift swiftui render|dispatch` CLI subcommands shipped; the stateful
      `SwiftUISession` lives in `crates/tswift-swiftui::session` for now
      (promotion into `tswift-wasm` with `on_patch`/`pump` deferred to Tier 6).
- [x] `web/swiftui-canvas/` dependency-free host package: `<swiftui-canvas>`
      Shadow-DOM element + `src/apply-patch.ts` + `src/modifier-css.ts`
      (+ `scripts/validate.mjs` offline check and `example/` editor preview).
- [x] **Layer B harness** (`crates/tswift-cli/tests/swiftui_goldens.rs`) + Counter `*.uiir.json`.
- [x] **Layer C harness** (same file) + Counter `*.events.json`/`*.patches.json`.
- [ ] Layer D macOS screenshot-diff CI job (non-gating artifact).
- [ ] `framework-coverage`/`stdlib-coverage`-style skill note for SwiftUI, or
      extend the existing `framework-coverage` skill workflow.
- [ ] Tier 2 fixtures (Greeting/Stack/Profile) once v1 is green.

---

## 11. Open questions (resolve when the tier lands)

- **Tier 3:** keyed-diff identity algorithm (LCS vs keyed-map) and the exact
  `move` patch encoding.
- **Tier 6:** the precise `pump` contract with `docs/adr/0005`'s executor; how
  `on_patch` batches multiple async completions in one frame.
- **Tier 7:** whether `GeometryReader` stays in scope after the round-trip cost is
  measured; the layout-feedback channel shape.
- **Tier 8:** the "portal"/detached-root extension to the patch protocol for
  out-of-tree presentation.
</content>
</invoke>
