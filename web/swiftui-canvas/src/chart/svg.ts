// Shared SVG element factory for chart axis / mark painters.

const SVG_NS = "http://www.w3.org/2000/svg";

export function el<K extends keyof SVGElementTagNameMap>(
  tag: K,
  attrs: Record<string, string | number | undefined> = {},
): SVGElementTagNameMap[K] {
  const node = document.createElementNS(SVG_NS, tag);
  for (const [k, v] of Object.entries(attrs)) {
    if (v === undefined) continue;
    node.setAttribute(k, String(v));
  }
  return node;
}
