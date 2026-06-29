# Plan — `.preferredColorScheme(_:)` (source-driven appearance)

**Status:** proposed
**Date:** 2026-06-28
**Related:**
- `docs/plan/swiftui-support.md` — the SwiftUI render-host strategy this extends
- `crates/tswift-swiftui/src/lib.rs` — view/modifier registry + `PRELUDE` tokens
- `crates/tswift-swiftui/src/uiir.rs` — UIIR JSON wire format (tagged-union tokens)
- `web/swiftui-canvas/src/canvas.ts` — the host that owns light/dark appearance
- `website/src/components/FullPlayground.astro` — the playground that hosts the
  `<swiftui-canvas>` preview (currently hardcodes appearance via `appearance=`)

---

## 1. Problem statement

In SwiftUI, a view's light/dark appearance is **source-driven**: it comes from
`.preferredColorScheme(_:)` (which sets the enclosing scene's scheme) or, absent
that, from the environment (`@Environment(\.colorScheme)` ← the device/OS
setting). A reader who sees a dark preview reasonably assumes *the code asked for
it* via `.preferredColorScheme(.dark)`.

Today that assumption is false:

- `.preferredColorScheme(_:)` is **not implemented** in `tswift-swiftui` (it is
  not in `MODIFIER_FNS`, and there is no `ColorScheme` token in `PRELUDE`).
- Appearance is decided **entirely host-side** — the `<swiftui-canvas>`
  `appearance` attribute (the prototype hardcodes `"light"`) or the OS
  `prefers-color-scheme` (which is what makes the Playwright dark snapshot
  baselines dark).

So appearance is divorced from the Swift source. The goal of this plan is to make
appearance **source-driven and faithful**: a view renders dark **iff** its source
says `.preferredColorScheme(.dark)`, otherwise it follows the current
environment.

## 2. Design (resolved)

`.preferredColorScheme(_:)` is a *scene-level* modifier: applied anywhere in the
tree it sets the appearance of the whole presentation. We model it as a normal
UIIR modifier that the **canvas** interprets at the host level (not as a
per-element CSS style).

### 2.1 Runtime (`tswift-swiftui`)

- Add a `ColorScheme` token to `PRELUDE`, same shape as `Color`/`Font`:
  ```swift
  struct ColorScheme { let token: String
      static let light = ColorScheme(token: "light")
      static let dark  = ColorScheme(token: "dark")
  }
  ```
- Register `.preferredColorScheme(_:)` in `MODIFIER_FNS` using the existing
  `modifier!` macro — it records a `_Modifier { name: "preferredColorScheme",
  value: <ColorScheme token> }` on the receiver view (copy-on-write), exactly
  like `.foregroundColor`.
- Extend `token_of` to recognize the `ColorScheme` type so the value serializes
  as a semantic token (see §2.2).

`.preferredColorScheme` accepts `ColorScheme?` in real SwiftUI (nil = inherit).
v1 supports the non-nil `.light`/`.dark` forms; `nil` is out of scope.

### 2.2 UIIR wire format (`uiir.rs`)

Add the tag mapping in `write_value`:
`"ColorScheme" => "colorScheme"`, so the modifier serializes as:

```json
{ "name": "preferredColorScheme", "value": { "$": "colorScheme", "name": "dark" } }
```

This is forward-compatible: existing hosts that don't understand it ignore the
modifier (the canvas already ignores unknown modifier names).

### 2.3 Host (`web/swiftui-canvas`) — effective appearance

The canvas resolves an **effective appearance** with this precedence:

1. **Source** — a `preferredColorScheme` modifier found in the mounted/patched
   UIIR tree (the root-most occurrence wins, mirroring how the outermost
   `.preferredColorScheme` sets the window in SwiftUI).
2. **Host attribute** — the embedder's `appearance="light|dark"`, if set.
3. **Environment** — otherwise `auto`, i.e. follow `prefers-color-scheme`.

Implementation: on `mount`/`applyPatches`, `PatchApplier` (which already walks
every node) records whether any node carries a `preferredColorScheme` modifier
and its value; `SwiftUICanvas` reflects the resolved scheme onto an internal
`data-scheme` attribute whose `:host([data-scheme="dark"])` rules mirror the
existing forced-appearance variables and `color-scheme`. Because it is recomputed
on every patch, a `@State`-driven `.preferredColorScheme(flag ? .dark : .light)`
flips live through the normal patch stream — no special channel.

`preferredColorScheme` must **not** be applied as a per-element CSS style; it is
consumed only for the host-level appearance resolution (skip it in
`applyModifiers`).

### 2.4 Prototype

Drop the hardcoded `appearance="light"` on `<swiftui-canvas>`. Appearance then
follows the source (`.preferredColorScheme`) and otherwise the viewer's OS. This
is now safe: the dark-screen "black-on-black" bug was a transparent-background
defect (an `#preview` id selector beating `:host`), already fixed — dark mode now
paints a proper dark screen with light labels.

## 3. Known limitation — `.light` is ambiguous (documented gap)

Leading-dot tokens resolve by *unique* static lookup across the prelude
namespaces (see the `PRELUDE` note in `lib.rs`). `FontWeight.light` already
exists, so once `ColorScheme.light` is added, the bare `.light` becomes
**ambiguous** and cannot resolve without contextual typing:

```swift
.preferredColorScheme(.dark)          // ✅ `.dark` is unique → ColorScheme.dark
.preferredColorScheme(.light)         // ❌ ambiguous: FontWeight.light vs ColorScheme.light
.preferredColorScheme(ColorScheme.light)  // ✅ qualified form works
```

This is the **same accepted limitation already documented for `.black`** (shared
by `Color` and `FontWeight`). Implementation tasks:

- Extend the `PRELUDE` doc-comment note in `lib.rs` to list `.light` alongside
  `.black` as an ambiguous leading-dot token requiring the qualified form.
- Add a unit test mirroring `qualified_token_resolves_when_leading_dot_is_ambiguous`
  asserting `ColorScheme.light` resolves while `.light` is rejected/ambiguous.

Contextual-typing disambiguation (so `.light` resolves from the
`preferredColorScheme(_:)` parameter type) is **out of scope**.

## 4. Snapshot strategy — default to the current environment

With source-driven appearance, the snapshot matrix stays meaningful by treating
the **current environment as the default selected value** when the source does
not specify a scheme — which is exactly SwiftUI's `@Environment(\.colorScheme)`
default:

- **Existing fixtures** (none use `.preferredColorScheme`) keep rendering per the
  Playwright project's emulated `colorScheme` — the `*-light-*` baselines stay
  light, the `*-dark-*` baselines stay dark. **No baseline churn.**
- **Add one fixture** — e.g. `tests/swiftui-fixtures/preferred_scheme.swift`
  using `.preferredColorScheme(.dark)` — to exercise the source-driven path. Its
  baseline must render **dark in *both* the light and dark projects**, proving the
  source overrides the environment. Regenerate goldens (`UPDATE_GOLDEN=1`) and
  the web baselines for that one fixture.

This keeps the dark projects honest: they verify the *environment default*, and
the new fixture verifies the *source override*.

## 5. Coverage

`.preferredColorScheme` flows into `registered_keys()` automatically via
`MODIFIER_FNS` (as `View.preferredColorScheme`), so framework-coverage tooling
picks it up with no extra wiring. Update the
`registered_keys_cover_v1_constructors` test's expected list accordingly.

## 6. Testing

- **Unit (`tswift-swiftui`)**: `render_root` of a view with
  `.preferredColorScheme(.dark)` carries the modifier; `uiir::to_json` emits
  `{"$":"colorScheme","name":"dark"}`; `ColorScheme.light` resolves, `.light` is
  ambiguous (§3).
- **Golden (`tswift-cli`)**: the new `preferred_scheme` fixture's `.uiir.json`.
- **Canvas**: a small DOM test (or snapshot) that mounting a tree with
  `preferredColorScheme: dark` sets the host to dark regardless of
  `prefers-color-scheme`, and that a patch flipping it updates the host.
- **Snapshot**: the new dark-in-both-projects fixture (§4).

## 7. Tasks

1. `PRELUDE`: add `ColorScheme` token; extend the ambiguity note to include
   `.light` (§3).
2. `MODIFIER_FNS`: register `preferredColorScheme`; extend `token_of`.
3. `uiir.rs`: add the `ColorScheme → "colorScheme"` tag mapping.
4. `canvas.ts` / `apply-patch.ts`: resolve effective appearance (source > host
   attribute > env), reflect via internal `data-scheme`; skip the modifier in
   `applyModifiers`.
5. Prototype: remove the hardcoded `appearance="light"`.
6. Fixtures: add `preferred_scheme.swift`; regenerate its golden + web baseline.
7. Tests: unit (§6), golden, canvas, snapshot; update `registered_keys` expected
   list.
8. Docs: note `.preferredColorScheme` support + the `.light` limitation in the
   canvas README and the prototype NOTES.

## 8. Out of scope

- `nil` (`ColorScheme?`) to inherit.
- Reading `@Environment(\.colorScheme)` inside `body`.
- Contextual-typing disambiguation of `.light`.
