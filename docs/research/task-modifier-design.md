# `.task {}` Modifier — Implementation Design

**Date:** 2026-07-05  
**Status:** Design spike — no product code changed.

---

## 1. Findings (file:line evidence)

### 1.1 How `.onAppear {}` is registered and fired

**Registration** (`crates/tswift-swiftui/src/modifiers.rs`)

`modifier_on_appear` (line ~380) calls `attach_event(recv, "onAppear", "appear", vec![], action)`.
`attach_event` does two things:
1. Appends an `_Modifier { name: "onAppear" }` to the view's `_modifiers` list (marker the host sees).
2. Calls `set_handler(recv, "appear", closure)` which merges the closure into the view's
   `_handlers: { "appear": Closure(id) }` struct field.

The closure is **never serialized** to UIIR; only the marker modifier is visible to the host.

**Firing** (`crates/tswift-swiftui/src/session.rs`)

`Session::dispatch` (line ~364) pattern-matches `event.event` against known names. Any unknown
name (including `"appear"`) falls through to the generic branch (line ~415):

```rust
} else if let Some(closure_id) = find_handler(&tree, &event.id, name) {
    self.interp.invoke_closure(closure_id, Vec::new())?;
}
```

`invoke_closure` calls `call_closure` → cooperative executor. **`await` inside the closure runs
inline to completion** before returning. `drain_pending_tasks` is NOT called here; only bare
`await` expressions inside the closure body run synchronously. `Task { }` spawned inside the
closure would be deferred (but `.task {}` shouldn't use `Task {}` inside itself).

`dispatch` then calls `session.render()` → new tree → caller diffs → patches.

**The host drives `appear`** — `render/compile` does NOT fire `onAppear` automatically.

**Probe result:**
```
# render alone (compile) — onAppear NOT fired:
→ text shows "Loading..."   # CONFIRMED

# dispatch with {"id":"0","event":"appear"}:
→ [[{"op":"setText","id":"0.0","text":"Loaded"}]]   # CONFIRMED: fired inline
```

`await` inside `.onAppear` also fires inline:
```
# dispatch {"id":"0","event":"appear"} on a view with .onAppear { await fetchData() }:
→ [[{"op":"setText","id":"0.0","text":"Loaded"}]]   # CONFIRMED
```

### 1.2 Initial tree emission and patch seam

`build()` in `crates/tswift-ffi/src/swiftui.rs` (line ~155):

```rust
let tree = session.render()?;   // initial tree
let tree_json = uiir::to_json(&tree);
bundle.session = Some(session);
Ok((bundle, tree_json, root))   // returned as {"ok":true,"tree":...}
```

`compile()` (line ~115) calls `build()` and emits the envelope in **one** JSON string.
`dispatch()` (line ~230) calls `session.dispatch(&event)` and diffs before/after to produce patches.

**Can the session produce a second render pass within one FFI call?** Yes — `build()` could call
`session.render()` a second time after firing task closures and include the diff as `taskPatches`
in the compile envelope. The seam is entirely in Rust; no extra FFI function needed. Alternatively,
a new `run_mount_tasks` FFI function can be added that fires tasks and returns patches, matching
the `dispatch` envelope contract exactly.

### 1.3 WHO-OWNS-DRIVING options

| Option | Mechanism | Files changed | FFI surface | UX | Notes |
|--------|-----------|--------------|-------------|-----|-------|
| **(A) Inline in compile** | After initial `session.render()` in `build()`, scan tree for `"task"` handlers, invoke them, re-render, return only the **final** tree. | `modifiers.rs`, `session.rs` (new `run_tasks_in`), `swiftui.rs` (`build`) | None new | No "Loading..." — compile blocks until tasks finish | Simplest; hides loading state. Fails cleanly if `await` inside task has no transport |
| **(B) Separate `run_mount_tasks` FFI call** | Compile returns initial tree ("Loading…"). New `tswift_swiftui_run_mount_tasks(ctx)` fires pending task closures and returns patches. Host calls it immediately after mount. | `modifiers.rs`, `session.rs`, `swiftui.rs`, `lib.rs` (FFI), `PreviewSession.swift` | **Yes** — new `tswift_swiftui_run_mount_tasks` | Shows "Loading…" then "Loaded" — two host round-trips | Matches real SwiftUI UX; `compile` stays non-blocking; testable independently |
| **(C) Reuse `appear` event** | Map `.task {}` → `.onAppear {}` (same `"appear"` key, merging closures). Host fires `appear` as it already does for `onAppear`. | `modifiers.rs` only | None | Same as existing `onAppear` UX | Loses distinctness; can't coexist with `onAppear` on same view; host must fire `appear` event (which current `PreviewSession` does NOT do automatically) |

### 1.4 Cancellation / onDisappear

Real SwiftUI cancels the `.task {}` work when the view disappears. The cooperative executor has no
real concurrency; a `.task` closure runs to completion inline during its firing, so there is
nothing to cancel mid-flight. `onDisappear` semantics can be **skipped entirely** for v1 of this
modifier. Risk: if the host sends a `disappear` event before the task fires, the task still runs
on the next `run_mount_tasks` call — an acceptable deviation noted below.

---

## 2. Recommended design — Option B

**Rationale:** Option B is the only choice that (a) shows the "Loading…" → "Loaded" transition
the demo needs, (b) keeps `tswift_swiftui_compile` non-blocking (the host is @MainActor and must
not block the render thread during network I/O), and (c) stays within the existing FFI contract
(patches envelope, identical to `dispatch`). Option A hides the loading state and may block
compile; Option C loses the `.task`/`.onAppear` distinction and requires the host to know to fire
an `appear` event (which `PreviewSession` does not do today).

The cooperative executor makes Option B behave exactly like Option A from Rust's perspective
(both call `invoke_closure` inline) — the only difference is *when* the host calls us.

---

## 3. Exact files / functions to change

### 3.1 `crates/tswift-swiftui/src/modifiers.rs`

Add `modifier_task`:

```rust
fn modifier_task(_ctx: &mut dyn StdContext, recv: SwiftValue, args: Vec<Arg>) -> StdResult {
    let action = args
        .into_iter()
        .find_map(|a| matches!(a.value, SwiftValue::Closure(_)).then_some(a.value));
    attach_event(recv, "task", "task", Vec::new(), action)
}
```

Add to `MODIFIER_FNS`:

```rust
("task", modifier_task),
```

This emits a `{ name: "task" }` marker modifier (visible to the host/renderer so it could
optionally style a loading indicator) and stores the closure under `"task"` in `_handlers`.

### 3.2 `crates/tswift-swiftui/src/session.rs`

Add `run_mount_tasks(&mut self) -> Result<SwiftValue, EvalError>`:

```rust
pub fn run_mount_tasks(&mut self) -> Result<SwiftValue, EvalError> {
    let tree = match &self.current {
        Some(t) => t.clone(),
        None => return self.render(),
    };
    // Collect all "task" closure ids from the current tree.
    let mut task_closures: Vec<usize> = Vec::new();
    crate::tree::walk(&tree, &mut |_, _, obj| {
        if let Some(SwiftValue::Struct(handlers)) = obj.get(HANDLERS_FIELD) {
            if let Some(SwiftValue::Closure(cid)) = handlers.get("task") {
                task_closures.push(*cid);
            }
        }
    });
    for cid in task_closures {
        self.interp.invoke_closure(cid, Vec::new())?;
    }
    let tree = self.render()?;
    // Re-run onChange watchers in case tasks mutated watched state.
    let baseline = collect_watch_values(&tree);
    self.run_watchers(baseline, tree)
}
```

`run_watchers` is already private but called here internally; if needed, expose it or inline.
Alternatively: collect baseline before firing tasks, fire tasks, then call `self.render()` and
`self.run_watchers(baseline, tree)`.

### 3.3 `crates/tswift-ffi/src/swiftui.rs`

Add `run_mount_tasks(slot: &mut Option<SwiftUiSession>) -> String`:

```rust
pub(crate) fn run_mount_tasks(slot: &mut Option<SwiftUiSession>) -> String {
    let Some(bundle) = slot.as_mut() else {
        return dispatch_error_json("no active SwiftUI session — compile first");
    };
    let Some(session) = bundle.session.as_mut() else {
        return dispatch_error_json("session has not rendered yet");
    };
    let Some(before) = session.current_tree().cloned() else {
        return dispatch_error_json("no rendered tree");
    };
    match session.run_mount_tasks() {
        Ok(after) => {
            let patches = diff::diff(&before, &after);
            format!(
                "{{\"ok\":true,\"patches\":{},\"error\":null}}",
                diff::to_json(&patches)
            )
        }
        Err(e) => dispatch_error_json(&e.to_string()),
    }
}
```

### 3.4 `crates/tswift-ffi/src/lib.rs`

Add the extern C function:

```rust
/// Fire any pending `.task {}` closures on the current session's tree and
/// return a patch stream (same envelope as `tswift_swiftui_dispatch`).
/// Call once after a successful `tswift_swiftui_compile` to show
/// post-mount state. Safe to call even if no `.task` modifiers exist
/// (returns an empty patch list).
///
/// # Safety
/// `ctx` must be a live pointer from `tswift_context_new`.
#[no_mangle]
pub unsafe extern "C" fn tswift_swiftui_run_mount_tasks(
    ctx: *mut Context,
    // future: node_id to scope to a subtree; pass null for whole tree
) -> *mut c_char {
    let Some(ctx) = ctx.as_mut() else {
        return into_json_ptr(swiftui::dispatch_error_json("null context"));
    };
    into_json_ptr(swiftui::run_mount_tasks(&mut ctx.swiftui))
}
```

Add ABI drift-guard to the existing `c_abi_signatures_match_header` test:

```rust
let _run_tasks: unsafe extern "C" fn(*mut Context) -> *mut c_char =
    tswift_swiftui_run_mount_tasks;
```

Also update `include/tswift_ffi.h` with the new declaration.

### 3.5 `ios/TSwift/Sources/TSwiftUI/PreviewSession.swift`

After a successful compile, call `runMountTasks`:

```swift
public func compile(_ source: String) {
    // ... (existing code) ...
    guard envelope.ok, let tree = envelope.tree else { ... }
    root = envelope.root
    lastError = nil
    model = RenderModel(root: tree)
    // Fire .task {} closures — async tasks that run on mount.
    runMountTasks()
}

public func runMountTasks() {
    let raw = context.handle.withUnsafeMutablePointer { ptr -> String in
        guard let result = tswift_swiftui_run_mount_tasks(ptr) else { return "" }
        defer { tswift_string_free(result) }
        return String(cString: result)
    }
    // Decode the same DispatchEnvelope used by dispatch()
    guard let envelope = try? JSONDecoder().decode(DispatchEnvelope.self, from: Data(raw.utf8)),
          envelope.ok, let patches = envelope.patches else { return }
    model.apply(patches)
}
```

> Note: actual `context.handle` access depends on `TSwiftContext`'s API surface. The pattern
> mirrors the existing `dispatch` implementation in PreviewSession.

---

## 4. New / changed FFI signature

**Yes.** New function added (additive, no existing signature changes):

```c
// tswift_ffi.h
char *tswift_swiftui_run_mount_tasks(TSwiftContext *ctx);
```

Return: owned JSON string (same `{"ok":bool,"patches":[…],"error":string|null}` envelope as
`tswift_swiftui_dispatch`). Caller must free with `tswift_string_free`. Empty patch list `[]`
when no `.task` modifiers are registered. Error when no session exists.

---

## 5. Test plan

### Rust unit test — `crates/tswift-swiftui/src/session.rs`

```rust
#[test]
fn task_modifier_fires_on_run_mount_tasks() {
    let mut interp = events_interp(r#"
struct TaskView: View {
    @State private var loaded = false
    var body: some View {
        Text(loaded ? "Loaded" : "Loading...")
            .task { loaded = true }
    }
}
"#);
    let mut session = Session::new(&mut interp, "TaskView").expect("session");
    let initial = session.render().expect("render");
    assert!(uiir::to_json(&initial).contains("Loading..."), "initial");
    let after = session.run_mount_tasks().expect("run_mount_tasks");
    assert!(uiir::to_json(&after).contains("Loaded"), "after tasks");
}

#[test]
fn task_modifier_coexists_with_on_appear() {
    let mut interp = events_interp(r#"
struct TwoView: View {
    @State private var a = false
    @State private var b = false
    var body: some View {
        Text(a && b ? "both" : "neither")
            .onAppear { a = true }
            .task { b = true }
    }
}
"#);
    let mut session = Session::new(&mut interp, "TwoView").expect("session");
    session.render().expect("render");
    // Dispatch appear separately
    let appear_event = Event { id: "0".into(), event: "appear".into(), value: None };
    session.dispatch(&appear_event).expect("appear");
    // Then run tasks
    let after = session.run_mount_tasks().expect("tasks");
    assert!(uiir::to_json(&after).contains("both"), "both handlers fired");
}
```

### Rust integration test — `crates/tswift-ffi/src/swiftui.rs`

```rust
#[test]
fn compile_then_run_mount_tasks_patches_task_state() {
    let mut slot = None;
    let result = compile(&mut slot, r#"
struct V: View {
    @State private var ready = false
    var body: some View {
        Text(ready ? "ready" : "wait").task { ready = true }
    }
}
"#);
    assert!(result.contains("\"ok\":true"), "{result}");
    assert!(result.contains("wait"), "initial tree shows wait");
    let patches = run_mount_tasks(&mut slot);
    assert!(patches.contains("\"ok\":true"), "{patches}");
    assert!(patches.contains("ready"), "task updated state: {patches}");
}
```

### CLI smoke probe

```bash
# Verify .task fires via run_mount_tasks (once implemented):
cargo run -p tswift-cli -- swiftui render /tmp/probe_task.swift   # should show "Loading..."
# (CLI dispatch with a synthetic "task" event to verify routing)
echo '[{"id":"0","event":"task"}]' > /tmp/task_ev.json
cargo run -p tswift-cli -- swiftui dispatch /tmp/probe_task.swift /tmp/task_ev.json
# Expected: [[{"op":"setText","id":"0.0","text":"Loaded"}]]
```

---

## 6. Risks and skipped scope

| Risk | Likelihood | Mitigation |
|------|-----------|-----------|
| `_handlers` supports one closure per event key; two `.task {}` on the same view silently drops the first | Medium | Acceptable for v1; document limitation |
| `run_mount_tasks` called before `compile` → graceful error JSON (no session) | Low | Handled in implementation |
| Host calling `run_mount_tasks` with no `.task` modifiers → empty patches | Certain | Correct/benign behavior |
| URLSession in a `.task` closure needs transport set up before `run_mount_tasks` | Certain | Transport is set before compile in iOS host; no issue |
| Re-entrant `run_mount_tasks` (called twice) re-fires tasks | Low | Tasks are closures, calling twice mutates state twice — caller must not call twice. Document. |

**Skipped scope (v1):**

- **Cancellation**: `.task` in real SwiftUI is cancelled when the view disappears. Skipped — the cooperative executor has no mid-flight cancellation. Noted as tripwire: add a `"task_cancel"` event + `Task.cancel()` integration if/when the executor grows preemption.
- **`priority:` parameter**: `.task(priority: .background) { }` — the priority label is ignored (parsed, dropped). One modifier signature covers all priorities.
- **Multiple `.task {}` on one view**: first wins (same `set_handler` replace-semantics as all other events). Could be fixed with an array-valued handler slot; out of scope.
- **`onDisappear` cancellation pairing**: not implemented.
- **WASM host**: `tswift-wasm` has its own `swiftui.rs` session driver; a parallel `run_mount_tasks` export would be needed there. Out of scope for this spike.

---

## 7. Verified probes

```bash
# Probe 1: does render (compile) fire onAppear automatically?
cargo run -q -p tswift-cli -- swiftui render /tmp/probe_appear.swift
# → tree contains "Loading..." — onAppear NOT fired during render. CONFIRMED.

# Probe 2: does dispatching "appear" fire onAppear?
cargo run -q -p tswift-cli -- swiftui dispatch /tmp/probe_appear.swift /tmp/probe_appear_events.json
# events: [{"id":"0","event":"appear"}]
# → [[{"op":"setText","id":"0.0","text":"Loaded"}]]   CONFIRMED: appear fires inline.

# Probe 3: does await in onAppear run inline?
cargo run -q -p tswift-cli -- swiftui dispatch /tmp/probe_async_appear.swift /tmp/probe_appear_events.json
# → [[{"op":"setText","id":"0.0","text":"Loaded"}]]   CONFIRMED: await runs synchronously.

# Probe 4: does .task {} exist today?
cargo run -q -p tswift-cli -- swiftui render /tmp/probe_task.swift
# → error: unsupported construct: method .task() on VStack   CONFIRMED absent.
```
