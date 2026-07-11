// Pure URL-parameter helpers for the `/embed/` route and the "Embed" snippet
// generator (`FullPlayground.astro`). Kept dependency-free and DOM-free so
// they're directly unit-testable under plain Node (see `website/test/
// embed-code.mjs`).

import { decodeEmbedCode, encodeEmbedCode, EmbedCodeError } from './embed-code.js';

/** @typedef {{chrome: 'off'|'on', theme: 'light'|'dark'|'auto', codeParam: string,
 *   originParam: string}} EmbedParams */

/**
 * Parse the `/embed/` route's query string into a validated, defaulted
 * shape. Unknown/malformed `chrome`/`theme` values fall back to their safe
 * default rather than erroring — only a missing/invalid `code` is treated
 * as a hard error (by the caller, via `decodeEmbedCode`/`resolveEmbedSource`).
 *
 * `originParam` is the raw (trimmed) value of an optional `?origin=` query
 * param: when the embedding page supplies its own origin, the embed page
 * addresses every postMessage directly to it instead of broadcasting with
 * `"*"` (see `resolveEmbedSource`'s sibling usage in `embed.astro`). It is
 * *not* validated here — an invalid value simply makes `postMessage` throw
 * at call time, which the caller already guards against.
 *
 * @param {string} search - e.g. `location.search`, including the leading `?`.
 * @returns {EmbedParams}
 */
export function parseEmbedParams(search) {
  const params = new URLSearchParams(search);
  const chromeRaw = (params.get('chrome') || 'off').trim().toLowerCase();
  const chrome = chromeRaw === 'on' ? 'on' : 'off';
  const themeRaw = (params.get('theme') || 'auto').trim().toLowerCase();
  const theme = themeRaw === 'light' || themeRaw === 'dark' ? themeRaw : 'auto';
  const codeParam = params.get('code') || '';
  const originParam = (params.get('origin') || '').trim();
  return { chrome, theme, codeParam, originParam };
}

/**
 * Resolve the `/embed/` route's Swift source from its raw query string,
 * distinguishing three cases a naive `decodeEmbedCode(params.get('code'))`
 * conflates:
 *  - `code` entirely absent from the URL → hard error ("missing"): there is
 *    nothing to render, and the page should show the inline error box.
 *  - `code=` present but empty (`params.has('code')` true, value `""`) → an
 *    *explicitly* empty program, not an error: resolves to `""` so the page
 *    renders an empty console instead of a confusing "missing" message for
 *    a link that was deliberately built with no source.
 *  - `code=<...>` present and non-empty → decoded via `decodeEmbedCode`,
 *    which still throws `EmbedCodeError` for genuinely malformed input
 *    (bad base64url, invalid UTF-8, over the length ceiling, etc).
 *
 * Kept dependency-free/DOM-free (like the rest of this module) so the
 * three-way split is unit-testable without a browser.
 *
 * @param {string} search - e.g. `location.search`, including the leading `?`.
 * @returns {{ source: string }}
 * @throws {EmbedCodeError} for a missing or malformed `code`.
 */
export function resolveEmbedSource(search) {
  const params = new URLSearchParams(search);
  if (!params.has('code')) {
    throw new EmbedCodeError('missing "code" parameter');
  }
  const codeParam = params.get('code') || '';
  if (codeParam === '') return { source: '' };
  return { source: decodeEmbedCode(codeParam) };
}

/**
 * Build the path (+ query) for "open in playground with this code
 * pre-filled", relative to the site base.
 *
 * @param {string} base - the site's `BASE_URL` (e.g. `/`), always ending in `/`.
 * @param {string} codeParam - an already-encoded `code` value (base64url).
 */
export function playgroundUrl(base, codeParam) {
  const prefix = base.endsWith('/') ? base : `${base}/`;
  const qs = codeParam ? `?code=${codeParam}` : '';
  return `${prefix}playground/${qs}`;
}

/**
 * Build the `<iframe>` HTML snippet the "Embed" button in the playground
 * copies to the clipboard. `origin` + `base` together form the absolute
 * embed URL so the snippet works when pasted onto a third-party page.
 *
 * @param {{origin: string, base: string, source: string, chrome?: 'off'|'on',
 *   theme?: 'light'|'dark'|'auto', width?: string, height?: number}} opts
 * @returns {{html: string, url: string}}
 */
export function buildEmbedSnippet({
  origin,
  base,
  source,
  chrome = 'off',
  theme = 'auto',
  width = '100%',
  height = 520,
}) {
  const code = encodeEmbedCode(source);
  const prefix = base.endsWith('/') ? base : `${base}/`;
  const params = new URLSearchParams();
  params.set('embed', '1');
  params.set('chrome', chrome);
  if (theme !== 'auto') params.set('theme', theme);
  params.set('code', code);
  const url = `${origin}${prefix}embed/?${params.toString()}`;
  // This generator can only know the URL the embed *iframe* will load, not
  // the origin of whatever third-party page the caller ultimately pastes
  // this snippet onto — so it cannot fill in `&origin=` for them. Leave an
  // explicit, actionable placeholder comment instead of silently shipping
  // an unauthenticated ('*'-targeted) postMessage protocol by default; see
  // /how-it-works/embedding for what `origin` unlocks.
  const html =
    `<iframe src="${url}" width="${width}" height="${height}" ` +
    `style="border:0;border-radius:10px;overflow:hidden" allow="clipboard-write" ` +
    `title="tswift embed"></iframe>\n` +
    `<!-- Optional: for origin-checked postMessage events (tswift-embed:ready/error/resize),
` +
    `     append &origin=<url-encoded-origin-of-THIS-page> to the iframe's src above. -->`;
  return { html, url };
}
