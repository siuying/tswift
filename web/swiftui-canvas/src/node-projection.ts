// node-projection.ts — host-projection seam over the canonical UiirTree.
// After a generic patch mutates the tree, the applier asks this module to
// refresh any affected projections. Chart ownership (ancestor walk for kind
// "Chart") lives HERE — not in the framework-neutral store.
//
// Chart SVG is painted only through this seam (refresh). apply-patch never
// calls ChartProjection.mount directly — one update path for every patch.

import type { UiirNode } from "./uiir-types.js";
import type { UiirTree } from "./uiir-tree.js";
import { renderChart } from "./chart-render.js";

/** Framework-neutral projection of a canonical node onto a host element. */
export interface NodeProjection {
  mount(host: HTMLElement, node: UiirNode): void;
  refresh(host: HTMLElement, node: UiirNode): void;
}

/**
 * Chart host projection: paints deterministic SVG (+ legend) from the
 * canonical Chart subtree. Mark/axis leaves have no separate projection —
 * they exist only as canonical nodes that this projection reads.
 */
export const ChartProjection: NodeProjection = {
  mount(host: HTMLElement, node: UiirNode): void {
    renderChart(host, node);
  },
  refresh(host: HTMLElement, node: UiirNode): void {
    renderChart(host, node);
  },
};

/**
 * Nearest ancestor (or self) whose kind is `Chart`, decided by this
 * projection seam. Returns undefined when `nodeId` is not under a Chart.
 */
export function owningChartId(tree: UiirTree, nodeId: string): string | undefined {
  for (const id of tree.ancestors(nodeId, { includeSelf: true })) {
    const rec = tree.get(id);
    if (rec?.kind === "Chart") return id;
  }
  return undefined;
}

/**
 * After a canonical-tree mutation, refresh every Chart projection touched by
 * `nodeId`: the owning Chart (ancestor/self) and any Chart hosts in the
 * subtree (mount/insert/replace of a container that embeds Charts). Each
 * Chart id is refreshed at most once. Non-chart trees are a no-op.
 *
 * This is the sole Chart paint path — hosts are looked up by id so the
 * applier stays free of Chart-specific remove/setText/insert branches.
 */
export function refreshAffectedProjection(
  tree: UiirTree,
  nodeId: string,
  hostOf: (id: string) => HTMLElement | undefined,
): void {
  const chartIds = new Set<string>();
  const owner = owningChartId(tree, nodeId);
  if (owner) chartIds.add(owner);
  for (const id of tree.subtreeIds(nodeId)) {
    if (tree.get(id)?.kind === "Chart") chartIds.add(id);
  }
  for (const chartId of chartIds) {
    const host = hostOf(chartId);
    const uiir = tree.toUiirNode(chartId);
    if (!host || !uiir) continue;
    ChartProjection.refresh(host, uiir);
  }
}
