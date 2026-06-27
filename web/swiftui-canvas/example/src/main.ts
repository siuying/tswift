import "./styles.css";
import "../../src/canvas.ts";
import type { Modifier, Patch, UiirNode } from "../../src/index.ts";

interface SwiftUICanvasElement extends HTMLElement {
  mount(tree: UiirNode): void;
  applyPatches(patches: Patch[]): void;
}

const sample = `import SwiftUI

struct ContentView: View {
    @State private var count = 0

    var body: some View {
        VStack {
            Text("Hello, SwiftUI")
                .font(.title)
                .fontWeight(.bold)
                .foregroundColor(.blue)
            Text("Tap the button to patch the preview")
                .font(.caption)
                .foregroundColor(.secondary)
            Button("Increment") { count += 1 }
                .padding(12)
                .background(Color.blue)
                .foregroundColor(.white)
                .cornerRadius(12)
        }
        .padding()
    }
}`;

const source = document.querySelector<HTMLTextAreaElement>("#source");
const renderButton = document.querySelector<HTMLButtonElement>("#render");
const status = document.querySelector<HTMLElement>("#status");
const sourceMeta = document.querySelector<HTMLElement>("#source-meta");
const uiir = document.querySelector<HTMLElement>("#uiir");
const canvas = document.querySelector<SwiftUICanvasElement>("#canvas");

if (!source || !renderButton || !status || !sourceMeta || !uiir || !canvas) {
  throw new Error("Example markup is missing required elements");
}

let latestTree: UiirNode | undefined;
let tapCount = 0;

source.value = localStorage.getItem("swiftui-canvas-example-source") ?? sample;
renderPreview();

source.addEventListener("input", () => {
  localStorage.setItem("swiftui-canvas-example-source", source.value);
  updateMeta();
  window.clearTimeout(Number(source.dataset.renderTimer ?? 0));
  source.dataset.renderTimer = String(window.setTimeout(renderPreview, 250));
});

renderButton.addEventListener("click", renderPreview);

canvas.addEventListener("swiftui-event", (event) => {
  tapCount += 1;
  status.textContent = `Button tap → setText patch #${tapCount}`;
  const firstText = latestTree?.children.find((child) => child.kind === "Text");
  if (firstText) canvas.applyPatches([{ op: "setText", id: firstText.id, text: `Tapped ${tapCount}` }]);
});

function renderPreview(): void {
  updateMeta();
  try {
    tapCount = 0;
    latestTree = swiftSnippetToUiir(source.value);
    canvas.mount(latestTree);
    uiir.textContent = JSON.stringify(latestTree, null, 2);
    status.textContent = "Preview rendered";
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error);
    status.textContent = message;
    uiir.textContent = message;
  }
}

function updateMeta(): void {
  const lines = source.value.split("\n").length;
  sourceMeta.textContent = `${lines} lines · ${source.value.length} chars`;
}

function swiftSnippetToUiir(swift: string): UiirNode {
  const rootKind = swift.includes("HStack") ? "HStack" : "VStack";
  const components = [...swift.matchAll(/\b(Text|Button)\s*\(\s*"((?:\\.|[^"\\])*)"/g)];

  if (components.length === 0) {
    throw new Error("Add at least one Text(\"…\") or Button(\"…\") to preview.");
  }

  return {
    id: "0",
    kind: rootKind,
    args: {},
    modifiers: swift.includes(".padding()") ? [{ name: "padding", value: null }] : [],
    children: components.map((match, index) => {
      const start = match.index ?? 0;
      const next = components[index + 1]?.index ?? swift.length;
      const chain = swift.slice(start, next);
      const kind = match[1] ?? "Text";
      const label = decodeSwiftString(match[2] ?? "");

      return {
        id: `0.${index}`,
        kind,
        args: kind === "Button" ? { title: label } : { verbatim: label },
        modifiers: modifiersFromChain(chain),
        children: [],
      };
    }),
  };
}

function modifiersFromChain(chain: string): Modifier[] {
  const modifiers: Modifier[] = [];
  const font = chain.match(/\.font\(\.(\w+)\)/)?.[1];
  const weight = chain.match(/\.fontWeight\(\.(\w+)\)/)?.[1];
  const foreground = chain.match(/\.foregroundColor\((?:Color\.)?\.?(\w+)\)/);
  const background = chain.match(/\.background\((?:Color\.)?\.?(\w+)\)/);
  const padding = chain.match(/\.padding\((\d+(?:\.\d+)?)?\)/);
  const cornerRadius = chain.match(/\.cornerRadius\((\d+(?:\.\d+)?)\)/)?.[1];

  if (font) modifiers.push({ name: "font", value: { $: "textStyle", name: font } });
  if (weight) modifiers.push({ name: "fontWeight", value: { $: "weight", name: weight } });
  if (foreground) modifiers.push({ name: "foregroundColor", value: colorToken(foreground[1] ?? "primary") });
  if (padding) modifiers.push({ name: "padding", value: padding[1] ? Number(padding[1]) : null });
  if (background) modifiers.push({ name: "background", value: colorToken(background[1] ?? "clear") });
  if (cornerRadius) modifiers.push({ name: "cornerRadius", value: Number(cornerRadius) });

  return modifiers;
}

function colorToken(name: string): { $: "color"; name: string } {
  return { $: "color", name };
}

function decodeSwiftString(value: string): string {
  return value.replace(/\\n/g, "\n").replace(/\\"/g, '"');
}
