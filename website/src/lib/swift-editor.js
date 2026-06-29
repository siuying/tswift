// CodeMirror 6 Swift editor for the playground.
//
// Replaces the old plain <textarea>: Swift syntax highlighting (the legacy
// CodeMirror Swift mode) plus a `@codemirror/lint` linter that surfaces the
// *frontend's* own diagnostics (line/col/severity from `swiftDiagnostics`) as
// inline squiggles, a gutter marker, and hover tooltips — the single source of
// error truth shared with the `runSwift` compile phase.
//
// `createSwiftEditor` returns a small shim with a `.value` getter/setter and an
// `addEventListener('input', …)` method so the rest of FullPlayground.astro can
// keep treating it like the textarea it replaced.

import { EditorView, basicSetup } from 'codemirror';
import { EditorState } from '@codemirror/state';
import { StreamLanguage, HighlightStyle, syntaxHighlighting } from '@codemirror/language';
import { swift } from '@codemirror/legacy-modes/mode/swift';
import { tags as t } from '@lezer/highlight';
import { linter, lintGutter, forceLinting } from '@codemirror/lint';

// Highlight palette tuned to the dark `--code-bg` editor surface (the site's
// burnt-orange / steel-blue accents).
const highlightStyle = HighlightStyle.define([
  { tag: t.keyword, color: '#e0884f', fontWeight: '600' },
  { tag: [t.string, t.special(t.string)], color: '#7ee0a8' },
  { tag: [t.number, t.bool, t.null], color: '#d9a85f' },
  { tag: t.comment, color: '#7c8294', fontStyle: 'italic' },
  { tag: [t.definition(t.variableName), t.function(t.variableName)], color: '#9ec1e8' },
  { tag: [t.typeName, t.className], color: '#c4a6e8' },
  { tag: t.operator, color: '#c8cedd' },
  { tag: t.meta, color: '#e0884f' }, // attributes / directives (@main, #if)
]);

const editorTheme = EditorView.theme(
  {
    '&': { color: '#dfe4f2', backgroundColor: 'transparent', height: '100%' },
    '.cm-content': { fontFamily: 'var(--font-mono)', fontSize: '.875rem', padding: '1.1rem 0.5rem' },
    '.cm-gutters': {
      backgroundColor: 'transparent',
      color: '#5b6170',
      border: 'none',
      paddingLeft: '0.5rem',
    },
    '.cm-activeLine': { backgroundColor: 'rgba(255,255,255,.03)' },
    '.cm-activeLineGutter': { backgroundColor: 'transparent' },
    '.cm-cursor': { borderLeftColor: '#e0884f' },
    '&.cm-focused .cm-selectionBackground, .cm-selectionBackground': {
      backgroundColor: 'rgba(217,98,46,.25)',
    },
    '.cm-scroller': { overflow: 'auto', lineHeight: '1.65' },
  },
  { dark: true }
);

// Map one frontend diagnostic ({line,col,severity,message}, 1-based) to an
// absolute CodeMirror range: from the error column to the end of that word
// (falling back to the rest of the line) so the squiggle has visible width.
function toCmRange(doc, d) {
  const lineNo = Math.min(Math.max(d.line, 1), doc.lines);
  const line = doc.line(lineNo);
  const from = Math.min(line.from + Math.max(d.col - 1, 0), line.to);
  const rest = doc.sliceString(from, line.to);
  const word = /^[A-Za-z0-9_]+/.exec(rest);
  const to = word ? from + word[0].length : Math.max(from + 1, line.to);
  return { from, to: Math.min(to, doc.length) };
}

// Build a CM linter from a `diagnose(text) -> {ok, diagnostics:[…]} | null`
// callback (the wasm `swiftDiagnostics`, JSON-parsed). Returns no markers until
// the wasm module is ready (diagnose returns null), so the editor never blocks.
function swiftLinter(getDiagnose) {
  return linter(
    (view) => {
      const diagnose = getDiagnose();
      if (!diagnose) return [];
      let result;
      try {
        result = diagnose(view.state.doc.toString());
      } catch {
        return [];
      }
      if (!result || !Array.isArray(result.diagnostics)) return [];
      const doc = view.state.doc;
      return result.diagnostics.map((d) => {
        const { from, to } = toCmRange(doc, d);
        return {
          from,
          to,
          severity: d.severity === 'warning' ? 'warning' : 'error',
          message: d.message,
        };
      });
    },
    { delay: 300 }
  );
}

/**
 * Mount a Swift editor into `parent`.
 *
 * @param {Object}   opts
 * @param {Element}  opts.parent     container element
 * @param {string}   [opts.doc]      initial document
 * @param {() => void} [opts.onInput] called on every document change
 * @param {() => (null | ((text:string) => any))} [opts.getDiagnose]
 *        returns the JSON-parsing `swiftDiagnostics` wrapper, or null until ready
 * @returns {{ value: string, addEventListener: Function, forceLint: Function, view: EditorView }}
 */
export function createSwiftEditor({ parent, doc = '', onInput, getDiagnose = () => null }) {
  const inputListeners = new Set();
  if (onInput) inputListeners.add(onInput);

  const updateListener = EditorView.updateListener.of((update) => {
    if (update.docChanged) inputListeners.forEach((cb) => cb());
  });

  const view = new EditorView({
    parent,
    state: EditorState.create({
      doc,
      extensions: [
        basicSetup,
        StreamLanguage.define(swift),
        syntaxHighlighting(highlightStyle),
        editorTheme,
        lintGutter(),
        swiftLinter(getDiagnose),
        updateListener,
        EditorView.lineWrapping,
      ],
    }),
  });

  return {
    view,
    get value() {
      return view.state.doc.toString();
    },
    set value(next) {
      view.dispatch({
        changes: { from: 0, to: view.state.doc.length, insert: next },
      });
    },
    addEventListener(type, cb) {
      if (type === 'input') inputListeners.add(cb);
    },
    // Re-run the linter now (e.g. once the wasm module finishes loading, so the
    // initial document gets diagnosed without waiting for the first keystroke).
    forceLint() {
      forceLinting(view);
    },
  };
}
