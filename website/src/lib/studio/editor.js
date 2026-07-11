// CodeMirror 6 multi-file editor host for the Web Studio.
//
// Unlike the single-document playground editor (`../swift-editor.js`), Studio
// keeps a per-file `EditorState` so switching tabs preserves each file's undo
// history, selection, and scroll position. One `EditorView` is reused; opening
// a file swaps its state in via `view.setState`.
//
// Diagnostics are module-wide: the caller feeds the latest per-file diagnostics
// (from the wasm `swiftDiagnosticsModule`) and the linter renders only those
// belonging to the currently-open file, as gutter markers + inline squiggles.

import { EditorView, keymap, lineNumbers, highlightActiveLine } from '@codemirror/view';
import { EditorState, Compartment } from '@codemirror/state';
import {
  StreamLanguage,
  HighlightStyle,
  syntaxHighlighting,
  indentOnInput,
  bracketMatching,
} from '@codemirror/language';
import { swift } from '@codemirror/legacy-modes/mode/swift';
import { tags as t } from '@lezer/highlight';
import { lintGutter, setDiagnostics as setLintDiagnostics } from '@codemirror/lint';
import {
  history,
  historyKeymap,
  defaultKeymap,
  indentWithTab,
} from '@codemirror/commands';
import { closeBrackets, closeBracketsKeymap } from '@codemirror/autocomplete';

const highlightStyle = HighlightStyle.define([
  { tag: t.keyword, color: '#e0884f', fontWeight: '600' },
  { tag: [t.string, t.special(t.string)], color: '#7ee0a8' },
  { tag: [t.number, t.bool, t.null], color: '#d9a85f' },
  { tag: t.comment, color: '#7c8294', fontStyle: 'italic' },
  { tag: [t.definition(t.variableName), t.function(t.variableName)], color: '#9ec1e8' },
  { tag: [t.typeName, t.className], color: '#c4a6e8' },
  { tag: t.operator, color: '#c8cedd' },
  { tag: t.meta, color: '#e0884f' },
]);

const editorTheme = EditorView.theme(
  {
    '&': { color: '#dfe4f2', backgroundColor: 'transparent', height: '100%' },
    '.cm-content': { fontFamily: 'var(--font-mono)', fontSize: '.85rem', padding: '0.75rem 0.5rem' },
    '.cm-gutters': { backgroundColor: 'transparent', color: '#5b6170', border: 'none' },
    '.cm-activeLine': { backgroundColor: 'rgba(255,255,255,.03)' },
    '.cm-activeLineGutter': { backgroundColor: 'rgba(255,255,255,.04)' },
    '.cm-cursor': { borderLeftColor: '#e0884f' },
    '&.cm-focused .cm-selectionBackground, .cm-selectionBackground': {
      backgroundColor: 'rgba(217,98,46,.25)',
    },
    '.cm-scroller': { overflow: 'auto', lineHeight: '1.6' },
    '.cm-lintRange-error': { textDecoration: 'underline wavy #e0674f' },
    '.cm-lintRange-warning': { textDecoration: 'underline wavy #d9a85f' },
  },
  { dark: true },
);

// Map a 1-based {line,col} diagnostic to an absolute range, widening to the end
// of the word at that column so the squiggle is visible.
function toRange(doc, d) {
  const lineNo = Math.min(Math.max(d.line, 1), doc.lines);
  const line = doc.line(lineNo);
  const from = Math.min(line.from + Math.max((d.col || 1) - 1, 0), line.to);
  const rest = doc.sliceString(from, line.to);
  const word = /^[A-Za-z0-9_]+/.exec(rest);
  const to = word ? from + word[0].length : Math.max(from + 1, line.to);
  return { from, to: Math.min(to, doc.length) };
}

/**
 * Create a Studio editor bound to `parent`.
 *
 * @param {Object} opts
 * @param {Element} opts.parent
 * @param {() => (Array<{line,col,severity,message}>)} opts.getDiagnostics
 *        latest diagnostics for the currently-open file
 * @param {() => void} opts.onInput      fired on every document change
 * @param {() => void} opts.onRun        fired on Cmd/Ctrl-Enter
 * @param {() => void} opts.onSave       fired on Cmd/Ctrl-S (prevents browser save)
 */
export function createStudioEditor({ parent, getDiagnostics, onInput, onRun, onSave }) {
  const states = new Map(); // path -> EditorState
  let currentPath = null;

  // Diagnostics are pushed (via `setDiagnostics` state effects), not pulled by
  // a `linter()` source function. A pull-based linter only re-runs on its own
  // debounce timer or on document changes/selection moves — after a debounced
  // wasm analyze() resolves with no further keystrokes, the gutter/squiggles
  // would go stale until the next edit. Pushing lets `refreshDiagnostics()`
  // update the view the instant new diagnostics are available.
  function toLintDiagnostics(doc, diags) {
    if (!Array.isArray(diags)) return [];
    return diags.map((d) => {
      const { from, to } = toRange(doc, d);
      return {
        from,
        to,
        severity: d.severity === 'warning' ? 'warning' : 'error',
        message: d.message,
      };
    });
  }

  function applyDiagnostics() {
    const diags = getDiagnostics ? getDiagnostics() : [];
    view.dispatch(setLintDiagnostics(view.state, toLintDiagnostics(view.state.doc, diags)));
  }

  const shortcutKeymap = keymap.of([
    {
      key: 'Mod-Enter',
      run: () => {
        if (onRun) onRun();
        return true;
      },
    },
    {
      key: 'Mod-s',
      run: () => {
        if (onSave) onSave();
        return true;
      },
    },
  ]);

  function extensions() {
    return [
      lineNumbers(),
      lintGutter(),
      history(),
      indentOnInput(),
      bracketMatching(),
      closeBrackets(),
      highlightActiveLine(),
      StreamLanguage.define(swift),
      syntaxHighlighting(highlightStyle),
      editorTheme,
      EditorView.lineWrapping,
      shortcutKeymap,
      keymap.of([...closeBracketsKeymap, ...defaultKeymap, ...historyKeymap, indentWithTab]),
      EditorView.updateListener.of((u) => {
        if (u.docChanged) {
          if (currentPath) states.set(currentPath, u.state);
          if (onInput) onInput();
        }
      }),
    ];
  }

  const view = new EditorView({ parent, state: EditorState.create({ doc: '', extensions: extensions() }) });

  return {
    view,
    /** Open `path` with `source`, preserving prior per-file state when re-opened. */
    open(path, source) {
      if (currentPath) states.set(currentPath, view.state);
      currentPath = path;
      let state = states.get(path);
      if (!state) {
        state = EditorState.create({ doc: source, extensions: extensions() });
        states.set(path, state);
      }
      view.setState(state);
      applyDiagnostics();
    },
    /** Re-push diagnostics for the currently-open file (call after analyze()). */
    refreshDiagnostics() {
      applyDiagnostics();
    },
    /** Forget a closed/renamed/deleted file's cached state. */
    forget(path) {
      states.delete(path);
      if (currentPath === path) currentPath = null;
    },
    /** Current document text. */
    get value() {
      return view.state.doc.toString();
    },
    /** Replace the current document text wholesale. */
    set value(next) {
      view.dispatch({ changes: { from: 0, to: view.state.doc.length, insert: next } });
    },
    /** Move the cursor to a 1-based line and scroll it into view. */
    gotoLine(line) {
      const doc = view.state.doc;
      const lineNo = Math.min(Math.max(line, 1), doc.lines);
      const pos = doc.line(lineNo).from;
      view.dispatch({ selection: { anchor: pos }, scrollIntoView: true });
      view.focus();
    },
    focus() {
      view.focus();
    },
  };
}
