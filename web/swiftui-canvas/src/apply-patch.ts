// apply-patch.ts — the thin patch applier (plan §3.2/§4). It owns a
// `Map<nodeId, HTMLElement>` and mutates the host tree from the Rust diff
// engine's keyed patch stream. No vdom, no reconciler — the runtime already
// did the diffing.

import { applyModifiers, FRAME_ALIGN } from "./modifier-css.js";
import type { Modifier, UiirValue } from "./modifier-css.js";
import { sfGlyph } from "./sf-symbols.js";

/** A UIIR node from `tswift swiftui render` / patch payloads. */
export interface UiirNode {
  id: string;
  kind: string;
  args: Record<string, unknown>;
  modifiers: Modifier[];
  children: UiirNode[];
}

/** A patch op from `tswift swiftui dispatch`. */
export type Patch =
  | { op: "mount"; node: UiirNode }
  | { op: "insert"; parentId: string; index: number; node: UiirNode }
  | { op: "remove"; id: string }
  | { op: "replace"; id: string; node: UiirNode }
  | { op: "setText"; id: string; text: string }
  | { op: "setModifiers"; id: string; modifiers: Modifier[] }
  | { op: "setArgs"; id: string; args: Record<string, unknown> }
  | { op: "move"; parentId: string; id: string; index: number };

/** How a host node reports an event back to the runtime. */
export type EventSink = (id: string, event: string, value: unknown) => void;

/**
 * Applies a patch stream into a root element, keeping an id→element map. One
 * instance per mounted `<swiftui-canvas>`.
 */
export class PatchApplier {
  private nodes = new Map<string, HTMLElement>();
  /** Last-applied modifier list per node, so `setArgs` can rebuild a node's
   * base style from scratch and re-apply modifiers without ever capturing
   * modifier CSS into the base. */
  private mods = new Map<string, Modifier[]>();
  /** Ids carrying an `.onDisappear` handler, so `remove`/`forget` can fire a
   * `disappear` event as the host unmounts the node (ADR-0013 §3). */
  private disappearIds = new Set<string>();
  /** Per-node `AbortController` for the marker-modifier event listeners, so a
   * `setModifiers` reconcile can detach the previous listeners before wiring the
   * new marker set (a conditional `.onTapGesture`/`.onSubmit` toggled across a
   * re-render must not leave stale listeners). */
  private handlerAborts = new Map<string, AbortController>();

  constructor(
    private readonly root: HTMLElement,
    private readonly emit: EventSink,
  ) {}

  /** Apply an ordered batch of patches. */
  apply(patches: Patch[]): void {
    for (const patch of patches) this.applyOne(patch);
  }

  private applyOne(patch: Patch): void {
    switch (patch.op) {
      case "mount": {
        this.root.replaceChildren();
        this.nodes.clear();
        this.mods.clear();
        this.disappearIds.clear();
        for (const ac of this.handlerAborts.values()) ac.abort();
        this.handlerAborts.clear();
        this.root.appendChild(this.build(patch.node));
        break;
      }
      case "insert": {
        const parent = this.nodes.get(patch.parentId);
        if (!parent) return;
        // A child inserted under a Picker is an <option>, not a nested view.
        const el =
          parent instanceof HTMLSelectElement
            ? this.buildOption(patch.node)
            : this.build(patch.node);
        // A child inserted under a ZStack must overlap like the others.
        if (parent.dataset.zstack === "1") el.style.gridArea = "1 / 1";
        parent.insertBefore(el, patchRef(parent, patch.index));
        // A screen pushed onto a faux NavigationStack: re-sync bar + visibility.
        if (parent.dataset.kind === "NavigationStack") this.syncNavStack(parent);
        break;
      }
      case "remove": {
        const el = this.nodes.get(patch.id);
        const parent = el?.parentElement ?? null;
        el?.remove();
        this.forget(patch.id);
        // A screen popped off a faux NavigationStack: re-sync bar + visibility.
        if (parent?.dataset.kind === "NavigationStack") this.syncNavStack(parent);
        break;
      }
      case "move": {
        // Keyed reorder: relocate the existing element (preserving its DOM
        // node and any host state) to index `index`. The target is computed
        // among the *other* children so it is correct whether the element
        // moves left or right (insertBefore implicitly removes it first).
        const parent = this.nodes.get(patch.parentId);
        const el = this.nodes.get(patch.id);
        if (!parent || !el) return;
        const ref = patchRef(parent, patch.index, el);
        if (ref !== el) parent.insertBefore(el, ref);
        break;
      }
      case "replace": {
        const old = this.nodes.get(patch.id);
        if (!old) return;
        // Forget the old subtree's ids *before* building the replacement, which
        // re-registers the same ids — otherwise `forget` would delete the new
        // entries and later patches/events for the subtree become no-ops.
        this.forget(patch.id);
        // Replacing a Picker option keeps it an <option>.
        const el =
          old instanceof HTMLOptionElement
            ? this.buildOption(patch.node)
            : this.build(patch.node);
        old.replaceWith(el);
        // Restore the ZStack overlay placement the parent applies at build time
        // (a freshly built replacement has not been through that child loop).
        if (el.parentElement?.dataset.zstack === "1") el.style.gridArea = "1 / 1";
        break;
      }
      case "setText": {
        const el = this.nodes.get(patch.id);
        if (el) {
          el.textContent = patch.text;
          // Setting textContent drops any attached composite layers; rebuild
          // them from the remembered modifiers (#204).
          this.applyComposites(el, this.mods.get(patch.id) ?? []);
        }
        break;
      }
      case "setModifiers": {
        const el = this.nodes.get(patch.id);
        if (!el) break;
        // A Picker option's identity is its `tag` value, not CSS styling.
        if (el instanceof HTMLOptionElement) {
          const tag = patch.modifiers.find((m) => m.name === "tag");
          if (tag && (typeof tag.value === "string" || typeof tag.value === "number")) {
            el.value = String(tag.value);
          }
        } else {
          applyModifiers(el, patch.modifiers);
          this.mods.set(patch.id, patch.modifiers);
          this.applyComposites(el, patch.modifiers);
          // Reconcile event listeners against the new marker set: attach markers
          // that appeared, detach ones that were removed, and re-sync disappear
          // tracking. Not a fresh mount, so `onAppear` does not re-fire.
          this.attachHandlers(el, patch.id, patch.modifiers, false);
          // A screen root's navigationTitle may have changed: refresh the bar.
          if (el.parentElement?.dataset.kind === "NavigationStack") {
            this.syncNavStack(el.parentElement);
          }
        }
        break;
      }
      case "setArgs": {
        const existing = this.nodes.get(patch.id);
        if (existing) {
          // A few kinds pick their DOM shape from their args (ProgressView's
          // label wrapper, #206); restructure in place if that shape changed.
          const el = this.restructureForArgs(existing, patch.id, patch.args);
          // Rebuild from the pristine intrinsic style so arg-owned styling
          // (stack `gap`, Spacer `flex-basis`, …) is total: removed args revert
          // to their defaults instead of leaking. Then recompute the base and
          // re-apply the remembered modifiers — never capturing modifier CSS.
          el.style.cssText = el.dataset.intrinsicStyle ?? "";
          // The ZStack overlay places each child in the same grid cell at build
          // time; that placement lives outside the child's intrinsic style, so
          // the reset above drops it — restore it for any ZStack child (and for
          // a node just rebuilt by `restructureForArgs`, which is already in the
          // DOM under its parent).
          if (el.parentElement?.dataset.zstack === "1") el.style.gridArea = "1 / 1";
          this.applyArgs(el, el.dataset.kind ?? "", patch.args);
          el.dataset.baseStyle = el.style.cssText;
          const mods = this.mods.get(patch.id) ?? [];
          applyModifiers(el, mods);
          // The cssText reset drops the composite anchoring (position/isolation);
          // rebuild the layers from the remembered modifiers (#204).
          this.applyComposites(el, mods);
        }
        break;
      }
    }
  }

  /** Render a node's `background`/`overlay` arbitrary-view composites (#204) as
   * absolutely-positioned layers: a background paints behind the content
   * (negative z-index so normal-flow content/text sits above), an overlay in
   * front (and click-through). A color/token background is *not* a composite —
   * it stays in `modifier-css`. Idempotent: prior layers are cleared first. */
  private applyComposites(el: HTMLElement, modifiers: Modifier[]): void {
    el.querySelectorAll(":scope > .composite-layer").forEach((n) => n.remove());
    let anchored = false;
    for (const mod of modifiers) {
      if (mod.name !== "background" && mod.name !== "overlay") continue;
      const comp = compositeOf(mod.value);
      if (!comp) continue;
      const layer = document.createElement("div");
      layer.className = "composite-layer";
      const a = comp.alignment ? FRAME_ALIGN[comp.alignment] : undefined;
      layer.style.cssText =
        `position:absolute;inset:0;display:flex;pointer-events:none;` +
        `justify-content:${a?.justify ?? "center"};align-items:${a?.align ?? "center"};`;
      // Background paints behind the content (negative z, contained by the
      // `isolation` stacking context below); overlay paints in front. Both are
      // click-through so the host stays interactive (a composite's own controls
      // are decorative in v1, and would otherwise emit `0`-rooted event ids).
      layer.style.zIndex = mod.name === "background" ? "-1" : "1";
      layer.appendChild(this.buildDetached(comp.node));
      el.appendChild(layer);
      anchored = true;
    }
    if (anchored) {
      // A local stacking context so a `z-index:-1` background layer is contained
      // (paints above the host's own background, never behind its ancestors).
      el.style.isolation = "isolate";
      const pos = el.style.position;
      if (pos !== "absolute" && pos !== "fixed" && pos !== "relative") {
        el.style.position = "relative";
      }
    }
  }

  /** Build a nested composite subtree (#204) like `build`, but *without*
   * registering it in the patch-addressed node map — its ids are `0`-rooted and
   * would collide with the real tree, and it is re-rendered wholesale whenever
   * the host modifier changes. (An interactive control inside a composite is not
   * individually addressable in v1.) */
  private buildDetached(node: UiirNode): HTMLElement {
    const el = this.element(node);
    el.dataset.kind = node.kind;
    el.dataset.intrinsicStyle = el.style.cssText;
    this.applyArgs(el, node.kind, node.args);
    el.dataset.baseStyle = el.style.cssText;
    applyModifiers(el, node.modifiers);
    this.applyComposites(el, node.modifiers);
    for (const child of node.children) {
      const childEl = this.buildDetached(child);
      if (el.dataset.zstack === "1") childEl.style.gridArea = "1 / 1";
      el.appendChild(childEl);
    }
    return el;
  }

  /** A `ProgressView`'s DOM shape depends on whether it has a title label (a
   * bare `<progress>` vs a labelled wrapper, #206). When a `setArgs` flips that
   * presence, rebuild the element in place so the structure matches and re-register
   * it; otherwise the existing element is returned unchanged. ProgressView is a
   * leaf, so no child subtree needs rebuilding. */
  private restructureForArgs(
    el: HTMLElement,
    id: string,
    args: Record<string, unknown>,
  ): HTMLElement {
    if ((el.dataset.kind ?? "") !== "ProgressView") return el;
    const hasLabel = !(el instanceof HTMLProgressElement); // a wrapper carries a label
    const wantsLabel = typeof args.label === "string";
    if (hasLabel === wantsLabel) return el;
    const node: UiirNode = {
      id,
      kind: "ProgressView",
      args,
      modifiers: this.mods.get(id) ?? [],
      children: [],
    };
    const fresh = this.build(node); // re-registers in this.nodes + dataset
    el.replaceWith(fresh);
    return fresh;
  }

  /** Drop `id` and any descendant ids from the node map, firing `disappear`
   * for any unmounted node that registered `.onDisappear` (ADR-0013 §3). */
  private forget(id: string): void {
    const prefix = `${id}.`;
    for (const key of [...this.nodes.keys()]) {
      if (key === id || key.startsWith(prefix)) {
        this.nodes.delete(key);
        this.mods.delete(key);
        this.handlerAborts.get(key)?.abort();
        this.handlerAborts.delete(key);
        if (this.disappearIds.delete(key)) this.emit(key, "disappear", null);
      }
    }
  }

  /** Wire the lifecycle/gesture/submit marker modifiers on a node to DOM
   * listeners that report the corresponding runtime event (ADR-0013 §3). The
   * runtime carries only markers — the captured closures live in its handler
   * map — so the host's only job is to attach listeners and fire event names.
   * (`onChange` is runtime-internal and never appears here.)
   *
   * Idempotent + reconciling: each call first aborts the node's previous handler
   * listeners (so a marker removed across a `setModifiers` re-render detaches its
   * listener) and re-syncs `disappearIds`. `fireAppear` is true only on the
   * initial mount — a persisted node that merely *gains* an `onAppear` marker
   * mid-life is already on screen, so re-firing `appear` would be spurious. */
  private attachHandlers(
    el: HTMLElement,
    id: string,
    modifiers: Modifier[],
    fireAppear: boolean,
  ): void {
    // Detach any listeners from a prior build/reconcile, then re-sync tracking.
    this.handlerAborts.get(id)?.abort();
    const ac = new AbortController();
    this.handlerAborts.set(id, ac);
    const { signal } = ac;
    this.disappearIds.delete(id);
    // A `Button` already emits `tap` from its own click listener and owns the
    // `tap` event authoritatively (the runtime keeps the Button action, see
    // `modifier_on_tap_gesture`), so wiring a gesture `click` here would
    // double-emit `tap`; skip tap wiring on a Button.
    const isButton = el instanceof HTMLButtonElement;
    for (const mod of modifiers) {
      switch (mod.name) {
        case "onTapGesture": {
          if (isButton) break;
          // Honor `count:` — a multi-tap (count >= 2) fires on `dblclick` to
          // match SwiftUI's `.onTapGesture(count:)`; the default is a click.
          const count = Number(
            (mod.value as { count?: number } | null)?.count ?? 1,
          );
          const eventName = count >= 2 ? "dblclick" : "click";
          el.addEventListener(eventName, () => this.emit(id, "tap", null), { signal });
          el.style.cursor = "pointer";
          break;
        }
        case "onLongPressGesture": {
          // Press-and-hold: emit `longPress` once the pointer is held past the
          // minimum duration (default 0.5s) without lifting.
          const min =
            typeof (mod.value as { minimumDuration?: number } | null)?.minimumDuration ===
            "number"
              ? (mod.value as { minimumDuration: number }).minimumDuration * 1000
              : 500;
          let timer: ReturnType<typeof setTimeout> | undefined;
          const cancel = () => {
            if (timer !== undefined) clearTimeout(timer);
            timer = undefined;
          };
          el.addEventListener(
            "pointerdown",
            () => {
              cancel();
              timer = setTimeout(() => this.emit(id, "longPress", null), min);
            },
            { signal },
          );
          el.addEventListener("pointerup", cancel, { signal });
          el.addEventListener("pointerleave", cancel, { signal });
          el.style.cursor = "pointer";
          break;
        }
        case "onSubmit":
          // A text field submit: Enter in the input, matching `.submitLabel`.
          el.addEventListener(
            "keydown",
            (e) => {
              if ((e as KeyboardEvent).key === "Enter") this.emit(id, "submit", null);
            },
            { signal },
          );
          break;
        case "onAppear":
          // Fire once the node is mounted; defer so the whole batch lands first.
          if (fireAppear) queueMicrotask(() => this.emit(id, "appear", null));
          break;
        case "onDisappear":
          this.disappearIds.add(id);
          break;
        default:
          break;
      }
    }
  }

  /** Build a DOM subtree for a UIIR node and register it (and descendants). */
  private build(node: UiirNode): HTMLElement {
    const el = this.element(node);
    el.dataset.kind = node.kind;
    el.dataset.id = node.id;
    // Pristine style straight from `element()` — no args, no modifiers — so
    // `setArgs` can revert arg-owned style to defaults.
    el.dataset.intrinsicStyle = el.style.cssText;
    this.applyArgs(el, node.kind, node.args);
    // Capture the base style *after* args so arg-derived styling (e.g. a
    // RoundedRectangle's corner radius) survives the idempotent reset that
    // `applyModifiers`/`setModifiers` performs.
    el.dataset.baseStyle = el.style.cssText;
    applyModifiers(el, node.modifiers);
    this.mods.set(node.id, node.modifiers);
    this.applyComposites(el, node.modifiers);
    this.attachHandlers(el, node.id, node.modifiers, true);
    this.nodes.set(node.id, el);
    if (node.kind === "Picker" && el instanceof HTMLSelectElement) {
      // A Picker's tagged children become <option>s, not nested views.
      for (const child of node.children) {
        el.appendChild(this.buildOption(child));
      }
      if (typeof node.args.selection === "string") el.value = node.args.selection;
    } else if (node.kind === "TabView") {
      this.buildTabView(el, node);
    } else if (node.kind === "NavigationStack") {
      this.buildNavStack(el, node);
    } else if (node.kind === "AsyncImage" && node.args.phase !== undefined) {
      // v1.5: render children (runtime-owned phase content), start image preload
      // when the node arrives in the "empty" phase (initial mount).
      for (const child of node.children) {
        el.appendChild(this.build(child));
      }
      if (
        typeof node.args.url === "string" &&
        node.args.phase === "empty"
      ) {
        this.startAsyncImageLoad(node.id, node.args.url);
      }
    } else {
      for (const child of node.children) {
        const childEl = this.build(child);
        // ZStack overlays its children: place each in the same grid cell.
        if (el.dataset.zstack === "1") childEl.style.gridArea = "1 / 1";
        el.appendChild(childEl);
      }
    }
    return el;
  }

  /** Build a faux `NavigationStack` (ADR-0013 §1): a rendered nav bar (a back
   * button + the topmost screen's `navigationTitle`) plus the screen children,
   * only the topmost shown. The `.nav-bar` is a synthetic element excluded from
   * patch addressing (like a TabView's tab bar); push/pop arrive as ordinary
   * `insert`/`remove` patches, after which the stack is re-synced. */
  private buildNavStack(el: HTMLElement, node: UiirNode): void {
    const bar = document.createElement("div");
    bar.className = "nav-bar";
    bar.style.cssText =
      "display:flex;flex-direction:row;align-items:center;gap:8px;min-height:44px;" +
      "border-bottom:1px solid #e0e0e0;padding:0 8px;";
    const back = document.createElement("button");
    back.className = "nav-back";
    back.textContent = "\u2039 Back";
    back.style.cssText =
      "border:none;background:none;cursor:pointer;color:#007aff;font-size:15px;";
    back.addEventListener("click", () => this.emit(node.id, "back", null));
    const title = document.createElement("span");
    title.className = "nav-title";
    title.style.cssText = "font-weight:600;flex:1;text-align:center;";
    bar.append(back, title);
    el.appendChild(bar);
    for (const child of node.children) {
      el.appendChild(this.build(child));
    }
    this.syncNavStack(el);
  }

  /** Reflect a `NavigationStack`'s current stack: show only the topmost screen,
   * reveal the back button when more than one screen is present, and set the
   * bar title from the topmost screen's `navigationTitle` modifier. */
  private syncNavStack(el: HTMLElement): void {
    const screens = Array.from(el.children).filter(
      (c) => !c.classList.contains("nav-bar"),
    ) as HTMLElement[];
    screens.forEach((s, i) => {
      s.style.display = i === screens.length - 1 ? "" : "none";
    });
    const bar = el.querySelector(":scope > .nav-bar");
    if (!bar) return;
    const back = bar.querySelector(".nav-back") as HTMLElement | null;
    if (back) back.style.visibility = screens.length > 1 ? "visible" : "hidden";
    const title = bar.querySelector(".nav-title") as HTMLElement | null;
    if (title) {
      const top = screens[screens.length - 1];
      const id = top?.dataset.id;
      const mods = id ? this.mods.get(id) ?? [] : [];
      const navTitle = mods.find((m) => m.name === "navigationTitle");
      title.textContent =
        navTitle && typeof navTitle.value === "string" ? navTitle.value : "";
    }
  }

  /** Build a `TabView` (ADR-0013 §2): every tab renders eagerly as a child;
   * only the selected one is shown. A synthetic bottom `.tab-bar` (excluded from
   * patch addressing, like a Section header) carries one button per tab, built
   * from the child's `tabItem` marker; a click emits `select` with the tab's
   * tag-or-index. `selection` updates arrive via `setArgs` → `syncTabView`. */
  private buildTabView(el: HTMLElement, node: UiirNode): void {
    const tags = node.children.map((c, i) => tabTagOrIndex(c, i));
    el.dataset.tabTags = JSON.stringify(tags);
    for (const child of node.children) {
      el.appendChild(this.build(child));
    }
    const bar = document.createElement("div");
    bar.className = "tab-bar";
    bar.style.cssText =
      "display:flex;flex-direction:row;justify-content:space-around;" +
      "border-top:1px solid #e0e0e0;";
    node.children.forEach((child, i) => {
      const item = document.createElement("button");
      item.className = "tab-item";
      item.style.cssText =
        "display:flex;flex-direction:column;align-items:center;gap:2px;flex:1;" +
        "border:none;background:none;padding:6px;cursor:pointer;";
      const marker = child.modifiers.find((m) => m.name === "tabItem");
      const comp = marker ? compositeOf(marker.value) : undefined;
      if (comp) item.appendChild(this.buildDetached(comp.node));
      const tag = tags[i];
      item.addEventListener("click", () => this.emit(node.id, "select", tag));
      bar.appendChild(item);
    });
    el.appendChild(bar);
    this.syncTabView(el, node.args.selection);
  }

  /** Reflect a `TabView`'s current `selection` (a tag-or-index): show only the
   * matching content child and highlight its tab-bar button. Falls back to the
   * first tab when the selection matches none. */
  private syncTabView(el: HTMLElement, selection: unknown): void {
    const tags = JSON.parse(el.dataset.tabTags ?? "[]") as unknown[];
    const contents = Array.from(el.children).filter(
      (c) => !c.classList.contains("tab-bar"),
    ) as HTMLElement[];
    let active = tags.findIndex((t) => t === selection);
    if (active < 0) active = 0;
    contents.forEach((c, i) => {
      c.style.display = i === active ? "" : "none";
    });
    const bar = el.querySelector(":scope > .tab-bar");
    if (bar) {
      Array.from(bar.children).forEach((b, i) => {
        (b as HTMLElement).style.opacity = i === active ? "1" : "0.5";
      });
    }
  }

  /** Start a background image load for a v1.5 `AsyncImage` node id (ADR-0013
   * §4) and emit `imagePhase` events when the URL resolves or fails.
   *
   * Stores `url` in `el.dataset.asyncImageUrl` so `applyArgs` can detect URL
   * changes and call this method again with the new URL. Callbacks are
   * guarded by a double liveness check: the node must still be in the map
   * (not unmounted) **and** the stored URL must still match (no newer load
   * started for the same node) before emitting an event. This prevents both
   * stale-after-unmount and stale-after-URL-change races (Fix #2/#1). */
  private startAsyncImageLoad(nodeId: string, url: string): void {
    if (!url) return;
    // Record the URL on the element so callbacks and applyArgs can compare.
    const el = this.nodes.get(nodeId);
    if (el) el.dataset.asyncImageUrl = url;
    const img = document.createElement("img");
    img.onload = () => {
      const live = this.nodes.get(nodeId);
      if (live && live.dataset.asyncImageUrl === url)
        this.emit(nodeId, "imagePhase", "success");
    };
    img.onerror = () => {
      const live = this.nodes.get(nodeId);
      if (live && live.dataset.asyncImageUrl === url)
        this.emit(nodeId, "imagePhase", "failure");
    };
    img.src = url;
  }

  /** Build a Picker `<option>` from a tagged child view (label + tag value). */
  private buildOption(child: UiirNode): HTMLOptionElement {
    const opt = document.createElement("option");
    const tag = child.modifiers.find((m) => m.name === "tag");
    if (tag && (typeof tag.value === "string" || typeof tag.value === "number")) {
      opt.value = String(tag.value);
    } else {
      // Untagged option: an empty value, so selecting it can't write the label
      // text into the binding.
      opt.value = "";
    }
    if (typeof child.args.verbatim === "string") opt.textContent = child.args.verbatim;
    this.nodes.set(child.id, opt);
    return opt;
  }

  /** Create the host primitive for a SwiftUI concept (the lowering boundary). */
  private element(node: UiirNode): HTMLElement {
    switch (node.kind) {
      case "VStack":
      case "LazyVStack": {
        const el = document.createElement("div");
        el.style.cssText = "display:flex;flex-direction:column;align-items:center;gap:8px;";
        return el;
      }
      case "HStack":
      case "LazyHStack": {
        const el = document.createElement("div");
        el.style.cssText = "display:flex;flex-direction:row;align-items:center;gap:8px;";
        return el;
      }
      case "Grid": {
        // A real 2-D grid so columns align across rows. The column count is the
        // widest GridRow; each GridRow is `display:contents` so its cells become
        // direct grid items flowing row by row.
        const el = document.createElement("div");
        const cols = Math.max(1, ...node.children.map((r) => r.children.length));
        el.style.cssText = `display:inline-grid;grid-template-columns:repeat(${cols}, auto);gap:8px;justify-items:center;align-items:center;`;
        return el;
      }
      case "GridRow": {
        // Transparent: its cells participate directly in the parent Grid.
        const el = document.createElement("div");
        el.style.cssText = "display:contents;";
        return el;
      }
      case "LazyVGrid": {
        // CSS grid whose column tracks come from the `columns: [GridItem]` arg
        // (set in applyArgs); children flow row by row (#205).
        const el = document.createElement("div");
        el.style.cssText = "display:grid;gap:8px;justify-items:center;align-items:start;";
        return el;
      }
      case "LazyHGrid": {
        // Row tracks from `rows: [GridItem]`; children flow column by column.
        const el = document.createElement("div");
        el.style.cssText =
          "display:grid;grid-auto-flow:column;gap:8px;justify-items:center;align-items:center;";
        return el;
      }
      case "ZStack": {
        // Depth container: children overlap, centered, back-to-front.
        const el = document.createElement("div");
        el.style.cssText = "display:grid;place-items:center;";
        el.dataset.zstack = "1";
        return el;
      }
      case "ForEach":
      case "Group": {
        // A transparent group: its children lay out as if direct children of
        // the surrounding container (no box of its own).
        const el = document.createElement("div");
        el.style.cssText = "display:contents;";
        return el;
      }
      case "Divider": {
        // A thin rule; stretches across the container's cross axis.
        const el = document.createElement("div");
        el.style.cssText =
          "flex:0 0 auto;align-self:stretch;min-width:1px;min-height:1px;background:var(--swiftui-separator, #e0e0e0);";
        return el;
      }
      case "ScrollView": {
        // A scroll container. Default axis is vertical; `axes` arg may switch it.
        const el = document.createElement("div");
        el.style.cssText = "display:flex;flex-direction:column;overflow-y:auto;overflow-x:hidden;";
        return el;
      }
      case "List":
      case "Form": {
        // A vertically scrolling, grouped list of rows.
        const el = document.createElement("div");
        el.style.cssText =
          "display:flex;flex-direction:column;overflow-y:auto;border:1px solid #e0e0e0;border-radius:8px;";
        return el;
      }
      case "Section": {
        // A grouped block; its `header` arg renders as a caption above rows.
        const el = document.createElement("div");
        el.style.cssText = "display:flex;flex-direction:column;";
        return el;
      }
      case "Circle":
      case "Ellipse":
      case "Rectangle":
      case "RoundedRectangle":
      case "Capsule": {
        const el = document.createElement("div");
        el.style.cssText = `${shapeRadius(node)}background:currentColor;width:40px;height:40px;`;
        return el;
      }
      case "Spacer": {
        const el = document.createElement("div");
        el.style.cssText = "flex:1 1 auto;";
        return el;
      }
      case "Label": {
        // An icon + title row.
        const el = document.createElement("span");
        el.style.cssText = "display:inline-flex;align-items:center;gap:6px;";
        const icon = document.createElement("span");
        icon.className = "label-icon";
        const text = document.createElement("span");
        text.className = "label-text";
        el.append(icon, text);
        return el;
      }
      case "Image": {
        // A remote image (from AsyncImage content closure) uses an <img> element;
        // a system/bundle image uses a <span> with text content.
        if (typeof node.args.url === "string") {
          const img = document.createElement("img");
          img.alt = "";
          img.style.cssText = "object-fit:contain;max-width:100%;max-height:100%;display:block;";
          img.src = node.args.url;
          return img;
        }
        const el = document.createElement("span");
        el.style.cssText = "display:inline-flex;align-items:center;justify-content:center;";
        return el;
      }
      case "AsyncImage": {
        // v1 bare (no `phase` arg): native <img> element — host loads natively,
        // no imagePhase events emitted (ADR-0013 §4).
        if (node.args.phase === undefined) {
          const img = document.createElement("img");
          img.alt = "";
          img.style.cssText = "object-fit:contain;max-width:100%;max-height:100%;display:block;";
          if (typeof node.args.url === "string") img.src = node.args.url;
          return img;
        }
        // v1.5 with closures: a transparent container; the runtime-evaluated
        // children are the phase-appropriate content.
        const el = document.createElement("div");
        el.style.cssText = "display:contents;";
        return el;
      }
      case "ProgressView": {
        // Native <progress>: determinate when `value` is set, else indeterminate.
        // With a title label (#206), wrap the bar in a labelled column; without
        // one, stay a bare <progress> so existing goldens are unchanged.
        const bar = document.createElement("progress");
        bar.max = 1;
        if (typeof node.args.label !== "string") return bar;
        const el = document.createElement("div");
        el.style.cssText = "display:flex;flex-direction:column;gap:4px;";
        const label = document.createElement("span");
        label.className = "progress-label";
        label.style.cssText = "font-size:13px;";
        bar.classList.add("progress-bar");
        el.append(label, bar);
        return el;
      }
      case "Button": {
        const el = document.createElement("button");
        el.addEventListener("click", () => this.emit(node.id, "tap", null));
        return el;
      }
      case "NavigationStack": {
        // A faux navigation container (ADR-0013 §1): a rendered nav bar plus the
        // stack of screen children, only the topmost shown. `position:relative`
        // anchors the bar; `buildNavStack` adds the bar and syncs visibility.
        const el = document.createElement("div");
        el.style.cssText =
          "display:flex;flex-direction:column;position:relative;flex:1;min-height:0;";
        return el;
      }
      case "NavigationLink": {
        // A tappable row that pushes its (host-invisible) destination: a `tap`
        // routes to the runtime, which appends the destination to the stack.
        const el = document.createElement("div");
        el.style.cssText =
          "display:flex;flex-direction:row;align-items:center;justify-content:space-between;" +
          "gap:8px;cursor:pointer;color:#007aff;";
        el.addEventListener("click", () => this.emit(node.id, "tap", null));
        return el;
      }
      case "Toggle": {
        // A labelled switch: a checkbox emits `set` with its new boolean.
        const el = document.createElement("label");
        el.style.cssText = "display:flex;flex-direction:row;align-items:center;gap:8px;";
        const input = document.createElement("input");
        input.type = "checkbox";
        input.addEventListener("change", () => this.emit(node.id, "set", input.checked));
        const text = document.createElement("span");
        el.append(input, text);
        return el;
      }
      case "TextField":
      case "SecureField": {
        // A text input emitting `set` with its string value on each edit. The
        // base box (padding/border) lives in the stylesheet, not inline, so a
        // theme or a user modifier can override it without `!important`.
        const input = document.createElement("input");
        input.type = node.kind === "SecureField" ? "password" : "text";
        input.addEventListener("input", () =>
          this.emit(node.id, "set", input.value),
        );
        return input;
      }
      case "Slider": {
        // A range input emitting `set` with its numeric value as the user drags.
        const input = document.createElement("input");
        input.type = "range";
        input.addEventListener("input", () =>
          this.emit(node.id, "set", Number(input.value)),
        );
        return input;
      }
      case "Stepper": {
        // A label plus -/+ buttons; each click computes the clamped next value
        // from the node's args and emits `set` with it.
        const el = document.createElement("div");
        el.style.cssText = "display:flex;flex-direction:row;align-items:center;gap:8px;";
        const label = document.createElement("span");
        label.className = "stepper-label";
        const dec = document.createElement("button");
        dec.textContent = "\u2212";
        const inc = document.createElement("button");
        inc.textContent = "+";
        const bump = (dir: number) => {
          // The current value/step/bounds live on the element's dataset, kept
          // fresh by `applyArgs`. We optimistically advance the dataset before
          // emitting so a rapid second click computes from the new value (not
          // the stale one that a not-yet-applied `setArgs` would carry); the
          // runtime's authoritative `setArgs` reconciles afterward.
          const cur = Number(el.dataset.value ?? "0");
          const step = Number(el.dataset.step ?? "1");
          let next = cur + dir * step;
          if (el.dataset.lowerBound !== undefined)
            next = Math.max(next, Number(el.dataset.lowerBound));
          if (el.dataset.upperBound !== undefined)
            next = Math.min(next, Number(el.dataset.upperBound));
          if (next !== cur) {
            el.dataset.value = String(next);
            this.emit(node.id, "set", next);
          }
        };
        dec.addEventListener("click", () => bump(-1));
        inc.addEventListener("click", () => bump(1));
        el.append(label, dec, inc);
        return el;
      }
      case "Picker": {
        // A choice control: a <select> whose options are the tagged children.
        // It emits `set` with the chosen option's tag value.
        const sel = document.createElement("select");
        sel.addEventListener("change", () => this.emit(node.id, "set", sel.value));
        return sel;
      }
      case "TabView": {
        // A tabbed container: a content stack above a synthetic bottom tab bar
        // (built in `buildTabView`). Runtime owns selection (ADR-0013 §2).
        const el = document.createElement("div");
        el.style.cssText = "display:flex;flex-direction:column;position:relative;";
        return el;
      }
      case "Text":
      default:
        return document.createElement("span");
    }
  }

  /** Reflect constructor args onto the host node (text/title/checked state). */
  private applyArgs(el: HTMLElement, kind: string, args: Record<string, unknown>): void {
    if (kind === "Text" && typeof args.verbatim === "string") {
      el.textContent = args.verbatim;
    } else if (
      kind === "VStack" ||
      kind === "HStack" ||
      kind === "ZStack" ||
      kind === "LazyVStack" ||
      kind === "LazyHStack"
    ) {
      // `spacing:` overrides the default inter-child gap (C2/C6). ZStack ignores it.
      if (typeof args.spacing === "number" && kind !== "ZStack") {
        el.style.gap = `${args.spacing}px`;
      }
      // `alignment:` positions children on the stack's cross axis (C2, #189).
      applyStackAlignment(el, kind, args.alignment);
    } else if (kind === "LazyVGrid" || kind === "LazyHGrid") {
      // Build the CSS grid template from the `columns`/`rows` GridItem array.
      const tracks = (kind === "LazyVGrid" ? args.columns : args.rows) as unknown;
      const template = gridTemplate(tracks);
      if (template) {
        if (kind === "LazyVGrid") el.style.gridTemplateColumns = template;
        else el.style.gridTemplateRows = template;
      }
      if (typeof args.spacing === "number") el.style.gap = `${args.spacing}px`;
      // `alignment:` positions items within their track on the grid's cross
      // axis (LazyVGrid → justify-items, LazyHGrid → align-items) (#205).
      const gridAlign = (args.alignment as { name?: string } | undefined)?.name;
      const place = gridAlign ? GRID_ITEM_ALIGN[gridAlign] : undefined;
      if (place) {
        if (kind === "LazyVGrid") el.style.justifyItems = place;
        else el.style.alignItems = place;
      }
    } else if (kind === "Spacer") {
      // `minLength:` is the spacer's minimum length along the stack axis (C2).
      if (typeof args.minLength === "number") {
        el.style.flexBasis = `${args.minLength}px`;
      }
    } else if (kind === "Label") {
      const icon = el.querySelector(".label-icon");
      const text = el.querySelector(".label-text");
      if (icon) icon.textContent = typeof args.systemImage === "string" ? sfGlyph(args.systemImage) : "";
      if (text) text.textContent = typeof args.title === "string" ? args.title : "";
    } else if (kind === "Image") {
      if (el instanceof HTMLImageElement) {
        // Remote image from an AsyncImage content closure.
        if (typeof args.url === "string" && el.src !== args.url) el.src = args.url;
      } else if (typeof args.systemName === "string") {
        el.textContent = sfGlyph(args.systemName);
        el.removeAttribute("title"); // clear any stale asset-name tooltip
      } else if (typeof args.name === "string") {
        // No asset bundle on the web; show a placeholder labelled by name.
        el.textContent = "\u{1F5BC}"; // 🖼
        el.title = args.name;
      } else {
        el.textContent = "";
        el.removeAttribute("title");
      }
    } else if (kind === "AsyncImage") {
      if (el instanceof HTMLImageElement) {
        // v1 bare: update the img src when the URL arg changes.
        if (typeof args.url === "string" && el.src !== args.url) el.src = args.url;
      } else if (args.phase !== undefined) {
        // v1.5: a `setArgs` with a changed URL means the image source changed
        // (the runtime has already reset the phase to "empty"). Re-trigger the
        // background load so the host fires fresh `imagePhase` events for the
        // new URL (Fix #1).
        const newUrl = typeof args.url === "string" ? args.url : "";
        const prevUrl = el.dataset.asyncImageUrl ?? "";
        if (newUrl && newUrl !== prevUrl) {
          const nodeId = el.dataset.id ?? "";
          if (nodeId) this.startAsyncImageLoad(nodeId, newUrl);
        }
      }
    } else if (kind === "ProgressView") {
      // The bar is the element itself (no label) or the wrapped `.progress-bar`.
      const bar =
        el instanceof HTMLProgressElement
          ? el
          : (el.querySelector("progress.progress-bar") as HTMLProgressElement | null);
      const label = el.querySelector(".progress-label");
      if (label) label.textContent = typeof args.label === "string" ? args.label : "";
      if (bar) {
        if (typeof args.value === "number") {
          bar.max = typeof args.total === "number" ? args.total : 1;
          bar.value = args.value;
        } else {
          bar.removeAttribute("value"); // indeterminate
        }
      }
    } else if (kind === "ScrollView") {
      // `axes` switches the scroll direction (C3); default is vertical.
      const axis = args.axes as { name?: string } | undefined;
      if (axis?.name === "horizontal") {
        el.style.flexDirection = "row";
        el.style.overflowX = "auto";
        el.style.overflowY = "hidden";
      }
    } else if (kind === "Button" && typeof args.title === "string") {
      el.textContent = args.title;
    } else if (kind === "NavigationLink" && typeof args.title === "string") {
      // The title-string form (no label children) renders its title as text; a
      // trailing disclosure chevron hints the push affordance.
      el.textContent = args.title;
      const chevron = document.createElement("span");
      chevron.textContent = "\u203A";
      chevron.style.opacity = "0.5";
      el.appendChild(chevron);
    } else if (kind === "RoundedRectangle" && typeof args.cornerRadius === "number") {
      el.style.borderRadius = `${args.cornerRadius}px`;
    } else if (kind === "Section") {
      // The header caption is a synthetic child, kept out of the patch-addressed
      // child list (see `patchRef`). Add/update it, or remove it if the arg is
      // gone, without disturbing row positions.
      let head = el.querySelector(":scope > .section-header") as HTMLElement | null;
      if (typeof args.header === "string") {
        if (!head) {
          head = document.createElement("div");
          head.className = "section-header";
          head.style.cssText =
            "font-size:13px;font-weight:600;text-transform:uppercase;color:#6b6b6b;padding:8px 12px;";
          el.prepend(head);
        }
        head.textContent = args.header;
      } else {
        head?.remove();
      }
    } else if (kind === "Toggle") {
      const input = el.querySelector("input");
      const label = el.querySelector("span");
      if (input instanceof HTMLInputElement && typeof args.isOn === "boolean") {
        input.checked = args.isOn;
      }
      if (label && typeof args.title === "string") {
        label.textContent = args.title;
      }
    } else if (kind === "TextField" || kind === "SecureField") {
      if (el instanceof HTMLInputElement) {
        // Only overwrite when the model and DOM disagree, so we don't fight the
        // user's caret mid-edit on the echo back.
        if (typeof args.text === "string" && el.value !== args.text) {
          el.value = args.text;
        }
        if (typeof args.title === "string") el.placeholder = args.title;
      }
    } else if (kind === "Slider" && el instanceof HTMLInputElement) {
      if (typeof args.lowerBound === "number") el.min = String(args.lowerBound);
      if (typeof args.upperBound === "number") el.max = String(args.upperBound);
      // Always set step: `"any"` restores continuous dragging when a previous
      // `step:` is removed, instead of leaving the browser default of `1`.
      el.step = typeof args.step === "number" ? String(args.step) : "any";
      if (typeof args.value === "number" && Number(el.value) !== args.value) {
        el.value = String(args.value);
      }
      // Expose the value as a 0–100% of the range so a theme can paint a filled
      // track (e.g. iOS). Theme-agnostic and unused by the default skin, so it
      // does not affect the default rendering.
      const lo = Number(el.min || "0");
      const hi = Number(el.max || "1");
      const cur = Number(el.value || "0");
      const pct = hi > lo ? ((cur - lo) / (hi - lo)) * 100 : 0;
      el.style.setProperty("--swiftui-slider-fill", `${pct}%`);
    } else if (kind === "Picker" && el instanceof HTMLSelectElement) {
      // Options are built in `build`; here we only reflect the active tag.
      if (typeof args.selection === "string") el.value = args.selection;
    } else if (kind === "TabView") {
      // Reflect the runtime-owned selection: swap the visible tab + highlight.
      this.syncTabView(el, args.selection);
    } else if (kind === "Stepper") {
      // Stash the live value/step/bounds for the button handlers; reflect label.
      el.dataset.value = String(typeof args.value === "number" ? args.value : 0);
      el.dataset.step = String(typeof args.step === "number" ? args.step : 1);
      if (typeof args.lowerBound === "number") el.dataset.lowerBound = String(args.lowerBound);
      else delete el.dataset.lowerBound;
      if (typeof args.upperBound === "number") el.dataset.upperBound = String(args.upperBound);
      else delete el.dataset.upperBound;
      const label = el.querySelector(".stepper-label");
      if (label && typeof args.title === "string") {
        label.textContent =
          typeof args.value === "number" ? `${args.title}: ${args.value}` : args.title;
      }
    }
  }
}

/**
 * The DOM node before which a positional `insert`/`move` at logical `index`
 * should land, addressing only patch-managed children and skipping synthetic
 * nodes (a `Section`'s `.section-header`) and an optionally excluded element
 * (the node being moved). Returns `null` to append at the end.
 */
/** Build a CSS grid track template from a `[GridItem]` arg (#205). Each
 * `{ kind, value, max?, spacing? }` becomes one track: `flexible` →
 * `minmax(v, max|1fr)`, `fixed` → `v px`, `adaptive` → an auto-filled
 * `repeat(...)` segment. A finite `max` is honored; its absence means unbounded
 * (`1fr`). Per-`GridItem` `spacing` is not representable as a CSS grid track
 * (grid `gap` is uniform), so it is intentionally not applied on web — a
 * documented host approximation; the stack/grid-level `spacing:` is honored. */
function gridTemplate(tracks: unknown): string | undefined {
  if (!Array.isArray(tracks) || tracks.length === 0) return undefined;
  const segments: string[] = [];
  for (const t of tracks) {
    const item = t as { kind?: string; value?: number; max?: number };
    const v = typeof item.value === "number" ? item.value : 0;
    const upper = typeof item.max === "number" ? `${item.max}px` : "1fr";
    switch (item.kind) {
      case "fixed":
        segments.push(`${v}px`);
        break;
      case "adaptive":
        segments.push(`repeat(auto-fill, minmax(${v || 1}px, ${upper}))`);
        break;
      case "flexible":
      default:
        segments.push(v > 0 || typeof item.max === "number" ? `minmax(${v}px, ${upper})` : "1fr");
        break;
    }
  }
  return segments.join(" ");
}

/** Interpret a `background`/`overlay` modifier value as an arbitrary-view
 * composite (#204): either a bare nested node, or `{ value: <node>, alignment }`.
 * A color/token value (no nested `kind`) returns undefined — not a composite. */
function compositeOf(value: UiirValue): { node: UiirNode; alignment?: string } | undefined {
  if (!value || typeof value !== "object" || Array.isArray(value)) return undefined;
  const v = value as Record<string, unknown>;
  if (typeof v.kind === "string") return { node: value as unknown as UiirNode };
  const inner = v.value as Record<string, unknown> | undefined;
  if (inner && typeof inner.kind === "string") {
    const align = v.alignment as { name?: string } | undefined;
    return { node: inner as unknown as UiirNode, alignment: align?.name };
  }
  return undefined;
}

/** A tab's selection identity (ADR-0013 §2): its `.tag(_)` value if present,
 * else its structural index. Used to build the tab bar and to match the
 * TabView's `selection` arg against the shown child. */
function tabTagOrIndex(child: UiirNode, index: number): string | number {
  const tag = child.modifiers.find((m) => m.name === "tag");
  if (tag && (typeof tag.value === "string" || typeof tag.value === "number")) {
    return tag.value;
  }
  return index;
}

/** CSS `justify-items`/`align-items` keyword for a lazy-grid `alignment:` token
 * on the grid's cross axis (LazyVGrid → horizontal, LazyHGrid → vertical). */
const GRID_ITEM_ALIGN: Record<string, string> = {
  leading: "start",
  trailing: "end",
  top: "start",
  bottom: "end",
  center: "center",
};

/** Cross-axis flex `align-items` keyword for a 1-D stack alignment token. */
const STACK_CROSS_ALIGN: Record<string, string> = {
  leading: "flex-start",
  trailing: "flex-end",
  top: "flex-start",
  bottom: "flex-end",
  center: "center",
  firstTextBaseline: "first baseline",
  lastTextBaseline: "last baseline",
};

/** ZStack `place-items` (`align justify`) for a 2-D alignment token. The
 * baseline-relative 2-D alignments have no CSS grid analogue, so their vertical
 * component approximates to centered (matching the other host's intent). */
const ZSTACK_PLACE: Record<string, string> = {
  center: "center center",
  leading: "center start",
  trailing: "center end",
  top: "start center",
  bottom: "end center",
  topLeading: "start start",
  topTrailing: "start end",
  bottomLeading: "end start",
  bottomTrailing: "end end",
  leadingFirstTextBaseline: "center start",
  centerFirstTextBaseline: "center center",
  trailingFirstTextBaseline: "center end",
};

/** The UIIR token tag a stack kind's `alignment:` carries (so a mis-namespaced
 * token is ignored rather than silently applied — parity with iOS, which
 * matches on the tag). */
const STACK_ALIGN_TAG: Record<string, string> = {
  VStack: "hAlign",
  LazyVStack: "hAlign",
  HStack: "vAlign",
  LazyHStack: "vAlign",
  ZStack: "align",
};

/** Apply a stack's `alignment:` arg on its cross axis (issue #189). VStack
 * (column) and HStack (row) use flex `align-items`; ZStack uses grid
 * `place-items`. The token's tag must match the stack's expected namespace. */
function applyStackAlignment(el: HTMLElement, kind: string, alignment: unknown): void {
  const token = alignment as { $?: string; name?: string } | undefined;
  const name = token?.name;
  if (!name || token?.$ !== STACK_ALIGN_TAG[kind]) return;
  if (kind === "ZStack") {
    const place = ZSTACK_PLACE[name];
    if (place) el.style.placeItems = place;
    return;
  }
  const align = STACK_CROSS_ALIGN[name];
  if (align) el.style.alignItems = align;
}

function patchRef(
  parent: HTMLElement,
  index: number,
  exclude?: Element,
): Node | null {
  const addressable = Array.from(parent.children).filter(
    (c) =>
      c !== exclude &&
      !c.classList.contains("section-header") &&
      // A TabView's synthetic bottom tab bar is not a patch-addressed child.
      !c.classList.contains("tab-bar") &&
      // Composite background/overlay layers are not patch-addressed children.
      !c.classList.contains("composite-layer"),
  );
  return addressable[index] ?? null;
}

/** The intrinsic corner radius for a shape primitive, as a CSS declaration. */
function shapeRadius(node: UiirNode): string {
  switch (node.kind) {
    case "Circle":
    case "Ellipse":
      return "border-radius:50%;";
    case "Capsule":
      return "border-radius:9999px;";
    default:
      return "";
  }
}

/** Build the initial mount patch from a rendered UIIR tree. */
export function mountPatch(tree: UiirNode): Patch[] {
  return [{ op: "mount", node: tree }];
}
