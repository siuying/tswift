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

  if ((sample.id === 'swiftdata' || sample.id === 'swiftdata-swiftui') && !dbAvailable) {
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

// Regression: a SwiftUI view backed by SwiftData (`@Query` +
// `.modelContainer(for:)`) must render + mutate through the `swiftUICompile`
// session, not just `runSwiftModule`. Previously the swiftui compile path
// never installed the host-call handler before `tswift_swiftdata::install`, so
// the `tswift.db.*` signatures failed to register and `.modelContainer(for:)`
// threw "SwiftData is unavailable" even with `tswift.db` backed.
if (!dbAvailable) {
  console.log('  skip SwiftData SwiftUI render: sqlite-wasm (tswift.db) unavailable');
} else {
  const SWIFTDATA_VIEW = [
    'import SwiftData',
    'import SwiftUI',
    '',
    '@Model',
    'class Note {',
    '  var title: String',
    '  init(title: String) { self.title = title }',
    '}',
    '',
    'struct NoteList: View {',
    '  @Query(sort: \\.title) var tasks: [Note]',
    '  var body: some View {',
    '    VStack {',
    '      Button("add") {',
    '        if let ctx = try? __tswiftCurrentModelContext() {',
    '          ctx.insert(Note(title: "row-\\(tasks.count + 1)"))',
    '          try? ctx.save()',
    '        }',
    '      }',
    '      List {',
    '        ForEach(tasks) { task in',
    '          Text(task.title)',
    '        }',
    '      }',
    '    }',
    '  }',
    '}',
    '',
    'struct RootView: View {',
    '  var body: some View {',
    '    NoteList()',
    '      .modelContainer(for: Note.self, inMemory: true)',
    '  }',
    '}',
  ].join('\n');

  check('SwiftData-backed SwiftUI view renders through swiftUICompile', () => {
    const r = JSON.parse(mod.swiftUICompile(SWIFTDATA_VIEW));
    assert(r.ok === true, `compile failed: ${r.error}`);
    assert(r.root === 'RootView', `bad root: ${JSON.stringify(r)}`);
    assert(r.tree && r.tree.kind, 'expected a UIIR tree');
  });

  check('SwiftData dispatch inserts+saves and re-renders a new row', () => {
    // The "add" button is child 0.0; tapping it inserts+saves a Note, and the
    // re-render (body re-evaluates every dispatch) must surface an inserted
    // Text patch for the new row.
    const after = JSON.parse(mod.swiftUIDispatch('0.0', 'tap', ''));
    assert(after.ok === true, `dispatch failed: ${after.error}`);
    assert(Array.isArray(after.patches), `expected patches: ${JSON.stringify(after)}`);
    const inserted = after.patches.find(
      (p) => p.op === 'insert' && p.node && p.node.args && p.node.args.verbatim === 'row-1',
    );
    assert(inserted, `tap did not insert row-1: ${JSON.stringify(after.patches)}`);
  });
}

if (failures > 0) {
  console.error(`\n${failures} studio-wasm check(s) failed`);
  process.exit(1);
}
console.log('\nall studio-wasm checks passed');
