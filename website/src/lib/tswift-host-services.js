// Browser-side backing for the `tswift.defaults.*` / `tswift.fs.*` host
// services (Foundation's `UserDefaults` / `FileManager`; see
// `crates/tswift-foundation/src/user_defaults.rs` and `file_manager.rs` for
// the wire contract this module implements). Also documented in
// `docs/adr/0014-host-services-web-ios.md`.
//
// ## Platform tier: web (degraded, and DOCUMENTED as such)
//
// The playground runs the wasm interpreter on the **main thread**, not a
// worker (see `FullPlayground.astro`'s `initWasm()` — a plain dynamic
// `import()`, no `new Worker(...)`). The interpreter boundary itself is
// synchronous by design (ADR-0005), so a real filesystem tier needs a
// synchronous storage API. Two candidates exist in browsers:
//
//   - OPFS's *synchronous access handle* API (`createSyncAccessHandle`) is
//     only available inside a **worker** — calling it on the main thread
//     throws. Since this playground doesn't run wasm in a worker, that tier
//     is unavailable here. Wiring it up is future work if/when the
//     interpreter moves off the main thread; this module documents the
//     tripwire rather than faking a not-actually-persistent OPFS shim.
//   - `localStorage` IS synchronous and available on the main thread, so
//     that's the tier this module ships: a virtual, flat-namespaced
//     filesystem persisted to `localStorage` (falling back to a pure
//     in-memory `Map` for anything that doesn't fit — see "Limits" below).
//
// This is a genuinely degraded tier relative to native (`std::fs`, unrooted,
// real disk) and iOS (the app's real sandbox container via Foundation's
// `FileManager`) — named honestly rather than presented as parity:
//
//   | Platform | `tswift.defaults.*`        | `tswift.fs.*`                    |
//   |----------|-----------------------------|-----------------------------------|
//   | native   | in-process / file-backed    | real, unrooted filesystem         |
//   | iOS      | real `UserDefaults.standard`| real `FileManager`, app sandbox   |
//   | web      | `localStorage`, namespaced  | virtual fs; `localStorage`-backed  |
//   |          |                             | when it fits, else in-memory only |
//
// ## Limits
//
// `localStorage` is small (~5MB per origin in most browsers) and every value
// is a UTF-16 string. Binary file content crosses this module's virtual fs as
// base64 already (the wire format itself), so no extra encoding step is
// needed, but base64 costs ~33% overhead on top of the quota. Writes that
// would blow the quota (a `QuotaExceededError`, or content over
// `MAX_LOCALSTORAGE_VALUE_CHARS`) transparently fall back to **memory-only**
// storage for that one entry — the write still succeeds within the page's
// lifetime (matching what the running script observes), it just does not
// survive a reload. This module does not attempt partial/streamed writes or
// eviction policies; it is a playground-scale degraded tier, not a real
// filesystem.

const DEFAULTS_PREFIX = 'tswift:defaults:';
const FS_PREFIX = 'tswift:fs:';
const FS_DIR_MARKER = '\u0000dir\u0000';

// Conservative per-value cap so one large write doesn't trip a
// QuotaExceededError after other keys have already been evicted from the
// in-memory fallback map (best-effort; real quota varies by browser).
const MAX_LOCALSTORAGE_VALUE_CHARS = 512 * 1024;

function hasLocalStorage() {
  try {
    return typeof localStorage !== 'undefined' && localStorage !== null;
  } catch {
    // Some environments (privacy mode, sandboxed iframes) throw just
    // *accessing* `localStorage`, not only on read/write.
    return false;
  }
}

// A namespaced in-memory fallback used both as this module's sole store when
// no `localStorage` exists (e.g. this file loaded under Node for tests) and
// as the per-entry overflow store when a `localStorage` write is rejected.
const memoryFallback = new Map();

function lsGet(key) {
  if (hasLocalStorage()) {
    try {
      const v = localStorage.getItem(key);
      if (v !== null) return v;
    } catch {
      // fall through to memory fallback
    }
  }
  return memoryFallback.has(key) ? memoryFallback.get(key) : null;
}

function lsSet(key, value) {
  if (hasLocalStorage() && value.length <= MAX_LOCALSTORAGE_VALUE_CHARS) {
    try {
      localStorage.setItem(key, value);
      memoryFallback.delete(key);
      return;
    } catch {
      // QuotaExceededError or similar — degrade to memory-only for this key.
    }
  }
  memoryFallback.set(key, value);
  if (hasLocalStorage()) {
    try {
      localStorage.removeItem(key);
    } catch {
      /* best effort */
    }
  }
}

function lsRemove(key) {
  memoryFallback.delete(key);
  if (hasLocalStorage()) {
    try {
      localStorage.removeItem(key);
    } catch {
      /* best effort */
    }
  }
}

function lsKeys(prefix) {
  const keys = new Set();
  if (hasLocalStorage()) {
    try {
      for (let i = 0; i < localStorage.length; i++) {
        const k = localStorage.key(i);
        if (k && k.startsWith(prefix)) keys.add(k);
      }
    } catch {
      /* best effort */
    }
  }
  for (const k of memoryFallback.keys()) {
    if (k.startsWith(prefix)) keys.add(k);
  }
  return keys;
}

// ── tswift.defaults.* ────────────────────────────────────────────────────
//
// Wire schema (`crates/tswift-foundation/src/user_defaults.rs`):
//   tswift.defaults.set(key: String, value: String) -> Void
//     `value` is the JSON encoding of the stored Swift value.
//   tswift.defaults.get(key: String) -> String?
//     the JSON encoding of the stored value, or `nil`.
//   tswift.defaults.remove(key: String) -> Void

function defaultsCall(name, args) {
  switch (name) {
    case 'tswift.defaults.set': {
      const [key, value] = args;
      lsSet(DEFAULTS_PREFIX + key, JSON.stringify(value));
      return 'null';
    }
    case 'tswift.defaults.get': {
      const [key] = args;
      const stored = lsGet(DEFAULTS_PREFIX + key);
      if (stored === null) return 'null';
      // `stored` is `JSON.stringify(value)`; the host fn's declared return
      // type is `String?`, so the reply must itself be a JSON string whose
      // *content* is that stored JSON text (double-encoded, matching the
      // CLI backing — see `crates/tswift-cli/src/defaults.rs`'s test).
      return JSON.stringify(JSON.parse(stored));
    }
    case 'tswift.defaults.remove': {
      const [key] = args;
      lsRemove(DEFAULTS_PREFIX + key);
      return 'null';
    }
    default:
      throw new Error(`unknown host fn \`${name}\``);
  }
}

// ── tswift.fs.* ──────────────────────────────────────────────────────────
//
// Virtual filesystem: every path is a flat string key, normalized (see
// `normalizePath` below) so `.`/`..` segments and repeated `/`s collapse to
// one canonical form — `"/a//b/../c"` and `"/a/c"` address the same virtual
// entry, matching the lexical part of what the CLI's real, unrooted
// filesystem does via the OS path resolver (`crates/tswift-cli/src/fs.rs`).
// `"/"` is an *implicit* directory: it always exists and is always a
// directory, with no stored entry of its own (mirroring how the CLI never
// needs to `mkdir` a filesystem root). A directory is a marker entry
// (`FS_DIR_MARKER`), a file is its base64 content. There is no real
// hierarchy walk — `list` finds entries whose stored path has `path + "/"`
// as a strict prefix with no further `/`, matching a single directory level.

// Resolve `.`/`..` segments and collapse repeated `/`s into one canonical
// path string, without touching a real filesystem (there is none here) or a
// notion of "current directory" — `..` past the root of an absolute path is
// a no-op (matches real path lexical resolution: `/..` is `/`), and `..`
// past the start of a relative path is kept literally (no cwd to resolve
// against). Two paths that normalize to the same string address the same
// virtual entry everywhere in this module (`fsKey`/`fsExists`/`fsIsDirectory`/
// `fsChildren`/`fsParent` all route through this).
function normalizePath(path) {
  const isAbsolute = path.startsWith('/');
  const out = [];
  for (const part of path.split('/')) {
    if (part === '' || part === '.') continue;
    if (part === '..') {
      if (out.length > 0 && out[out.length - 1] !== '..') {
        out.pop();
      } else if (!isAbsolute) {
        out.push('..');
      }
      // Absolute `..` at the root is a no-op — nothing to pop, nothing to push.
      continue;
    }
    out.push(part);
  }
  const joined = out.join('/');
  if (isAbsolute) return '/' + joined;
  return joined === '' ? '.' : joined;
}

function fsKey(path) {
  return FS_PREFIX + normalizePath(path);
}

// The parent directory's normalized path, or `null` if `normPath` is already
// the root (which has no parent). A top-level entry's parent is the
// implicit root (`''` for a relative top-level entry, `'/'` for an absolute
// one) — both are treated as "always exists" by `fsParentExists`.
function fsParent(normPath) {
  if (normPath === '/') return null;
  const idx = normPath.lastIndexOf('/');
  if (idx === -1) return '';
  if (idx === 0) return '/';
  return normPath.slice(0, idx);
}

function fsParentExists(normPath) {
  const parent = fsParent(normPath);
  if (parent === null) return true; // root has no parent to require
  if (parent === '' || parent === '/') return true; // implicit root directory
  return fsIsDirectory(parent);
}

function thrown(message) {
  return JSON.stringify({ $thrown: message });
}

function fsExists(path) {
  const norm = normalizePath(path);
  if (norm === '/') return true; // root is an implicit directory
  return lsGet(FS_PREFIX + norm) !== null;
}

function fsIsDirectory(path) {
  const norm = normalizePath(path);
  if (norm === '/') return true; // root is an implicit directory
  return lsGet(FS_PREFIX + norm) === FS_DIR_MARKER;
}

function fsChildren(path) {
  const norm = normalizePath(path);
  // Root's own key is `FS_PREFIX + '/'` — do NOT special-case it to `''`
  // here, that was the source of the `//`-prefix bug (every entry's key
  // already starts with a single `/`, so prefixing `'/' + '/'` matched
  // nothing and `list('/')` always came back empty).
  const prefix = norm === '/' ? FS_PREFIX + '/' : FS_PREFIX + norm + '/';
  const names = new Set();
  for (const k of lsKeys(FS_PREFIX)) {
    if (!k.startsWith(prefix) || k === fsKey(norm)) continue;
    const rest = k.slice(prefix.length);
    if (rest.length === 0) continue;
    names.add(rest.split('/')[0]);
  }
  return [...names].sort();
}

// A JS port of `tswift_core::base64::decode`'s *validation* rules (see
// `crates/tswift-core/src/base64.rs`) — this module never needs the decoded
// bytes (file content crosses the wire, and is stored, as base64 text
// as-is), only whether `content` is a well-formed base64 string, matching
// the CLI's `tswift.fs.write` (which decodes and returns `false` on `None`
// rather than accepting arbitrary strings as "content"). Whitespace is
// stripped first (Foundation's lenient `Data(base64Encoded:)` mode); the
// cleaned length must be a whole number of 4-char groups, and `=` padding
// is only valid as a trailing run in the final group.
function isValidBase64(content) {
  if (typeof content !== 'string') return false;
  const cleaned = content.replace(/\s+/g, '');
  if (cleaned.length % 4 !== 0) return false;
  const chunkCount = cleaned.length / 4;
  const bodyRe = /^[A-Za-z0-9+/]*$/;
  for (let i = 0; i < chunkCount; i++) {
    const chunk = cleaned.slice(i * 4, i * 4 + 4);
    const pad = (chunk.match(/=*$/) || [''])[0].length;
    if (pad > 0 && (i !== chunkCount - 1 || pad > 2)) return false;
    const body = chunk.slice(0, 4 - pad);
    if (!bodyRe.test(body)) return false;
  }
  return true;
}

// Shared destination-parent validation for `copy`/`move`, matching the CLI
// oracle: `fs::copy`/`fs::rename` fail with ENOENT if the destination's
// parent directory doesn't exist, or ENOTDIR if that parent exists but is a
// plain file (can't create an entry "inside" a file). Returns a thrown-error
// JSON string (with `verb`-specific wording, matching the copy/move-specific
// "already exists"/"no such file or directory" checks each op already makes)
// or `null` if the destination's parent is fine.
function fsDestinationParentError(verb, from, to) {
  const parent = fsParent(normalizePath(to));
  if (parent === null || parent === '' || parent === '/') return null; // implicit root, always exists
  if (!fsExists(parent)) {
    return thrown(`couldn\u2019t ${verb} \u201c${from}\u201d to \u201c${to}\u201d: no such file or directory`);
  }
  if (!fsIsDirectory(parent)) {
    return thrown(`couldn\u2019t ${verb} \u201c${from}\u201d to \u201c${to}\u201d: not a directory`);
  }
  return null;
}

function fsCall(name, args) {
  switch (name) {
    case 'tswift.fs.exists': {
      const [path] = args;
      return JSON.stringify(fsExists(path));
    }
    case 'tswift.fs.isDirectory': {
      const [path] = args;
      return JSON.stringify(fsIsDirectory(path));
    }
    case 'tswift.fs.read': {
      const [path] = args;
      const v = lsGet(fsKey(path));
      if (v === null || v === FS_DIR_MARKER) return 'null';
      return JSON.stringify(v);
    }
    case 'tswift.fs.list': {
      const [path] = args;
      if (!fsExists(path)) {
        return thrown(`couldn\u2019t list \u201c${path}\u201d: no such file or directory`);
      }
      if (!fsIsDirectory(path)) {
        return thrown(`couldn\u2019t list \u201c${path}\u201d: not a directory`);
      }
      return JSON.stringify(fsChildren(path));
    }
    case 'tswift.fs.mkdir': {
      const [path, intermediate] = args;
      if (fsExists(path) && !intermediate) {
        return thrown(`couldn\u2019t create directory \u201c${path}\u201d: file exists`);
      }
      if (intermediate) {
        // Matches `fs::create_dir_all`: walk the path component by
        // component. A component that already exists as a *file* blocks
        // everything beneath it — attempting to traverse through it fails
        // with "not a directory" (nothing gets created past that point),
        // while the *final* component already existing as a file fails
        // with "file exists" instead (matching the plain, non-intermediate
        // `mkdir` check above — `create_dir_all` on a path that is itself
        // an existing file reports `EEXIST`, not `ENOTDIR`).
        const norm = normalizePath(path);
        const isAbsolute = norm.startsWith('/');
        const parts = norm.split('/').filter(Boolean);
        let cur = '';
        for (let i = 0; i < parts.length; i++) {
          cur = cur === '' ? (isAbsolute ? `/${parts[i]}` : parts[i]) : `${cur}/${parts[i]}`;
          const key = FS_PREFIX + cur;
          const existing = lsGet(key);
          const isLast = i === parts.length - 1;
          if (existing === null) {
            lsSet(key, FS_DIR_MARKER);
          } else if (existing !== FS_DIR_MARKER) {
            return isLast
              ? thrown(`couldn\u2019t create directory \u201c${path}\u201d: file exists`)
              : thrown(`couldn\u2019t create directory \u201c${path}\u201d: not a directory`);
          }
          // `existing === FS_DIR_MARKER`: already a directory, idempotent no-op.
        }
      } else {
        lsSet(fsKey(path), FS_DIR_MARKER);
      }
      return 'null';
    }
    case 'tswift.fs.remove': {
      const [path] = args;
      if (!fsExists(path)) {
        return thrown(`couldn\u2019t remove \u201c${path}\u201d: no such file or directory`);
      }
      if (fsIsDirectory(path)) {
        for (const name of fsChildren(path)) {
          fsCall('tswift.fs.remove', [`${normalizePath(path)}/${name}`]);
        }
      }
      lsRemove(fsKey(path));
      return 'null';
    }
    case 'tswift.fs.write': {
      const [path, content, atomically] = args;
      void atomically; // the virtual fs has no partial-write race to guard against
      const norm = normalizePath(path);
      // Matches the CLI's native backing (`fs::write` semantics, see
      // `crates/tswift-cli/src/fs.rs`): base64-decode `content` (returning
      // `false` — not accepting an arbitrary string — on invalid input),
      // refuse to `createFile` over an existing directory, and require the
      // parent directory to already exist (no implicit `mkdir -p`).
      if (!isValidBase64(content)) return JSON.stringify(false);
      if (fsIsDirectory(norm)) return JSON.stringify(false);
      if (!fsParentExists(norm)) return JSON.stringify(false);
      lsSet(fsKey(norm), content);
      return JSON.stringify(true);
    }
    case 'tswift.fs.copy': {
      const [from, to] = args;
      if (fsExists(to)) {
        return thrown(
          `couldn\u2019t copy \u201c${from}\u201d to \u201c${to}\u201d: an item with the same name already exists at the destination.`,
        );
      }
      if (!fsExists(from)) {
        return thrown(`couldn\u2019t copy \u201c${from}\u201d to \u201c${to}\u201d: no such file or directory`);
      }
      const copyParentErr = fsDestinationParentError('copy', from, to);
      if (copyParentErr) return copyParentErr;
      if (fsIsDirectory(from)) {
        lsSet(fsKey(to), FS_DIR_MARKER);
        for (const name of fsChildren(from)) {
          fsCall('tswift.fs.copy', [`${normalizePath(from)}/${name}`, `${normalizePath(to)}/${name}`]);
        }
      } else {
        lsSet(fsKey(to), lsGet(fsKey(from)));
      }
      return 'null';
    }
    case 'tswift.fs.move': {
      const [from, to] = args;
      if (fsExists(to)) {
        return thrown(
          `couldn\u2019t move \u201c${from}\u201d to \u201c${to}\u201d: an item with the same name already exists at the destination.`,
        );
      }
      const moveParentErr = fsDestinationParentError('move', from, to);
      if (moveParentErr) return moveParentErr;
      const copyResult = fsCall('tswift.fs.copy', [from, to]);
      if (JSON.parse(copyResult) && JSON.parse(copyResult).$thrown) return copyResult;
      fsCall('tswift.fs.remove', [from]);
      return 'null';
    }
    default:
      throw new Error(`unknown host fn \`${name}\``);
  }
}

// ── Public entry point ──────────────────────────────────────────────────

/// The set of host-service namespaces this module backs. Feed straight into
/// `globalThis.tswiftHostServices` (see `installTSwiftHostServices` below).
export const TSWIFT_HOST_SERVICE_NAMESPACES = ['tswift.defaults', 'tswift.fs'];

/**
 * Dispatch one `tswiftHost(name, argsJson)` call to whichever service owns
 * `name`'s namespace. Returns a JSON string (see `crates/tswift-core/src/
 * host_bridge.rs`'s `HostCallHandler` contract) or throws to signal a
 * host-side (non-catchable) failure.
 */
export function tswiftHostServiceCall(name, argsJson) {
  const args = JSON.parse(argsJson);
  if (name.startsWith('tswift.defaults.')) return defaultsCall(name, args);
  if (name.startsWith('tswift.fs.')) return fsCall(name, args);
  throw new Error(`unknown host fn \`${name}\``);
}

/**
 * Wire `globalThis.tswiftHostServices` (declares the two namespaces backed
 * by this module) and `globalThis.tswiftHost` (the dispatch hook) so the
 * `tswift-wasm` runtime backs `UserDefaults`/`FileManager` for any script it
 * runs. Composes with any function-specific host functions a page also
 * registers via `registerHostFunction`: if `globalThis.tswiftHost` is already
 * a function when this runs, it is chained as a fallback for names this
 * module does not own.
 */
export function installTSwiftHostServices() {
  const existingHook =
    typeof globalThis.tswiftHost === 'function' ? globalThis.tswiftHost : null;

  const existingServices = Array.isArray(globalThis.tswiftHostServices)
    ? globalThis.tswiftHostServices
    : [];
  globalThis.tswiftHostServices = [
    ...new Set([...existingServices, ...TSWIFT_HOST_SERVICE_NAMESPACES]),
  ];

  globalThis.tswiftHost = (name, argsJson) => {
    if (name.startsWith('tswift.defaults.') || name.startsWith('tswift.fs.')) {
      return tswiftHostServiceCall(name, argsJson);
    }
    if (existingHook) return existingHook(name, argsJson);
    throw new Error(`unknown host fn \`${name}\``);
  };
}

// Exposed for the round-trip smoke test (`website/test/wasm-smoke.mjs`),
// which runs under Node without a real `localStorage` and needs to reset
// state between checks.
export function __resetTSwiftHostServicesForTests() {
  memoryFallback.clear();
  if (hasLocalStorage()) {
    try {
      for (const k of lsKeys(DEFAULTS_PREFIX)) localStorage.removeItem(k);
      for (const k of lsKeys(FS_PREFIX)) localStorage.removeItem(k);
    } catch {
      /* best effort */
    }
  }
}
