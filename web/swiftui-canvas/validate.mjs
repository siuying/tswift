// validate.mjs — offline protocol check for the web host. Loads the committed
// SwiftUI goldens and asserts they conform to the UIIR + patch protocol that
// `apply-patch.ts` consumes. No browser, no toolchain — just `node`.
//
//   node web/swiftui-canvas/validate.mjs
//
// Exit 0 = the goldens are a valid wire payload for this host.

import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const here = dirname(fileURLToPath(import.meta.url));
const fixtures = join(here, "../../tests/swiftui-fixtures");

const PATCH_OPS = new Set([
  "mount",
  "insert",
  "remove",
  "replace",
  "setText",
  "setModifiers",
  "setArgs",
]);

let failures = 0;
const fail = (msg) => {
  console.error(`✗ ${msg}`);
  failures++;
};

/** Recursively validate a UIIR node's required shape. */
function checkNode(node, path) {
  for (const key of ["id", "kind", "args", "modifiers", "children"]) {
    if (!(key in node)) fail(`${path}: node missing "${key}"`);
  }
  if (!Array.isArray(node.modifiers)) fail(`${path}: modifiers must be an array`);
  if (!Array.isArray(node.children)) fail(`${path}: children must be an array`);
  for (const m of node.modifiers ?? []) {
    if (typeof m.name !== "string" || !("value" in m)) {
      fail(`${path}: bad modifier ${JSON.stringify(m)}`);
    }
  }
  (node.children ?? []).forEach((c, i) => checkNode(c, `${path}.${i}`));
}

function checkUiir(name) {
  const tree = JSON.parse(readFileSync(join(fixtures, `${name}.uiir.json`), "utf8"));
  checkNode(tree, name);
  if (tree.id !== "0") fail(`${name}: root id must be "0"`);
}

function checkPatches(name) {
  const streams = JSON.parse(readFileSync(join(fixtures, `${name}.patches.json`), "utf8"));
  if (!Array.isArray(streams)) return fail(`${name}.patches: must be an array of streams`);
  streams.forEach((stream, i) => {
    if (!Array.isArray(stream)) return fail(`${name}.patches[${i}]: stream must be an array`);
    for (const patch of stream) {
      if (!PATCH_OPS.has(patch.op)) fail(`${name}.patches[${i}]: unknown op "${patch.op}"`);
    }
  });
}

// The committed v1 fixture set. New fixtures get a line here.
for (const name of ["counter"]) {
  checkUiir(name);
  checkPatches(name);
}

if (failures > 0) {
  console.error(`\n${failures} validation failure(s)`);
  process.exit(1);
}
console.log("✓ swiftui goldens conform to the host protocol");
