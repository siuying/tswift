// Build the editor presets for the Astro page by joining the ordered
// `manifest.mjs` metadata with the `.swift` source loaded at build time.
//
// `import.meta.glob` is a Vite feature: it inlines every `./*.swift` file as a
// raw string at build, so the browser bundle ships the source with no runtime
// fetch. The Node smoke test cannot use Vite, so it reads the same files via
// `manifest.mjs` + `fs` instead (see `test/wasm-smoke.mjs`).

import { presetManifest } from './manifest.mjs';

const sources = import.meta.glob('./*.swift', {
  query: '?raw',
  import: 'default',
  eager: true,
});

export const presets = presetManifest.map(({ group, label, file, supported }) => {
  const raw = sources[`./${file}`];
  if (raw === undefined) {
    throw new Error(`preset source not found: ${file}`);
  }
  return {
    group,
    label,
    ...(supported === false ? { supported: false } : {}),
    code: raw.replace(/\n$/, ''),
  };
});
