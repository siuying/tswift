// Ordered catalog of SwiftUI editor presets. Each entry pairs display metadata
// with the `.swift` file in this directory holding the runnable `View`.
//
// Keeping source in real `.swift` files (not escaped template literals) means
// no `\\(` double-escaping, and the files render as-is through the SwiftUI host.
// This module is plain ESM so both the Astro page (via the Vite loader in
// `index.js`) and the Node `wasm-smoke.mjs` test can import it.
//
// `supported: false` hides a preset from the UI while keeping its source.

export const presetManifest = [
  // ── State ────────────────────────────────────────────────────
  { group: 'State', label: 'Counter', file: '01-counter.swift' },
  { group: 'State', label: 'Observable (MVVM)', file: '02-observable.swift' },
  // ── Controls ─────────────────────────────────────────────────
  { group: 'Controls', label: 'Toggle', file: '03-greeting.swift' },
  { group: 'Controls', label: 'Text Field', file: '04-form.swift' },
  { group: 'Controls', label: 'Slider & Stepper', file: '05-controls.swift' },
  { group: 'Controls', label: 'Picker', file: '06-picker.swift' },
  // ── Collections ──────────────────────────────────────────────
  { group: 'Collections', label: 'ForEach List', file: '07-list.swift' },
  { group: 'Collections', label: 'Sections', file: '08-sections.swift' },
  // ── Layout ───────────────────────────────────────────────────
  { group: 'Layout', label: 'Composition', file: '09-profile.swift' },
  { group: 'Layout', label: 'ZStack & Shapes', file: '10-stack.swift' },
  // ── Environment ──────────────────────────────────────────────
  { group: 'Environment', label: 'EnvironmentObject', file: '11-environment.swift' },
];
