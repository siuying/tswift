# ADR-0019: SwiftUI presentation modifiers as in-tree nodes

- **Status:** Accepted
- **Date:** 2026-07-17
- **Context slice:** SwiftUI modifier breadth — the binding-gated,
  deferred-content presentation family (`.sheet`, `.fullScreenCover`,
  `.popover`, and later `.alert`/`.confirmationDialog`).
- **Builds on:** ADR-0006 (render-host architecture), ADR-0013 (in-tree
  navigation; runtime-owned gated subtrees).

## Context

ADR-0013 deferred presentation modifiers behind a tripwire that assumed they
would need a "portal / detached-root patch op" — a break from the pure-tree
UIIR. Revisiting the code after ADR-0013 shipped, every mechanism a sheet needs
already exists:

- `NavigationStack` already appends runtime-owned, **gated subtrees as ordinary
  children** via a session rewrite pass (`session::nav_stack_node`), with no new
  patch ops — insert/remove fall out of the existing diff.
- `NavigationLink` already captures a **deferred `@ViewBuilder` closure**
  (`_destination`) re-evaluated fresh each render so the subtree stays live
  against `@State` (`realize_pushed_screen`).
- `NavigationStack(path:)` already treats a **binding as the single source of
  truth**, with host events writing back through it.

A sheet is exactly this pattern with a `Bool`/`Item?` gate instead of a path.
The only genuinely new thing is a **host rendering rule**, not a protocol change.

## Decision

### 1. Capture: a deferred `_presentations` record, not a `_Modifier`

`.sheet(isPresented:onDismiss:content:)` (and `.fullScreenCover`, `.popover`)
does **not** append a serialized `_Modifier`. It stashes a `_Presentation`
record onto the receiver's `_presentations` list (`PRESENTATIONS_FIELD`):

```
_Presentation { style, _binding, _content: Closure, _onDismiss: Closure? }
```

All fields are internal (`_`-prefixed → never serialized), so every existing
golden is byte-identical. Content is **deferred** (the closure is stored, not
evaluated) — a closed sheet never runs its body, and an open sheet re-reads
`@State` on every render.

### 2. Realize: an in-tree `Presentation` child node

A session render pass (`session::presentation_node`, peer of `nav_stack_node`)
reads each record's binding. When it reads truthy (`Bool(true)`, or a non-`nil`
`item:` value), it evaluates the content closure and appends a node as the
presenting node's **last child**:

```json
{"id":"0.2","kind":"Presentation","args":{"style":"sheet"},
 "modifiers":[],"children":[ <realized content subtree> ]}
```

- **Child-of-presenter, not hoisted to root:** structural ids stay local and
  stable; a popover's anchor is its parent node for free; content inherits the
  presenter's environment (matching SwiftUI).
- **One node kind + `style` arg** covers `sheet`/`fullScreenCover`/`popover`
  (and later `alert`/`confirmationDialog`) uniformly.

**Host contract (named degraded tier):** a `Presentation` child never renders in
flow — the applier portals it to a top layer (web: `position:fixed` overlay +
scrim; iOS: native `.sheet`/`.popover`). This is an explicit rendering rule, not
a new patch op.

### 3. State ownership: runtime, via the binding

The gating binding is the sole source of truth (ADR-0006/0013 invariant). The
host holds no presented/dismissed state and emits one new event:

- `{id: <Presentation node id>, event: "dismiss"}` → the runtime writes
  `false` (`isPresented:`) or `nil` (`item:`) through the binding and fires the
  captured `onDismiss` closure. The next render drops the node; the diff emits a
  plain `remove`.
- **Programmatic close** (a button inside the sheet setting the state `false`)
  needs nothing new — the next render simply omits the node.

## Consequences

- No new patch op; no new UIIR structural concept; zero golden churn for
  existing fixtures. The re-tripwire for a real portal patch op is unchanged:
  only needed if a presentation must survive its presenter's removal (which
  SwiftUI itself does not do).
- **Known fidelity gap (degraded tier):** `onDismiss` fires on a host `dismiss`
  event, but *not* on programmatic close (state set to `false` from inside).
  SwiftUI fires it on any dismissal. Closing the gap needs the session to track
  per-node presented state across renders and detect true→false transitions;
  deferred until a fixture demands it.
- Deferred siblings, unchanged from the advisor analysis: `.alert` /
  `.confirmationDialog` (same node kind + `title`/message args + auto-dismiss on
  action), `@Namespace`/`matchedGeometryEffect` (identity token, no morph),
  `.onKeyPress` (needs a handled-flag on the dispatch response),
  `.visualEffect` (needs host geometry feedback), `.transaction`
  (animation-hint tier only).
