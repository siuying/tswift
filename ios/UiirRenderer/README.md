# UiirRenderer — Layer D iOS UIIR Renderer

The native half of the SwiftUI Layer D verification loop
(`docs/plan/swiftui-support.md` §5.4, `docs/plan/layer-d-ios-renderer.md`).

It reads the offline-generated UIIR + patch fixtures from
`tests/swiftui-fixtures/`, builds **real SwiftUI views** from them, replays each
fixture's patch stream against a live tree, and snapshots every step with
[swift-snapshot-testing]. It is non-gating confidence that the iOS token/CSS
mapping matches the web `<swiftui-canvas>` host — not an interpreter test
(Layers B/C cover that).

[swift-snapshot-testing]: https://github.com/pointfreeco/swift-snapshot-testing

## Layout

| File | Role |
|---|---|
| `UiirValue.swift` / `UiirNode.swift` | `Decodable` mirror of the §3.1 UIIR wire format |
| `Patch.swift` | `Decodable` mirror of the §3.2 patch ops |
| `RenderModel.swift` | mutable tree + `apply(_:)` — mirrors `web/swiftui-canvas/src/apply-patch.ts` |
| `Tokens.swift` | `textStyle`/`weight`/`color` token tables — mirrors `modifier-css.ts` |
| `ModifierApply.swift` | ordered modifier → SwiftUI chain |
| `ViewFactory.swift` | UIIR `kind` → `AnyView` — mirrors `apply-patch.ts` `element()` |
| `FixtureLoader.swift` | locates `tests/swiftui-fixtures/` from `#filePath` |

## Pinned environment

Snapshots are scale/font sensitive. Baselines were recorded on:

- **Xcode** 26.1.1 / **Swift** 6.3.2
- **Simulator:** iPhone 16 Pro, **iOS 18.5**

Each fixture step is captured across a **device × appearance matrix** (4 images
per step), named `<step>-<device>-<scheme>`:

- `iphone` — `.iPhone13` (@3x) · `ipad` — `.iPadPro11(.portrait)` (@2x)
- `light` / `dark` — driven by the snapshot `UITraitCollection`

The host fills the device and uses `Color(.systemBackground)`, so semantic
colors and the background adapt to appearance. Re-record only on this
configuration or the baselines will mismatch.

## Run

```sh
cd ios/UiirRenderer
xcodebuild test -scheme UiirRenderer \
  -destination 'platform=iOS Simulator,name=iPhone 16 Pro,OS=18.5'
```

## Re-recording baselines

Set `recording = true` in `Tests/UiirRendererTests/SnapshotTests.swift`, run
once (it fails by design while recording), eyeball the new PNGs under
`Tests/UiirRendererTests/__Snapshots__/`, set `recording = false`, re-run to
confirm green, then commit.

Baselines are **Git LFS**-tracked (`.gitattributes` at the repo root). Ensure
`git lfs install` has been run locally before cloning/pulling.
