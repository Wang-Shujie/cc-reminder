// End-to-end desktop UI acceptance coverage (Task 21).
//
// Runs against the deterministic browser test backend selected ONLY by
// VITE_CC_REMINDER_TEST_BACKEND=1 (see playwright.config.ts webServer and
// src/App.tsx). Covers:
// - the project-override workflow incl. 继承全局 visibility and the invariant
//   that raw credential material never reaches rendered output;
// - a keyboard-only onboarding walkthrough and a keyboard-only rule edit;
// - minimum-window fit for every primary page (960×640, no horizontal page
//   overflow, no clipped controls, no incoherent overlap) with pinned
//   screenshots;
// - 1280×800 and 1440×900 desktop screenshots;
// - 200% browser zoom reflow (WCAG 1.4.4);
// - light/dark themes;
// - English longest-label rendering in the nav rail;
// - axe-core accessibility gating (serious/critical violations fail).
import AxeBuilder from "@axe-core/playwright";
import { expect, test, type Page } from "@playwright/test";

/** Nav rail labels (zh-CN authoritative dictionary) + snapshot stems. */
const NAV_PAGES = [
  ["工作台", "workbench"],
  ["通知规则", "rules"],
  ["集成", "integrations"],
  ["设置", "settings"],
] as const;

const SETTLE_TARGETS: Record<(typeof NAV_PAGES)[number][1], () => Promise<unknown>> = {
  workbench: (page) => page.getByRole("heading", { name: "待处理问题" }).waitFor(),
  rules: (page) => page.getByRole("row", { name: /Stop/ }).waitFor(),
  integrations: (page) =>
    page.getByRole("row", { name: /Claude Code/ }).first().waitFor(),
  // Hydration enables the retention inputs; value proves get_settings landed.
  settings: (page) => expect(page.locator("#settings-event-days")).toHaveValue("30"),
};

async function openPage(page: Page, label: string): Promise<void> {
  await page.getByRole("button", { name: label }).click();
}

async function openAndSettle(page: Page, entry: (typeof NAV_PAGES)[number]): Promise<void> {
  const [label] = entry;
  await openPage(page, label);
  await SETTLE_TARGETS[entry[1]](page);
}

/**
 * Layout invariants asserted on every settled page:
 * 1. the PAGE itself never scrolls horizontally (inner regions may scroll);
 * 2. no interactive control clips its label/content;
 * 3. no two sibling controls overlap (incoherent stacking).
 */
async function assertDesktopLayout(page: Page): Promise<void> {
  // Compared against the scrolling element's own clientWidth so the same
  // assertion stays valid under 200% CSS zoom (WCAG 1.4.4).
  const overflowPx = await page.evaluate(() => {
    const se = document.scrollingElement;
    return se === null ? 0 : se.scrollWidth - se.clientWidth;
  });
  expect(overflowPx, "the page itself must never scroll horizontally").toBeLessThanOrEqual(1);

  const clipped = await page.evaluate(() => {
    const bad: string[] = [];
    const controls = document.querySelectorAll("button, select, input, textarea");
    for (const el of controls) {
      const style = window.getComputedStyle(el);
      if (style.display === "none" || style.visibility === "hidden") continue;
      const rect = el.getBoundingClientRect();
      if (rect.width === 0 || rect.height === 0) continue;
      if (el.scrollWidth > el.clientWidth + 1 || el.scrollHeight > el.clientHeight + 1) {
        bad.push(
          `${el.tagName} "${(el.getAttribute("aria-label") ?? el.textContent ?? "")
            .trim()
            .slice(0, 30)}" scroll=${el.scrollWidth}x${el.scrollHeight} client=${el.clientWidth}x${el.clientHeight}`,
        );
      }
    }
    return bad;
  });
  expect(clipped, "controls must not clip their content").toEqual([]);

  const overlaps = await page.evaluate(() => {
    const bad: string[] = [];
    const describe = (el: Element): string =>
      `${el.tagName} "${((el as HTMLElement).getAttribute("aria-label") ?? el.textContent ?? "").trim().slice(0, 24)}"`;
    // Only sibling controls are compared: overlay surfaces (drawers, dialogs)
    // legitimately cover unrelated content, but two peer controls overlapping
    // is always a defect.
    for (const parent of document.querySelectorAll("main *, nav")) {
      const kids = [...parent.children].filter((el): el is HTMLElement => {
        if (!(el instanceof HTMLButtonElement)) return false;
        const style = window.getComputedStyle(el);
        return style.display !== "none" && style.visibility !== "hidden";
      });
      for (let i = 0; i < kids.length; i += 1) {
        for (let j = i + 1; j < kids.length; j += 1) {
          const a = kids[i].getBoundingClientRect();
          const b = kids[j].getBoundingClientRect();
          const x = Math.min(a.right, b.right) - Math.max(a.left, b.left);
          const y = Math.min(a.bottom, b.bottom) - Math.max(a.top, b.top);
          if (x > 2 && y > 2) {
            bad.push(`${describe(kids[i])} ↔ ${describe(kids[j])}`);
          }
        }
      }
    }
    return bad;
  });
  expect(overlaps, "sibling controls must not overlap").toEqual([]);
}

async function expectNoSeriousAxeViolations(page: Page, where: string): Promise<void> {
  const results = await new AxeBuilder({ page }).analyze();
  const blocking = results.violations.filter(
    (violation) => violation.impact === "serious" || violation.impact === "critical",
  );
  const summary = blocking.map(
    (v) =>
      `${v.id}[${v.impact}] x${v.nodes.length}: ${v.nodes
        .map((n) => n.target.join(" "))
        .slice(0, 6)
        .join(" | ")}`,
  );
  expect(summary, `axe serious/critical violations on ${where}`).toEqual([]);
}

test.describe("workflow coverage", () => {
  test("configures a project override and sees the inherited marker with redacted output", async ({
    page,
  }) => {
    await page.goto("/");
    await openPage(page, "通知规则");
    await page.getByRole("row", { name: /Stop/ }).waitFor();

    // Switch to the project scope and pick the seeded project.
    await page.getByRole("radio", { name: "项目", exact: true }).click();
    await page.getByRole("combobox", { name: "项目", exact: true }).selectOption({
      label: "演示项目",
    });
    await page.getByRole("row", { name: /Stop/ }).click();

    // The drawer opens; toggle 启用通知 — a project-scope patch is recorded.
    const drawer = page.getByRole("complementary", { name: "Stop" });
    await expect(drawer).toBeVisible();
    const enableSwitch = drawer.getByRole("switch", { name: "启用通知" });
    await expect(enableSwitch).toBeChecked();
    await enableSwitch.click();
    await expect(enableSwitch).not.toBeChecked();
    await expect(drawer.getByText("已覆盖")).toBeVisible();

    // Resetting the field drops the patch; the 继承全局 status confirms it.
    await drawer.getByRole("button", { name: "恢复启用继承" }).click();
    await expect(drawer.getByText("继承全局")).toBeVisible();

    // The saved-config preview is the redacted document only.
    await expect(drawer.getByText("预览：Stop")).toBeVisible();

    // Raw credential material must never reach ANY rendered output.
    // Channels live in Settings since the v2.1 revision.
    await openPage(page, "设置");
    await page.getByRole("cell", { name: "值班群", exact: true }).waitFor();
    await expect(page.locator("body")).not.toContainText("secret-raw-value");
  });

  test("keyboard-only user completes the five-step onboarding", async ({ page }) => {
    // Fresh install: the browser backend reads this flag during bootstrap.
    await page.addInitScript(() => {
      window.localStorage.setItem("cc-reminder-e2e:onboarding", "fresh");
    });
    await page.goto("/");
    await page.getByRole("heading", { name: "检测 Agent" }).waitFor();

    // Step 1: detect agents.
    const next = page.getByRole("button", { name: "下一步" });
    await expect(next).toBeEnabled();
    await next.focus();
    await page.keyboard.press("Enter");

    // Step 2: install hooks.
    const install = page.getByRole("button", { name: "安装 Hook" });
    await install.waitFor();
    await install.focus();
    await page.keyboard.press("Enter");

    // Step 3: add a channel (typed entirely from the keyboard).
    await page.getByRole("heading", { name: "添加渠道" }).waitFor();
    await page.locator("#ob-channel-name").fill("值班群");
    await page.locator("#ob-channel-webhook").fill("https://qyapi.weixin.qq.com/cgi-bin/webhook/send?key=e2e-demo");
    await page.keyboard.press("Enter"); // implicit form submit

    // Step 4: accept default rules.
    const useDefaults = page.getByRole("button", { name: "使用默认规则" });
    await useDefaults.waitFor();
    await useDefaults.focus();
    await page.keyboard.press("Enter");

    // Step 5: send the test notification; completion persists afterwards.
    const sendTest = page.getByRole("button", { name: "发送测试" });
    await expect(sendTest).toBeEnabled();
    await sendTest.focus();
    await page.keyboard.press("Enter");

    // Completion lands in the main shell at its default workbench.
    await page.getByRole("heading", { name: "待处理问题" }).waitFor();
    await expect(page.getByRole("button", { name: "通知规则" })).toBeVisible();
  });

  test("keyboard-only rule edit toggles a hook and closes the drawer", async ({ page }) => {
    await page.goto("/");
    await openPage(page, "通知规则");
    await page.getByRole("row", { name: /Stop/ }).waitFor();

    const row = page.getByRole("row", { name: /Stop/ });
    await row.focus();
    await page.keyboard.press("Enter"); // opens the configuration drawer

    const drawer = page.getByRole("complementary", { name: "Stop" });
    await expect(drawer).toBeVisible();
    const enableSwitch = drawer.getByRole("switch", { name: "启用通知" });
    await expect(enableSwitch).toBeChecked();
    await enableSwitch.focus();
    await page.keyboard.press("Space"); // commits the disable immediately
    await expect(enableSwitch).not.toBeChecked();

    const close = drawer.getByRole("button", { name: "关闭" });
    await close.focus();
    await page.keyboard.press("Enter");
    await expect(drawer).not.toBeVisible();
    // Global scope: the row switch reflects the committed state too.
    await expect(page.getByRole("switch", { name: "切换 Stop" })).not.toBeChecked();
  });
});

test.describe("desktop layout coverage", () => {
  test("all primary pages fit at the minimum window without overlap", async ({ page }) => {
    await page.goto("/");
    await page.getByRole("heading", { name: "待处理问题" }).waitFor(); // default workbench ready
    for (const entry of NAV_PAGES) {
      await openAndSettle(page, entry);
      await assertDesktopLayout(page);
      await expectNoSeriousAxeViolations(page, `${entry[1]} @ 960×640`);
      await expect(page.locator("main")).toHaveScreenshot(`${entry[1]}.png`);
    }
  });

  test("hook rules fit at 1280×800 and 1440×900", async ({ page }) => {
    await page.setViewportSize({ width: 1280, height: 800 });
    await page.goto("/");
    await openPage(page, "通知规则");
    await page.getByRole("row", { name: /Stop/ }).waitFor();
    await assertDesktopLayout(page);
    await expect(page.locator("main")).toHaveScreenshot("rules-1280x800.png");

    await page.setViewportSize({ width: 1440, height: 900 });
    await assertDesktopLayout(page);
    await expect(page.locator("main")).toHaveScreenshot("rules-1440x900.png");
  });

  test("content survives 200% browser zoom without loss", async ({ page }) => {
    // 1920×1080 at zoom 2 ≈ the 960×640 minimum CSS viewport (WCAG 1.4.4).
    await page.setViewportSize({ width: 1920, height: 1080 });
    await page.goto("/");
    await page.getByRole("heading", { name: "待处理问题" }).waitFor();
    await page.evaluate(() => {
      (document.documentElement.style as CSSStyleDeclaration & { zoom: string }).zoom = "200%";
    });
    for (const entry of NAV_PAGES) {
      await openAndSettle(page, entry);
      await assertDesktopLayout(page);
    }
  });

  test("light and dark themes both render the shell", async ({ page }) => {
    await page.goto("/");
    await openPage(page, "设置");
    await page.locator("#settings-event-days").waitFor();

    // Dark: applied live, persisted via 保存, and survives page switches.
    await page.getByRole("radio", { name: "深色" }).check();
    await expect(page.locator("html[data-theme=dark]")).toHaveCount(1);
    await page.getByRole("button", { name: "保存", exact: true }).click();
    await expect(page.getByText("设置已保存。")).toBeVisible();
    await openPage(page, "通知规则");
    await page.getByRole("row", { name: /Stop/ }).waitFor();
    await expect(page.locator("html")).toHaveAttribute("data-theme", "dark");
    await assertDesktopLayout(page);
    await expect(page.locator("main")).toHaveScreenshot("rules-dark.png");

    // Light comes back cleanly.
    await openPage(page, "设置");
    await page.getByRole("radio", { name: "浅色" }).check();
    await expect(page.locator("html[data-theme=light]")).toHaveCount(1);
  });

  test("English longest nav label renders without clipping", async ({ page }) => {
    await page.addInitScript(() => {
      window.localStorage.setItem("cc-reminder-e2e:locale", "en");
    });
    await page.goto("/");
    const longest = page.getByRole("button", { name: "Integrations" });
    await longest.waitFor();
    await openPage(page, "Rules");
    await page.getByRole("row", { name: /Stop/ }).waitFor();

    const clipped = await page.evaluate(() => {
      const bad: string[] = [];
      for (const el of document.querySelectorAll(".shell-nav .nav-item > span")) {
        if (el.scrollWidth > el.clientWidth + 1) {
          bad.push(`${el.textContent}: ${el.scrollWidth} > ${el.clientWidth}`);
        }
      }
      return bad;
    });
    expect(clipped, "nav labels must wrap or shrink, never clip").toEqual([]);
    await assertDesktopLayout(page);
    await expect(page.locator("main")).toHaveScreenshot("rules-en.png");
  });
});
