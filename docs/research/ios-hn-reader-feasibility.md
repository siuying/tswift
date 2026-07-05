# iOS HN Reader — Feasibility Research Spike

**Date:** 2026-07-05  
**Goal:** Determine what Swift subset the tswift runtime supports for a networked SwiftUI "Hacker News Reader" live preview, and produce a verified-working Swift source string.

---

## 1. Async / Task / URLSession / `.task {}` support

### async / await — ✅ FULLY SUPPORTED
File: `crates/tswift-core/src/interp/concurrency.rs` (entire module)  
Feature-checklist: `docs/swift-runtime/feature-checklist.md` lines 293–334

The runtime uses a **cooperative single-threaded executor**: every `await expr` evaluates `expr` inline to completion before returning. `async` functions, `async let`, `withTaskGroup`, `withUnsafeContinuation`, `for await`, `AsyncStream` are all implemented.

Key constraint: **`Task { }` spawns a pending task that is only drained at program end** (via `drain_pending_tasks()` in `Interpreter::run()`; see `concurrency.rs:313`). During `Session::dispatch`, `invoke_closure` is called but `drain_pending_tasks` is **not**. Therefore `Task { await fetch() }` inside a Button action runs the fetch **after** dispatch returns, after re-render — state never updates during the event.

### `.task {}` modifier — ❌ NOT IMPLEMENTED
File: `crates/tswift-swiftui/src/modifiers.rs:257`  
The `MODIFIER_TABLE` contains: `onAppear`, `onDisappear`, `onChange`, `onTapGesture`, `onLongPressGesture`, `onSubmit`, and others — **`.task {}` is absent**. There is no `.task` entry anywhere in `crates/tswift-swiftui/src/`.

### `await` directly in a Button/event closure — ✅ WORKS (non-standard Swift)
`Session::dispatch` (session.rs:459) calls `interp.invoke_closure(closure_id, vec![])`.  
`invoke_closure` → `call_closure` → cooperative executor runs `await` **inline**.  
Verified: a Button action containing `await someMethod()` executes the method fully during dispatch and the re-render sees the mutated `@State`. Command + result:

```
cargo run -p tswift-cli -- swiftui dispatch /tmp/hn_test_await_v2.swift /tmp/ev_fetch.json
→ [[{"op":"setText","id":"0.0","text":"fetched!"}]]   # PASS
```

Note: In real Swift `Button("x") { await foo() }` is a type error (action is `() -> Void`). tswift's cooperative executor is transparent to sync/async; the closure runs synchronously from Rust's perspective.

### `URLSession.data(from:)` — ✅ SUPPORTED (requires transport)
File: `crates/tswift-core/src/interp/dispatch.rs:1814` (URLSessionDataTask), `crates/tswift-core/src/http.rs` (transport seam)  
Feature-checklist line 475: `URLSession data(from:)/data(for:)` fully implemented.

**No transport = interpreter-level error** (not Swift `URLError`):
```
error: unsupported construct: URLSession needs a network transport; this embedding has none configured
```
This is **not catchable** via Swift `do/catch` — it surfaces as an `EvalError`. The tswift CLI `swiftui render/dispatch` subcommand does not install a transport. The iOS host must call `context.installURLSessionHTTPHandler()`.

---

## 2. Async fetch → preview propagation mechanism

`Session::dispatch` in `crates/tswift-swiftui/src/session.rs:364`:

```
dispatch(event)
  └─ invoke_closure(closure_id, [])     # runs handler inline via cooperative executor
  └─ self.render()                      # re-renders with updated @State
  └─ run_watchers(baseline, tree)       # fires onChange watchers
```

- `await someMethod()` **directly** in the closure body: runs inline → `@State` updated → `render()` sees new data → patches emitted. ✅
- `Task { await someMethod() }` in the closure body: task spawned but not drained before `render()` → patches empty. ❌
- `.onAppear { await fetch() }`: `.onAppear` closure runs synchronously during `appear` event dispatch. `await` directly in the closure body runs inline (same mechanism as Button). Works but only fires when host sends `appear` event after mount.
- `.task {}`: not implemented, not routable. ❌

---

## 3. JSONDecoder / Codable

File: `crates/tswift-core/src/interp/coding.rs:1,584`  
Feature-checklist line 218: `Synthesized Codable` — ✅ fully implemented.  
Test fixture: `crates/tswift-cli/tests/fixtures/foundation_jsondecoder.swift`

`JSONDecoder().decode(T.self, from: data)` works for structs with `String`, `Int`, `Double`, `Bool`, `Optional`, nested structs, and arrays. `try?` wrapping works.

---

## 4. Chosen approach + capability tier

**TIER: `button-fetch`**

The most reliable approach given (1)–(3):

- Initial render: shows "Tap Load" button + empty list.
- On button tap: the Button's action closure calls `await loadStories()` **directly** (no `Task {}` wrapper). The cooperative executor runs the fetch inline, updates `@State`, and the session re-renders with data.
- **Non-standard Swift**: `Button("Load") { await loadStories() }` is a type error in the real Swift compiler (action is `() -> Void`). tswift accepts it because the runtime is transparent to sync/async.
- Alternative compliant spelling: call a synchronous wrapper that internally bridges the async fetch — but tswift's seam is already synchronous, so the direct `await` is the cleanest form here.

### Why not full-async (`.task {}` + auto-rerender)?
`.task {}` is unimplemented. There is no automatic "task completed → trigger rerender" hook in the session. The session is pull-driven: re-render only occurs on explicit `dispatch` calls.

### Why not sync-on-compile?
Compile runs `interp.run()` but that is for top-level declarations; the session renders `body` lazily per `render()`. Blocking on network during compile would require a different architecture.

---

## 5. Host-side requirements

1. Create a context with a real HTTP handler:
   ```swift
   let context = TSwiftContext()
   context.installURLSessionHTTPHandler()          // ios/TSwift/Sources/TSwiftCore/TSwiftHTTP.swift
   let session = PreviewSession(context: context)  // ios/TSwift/Sources/TSwiftUI/PreviewSession.swift
   ```
2. Pass context to `PreviewSession(context:)` (already supported; see `PreviewSession.init`).
3. iOS app needs `NSAppTransportSecurity` → `NSAllowsArbitraryLoads` or HN Firebase domain exception if ATS is on.
4. Without step 1, the HN reader source compiles and renders its initial state, but **tapping Load crashes dispatch** with an interpreter-level error (not a graceful Swift error).

---

## 6. Verified Swift source

Run: `cargo run -p tswift-cli -- swiftui render /tmp/hn_reader_final.swift`  
Result (PASS — renders initial UIIR):
```json
{"id":"0","kind":"NavigationStack","args":{},"modifiers":[],"children":[
  {"id":"0.0","kind":"VStack","args":{},"modifiers":[{"name":"navigationTitle","value":"Hacker News"}],"children":[
    {"id":"0.0.0","kind":"Text","args":{"verbatim":"Tap Load to fetch top stories"},"modifiers":[],"children":[]},
    {"id":"0.0.1","kind":"Button","args":{"title":"Load Top Stories"},"modifiers":[],"children":[]}
  ]}
]}
```

Dispatch verification (without transport): `cargo run -p tswift-cli -- swiftui dispatch ... /tmp/ev_loadbtn.json`  
→ `error: unsupported construct: URLSession needs a network transport` (expected; proves routing reaches URLSession).

### Ready-to-paste Swift source

```swift
import Foundation

struct HNStory: Decodable {
    let id: Int
    let title: String
    let by: String
    let score: Int
    let url: String?
}

struct HNReaderView: View {
    @State private var stories: [HNStory] = []
    @State private var statusMessage = "Tap Load to fetch top stories"
    @State private var isLoading = false

    func loadStories() async {
        isLoading = true
        statusMessage = "Loading…"
        do {
            let topUrl = URL(string: "https://hacker-news.firebaseio.com/v0/topstories.json")!
            let (idsData, _) = try await URLSession.shared.data(from: topUrl)
            let ids = try JSONDecoder().decode([Int].self, from: idsData)
            var result: [HNStory] = []
            for id in ids.prefix(20) {
                let itemUrl = URL(string: "https://hacker-news.firebaseio.com/v0/item/\(id).json")!
                if let (itemData, _) = try? await URLSession.shared.data(from: itemUrl),
                   let story = try? JSONDecoder().decode(HNStory.self, from: itemData) {
                    result.append(story)
                }
            }
            stories = result
            statusMessage = "Loaded \(result.count) stories"
        } catch {
            statusMessage = "Error: \(error)"
        }
        isLoading = false
    }

    var body: some View {
        NavigationStack {
            VStack {
                Text(statusMessage)
                if !isLoading {
                    Button("Load Top Stories") {
                        await loadStories()
                    }
                }
                if !stories.isEmpty {
                    List {
                        ForEach(stories, id: \.id) { story in
                            VStack {
                                Text(story.title)
                                Text("by \(story.by) | \(story.score) pts")
                            }
                        }
                    }
                }
            }
            .navigationTitle("Hacker News")
        }
    }
}
```

**Pattern note:** `Button("Load Top Stories") { await loadStories() }` uses `await` in a non-async closure. This is accepted by the tswift runtime (cooperative executor runs it inline) but is a Swift compiler error in real Swift 6. The iOS catalog host should suppress or not surface this diagnostic to the user.

---

## 7. Gaps / Risks

| Risk | Detail | Verify-by |
|------|--------|-----------|
| ForEach with custom `id: \.id` | Works (verified with struct) | — |
| `if let` in closure body | Not tested in dispatch path | Add dispatch test |
| Error string from `\(error)` | `error` is a `SwiftValue`; `\(error)` may print internal representation | Run with mock transport that fails |
| ATS blocking HN | Firebase API is HTTPS; needs standard ATS or domain exemption | iOS device test |
| Slow fetch (20 serial requests) | Each `data(from:)` blocks the interpreter thread synchronously | Limit prefix(5) for demo |
