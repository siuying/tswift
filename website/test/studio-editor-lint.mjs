// Pins the fix for the stale-CM6-lint-refresh review issue: after a debounced
// wasm `swiftDiagnosticsModule` analyze() resolves, Studio.astro must push the
// new diagnostics into CodeMirror's lint state, not rely on a pull-based
// `linter()` source re-running on its own timer/doc-change triggers (which
// leaves inline squiggles/gutter markers stale once the user stops typing).
//
// `@codemirror/lint`'s `EditorView` needs a real DOM (`document.createElement`
// with layout, `getClientRects`, etc.) that isn't available in this project's
// offline Node test tier (no jsdom in node_modules/lockfile — see
// `playground-init-ordering.mjs`'s doc comment for the same constraint on a
// sibling Astro component). So, matching that file's precedent, this pins the
// fix structurally by asserting the actual source shape rather than mounting
// a live editor.

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
const editorSrc = fs.readFileSync(path.join(here, '..', 'src', 'lib', 'studio', 'editor.js'), 'utf8');
const studioSrc = fs.readFileSync(path.join(here, '..', 'src', 'components', 'Studio.astro'), 'utf8');

check('editor.js imports setDiagnostics (push), not a pull-based linter() source', () => {
  assert(
    /import\s*\{[^}]*setDiagnostics as setLintDiagnostics[^}]*\}\s*from\s*'@codemirror\/lint'/.test(editorSrc),
    'expected editor.js to import `setDiagnostics` from @codemirror/lint',
  );
  assert(
    !/\blinter\(\s*\n?\s*\(view\)/.test(editorSrc),
    'expected the old pull-based `linter((view) => ...)` source to be removed',
  );
});

check('editor.js exposes refreshDiagnostics() that dispatches a setDiagnostics effect', () => {
  assert(/refreshDiagnostics\s*\(\s*\)\s*\{/.test(editorSrc), 'expected a `refreshDiagnostics()` method');
  assert(
    /view\.dispatch\(setLintDiagnostics\(view\.state,/.test(editorSrc),
    'expected refreshDiagnostics (via applyDiagnostics) to dispatch a setDiagnostics effect',
  );
});

check('editor.js re-pushes diagnostics when a file is opened, not just on refreshDiagnostics()', () => {
  const openIdx = editorSrc.indexOf('open(path, source) {');
  assert(openIdx !== -1, 'expected an open(path, source) method');
  const openBody = editorSrc.slice(openIdx, editorSrc.indexOf('\n    },', openIdx));
  assert(
    /applyDiagnostics\(\);/.test(openBody),
    'expected open() to call applyDiagnostics() so switching tabs shows that file\'s diagnostics immediately',
  );
});

check('Studio.astro analyze() refreshes CM6 diagnostics via the editor, not a no-op dispatch({})', () => {
  assert(
    !/editor\.view\.dispatch\(\{\}\)/.test(studioSrc),
    'expected the no-op `editor.view.dispatch({})` linter nudge to be removed',
  );
  assert(
    /editor\.refreshDiagnostics\(\);/.test(studioSrc),
    'expected analyze() to call editor.refreshDiagnostics()',
  );
});

console.log(failures === 0 ? '\nall studio-editor-lint checks passed' : `\n${failures} check(s) failed`);
process.exit(failures === 0 ? 0 : 1);
