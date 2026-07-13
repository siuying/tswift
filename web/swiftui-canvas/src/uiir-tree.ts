// uiir-tree.ts — framework-neutral canonical UIIR node store for the web host.
// One flat index (id → record) that every patch mutates once; host projections
// (DOM, SVG, …) read from this store. TS analogue of the iOS RenderModel.
//
// No framework-specific kinds live here — only parent/child navigation.
// Projection seams decide what an ancestor means.
//
// Invariant: every mutator resolves the target id from the index first. A miss
// is an explicit typed violation — never a silent no-op or opportunistic index.

import type { Modifier, UiirNode } from "./uiir-types.js";

/** Flat canonical record for one UIIR node (tree edges via parent/child ids). */
export interface TreeRecord {
  id: string;
  kind: string;
  args: Record<string, unknown>;
  modifiers: Modifier[];
  parentId: string | null;
  childIds: string[];
}

/**
 * Typed invariant violation: a patch addressed a node id that is not in the
 * canonical store. Callers must not fall through to DOM-only mutation.
 */
export class UnknownNodeError extends Error {
  readonly code = "unknown_node" as const;
  constructor(readonly nodeId: string) {
    super(`UIIR invariant violation: unknown node '${nodeId}'`);
    this.name = "UnknownNodeError";
  }
}

/**
 * Mutable id → record index for the full mounted UIIR tree.
 * Structure ops (insert/remove/move/replace) and property ops (setArgs /
 * setModifiers / setText) update this store generically for every kind.
 */
export class UiirTree {
  private readonly byId = new Map<string, TreeRecord>();

  clear(): void {
    this.byId.clear();
  }

  get(id: string): TreeRecord | undefined {
    return this.byId.get(id);
  }

  has(id: string): boolean {
    return this.byId.has(id);
  }

  /** Immediate parent id, or `null` for the root / unknown id. */
  parentOf(id: string): string | null {
    return this.byId.get(id)?.parentId ?? null;
  }

  /**
   * Walk from `id` toward the root. With `includeSelf: true` (default), the
   * first yielded id is `id` itself when it is indexed.
   */
  *ancestors(id: string, opts?: { includeSelf?: boolean }): Generator<string> {
    const includeSelf = opts?.includeSelf !== false;
    let cur: string | null = includeSelf ? id : (this.byId.get(id)?.parentId ?? null);
    while (cur !== null) {
      const rec = this.byId.get(cur);
      if (!rec) return;
      yield cur;
      cur = rec.parentId;
    }
  }

  /**
   * Depth-first pre-order walk of the subtree rooted at `id` (inclusive).
   * Yields nothing when `id` is not indexed.
   */
  *subtreeIds(id: string): Generator<string> {
    const rec = this.byId.get(id);
    if (!rec) return;
    yield id;
    for (const cid of rec.childIds) {
      yield* this.subtreeIds(cid);
    }
  }

  /**
   * Index a nested `UiirNode` and all descendants under `parentId`.
   * Only called from creation patches (`mount` / `insert` / successful
   * `replace` after the old id was resolved). Does not splice into the
   * parent's `childIds` — structure ops own that edge update.
   */
  indexSubtree(node: UiirNode, parentId: string | null): void {
    this.byId.set(node.id, {
      id: node.id,
      kind: node.kind,
      args: node.args,
      modifiers: node.modifiers,
      parentId,
      childIds: node.children.map((c) => c.id),
    });
    for (const child of node.children) {
      this.indexSubtree(child, node.id);
    }
  }

  /** Mount-time: replace the entire store with `root` (parent null). */
  mount(root: UiirNode): void {
    this.byId.clear();
    this.indexSubtree(root, null);
  }

  /**
   * Insert a nested subtree as child `index` of `parentId`.
   * Parent must already be indexed; otherwise {@link UnknownNodeError}.
   * Indexes the incoming node only after the parent is resolved.
   */
  insert(parentId: string, index: number, node: UiirNode): void {
    const parent = this.require(parentId);
    const at = Math.max(0, Math.min(index, parent.childIds.length));
    parent.childIds.splice(at, 0, node.id);
    this.indexSubtree(node, parentId);
  }

  /**
   * Unlink `id` from its parent and drop the whole subtree from the index.
   * Throws {@link UnknownNodeError} when `id` is not indexed.
   */
  remove(id: string): void {
    const rec = this.require(id);
    if (rec.parentId !== null) {
      const parent = this.byId.get(rec.parentId);
      if (parent) {
        const idx = parent.childIds.indexOf(id);
        if (idx >= 0) parent.childIds.splice(idx, 1);
      }
    }
    this.unindexSubtree(id);
  }

  /**
   * Reorder `id` among `parentId`'s children to `index` (same semantics as the
   * DOM move: index is among remaining siblings after removal).
   * Throws when parent or id is missing, or id is not a child of parent.
   */
  move(parentId: string, id: string, index: number): void {
    const parent = this.require(parentId);
    this.require(id);
    const from = parent.childIds.indexOf(id);
    if (from < 0) {
      throw new UnknownNodeError(id);
    }
    parent.childIds.splice(from, 1);
    const to = Math.max(0, Math.min(index, parent.childIds.length));
    parent.childIds.splice(to, 0, id);
  }

  /**
   * Replace the subtree at `id` with `node` (typically `node.id === id`).
   * Preserves the slot in the parent's `childIds`. Requires `id` to already
   * be indexed — never indexes an orphan on a lookup miss.
   */
  replace(id: string, node: UiirNode): void {
    const old = this.require(id);
    const parentId = old.parentId;
    if (parentId !== null) {
      const parent = this.byId.get(parentId);
      if (parent) {
        const idx = parent.childIds.indexOf(id);
        if (idx >= 0) parent.childIds[idx] = node.id;
      }
    }
    this.unindexSubtree(id);
    this.indexSubtree(node, parentId);
  }

  setModifiers(id: string, modifiers: Modifier[]): void {
    this.require(id).modifiers = modifiers;
  }

  setArgs(id: string, args: Record<string, unknown>): void {
    this.require(id).args = args;
  }

  /**
   * Update the text payload for a leaf (Text → `args.verbatim`). Mutates the
   * existing args object so other keys are preserved.
   */
  setText(id: string, text: string): void {
    const rec = this.require(id);
    rec.args = { ...rec.args, verbatim: text };
  }

  /**
   * Materialize a nested `UiirNode` tree rooted at `id` for host projections.
   * Returns undefined if `id` is not indexed.
   */
  toUiirNode(id: string): UiirNode | undefined {
    const rec = this.byId.get(id);
    if (!rec) return undefined;
    const children: UiirNode[] = [];
    for (const cid of rec.childIds) {
      const child = this.toUiirNode(cid);
      if (child) children.push(child);
    }
    return {
      id: rec.id,
      kind: rec.kind,
      args: rec.args,
      modifiers: rec.modifiers,
      children,
    };
  }

  /** Resolve `id` or throw {@link UnknownNodeError}. */
  private require(id: string): TreeRecord {
    const rec = this.byId.get(id);
    if (!rec) throw new UnknownNodeError(id);
    return rec;
  }

  private unindexSubtree(id: string): void {
    const rec = this.byId.get(id);
    if (!rec) return;
    // Copy child ids — unindex mutates/deletes as it walks.
    for (const cid of [...rec.childIds]) this.unindexSubtree(cid);
    this.byId.delete(id);
  }
}
