// Ordered catalog of editor presets. Each entry pairs display metadata with the
// `.swift` file in this directory that holds the runnable source. Keeping the
// code in real `.swift` files (instead of escaped template literals) means no
// `\\(` / `\\d` double-escaping, and the files run as-is through the runtime.
//
// This module is plain ESM so it can be imported by both the Astro page (via
// the Vite loader in `index.js`) and the Node `wasm-smoke.mjs` test.
//
// `supported: false` hides a preset from the UI (a feature the runtime cannot
// execute yet) while keeping its source on disk.

export const presetManifest = [
  // ── Basics ──────────────────────────────────────────────────
  { group: 'Basics', label: 'Hello World', file: '01-hello-world.swift' },
  { group: 'Basics', label: 'Fibonacci', file: '02-fibonacci.swift' },
  // ── Functions & Closures ─────────────────────────────────────
  { group: 'Closures', label: 'Closures & HOF', file: '03-closures-hof.swift' },
  // ── Value Types ──────────────────────────────────────────────
  { group: 'Value Types', label: 'Structs', file: '04-structs.swift' },
  { group: 'Value Types', label: 'Enums', file: '05-enums.swift' },
  { group: 'Value Types', label: 'Optionals', file: '06-optionals.swift' },
  // ── Reference Types ──────────────────────────────────────────
  { group: 'Reference Types', label: 'Classes', file: '07-classes.swift' },
  // ── Protocols & Generics ─────────────────────────────────────
  { group: 'Protocols', label: 'Protocols', file: '08-protocols.swift' },
  { group: 'Protocols', label: 'Generics', file: '09-generics.swift' },
  // ── Error Handling ───────────────────────────────────────────
  { group: 'Errors', label: 'Error Handling', file: '10-error-handling.swift' },
  // ── Advanced ─────────────────────────────────────────────────
  { group: 'Advanced', label: 'Property Wrappers', file: '11-property-wrappers.swift', supported: false },
  { group: 'Advanced', label: 'Switch Patterns', file: '12-switch-patterns.swift' },
  // ── Stdlib ───────────────────────────────────────────────────
  { group: 'Stdlib', label: 'Strings', file: '13-strings.swift' },
  { group: 'Stdlib', label: 'Collections', file: '14-collections.swift' },
  { group: 'Stdlib', label: 'Regex', file: '15-regex.swift' },
];
