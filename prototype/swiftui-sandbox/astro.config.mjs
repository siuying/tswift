import { defineConfig } from 'astro/config';

// Allow Vite's dev server to read the sibling `web/swiftui-canvas` package
// source (imported directly, like that package's own example/). The repo root
// is two levels up from this config file.
const repoRoot = new URL('../../', import.meta.url).pathname;

export default defineConfig({
  vite: {
    server: { fs: { allow: [repoRoot] } },
  },
});
