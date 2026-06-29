import { defineConfig } from 'astro/config';
import mdx from '@astrojs/mdx';
import react from '@astrojs/react';

// Allow Vite to read the sibling `web/swiftui-canvas` package source, imported
// directly into the playground (like that package's own example/). The repo
// root is one level up from this config file.
const repoRoot = new URL('../', import.meta.url).pathname;

export default defineConfig({
  // site + base are injected by CI for GitHub Pages; leave undefined for
  // local dev and Cloudflare Pages (where the base is always "/").
  site: process.env.SITE_URL,
  base: process.env.BASE_PATH,
  integrations: [mdx(), react()],
  vite: {
    // Allow the wasm file served from public/ to be fetched cross-origin in dev
    server: {
      headers: {
        'Cross-Origin-Opener-Policy': 'same-origin',
        'Cross-Origin-Embedder-Policy': 'require-corp',
      },
      fs: { allow: [repoRoot] },
    },
  },
});
