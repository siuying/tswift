---
name: update-website
description: Keeps the tswift project website in sync with runtime implementation changes — coverage numbers, feature tables, WASM binary, and content. Use when features are implemented, coverage changes, the runtime is updated, or the user asks to update/sync/refresh the website.
---

# Update Website

The website lives in `website/`. Content is in MDX — easy to edit. See [FILE-MAP.md](FILE-MAP.md) for the full map.

## Workflow

### 1. Identify what changed

- **Language feature** (Tiers 0–9) → update `website/src/pages/status/language.mdx`
- **Stdlib API** → update `website/src/pages/status/stdlib.mdx`
- **Foundation API** → update `website/src/pages/status/foundation.mdx`
- **Runtime WASM** → copy new wasm files (see step 4)
- **Architecture / docs** → update the relevant `how-it-works/*.mdx`

### 2. Update feature tables

In each status MDX file, change the status column:

| Symbol | Meaning    |
| ------ | ---------- |
| `✅`   | Fully done |
| `🟡`   | Partial    |
| `⬜`   | Todo       |

### 3. Update coverage bar numbers

After updating the tables, **recount** `done`/`total` and update every `<CoverageBar>` that was affected. The bars appear in:

- The changed section's MDX file (e.g. `<CoverageBar label="Tier 0 — Lexical" done={N} total={M} />`)
- `website/src/pages/status/index.astro` — the dashboard (update the matching bar **and** the `.pct-big` number in the status card)

To count: grep the MDX file for `| ✅` (done) and `| 🟡` (partial counts as done for the bar), total is the row count.

### 4. Update WASM (if runtime changed)

```sh
wasm-pack build crates/tswift-wasm --target web \
  --out-dir website/public/wasm --out-name tswift_wasm
```

### 5. Build and verify

```sh
cd website
npm run build        # must complete with 0 errors, 11 pages
```

If the build fails, check for `{#anchor}` syntax in MDX headings — replace with plain headings (MDX parses `{...}` as JSX).

### 6. Commit

```sh
git add website/
git commit -m "docs(website): update coverage for <what changed>"
```

## Notes

- Never add `{#anchor-id}` to MDX headings — the MDX JSX parser rejects them.
- WASM dynamic import uses `/* @vite-ignore */` — don't remove that comment.
- Dev server: `cd website && npm run dev` (hot-reloads MDX changes instantly).
