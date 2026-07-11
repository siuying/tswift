// Pure transforms over the wasm `listSymbols` output: build a nested outline
// tree for the sidebar, and a flat quick-open index (files + symbols) with a
// small subsequence fuzzy matcher. No DOM, no wasm — unit-testable in Node.
//
// A symbol from `listSymbols` is:
//   { name, kind, file, line, container?, signature? }

import { basename } from './project.js';

/**
 * Group symbols into an outline: top-level symbols (no container) become roots,
 * and any symbol whose `container` matches a root's name nests beneath it.
 * Ordering within a group follows source line. Symbols are scoped per-file so
 * two files with a same-named type don't cross-nest.
 */
export function buildOutline(symbols) {
  const byFile = new Map();
  for (const s of symbols) {
    if (!byFile.has(s.file)) byFile.set(s.file, []);
    byFile.get(s.file).push(s);
  }
  const files = [];
  for (const [file, syms] of byFile) {
    const roots = [];
    const containers = new Map(); // name -> node (for nesting members)
    const sorted = [...syms].sort((a, b) => a.line - b.line);
    for (const s of sorted) {
      const node = { ...s, children: [] };
      if (s.container && containers.has(s.container)) {
        containers.get(s.container).children.push(node);
      } else {
        roots.push(node);
      }
      // A container (struct/class/enum/…) can host later members.
      if (isContainerKind(s.kind)) containers.set(s.name, node);
    }
    files.push({ file, roots });
  }
  files.sort((a, b) => a.file.localeCompare(b.file));
  return files;
}

function isContainerKind(kind) {
  return ['struct', 'class', 'enum', 'protocol', 'actor', 'extension'].includes(kind);
}

/**
 * Flat quick-open index: one entry per file plus one per symbol. Each entry is
 * `{ type:'file'|'symbol', label, detail, file, line }` — `line` is 1 for a
 * file entry, the declaration line for a symbol.
 */
export function quickOpenIndex(project, symbols) {
  const entries = [];
  for (const f of project.files) {
    entries.push({ type: 'file', label: basename(f.path), detail: f.path, file: f.path, line: 1 });
  }
  for (const s of symbols) {
    const detail = s.container ? `${s.container} · ${basename(s.file)}` : basename(s.file);
    entries.push({
      type: 'symbol',
      label: s.name,
      kind: s.kind,
      detail,
      file: s.file,
      line: s.line,
    });
  }
  return entries;
}

/**
 * Filter+rank quick-open entries by a query using a case-insensitive
 * subsequence match against the label (contiguous and prefix matches rank
 * higher). Empty query returns everything in original order. Pure + stable.
 */
export function filterEntries(entries, query) {
  const q = (query ?? '').trim().toLowerCase();
  if (!q) return entries.slice(0, 50);
  const scored = [];
  for (const e of entries) {
    const score = fuzzyScore(e.label.toLowerCase(), q);
    if (score >= 0) scored.push({ e, score });
  }
  scored.sort((a, b) => b.score - a.score);
  return scored.map((s) => s.e).slice(0, 50);
}

/**
 * Subsequence score: -1 if `q` is not a subsequence of `text`. Higher is
 * better — a contiguous substring beats a scattered match, an early match beats
 * a late one, and a shorter haystack beats a longer one (all else equal).
 */
export function fuzzyScore(text, q) {
  const idx = text.indexOf(q);
  if (idx !== -1) {
    // Contiguous substring: strong bonus, prefix strongest, shorter is better.
    return 1000 - idx * 5 - (text.length - q.length) + (idx === 0 ? 200 : 0);
  }
  // Scattered subsequence.
  let ti = 0;
  let matched = 0;
  let firstAt = -1;
  for (let qi = 0; qi < q.length; qi += 1) {
    const found = text.indexOf(q[qi], ti);
    if (found === -1) return -1;
    if (firstAt === -1) firstAt = found;
    matched += 1;
    ti = found + 1;
  }
  return 200 - firstAt * 2 - (text.length - matched);
}
