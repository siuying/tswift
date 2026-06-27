# Plan — Layer D: iOS UIIR Renderer & Snapshot Harness (local)

**Status:** proposed
**Date:** 2026-06-28
**Parent:** `docs/plan/swiftui-support.md` §5.4 (Layer D)
**Scope note:** This plan covers building the iOS UIIR renderer and running its
snapshot tests **locally** (macOS + Xcode + iOS Simulator). The CI job is
explicitly **out of scope for now** — we want a green local `xcodebuild test`
first. CI wiring is a follow-up once baselines exist.

---

## 1. Goal

Build a Swift package that reads the pre-generated UIIR JSON
(`tests/swiftui-fixtures/*.uiir.json`) and patch JSON (`*.patches.json`),
constructs **real SwiftUI views** from them, applies patches to a live tree, and
captures snapshots via [swift-snapshot-testing] at each step. This is the
native half of the Layer D perceptual-diff loop.

What this is **not**:
- Not a UITest target — events come from `*.patches.json`, not gestures (decided).
- Not dependent on the Rust static lib — the harness only reads JSON (decided).
- Not a gate — pixel parity with web is unattainable; it is confidence only.

The single deliverable for this plan: a Swift package under `ios/` whose
`xcodebuild test` builds the renderer, mounts every fixture, applies its
patches, and produces committed snapshot baselines.

[swift-snapshot-testing]: https://github.com/pointfreeco/swift-snapshot-testing

---

## 2. Decisions (resolving the handoff's open questions)

1. **Layout: standalone SwiftPM package** at `ios/UiirRenderer/` with a library
   target (`UiirRenderer`) and a test target (`UiirRendererTests`). No Xcode
   `.xcodeproj`/workspace — SwiftPM is enough for `xcodebuild test
   -scheme UiirRenderer -destination 'platform=iOS Simulator,...'`. Keeps the
   repo tooling-light and the renderer reusable by a future native host.

2. **`AnyView` type erasure.** Each UIIR node maps to a `func render(_ node)
   -> AnyView`. This is a test harness, not production; `AnyView` is the right
   call and avoids a generic-ViewBuilder maze. Confirmed acceptable.

3. **Mutable tree via a reference model.** Wrap the decoded tree in a
   `final class RenderModel: ObservableObject` holding the root `UiirNode` as
   a mutable value tree (struct nodes, replaced in place by id). Patch
   application mutates it and bumps `objectWillChange`; the snapshot host view
   observes it. (For snapshots we can also just re-decode + re-render between
   asserts — but a real mutable model exercises the patch logic, which is the
   point of mirroring `apply-patch.ts`.)

4. **Fixtures are read from the repo, not copied.** The test target locates
   `tests/swiftui-fixtures/` by walking up from `#filePath`, so fixtures stay
   single-sourced. (SwiftPM resources are an alternative but would duplicate.)

5. **Baselines committed under** `ios/UiirRenderer/Tests/UiirRendererTests/__Snapshots__/`
   (swift-snapshot-testing convention), **tracked via Git LFS** — PNGs are
   binary and churn on re-record, so `.gitattributes` routes
   `ios/UiirRenderer/Tests/**/__Snapshots__/**/*.png` through LFS. First run in
   record mode, then commit.

6. **Simulator pinning** recorded in `ios/UiirRenderer/README.md`. Local
   toolchain is Xcode 26.1.1 / Swift 6.3.2; pin **iPhone 16 Pro, iOS 18.5**
   (@3x) so reruns are deterministic.

---

## 3. Package layout

```
ios/UiirRenderer/
  Package.swift                       # SwiftPM manifest; swift-snapshot-testing dep
  README.md                           # how to run; pinned simulator
  Sources/UiirRenderer/
    UiirValue.swift                   # tagged-union Decodable (null|num|str|bool|{$,name}|{k:v})
    UiirNode.swift                    # UiirNode + UiirModifier Decodable
    Patch.swift                       # Patch enum Decodable (mirror apply-patch.ts ops)
    RenderModel.swift                 # mutable node tree + applyPatch (mirror apply-patch.ts)
    ViewFactory.swift                 # UiirNode -> AnyView (kind table)
    ModifierApply.swift               # modifiers -> SwiftUI (mirror modifier-css.ts tables)
    Tokens.swift                      # textStyle/weight/color name tables -> SwiftUI values
    FixtureLoader.swift               # locate + decode tests/swiftui-fixtures/*.json
  Tests/UiirRendererTests/
    SnapshotTests.swift               # per-fixture initial + per-patch-step asserts
    __Snapshots__/                    # committed PNG baselines (after record run)
```

---

## 4. Implementation slices (TDD, fixture by fixture)

Use the **`tdd`** skill. Each slice = decode model → render → snapshot →
commit baseline. Build the simplest fixtures first; each later fixture adds
exactly one renderer capability, mirroring the web host's tier order.

### Slice 0 — package + decode + first snapshot (`counter`)
- `Package.swift` with swift-snapshot-testing.
- `UiirValue`, `UiirModifier`, `UiirNode` `Decodable`; unit-test that
  `counter.uiir.json` round-trips (id/kind/args/modifiers/children).
- `ViewFactory` for `VStack`, `Text`, `Button`; `ModifierApply` for `font`,
  `fontWeight`, `foregroundColor`, `padding`, `background`, `cornerRadius`.
- `Tokens`: `textStyle` (`largeTitle…`) → `Font`, `weight` → `Font.Weight`,
  `color` (`white/blue/indigo…`) → `Color`. Mirror the names in
  `web/swiftui-canvas/src/modifier-css.ts`.
- `SnapshotTests.testCounterInitial()` — `assertSnapshot(of: host, as: .image(...))`.
- `Patch` enum + `RenderModel.apply`: `setText`. Replay `counter.patches.json`
  (two `setText`), snapshot after each.

### Slice 1 — `greeting` (Toggle, ternary body, setModifiers)
- `Toggle(args.label, isOn: .constant(args.isOn))`.
- Patch `setModifiers` (whole-list replace) in `RenderModel`.

### Slice 2 — `stack` + `profile` (non-interactive)
- `ZStack`, `Circle`, `Rectangle`, `RoundedRectangle(cornerRadius:)`,
  `Spacer`, `HStack`; modifiers `fill`, `frame {width,height}`.
- `profile` exercises nested containers/composition (no new node kinds).

### Slice 3 — `list` + `sections` (collections, move patch)
- `List`, `Section(header:)`, `ForEach` (enumerate children by id).
- Patch `move(parentId, id, index)` + `insert`/`remove`/`replace` in
  `RenderModel` (mirror `apply-patch.ts` keyed reorder).

### Slice 4 — `controls` + `form` + `picker` (input controls)
- `Slider(value: .constant, in: lower...upper, step:)`,
  `Stepper(title, value: .constant, in:, step:)`,
  `TextField(placeholder, text: .constant)`, `SecureField`,
  `Picker(title, selection: .constant)` with tagged `Text` children → options.
- Patch `setArgs` (constructor-arg delta) in `RenderModel`.

### Slice 5 — `observable` + `environment`
- These add no new node kinds (Button/Text/VStack) — they're patch-stream
  fixtures. Confirm `setText`/`setModifiers` replay renders correctly.

After each slice: run `xcodebuild test`, record new baselines, eyeball the
PNGs, commit (`feat(ios)` per AGENTS.md conventions).

---

## 5. Key mirror points (keep iOS == web semantics)

| Source of truth (web) | iOS mirror |
|---|---|
| `apply-patch.ts` patch ops + keyed reorder | `Patch.swift` + `RenderModel.apply` |
| `modifier-css.ts` `applyModifiers` order/reset | `ModifierApply.swift` ordered chain |
| `modifier-css.ts` TEXT_STYLE/FONT_WEIGHT/COLOR tables | `Tokens.swift` |
| `apply-patch.ts` `element()` kind→primitive | `ViewFactory.swift` kind→AnyView |
| Picker tagged-child → `<option>` | Picker tagged `Text` child handling |

Mirror **ordering and token names exactly**; the whole value of Layer D is
catching drift between these two tables.

---

## 6. Running locally

```sh
cd ios/UiirRenderer
# first run records baselines:
xcodebuild test -scheme UiirRenderer \
  -destination 'platform=iOS Simulator,name=iPhone 16 Pro,OS=18.5'   # pin exact OS
# (record mode toggled in SnapshotTests via isRecording / withSnapshotTesting)
```

Document the exact `xcrun simctl list` device/runtime chosen in the package
README so reruns match the committed baselines.

---

## 7. Out of scope (follow-ups)

- **CI job** (macOS runner, Xcode/simulator pin, artifact upload) — deferred.
- **Web screenshot half** (Playwright on `<swiftui-canvas>`) — separate task.
- **Perceptual diff tooling** (`odiff`/`pixelmatch` side-by-side) — separate task.
- **Rust static lib / C ABI** native host (ADR-0006) — unrelated milestone.

---

## 8. Definition of done (this plan)

1. `ios/UiirRenderer/` SwiftPM package builds.
2. Every `tests/swiftui-fixtures/*.swift` fixture renders initial + each
   patch step via `assertSnapshot`.
3. Baselines committed under `__Snapshots__/`.
4. `xcodebuild test` passes locally against committed baselines.
5. `ios/UiirRenderer/README.md` documents the pinned simulator + run command.
