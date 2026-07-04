# ADR-0013: SwiftUI in-tree navigation and discrete async events

- **Status:** Accepted
- **Date:** 2026-07-05
- **Context slice:** SwiftUI navigation & async breadth (roadmap
  [`docs/plan/swiftui-usability-roadmap.md`](../plan/swiftui-usability-roadmap.md) §5
  "deferred tiers"; architecture [`docs/plan/swiftui-support.md`](../plan/swiftui-support.md)
  Tier 8 + §11)
- **Builds on:** ADR-0006 (render-host architecture), ADR-0009 (patch-metadata
  extension precedent)

## Context

The render-host contract (ADR-0006) has two load-bearing invariants:

1. **The UIIR is a pure tree.** Structural-path ids, keyed diffing, and a small
   closed patch vocabulary (`mount/insert/remove/move/replace/setText/setArgs/
   setModifiers`). There is no out-of-tree or detached-root notion.
2. **Events are discrete and request→response.** The host sends
   `{id, event, value}`; the runtime runs the handler synchronously,
   re-renders, diffs, and returns patches. The runtime never initiates a push.

`NavigationStack`, `NavigationLink`, `TabView`, and `AsyncImage` were deferred
because Tier 8 was assumed to need a "portal"/detached-root patch extension,
and async was assumed to need the executor pump seam (ADR-0005). This ADR
records that **neither extension is needed for these four features**, and names
the tripwires for when the extensions genuinely become necessary.

## Decision

### 1. Navigation stack is runtime-owned, in-tree state

`NavigationStack` renders **every screen in the stack as an ordinary child**:

```json
{"id":"0","kind":"NavigationStack","args":{},"children":[
  {<root screen>}, {<pushed screen>}, ...]}
```

- **Push.** Tapping a `NavigationLink` is a normal discrete event. The runtime
  appends the link's captured `destination` closure to per-stack session state
  and re-renders; the destination closure is evaluated at render time (and
  re-evaluated on every render, so pushed screens stay live against `@State`
  changes). The diff emits a plain `insert` under the stack node.
- **Pop.** The host's back affordance emits `{id: <stack id>, event: "back"}`;
  the runtime pops the stack state; the diff emits `remove`.
- **Host contract.** Children of a `NavigationStack` node form the navigation
  stack, topmost visible. A stack with more than one child shows a back
  affordance. `navigationTitle` is an ordinary recorded modifier on a screen's
  root node; the host renders the bar.
- **Fidelity tiers (named, honest).** iOS drives a **native**
  `NavigationStack`/`UINavigationController` from the children (system push/pop
  animation, swipe-back). Web renders a **faux** stack (show last child, CSS
  transition, back button in a rendered bar). This drift is accepted, like the
  color/SF-Symbol drift surfaces.
- **Scope split.** Destination-based `NavigationLink(destination:)` lands
  first. Value-based links + `navigationDestination(for:)` + `NavigationPath`
  land second (they add value→destination matching, not new protocol).

**Rejected: host-owned navigation** (link carries a pre-rendered destination
subtree; host pushes locally). Destinations that read runtime state cannot be
pre-rendered, and back/deep-link state would be split across the FFI boundary —
the least forgiving layer in the system.

### 2. TabView selection is runtime-owned

All tabs render eagerly as children (small N; the laziness drift from real
SwiftUI is accepted). `.tabItem { ... }` is a recorded modifier on each child;
the host builds the tab bar from those markers. Selecting a tab emits
`{id: <tabview id>, event: "select", value: <tag-or-index>}`; the runtime
updates the selection binding/state; selection is serialized as an arg so
changes flow through `setArgs`. Selection value: the child's `.tag(_)` when
present, else its index.

### 3. Event handlers generalize `_action` into a handler map

View values grow a `_handlers` field (event name → captured closure), the
generalization of `Button`'s `_action`. Closures never serialize; the UIIR
carries only **marker modifiers** (e.g. `{"name":"onTapGesture","value":null}`)
so hosts know which listeners to attach. Dispatch routes by event name into the
handler map. `onAppear`/`onDisappear` are fired by hosts on mount/unmount;
`onChange(of:)` is runtime-internal (watched-value comparison after each
dispatch, before diffing — hosts uninvolved).

### 4. AsyncImage loads host-side; phases arrive as discrete events

- **v1:** `AsyncImage(url:)` serializes as a node with a `url` arg; the host
  loads it natively (web `<img>`, iOS `AsyncImage`). Zero runtime async.
- **v1.5 (content/placeholder/phase closures):** the host reports load progress
  as an ordinary event `{id, event: "imagePhase", value: "empty"|"success"|
  "failure"}`; the runtime re-renders with that phase and the content closure
  runs. Same request→response shape as `set`; no runtime-initiated pushes.

## Tripwires (what would reopen this decision)

- **Portal/detached-root patch op** — needed the moment we implement genuinely
  out-of-tree presentation: `.sheet`, `.alert`, `.confirmationDialog`,
  `.popover`, `.fullScreenCover`. These escape parent/child flow and cannot be
  encoded as in-tree children without lying about z-order and dismissal
  semantics. When one of them is scheduled, extend the patch protocol (new ADR)
  rather than shoehorning.
- **Executor pump seam (ADR-0005 integration)** — needed the moment the
  *runtime* must initiate an update with no triggering host event: `.task`,
  `Task {}` spawned inside an event handler that mutates `@State` after
  dispatch returns, timers/`.onReceive`/`TimelineView`, `.refreshable`, or
  interpreted async networking driving UI. The seam is either a host-polled
  `pump()` that advances the executor and returns patches, or an `on_patch`
  host callback. This is a three-applier contract change; do not add it for
  anything expressible as a discrete host event.

## Consequences

- Navigation and tabs ship with **zero new patch ops** — existing goldens,
  diff engine, and appliers keep their contracts; hosts add view kinds and two
  event names (`back`, `select`).
- The runtime stays the single owner of all UI state (navigation, selection,
  image phase), preserving the one-way flow that makes patch goldens
  deterministic.
- Web and iOS navigation fidelity intentionally differ (faux vs native); this
  is a named drift surface, not a bug.
- `.task` and sheet-style presentation remain visibly deferred with explicit
  triggers, instead of silently blocked.
