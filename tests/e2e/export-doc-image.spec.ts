// Opt-in exporter for docs/images/hook-rules.png (Task 21).
//
// Run:  CC_REMINDER_EXPORT_DOCS=1 pnpm test:e2e --export-doc-image
// Captures the deterministic, reviewed Hook Rules page exactly as rendered at
// 1280×800 against the browser test backend — never a mockup, and the page
// contains no credential fields.
import { expect, test } from "@playwright/test";
import fs from "node:fs";
import path from "node:path";

test("export the reviewed Hook Rules documentation image", async ({ page }) => {
  test.skip(process.env.CC_REMINDER_EXPORT_DOCS !== "1", "opt-in via CC_REMINDER_EXPORT_DOCS=1");

  await page.setViewportSize({ width: 1280, height: 800 });
  await page.goto("/");
  await page.getByRole("button", { name: "通知规则" }).click();
  await page.getByRole("row", { name: /Stop/ }).waitFor();
  // Settled state only: no drawers, no dialogs, no transient status text.
  await expect(page.locator(".drawer")).toHaveCount(0);

  const out = path.join(process.cwd(), "docs", "images", "hook-rules.png");
  fs.mkdirSync(path.dirname(out), { recursive: true });
  await page.locator("main").screenshot({
    path: out,
    animations: "disabled",
    caret: "hide",
    scale: "css",
  });
  expect(fs.existsSync(out)).toBe(true);
});
