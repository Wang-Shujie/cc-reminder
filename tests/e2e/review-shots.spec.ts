// Opt-in finish-review capture (wayfinding redesign). NOT part of pnpm verify.
// Run: CC_REMINDER_REVIEW=1 pnpm test:e2e tests/e2e/review-shots.spec.ts
// Captures the four destinations at 960×640 and 1280×800 into
// .impeccable/review/ for the finish reviewer.
import { test } from "@playwright/test";
import fs from "node:fs";
import path from "node:path";

const SHOTS: [string, number, number][] = [
  ["workbench", 960, 640],
  ["rules", 960, 640],
  ["integrations", 960, 640],
  ["settings", 960, 640],
  ["workbench-1280", 1280, 800],
  ["rules-1280", 1280, 800],
];

test("capture finish-review screenshots", async ({ page }) => {
  test.skip(process.env.CC_REMINDER_REVIEW !== "1", "opt-in");

  const outDir = path.join(process.cwd(), ".impeccable", "review");
  fs.mkdirSync(outDir, { recursive: true });

  for (const [name, width, height] of SHOTS) {
    await page.setViewportSize({ width, height });
    await page.goto("/");
    // The previous shot's nav click persists last-page; wait for ANY settled
    // page, then navigate to the shot's destination explicitly.
    await page.getByRole("heading", { level: 1 }).waitFor();
    const label =
      name === "workbench" || name === "workbench-1280"
        ? "工作台"
        : name.startsWith("rules")
          ? "通知规则"
          : name.startsWith("integrations")
            ? "集成"
            : "设置";
    await page.getByRole("button", { name: label }).click();
    await page.evaluate(() => {
      (document.documentElement.style as CSSStyleDeclaration & { zoom: string }).zoom = "1";
    });
    const settle: Record<string, () => Promise<unknown>> = {
      workbench: () => Promise.resolve(),
      "workbench-1280": () => Promise.resolve(),
      rules: () => page.getByRole("row", { name: /Stop/ }).waitFor(),
      "rules-1280": () => page.getByRole("row", { name: /Stop/ }).waitFor(),
      integrations: () => page.getByRole("row", { name: /Claude Code/ }).first().waitFor(),
      settings: () => page.locator("#settings-event-days").waitFor(),
    };
    await settle[name]!();
    // Full viewport: the rail and status banner ARE the redesign under review.
    await page.screenshot({
      path: path.join(outDir, `${name}.png`),
      animations: "disabled",
      caret: "hide",
      scale: "css",
    });
  }
});
