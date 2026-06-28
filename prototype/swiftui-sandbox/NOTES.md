# tswift SwiftUI sandbox prototype

**Question:** can a throwaway Astro app make tswift feel like CodePen/CodeSandbox
for SwiftUI — write one `View`, see it rendered live on the right, and have its
controls (buttons, toggles, sliders, pickers) actually drive `@State`? This is a
PoC toward a full multi-file editor.

Run locally:

```bash
cd prototype/swiftui-sandbox
npm install
npm run dev
```

Open <http://127.0.0.1:4322/>.

## How it works

```
Swift source ──▶ tswift frontend (lexer→parser→sema)
             ──▶ SwiftUI render host → UIIR tree (JSON)        [swiftUICompile]
             ──▶ <swiftui-canvas>.mount(tree)  — DOM in a device frame
control event ──▶ swiftUIDispatch(id, event, value) → @State mutation + diff
             ──▶ <swiftui-canvas>.applyPatches(stream) — in-place DOM update
```

There is **no server** and no codegen: the browser loads `tswift_wasm_bg.wasm`
and calls two functions in `crates/tswift-wasm/src/swiftui.rs`:

- `swiftUICompile(source)` → `{ok, root, tree, error}` — analyzes the program,
  finds the root `View`, starts a stateful `Session` (whose `@State` persists),
  renders the initial UIIR **tree**.
- `swiftUIDispatch(id, event, value)` → `{ok, patches, error}` — routes a host
  event (`tap` / `set`) into the live session and returns the minimal **patch
  stream** (`tswift_swiftui::diff`) between the trees before/after the event.

The session lives in a `thread_local` on the Rust side (wasm is single-threaded);
recompiling replaces it.

The DOM is rendered by the shared, snapshot-tested **`web/swiftui-canvas`**
package — the single source of DOM truth across the project. Its
`<swiftui-canvas>` custom element `mount(tree)`s the initial tree and
`applyPatches(stream)`es each event's diff into a `Map<nodeId, HTMLElement>`,
mutating nodes in place. Because the live element is never torn down, input
focus, an in-flight slider drag, and scroll position survive for free — no
capture/restore hacks. The prototype imports that package's source directly
(like its own `example/`); Vite compiles it.

Build/deploy to Cloudflare Pages:

```bash
npm run build
npm run deploy
```

Test the compiled wasm (builds it, then drives every supported preset + a tap
through the real `.wasm` in Node):

```bash
npm test
```

## Limits / known gaps

- One file only (the whole point of the next step is multi-file). The root `View`
  is auto-detected (the view nobody else constructs).
- Layout is approximate CSS flexbox (owned by `web/swiftui-canvas`), not
  SwiftUI's real layout engine.
- Recompiling leaks the previous interpreter (bounded `Box::leak`, fine for a
  throwaway sandbox).
- Only the v1 SwiftUI surface the runtime supports is rendered; unsupported view
  kinds/modifiers are ignored by the canvas.

## Verdict placeholder

After trying it, record whether the live-preview interaction loop is worth
absorbing into a real product surface (and how multi-file would slot in), then
delete this prototype.
