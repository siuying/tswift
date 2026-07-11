// Smoke test for the Web Studio against the *shipped* wasm artifact.
//
// The Studio (`src/components/Studio.astro`) drives multi-file projects through
// `runSwiftModule` / `swiftUICompileModule` / `listSymbols` / `swiftDiagnostics
// Module`. A stale wasm (missing `listSymbols`, added after the last artifact
// rebuild) would silently break the outline and quick-open, so this loads the
// real `.wasm` the site serves and runs every starter sample through the same
// module wire shapes the component builds — proving each sample actually runs
// and that the required exports exist.
//
// Run with: npm test

import { readFileSync } from 'node:fs';
import process from 'node:process';

// A Node localStorage shim (the host-services module needs it), installed
// before importing anything that reads it — mirrors wasm-smoke.mjs.
if (typeof globalThis.localStorage === 'undefined') {
  const store = new Map();
  globalThis.localStorage = {
    getItem: (k) => (store.has(k) ? store.get(k) : null),
    setItem: (k, v) => store.set(k, String(v)),
    removeItem: (k) => store.delete(k),
    clear: () => store.clear(),
  };
}

const { installTSwiftHostServices } = await import(
  new URL('../src/lib/tswift-host-services.js', import.meta.url)
);
installTSwiftHostServices();
// tswift.db (SwiftData) needs the real sqlite-wasm module; if it can't load
// (offline / not installed) the SwiftData console sample degrades to a skip.
const { installTSwiftDbHostService } = await import(
  new URL('../src/lib/tswift-db-host-service.js', import.meta.url)
);
const dbAvailable = await installTSwiftDbHostService().catch(() => false);

const wasmDir = new URL('../public/wasm/', import.meta.url);
const mod = await import(new URL('tswift_wasm.js', wasmDir));
mod.initSync({ module: readFileSync(new URL('tswift_wasm_bg.wasm', wasmDir)) });

const { SAMPLES } = await import(new URL('../src/lib/studio/samples.js', import.meta.url));
const { moduleJson, isSwiftUIProject } = await import(
  new URL('../src/lib/studio/module.js', import.meta.url)
);
const { createProject } = await import(new URL('../src/lib/studio/project.js', import.meta.url));

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

check('wasm exports the module + symbol entry points Studio needs', () => {
  for (const name of ['runSwiftModule', 'swiftUICompileModule', 'swiftUIDispatch', 'listSymbols', 'swiftDiagnosticsModule']) {
    assert(typeof mod[name] === 'function', `${name} missing (stale wasm?)`);
  }
});

for (const sample of SAMPLES) {
  const project = createProject(sample.name, sample.files);
  const json = moduleJson(project);

  if (sample.id === 'swiftdata' && !dbAvailable) {
    console.log(`  skip ${sample.name}: sqlite-wasm (tswift.db) unavailable in this environment`);
    continue;
  }

  check(`sample "${sample.name}" lists symbols`, () => {
    const res = JSON.parse(mod.listSymbols(json));
    assert(res.ok === true, `listSymbols failed: ${res.error}`);
    assert(res.symbols.length > 0, 'expected at least one symbol');
  });

  check(`sample "${sample.name}" has clean diagnostics`, () => {
    const res = JSON.parse(mod.swiftDiagnosticsModule(json));
    const errors = (res.diagnostics || []).filter((d) => d.severity === 'error');
    assert(errors.length === 0, `unexpected errors: ${JSON.stringify(errors)}`);
  });

  if (isSwiftUIProject(project)) {
    check(`sample "${sample.name}" renders a SwiftUI tree`, () => {
      const res = JSON.parse(mod.swiftUICompileModule(json));
      assert(res.ok === true, `compile failed: ${res.error}`);
      assert(res.root, `no root view: ${JSON.stringify(res)}`);
      assert(res.tree && res.tree.kind, 'expected a UIIR tree');
    });
  } else {
    check(`sample "${sample.name}" runs to completion`, () => {
      const res = JSON.parse(mod.runSwiftModule(json));
      assert(res.run && res.run.ok === true, `run failed: ${JSON.stringify(res.run || res.compile)}`);
      assert((res.run.stdout || '').length > 0, 'expected stdout');
    });
  }
}

if (failures > 0) {
  console.error(`\n${failures} studio-wasm check(s) failed`);
  process.exit(1);
}
console.log('\nall studio-wasm checks passed');
