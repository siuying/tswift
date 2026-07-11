// Browser-side backing for the `tswift.db.*` host service (SQL over the host
// bridge; see `crates/tswift-swiftdata/src/db.rs` for the op names / tagged
// `DbValue` wire codec this module implements, and
// `docs/adr/0015-db-host-service-wire.md` for the full wire contract).
//
// ## Which sqlite backing option landed (see the slice-8 task brief's
// options a/b/c)
//
// **Option (a): the official npm package `@sqlite.org/sqlite-wasm`.**
// Network access to the npm registry was available in this environment
// (`npm view @sqlite.org/sqlite-wasm` resolved, `npm install` succeeded), so
// per the task's own ordering this is the backing that ships — real SQLite
// compiled to wasm by the SQLite project itself (Apache-2.0 *wrapper* around
// the public-domain SQLite C source; see
// `node_modules/@sqlite.org/sqlite-wasm/README.md`), not a hand-rolled SQL
// engine and not a manually-vendored amalgamation zip. `website/package.json`
// now depends on it directly.
//
// ## Platform tier: web (degraded, and DOCUMENTED as such — mirrors
// `tswift-host-services.js`'s own tier-naming convention)
//
// The playground runs the wasm interpreter on the **main thread**, not a
// worker (see `FullPlayground.astro`'s `initWasm()`). `tswiftHost` is a
// *synchronous* call boundary (ADR-0005), so this module can only use
// sqlite-wasm's synchronous, main-thread-safe API surface:
//
//   - OPFS (real per-file persistent storage) requires sqlite-wasm's
//     worker-hosted async proxy — unavailable here for the same reason
//     `tswift-host-services.js` can't use OPFS's sync access handles on the
//     main thread. Deferred until the interpreter itself moves off the main
//     thread (tracked alongside that module's own OPFS tripwire).
//   - **kvvfs** (`sqlite3.oo1.DB(":localStorage:", ...)`) *is* synchronous
//     and main-thread-safe: it stores the database's pages as ordinary
//     `localStorage` entries. This is the tier this module ships for
//     persistence.
//
// | Platform | `tswift.db.*`                                              |
// |----------|-------------------------------------------------------------|
// | native   | real file-backed SQLite (or `:memory:`), any path           |
// | iOS      | real file-backed SQLite in the app sandbox, any path        |
// | web      | `:memory:` ephemeral, OR one shared `localStorage`-backed   |
// |          | kvvfs database for every other path (see "Path mapping")    |
//
// ### Path mapping (the one real deviation from native/iOS parity)
//
// kvvfs is not a general filesystem — it exposes exactly two fixed named
// stores, `"local"` (persisted, `localStorage`) and `"session"` (tab-lived,
// `sessionStorage`), not arbitrary file paths. So `tswift.db.open(path)`
// maps:
//
//   - `path === ":memory:"` → a fresh, private in-memory database (matches
//     native/iOS exactly: every `:memory:` open is its own database).
//   - any other `path` → the **one shared** kvvfs `"local"` database.
//     Multiple `open()` calls with *different* path strings all resolve to
//     the same underlying storage (same tables visible from every handle) —
//     unlike native/iOS, where different paths are different databases. This
//     is the honest consequence of kvvfs's fixed two-slot design on the main
//     thread; a real per-path store needs the OPFS/worker tier above.
//
// ## Known simplification: 64-bit integers beyond `Number.MAX_SAFE_INTEGER`
//
// SQLite's `INTEGER` storage class is a full 64-bit signed integer; this
// module reads/writes it through JS `number`/`BigInt` (sqlite-wasm returns a
// `BigInt` from `sqlite3_column_type`-guarded reads when a value doesn't fit
// a `Number` losslessly) but the *wire* JSON text is built by hand
// (`JSON.stringify` cannot serialize `BigInt`) precisely so those values
// still round-trip losslessly as JSON integer literals — see `intPayload`/
// `bigIntFromJson` below. Decoding *params* sent in from Swift currently
// parses with `JSON.parse`, whose numbers are IEEE-754 doubles — an inbound
// bind value outside the safe-integer range loses precision before it ever
// reaches this module. Native/iOS do not have this limitation (full `i64`
// both ways). Tripwire: revisit if a script needs to *bind* (not just read)
// a full 64-bit integer param on web.

const DB_NAMESPACE = 'tswift.db';
const MEMORY_PATH = ':memory:';
// kvvfs's fixed "persisted in localStorage" special filename.
const KVVFS_LOCAL_FILENAME = ':localStorage:';

// ── Tagged DbValue codec (mirrors `tswift_swiftdata::db::DbValue`) ───────

const B64_ALPHABET = 'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/';

function base64Encode(bytes) {
  let out = '';
  for (let i = 0; i < bytes.length; i += 3) {
    const b0 = bytes[i];
    const b1 = i + 1 < bytes.length ? bytes[i + 1] : 0;
    const b2 = i + 2 < bytes.length ? bytes[i + 2] : 0;
    const triple = (b0 << 16) | (b1 << 8) | b2;
    out += B64_ALPHABET[(triple >> 18) & 0x3f];
    out += B64_ALPHABET[(triple >> 12) & 0x3f];
    out += i + 1 < bytes.length ? B64_ALPHABET[(triple >> 6) & 0x3f] : '=';
    out += i + 2 < bytes.length ? B64_ALPHABET[triple & 0x3f] : '=';
  }
  return out;
}

function base64Decode(text) {
  const cleaned = text.replace(/\s+/g, '');
  if (cleaned.length % 4 !== 0) return null;
  const bytes = [];
  for (let i = 0; i < cleaned.length; i += 4) {
    const chunk = cleaned.slice(i, i + 4);
    const vals = [];
    let pad = 0;
    for (const ch of chunk) {
      if (ch === '=') {
        pad += 1;
        continue;
      }
      const v = B64_ALPHABET.indexOf(ch);
      if (v === -1 || pad > 0) return null; // '=' must only trail
      vals.push(v);
    }
    if (vals.length < 2) return null;
    const triple = (vals[0] << 18) | (vals[1] << 12) | ((vals[2] ?? 0) << 6) | (vals[3] ?? 0);
    bytes.push((triple >> 16) & 0xff);
    if (vals.length > 2) bytes.push((triple >> 8) & 0xff);
    if (vals.length > 3) bytes.push(triple & 0xff);
  }
  return new Uint8Array(bytes);
}

// A JSON integer literal for an `int` payload, exact for the full `i64`
// range (built by hand — `JSON.stringify` cannot serialize `BigInt`, and a
// plain `Number` loses precision past `Number.MAX_SAFE_INTEGER`).
function intLiteral(n) {
  return typeof n === 'bigint' ? n.toString() : String(Math.trunc(n));
}

// The `real` payload: a bare JSON number literal for a finite,
// non-negative-zero value; a tagged sentinel string for
// `NaN`/`±Infinity`/`-0` (mirrors `tswift_swiftdata::db::real_payload` —
// plain JSON has no `NaN`/`Infinity` literal and `-0` re-parses as `0`).
function realLiteral(d) {
  if (Number.isNaN(d)) return '"nan"';
  if (d === Infinity) return '"inf"';
  if (d === -Infinity) return '"-inf"';
  if (Object.is(d, -0)) return '"-0"';
  return String(d);
}

function realFromSentinelOrNumber(payload) {
  if (typeof payload === 'string') {
    switch (payload) {
      case 'nan':
        return NaN;
      case 'inf':
        return Infinity;
      case '-inf':
        return -Infinity;
      case '-0':
      case '-0.0':
        return -0;
      default:
        throw new Error(`db value tagged \`real\` has unknown sentinel string \`${payload}\``);
    }
  }
  // A tagged-number marker from `parseTaggedNumberJson` (the normal wire
  // path — see that function's doc): a `real` tag accepts either an integer
  // or a float literal, mirroring `DbValue::from_json`'s `Json::Int | Json::Double`
  // acceptance for the `real` tag (an integer payload widens losslessly-ish
  // to `f64`, same as Rust's `*i as f64`).
  if (payload && typeof payload === 'object' && payload.__jsonNumber) {
    return payload.isInt ? Number(payload.big) : payload.value;
  }
  if (typeof payload === 'number') return payload;
  throw new Error('db value tagged `real` must carry a JSON number or sentinel string');
}

// The full `i64` range, as `BigInt` (matches the CLI's `Json::Int(i64)`).
const I64_MIN = -(2n ** 63n);
const I64_MAX = 2n ** 63n - 1n;
// `BigInt`s of `Number.MAX_SAFE_INTEGER`/`MIN_SAFE_INTEGER` — the boundary
// past which a `BigInt` can't be narrowed to `Number` losslessly.
const MAX_SAFE_BIG = BigInt(Number.MAX_SAFE_INTEGER);
const MIN_SAFE_BIG = BigInt(Number.MIN_SAFE_INTEGER);

// Validate + narrow an `int`-tag payload to a JS `Number` (if it fits
// losslessly) or a `BigInt` (if it doesn't), matching the CLI's
// `Json::Int(i64)` strictness: no fractional literals, no values outside the
// `i64` range. Returns `null` if `payload` isn't an integer-shaped JSON
// number at all (wrong type — includes a fractional literal), or `undefined`
// if it parsed as a whole number but doesn't fit in `i64` (out of range).
// Accepts both the tagged-number marker from `parseTaggedNumberJson` (the
// real wire path — full literal-text precision) and a plain `Number` (for
// callers that hand `dbValueFromJson` an already-`JSON.parse`d value, e.g.
// direct unit tests — `JSON.parse` has already collapsed int/float into one
// `number` type there, so only whole-valued numbers are accepted and
// magnitude is checked against `Number`'s own safe-integer range, not a
// literal-text `BigInt` parse).
function intPayloadValue(payload) {
  let big;
  if (payload && typeof payload === 'object' && payload.__jsonNumber) {
    if (!payload.isInt) return null; // fractional literal (e.g. `5.0`): reject.
    big = payload.big;
  } else if (typeof payload === 'number' && Number.isInteger(payload)) {
    big = BigInt(payload);
  } else {
    return null;
  }
  if (big < I64_MIN || big > I64_MAX) return undefined; // out of i64 range.
  return big >= MIN_SAFE_BIG && big <= MAX_SAFE_BIG ? Number(big) : big;
}

// Encode one column/bind value — `{ tag: 'null'|'int'|'real'|'text'|'blob',
// value }` (this module's internal representation, not the wire shape) — to
// its wire JSON text.
function dbValueToJsonText(entry) {
  switch (entry.tag) {
    case 'null':
      return '{"null":null}';
    case 'int':
      return `{"int":${intLiteral(entry.value)}}`;
    case 'real':
      return `{"real":${realLiteral(entry.value)}}`;
    case 'text':
      return `{"text":${JSON.stringify(entry.value)}}`;
    case 'blob':
      return `{"blob":${JSON.stringify(base64Encode(entry.value))}}`;
    default:
      throw new Error(`unknown db value tag \`${entry.tag}\``);
  }
}

// Decode one wire-shape tagged value (already `JSON.parse`d) into this
// module's internal `{ tag, value }` representation.
function dbValueFromJson(node) {
  if (typeof node !== 'object' || node === null || Array.isArray(node)) {
    throw new Error('db value must be a single-key tagged object');
  }
  const keys = Object.keys(node);
  if (keys.length !== 1) throw new Error('db value must have exactly one tag key');
  const [tag] = keys;
  const payload = node[tag];
  switch (tag) {
    case 'null':
      return { tag: 'null', value: null };
    case 'int': {
      const value = intPayloadValue(payload);
      if (value === null) throw new Error('db value tagged `int` must carry a JSON number');
      if (value === undefined) {
        const raw = payload && typeof payload === 'object' && payload.__jsonNumber ? payload.raw : String(payload);
        throw new Error(`db value tagged \`int\` is out of range for a 64-bit integer: ${raw}`);
      }
      return { tag: 'int', value };
    }
    case 'real':
      return { tag: 'real', value: realFromSentinelOrNumber(payload) };
    case 'text':
      if (typeof payload !== 'string') throw new Error('db value tagged `text` must carry a JSON string');
      return { tag: 'text', value: payload };
    case 'blob': {
      if (typeof payload !== 'string') throw new Error('db value tagged `blob` must carry a base64 JSON string');
      const bytes = base64Decode(payload);
      if (bytes === null) throw new Error('db value tagged `blob` has invalid base64');
      return { tag: 'blob', value: bytes };
    }
    default:
      throw new Error(`unknown db value tag \`${tag}\``);
  }
}

// A JSON parser dedicated to `params` text that classifies number literals
// the same way the CLI's own tokenizer does
// (`tswift_core::json`'s `parse_number`): a literal with no `.`/`e`/`E` is
// an *integer* token, kept as exact literal text + a `BigInt` (so it can be
// range-checked against the full `i64` range without precision loss); a
// literal containing any of those is a *float* token, parsed as an
// IEEE-754 `Number` (matching `f64::parse`). Plain `JSON.parse` collapses
// both into the same `number` type and silently rounds a literal past
// `Number.MAX_SAFE_INTEGER` — this parser keeps the distinction alive so
// `dbValueFromJson`'s `int` case (via `intPayloadValue`) can reject a
// fractional or out-of-`i64`-range `int`-tag payload with a structured
// error, exactly like `DbValue::from_json` does, instead of silently
// truncating/rounding it. Number leaves decode to a `{__jsonNumber: true,
// isInt, raw, big?, value?}` marker (see `intPayloadValue`/
// `realFromSentinelOrNumber`); every other JSON shape (object/array/
// string/bool/null) decodes to the ordinary JS equivalent, exactly like
// `JSON.parse`.
function parseTaggedNumberJson(text) {
  let i = 0;
  const n = text.length;
  const fail = (msg) => {
    throw new Error(msg);
  };
  const skipWs = () => {
    while (i < n && (text[i] === ' ' || text[i] === '\t' || text[i] === '\n' || text[i] === '\r')) i++;
  };
  const parseValue = () => {
    skipWs();
    const c = text[i];
    if (c === '{') return parseObject();
    if (c === '[') return parseArray();
    if (c === '"') return parseString();
    if (text.startsWith('true', i)) {
      i += 4;
      return true;
    }
    if (text.startsWith('false', i)) {
      i += 5;
      return false;
    }
    if (text.startsWith('null', i)) {
      i += 4;
      return null;
    }
    if (c === '-' || (c >= '0' && c <= '9')) return parseNumber();
    fail(`invalid params JSON: unexpected character at position ${i}`);
  };
  const parseObject = () => {
    i++; // '{'
    skipWs();
    const obj = {};
    if (text[i] === '}') {
      i++;
      return obj;
    }
    for (;;) {
      skipWs();
      if (text[i] !== '"') fail('invalid params JSON: expected a string key');
      const key = parseString();
      skipWs();
      if (text[i] !== ':') fail('invalid params JSON: expected `:`');
      i++;
      obj[key] = parseValue();
      skipWs();
      if (text[i] === ',') {
        i++;
        continue;
      }
      if (text[i] === '}') {
        i++;
        break;
      }
      fail('invalid params JSON: expected `,` or `}`');
    }
    return obj;
  };
  const parseArray = () => {
    i++; // '['
    skipWs();
    const arr = [];
    if (text[i] === ']') {
      i++;
      return arr;
    }
    for (;;) {
      arr.push(parseValue());
      skipWs();
      if (text[i] === ',') {
        i++;
        continue;
      }
      if (text[i] === ']') {
        i++;
        break;
      }
      fail('invalid params JSON: expected `,` or `]`');
    }
    return arr;
  };
  const parseString = () => {
    const start = i;
    if (text[i] !== '"') fail('invalid params JSON: expected a string');
    i++; // opening quote
    while (i < n && text[i] !== '"') {
      i += text[i] === '\\' ? 2 : 1;
    }
    if (text[i] !== '"') fail('invalid params JSON: unterminated string');
    i++; // closing quote
    return JSON.parse(text.slice(start, i));
  };
  const parseNumber = () => {
    const start = i;
    if (text[i] === '-') i++;
    let isInt = true;
    while (i < n) {
      const ch = text[i];
      if (ch >= '0' && ch <= '9') {
        i++;
        continue;
      }
      if (ch === '.' || ch === 'e' || ch === 'E' || ch === '+' || ch === '-') {
        isInt = false;
        i++;
        continue;
      }
      break;
    }
    const raw = text.slice(start, i);
    if (raw === '' || raw === '-') fail(`invalid params JSON: invalid number literal near position ${start}`);
    if (isInt) {
      let big;
      try {
        big = BigInt(raw);
      } catch {
        fail(`invalid params JSON: invalid integer literal \`${raw}\``);
      }
      return { __jsonNumber: true, isInt: true, raw, big };
    }
    const value = Number(raw);
    if (Number.isNaN(value)) fail(`invalid params JSON: invalid number literal \`${raw}\``);
    return { __jsonNumber: true, isInt: false, raw, value };
  };
  const result = parseValue();
  skipWs();
  if (i !== n) fail('invalid params JSON: trailing characters after JSON value');
  return result;
}

// Decode `params` (a JSON-array-of-tagged-values `String`) into internal
// `{tag,value}` entries.
function decodeParams(paramsText) {
  const parsed = parseTaggedNumberJson(paramsText);
  if (!Array.isArray(parsed)) throw new Error('params must be a JSON array');
  return parsed.map(dbValueFromJson);
}

// Encode `tswift.db.query`'s reply: a JSON array of column-name-keyed
// objects, one per row, with duplicate column names disambiguated exactly
// like `tswift_swiftdata::db::disambiguate_columns` (`a`, `a_1`, `a_2`, …,
// advancing past any real collision).
function encodeRows(rows) {
  const out = rows.map((row) => {
    const used = new Set();
    const next = new Map();
    const pairs = [];
    for (const [name, entry] of row) {
      let key = name;
      if (used.has(key)) {
        let counter = next.get(name) ?? 0;
        let candidate;
        do {
          counter += 1;
          candidate = `${name}_${counter}`;
        } while (used.has(candidate));
        next.set(name, counter);
        key = candidate;
      }
      used.add(key);
      pairs.push(`${JSON.stringify(key)}:${dbValueToJsonText(entry)}`);
    }
    return `{${pairs.join(',')}}`;
  });
  return `[${out.join(',')}]`;
}

function thrown(message) {
  return JSON.stringify({ $thrown: message });
}

// ── SQLite-wasm plumbing ──────────────────────────────────────────────────

let sqlite3Promise = null;

// Lazily load and initialize `@sqlite.org/sqlite-wasm`. Resolves to the
// initialized `Sqlite3Static` module, or throws if the package is missing or
// fails to initialize (e.g. no network to fetch its `.wasm` asset) — the
// caller (`installTSwiftDbHostService`) treats that as "no sqlite module
// present" and does not declare the `tswift.db` capability.
async function loadSqlite3() {
  if (!sqlite3Promise) {
    sqlite3Promise = (async () => {
      const mod = await import('@sqlite.org/sqlite-wasm');
      return mod.default();
    })();
  }
  return sqlite3Promise;
}

// One open connection, keyed by the ascending handle this module mints —
// mirrors `crates/tswift-cli/src/db.rs`'s `DbHandler` (an ascending `i64`
// handle counter + a handle→connection table), just JS-shaped (`Map` instead
// of `Mutex<HashMap<...>>` — the interpreter's single-threaded, ADR-0005, so
// no lock is needed here either).
class DbRegistry {
  constructor(sqlite3) {
    this.sqlite3 = sqlite3;
    this.nextHandle = 1;
    this.conns = new Map();
  }

  open(path) {
    const filename = path === MEMORY_PATH ? MEMORY_PATH : KVVFS_LOCAL_FILENAME;
    const db = new this.sqlite3.oo1.DB(filename, 'c');
    const handle = this.nextHandle++;
    this.conns.set(handle, db);
    return handle;
  }

  get(handle) {
    return this.conns.get(handle);
  }

  close(handle) {
    const db = this.conns.get(handle);
    if (!db) return false;
    this.conns.delete(handle);
    db.close();
    return true;
  }
}

function bindParams(stmt, params) {
  params.forEach((entry, i) => {
    const idx = i + 1; // 1-based, matching SQLite's own `?` numbering.
    switch (entry.tag) {
      case 'null':
        stmt.bind(idx, null);
        break;
      case 'int':
        stmt.bind(idx, entry.value);
        break;
      case 'real':
        stmt.bind(idx, entry.value);
        break;
      case 'text':
        stmt.bind(idx, entry.value);
        break;
      case 'blob':
        stmt.bindAsBlob(idx, entry.value);
        break;
      default:
        throw new Error(`unknown db value tag \`${entry.tag}\``);
    }
  });
}

// Read result column `i` of the statement's current row, tagged by SQLite's
// own storage class (`sqlite3_column_type`) — the same disambiguation the
// native/iOS backings do via `sqlite3_column_type`/`ColumnValue`.
function readColumn(sqlite3, stmt, i) {
  const capi = sqlite3.capi;
  const t = capi.sqlite3_column_type(stmt, i);
  switch (t) {
    case capi.SQLITE_INTEGER:
      return { tag: 'int', value: stmt.get(i) }; // number or BigInt.
    case capi.SQLITE_FLOAT:
      return { tag: 'real', value: stmt.get(i) };
    case capi.SQLITE_TEXT:
      return { tag: 'text', value: stmt.get(i) };
    case capi.SQLITE_BLOB:
      return { tag: 'blob', value: stmt.getBlob(i) };
    default:
      return { tag: 'null', value: null };
  }
}

// `sqlite3.oo1.Database#prepare` throws (rather than returning a null
// statement, as the C API's `sqlite3_prepare_v2` does) for SQL that compiles
// to no statement at all (empty / all-whitespace / comment-only). The
// native/iOS backings treat that as a completed no-op — `execute` reports 0
// rows affected, `query` returns an empty result set (see ADR-0015's "Empty /
// comment-only SQL" section) — so this module special-cases that one message
// to match, rather than surfacing it as a SQL syntax error.
function isEmptyStatementError(err) {
  return err instanceof Error && /Cannot prepare empty SQL/i.test(err.message);
}

function sqliteErrorMessage(err) {
  // `SQLite3Error` (thrown by oo1/capi calls) carries `resultCode` and a
  // `message` shaped like `sqlite3_errmsg()`'s text; format identically to
  // the native/iOS backing's `"SQLite error <code>: <message>"` so a
  // script's caught error text reads the same across platforms.
  const code = err && typeof err.resultCode === 'number' ? err.resultCode : -1;
  const message = err && err.message ? err.message : String(err);
  return `SQLite error ${code}: ${message}`;
}

// Dispatch one `tswift.db.*` call. `registry` is this page's `DbRegistry`.
function dbCall(sqlite3, registry, name, args) {
  const opName = name.slice(DB_NAMESPACE.length + 1);
  switch (opName) {
      case 'open': {
        const [path] = args;
        const handle = registry.open(path);
        return JSON.stringify(handle);
      }
      case 'close': {
        const [handle] = args;
        if (!registry.close(handle)) {
          return thrown(`tswift.db.close: handle ${handle} is not open (already closed, or never opened)`);
        }
        return 'null';
      }
      case 'execute':
      case 'query': {
        const [handle, sql, paramsText] = args;
        let params;
        try {
          params = decodeParams(paramsText);
        } catch (e) {
          return thrown(`${name}: ${e.message}`);
        }
        const db = registry.get(handle);
        if (!db) return thrown(`${name}: handle ${handle} is not open`);
        let stmt;
        try {
          stmt = db.prepare(sql);
        } catch (e) {
          if (isEmptyStatementError(e)) {
            // A no-op statement: `sqlite3_changes`/`sqlite3_last_insert_rowid`
            // are *connection*-level state, not statement-level — SQLite does
            // not reset them just because this particular statement compiled
            // to nothing (`crates/tswift-cli/src/db.rs`'s `OP_EXECUTE` never
            // special-cases the empty-statement case either: it always reports
            // `conn.changes()`/`conn.last_insert_rowid()` verbatim, whatever
            // the *previous* successful DML on this connection left behind).
            // So this must read the connection's real current values, not
            // fabricate zeros — zero is only correct here by coincidence, on
            // a connection where no DML has run yet. `query` has no such
            // state to preserve: an empty result set is always correct.
            if (opName === 'execute') {
              const rowsAffected = sqlite3.capi.sqlite3_changes(db.pointer);
              const lastInsertRowid = sqlite3.capi.sqlite3_last_insert_rowid(db.pointer);
              const reply = `{"rowsAffected":${rowsAffected},"lastInsertRowid":${intLiteral(lastInsertRowid)}}`;
              return JSON.stringify(reply);
            }
            return JSON.stringify('[]');
          }
          return thrown(sqliteErrorMessage(e));
        }
        try {
          bindParams(stmt, params);
          if (opName === 'execute') {
            while (stmt.step()) {
              /* discard rows, matching the CLI's step_to_completion */
            }
            const rowsAffected = sqlite3.capi.sqlite3_changes(db.pointer);
            const lastInsertRowid = sqlite3.capi.sqlite3_last_insert_rowid(db.pointer);
            const reply = `{"rowsAffected":${rowsAffected},"lastInsertRowid":${intLiteral(lastInsertRowid)}}`;
            return JSON.stringify(reply);
          }
          const colCount = stmt.columnCount;
          const colNames = Array.from({ length: colCount }, (_, i) => stmt.getColumnName(i));
          const rows = [];
          while (stmt.step()) {
            const row = colNames.map((colName, i) => [colName, readColumn(sqlite3, stmt, i)]);
            rows.push(row);
          }
          return JSON.stringify(encodeRows(rows));
        } catch (e) {
          return thrown(sqliteErrorMessage(e));
        } finally {
          stmt.finalize();
        }
      }
      case 'begin':
      case 'commit':
      case 'rollback': {
        const [handle] = args;
        const db = registry.get(handle);
        if (!db) return thrown(`tswift.db.${opName}: handle ${handle} is not open`);
        try {
          db.exec(opName.toUpperCase());
        } catch (e) {
          return thrown(sqliteErrorMessage(e));
        }
        return 'null';
      }
      default:
        // A protocol-level failure (unknown op name) — not a catchable
        // `$thrown`, matching `db.rs`'s own `Err` vs. `Ok(thrown(...))` split
        // (data errors are `$thrown`, protocol errors are a bridge `Err`).
        throw new Error(`unknown host fn \`${name}\``);
  }
}

// ── Public entry point ────────────────────────────────────────────────────

/**
 * Load `@sqlite.org/sqlite-wasm`, initialize it, and wire `tswift.db.*` onto
 * `globalThis.tswiftHost` — declaring the `tswift.db` capability only once
 * the sqlite module is actually present and initialized (per the task
 * brief's "declare the namespace only when the module is present" rule). If
 * the package is missing (not installed / build didn't bundle it) or fails
 * to initialize (e.g. no network to fetch its `.wasm` asset), this resolves
 * to `false` and leaves `tswift.db` undeclared — a script attempting to use
 * it sees the ordinary "no host service declared" capability diagnostic
 * (the framework layer's own `HostService`/`Capabilities` gate), not a
 * crash. Composes with `installTSwiftHostServices()` (`tswift.defaults`/
 * `tswift.fs`) and any other page-installed host function exactly like that
 * module does (chains the pre-existing `globalThis.tswiftHost` as a
 * fallback for names it does not own).
 *
 * Call once per page, before running any script that touches `tswift.db`
 * (e.g. `await installTSwiftDbHostService()` alongside
 * `installTSwiftHostServices()` in `initWasm()`).
 */
export async function installTSwiftDbHostService() {
  let sqlite3;
  try {
    sqlite3 = await loadSqlite3();
  } catch {
    return false;
  }
  const registry = new DbRegistry(sqlite3);

  const existingHook = typeof globalThis.tswiftHost === 'function' ? globalThis.tswiftHost : null;

  const existingServices = Array.isArray(globalThis.tswiftHostServices) ? globalThis.tswiftHostServices : [];
  globalThis.tswiftHostServices = [...new Set([...existingServices, DB_NAMESPACE])];

  globalThis.tswiftHost = (name, argsJson) => {
    if (name.startsWith(`${DB_NAMESPACE}.`)) {
      const args = JSON.parse(argsJson);
      return dbCall(sqlite3, registry, name, args);
    }
    if (existingHook) return existingHook(name, argsJson);
    throw new Error(`unknown host fn \`${name}\``);
  };

  return true;
}

// Exposed for tests: dispatch straight against an already-initialized
// `sqlite3`/`registry` pair without touching `globalThis`.
export function __makeTestDbDispatcher(sqlite3) {
  const registry = new DbRegistry(sqlite3);
  return (name, argsJson) => dbCall(sqlite3, registry, name, JSON.parse(argsJson));
}

// Exposed for pure-codec unit tests (no sqlite3 module needed).
export const __testing = {
  base64Encode,
  base64Decode,
  intLiteral,
  realLiteral,
  realFromSentinelOrNumber,
  dbValueToJsonText,
  dbValueFromJson,
  decodeParams,
  encodeRows,
  parseTaggedNumberJson,
  intPayloadValue,
};
