// Opt-in finish-review capture (native-mac redesign). NOT part of pnpm verify.
// Run: CC_REMINDER_REVIEW=1 pnpm test:e2e tests/e2e/review-shots.spec.ts
// Captures light AND dark passes over all five destinations at 960×640 plus
// the open rules drawer sheet, into .impeccable/review/ for the reviewer.
import { expect, test, type Page } from "@playwright/test";
import fs from "node:fs";
import path from "node:path";

const PAGES: [string, string, (page: Page) => Promise<unknown>][] = [
  ["工作台", "workbench", () => Promise.resolve()],
  ["通知规则", "rules", (page) => page.getByRole("row", { name: /Stop/ }).waitFor()],
  ["项目", "projects", (page) => page.getByRole("cell", { name: "演示项目" }).waitFor()],
  [
    "集成",
    "integrations",
    (page) => page.getByRole("row", { name: /Claude Code/ }).first().waitFor(),
  ],
  ["设置", "settings", (page) => page.locator("#settings-event-days").waitFor()],
];

async function snap(
  page: Page,
  outDir: string,
  name: string,
): Promise<void> {
  await page.screenshot({
    path: path.join(outDir, `${name}.png`),
    animations: "disabled",
    caret: "hide",
    scale: "css",
  });
}

test("capture native-mac redesign screenshots", async ({ page }) => {
  test.skip(process.env.CC_REMINDER_REVIEW !== "1", "opt-in");

  const outDir = path.join(process.cwd(), ".impeccable", "review");
  fs.mkdirSync(outDir, { recursive: true });
  await page.setViewportSize({ width: 960, height: 640 });

  // Light pass (fresh profile defaults to the configured light theme).
  await page.goto("/");
  await page.getByRole("navigation").waitFor();
  for (const [label, name, settle] of PAGES) {
    await page.getByRole("button", { name: label }).click();
    await settle(page);
    await snap(page, outDir, `light-${name}`);
  }

  // Drawer sheet open (the signature motion surface).
  await page.getByRole("button", { name: "通知规则" }).click();
  const stopRow = page.getByRole("row", { name: /Stop/ });
  await stopRow.waitFor();
  await stopRow.click();
  await expect(page.getByRole("complementary", { name: "Stop" })).toBeVisible();
  await snap(page, outDir, "light-drawer");
  await page
    .getByRole("complementary", { name: "Stop" })
    .getByRole("button", { name: "关闭" })
    .click();

  // Dark pass: applied through the real settings flow so persistence runs.
  await page.getByRole("button", { name: "设置" }).click();
  await page.locator("#settings-event-days").waitFor();
  await page.getByRole("radio", { name: "深色" }).check();
  await page.getByRole("button", { name: "保存", exact: true }).click();
  await expect(page.locator("html[data-theme=dark]")).toHaveCount(1);

  for (const [label, name, settle] of PAGES) {
    await page.getByRole("button", { name: label }).click();
    await settle(page);
    await snap(page, outDir, `dark-${name}`);
  }
});
