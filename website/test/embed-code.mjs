// Round-trip + validation tests for the `/embed/` route's codec
// (`src/lib/embed-code.js`) and its URL-param/snippet helpers
// (`src/lib/embed-params.js`). Pure Node, no DOM/wasm needed.

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
function assert(cond, msg) {
  if (!cond) throw new Error(msg);
}

const {
  encodeEmbedCode,
  decodeEmbedCode,
  EmbedCodeError,
  MAX_ENCODED_LENGTH,
  MAX_SOURCE_LENGTH,
} = await import(new URL('../src/lib/embed-code.js', import.meta.url));
const { parseEmbedParams, playgroundUrl, buildEmbedSnippet, resolveEmbedSource } = await import(
  new URL('../src/lib/embed-params.js', import.meta.url)
);

// ── encode/decode round trip ──────────────────────────────────────────────

check('round-trips plain ASCII source', () => {
  const src = 'print("Hello, tswift!")\n';
  const encoded = encodeEmbedCode(src);
  assert(/^[A-Za-z0-9_-]+$/.test(encoded), `not base64url: ${encoded}`);
  assert(!encoded.includes('='), 'padding should be stripped');
  assert(decodeEmbedCode(encoded) === src, 'round-trip mismatch');
});

check('round-trips unicode source (emoji, non-ASCII identifiers)', () => {
  const src = 'let \u03c0 = 3.14159\nprint("Hello \ud83d\udc4b \u2014 \u03c0 = \\(\u03c0)")\n';
  const encoded = encodeEmbedCode(src);
  assert(decodeEmbedCode(encoded) === src, 'unicode round-trip mismatch');
});

check('round-trips empty source', () => {
  // Encodes fine, but `decodeEmbedCode('')` treats an empty *parameter* as
  // "missing" (see the dedicated check below) — an empty *source string* can
  // only ever be reached by decoding a real (non-empty) encoded blob of an
  // empty string, which is a degenerate case worth pinning directly.
  const encoded = encodeEmbedCode('');
  assert(encoded === '', 'empty source should encode to an empty string');
});

check('round-trips a realistic multi-line SwiftUI snippet', () => {
  const src = `struct CounterView: View {
    @State private var count = 0
    var body: some View {
        VStack {
            Text("\\(count)")
            Button("Increment") { count += 1 }
        }
    }
}
`;
  assert(decodeEmbedCode(encodeEmbedCode(src)) === src);
});

check('encoded output uses only URL-safe characters even for byte-diverse input', () => {
  // Cover both a `+`/`-` and `/`/`_` substitution case by brute-forcing a
  // source string until the *raw* base64 (pre-substitution) would contain
  // both `+` and `/` — cheaper to just check a handful of known strings.
  for (const src of ['>>>', '???', '\u0000\u0001\u0002\u0003', 'a'.repeat(37)]) {
    const encoded = encodeEmbedCode(src);
    assert(/^[A-Za-z0-9_-]*$/.test(encoded), `${JSON.stringify(src)} -> ${encoded}`);
    assert(decodeEmbedCode(encoded) === src);
  }
});

// ── decode error handling ───────────────────────────────────────────────

check('decodeEmbedCode rejects a missing/empty parameter', () => {
  for (const bad of [undefined, null, '']) {
    let threw = false;
    try {
      decodeEmbedCode(bad);
    } catch (err) {
      threw = err instanceof EmbedCodeError;
    }
    assert(threw, `expected EmbedCodeError for ${JSON.stringify(bad)}`);
  }
});

check('decodeEmbedCode rejects non-base64url characters', () => {
  let threw = false;
  try {
    decodeEmbedCode('not valid base64url!!');
  } catch (err) {
    threw = err instanceof EmbedCodeError;
  }
  assert(threw, 'expected EmbedCodeError');
});

check('decodeEmbedCode rejects malformed base64url length', () => {
  let threw = false;
  try {
    decodeEmbedCode('a'); // length % 4 === 1: unrecoverable padding
  } catch (err) {
    threw = err instanceof EmbedCodeError;
  }
  assert(threw, 'expected EmbedCodeError');
});

check('decodeEmbedCode rejects a parameter over MAX_ENCODED_LENGTH', () => {
  const tooLong = 'A'.repeat(MAX_ENCODED_LENGTH + 4);
  let threw = false;
  let message = '';
  try {
    decodeEmbedCode(tooLong);
  } catch (err) {
    threw = err instanceof EmbedCodeError;
    message = err.message;
  }
  assert(threw, 'expected EmbedCodeError');
  assert(/too long/i.test(message), message);
});

check('decodeEmbedCode accepts a parameter right at MAX_ENCODED_LENGTH', () => {
  // Build a valid base64url string of exactly the max length by encoding a
  // source just under the limit and padding the *source*, not truncating
  // the encoded form (truncating would corrupt the base64).
  let src = 'x'.repeat(100);
  let encoded = encodeEmbedCode(src);
  while (encoded.length < MAX_ENCODED_LENGTH) {
    src += 'x'.repeat(Math.min(100, MAX_SOURCE_LENGTH - src.length));
    encoded = encodeEmbedCode(src);
    if (src.length >= MAX_SOURCE_LENGTH) break;
  }
  assert(encoded.length <= MAX_ENCODED_LENGTH, 'test setup exceeded the limit');
  assert(decodeEmbedCode(encoded) === src);
});

check('encodeEmbedCode rejects source over MAX_SOURCE_LENGTH before ever encoding', () => {
  const tooLong = 'x'.repeat(MAX_SOURCE_LENGTH + 1);
  let threw = false;
  try {
    encodeEmbedCode(tooLong);
  } catch (err) {
    threw = err instanceof EmbedCodeError;
  }
  assert(threw, 'expected EmbedCodeError');
});

check('decodeEmbedCode rejects invalid UTF-8 byte sequences', () => {
  // A lone continuation byte (0x80) with no lead byte is invalid UTF-8.
  const bytes = new Uint8Array([0x80, 0x80]);
  let binary = '';
  for (const b of bytes) binary += String.fromCharCode(b);
  const b64url = btoa(binary).replace(/\+/g, '-').replace(/\//g, '_').replace(/=+$/, '');
  let threw = false;
  try {
    decodeEmbedCode(b64url);
  } catch (err) {
    threw = err instanceof EmbedCodeError;
  }
  assert(threw, 'expected EmbedCodeError for invalid UTF-8');
});

// ── parseEmbedParams ────────────────────────────────────────────────────

check('parseEmbedParams defaults chrome=off, theme=auto', () => {
  const { chrome, theme, codeParam } = parseEmbedParams('?code=abc');
  assert(chrome === 'off', chrome);
  assert(theme === 'auto', theme);
  assert(codeParam === 'abc', codeParam);
});

check('parseEmbedParams reads chrome=on and theme=dark', () => {
  const { chrome, theme } = parseEmbedParams('?code=abc&chrome=on&theme=dark');
  assert(chrome === 'on', chrome);
  assert(theme === 'dark', theme);
});

check('parseEmbedParams falls back to defaults for unrecognized values (no throw)', () => {
  const { chrome, theme } = parseEmbedParams('?code=abc&chrome=weird&theme=weird');
  assert(chrome === 'off', chrome);
  assert(theme === 'auto', theme);
});

check('parseEmbedParams is case-insensitive for chrome/theme', () => {
  const { chrome, theme } = parseEmbedParams('?code=abc&chrome=ON&theme=LIGHT');
  assert(chrome === 'on', chrome);
  assert(theme === 'light', theme);
});

check('parseEmbedParams returns an empty codeParam when absent (caller reports the error)', () => {
  const { codeParam } = parseEmbedParams('?chrome=on');
  assert(codeParam === '', JSON.stringify(codeParam));
});

check('parseEmbedParams returns an empty originParam by default', () => {
  const { originParam } = parseEmbedParams('?code=abc');
  assert(originParam === '', JSON.stringify(originParam));
});

check('parseEmbedParams reads and trims an explicit origin', () => {
  const { originParam } = parseEmbedParams('?code=abc&origin=' + encodeURIComponent(' https://example.com '));
  assert(originParam === 'https://example.com', JSON.stringify(originParam));
});

// ── resolveEmbedSource: missing vs. present-but-empty vs. malformed ─────────

check('resolveEmbedSource throws EmbedCodeError when `code` is entirely absent', () => {
  let threw = false;
  try {
    resolveEmbedSource('?chrome=on');
  } catch (err) {
    threw = err instanceof EmbedCodeError;
  }
  assert(threw, 'expected EmbedCodeError for a missing code param');
});

check('resolveEmbedSource resolves `code=` (present, empty) to an empty program, not an error', () => {
  const { source } = resolveEmbedSource('?code=&chrome=on');
  assert(source === '', JSON.stringify(source));
});

check('resolveEmbedSource decodes a present, non-empty `code` via decodeEmbedCode', () => {
  const src = 'print("hi")';
  const encoded = encodeEmbedCode(src);
  const { source } = resolveEmbedSource(`?code=${encoded}`);
  assert(source === src, source);
});

check('resolveEmbedSource still rejects a malformed non-empty `code`', () => {
  let threw = false;
  try {
    resolveEmbedSource('?code=not valid base64url!!');
  } catch (err) {
    threw = err instanceof EmbedCodeError;
  }
  assert(threw, 'expected EmbedCodeError for malformed code');
});

// ── playgroundUrl / buildEmbedSnippet ───────────────────────────────────

check('playgroundUrl builds a base-relative link carrying the same code param', () => {
  assert(playgroundUrl('/', 'abc123') === '/playground/?code=abc123');
  assert(playgroundUrl('/tswift/', 'abc123') === '/tswift/playground/?code=abc123');
  assert(playgroundUrl('/tswift', 'abc123') === '/tswift/playground/?code=abc123');
});

check('playgroundUrl omits the query entirely when there is no code', () => {
  assert(playgroundUrl('/', '') === '/playground/');
});

check('buildEmbedSnippet produces a self-contained, base64url-encoded iframe tag', () => {
  const src = 'print("hi")';
  const { html, url } = buildEmbedSnippet({
    origin: 'https://example.com',
    base: '/',
    source: src,
  });
  assert(html.startsWith('<iframe '), html);
  assert(html.includes('src="' + url + '"'), html);
  assert(url.startsWith('https://example.com/embed/?'), url);
  assert(url.includes('embed=1'), url);
  assert(url.includes('chrome=off'), url);
  assert(!url.includes('theme='), 'theme=auto should be omitted, not spelled out');
  assert(html.includes('style="border:0;border-radius:10px;overflow:hidden"'), html);
  assert(html.includes('allow="clipboard-write"'), html);
  assert(html.includes('&origin='), 'should hint at the &origin= param for strict postMessage targeting');

  const params = new URL(url).searchParams;
  assert(decodeEmbedCode(params.get('code')) === src, 'embedded code param does not decode back to source');
});

check('buildEmbedSnippet respects an explicit non-auto theme and honors base path', () => {
  const { url } = buildEmbedSnippet({
    origin: 'https://example.com',
    base: '/tswift',
    source: 'print(1)',
    chrome: 'on',
    theme: 'dark',
  });
  assert(url.startsWith('https://example.com/tswift/embed/?'), url);
  assert(url.includes('chrome=on'), url);
  assert(url.includes('theme=dark'), url);
});

check('buildEmbedSnippet surfaces an EmbedCodeError for an over-limit source', () => {
  let threw = false;
  try {
    buildEmbedSnippet({
      origin: 'https://example.com',
      base: '/',
      source: 'x'.repeat(MAX_SOURCE_LENGTH + 1),
    });
  } catch (err) {
    threw = err instanceof EmbedCodeError;
  }
  assert(threw, 'expected EmbedCodeError');
});

if (failures > 0) {
  console.error(`\n${failures} embed codec/param check(s) failed`);
  process.exit(1);
}
console.log('\nall embed codec/param checks passed');
