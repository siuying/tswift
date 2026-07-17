You are an architecture advisor for **tswift**, a lightweight Swift compiler +
runtime written in Rust. This is analysis only — do NOT edit, create, or delete
any files, and do NOT write code. Give a concrete recommendation with reasoning.

## Context

tswift interprets Swift and renders SwiftUI to a UI intermediate representation
("UIIR") — a serializable node tree consumed by a web host. SwiftUI views are
modeled as `SwiftValue::Struct` records; view modifiers are chained calls that
add fields / children to those records, then a serializer emits UIIR nodes.

The runtime already supports:
- **Closures** as `SwiftValue::Closure(id)` — used today by `Button` actions,
  `onTapGesture`, `onAppear`, `onChange`, and `@ViewBuilder` trailing closures
  in `.overlay { … }` / `.background { … }` (nested views are evaluated and
  serialized as child UIIR nodes).
- **Bindings** — a `Binding<Value>` prelude struct backed by a shared
  `_StateBox` reference box; `$state` projects a two-way binding. `Toggle`,
  `TextField`, `Slider`, and `NavigationStack(path:)` already capture a binding
  into a `_binding` field (`BINDING_FIELD`) and read/write through it.

## The problem

A large tail of SwiftUI modifiers is still unimplemented because they need
**closures, bindings, and/or transactions combined**, beyond the simple
value/token passthroughs done so far. Representative targets:

1. `.sheet(isPresented:onDismiss:content:)`, `.popover(...)`,
   `.alert(_:isPresented:actions:message:)`,
   `.confirmationDialog(...)`, `.fullScreenCover(...)` — a `Binding<Bool>` (or
   `Binding<Item?>`) gates a `@ViewBuilder` content closure that must be
   evaluated and serialized as a detached/overlay subtree, plus an `onDismiss`
   action closure.
2. `.matchedGeometryEffect(id:in:)` — needs a `Namespace.ID` identity token.
3. `.onKeyPress(_:action:)` — event closure returning a result enum.
4. `.visualEffect { content, proxy in … }` — closure taking a proxy and
   returning a modified effect value.
5. `.transaction { t in … }` / `withTransaction(_:)` — a `Transaction` value
   threaded through an animation-scoped mutation closure.
6. `.searchScopes(_:scopes:)` — a selection binding + `@ViewBuilder` scope list.

## The question

Given the existing closure + binding + `@ViewBuilder`-serialization
infrastructure, design the **runtime/UIIR contract** for these modifiers.
Specifically:

- **Presentation modifiers (sheet/popover/alert/etc.)**: How should the gating
  binding + deferred content closure be represented in UIIR so the web host can
  present/dismiss? Should content be eagerly serialized (like `.overlay`) or
  captured as a deferred closure re-evaluated on presentation? Who owns the
  presented/dismissed state — runtime or host — and how does dismissal write
  back through the binding? What is the smallest UIIR node/field contract that
  covers sheet, popover, alert, confirmationDialog, fullScreenCover uniformly?
- **Namespace / matchedGeometryEffect**: minimal identity-token model.
- **Transaction**: is a `Transaction` value + `withTransaction` worth modeling
  now, or should it degrade to a recorded animation context? Name the degraded
  tier honestly if so.
- **Sequencing**: which of these is the right *first* vertical slice to build
  (highest coverage unlock for least new infra), and what is a phased plan that
  keeps each step behavior-preserving and green?

## Files to read (do not edit)

- `crates/tswift-swiftui/src/modifiers.rs` — modifier install table; closure /
  `@ViewBuilder` handling (see `.overlay`/`.background`, `onTapGesture`,
  `onChange`, `make_handlers`, `HANDLERS_FIELD`).
- `crates/tswift-swiftui/src/lib.rs` — `Binding`/`_StateBox` prelude,
  `BINDING_FIELD`, `Toggle`/`TextField`/`Slider` binding capture.
- `crates/tswift-swiftui/src/uiir.rs` — UIIR node structure + serialization.
- `crates/tswift-swiftui/src/navigation.rs` — `NavigationStack(path:)` binding
  capture and `navigationDestination` (closest existing analogue to a gated,
  data-driven presented subtree).
- `crates/tswift-swiftui/src/session.rs` — session/render/diff loop (how UIIR
  is produced and re-rendered on state change).
- `crates/tswift-swiftui/src/diff.rs` — tree diffing (relevant to how a
  presented subtree would appear/disappear across renders).

Also skim `docs/adr/` for load-bearing SwiftUI/UIIR decisions before answering.

Deliverable: a recommended design for the presentation-modifier vertical slice
(the biggest unlock), the minimal UIIR contract, and a phased build order.
Analysis only. Do NOT edit, create, or delete any files. Do NOT write code.
