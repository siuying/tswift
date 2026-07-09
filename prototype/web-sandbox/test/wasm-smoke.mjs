// Smoke test for the *real* compiled wasm artifact.
//
// Native `cargo test` cannot catch wasm-only panics (e.g. `SystemTime::now()`
// is unimplemented on wasm32 and aborts to `RuntimeError: unreachable`), because
// the host has a working clock. This loads `src/wasm/qswift_wasm_bg.wasm` and
// runs it through Node, asserting representative programs actually execute.
//
// Run with: npm test   (which builds the wasm first)

import { readFileSync } from 'node:fs';
import process from 'node:process';

const wasmDir = new URL('../src/wasm/', import.meta.url);
const { initSync, runSwift, registerHostFunction, clearHostFunctions } = await import(new URL('qswift_wasm.js', wasmDir));
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
// Presets live as real `.swift` files under src/presets/, ordered by
// manifest.mjs. The Astro page loads them via Vite's `?raw` glob; here in Node
// we read the same files straight off disk so the test exercises the exact
// source the page ships.
const { presetManifest } = await import(new URL('../src/presets/manifest.mjs', import.meta.url));
const presets = presetManifest.map((p) => ({
  group: p.group,
  label: p.label,
  supported: p.supported,
  code: readFileSync(new URL(`../src/presets/${p.file}`, import.meta.url), 'utf8').replace(/\n$/, ''),
}));
const supported = presets.filter((p) => p.supported !== false);
assert(supported.length > 0, 'no supported presets found');
for (const p of supported) {
  check(`preset "${p.label}" runs`, () => {
    const r = JSON.parse(runSwift(p.code));
    assert(r.ok === true, `expected ok=true, got ${JSON.stringify(r.run || r.compile)}`);
  });
}

// 5. tswiftHost hook — host-native function bridge (issue #249).
//
// registerHostFunction() wires a schema; globalThis.tswiftHost is the
// synchronous dispatch hook.  Prior art: tswiftHttp checks above.

// 5a. A registered function backed by a live hook returns its result.
check('tswiftHost: registered function returns result', () => {
  globalThis.tswiftHost = (name, argsJson) => {
    const args = JSON.parse(argsJson);
    if (name === 'add') return JSON.stringify(args[0] + args[1]);
    throw new Error(`unknown: ${name}`);
  };
  const reg = JSON.parse(
    registerHostFunction(
      JSON.stringify({ name: 'add', params: [{ type: 'Int' }, { type: 'Int' }], returns: 'Int' }),
    ),
  );
  assert(reg.ok === true, `registration failed: ${JSON.stringify(reg)}`);
  const r = JSON.parse(runSwift('print(add(3, 4))'));
  clearHostFunctions();
  delete globalThis.tswiftHost;
  assert(r.ok === true, `expected ok=true, got ${JSON.stringify(r)}`);
  assert(r.run && r.run.stdout.trim() === '7', `bad stdout: ${JSON.stringify(r.run)}`);
});

// 5b. Absent hook surfaces as a runtime error, not a wasm trap.
check('tswiftHost: absent hook is a runtime error', () => {
  delete globalThis.tswiftHost;
  const reg = JSON.parse(
    registerHostFunction(JSON.stringify({ name: 'ping', returns: 'String' })),
  );
  assert(reg.ok === true, `registration failed: ${JSON.stringify(reg)}`);
  const r = JSON.parse(runSwift('let x = ping()
print(x)'));
  clearHostFunctions();
  assert(r.ok === false, `expected ok=false, got ${JSON.stringify(r)}`);
  assert(r.compile && r.compile.ok === true, 'expected compile.ok=true');
  assert(r.run && r.run.ok === false, 'expected run.ok=false');
  assert(
    r.run.stderr.includes('not available'),
    `bad stderr: ${r.run.stderr}`,
  );
});

// 5c. A thrown JS exception becomes a runtime error, not a wasm trap.
check('tswiftHost: thrown JS exception is a runtime error', () => {
  globalThis.tswiftHost = (_name, _argsJson) => {
    throw new Error('boom from js');
  };
  const reg = JSON.parse(
    registerHostFunction(JSON.stringify({ name: 'bang', returns: 'Void' })),
  );
  assert(reg.ok === true, `registration failed: ${JSON.stringify(reg)}`);
  const r = JSON.parse(runSwift('bang()'));
  clearHostFunctions();
  delete globalThis.tswiftHost;
  assert(r.ok === false, `expected ok=false, got ${JSON.stringify(r)}`);
  assert(r.run && r.run.ok === false, 'expected run.ok=false');
  assert(r.run.stderr.includes('boom from js'), `bad stderr: ${r.run.stderr}`);
});

// 5d. Malformed schema is rejected by registerHostFunction.
check('tswiftHost: invalid schema returns error', () => {
  const reg = JSON.parse(registerHostFunction(JSON.stringify({ returns: 'Void' }))); // missing name
  assert(reg.ok === false, `expected ok=false, got ${JSON.stringify(reg)}`);
  assert(reg.error && reg.error.length > 0, `expected non-empty error, got ${JSON.stringify(reg)}`);
});

// 5e. String-result host function round-trips through the bridge.
check('tswiftHost: string return value round-trips', () => {
  globalThis.tswiftHost = (name, _argsJson) => {
    if (name === 'greeting') return JSON.stringify('Hello from host');
    return 'null';
  };
  const reg = JSON.parse(
    registerHostFunction(JSON.stringify({ name: 'greeting', returns: 'String' })),
  );
  assert(reg.ok === true, `registration failed: ${JSON.stringify(reg)}`);
  const r = JSON.parse(runSwift('print(greeting())'));
  clearHostFunctions();
  delete globalThis.tswiftHost;
  assert(r.ok === true, `expected ok=true, got ${JSON.stringify(r)}`);
  assert(
    r.run && r.run.stdout.trim() === 'Hello from host',
    `bad stdout: ${JSON.stringify(r.run)}`,
  );
});

if (failures > 0) {
  console.error(`
${failures} wasm smoke check(s) failed`);
  process.exit(1);
}
console.log('
all wasm smoke checks passed');

