// uiir-types.ts — shared UIIR wire contract for the web host.
// Single source of truth for node / patch / modifier shapes consumed by
// apply-patch, modifier-css, and the Charts decoder.

/** A UIIR modifier value: a tagged-union token, a scalar, or an object. */
export type UiirValue =
  | null
  | number
  | string
  | boolean
  | { $: string; name: string }
  | UiirValue[]
  | { [key: string]: UiirValue };

export interface Modifier {
  name: string;
  value: UiirValue;
}

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
