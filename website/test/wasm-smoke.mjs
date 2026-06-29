// Smoke test for the *shipped* website wasm artifact, SwiftUI entry points.
//
// The production playground (`src/components/FullPlayground.astro`) drives the
// live SwiftUI preview through `swiftUICompile` / `swiftUIDispatch` exported by
// `public/wasm/tswift_wasm.js`. Native `cargo test` can't catch wasm-only
// panics, and a stale `runSwift`-only wasm would silently break the preview, so
// this loads the real `.wasm` the site serves and drives it through Node:
// every SwiftUI preset compiles + renders a UIIR tree, and a counter tap
// mutates @State and returns a minimal in-place patch.
//
// Run with: npm test

import { readFileSync } from 'node:fs';
import process from 'node:process';

const wasmDir = new URL('../public/wasm/', import.meta.url);
const { initSync, swiftUICompile, swiftUIDispatch } = await import(
  new URL('tswift_wasm.js', wasmDir)
);
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

// 1. The SwiftUI host exports exist (a stale runSwift-only wasm would break the
//    preview silently).
check('wasm exports the SwiftUI host entry points', () => {
  assert(typeof swiftUICompile === 'function', 'swiftUICompile missing');
  assert(typeof swiftUIDispatch === 'function', 'swiftUIDispatch missing');
});

// 2. A counter compiles to a full tree, and a button tap returns a minimal
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
  const after = JSON.parse(swiftUIDispatch('0.1', 'tap', ''));
  assert(after.ok === true, `dispatch failed: ${JSON.stringify(after)}`);
  assert(Array.isArray(after.patches), `expected a patches array: ${JSON.stringify(after)}`);
  const setText = after.patches.find((p) => p.op === 'setText' && p.id === '0.0');
  assert(setText && setText.text === '1', `tap did not emit setText=1: ${JSON.stringify(after.patches)}`);
});

// 3. A program with no View is a structured compile error, not a wasm trap —
//    the playground falls back to the text `runSwift` path in this case.
check('missing View returns a structured error (text-mode fallback)', () => {
  const r = JSON.parse(swiftUICompile('let x = 1'));
  assert(r.ok === false, `expected ok=false, got ${JSON.stringify(r)}`);
  assert(r.root == null, `expected root=null, got ${r.root}`);
});

// 4. The SwiftUI playground presets (mirrored from FullPlayground.astro) all
//    compile + render a tree.
const SWIFTUI_PRESETS = {
  Toggle: [
    'struct GreetingView: View {',
    '  @State private var isOn = true',
    '  var body: some View {',
    '    VStack(spacing: 16) {',
    '      Toggle("Show greeting", isOn: $isOn)',
    '      if isOn { Text("Hello!") }',
    '    }',
    '  }',
    '}',
  ].join('\n'),
  List: [
    'struct FruitList: View {',
    '  let fruits = ["Apple", "Banana", "Cherry"]',
    '  var body: some View {',
    '    List {',
    '      ForEach(fruits, id: \\.self) { fruit in',
    '        HStack { Text(fruit); Spacer(); Text("🍎") }',
    '      }',
    '    }',
    '  }',
    '}',
  ].join('\n'),
  Profile: [
    'struct ProfileCard: View {',
    '  var body: some View {',
    '    VStack(spacing: 12) {',
    '      Text("🦜").font(.largeTitle)',
    '      Text("Unlucky Parrot").font(.title).fontWeight(.bold)',
    '      Text("SwiftUI on tswift").foregroundColor(.secondary)',
    '    }.padding()',
    '  }',
    '}',
  ].join('\n'),
};
for (const [label, code] of Object.entries(SWIFTUI_PRESETS)) {
  check(`preset "${label}" renders`, () => {
    const r = JSON.parse(swiftUICompile(code));
    assert(r.ok === true, `expected ok=true, got ${JSON.stringify(r.error || r)}`);
    assert(r.tree && typeof r.tree.kind === 'string', 'expected a UIIR tree with a root kind');
  });
}

if (failures > 0) {
  console.error(`\n${failures} website swiftui wasm smoke check(s) failed`);
  process.exit(1);
}
console.log('\nall website swiftui wasm smoke checks passed');
