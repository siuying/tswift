// Pure helpers that turn a Studio `Project` into the wire shapes the wasm
// exports consume, plus the thin client-side decisions about *how* to run it.
//
// The wasm `runSwiftModule` / `swiftUICompileModule` own the real compilation
// semantics; this module only (a) drops the non-source SwiftPM manifest, (b)
// orders the remaining files so the entry file leads, and (c) decides console
// vs. SwiftUI mode. Keep it thin — no parsing beyond cheap text heuristics.

import { isManifest, basename, sortFiles } from './project.js';

/**
 * Ordered `[{ path, contents }]` for the wasm module exports.
 *
 * Package.swift is excluded (it is a build manifest, not a compilation unit).
 * When a manifest is present we honour the conventional SwiftPM `Sources/`
 * layout only enough to keep entry selection sane — the real target/entry
 * resolution stays server-side in wasm; we just order files deterministically
 * with the entry file first so top-level statements execute in a stable order.
 */
export function moduleFiles(project) {
  const sources = project.files.filter((f) => !isManifest(f.path));
  const entry = entryFile(project);
  const ordered = sortFiles(sources).sort((a, b) => {
    if (entry) {
      if (a.path === entry) return -1;
      if (b.path === entry) return 1;
    }
    return 0;
  });
  return ordered.map((f) => ({ path: f.path, contents: f.source }));
}

/** JSON string in the `{"files":[{path,contents}]}` shape the wasm exports take. */
export function moduleJson(project) {
  return JSON.stringify({ files: moduleFiles(project) });
}

/**
 * Pick the entry file: the one carrying top-level executable statements or an
 * `@main` attribute. Falls back to `main.swift`, then the first source file.
 * Returns a path string or null when the project has only the manifest.
 */
export function entryFile(project) {
  const sources = project.files.filter((f) => !isManifest(f.path));
  if (sources.length === 0) return null;

  const mainAttr = sources.find((f) => /(^|\n)\s*@main\b/.test(f.source));
  if (mainAttr) return mainAttr.path;

  const named = sources.find((f) => basename(f.path) === 'main.swift');
  if (named) return named.path;

  const topLevel = sources.find((f) => hasTopLevelStatement(f.source));
  if (topLevel) return topLevel.path;

  return sources[0].path;
}

/**
 * Whether the project should render as a live SwiftUI canvas (some source
 * declares a `View`/`App`) rather than run as a console program.
 */
export function isSwiftUIProject(project) {
  return project.files.some((f) => !isManifest(f.path) && declaresView(f.source));
}

/** Cheap `View`/`App` conformance heuristic (comments/strings not stripped). */
export function declaresView(source) {
  return (
    /\b(struct|class)\s+\w+\s*:\s*[^{]*\bView\b/.test(source) ||
    /\b(struct|class)\s+\w+\s*:\s*[^{]*\bApp\b/.test(source) ||
    /\bsome\s+View\b/.test(source)
  );
}

/**
 * Heuristic for "this file has top-level code that runs on launch": a line that
 * is neither blank, a comment, an import, nor the start of a declaration.
 */
function hasTopLevelStatement(source) {
  const decl = /^(import|@|public|private|internal|fileprivate|open|final|struct|class|enum|protocol|extension|actor|func|typealias|associatedtype|indirect|static)\b/;
  let depth = 0;
  for (const raw of source.split('\n')) {
    const line = raw.trim();
    if (!line || line.startsWith('//')) continue;
    // Only consider lines at brace depth 0 (i.e. genuinely top-level).
    if (depth === 0 && !decl.test(line)) return true;
    depth += (line.match(/\{/g) || []).length - (line.match(/\}/g) || []).length;
    if (depth < 0) depth = 0;
  }
  return false;
}
