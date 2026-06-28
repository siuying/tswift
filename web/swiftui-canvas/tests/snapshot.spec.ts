import { readFileSync, readdirSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

import { expect, test, type Page } from "@playwright/test";

// Layer D web harness. For each UIIR fixture: mount it on <swiftui-canvas>,
// screenshot the initial render, then replay each patch step (mirroring the
// fixture's *.patches.json) and screenshot after each. This is the web half of
// the web↔native perceptual diff; the iOS half lives in ios/UiirRenderer.

const here = dirname(fileURLToPath(import.meta.url));
const fixturesDir = join(here, "..", "..", "..", "tests", "swiftui-fixtures");

/** UIIR node — kept loose; the canvas owns the real types. */
type UiirNode = Record<string, unknown>;
type Patch = Record<string, unknown>;

declare global {
  interface Window {
    harness: {
      mount(tree: UiirNode): void;
      applyPatches(patches: Patch[]): void;
    };
    harnessReady: boolean;
  }
}

function fixtureNames(): string[] {
  return readdirSync(fixturesDir)
    .filter((f) => f.endsWith(".uiir.json"))
    .map((f) => f.slice(0, -".uiir.json".length))
    .sort();
}

function loadUiir(name: string): UiirNode {
  return JSON.parse(readFileSync(join(fixturesDir, `${name}.uiir.json`), "utf8"));
}

function loadPatches(name: string): Patch[][] {
  try {
    return JSON.parse(readFileSync(join(fixturesDir, `${name}.patches.json`), "utf8"));
  } catch {
    return []; // fixtures without a patch stream render initial-only.
  }
}

async function gotoHarness(page: Page): Promise<void> {
  await page.goto("/");
  await page.waitForFunction(() => window.harnessReady === true);
}

/** Screenshot the canvas element, named to align with the iOS baselines. */
async function shot(page: Page, name: string): Promise<void> {
  const canvas = page.locator("#canvas");
  await expect(canvas).toHaveScreenshot(`${name}.png`, {
    animations: "disabled",
    // Perceptual diff is downstream; allow sub-pixel AA noise here.
    maxDiffPixelRatio: 0.01,
  });
}

for (const name of fixtureNames()) {
  test(name, async ({ page }) => {
    await gotoHarness(page);

    const tree = loadUiir(name);
    await page.evaluate((t) => window.harness.mount(t), tree);
    await shot(page, `${name}-0-initial`);

    const steps = loadPatches(name);
    for (let i = 0; i < steps.length; i++) {
      const step = steps[i];
      await page.evaluate((s) => window.harness.applyPatches(s), step);
      await shot(page, `${name}-${i + 1}`);
    }
  });
}
