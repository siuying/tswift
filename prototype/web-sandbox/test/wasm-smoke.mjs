// Smoke test for the *real* compiled wasm artifact.
//
// Native `cargo test` cannot catch wasm-only panics (e.g. `SystemTime::now()`
// is unimplemented on wasm32 and aborts to `RuntimeError: unreachable`), because
// the host has a working clock. This loads `src/wasm/qswift_wasm_bg.wasm` and
// runs it through Node, asserting representative programs actually execute.
//
// Run with: npm test   (which builds the wasm first)

import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import process from 'node:process';

const wasmDir = new URL('../src/wasm/', import.meta.url);
const { initSync, runSwift } = await import(new URL('qswift_wasm.js', wasmDir));
initSync({ module: readFileSync(new URL('qswift_wasm_bg.wasm', wasmDir)) });

let failures = 0;
function check(name, fn) {
  try {
    fn();
    console.log(`  ok   ${name}`);
  } catch (err) {
    failures += 1;
    console.error(`  FAIL ${name}\n       ${err.message}`);
  }
}
function assert(cond, msg) {
  if (!cond) throw new Error(msg);
}

// 1. Hello world must run — the direct regression for the RNG/SystemTime panic.
check('hello world runs without panic', () => {
  const r = JSON.parse(runSwift('let who = "Swift"\nprint("Hello \\(who)!")'));
  assert(r.ok === true, `expected ok=true, got ${JSON.stringify(r)}`);
  assert(r.run && r.run.stdout === 'Hello Swift!\n', `bad stdout: ${JSON.stringify(r.run)}`);
});

// 2. A program that consumes the RNG must not panic either (the RNG is seeded
//    in Interpreter::new(), the exact path that aborted on wasm).
check('rng-backed api does not panic', () => {
  const r = JSON.parse(runSwift('print([1, 2, 3].shuffled().count)'));
  assert(r.ok === true, `expected ok=true, got ${JSON.stringify(r)}`);
  assert(r.run.stdout.trim() === '3', `bad stdout: ${JSON.stringify(r.run)}`);
});

// 3. Runtime error is reported as JSON, not a thrown wasm trap.
check('runtime error returns structured failure', () => {
  const r = JSON.parse(runSwift('let a = [1, 2]\nprint(a[9])'));
  assert(r.ok === false, `expected ok=false, got ${JSON.stringify(r)}`);
  assert(r.compile.ok === true, 'expected compile.ok=true');
  assert(r.run && r.run.ok === false, 'expected run.ok=false');
});

// 4. Each supported preset in the page must run cleanly.
const indexSrc = readFileSync(fileURLToPath(new URL('../src/pages/index.astro', import.meta.url)), 'utf8');
const presetsMatch = indexSrc.match(/const presets = \[([\s\S]*?)\n\];/);
assert(presetsMatch, 'could not locate presets array in index.astro');
// eslint-disable-next-line no-eval
const presets = eval(`[${presetsMatch[1]}\n]`);
const supported = presets.filter((p) => p.supported !== false);
assert(supported.length > 0, 'no supported presets found');
for (const p of supported) {
  check(`preset "${p.label}" runs`, () => {
    const r = JSON.parse(runSwift(p.code));
    assert(r.ok === true, `expected ok=true, got ${JSON.stringify(r.run || r.compile)}`);
  });
}

if (failures > 0) {
  console.error(`\n${failures} wasm smoke check(s) failed`);
  process.exit(1);
}
console.log('\nall wasm smoke checks passed');
