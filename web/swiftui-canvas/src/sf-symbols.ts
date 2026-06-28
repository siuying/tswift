// sf-symbols.ts — a small SF Symbol name → Unicode glyph table for the web host.
// SF Symbols are an Apple proprietary font we can't ship, so the web renderer
// approximates the common symbols with Unicode/emoji. This is an explicitly
// accepted iOS-vs-web drift surface (like the color table in modifier-css.ts);
// iOS uses the real `Image(systemName:)`.

/** Common SF Symbol base names → an approximating glyph. */
const SF_GLYPH: Record<string, string> = {
  house: "\u{1F3E0}", // 🏠
  star: "\u2605", // ★
  heart: "\u2665", // ♥
  person: "\u{1F464}", // 👤
  "person.circle": "\u{1F464}",
  gear: "\u2699", // ⚙
  gearshape: "\u2699",
  magnifyingglass: "\u{1F50D}", // 🔍
  trash: "\u{1F5D1}", // 🗑
  plus: "\u002B", // +
  minus: "\u2212", // −
  checkmark: "\u2713", // ✓
  xmark: "\u2715", // ✕
  bell: "\u{1F514}", // 🔔
  envelope: "\u2709", // ✉
  phone: "\u{1F4DE}", // 📞
  calendar: "\u{1F4C5}", // 📅
  clock: "\u{1F551}", // 🕑
  folder: "\u{1F4C1}", // 📁
  doc: "\u{1F4C4}", // 📄
  photo: "\u{1F5BC}", // 🖼
  camera: "\u{1F4F7}", // 📷
  pencil: "\u270F", // ✏
  bookmark: "\u{1F516}", // 🔖
  flag: "\u2691", // ⚑
  tag: "\u{1F3F7}", // 🏷
  cart: "\u{1F6D2}", // 🛒
  bag: "\u{1F6CD}", // 🛍
  creditcard: "\u{1F4B3}", // 💳
  location: "\u{1F4CD}", // 📍
  map: "\u{1F5FA}", // 🗺
  globe: "\u{1F310}", // 🌐
  "sun.max": "\u2600", // ☀
  moon: "\u{1F319}", // 🌙
  cloud: "\u2601", // ☁
  bolt: "\u26A1", // ⚡
  play: "\u25B6", // ▶
  pause: "\u23F8", // ⏸
  stop: "\u25A0", // ■
  forward: "\u23E9", // ⏩
  backward: "\u23EA", // ⏪
  "music.note": "\u266A", // ♪
  mic: "\u{1F3A4}", // 🎤
  speaker: "\u{1F50A}", // 🔊
  wifi: "\u{1F4F6}", // 📶
  lock: "\u{1F512}", // 🔒
  key: "\u{1F511}", // 🔑
  eye: "\u{1F441}", // 👁
  message: "\u{1F4AC}", // 💬
  paperplane: "\u2708", // ✈
  "arrow.right": "\u2192", // →
  "arrow.left": "\u2190", // ←
  "arrow.up": "\u2191", // ↑
  "arrow.down": "\u2193", // ↓
  "chevron.right": "\u203A", // ›
  "chevron.left": "\u2039", // ‹
  "chevron.up": "\u2303", // ⌃
  "chevron.down": "\u2304", // ⌄
  info: "\u2139", // ℹ
  "info.circle": "\u2139",
  exclamationmark: "\u2757", // ❗
  questionmark: "\u2753", // ❓
  "hand.thumbsup": "\u{1F44D}", // 👍
  "hand.thumbsdown": "\u{1F44E}", // 👎
};

/** Fallback glyph for an unmapped SF Symbol name. */
const SF_FALLBACK = "\u25A2"; // ▢

/**
 * Resolve an SF Symbol name (e.g. `"star.fill"`, `"person.circle.fill"`) to an
 * approximating glyph. Variant suffixes (`.fill`, `.circle`, `.slash`, …) are
 * progressively stripped so a variant falls back to its base symbol's glyph.
 */
export function sfGlyph(name: string): string {
  if (!name) return "";
  if (SF_GLYPH[name]) return SF_GLYPH[name];
  // Progressively drop the trailing dotted component to find a base match.
  const parts = name.split(".");
  while (parts.length > 1) {
    parts.pop();
    const base = parts.join(".");
    if (SF_GLYPH[base]) return SF_GLYPH[base];
  }
  return SF_FALLBACK;
}
