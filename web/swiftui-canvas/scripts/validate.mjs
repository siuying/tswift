// validate.mjs — offline protocol check for the web host. Loads the committed
// SwiftUI goldens and asserts they conform to the UIIR + patch protocol that
// `src/apply-patch.ts` consumes. No browser, no toolchain — just `node`.
//
//   npm --prefix web/swiftui-canvas run validate
//
// Exit 0 = the goldens are a valid wire payload for this host.

import { existsSync, readFileSync, readdirSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const here = dirname(fileURLToPath(import.meta.url));
const fixtures = join(here, "../../../tests/swiftui-fixtures");

const PATCH_OPS = new Set([
  "mount",
  "insert",
  "remove",
  "replace",
  "setText",
  "setModifiers",
  "setArgs",
  "move",
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
  const path = join(fixtures, `${name}.patches.json`);
  // A fixture without scripted events (no patch golden) is valid — skip it.
  if (!existsSync(path)) return;
  const streams = JSON.parse(readFileSync(path, "utf8"));
  if (!Array.isArray(streams)) return fail(`${name}.patches: must be an array of streams`);
  streams.forEach((stream, i) => {
    if (!Array.isArray(stream)) return fail(`${name}.patches[${i}]: stream must be an array`);
    for (const patch of stream) {
      if (!PATCH_OPS.has(patch.op)) fail(`${name}.patches[${i}]: unknown op "${patch.op}"`);
    }
  });
}

// Discover every fixture with a committed UIIR golden (mirrors the Rust
// harness) so the check can't drift as fixtures are added.
const names = readdirSync(fixtures)
  .filter((f) => f.endsWith(".uiir.json"))
  .map((f) => f.slice(0, -".uiir.json".length))
  .sort();
if (names.length === 0) fail("no *.uiir.json fixtures found");
for (const name of names) {
  checkUiir(name);
  checkPatches(name);
}

if (failures > 0) {
  console.error(`\n${failures} validation failure(s)`);
  process.exit(1);
}
console.log("✓ swiftui goldens conform to the host protocol");
