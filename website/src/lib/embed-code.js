// Codec for the `/embed/` route's `code` URL parameter.
//
// ## Why base64url of UTF-8, not lz-string
//
// The task brief allowed either lz-string (if already a dependency, or a
// tiny vendored util) or plain base64url. This repo has no lz-string
// dependency anywhere (`package-lock.json` has zero hits) and the project's
// standing rule is "no new npm deps unless trivial + pinned, prefer none"
// (see `notes.md`). A vendored LZ implementation is not "tiny" if done
// honestly (real LZ-string is ~300 lines of bit-packing code) and would be
// unauditable-by-eye in a way a plain base64 transform is not. So this
// module ships **base64url of the UTF-8 bytes of the source**, no
// compression. It costs ~33% size overhead vs. raw UTF-8 (not vs.
// lz-string, which typically beats raw UTF-8 substantially on repetitive
// source text), but every browser and Node ships `btoa`/`atob` +
// `TextEncoder`/`TextDecoder` natively — zero dependency surface, and the
// encoding is trivially inspectable/debuggable by hand. If embed links ever
// need to carry much larger snippets than the length limit below allows,
// swap this module's `encodeEmbedCode`/`decodeEmbedCode` bodies for an
// lz-string (or DEFLATE via the browser's native `CompressionStream`) pass
// — the call sites (`embed.astro`, the Embed-snippet generator, and this
// module's own tests) don't need to change, only the two functions here.
//
// URL-safe base64 ("base64url", RFC 4648 §5): `+`→`-`, `/`→`_`, no `=`
// padding (padding is deterministically recoverable from length, so it's
// dropped to keep URLs shorter and avoid `=` needing percent-encoding in a
// query string).

/** Max characters allowed in the *encoded* `code` URL parameter. */
export const MAX_ENCODED_LENGTH = 20000;

/** Max characters allowed in the *decoded* Swift source (defense in depth;
 * base64 overhead means this is already implied by MAX_ENCODED_LENGTH, but
 * kept as an explicit, independently-checked ceiling in case the codec ever
 * changes to something with a different expansion ratio). */
export const MAX_SOURCE_LENGTH = 16000;

/** Thrown by `decodeEmbedCode`/`encodeEmbedCode` for any malformed or
 * over-limit input. Callers should catch this specifically and render a
 * friendly inline message rather than letting it propagate as a crash. */
export class EmbedCodeError extends Error {
  constructor(message) {
    super(message);
    this.name = 'EmbedCodeError';
  }
}

function bytesToBinaryString(bytes) {
  // `String.fromCharCode(...bytes)` blows the call stack for large inputs;
  // chunk it. 0x8000 is comfortably under every engine's argument-count
  // limit for `Function.prototype.apply`/spread.
  const CHUNK = 0x8000;
  let binary = '';
  for (let i = 0; i < bytes.length; i += CHUNK) {
    binary += String.fromCharCode(...bytes.subarray(i, i + CHUNK));
  }
  return binary;
}

function toBase64Url(base64) {
  return base64.replace(/\+/g, '-').replace(/\//g, '_').replace(/=+$/, '');
}

function fromBase64Url(base64url) {
  let base64 = base64url.replace(/-/g, '+').replace(/_/g, '/');
  const pad = base64.length % 4;
  if (pad === 1) {
    throw new EmbedCodeError('code parameter is not valid base64url (bad length)');
  }
  if (pad) base64 += '='.repeat(4 - pad);
  return base64;
}

/**
 * Encode Swift source into the `code` URL parameter's wire format
 * (base64url of its UTF-8 bytes). Throws `EmbedCodeError` if the encoded
 * result would exceed `MAX_ENCODED_LENGTH` — callers building a snippet
 * should surface that as "snippet too large to embed" rather than emitting
 * a broken link.
 */
export function encodeEmbedCode(source) {
  if (typeof source !== 'string') {
    throw new EmbedCodeError('source must be a string');
  }
  if (source.length > MAX_SOURCE_LENGTH) {
    throw new EmbedCodeError(
      `source is too long to embed (${source.length} chars, max ${MAX_SOURCE_LENGTH})`,
    );
  }
  const bytes = new TextEncoder().encode(source);
  const encoded = toBase64Url(btoa(bytesToBinaryString(bytes)));
  if (encoded.length > MAX_ENCODED_LENGTH) {
    throw new EmbedCodeError(
      `encoded snippet is too long to embed (${encoded.length} chars, max ${MAX_ENCODED_LENGTH})`,
    );
  }
  return encoded;
}

/**
 * Decode the `code` URL parameter back into Swift source. Throws
 * `EmbedCodeError` (never a raw platform error) for: missing/empty input,
 * over the length limit, non-base64url characters, malformed base64, or
 * invalid UTF-8 — every failure mode a caller needs to render as one
 * friendly inline message.
 */
export function decodeEmbedCode(param) {
  if (typeof param !== 'string' || param.length === 0) {
    throw new EmbedCodeError('missing "code" parameter');
  }
  if (param.length > MAX_ENCODED_LENGTH) {
    throw new EmbedCodeError(
      `"code" parameter is too long (${param.length} chars, max ${MAX_ENCODED_LENGTH})`,
    );
  }
  if (!/^[A-Za-z0-9_-]+$/.test(param)) {
    throw new EmbedCodeError('"code" parameter is not valid base64url');
  }
  const base64 = fromBase64Url(param);
  let binary;
  try {
    binary = atob(base64);
  } catch {
    throw new EmbedCodeError('"code" parameter is not valid base64');
  }
  const bytes = new Uint8Array(binary.length);
  for (let i = 0; i < binary.length; i++) bytes[i] = binary.charCodeAt(i);
  let source;
  try {
    source = new TextDecoder('utf-8', { fatal: true }).decode(bytes);
  } catch {
    throw new EmbedCodeError('"code" parameter is not valid UTF-8');
  }
  if (source.length > MAX_SOURCE_LENGTH) {
    throw new EmbedCodeError(
      `decoded source is too long (${source.length} chars, max ${MAX_SOURCE_LENGTH})`,
    );
  }
  return source;
}
