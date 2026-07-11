// Tests for `tswift.db.*`'s web backing (`src/lib/tswift-db-host-service.js`).
//
// Two tiers, matching the module's own "option (a): @sqlite.org/sqlite-wasm,
// declared only when actually present" contract:
//
//   1. Pure codec unit tests (`__testing`) — no sqlite3 module needed, run
//      unconditionally.
//   2. A full wire-level round trip against the *real* sqlite-wasm module
//      (open/execute/query/close, typed values, transactions, errors,
//      empty-SQL no-op, duplicate columns, kvvfs persistence across separate
//      `DbRegistry` instances) — mirrors `crates/tswift-cli/src/db.rs`'s own
//      test suite at the wire level. If `@sqlite.org/sqlite-wasm` cannot be
//      loaded (not installed, or no network to fetch its `.wasm` asset —
//      see the module's own doc comment), these degrade to a single skip
//      notice rather than failing the whole suite, matching the "declare/
//      test only when the module is present" rule the task brief sets for
//      the production install path too.
//
// There is currently no Swift-facing `tswift.db` API (no `@Model`/SQL
// surface — see `docs/adr/0015-db-host-service-wire.md`: that is explicitly
// future work), so unlike `tswift.defaults`/`tswift.fs` (exercised through
// real Swift source via `runSwift` in `wasm-smoke.mjs`), this suite drives
// the wire directly (`tswiftHost('tswift.db.open', ...)`), exactly like the
// native CLI's own `db.rs` tests do against `HostCallHandler::call`.

import process from 'node:process';

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
async function checkAsync(name, fn) {
  try {
    await fn();
    console.log(`  ok   ${name}`);
  } catch (err) {
    failures += 1;
    console.error(`  FAIL ${name}\n       ${err.stack || err.message}`);
  }
}
function assert(cond, msg) {
  if (!cond) throw new Error(msg);
}

const {
  __testing,
  __makeTestDbDispatcher,
  installTSwiftDbHostService,
} = await import(new URL('../src/lib/tswift-db-host-service.js', import.meta.url));

// ── 1. Pure codec unit tests (no sqlite3 module needed) ──────────────────

check('base64 round-trips arbitrary bytes', () => {
  const bytes = new Uint8Array([0, 1, 2, 255, 254, 253, 128, 127]);
  const text = __testing.base64Encode(bytes);
  const back = __testing.base64Decode(text);
  assert(back && back.length === bytes.length, 'length mismatch');
  for (let i = 0; i < bytes.length; i++) assert(back[i] === bytes[i], `byte ${i} mismatch`);
});

check('base64Decode rejects malformed input', () => {
  assert(__testing.base64Decode('not base64!!') === null);
  assert(__testing.base64Decode('abc') === null); // not a multiple of 4
});

check('every DbValue tag round-trips through JSON text', () => {
  const entries = [
    { tag: 'null', value: null },
    { tag: 'int', value: -42 },
    { tag: 'text', value: 'hi' },
    { tag: 'blob', value: new Uint8Array([0, 1, 2, 255]) },
  ];
  for (const entry of entries) {
    const text = __testing.dbValueToJsonText(entry);
    const back = __testing.dbValueFromJson(JSON.parse(text));
    if (entry.tag === 'blob') {
      assert(back.tag === 'blob' && back.value.length === entry.value.length, `blob mismatch: ${text}`);
    } else {
      assert(back.tag === entry.tag && back.value === entry.value, `mismatch: ${text}`);
    }
  }
});

check('real 5.0 does not collapse to an int-looking payload losing the `real` tag', () => {
  const text = __testing.dbValueToJsonText({ tag: 'real', value: 5.0 });
  const back = __testing.dbValueFromJson(JSON.parse(text));
  assert(back.tag === 'real' && back.value === 5, `${text}`);
});

check('non-finite and signed-zero reals round-trip losslessly (like tswift_swiftdata::db)', () => {
  for (const [value, label] of [
    [NaN, 'nan'],
    [Infinity, 'inf'],
    [-Infinity, '-inf'],
    [-0, '-0'],
  ]) {
    const text = __testing.dbValueToJsonText({ tag: 'real', value });
    assert(JSON.parse(text), `must be valid JSON: ${text} (${label})`);
    const back = __testing.dbValueFromJson(JSON.parse(text));
    if (Number.isNaN(value)) {
      assert(Number.isNaN(back.value), `expected NaN, got ${back.value}`);
    } else {
      assert(Object.is(back.value, value), `expected ${value} (${label}), got ${back.value}`);
    }
  }
});

check('duplicate column names are disambiguated (a, a_1, a_2)', () => {
  const rows = [
    [
      ['a', { tag: 'int', value: 1 }],
      ['a', { tag: 'int', value: 2 }],
      ['a', { tag: 'int', value: 3 }],
      ['b', { tag: 'int', value: 4 }],
    ],
  ];
  const text = __testing.encodeRows(rows);
  const parsed = JSON.parse(text);
  assert(Object.keys(parsed[0]).join(',') === 'a,a_1,a_2,b', JSON.stringify(parsed));
});

check('decodeParams rejects malformed JSON with a plain Error (caller wraps it as $thrown)', () => {
  assert.throws?.(() => __testing.decodeParams('not json'));
  try {
    __testing.decodeParams('not json');
    throw new Error('expected decodeParams to throw');
  } catch (e) {
    assert(e instanceof Error);
  }
});

// ── `int`-tag strictness (matches `DbValue::from_json`'s `Json::Int(i64)`) ─

check('decodeParams rejects a fractional `int`-tag payload (`5.0`), not silent truncation', () => {
  try {
    __testing.decodeParams('[{"int":5.0}]');
    throw new Error('expected decodeParams to throw');
  } catch (e) {
    assert(e instanceof Error && /must carry a JSON number/.test(e.message), e.message);
  }
});

check('decodeParams rejects an `int`-tag payload written with an exponent (`1e2`)', () => {
  try {
    __testing.decodeParams('[{"int":1e2}]');
    throw new Error('expected decodeParams to throw');
  } catch (e) {
    assert(e instanceof Error && /must carry a JSON number/.test(e.message), e.message);
  }
});

check('decodeParams rejects an `int`-tag payload one past the i64 range on both ends', () => {
  for (const raw of ['9223372036854775808', '-9223372036854775809']) {
    try {
      __testing.decodeParams(`[{"int":${raw}}]`);
      throw new Error(`expected decodeParams to throw for ${raw}`);
    } catch (e) {
      assert(e instanceof Error && /out of range/.test(e.message), `${raw}: ${e.message}`);
    }
  }
});

check('decodeParams accepts the exact i64 boundary values losslessly', () => {
  const [minEntry] = __testing.decodeParams('[{"int":-9223372036854775808}]');
  const [maxEntry] = __testing.decodeParams('[{"int":9223372036854775807}]');
  assert(minEntry.tag === 'int' && minEntry.value === -9223372036854775808n, `${minEntry.tag} ${minEntry.value}`);
  assert(maxEntry.tag === 'int' && maxEntry.value === 9223372036854775807n, `${maxEntry.tag} ${maxEntry.value}`);
});

check('decodeParams keeps a small `int`-tag payload as a plain Number, not BigInt', () => {
  const [entry] = __testing.decodeParams('[{"int":42}]');
  assert(entry.tag === 'int' && entry.value === 42 && typeof entry.value === 'number', JSON.stringify(entry));
});

check('parseTaggedNumberJson preserves full i64 literal-text precision that JSON.parse loses', () => {
  const raw = '9223372036854775807'; // Number.MAX_SAFE_INTEGER-exceeding i64::MAX.
  // Sanity: plain `JSON.parse` really does round this literal (double
  // precision runs out well before the full i64 range — both this literal
  // and one one less than it round to the exact same `Number`) — the whole
  // point of `parseTaggedNumberJson` existing.
  assert(JSON.parse(raw) === JSON.parse('9223372036854775806'), 'expected plain JSON.parse to lose precision on this literal');
  const node = __testing.parseTaggedNumberJson(raw);
  assert(node.__jsonNumber === true && node.isInt === true && node.big === 9223372036854775807n, `${node.isInt} ${node.big}`);
});

// ── 2. Full wire-level round trip against real sqlite-wasm ───────────────

let sqlite3 = null;
try {
  const mod = await import('@sqlite.org/sqlite-wasm');
  sqlite3 = await mod.default();
} catch (err) {
  console.log(`  SKIP sqlite-wasm-backed tswift.db.* tests (module unavailable: ${err.message})`);
}

if (sqlite3) {
  const j = (...args) => JSON.stringify(args);
  const unwrapJsonString = (reply) => JSON.parse(reply);

  check('open/execute/query/close round-trips a real table', () => {
    const call = __makeTestDbDispatcher(sqlite3);
    const handle = JSON.parse(call('tswift.db.open', j(':memory:')));
    const create = call(
      'tswift.db.execute',
      j(handle, 'CREATE TABLE t (id INTEGER PRIMARY KEY, name TEXT)', '[]'),
    );
    assert(JSON.parse(unwrapJsonString(create)).rowsAffected === 0, create);

    const insert = call(
      'tswift.db.execute',
      j(handle, 'INSERT INTO t (name) VALUES (?)', JSON.stringify([{ text: 'alice' }])),
    );
    const insertResult = JSON.parse(unwrapJsonString(insert));
    assert(insertResult.rowsAffected === 1 && insertResult.lastInsertRowid === 1, insert);

    const selected = call('tswift.db.query', j(handle, 'SELECT id, name FROM t', '[]'));
    const rows = JSON.parse(unwrapJsonString(selected));
    assert(rows.length === 1, selected);
    assert(rows[0].id.int === 1 && rows[0].name.text === 'alice', selected);

    assert(call('tswift.db.close', j(handle)) === 'null');
  });

  check('typed values round-trip including blob and null', () => {
    const call = __makeTestDbDispatcher(sqlite3);
    const handle = JSON.parse(call('tswift.db.open', j(':memory:')));
    call('tswift.db.execute', j(handle, 'CREATE TABLE t (i INTEGER, r REAL, s TEXT, b BLOB, n TEXT)', '[]'));
    const blobB64 = __testing.base64Encode(new Uint8Array([9, 8, 7]));
    const params = JSON.stringify([{ int: 7 }, { real: 2.5 }, { text: 'hi' }, { blob: blobB64 }, { null: null }]);
    call('tswift.db.execute', j(handle, 'INSERT INTO t VALUES (?, ?, ?, ?, ?)', params));
    const selected = JSON.parse(unwrapJsonString(call('tswift.db.query', j(handle, 'SELECT * FROM t', '[]'))));
    const row = selected[0];
    assert(row.i.int === 7, JSON.stringify(row));
    assert(row.r.real === 2.5, JSON.stringify(row));
    assert(row.s.text === 'hi', JSON.stringify(row));
    assert(row.b.blob === blobB64, JSON.stringify(row));
    assert(row.n.null === null, JSON.stringify(row));
  });

  check('transactions commit and roll back', () => {
    const call = __makeTestDbDispatcher(sqlite3);
    const handle = JSON.parse(call('tswift.db.open', j(':memory:')));
    call('tswift.db.execute', j(handle, 'CREATE TABLE t (v INTEGER)', '[]'));

    assert(call('tswift.db.begin', j(handle)) === 'null');
    call('tswift.db.execute', j(handle, 'INSERT INTO t VALUES (1)', '[]'));
    assert(call('tswift.db.rollback', j(handle)) === 'null');
    let count = JSON.parse(unwrapJsonString(call('tswift.db.query', j(handle, 'SELECT COUNT(*) AS c FROM t', '[]'))));
    assert(count[0].c.int === 0, JSON.stringify(count));

    assert(call('tswift.db.begin', j(handle)) === 'null');
    call('tswift.db.execute', j(handle, 'INSERT INTO t VALUES (1)', '[]'));
    assert(call('tswift.db.commit', j(handle)) === 'null');
    count = JSON.parse(unwrapJsonString(call('tswift.db.query', j(handle, 'SELECT COUNT(*) AS c FROM t', '[]'))));
    assert(count[0].c.int === 1, JSON.stringify(count));
  });

  check('invalid/double-closed handles are a catchable $thrown, not a crash', () => {
    const call = __makeTestDbDispatcher(sqlite3);
    const handle = JSON.parse(call('tswift.db.open', j(':memory:')));
    assert(call('tswift.db.close', j(handle)) === 'null');
    const reply = JSON.parse(call('tswift.db.close', j(handle)));
    assert(typeof reply.$thrown === 'string', JSON.stringify(reply));

    const badHandleReply = JSON.parse(call('tswift.db.query', j(999, 'SELECT 1', '[]')));
    assert(typeof badHandleReply.$thrown === 'string', JSON.stringify(badHandleReply));
  });

  check('SQL syntax error is a structured $thrown', () => {
    const call = __makeTestDbDispatcher(sqlite3);
    const handle = JSON.parse(call('tswift.db.open', j(':memory:')));
    const reply = JSON.parse(call('tswift.db.execute', j(handle, 'NOT VALID SQL', '[]')));
    assert(typeof reply.$thrown === 'string' && reply.$thrown.includes('SQLite error'), JSON.stringify(reply));
  });

  check('malformed params payload is $thrown, not an uncaught throw', () => {
    const call = __makeTestDbDispatcher(sqlite3);
    const handle = JSON.parse(call('tswift.db.open', j(':memory:')));
    const reply = JSON.parse(call('tswift.db.execute', j(handle, 'SELECT 1', 'not valid json')));
    assert(typeof reply.$thrown === 'string', JSON.stringify(reply));
  });

  check('empty and comment-only SQL is a no-op, not a syntax error', () => {
    const call = __makeTestDbDispatcher(sqlite3);
    const handle = JSON.parse(call('tswift.db.open', j(':memory:')));
    const execReply = JSON.parse(unwrapJsonString(call('tswift.db.execute', j(handle, '-- nothing here', '[]'))));
    assert(execReply.rowsAffected === 0, JSON.stringify(execReply));
    const queryReply = JSON.parse(unwrapJsonString(call('tswift.db.query', j(handle, '   ', '[]'))));
    assert(Array.isArray(queryReply) && queryReply.length === 0, JSON.stringify(queryReply));
  });

  check('empty-SQL execute reports the connection\'s real current changes/lastInsertRowid, not fabricated zeros', () => {
    // `sqlite3_changes`/`sqlite3_last_insert_rowid` are *connection*-level
    // state (see the module's own doc at the empty-statement branch); an
    // empty/comment-only `execute` right after a real INSERT must report
    // that INSERT's numbers verbatim, not reset to 0 — exactly like
    // `crates/tswift-cli/src/db.rs`'s `OP_EXECUTE` (which never
    // special-cases the no-op-statement case at all).
    const call = __makeTestDbDispatcher(sqlite3);
    const handle = JSON.parse(call('tswift.db.open', j(':memory:')));
    call('tswift.db.execute', j(handle, 'CREATE TABLE t (id INTEGER PRIMARY KEY)', '[]'));
    const insert = JSON.parse(unwrapJsonString(call('tswift.db.execute', j(handle, 'INSERT INTO t DEFAULT VALUES', '[]'))));
    assert(insert.rowsAffected === 1 && insert.lastInsertRowid === 1, JSON.stringify(insert));

    const noop = JSON.parse(unwrapJsonString(call('tswift.db.execute', j(handle, '-- nothing here', '[]'))));
    assert(
      noop.rowsAffected === insert.rowsAffected && noop.lastInsertRowid === insert.lastInsertRowid,
      `expected the no-op execute to echo the prior INSERT's real state, got ${JSON.stringify(noop)}`,
    );
  });

  check('duplicate result columns are disambiguated end-to-end', () => {
    const call = __makeTestDbDispatcher(sqlite3);
    const handle = JSON.parse(call('tswift.db.open', j(':memory:')));
    call('tswift.db.execute', j(handle, 'CREATE TABLE t (a INTEGER)', '[]'));
    call('tswift.db.execute', j(handle, 'INSERT INTO t VALUES (5)', '[]'));
    const rows = JSON.parse(unwrapJsonString(call('tswift.db.query', j(handle, 'SELECT a, a, a FROM t', '[]'))));
    assert(Object.keys(rows[0]).join(',') === 'a,a_1,a_2', JSON.stringify(rows));
  });

  // kvvfs persistence: requires a `localStorage`, absent under Node — shim
  // one exactly like `wasm-smoke.mjs` does for the `tswift.fs`/`tswift.defaults`
  // web tier tests. This proves the "any non-`:memory:` path shares the one
  // persisted kvvfs `local` store" behavior documented at the top of the module
  // (a real, honest deviation from native/iOS's per-path files — see the
  // module doc's "Path mapping" section).
  check('a non-:memory: path persists via kvvfs across separate DbRegistry instances', () => {
    if (typeof globalThis.localStorage === 'undefined') {
      const store = new Map();
      globalThis.localStorage = {
        getItem: (k) => (store.has(k) ? store.get(k) : null),
        setItem: (k, v) => store.set(k, String(v)),
        removeItem: (k) => store.delete(k),
        get length() {
          return store.size;
        },
        key: (i) => [...store.keys()][i] ?? null,
      };
    }
    let call = __makeTestDbDispatcher(sqlite3);
    const h1 = JSON.parse(call('tswift.db.open', j('some/path.db')));
    call('tswift.db.execute', j(h1, 'CREATE TABLE IF NOT EXISTS t (v TEXT)', '[]'));
    call('tswift.db.execute', j(h1, 'DELETE FROM t', '[]'));
    call('tswift.db.execute', j(h1, 'INSERT INTO t VALUES (?)', JSON.stringify([{ text: 'persisted' }])));
    call('tswift.db.close', j(h1));

    // A fresh dispatcher/registry, a *different* path string, same
    // localStorage: still sees the row (documents the shared-store behavior).
    call = __makeTestDbDispatcher(sqlite3);
    const h2 = JSON.parse(call('tswift.db.open', j('different/path.db')));
    const rows = JSON.parse(unwrapJsonString(call('tswift.db.query', j(h2, 'SELECT v FROM t', '[]'))));
    assert(rows.length === 1 && rows[0].v.text === 'persisted', JSON.stringify(rows));
    // Clean up so this test is repeatable / doesn't leak into other runs.
    call('tswift.db.execute', j(h2, 'DELETE FROM t', '[]'));
    call('tswift.db.close', j(h2));
  });

  await checkAsync('installTSwiftDbHostService wires globalThis.tswiftHost and tswiftHostServices', async () => {
    delete globalThis.tswiftHost;
    globalThis.tswiftHostServices = undefined;
    const ok = await installTSwiftDbHostService();
    assert(ok === true, 'expected installTSwiftDbHostService to resolve true when sqlite3 is available');
    assert(globalThis.tswiftHostServices.includes('tswift.db'), JSON.stringify(globalThis.tswiftHostServices));
    assert(typeof globalThis.tswiftHost === 'function');
    const handle = JSON.parse(globalThis.tswiftHost('tswift.db.open', j(':memory:')));
    assert(typeof handle === 'number');
    globalThis.tswiftHost('tswift.db.close', j(handle));
  });

  await checkAsync('installTSwiftDbHostService composes with a pre-existing tswiftHost hook', async () => {
    delete globalThis.tswiftHost;
    globalThis.tswiftHostServices = ['tswift.fs'];
    let sawFsCall = false;
    globalThis.tswiftHost = (name) => {
      if (name.startsWith('tswift.fs.')) {
        sawFsCall = true;
        return 'null';
      }
      throw new Error(`unexpected call: ${name}`);
    };
    await installTSwiftDbHostService();
    assert(globalThis.tswiftHostServices.includes('tswift.fs'), JSON.stringify(globalThis.tswiftHostServices));
    assert(globalThis.tswiftHostServices.includes('tswift.db'), JSON.stringify(globalThis.tswiftHostServices));
    globalThis.tswiftHost('tswift.fs.exists', j('/x'));
    assert(sawFsCall, 'expected the pre-existing tswift.fs hook to still be reachable');
    const handle = JSON.parse(globalThis.tswiftHost('tswift.db.open', j(':memory:')));
    globalThis.tswiftHost('tswift.db.close', j(handle));
  });
}

if (failures > 0) {
  console.error(`\n${failures} tswift.db.* web host-service check(s) failed`);
  process.exit(1);
}
console.log('\nall tswift.db.* web host-service checks passed');
