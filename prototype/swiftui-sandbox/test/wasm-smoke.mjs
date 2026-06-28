// Smoke test for the *real* compiled wasm artifact, SwiftUI entry points.
//
// Native `cargo test` cannot catch wasm-only panics, because the host has a
// working clock/RNG. This loads `src/wasm/tswift_wasm_bg.wasm` and drives the
// SwiftUI render host through Node: every supported preset must compile + render
// to a UIIR tree, and a counter tap must mutate @State and re-render.
//
// Run with: npm test   (which builds the wasm first)

import { readFileSync } from 'node:fs';
import process from 'node:process';

const wasmDir = new URL('../src/wasm/', import.meta.url);
const { initSync, swiftUICompile, swiftUIDispatch } = await import(new URL('tswift_wasm.js', wasmDir));
initSync({ module: readFileSync(new URL('tswift_wasm_bg.wasm', wasmDir)) });

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

// 1. A counter compiles to a full tree, and a button tap returns a minimal
//    in-place patch stream (the wire format `<swiftui-canvas>` applies).
check('counter renders and a tap returns a setText patch', () => {
  const src = [
    'struct CounterView: View {',
    '  @State private var count = 0',
    '  var body: some View {',
    '    VStack {',
    '      Text("\\(count)")',
    '      Button("Increment") { count += 1 }',
    '    }',
    '  }',
    '}',
  ].join('\n');
  const r = JSON.parse(swiftUICompile(src));
  assert(r.ok === true, `expected ok=true, got ${JSON.stringify(r)}`);
  assert(r.root === 'CounterView', `bad root: ${r.root}`);
  assert(r.tree.kind === 'VStack', `bad root kind: ${r.tree.kind}`);
  // Button is the second child: id "0.1". Only the counter Text (id "0.0")
  // changes, so the diff is a single in-place setText — not a re-mount.
  const after = JSON.parse(swiftUIDispatch('0.1', 'tap', ''));
  assert(after.ok === true, `dispatch failed: ${JSON.stringify(after)}`);
  assert(Array.isArray(after.patches), `expected a patches array: ${JSON.stringify(after)}`);
  const setText = after.patches.find((p) => p.op === 'setText' && p.id === '0.0');
  assert(setText && setText.text === '1', `tap did not emit setText=1: ${JSON.stringify(after.patches)}`);
});

// 2. A program with no View is a structured compile error, not a wasm trap.
check('missing View returns a structured error', () => {
  const r = JSON.parse(swiftUICompile('let x = 1'));
  assert(r.ok === false, `expected ok=false, got ${JSON.stringify(r)}`);
  assert(typeof r.error === 'string' && r.error.length > 0, 'expected an error string');
});

// 3. Every supported preset compiles + renders a tree.
const { presetManifest } = await import(new URL('../src/presets/manifest.mjs', import.meta.url));
const presets = presetManifest.map((p) => ({
  label: p.label,
  supported: p.supported,
  code: readFileSync(new URL(`../src/presets/${p.file}`, import.meta.url), 'utf8').replace(/\n$/, ''),
}));
const supported = presets.filter((p) => p.supported !== false);
assert(supported.length > 0, 'no supported presets found');
for (const p of supported) {
  check(`preset "${p.label}" renders`, () => {
    const r = JSON.parse(swiftUICompile(p.code));
    assert(r.ok === true, `expected ok=true, got ${JSON.stringify(r.error || r)}`);
    assert(r.tree && typeof r.tree.kind === 'string', 'expected a UIIR tree with a root kind');
  });
}

if (failures > 0) {
  console.error(`\n${failures} swiftui wasm smoke check(s) failed`);
  process.exit(1);
}
console.log('\nall swiftui wasm smoke checks passed');
