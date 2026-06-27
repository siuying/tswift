# ADR-0006: SwiftUI render host — runtime-evaluated, host-neutral UIIR with the diff engine in Rust

- **Status:** Accepted
- **Date:** 2026-06-27
- **Context slice:** SwiftUI rendering (framework layer)
- **Target platform:** iOS — the SwiftUI surface is extracted from the iPhoneSimulator SDK (not macOS); see `tools/framework-inventory/frameworks.toml` `[swiftui]`
- **Builds on:** ADR-0002 (tree-walker), `docs/plan/framework-support.md` (framework descriptors)
- **Drives:** `docs/plan/swiftui-support.md` (the staged implementation plan)

## Context

We want to **render SwiftUI in a browser — and later a native host — without a
Swift toolchain, LLVM, or native code generation**, so a user can edit a SwiftUI
`View` on the web and get a live, interactive preview, and so the same view tree
can later drive a real native renderer.

Unlike the stdlib and Foundation (value semantics the runtime reimplements as
behaviour), SwiftUI needs a **render/diff host**: instantiate a `View`, run
`body`, produce a view tree, react to events by mutating `@State`, re-evaluate,
and update the host. `docs/plan/framework-support.md` already flagged SwiftUI as
needing "a render/diff host, not value semantics … gated behind its own ADR."
This is that ADR. It records the cross-cutting architecture decisions; the staged
tiers, wire-format details, and verification loop live in
`docs/plan/swiftui-support.md`.

The key forces:

- **Interactivity is mandatory.** `@State` + `Button { count += 1 }` + live
  preview require *executing* Swift on every interaction.
- **Two hosts, one truth.** We want both a web (HTML/CSS) host *and* a future
  native (SwiftUI/UIKit) host to render the *same* view tree.
- **Product identity.** The project's stated identity is "no React, no Web
  Components, no proprietary runtime" — one compiler, one runtime, under one roof.
- **Autonomous verification.** A completed slice must be machine-verifiable,
  deterministically, offline, with no Apple toolchain on the critical path.

## Decision

Build a **runtime-evaluated, host-neutral UIIR** where the **diff engine lives in
Rust** and each host is a **thin patch-applier**. Seven decisions:

1. **Runtime-evaluated UIIR, not static AST→JSX transpilation.** The tree-walking
   interpreter instantiates the `View`, runs `body`, and produces an intermediate
   view tree (the UIIR). Static transpilation cannot express `@State` mutation,
   control flow in `body`, or event closures without re-implementing an evaluator
   in generated JS. We already have the evaluator (`crates/tswift-core`); reuse it.

2. **Host-neutral, semantic UIIR.** UIIR nodes are SwiftUI *concepts*
   (`VStack`, `Text`, `.font(.largeTitle)`), never pre-lowered to DOM. The
   concept→host-primitive lowering happens **only in the host adapter**. This is
   the React-DOM-vs-React-Native split: it makes the native host a near-identity
   mapping (`Text → SwiftUI.Text`, `.font(.largeTitle) → .font(.largeTitle)`)
   while the web adapter maps the same nodes to `<span>` + CSS.

3. **Stateful in-browser wasm session.** The interpreter instance lives in wasm
   and stays alive across interactions; `@State` is held in live Rust memory and
   never crosses the boundary. Only the view tree and patches are serialized. We
   promote `crates/tswift-wasm` from one-shot `runSwift` to a `SwiftUISession`
   handle (`render` / `dispatch` / later `on_patch` / `pump`); the native host
   instantiates the same object through a C ABI.

4. **No React; the Rust runtime owns the single diff engine; hosts are thin
   `applyPatch` appliers.** A Rust diff engine is needed for the native host
   regardless (no React reconciler there), so we build it once and both hosts
   share it. The runtime emits a **keyed patch stream**
   (`mount`/`insert`/`remove`/`replace`/`setText`/`setModifiers`/`setArgs`;
   `move` arrives with `ForEach`). Each host is `applyPatch` over a
   `Map<nodeId, hostNode>`. This matches the product identity and unifies web +
   native under one reconciler. **No vdom/reactive framework** (Preact/Lit/Solid)
   on the render path — they ship a competing reconciler and would fight the Rust
   one for the DOM.

5. **Shadow-DOM `<swiftui-canvas>` custom element for the web host.** The canvas
   is a Web Component with a shadow root for **CSS isolation** (host-page styles
   can't leak in; the SwiftUI-modifier→CSS design system can't leak out) and
   **portability** (droppable on any HTML document — the original goal). Shadow
   DOM is *encapsulation only*; the Rust-driven patch applier still does all
   diffing/mutation, targeting nodes inside the shadow root. The surrounding
   "Studio" chrome (editor, tabs) is unaffected and uses CodeMirror + Astro.

6. **Views are `SwiftValue::Struct`; modifiers are a flat CoW field; `@State` is
   the one core hook.** SwiftUI primitives (`Text`, `VStack`, `Button`, …) are
   Rust builtins in a new `tswift-swiftui` crate, registered exactly like
   `tswift-std`/`tswift-foundation`. A view value is a `SwiftValue::Struct` with a
   `type_name` and a flat ordered `_modifiers` field; `.font(x)` returns a
   copy-on-write copy with one appended modifier. The view-value tree *is* the
   UIIR — **no `tswift-core` change for view values.** The single core touch is
   `@State`: because the view `struct` is recreated every `body` evaluation, its
   storage must live in a **persistent render-node tree keyed by structural
   identity** that the interpreter consults instead of the struct field.

7. **Verification is deterministic UIIR + patch goldens (gate); screenshot diff is
   a non-blocking artifact.** Correctness is certified offline by (a) UIIR golden
   snapshots — `tswift swiftui render foo.swift` → canonical JSON asserted
   byte-exact, and (b) patch-stream goldens — `tswift swiftui dispatch foo.swift
   events.json` replayed and asserted. Surface coverage reuses the
   `framework-inventory` three-state loop. A macOS-only native-vs-web perceptual
   screenshot diff is an **artifact, not a gate** (pixel-parity with native is
   unattainable). This keeps the autonomous signal fast, deterministic, and free
   of any Apple toolchain on the critical path.

### Fidelity boundary (explicitly accepted)

- **The view tree and interaction loop are faithful:** `@State` persists across
  renders by identity, events mutate it, `body` re-evaluates, and the host is
  patched to match.
- **Pixel rendering is not iOS-identical.** System fonts, antialiasing, blur, and
  dynamic type differ between native and web. Semantic tokens (`.largeTitle`,
  `.indigo`) are resolved *per host*, so colors/typography drift by design.
  Fixtures assert on the **UIIR/patch goldens**, never on native-identical pixels.

## Consequences

- **Good:** one render/diff engine (Rust) serves both web and native; the web
  host is dependency-free and portable; SwiftUI plugs into the existing framework
  pattern with **no `tswift-core` change for view values**; verification is
  autonomous, deterministic, and offline.
- **Cost / risk:** we own reconciliation correctness that React would have given
  us — keyed moves and **element-identity preservation** so focus/caret/IME/scroll
  survive a re-render (bites at Tier 3 `ForEach` and Tier 4 input controls). The
  patch-stream goldens are the safety net. The single `@State` interpreter hook is
  load-bearing and required even by the v1 Counter slice.
- **Two later inflections, each its own ADR:** (a) **async** (`.task`) converts
  the request→response patch flow into runtime-initiated pushes and needs a
  host-drivable executor pump — gated on ADR-0005's executor; (b)
  **`GeometryReader`** needs a host→runtime layout round-trip that breaks the
  one-way flow — kept in scope but droppable.
- **Migration path:** the native host is purely additive — it is a second
  `applyPatch` over the same patch stream; nothing in the runtime or UIIR changes
  to support it. Animation/gestures/graphics are deferred to a backlog ADR and do
  not affect this core.

## Notes

- `unsafe` confinement (ADR-0001) is preserved: `tswift-swiftui` is safe Rust
  builtins + a tree-walk; the diff engine is safe Rust over the UIIR.
- The staged roadmap (v1 Counter → Tier 8), the full UIIR/patch schema, the
  crate/repo layout, and the four-layer verification loop live in
  `docs/plan/swiftui-support.md`.
- SwiftUI is registered as a `framework-inventory` descriptor; the scope manifest
  is `frameworks/swiftui/scope.toml` and the live registry dumps to
  `frameworks/swiftui/registered_keys.txt`.
</content>
