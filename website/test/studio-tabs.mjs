// Unit tests for the Web Studio's explicit open/close tab state
// (`src/lib/studio/tabs.js`). Pure data transforms — no DOM, no wasm —
// tested the same lightweight way as `studio.mjs`. The bottom section pins
// how `Studio.astro` wires that pure state to the DOM (boot-time restore of
// a persisted empty strip, and hiding the editor accessibly when the last
// tab closes) the same structural way `studio-sidebar-mode.mjs` and
// `studio-editor-lint.mjs` do — no jsdom in this project's offline test tier.

import fs from 'node:fs';
import path from 'node:path';
import process from 'node:process';
import { fileURLToPath } from 'node:url';
import {
  emptyTabs,
  openTab,
  closeTab,
  renameTab,
  sanitizeTabs,
  isValidTabsShape,
  serializeTabs,
  deserializeTabs,
  TABS_STORAGE_VERSION,
} from '../src/lib/studio/tabs.js';

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
function assertEqual(actual, expected, msg) {
  const a = JSON.stringify(actual);
  const e = JSON.stringify(expected);
  if (a !== e) throw new Error(`${msg}: expected ${e}, got ${a}`);
}

// ── openTab ──────────────────────────────────────────────────────────────

check('openTab on an empty strip opens and activates the file', () => {
  const t = openTab(emptyTabs(), 'main.swift');
  assertEqual(t, { openPaths: ['main.swift'], activePath: 'main.swift' }, 'first open');
});

check('openTab appends a new tab at the end and activates it', () => {
  let t = openTab(emptyTabs(), 'A.swift');
  t = openTab(t, 'B.swift');
  assertEqual(t, { openPaths: ['A.swift', 'B.swift'], activePath: 'B.swift' }, 'appended');
});

check('openTab on an already-open file just activates it (no duplicate, no reorder)', () => {
  let t = openTab(emptyTabs(), 'A.swift');
  t = openTab(t, 'B.swift');
  t = openTab(t, 'C.swift');
  t = openTab(t, 'A.swift');
  assertEqual(t, { openPaths: ['A.swift', 'B.swift', 'C.swift'], activePath: 'A.swift' }, 'reactivated in place');
});

// ── closeTab ─────────────────────────────────────────────────────────────

check('closeTab on the only open (active) tab reaches the honest empty state', () => {
  const t = closeTab(openTab(emptyTabs(), 'A.swift'), 'A.swift');
  assertEqual(t, { openPaths: [], activePath: null }, 'empty state');
});

check('closeTab on the active middle tab activates its right neighbor', () => {
  let t = openTab(openTab(openTab(emptyTabs(), 'A.swift'), 'B.swift'), 'C.swift');
  t = closeTab(t, 'B.swift');
  assertEqual(t, { openPaths: ['A.swift', 'C.swift'], activePath: 'C.swift' }, 'right neighbor');
});

check('closeTab on the active last tab activates its left neighbor', () => {
  let t = openTab(openTab(openTab(emptyTabs(), 'A.swift'), 'B.swift'), 'C.swift');
  t = openTab(t, 'C.swift'); // ensure C active
  t = closeTab(t, 'C.swift');
  assertEqual(t, { openPaths: ['A.swift', 'B.swift'], activePath: 'B.swift' }, 'left neighbor');
});

check('closeTab on an inactive tab leaves the active tab untouched', () => {
  let t = openTab(openTab(openTab(emptyTabs(), 'A.swift'), 'B.swift'), 'C.swift');
  t = openTab(t, 'A.swift'); // A active
  t = closeTab(t, 'C.swift');
  assertEqual(t, { openPaths: ['A.swift', 'B.swift'], activePath: 'A.swift' }, 'active unchanged');
});

check('closeTab on a path that is not open is a no-op', () => {
  const start = openTab(emptyTabs(), 'A.swift');
  const t = closeTab(start, 'Z.swift');
  assertEqual(t, start, 'unchanged');
});

// ── renameTab ────────────────────────────────────────────────────────────

check('renameTab updates the open path and the active path if it matches, preserving order', () => {
  let t = openTab(openTab(emptyTabs(), 'A.swift'), 'B.swift');
  t = renameTab(t, 'A.swift', 'A2.swift');
  assertEqual(t, { openPaths: ['A2.swift', 'B.swift'], activePath: 'B.swift' }, 'path renamed, order kept');
});

check('renameTab updates activePath when the renamed tab was active', () => {
  let t = openTab(openTab(emptyTabs(), 'A.swift'), 'B.swift');
  t = openTab(t, 'A.swift');
  t = renameTab(t, 'A.swift', 'A2.swift');
  assertEqual(t, { openPaths: ['A2.swift', 'B.swift'], activePath: 'A2.swift' }, 'active follows rename');
});

check('renameTab is a no-op for a path that is not open', () => {
  const start = openTab(emptyTabs(), 'A.swift');
  assertEqual(renameTab(start, 'Z.swift', 'Z2.swift'), start, 'unchanged');
});

// ── sanitizeTabs ─────────────────────────────────────────────────────────

check('sanitizeTabs drops open tabs for paths that no longer exist', () => {
  const t = { openPaths: ['A.swift', 'Deleted.swift', 'B.swift'], activePath: 'B.swift' };
  const s = sanitizeTabs(t, ['A.swift', 'B.swift']);
  assertEqual(s, { openPaths: ['A.swift', 'B.swift'], activePath: 'B.swift' }, 'deleted path dropped');
});

check('sanitizeTabs falls back activePath to the first remaining open tab if the active one was dropped', () => {
  const t = { openPaths: ['A.swift', 'Deleted.swift'], activePath: 'Deleted.swift' };
  const s = sanitizeTabs(t, ['A.swift']);
  assertEqual(s, { openPaths: ['A.swift'], activePath: 'A.swift' }, 'falls back to remaining tab');
});

check('sanitizeTabs on an all-invalid strip reaches the honest empty state, never a dangling id', () => {
  const t = { openPaths: ['Gone1.swift', 'Gone2.swift'], activePath: 'Gone1.swift' };
  const s = sanitizeTabs(t, ['A.swift']);
  assertEqual(s, { openPaths: [], activePath: null }, 'fully sanitized to empty');
});

check('sanitizeTabs deduplicates repeated open paths, keeping the first occurrence', () => {
  const t = { openPaths: ['A.swift', 'B.swift', 'A.swift'], activePath: 'A.swift' };
  const s = sanitizeTabs(t, ['A.swift', 'B.swift']);
  assertEqual(s, { openPaths: ['A.swift', 'B.swift'], activePath: 'A.swift' }, 'duplicate dropped, order kept');
});

check('sanitizeTabs tolerates garbage input (non-array openPaths, non-string entries, missing activePath)', () => {
  assertEqual(sanitizeTabs({ openPaths: 'not-an-array' }, ['A.swift']), { openPaths: [], activePath: null });
  assertEqual(sanitizeTabs({ openPaths: [1, null, 'A.swift'] }, ['A.swift']), {
    openPaths: ['A.swift'],
    activePath: 'A.swift',
  });
  assertEqual(sanitizeTabs(null, ['A.swift']), { openPaths: [], activePath: null });
});

// ── serialize / deserialize round-trip ──────────────────────────────────

check('serializeTabs + deserializeTabs round-trips a valid strip', () => {
  const t = { openPaths: ['A.swift', 'B.swift'], activePath: 'B.swift' };
  const restored = deserializeTabs(serializeTabs(t), ['A.swift', 'B.swift']);
  assertEqual(restored, t, 'round trip');
});

check('deserializeTabs sanitizes stale/deleted ids from a persisted strip rather than trusting them', () => {
  const t = { openPaths: ['A.swift', 'Deleted.swift'], activePath: 'Deleted.swift' };
  const restored = deserializeTabs(serializeTabs(t), ['A.swift']);
  assertEqual(restored, { openPaths: ['A.swift'], activePath: 'A.swift' }, 'stale id dropped on restore');
});

check('serializeTabs + deserializeTabs round-trips an explicit empty (all-tabs-closed) strip as itself, not as "nothing persisted"', () => {
  // A reload must be able to tell "the user closed their last tab and that
  // was saved" (a real, non-null Tabs object with an empty openPaths) apart
  // from "nothing was ever persisted" (deserializeTabs returns `null`) —
  // callers fall back to opening a default file only in the latter case.
  const restored = deserializeTabs(serializeTabs(emptyTabs()), ['A.swift', 'B.swift']);
  assert(restored !== null, 'a persisted empty strip must not be indistinguishable from "nothing persisted"');
  assertEqual(restored, { openPaths: [], activePath: null }, 'empty strip preserved verbatim');
});

check('deserializeTabs returns null for missing, malformed, or wrong-version input', () => {
  assert(deserializeTabs(null, ['A.swift']) === null, 'missing');
  assert(deserializeTabs('not json', ['A.swift']) === null, 'malformed');
  assert(deserializeTabs(JSON.stringify({ version: TABS_STORAGE_VERSION + 1, tabs: {} }), ['A.swift']) === null, 'wrong version');
  assert(deserializeTabs(JSON.stringify({ version: TABS_STORAGE_VERSION }), ['A.swift']) === null, 'missing tabs field');
});

// ── schema validation vs. sanitation (trust boundary) ────────────────────
// A *malformed* persisted strip must be REJECTED (deserialize → null → boot
// falls back to the default file), never quietly coerced down to the empty
// strip — otherwise a corrupt blob is indistinguishable from a user's
// deliberate "I closed my last tab" empty state, stranding them on a blank
// editor. `sanitizeTabs` (runtime reconcile) stays tolerant; `deserializeTabs`
// (the parse boundary) is strict.

check('isValidTabsShape accepts a well-formed strip (including the explicit-empty one) and rejects malformed shapes', () => {
  assert(isValidTabsShape({ openPaths: ['A.swift'], activePath: 'A.swift' }) === true, 'normal');
  assert(isValidTabsShape({ openPaths: [], activePath: null }) === true, 'valid explicit empty');
  assert(isValidTabsShape(null) === false, 'null');
  assert(isValidTabsShape({ openPaths: 'A.swift', activePath: 'A.swift' }) === false, 'openPaths not an array');
  assert(isValidTabsShape({ openPaths: ['A.swift', 3], activePath: 'A.swift' }) === false, 'non-string entry');
  assert(isValidTabsShape({ openPaths: ['A.swift'], activePath: 7 }) === false, 'non-string/non-null activePath');
  assert(isValidTabsShape({ activePath: null }) === false, 'missing openPaths');
});

check('deserializeTabs rejects a versioned strip with a wrong-typed openPaths (falls back, not sanitized-to-empty)', () => {
  const text = JSON.stringify({ version: TABS_STORAGE_VERSION, tabs: { openPaths: 'A.swift', activePath: 'A.swift' } });
  assert(deserializeTabs(text, ['A.swift']) === null, 'malformed openPaths must fall back to the default file, not the honest-empty state');
});

check('deserializeTabs rejects a versioned strip with a wrong-typed activePath', () => {
  const text = JSON.stringify({ version: TABS_STORAGE_VERSION, tabs: { openPaths: ['A.swift'], activePath: 7 } });
  assert(deserializeTabs(text, ['A.swift']) === null, 'non-string activePath must fall back');
});

check('deserializeTabs rejects a versioned strip whose openPaths holds non-string ids (corrupt, not silently pruned)', () => {
  const text = JSON.stringify({ version: TABS_STORAGE_VERSION, tabs: { openPaths: ['A.swift', 3], activePath: 'A.swift' } });
  assert(deserializeTabs(text, ['A.swift']) === null, 'a non-string tab id makes the whole persisted strip untrustworthy');
});

check('deserializeTabs still preserves a *valid* explicitly-empty strip (must not be confused with malformed)', () => {
  const text = JSON.stringify({ version: TABS_STORAGE_VERSION, tabs: { openPaths: [], activePath: null } });
  const restored = deserializeTabs(text, ['A.swift', 'B.swift']);
  assert(restored !== null, 'a well-formed empty strip is valid, not malformed');
  assertEqual(restored, { openPaths: [], activePath: null }, 'valid empty preserved verbatim');
});

// ── Studio.astro wiring (DOM/editor integration) ────────────────────────
// No jsdom in this project's offline npm cache (same recurring constraint as
// `studio-editor-lint.mjs`/`studio-sidebar-mode.mjs`), so these pin the fix
// structurally against the component's own source rather than mounting a
// live page.

const here = path.dirname(fileURLToPath(import.meta.url));
const studioSrc = fs.readFileSync(path.join(here, '..', 'src', 'components', 'Studio.astro'), 'utf8');

check('boot restores a persisted tab strip as-is, with no forced re-open of the project\'s active file', () => {
  const readIdx = studioSrc.indexOf('function readStoredTabs(validPaths)');
  assert(readIdx !== -1, 'expected a readStoredTabs(validPaths) function');
  const declIdx = studioSrc.indexOf('let tabs = readStoredTabs(projectPaths)', readIdx);
  assert(declIdx !== -1, 'expected the initial `tabs` assignment to read persisted tabs first');
  // The bug: a line right after this assignment used to unconditionally
  // reopen `project.activePath` whenever `tabs.activePath` was falsy —
  // which also fires for a *valid* persisted empty strip (openPaths: [],
  // activePath: null), silently resurrecting a tab the user had closed.
  const afterDecl = studioSrc.slice(declIdx, declIdx + 400);
  assert(
    !/if \(!tabs\.activePath\) tabs = openTab\(tabs, project\.activePath\);/.test(afterDecl),
    'expected no unconditional re-open of project.activePath right after restoring persisted tabs',
  );
});

check('reconcileTabs() only forces a re-open when NOT called with { allowEmpty: true }', () => {
  const fnIdx = studioSrc.indexOf('function reconcileTabs(');
  assert(fnIdx !== -1, 'expected a reconcileTabs() function');
  const body = studioSrc.slice(fnIdx, studioSrc.indexOf('\n  }', fnIdx));
  assert(/allowEmpty\s*=\s*false/.test(body), 'expected reconcileTabs to default allowEmpty to false');
  assert(/if \(!tabs\.activePath && !allowEmpty\) \{/.test(body), 'expected reconcileTabs to skip the forced re-open when allowEmpty is true');
  // project.activePath can itself be the canonical empty '' (last tab closed
  // before this structural edit ran) — the fallback must not blindly reopen
  // it as a bogus '' tab, and must fall back to the first project file.
  assert(
    /const fallback = project\.activePath \|\| project\.files\[0\]\?\.path;/.test(body),
    'expected reconcileTabs to fall back to the first file when project.activePath is the canonical empty \'\'',
  );
  assert(/if \(fallback\) tabs = openTab\(tabs, fallback\);/.test(body), 'expected reconcileTabs to only reopen a real path');
});

check('boot is the call site that opts into allowEmpty, preserving a persisted empty strip across reload', () => {
  const bootIdx = studioSrc.indexOf('reconcileTabs({ allowEmpty: true });');
  assert(bootIdx !== -1, 'expected exactly one boot-time reconcileTabs({ allowEmpty: true }) call');
  // Every other reconcileTabs() caller (add/rename/delete file) must keep the
  // default (forced re-open) behavior — only boot-time restore is allowed to
  // land on the honest empty state without user action in this session.
  const otherCalls = studioSrc.match(/reconcileTabs\([^)]*\);/g) || [];
  assert(otherCalls.length >= 3, 'expected reconcileTabs to still be called from the structural file-mutation paths');
  const nonBootCalls = otherCalls.filter((c) => c !== 'reconcileTabs({ allowEmpty: true });');
  assert(
    nonBootCalls.every((c) => c === 'reconcileTabs();'),
    `expected every non-boot reconcileTabs() call to use the default (forcing) behavior, got: ${nonBootCalls.join(', ')}`,
  );
});

check('closing the last tab hides the editor pane itself (not just an overlay), so no stale focusable content remains', () => {
  const fnIdx = studioSrc.indexOf('function updateEditorVisibility()');
  assert(fnIdx !== -1, 'expected an updateEditorVisibility() helper');
  const body = studioSrc.slice(fnIdx, studioSrc.indexOf('\n  }', fnIdx));
  assert(/const empty = tabs\.activePath === null;/.test(body), 'expected the empty state to be derived from tabs.activePath');
  assert(/editorEl\.hidden = empty;/.test(body), 'expected the CodeMirror host element itself to be hidden when empty (removes it from layout/tab order/a11y tree)');
  assert(/editorEmptyEl\.hidden = !empty;/.test(body), 'expected the empty-state placeholder to be shown exactly when the editor is hidden');
});

check('structural file-mutation handlers reconcile through the syncEditorToTabs seam, not a bare editor.open()', () => {
  // Deleting/renaming a file can leave project.js and tabs.js disagreeing on
  // the active path (project.js may pick the first file; tabs.js picks a
  // deterministic neighbor). Each handler must settle `tabs` via
  // reconcileTabs() and then reconcile project+editor through the single
  // syncEditorToTabs() seam — calling editor.open() directly would open the
  // tab neighbor while leaving project.activePath (persistence/diagnostics)
  // pointed elsewhere.
  for (const marker of ['renameFilePrompt', 'deleteFilePrompt']) {
    const fnIdx = studioSrc.indexOf(`function ${marker}(`);
    assert(fnIdx !== -1, `expected a ${marker}() function`);
    const body = studioSrc.slice(fnIdx, studioSrc.indexOf('\n  }', fnIdx));
    assert(/reconcileTabs\(\);/.test(body), `expected ${marker} to settle tabs via reconcileTabs()`);
    assert(/syncEditorToTabs\(\);/.test(body), `expected ${marker} to reconcile project/editor to tabs through the shared seam`);
    assert(!/editor\.open\(/.test(body), `expected ${marker} not to call editor.open() directly (that bypasses the project.activePath reconciliation seam)`);
  }
});

check('add/new-project handlers also reconcile through the syncEditorToTabs seam', () => {
  const addIdx = studioSrc.indexOf('function addFilePrompt(');
  const addBody = studioSrc.slice(addIdx, studioSrc.indexOf('\n  }', addIdx));
  assert(/syncEditorToTabs\(\);/.test(addBody) && !/editor\.open\(/.test(addBody),
    'expected addFilePrompt to open the new file through syncEditorToTabs, not editor.open()');
  // The New Project handler is an inline arrow; check the syncEditorToTabs
  // call appears in the createProject block instead of a bare editor.open().
  const npIdx = studioSrc.indexOf('project = store.createProject(sample.name, sample.files);');
  const npBody = studioSrc.slice(npIdx, npIdx + 300);
  assert(/syncEditorToTabs\(\);/.test(npBody) && !/editor\.open\(/.test(npBody),
    'expected the New Project flow to reconcile through syncEditorToTabs, not editor.open()');
});

check('delete-active-middle-tab converges project, tabs and editor on ONE canonical active path (no split-brain)', () => {
  // Behavioral model of the production seam (reconcileTabs + syncEditorToTabs)
  // using the real tabs.js functions: project.js picks the first file after a
  // delete, tabs.js picks the deterministic neighbor — the seam must make
  // project.activePath (and the editor) follow tabs so nothing disagrees.
  const files = ['A.swift', 'B.swift', 'C.swift'];
  let project = { files: files.map((p) => ({ path: p })), activePath: 'B.swift' };
  let tabs = { openPaths: [...files], activePath: 'B.swift' };
  let editorOpenedPath = 'B.swift';

  // delete the active middle file 'B.swift'
  project = { files: project.files.filter((f) => f.path !== 'B.swift'), activePath: 'A.swift' }; // project.js -> first
  tabs = closeTab(tabs, 'B.swift'); // tabs.js -> right neighbor
  assert(tabs.activePath === 'C.swift', 'tabs picks the deterministic right neighbor, not the first file');

  // reconcileTabs() (default): sanitize against the surviving files
  tabs = sanitizeTabs(tabs, project.files.map((f) => f.path));
  if (!tabs.activePath) tabs = openTab(tabs, project.activePath);
  // syncEditorToTabs(): project.activePath := tabs.activePath, editor opens it
  if (tabs.activePath) { project = { ...project, activePath: tabs.activePath }; editorOpenedPath = tabs.activePath; }

  assert(tabs.activePath === 'C.swift', 'tabs remain on the neighbor');
  assert(project.activePath === 'C.swift', 'project.activePath reconciled to the tab neighbor, overriding project.js first-file guess');
  assert(editorOpenedPath === 'C.swift', 'the editor opened the one canonical active path');
});

check('every tab-state mutation path (open/close/sync/boot) re-derives editor visibility through the shared helper', () => {
  for (const marker of ['renderTabs', 'syncEditorToTabs']) {
    const fnIdx = studioSrc.indexOf(`function ${marker}(`);
    assert(fnIdx !== -1, `expected a ${marker}() function`);
    const body = studioSrc.slice(fnIdx, studioSrc.indexOf('\n  }', fnIdx));
    assert(/updateEditorVisibility\(\);/.test(body), `expected ${marker}() to call the shared updateEditorVisibility() helper`);
  }
});

check('syncEditorToTabs sets project.activePath to the canonical empty \'\' when closing the last tab', () => {
  // Bug: syncEditorToTabs used to only assign project.activePath inside the
  // `if (tabs.activePath)` branch, leaving it stale (still pointing at the
  // just-closed file) whenever tabs/editor reached the honest-empty state —
  // a split-brain between tabs (nothing open) and project (something
  // "active"). project.activePath's own canonical empty is '' (see
  // project.js's createProject), so the else branch must assign that,
  // keeping project/tabs/editor on one consistent empty representation.
  const fnIdx = studioSrc.indexOf('function syncEditorToTabs()');
  const body = studioSrc.slice(fnIdx, studioSrc.indexOf('\n  }', fnIdx));
  assert(/\} else \{/.test(body), 'expected syncEditorToTabs to handle the null-activePath case explicitly');
  assert(
    /project = \{ \.\.\.project, activePath: '' \};/.test(body),
    "expected the empty branch to set project.activePath to the canonical empty ''",
  );
});

check('closing the last tab converges project, tabs, and editor on ONE canonical empty state (no split-brain)', () => {
  // Direct behavioral test of the fixed bug: drive the real tabs.js state
  // machine through open -> close-last-tab, then run the production seam
  // (syncEditorToTabs's now-fixed logic) against a fake project/editor and
  // assert every observer agrees on the empty representation.
  let project = { files: [{ path: 'A.swift', source: '' }], activePath: 'A.swift' };
  let tabs = openTab(emptyTabs(), 'A.swift');
  let editorOpenedWith = 'A.swift';

  tabs = closeTab(tabs, 'A.swift');
  assertEqual(tabs, { openPaths: [], activePath: null }, 'tabs reaches the honest empty state');

  // syncEditorToTabs (fixed): follow tabs.activePath, else canonicalize
  // project.activePath to '' and leave the editor untouched (no spurious open).
  if (tabs.activePath) {
    project = { ...project, activePath: tabs.activePath };
    editorOpenedWith = tabs.activePath;
  } else {
    project = { ...project, activePath: '' };
  }

  assert(tabs.activePath === null, 'tabs: empty');
  assert(project.activePath === '', "project: canonical empty '' (not stale, not null)");
  assert(editorOpenedWith === 'A.swift', 'editor.open() must not fire again — no spurious reopen of the closed file');

  // Persistence round-trip: project.activePath === '' matches no file, so
  // project.js's own deserialize() falls back to files[0] on reload —
  // proving '' is a safe, already-supported canonical empty, not a new one
  // that could break serialization.
  const matchesAFile = project.files.some((f) => f.path === project.activePath);
  assert(!matchesAFile, "project.activePath '' must not accidentally alias a real file path");
});

check('reconcileTabs() falls back to the first project file when project.activePath is the canonical empty (fallback coverage)', () => {
  // Model reconcileTabs's fixed fallback: after closing the last tab
  // (project.activePath now ''), a subsequent structural edit (e.g. add file)
  // must still land on a real, open tab — not a bogus '' tab, and not stay
  // stuck empty (structural edits never reach the honest-empty state).
  const project = { files: [{ path: 'A.swift' }, { path: 'B.swift' }], activePath: '' };
  let tabs = emptyTabs();

  tabs = sanitizeTabs(tabs, project.files.map((f) => f.path));
  const allowEmpty = false;
  if (!tabs.activePath && !allowEmpty) {
    const fallback = project.activePath || project.files[0]?.path;
    if (fallback) tabs = openTab(tabs, fallback);
  }

  assertEqual(tabs, { openPaths: ['A.swift'], activePath: 'A.swift' }, 'falls back to the first file, not a \'\' tab');
});

check('reopening a tab after the honest-empty state restores content via editor.open(), never by re-creating the editor', () => {
  const fnIdx = studioSrc.indexOf('function syncEditorToTabs()');
  const body = studioSrc.slice(fnIdx, studioSrc.indexOf('\n  }', fnIdx));
  assert(/if \(tabs\.activePath\) \{/.test(body), 'expected syncEditorToTabs to guard the open on a non-null activePath');
  assert(/editor\.open\(tabs\.activePath, store\.fileSource\(project, tabs\.activePath\) \?\? ''\);/.test(body),
    'expected syncEditorToTabs to reopen via editor.open(), which restores each file\'s cached EditorState (no content loss)');
});

// Hand-rolled state machine exercising the same shape updateEditorVisibility()
// touches (`.hidden` on two elements), driven by the pure `tabs.js` functions
// this file already imports — proves the *behavior* end-to-end (open → close
// last tab → hidden editor → reopen → visible again with prior content slot
// intact) without needing jsdom, matching studio-sidebar-mode.mjs's precedent.
check('open/close/reopen cycle: editor host hides exactly when honest-empty, and the closed file\'s slot survives', () => {
  const editorEl = { hidden: false };
  const editorEmptyEl = { hidden: true };
  const openedWith = []; // records editor.open(path) calls a real EditorState cache would key on
  function updateEditorVisibility(tabs) {
    const empty = tabs.activePath === null;
    editorEl.hidden = empty;
    editorEmptyEl.hidden = !empty;
  }
  function syncEditorToTabs(tabs) {
    if (tabs.activePath) openedWith.push(tabs.activePath);
    updateEditorVisibility(tabs);
  }

  let tabs = openTab(emptyTabs(), 'A.swift');
  syncEditorToTabs(tabs);
  assert(editorEl.hidden === false && editorEmptyEl.hidden === true, 'editor visible with one tab open');

  tabs = closeTab(tabs, 'A.swift');
  syncEditorToTabs(tabs);
  assert(editorEl.hidden === true && editorEmptyEl.hidden === false, 'editor hidden, placeholder shown after closing the last tab');
  assert(openedWith.length === 1, 'editor.open() must not fire for the null-activePath (honest empty) state');

  tabs = openTab(tabs, 'A.swift'); // reopen the same file
  syncEditorToTabs(tabs);
  assert(editorEl.hidden === false && editorEmptyEl.hidden === true, 'editor visible again after reopening');
  assertEqual(openedWith, ['A.swift', 'A.swift'], 'reopening re-issues editor.open() for the same path, restoring its cached content');
});

if (failures > 0) {
  console.error(`\n${failures} test(s) failed.`);
  process.exit(1);
} else {
  console.log('\nAll studio-tabs tests passed.');
}
