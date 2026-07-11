// Derives Language-tier coverage counts from the checked-in feature
// checklist at build time, instead of hand-typed literals scattered across
// pages (see docs/agents/../notes for why: those literals drifted out of
// sync with each other and with the checklist itself).
//
// Source of truth: docs/swift-runtime/feature-checklist.md — the same
// hand-maintained checklist the Rust-frontend/runtime team already updates
// as language features land (`[x]` done, `[~]` partial, `[ ]` todo). This
// module only reads and counts it; it never writes to it.
//
// There is no member-level extraction pipeline for the *language* surface
// (unlike stdlib/foundation/swiftui, which come from `tools/framework-
// inventory` reading Apple SDK `.swiftinterface` files) — the checklist
// itself is the closest thing to a ground truth, so we parse it directly
// rather than duplicating its counts by hand a second time.
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';

// NOTE: resolved from `process.cwd()`, not `import.meta.url` — Vite/Astro's
// production build inlines this module into a bundled chunk under `dist/`,
// which breaks any `import.meta.url`-relative path at build time (the chunk
// no longer lives next to `src/lib/`). `npm run build`/`dev` are always
// invoked with cwd = `website/` (see `website/package.json` scripts), so
// `website/../docs/...` is stable across dev and prerendered build alike.
const CHECKLIST_PATH = resolve(process.cwd(), '../docs/swift-runtime/feature-checklist.md');

export interface Totals {
  implemented: number;
  partial: number;
  missing: number;
  out_of_scope: number;
  total: number;
}

export interface LanguageTier {
  tier: number;
  heading: string;
  totals: Totals;
}

function emptyTotals(): Totals {
  return { implemented: 0, partial: 0, missing: 0, out_of_scope: 0, total: 0 };
}

/** Parse every `## Tier N — ...` section's checklist rows into per-tier totals. */
export function parseLanguageTiers(): LanguageTier[] {
  const text = readFileSync(CHECKLIST_PATH, 'utf-8');
  const lines = text.split('\n');
  const tiers: LanguageTier[] = [];
  let current: LanguageTier | null = null;

  for (const line of lines) {
    const heading = line.match(/^## Tier (\d+) — (.+)$/);
    if (heading) {
      current = { tier: Number(heading[1]), heading: heading[2], totals: emptyTotals() };
      tiers.push(current);
      continue;
    }
    if (!current) continue;
    if (line.startsWith('| [x] |')) {
      current.totals.implemented++;
      current.totals.total++;
    } else if (line.startsWith('| [~] |')) {
      current.totals.partial++;
      current.totals.total++;
    } else if (line.startsWith('| [ ] |')) {
      current.totals.total++;
    }
  }
  for (const t of tiers) {
    t.totals.missing = t.totals.total - t.totals.implemented - t.totals.partial;
  }
  return tiers;
}

/** Sum totals across an inclusive tier range (e.g. 0..9 for the language surface). */
export function sumTiers(tiers: LanguageTier[], fromTier: number, toTier: number): Totals {
  const totals = emptyTotals();
  for (const t of tiers) {
    if (t.tier < fromTier || t.tier > toTier) continue;
    totals.implemented += t.totals.implemented;
    totals.partial += t.totals.partial;
    totals.missing += t.totals.missing;
    totals.total += t.totals.total;
  }
  return totals;
}
