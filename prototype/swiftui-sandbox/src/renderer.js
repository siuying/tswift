// UIIR → DOM renderer for the SwiftUI sandbox preview.
//
// The Rust host serializes a `View` body into UIIR JSON (see
// `crates/tswift-swiftui/src/uiir.rs`): a tree of `{id, kind, args, modifiers,
// children}` nodes. This module walks that tree into real DOM, applying
// modifiers as inline styles and wiring interactive controls back to the host
// through a single `dispatch(id, event, value)` callback.
//
// `dispatch` returns the *next* UIIR tree (the host re-renders after every
// event), so the caller just re-runs `renderTree` with the result.

// SwiftUI semantic colors → CSS. Tuned for the dark device canvas.
const COLORS = {
  primary: '#ffffff', secondary: '#98989f', white: '#ffffff', black: '#000000',
  red: '#ff453a', orange: '#ff9f0a', yellow: '#ffd60a', green: '#32d74b',
  mint: '#66d4cf', teal: '#40c8e0', cyan: '#64d2ff', blue: '#0a84ff',
  indigo: '#5e5ce6', purple: '#bf5af2', pink: '#ff375f', brown: '#ac8e68',
  gray: '#8e8e93', clear: 'transparent',
};

// SwiftUI text styles → [font-size px, default weight].
const TEXT_STYLES = {
  largeTitle: [34, 400], title: [28, 400], title2: [22, 400], title3: [20, 400],
  headline: [17, 600], subheadline: [15, 400], body: [17, 400], callout: [16, 400],
  caption: [12, 400], caption2: [11, 400], footnote: [13, 400],
};

const WEIGHTS = {
  ultraLight: 100, thin: 200, light: 300, regular: 400, medium: 500,
  semibold: 600, bold: 700, heavy: 800, black: 900,
};

function colorOf(value) {
  if (value && value.$ === 'color') return COLORS[value.name] ?? value.name;
  if (typeof value === 'string') return COLORS[value] ?? value;
  return null;
}

function el(tag, className) {
  const node = document.createElement(tag);
  if (className) node.className = className;
  return node;
}

// Apply a node's ordered modifier list to a DOM element's style.
function applyModifiers(node, dom, modifiers) {
  for (const mod of modifiers ?? []) {
    const { name, value } = mod;
    switch (name) {
      case 'font': {
        const style = TEXT_STYLES[value?.name];
        if (style) {
          dom.style.fontSize = `${style[0]}px`;
          if (!dom.style.fontWeight) dom.style.fontWeight = String(style[1]);
        }
        break;
      }
      case 'fontWeight':
        dom.style.fontWeight = String(WEIGHTS[value?.name] ?? 400);
        break;
      case 'foregroundColor': {
        const c = colorOf(value);
        if (c) dom.style.color = c;
        break;
      }
      case 'background': {
        const c = colorOf(value);
        if (c) dom.style.background = c;
        break;
      }
      case 'fill': {
        const c = colorOf(value);
        if (c) dom.style.background = c;
        break;
      }
      case 'cornerRadius':
        dom.style.borderRadius = `${value ?? 0}px`;
        break;
      case 'frame':
        if (value && typeof value === 'object') {
          if (value.width != null) dom.style.width = `${value.width}px`;
          if (value.height != null) dom.style.height = `${value.height}px`;
          dom.style.flex = 'none';
        }
        break;
      case 'padding': {
        const amount = typeof value === 'number' ? value : 12;
        dom.style.padding = `${amount}px`;
        break;
      }
      case 'tag':
        // Identity carried for Picker options; consumed by the parent, not visual.
        break;
      default:
        break;
    }
  }
}

// Build a DOM element for one UIIR node. `dispatch(id, event, value)` routes
// interaction back to the host.
function renderNode(node, dispatch) {
  if (!node) return el('div');
  const { id, kind, args = {}, modifiers = [], children = [] } = node;
  let dom;

  switch (kind) {
    case 'Text':
      dom = el('span', 'sw-text');
      dom.textContent = args.verbatim ?? '';
      break;

    case 'VStack':
      dom = el('div', 'sw-vstack');
      for (const child of children) dom.appendChild(renderNode(child, dispatch));
      break;

    case 'HStack':
      dom = el('div', 'sw-hstack');
      for (const child of children) dom.appendChild(renderNode(child, dispatch));
      break;

    case 'ZStack':
      dom = el('div', 'sw-zstack');
      for (const child of children) {
        const layer = el('div', 'sw-zlayer');
        layer.appendChild(renderNode(child, dispatch));
        dom.appendChild(layer);
      }
      break;

    case 'List':
      dom = el('div', 'sw-list');
      for (const child of children) dom.appendChild(renderNode(child, dispatch));
      break;

    case 'Section':
      dom = el('div', 'sw-section');
      if (args.header) {
        const head = el('div', 'sw-section-header');
        head.textContent = String(args.header).toUpperCase();
        dom.appendChild(head);
      }
      for (const child of children) dom.appendChild(renderNode(child, dispatch));
      break;

    case 'ForEach':
      dom = el('div', 'sw-foreach');
      for (const child of children) dom.appendChild(renderNode(child, dispatch));
      break;

    case 'Spacer':
      dom = el('div', 'sw-spacer');
      break;

    case 'Button':
      dom = el('button', 'sw-button');
      dom.textContent = args.title ?? '';
      dom.addEventListener('click', () => dispatch(id, 'tap', ''));
      break;

    case 'Toggle': {
      dom = el('label', 'sw-toggle');
      const label = el('span', 'sw-toggle-label');
      label.textContent = args.title ?? '';
      const input = el('input');
      input.type = 'checkbox';
      input.checked = args.isOn === true;
      input.addEventListener('change', () => dispatch(id, 'set', String(input.checked)));
      dom.append(label, input);
      break;
    }

    case 'TextField':
    case 'SecureField': {
      dom = el('input', 'sw-field');
      dom.type = kind === 'SecureField' ? 'password' : 'text';
      dom.placeholder = args.title ?? '';
      dom.value = args.text ?? '';
      dom.addEventListener('input', () => dispatch(id, 'set', JSON.stringify(dom.value)));
      break;
    }

    case 'Slider': {
      dom = el('input', 'sw-slider');
      dom.type = 'range';
      dom.min = String(args.lowerBound ?? 0);
      dom.max = String(args.upperBound ?? 1);
      dom.step = String(args.step ?? 0.01);
      dom.value = String(args.value ?? 0);
      dom.addEventListener('input', () => dispatch(id, 'set', String(Number(dom.value))));
      break;
    }

    case 'Stepper': {
      dom = el('div', 'sw-stepper');
      const label = el('span', 'sw-stepper-label');
      label.textContent = `${args.title ?? ''} ${args.value ?? 0}`.trim();
      const step = Number(args.step ?? 1);
      const lo = args.lowerBound;
      const hi = args.upperBound;
      const clamp = (v) => {
        if (lo != null && v < lo) return null;
        if (hi != null && v > hi) return null;
        return v;
      };
      const minus = el('button', 'sw-stepper-btn');
      minus.textContent = '−';
      minus.addEventListener('click', () => {
        const next = clamp(Number(args.value ?? 0) - step);
        if (next != null) dispatch(id, 'set', String(next));
      });
      const plus = el('button', 'sw-stepper-btn');
      plus.textContent = '+';
      plus.addEventListener('click', () => {
        const next = clamp(Number(args.value ?? 0) + step);
        if (next != null) dispatch(id, 'set', String(next));
      });
      dom.append(label, minus, plus);
      break;
    }

    case 'Picker': {
      dom = el('div', 'sw-picker');
      if (args.title) {
        const label = el('span', 'sw-picker-label');
        label.textContent = args.title;
        dom.appendChild(label);
      }
      const seg = el('div', 'sw-segmented');
      for (const opt of children) {
        const tagMod = (opt.modifiers ?? []).find((m) => m.name === 'tag');
        const tag = tagMod ? tagMod.value : opt.args?.verbatim;
        const btn = el('button', 'sw-segment');
        btn.textContent = opt.args?.verbatim ?? String(tag);
        if (String(tag) === String(args.selection)) btn.classList.add('active');
        btn.addEventListener('click', () => dispatch(id, 'set', JSON.stringify(String(tag))));
        seg.appendChild(btn);
      }
      dom.appendChild(seg);
      break;
    }

    case 'Circle':
      dom = el('div', 'sw-shape sw-circle');
      break;
    case 'Capsule':
      dom = el('div', 'sw-shape sw-capsule');
      break;
    case 'Ellipse':
      dom = el('div', 'sw-shape sw-ellipse');
      break;
    case 'Rectangle':
      dom = el('div', 'sw-shape sw-rect');
      break;
    case 'RoundedRectangle':
      dom = el('div', 'sw-shape sw-rect');
      dom.style.borderRadius = `${args.cornerRadius ?? 0}px`;
      break;

    default:
      dom = el('div', 'sw-unknown');
      dom.textContent = `⟨${kind}⟩`;
      for (const child of children) dom.appendChild(renderNode(child, dispatch));
      break;
  }

  applyModifiers(node, dom, modifiers);
  // Tag the element with its UIIR id so the caller can restore focus/caret to
  // the same node after a re-render (otherwise typing in a field loses focus).
  if (id != null) dom.dataset.uiirId = id;
  return dom;
}

// Render a full UIIR tree into `container`, wiring `dispatch` for interaction.
export function renderTree(container, tree, dispatch) {
  container.replaceChildren(renderNode(tree, dispatch));
}
