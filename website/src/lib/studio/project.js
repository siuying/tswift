// Pure project/file model for the Web Studio (no DOM, no wasm) — every function
// here is a plain data transform so it can be unit-tested in Node.
//
// A `Project` is:
//   { name: string, files: [{ path, source }], activePath: string }
//
// `path` is a virtual project-relative path (`main.swift`, `Sources/App.swift`,
// `Package.swift`). Studio never touches a real filesystem — projects live in
// memory and are mirrored to localStorage by the caller.

/** Storage key for the autosaved project. */
export const STORAGE_KEY = 'tswift-studio-project';
/** Storage schema version, bumped if the shape changes incompatibly. */
export const STORAGE_VERSION = 1;

/** A path is a valid Swift source (or the manifest) if it is a clean, unique name. */
export function validatePath(project, path, { ignore = null } = {}) {
  const p = (path ?? '').trim();
  if (!p) return 'File name cannot be empty';
  if (p.startsWith('/') || p.endsWith('/')) return 'File name cannot start or end with "/"';
  if (/\/\//.test(p)) return 'File name cannot contain "//"';
  if (/[\\:*?"<>|]/.test(p)) return 'File name contains invalid characters';
  const isManifest = basename(p) === 'Package.swift';
  if (!p.endsWith('.swift')) return 'File must end in .swift';
  if (isManifest && p !== 'Package.swift') {
    return 'Package.swift must live at the project root';
  }
  const clash = project.files.some((f) => f.path === p && f.path !== ignore);
  if (clash) return `A file named "${p}" already exists`;
  return null;
}

/** The last path component (`Sources/App.swift` -> `App.swift`). */
export function basename(path) {
  const i = path.lastIndexOf('/');
  return i === -1 ? path : path.slice(i + 1);
}

/** Whether a path is the SwiftPM manifest (root Package.swift). */
export function isManifest(path) {
  return path === 'Package.swift';
}

/** Create a fresh project from a name + files, selecting the first file. */
export function createProject(name, files) {
  const list = files.map((f) => ({ path: f.path, source: f.source ?? '' }));
  return {
    name: name || 'Untitled',
    files: list,
    activePath: list.length ? list[0].path : '',
  };
}

/** Return a copy of `project` with a new file added (throws on invalid path). */
export function addFile(project, path, source = '') {
  const error = validatePath(project, path);
  if (error) throw new Error(error);
  const files = [...project.files, { path, source }];
  return { ...project, files: sortFiles(files), activePath: path };
}

/** Rename `oldPath` to `newPath` (throws on invalid target). */
export function renameFile(project, oldPath, newPath) {
  if (oldPath === newPath) return project;
  const error = validatePath(project, newPath, { ignore: oldPath });
  if (error) throw new Error(error);
  const files = project.files.map((f) =>
    f.path === oldPath ? { ...f, path: newPath } : f,
  );
  return {
    ...project,
    files: sortFiles(files),
    activePath: project.activePath === oldPath ? newPath : project.activePath,
  };
}

/** Delete `path`. Refuses to remove the final file so a project is never empty. */
export function deleteFile(project, path) {
  if (project.files.length <= 1) throw new Error('A project must keep at least one file');
  const files = project.files.filter((f) => f.path !== path);
  let activePath = project.activePath;
  if (activePath === path) activePath = files[0].path;
  return { ...project, files, activePath };
}

/** Update one file's source in place (identity if the path is unknown). */
export function updateSource(project, path, source) {
  let found = false;
  const files = project.files.map((f) => {
    if (f.path !== path) return f;
    found = true;
    return { ...f, source };
  });
  return found ? { ...project, files } : project;
}

/** Look up a file's source by path (or null). */
export function fileSource(project, path) {
  const f = project.files.find((x) => x.path === path);
  return f ? f.source : null;
}

/** Stable file ordering: Package.swift first, then `main.swift`, then alphabetical. */
export function sortFiles(files) {
  return [...files].sort((a, b) => rank(a.path) - rank(b.path) || a.path.localeCompare(b.path));
}

function rank(path) {
  if (isManifest(path)) return 0;
  if (basename(path) === 'main.swift') return 1;
  return 2;
}

// ── Persistence (localStorage mirror; pure serialize/deserialize below) ──────

/** Serialize a project to a versioned JSON string for storage. */
export function serialize(project) {
  return JSON.stringify({ version: STORAGE_VERSION, project });
}

/**
 * Parse a stored JSON string back to a project, or null if it is missing,
 * malformed, the wrong version, or structurally invalid. Never throws.
 */
export function deserialize(text) {
  if (!text) return null;
  let data;
  try {
    data = JSON.parse(text);
  } catch {
    return null;
  }
  if (!data || data.version !== STORAGE_VERSION) return null;
  const p = data.project;
  if (!p || typeof p.name !== 'string' || !Array.isArray(p.files) || p.files.length === 0) {
    return null;
  }
  // Reject structurally-invalid or duplicate paths outright rather than
  // silently loading a corrupt/tampered project (e.g. "//etc", a non-.swift
  // file, or two files sharing a path) — the caller falls back to the
  // starter project on `null`. Path *format* is checked against an empty
  // sibling list (validatePath's own clash check only guards against other
  // entries, and matches by value rather than identity, so it can't detect
  // duplicates by itself); uniqueness is tracked separately below.
  const seenPaths = new Set();
  for (const f of p.files) {
    if (!f || typeof f.path !== 'string' || typeof f.source !== 'string') return null;
    if (validatePath({ files: [] }, f.path) !== null) return null;
    if (seenPaths.has(f.path)) return null;
    seenPaths.add(f.path);
  }
  const activePath = p.files.some((f) => f.path === p.activePath)
    ? p.activePath
    : p.files[0].path;
  return { name: typeof p.name === 'string' ? p.name : 'Untitled', files: p.files, activePath };
}
