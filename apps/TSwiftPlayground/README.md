# TSwift Playground (iOS app)

A product iOS app: edit Swift in a [Runestone](https://github.com/simonbs/Runestone)
code editor and watch a **live, interactive SwiftUI preview** rendered by the
tswift runtime. The product sibling of `examples/ios`, which stays a minimal
link-smoke demo.

```
edit (Runestone CodeEditor)
   │  debounced ~250ms
   ▼
PreviewSession.compile(source)   ── tswift-ffi ──▶ UIIR tree
   ▼
RenderHostView(model:) + .uiirEventSink(…)        (UiirRenderer)
   ▲                                   │ tap / toggle / type
   └────────── applyPatches ◀── PreviewSession.dispatch
```

The runtime bridge (`PreviewSession` over `tswift-ffi`) and the renderer
(`UiirRenderer`) are reused unchanged from `ios/`.

## Build & run

The project is generated with [xcodegen](https://github.com/yonaskolb/XcodeGen)
(the `.xcodeproj` is git-ignored):

```sh
cd apps/TSwiftPlayground
xcodegen generate
open TSwiftPlayground.xcodeproj   # or: xcodebuild build -scheme TSwiftPlayground -destination 'generic/platform=iOS Simulator'
```

SwiftPM resolves two things on first build:

- **Runestone** (pinned to `0.5.2` in `project.yml`) — the code editor.
- **`TSwiftFFI.xcframework`** — the native runtime, via `ios/TSwift/ffi.pin`
  (a published release) or a locally built `ios/TSwift/Artifacts/` (run
  `scripts/build-xcframework.sh` for fast local iteration). See ADR-0008.

## Structure

| File | Role |
|------|------|
| `Sources/TSwiftPlaygroundApp.swift` | `@main` app entry. |
| `Sources/PlaygroundView.swift` | Editor + live preview; debounced recompile; samples menu; inline error banner. |
| `Sources/CodeEditor.swift` | `UIViewRepresentable` over Runestone's `TextView`. |
| `Sources/Samples.swift` | Bundled starter snippets (mirrors the website gallery). |

## Notes / follow-ups

- **Syntax highlighting** is deferred — Runestone runs without a Tree-sitter
  language (plain monospaced text). Adding `tree-sitter-swift` is a follow-up,
  not a blocker.
- **Recompile rebuilds the `RenderModel`** (interaction state within a single
  event is preserved by patch-in-place; a *recompile* currently rebuilds the
  model). Acceptable for v1.
- The CI build (`ios-playground-build` in `.github/workflows/ci.yml`) is
  **non-gating** — the regression signal lives in the `TSwiftUI` /
  `UiirRenderer` tests, never the app.
