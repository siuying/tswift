// Sidebar mode switcher state machine for the Studio's compact
// Files / Report / Symbols tablist (WAI-ARIA "tabs with roving tabindex"
// pattern). Kept DOM-agnostic and dependency-free so the exact production
// behavior is unit-testable in Node without jsdom: the mutators operate on
// any object exposing the small `.hidden` / `.classList` / `.setAttribute` /
// `.tabIndex` surface a real DOM element already provides, and the keyboard
// navigation is a pure function of (current mode, key).

export const SIDEBAR_MODES = ['files', 'report', 'symbols'];

/**
 * Apply `mode` as the single visible panel and roving tab stop.
 *
 * @param {string} mode one of SIDEBAR_MODES
 * @param {object} ui
 * @param {Record<string, {hidden: boolean}>} ui.panels mode -> panel element
 * @param {Iterable<{dataset: {mode: string}, classList: {toggle: Function},
 *   setAttribute: Function, tabIndex: number}>} ui.buttons tab buttons
 */
export function applySidebarMode(mode, { panels, buttons }) {
  for (const m of SIDEBAR_MODES) {
    if (panels[m]) panels[m].hidden = m !== mode;
  }
  for (const b of buttons) {
    const active = b.dataset.mode === mode;
    b.classList.toggle('active', active);
    b.setAttribute('aria-selected', active ? 'true' : 'false');
    b.tabIndex = active ? 0 : -1;
  }
}

/**
 * Pure roving-tabindex keyboard navigation: given the currently focused mode
 * and a keydown `key`, return the mode that should receive focus next, or
 * `null` if the key is not a navigation key (or `current` is unknown).
 */
export function nextSidebarMode(current, key) {
  const idx = SIDEBAR_MODES.indexOf(current);
  if (idx === -1) return null;
  const len = SIDEBAR_MODES.length;
  switch (key) {
    case 'ArrowRight':
    case 'ArrowDown':
      return SIDEBAR_MODES[(idx + 1) % len];
    case 'ArrowLeft':
    case 'ArrowUp':
      return SIDEBAR_MODES[(idx - 1 + len) % len];
    case 'Home':
      return SIDEBAR_MODES[0];
    case 'End':
      return SIDEBAR_MODES[len - 1];
    default:
      return null;
  }
}
