import { defineConfig } from "@playwright/test";

// Layer D web screenshot harness. Mirrors the iOS snapshot harness
// (ios/UiirRenderer): drive the real <swiftui-canvas> through mount + patch
// replay and screenshot each step. WebKit is used so the engine matches iOS
// Safari/SwiftUI as closely as a browser can (same -apple-system fonts).
//
// Non-gating: this produces side-by-side artifacts for the web↔native
// perceptual diff. See docs/plan/layer-d-web-harness.md.
//
// Device + appearance matrix. Each project pairs a viewport (iPhone/iPad,
// logical-point sizes + scale factors matching the iOS ViewImageConfig presets)
// with a color scheme (light/dark). Playwright appends the project name to each
// screenshot, yielding baselines like `counter-0-initial-iphone-light-darwin.png`
// that line up with the native `counter-0-initial-iphone-light` snapshots.
export default defineConfig({
  testDir: "./tests",
  testMatch: "**/*.spec.ts",
  // Screenshots are font/scale sensitive; never parallelize within a file and
  // keep workers at 1 so the dev server and rendering stay deterministic.
  fullyParallel: false,
  workers: 1,
  reporter: [["list"]],
  use: {
    baseURL: "http://127.0.0.1:4323",
  },
  projects: [
    // iPhone 13: 390×844 @3x.
    {
      name: "iphone-light",
      use: { browserName: "webkit", viewport: { width: 390, height: 844 }, deviceScaleFactor: 3, colorScheme: "light" },
    },
    {
      name: "iphone-dark",
      use: { browserName: "webkit", viewport: { width: 390, height: 844 }, deviceScaleFactor: 3, colorScheme: "dark" },
    },
    // iPad Pro 11": 834×1194 @2x.
    {
      name: "ipad-light",
      use: { browserName: "webkit", viewport: { width: 834, height: 1194 }, deviceScaleFactor: 2, colorScheme: "light" },
    },
    {
      name: "ipad-dark",
      use: { browserName: "webkit", viewport: { width: 834, height: 1194 }, deviceScaleFactor: 2, colorScheme: "dark" },
    },
  ],
  webServer: {
    command: "npx vite tests/harness --host 127.0.0.1 --port 4323 --strictPort",
    url: "http://127.0.0.1:4323",
    reuseExistingServer: !process.env.CI,
    timeout: 60_000,
  },
});
