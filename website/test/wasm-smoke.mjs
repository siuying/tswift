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
const { initSync, swiftUICompile, swiftUIDispatch, swiftDiagnostics } = await import(
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

// 4. The text-mode (runSwift) playground presets.
const { runSwift } = await import(new URL('tswift_wasm.js', wasmDir));
const TEXT_PRESETS = {
  'Hello World': `let language = "Swift"\nlet version = 6\nprint("Hello from \\(language) \\(version)! 👋")`,
  'Fibonacci': `func fib(_ n: Int) -> Int {\n  if n < 2 { return n }\n  return fib(n - 1) + fib(n - 2)\n}\nprint(fib(10))`,
  'Closures & HOF': `let nums = [1,2,3,4,5]\nprint(nums.map { $0 * 2 })\nprint(nums.filter { $0 % 2 == 0 })\nprint(nums.reduce(0, +))`,
  'Generics / popLast': [
    'func maxOf<T: Comparable>(_ a: T, _ b: T) -> T { a > b ? a : b }',
    'print(maxOf(3, 7))',
    'struct Stack<Element> {',
    '    private var items: [Element] = []',
    '    mutating func push(_ item: Element) { items.append(item) }',
    '    mutating func pop() -> Element? { items.popLast() }',
    '    var top: Element? { items.last }',
    '}',
    'var s = Stack<Int>()',
    's.push(1); s.push(2); s.push(3)',
    'print(s.top!)',
    'while let x = s.pop() { print(x) }',
  ].join('\n'),
  'Error Handling': [
    'enum E: Error { case bad }',
    'func f(_ x: Int) throws -> Int { guard x > 0 else { throw E.bad }; return x * 2 }',
    'do { print(try f(3)) } catch { print("err") }',
    'do { print(try f(-1)) } catch { print("caught") }',
  ].join('\n'),
};
for (const [label, code] of Object.entries(TEXT_PRESETS)) {
  check(`text preset "${label}" runs ok`, () => {
    const r = JSON.parse(runSwift(code));
    assert(r.run?.ok === true,
      `expected run.ok=true, got ${JSON.stringify({ compile: r.compile?.stderr, stderr: r.run?.stderr })}`);
  });
}

// 5. The SwiftUI playground presets (mirrored from FullPlayground.astro) all
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

// 5. The editor's live error-feedback channel: `swiftDiagnostics` lints without
//    running and reports structured positions/severities (CodeMirror's linter
//    maps these to inline squiggles).
check('swiftDiagnostics is exported and lints clean source', () => {
  assert(typeof swiftDiagnostics === 'function', 'swiftDiagnostics missing');
  const r = JSON.parse(swiftDiagnostics('let x = 1\nprint(x)'));
  assert(r.ok === true, `expected ok=true, got ${JSON.stringify(r)}`);
  assert(Array.isArray(r.diagnostics) && r.diagnostics.length === 0,
    `expected no diagnostics, got ${JSON.stringify(r.diagnostics)}`);
});

check('swiftDiagnostics reports an error with line/col/severity', () => {
  const r = JSON.parse(swiftDiagnostics('#error("boom")'));
  assert(r.ok === false, `expected ok=false, got ${JSON.stringify(r)}`);
  const d = r.diagnostics[0];
  assert(d && d.severity === 'error', `expected an error diagnostic: ${JSON.stringify(r)}`);
  assert(typeof d.line === 'number' && typeof d.col === 'number',
    `expected numeric line/col: ${JSON.stringify(d)}`);
  assert(d.message.includes('boom'), `expected message to carry 'boom': ${JSON.stringify(d)}`);
});

if (failures > 0) {
  console.error(`\n${failures} website swiftui wasm smoke check(s) failed`);
  process.exit(1);
}
console.log('\nall website swiftui wasm smoke checks passed');
