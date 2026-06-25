# qswift web sandbox prototype

Question: can a throwaway Astro app make qswift feel like a tiny CodeSandbox for one Swift file?

Run locally:

```bash
cd prototype/web-sandbox
npm install
npm run dev
```

Open <http://127.0.0.1:4321/>.

Deploy to Cloudflare Pages:

```bash
npm run deploy
```

Cloudflare Workers cannot spawn `cargo`, so `/api/run` needs a deployed runner service. Set `QSWIFT_RUNNER_URL` in Cloudflare Pages when you want remote execution. Without it, the UI deploys but run requests return a setup error.

Variants:

- `?variant=A` — IDE split pane
- `?variant=B` — terminal-first workflow
- `?variant=C` — learning-lab workflow

The app writes the editor content to a temporary `main.swift`, runs `cargo run -q -p qswift-cli -- dump --json` to compile/analyze it, then runs `cargo run -q -p qswift-cli -- run`. No state is persisted beyond browser `sessionStorage` for the editor text.

Verdict placeholder: after trying it, record which variant or interaction should be absorbed into a real product surface, then delete this prototype.
