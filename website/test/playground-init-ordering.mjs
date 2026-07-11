// Pins the fix for the `installTSwiftDbHostService()` fire-and-forget race
// in `FullPlayground.astro`/`MiniPlayground.astro`'s `initWasm()`: Swift
// running in the playground could call `tswift.db.*` the instant Run
// becomes clickable, so `dbInstallPromise` (the in-flight `tswift.db`
// capability declaration) must be `await`ed *before* `runBtn.disabled =
// false`, not fired-and-forgotten in parallel with it. There is no DOM/wasm
// harness available in this test tier (see `db-host-service.mjs`'s own
// doc for why the `tswift.db.*` wire itself is tested below the Astro
// component layer), so this pins the fix structurally: both components'
// `initWasm()` source must (a) kick the sqlite install off without an
// `await` of its own promise-returning call in isolation (so it can load in
// parallel with the wasm bundle fetch) and (b) `await` that saved promise
// strictly before the `runBtn.disabled = false` line.

import fs from 'node:fs';
import path from 'node:path';
import process from 'node:process';
import { fileURLToPath } from 'node:url';

let failures = 0;
function check(name, fn) {
  try {
    fn();
    console.log(`  ok   ${name}`);
  } catch (err) {
    failures += 1;
    console.error(`  FAIL ${name}\n       ${err.stack || err.message}`);
  }
}
function assert(cond, msg) {
  if (!cond) throw new Error(msg);
}

const here = path.dirname(fileURLToPath(import.meta.url));
const componentsDir = path.join(here, '..', 'src', 'components');

for (const file of ['FullPlayground.astro', 'MiniPlayground.astro']) {
  const source = fs.readFileSync(path.join(componentsDir, file), 'utf8');

  check(`${file}: initWasm() saves the db install as a promise instead of firing-and-forgetting it`, () => {
    assert(
      /const dbInstallPromise = import\(['"]\.\.\/lib\/tswift-db-host-service\.js['"]\)/.test(source),
      'expected initWasm() to bind the db-install chain to `dbInstallPromise`',
    );
  });

  check(`${file}: initWasm() awaits dbInstallPromise before enabling Run`, () => {
    const awaitIdx = source.indexOf('await dbInstallPromise;');
    // `lastIndexOf`: this file's own doc comment above the fix mentions the
    // literal text `runBtn.disabled = false` inside backticks before the
    // real statement runs — anchor on the actual assignment, not the prose.
    const enableIdx = source.lastIndexOf('runBtn.disabled = false;');
    assert(awaitIdx !== -1, '`await dbInstallPromise` not found');
    assert(enableIdx !== -1, '`runBtn.disabled = false` not found');
    assert(
      awaitIdx < enableIdx,
      `expected \`await dbInstallPromise\` (at ${awaitIdx}) to appear before \`runBtn.disabled = false\` (at ${enableIdx})`,
    );
  });

  check(`${file}: a failed db install is swallowed to \`false\`, never left unhandled`, () => {
    assert(
      /\.then\(\(m\) => m\.installTSwiftDbHostService\(\)\)\s*\n\s*\.catch\(\(\) => false\)/.test(source),
      'expected the db-install chain to end in `.catch(() => false)`',
    );
  });
}

if (failures > 0) {
  console.error(`\n${failures} playground-init-ordering check(s) failed`);
  process.exit(1);
}
console.log('\nall playground-init-ordering checks passed');
