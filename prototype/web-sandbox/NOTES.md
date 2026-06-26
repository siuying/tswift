# qswift web sandbox prototype

Question: can a throwaway Astro app make qswift feel like a tiny CodeSandbox for one Swift file, with compile/run happening entirely in the browser?

Run locally:

```bash
cd prototype/web-sandbox
npm install
npm run dev
```

Open <http://127.0.0.1:4321/>.

Build/deploy to Cloudflare Pages:

```bash
npm run build
npm run deploy
```

`npm run build` compiles `crates/qswift-wasm` with `wasm-pack` into `src/wasm/`, then Astro bundles the static site. There is no server API and no `QSWIFT_RUNNER_URL`: the browser loads `qswift_wasm_bg.wasm` and calls `runSwift(source)` directly.

Variants:

- `?variant=A` — IDE split pane
- `?variant=B` — terminal-first workflow
- `?variant=C` — learning-lab workflow

No state is persisted beyond browser `sessionStorage` for the editor text.

Verdict placeholder: after trying it, record which variant or interaction should be absorbed into a real product surface, then delete this prototype.
