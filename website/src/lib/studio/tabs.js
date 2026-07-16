// Pure open/close editor-tab state for the Web Studio (no DOM, no wasm).
//
// The file tree lists *every* file in the project; the tab strip lists only
// the files the user has explicitly opened, plus which one is active.
// `Tabs` is:
//   { openPaths: string[], activePath: string | null }
//
// `activePath` is `null` only when `openPaths` is empty — the "honest empty
// editor" state reached by closing the last open tab. Every other function
// keeps `activePath` in sync with `openPaths` (it's always a member, or
// null iff the list is empty).

/** Storage key for the persisted tab strip. */
export const TABS_STORAGE_KEY = 'tswift-studio-tabs';
/** Storage schema version, bumped if the shape changes incompatibly. */
export const TABS_STORAGE_VERSION = 1;

/** The empty tab strip (no tabs open, no active file). */
export function emptyTabs() {
  return { openPaths: [], activePath: null };
}

/** Open (or activate, if already open) `path`. Appends new tabs at the end. */
export function openTab(tabs, path) {
  const openPaths = tabs.openPaths.includes(path) ? tabs.openPaths : [...tabs.openPaths, path];
  return { openPaths, activePath: path };
}

/**
 * Close `path`'s tab. If it was active, the neighbor that slides into its
 * slot becomes active (the following tab, or the previous one if it was
 * last); if it was the only open tab, `activePath` becomes `null`.
 * Closing a path that isn't open is a no-op (returns `tabs` unchanged).
 */
export function closeTab(tabs, path) {
  const idx = tabs.openPaths.indexOf(path);
  if (idx === -1) return tabs;
  const openPaths = tabs.openPaths.filter((p) => p !== path);
  let activePath = tabs.activePath;
  if (activePath === path) {
    activePath = openPaths.length === 0 ? null : openPaths[Math.min(idx, openPaths.length - 1)];
  }
  return { openPaths, activePath };
}

/** Update an open tab's path in place (e.g. after a file rename), preserving order and active state. */
export function renameTab(tabs, oldPath, newPath) {
  if (oldPath === newPath) return tabs;
  const openPaths = tabs.openPaths.map((p) => (p === oldPath ? newPath : p));
  const activePath = tabs.activePath === oldPath ? newPath : tabs.activePath;
  return { openPaths, activePath };
}

/**
 * Reconcile a tab strip against the set of paths that currently exist
 * (`validPaths`). Drops open tabs for paths that no longer exist (deleted
 * files, or corrupt/tampered persisted state), drops duplicate entries
 * (keeping the first occurrence), and re-derives `activePath` if it no
 * longer points at an open tab. Never throws.
 */
export function sanitizeTabs(tabs, validPaths) {
  const validSet = new Set(validPaths);
  const seen = new Set();
  const openPaths = (Array.isArray(tabs?.openPaths) ? tabs.openPaths : []).filter((p) => {
    if (typeof p !== 'string' || !validSet.has(p) || seen.has(p)) return false;
    seen.add(p);
    return true;
  });
  let activePath = tabs?.activePath;
  if (typeof activePath !== 'string' || !openPaths.includes(activePath)) {
    activePath = openPaths.length ? openPaths[0] : null;
  }
  return { openPaths, activePath };
}

/**
 * Is `tabs` a *structurally* valid `Tabs` value — the shape a well-behaved
 * writer produces (`openPaths` an array of strings, `activePath` a string or
 * `null`)? This is a trust-boundary schema check, deliberately stricter than
 * `sanitizeTabs`: `sanitizeTabs` coerces garbage into a safe strip (dropping
 * bad entries), which is right for *reconciling* live state but wrong for
 * *parsing* persisted state — there, coercing a malformed blob down to
 * `{ openPaths: [], activePath: null }` would be indistinguishable from a
 * user's deliberate "I closed my last tab" empty strip, silently stranding
 * them on a blank editor at boot. A malformed blob must instead be rejected
 * (→ `null` → caller falls back to the default file); only a well-formed
 * strip (empty or not) is trusted and then sanitized against `validPaths`.
 */
export function isValidTabsShape(tabs) {
  if (!tabs || typeof tabs !== 'object') return false;
  if (!Array.isArray(tabs.openPaths)) return false;
  if (!tabs.openPaths.every((p) => typeof p === 'string')) return false;
  if (tabs.activePath !== null && typeof tabs.activePath !== 'string') return false;
  return true;
}

/** Serialize a tab strip to a versioned JSON string for storage. */
export function serializeTabs(tabs) {
  return JSON.stringify({ version: TABS_STORAGE_VERSION, tabs });
}

/**
 * Parse a stored JSON string back to a tab strip, sanitized against
 * `validPaths`. Returns `null` if the text is missing, malformed, or the
 * wrong version (never throws) — callers should fall back to opening the
 * project's active file in that case.
 */
export function deserializeTabs(text, validPaths) {
  if (!text) return null;
  let data;
  try {
    data = JSON.parse(text);
  } catch {
    return null;
  }
  if (!data || data.version !== TABS_STORAGE_VERSION || !isValidTabsShape(data.tabs)) return null;
  return sanitizeTabs(data.tabs, validPaths);
}
