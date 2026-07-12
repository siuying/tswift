// Pins the compact accessible Files/Report/Symbols sidebar mode switcher:
// the left sidebar used to stack the Files tree and the Outline panel on
// top of each other at all times, and "Problems" lived as a third tab in
// the right-hand Preview/Console tab bar. That left the sidebar cramped
// (two always-visible panels sharing limited vertical space) and split a
// user's "what's wrong with my project" question across two unrelated
// locations (right-tab Problems vs. left-panel Outline).
//
// Studio.astro now exposes exactly one compact, WAI-ARIA tablist-pattern
// switcher in the sidebar with three modes — Files, Report, Symbols — and
// only one panel is visible (not `hidden`) at a time. This is a DOM-driven
// component with no jsdom in this project's offline test tier (see
// `studio-editor-lint.mjs`'s doc comment for the same constraint), so — same
// precedent — this test pins the fix structurally against the component's
// source text rather than mounting a live page.

import fs from 'node:fs';
import path from 'node:path';
import process from 'node:process';
import { fileURLToPath } from 'node:url';
import {
  SIDEBAR_MODES,
  applySidebarMode,
  nextSidebarMode,
} from '../src/lib/studio/sidebar-mode.js';

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
const studioSrc = fs.readFileSync(path.join(here, '..', 'src', 'components', 'Studio.astro'), 'utf8');

check('sidebar exposes a role="tablist" switcher with exactly Files/Report/Symbols tabs', () => {
  assert(/class="st-mode-switcher"\s+role="tablist"/.test(studioSrc), 'expected a role="tablist" .st-mode-switcher container');
  for (const mode of ['files', 'report', 'symbols']) {
    assert(
      new RegExp(`role="tab" id="st-mode-${mode}"[^>]*data-mode="${mode}"`).test(studioSrc),
      `expected a role="tab" button for mode "${mode}"`,
    );
  }
});

check('each mode tab declares aria-selected + aria-controls wired to its tabpanel', () => {
  const pairs = [
    ['st-mode-files', 'st-panel-files'],
    ['st-mode-report', 'st-panel-report'],
    ['st-mode-symbols', 'st-panel-symbols'],
  ];
  for (const [tabId, panelId] of pairs) {
    const tabIdx = studioSrc.indexOf(`id="${tabId}"`);
    assert(tabIdx !== -1, `expected a tab with id="${tabId}"`);
    const tag = studioSrc.slice(tabIdx, studioSrc.indexOf('>', tabIdx));
    assert(tag.includes('aria-selected='), `expected aria-selected on #${tabId}`);
    assert(tag.includes(`aria-controls="${panelId}"`), `expected #${tabId} to control #${panelId}`);
    const panelIdx = studioSrc.indexOf(`id="${panelId}"`);
    assert(panelIdx !== -1, `expected a panel with id="${panelId}"`);
    const panelTag = studioSrc.slice(panelIdx, studioSrc.indexOf('>', panelIdx));
    assert(panelTag.includes('role="tabpanel"'), `expected #${panelId} to be role="tabpanel"`);
    assert(panelTag.includes(`aria-labelledby="${tabId}"`), `expected #${panelId} to be labelled by #${tabId}`);
  }
});

check('only the Files panel is visible by default; Report and Symbols start hidden', () => {
  const filesTag = studioSrc.slice(studioSrc.indexOf('id="st-panel-files"'), studioSrc.indexOf('id="st-panel-report"'));
  assert(!/\bhidden\b/.test(filesTag.split('>')[0]), 'expected st-panel-files to NOT carry the hidden attribute');
  for (const id of ['st-panel-report', 'st-panel-symbols']) {
    const idx = studioSrc.indexOf(`id="${id}"`);
    const openTag = studioSrc.slice(idx, studioSrc.indexOf('>', idx));
    assert(/\bhidden\b/.test(openTag), `expected #${id} to start with the hidden attribute`);
  }
});

check('the right-hand tab bar no longer has a separate Problems tab (moved into the sidebar Report mode)', () => {
  assert(!/data-rtab="problems"/.test(studioSrc), 'expected data-rtab="problems" to be removed');
  assert(!/showRightTab\(['"]problems['"]\)/.test(studioSrc), 'expected no remaining showRightTab(\'problems\') calls');
});

check('the Report tab still surfaces the live problem-count badge', () => {
  const reportIdx = studioSrc.indexOf('id="st-mode-report"');
  const reportBtn = studioSrc.slice(reportIdx, studioSrc.indexOf('</button>', reportIdx));
  assert(reportBtn.includes('id="st-problem-count"'), 'expected the Report tab button to contain the #st-problem-count badge');
});

check('applySidebarMode() drives roving aria-selected/tabIndex + panel .hidden, not raw style.display (shared seam)', () => {
  const seamSrc = fs.readFileSync(path.join(here, '..', 'src', 'lib', 'studio', 'sidebar-mode.js'), 'utf8');
  const fnIdx = seamSrc.indexOf('export function applySidebarMode');
  assert(fnIdx !== -1, 'expected an exported applySidebarMode() in the shared seam');
  const body = seamSrc.slice(fnIdx, seamSrc.indexOf('\nexport function nextSidebarMode', fnIdx));
  assert(/panels\[m\]\.hidden = m !== mode/.test(body), 'expected applySidebarMode to toggle the panel .hidden property');
  assert(/setAttribute\('aria-selected'/.test(body), 'expected applySidebarMode to update aria-selected');
  assert(/\.tabIndex = active \? 0 : -1/.test(body), 'expected applySidebarMode to maintain roving tabIndex');
  assert(!/style\.display/.test(body), 'expected applySidebarMode to avoid raw style.display');
});

check('the switcher wires arrow-key / Home / End keyboard nav through the shared nextSidebarMode seam', () => {
  const idx = studioSrc.indexOf(".st-mode-switcher').addEventListener('keydown'");
  assert(idx !== -1, 'expected a keydown listener on .st-mode-switcher');
  const body = studioSrc.slice(idx, studioSrc.indexOf('});', idx));
  assert(/nextSidebarMode\(document\.activeElement\?\.dataset\?\.mode, e\.key\)/.test(body),
    'expected the keydown handler to delegate to nextSidebarMode');
});

check('the SwiftUI event-dispatch handler routes both failure paths through Report + console', () => {
  const evtIdx = studioSrc.indexOf("canvas.addEventListener('swiftui-event'");
  assert(evtIdx !== -1, 'expected the swiftui-event listener');
  const body = studioSrc.slice(evtIdx, studioSrc.indexOf('\n  });', evtIdx));
  // Non-ok dispatch result (SwiftUI event/runtime failure) and a thrown
  // exception (event crash) must both surface diagnostics on Report, not just
  // set a status pill.
  assert(/reportEventFailure\('event error'/.test(body), 'expected the non-ok dispatch branch to call reportEventFailure');
  assert(/reportEventFailure\('event crash'/.test(body), 'expected the catch branch to call reportEventFailure');

  const rfIdx = studioSrc.indexOf('function reportEventFailure(');
  assert(rfIdx !== -1, 'expected a reportEventFailure() helper');
  const rf = studioSrc.slice(rfIdx, studioSrc.indexOf('\n  }', rfIdx));
  assert(/showSidebarMode\('report'\)/.test(rf), 'expected reportEventFailure to select Report mode');
  assert(/showRightTab\('console'\)/.test(rf), 'expected reportEventFailure to surface the console');
  assert(/consoleEl\.textContent = message/.test(rf), 'expected reportEventFailure to expose the diagnostic message');
});

check('showSidebarMode() delegates to the shared applySidebarMode seam (no copied switcher logic)', () => {
  const fnIdx = studioSrc.indexOf('function showSidebarMode(mode)');
  assert(fnIdx !== -1, 'expected a showSidebarMode(mode) function');
  const body = studioSrc.slice(fnIdx, studioSrc.indexOf('\n  }', fnIdx));
  assert(/applySidebarMode\(mode,\s*\{\s*panels:\s*modePanels,\s*buttons:\s*modeBtns\s*\}\)/.test(body),
    'expected showSidebarMode to delegate to the shared applySidebarMode helper');
  assert(/nextSidebarMode\(/.test(studioSrc), 'expected keyboard nav to use the shared nextSidebarMode helper');
});

check('the Run button handler covers every failure branch, each landing on Report mode', () => {
  const runIdx = studioSrc.indexOf("runBtn.addEventListener('click'");
  assert(runIdx !== -1, 'expected the Run button click handler');
  // Slice out just the handler body so branch-scoped assertions below can't
  // accidentally match an unrelated showSidebarMode('report') call elsewhere
  // in the file (which is exactly the gap the previous version of this test
  // had: it only proved *a* call existed, not that *every* failure path made
  // one).
  const closeIdx = studioSrc.indexOf("\n  });", runIdx);
  assert(closeIdx !== -1, 'expected to find the end of the Run button handler');
  const body = studioSrc.slice(runIdx, closeIdx);

  // Branch 1: SwiftUI project, swiftUICompileModule() throws or returns a
  // non-ok/non-root result (SwiftUI compile/runtime failure).
  const swiftUIFailIdx = body.indexOf('SwiftUI compile failed');
  assert(swiftUIFailIdx !== -1, 'expected the SwiftUI-failure comment/message');
  const swiftUIFailBranch = body.slice(swiftUIFailIdx, body.indexOf('return;', swiftUIFailIdx));
  assert(/showSidebarMode\('report'\)/.test(swiftUIFailBranch), 'expected the SwiftUI compile-failure branch to call showSidebarMode(\'report\')');
  assert(/showRightTab\('console'\)/.test(swiftUIFailBranch), 'expected the SwiftUI compile-failure branch to still surface the console');

  // Branch 2: non-SwiftUI project, runSwiftModule() succeeds structurally but
  // the program itself ran and failed (result.run present, result.run.ok
  // false) — a *runtime* failure, not a compile failure.
  const runtimeFailIdx = body.indexOf('if (!result.run.ok)');
  assert(runtimeFailIdx !== -1, 'expected an explicit !result.run.ok branch for runtime failures');
  const runtimeFailBranch = body.slice(runtimeFailIdx, body.indexOf('}', body.indexOf('}', runtimeFailIdx) + 1));
  assert(/showSidebarMode\('report'\)/.test(runtimeFailBranch), 'expected the runtime-failure branch to call showSidebarMode(\'report\')');

  // Branch 3: non-SwiftUI project, runSwiftModule() returned no `result.run`
  // at all (a compile failure).
  const compileFailIdx = body.indexOf('Compile failed.');
  assert(compileFailIdx !== -1, 'expected the non-SwiftUI compile-failure message');
  const compileFailBranch = body.slice(compileFailIdx, body.indexOf('}', compileFailIdx));
  assert(/showSidebarMode\('report'\)/.test(compileFailBranch), 'expected the non-SwiftUI compile-failure branch to call showSidebarMode(\'report\')');

  // Branch 4: the outer try/catch around JSON.parse(runSwiftModule(...)) —
  // an unexpected exception (e.g. malformed JSON, wasm trap) must not leave
  // the sidebar stuck on whatever mode it was in before Run was clicked.
  const catchIdx = body.indexOf('} catch (err) {');
  assert(catchIdx !== -1, 'expected a catch block around runSwiftModule()');
  const catchBranch = body.slice(catchIdx, body.indexOf('\n    }', catchIdx));
  assert(/showSidebarMode\('report'\)/.test(catchBranch), 'expected the exception-catch branch to call showSidebarMode(\'report\')');
  assert(/showRightTab\('console'\)/.test(catchBranch), 'expected the exception-catch branch to still surface the console');
});

// Build a minimal DOM-shaped fake (no jsdom in this project's offline cache —
// see this file's header) exposing exactly the `.hidden` / `.classList` /
// `.setAttribute` / `.tabIndex` surface applySidebarMode touches, so the test
// drives the *real* production state machine rather than a copy of it.
function makeSwitcherUi() {
  const panels = Object.fromEntries(SIDEBAR_MODES.map((m) => [m, { hidden: m !== 'files' }]));
  const buttons = SIDEBAR_MODES.map((m) => ({
    dataset: { mode: m },
    tabIndex: m === 'files' ? 0 : -1,
    attrs: { 'aria-selected': String(m === 'files'), class: m === 'files' ? 'active' : '' },
    classList: {
      toggle(cls, on) { this.owner.attrs.class = on ? cls : ''; },
    },
    setAttribute(k, v) { this.attrs[k] = v; },
  }));
  buttons.forEach((b) => { b.classList.owner = b; });
  const btn = (m) => buttons.find((b) => b.dataset.mode === m);
  return { panels, buttons, btn };
}

check('applySidebarMode() (the production seam) flips exactly one visible panel + roving tab stop', () => {
  const { panels, buttons, btn } = makeSwitcherUi();
  assert(!panels.files.hidden && panels.report.hidden && panels.symbols.hidden, 'files starts visible');

  applySidebarMode('report', { panels, buttons });
  assert(panels.files.hidden && !panels.report.hidden && panels.symbols.hidden, 'report is the only visible panel');
  assert(btn('report').tabIndex === 0 && btn('files').tabIndex === -1, 'report becomes the roving tab stop');
  assert(btn('report').attrs['aria-selected'] === 'true' && btn('files').attrs['aria-selected'] === 'false', 'aria-selected follows the active tab');
  assert(btn('report').attrs.class === 'active' && btn('files').attrs.class === '', 'active class follows the active tab');

  applySidebarMode('symbols', { panels, buttons });
  assert(panels.report.hidden && !panels.symbols.hidden, 'switching to symbols hides report, shows symbols');
  assert(btn('symbols').attrs['aria-selected'] === 'true' && btn('report').attrs['aria-selected'] === 'false', 'aria-selected moves to symbols');
});

check('nextSidebarMode() (the production seam) implements the WAI-ARIA roving-tabindex nav', () => {
  assert(nextSidebarMode('files', 'ArrowRight') === 'report', 'ArrowRight advances files -> report');
  assert(nextSidebarMode('symbols', 'ArrowRight') === 'files', 'ArrowRight wraps symbols -> files');
  assert(nextSidebarMode('files', 'ArrowLeft') === 'symbols', 'ArrowLeft wraps files -> symbols');
  assert(nextSidebarMode('report', 'ArrowDown') === 'symbols', 'ArrowDown behaves like ArrowRight');
  assert(nextSidebarMode('report', 'ArrowUp') === 'files', 'ArrowUp behaves like ArrowLeft');
  assert(nextSidebarMode('report', 'Home') === 'files', 'Home jumps to the first tab');
  assert(nextSidebarMode('report', 'End') === 'symbols', 'End jumps to the last tab');
  assert(nextSidebarMode('report', 'Enter') === null, 'a non-nav key returns null');
  assert(nextSidebarMode(undefined, 'ArrowRight') === null, 'an unknown current mode returns null');
});

console.log(failures === 0 ? '\nall studio-sidebar-mode checks passed' : `\n${failures} check(s) failed`);
process.exit(failures === 0 ? 0 : 1);
