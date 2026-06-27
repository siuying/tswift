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

`npm run build` compiles `crates/tswift-wasm` with `wasm-pack` into `src/wasm/`, then Astro bundles the static site. There is no server API and no `QSWIFT_RUNNER_URL`: the browser loads `tswift_wasm_bg.wasm` and calls `runSwift(source)` directly.

Test the compiled wasm (builds it, then runs representative programs + every
supported preset through the real `.wasm` in Node):

```bash
npm test
```

This catches wasm-only regressions that native `cargo test` cannot (e.g. a
panic from `SystemTime::now()`, which is unimplemented on wasm32). The crate
logic itself is also covered by native unit tests in `crates/tswift-wasm`.

Variants:

- `?variant=A` — IDE split pane
- `?variant=B` — terminal-first workflow
- `?variant=C` — learning-lab workflow

No state is persisted beyond browser `sessionStorage` for the editor text.

Verdict placeholder: after trying it, record which variant or interaction should be absorbed into a real product surface, then delete this prototype.
