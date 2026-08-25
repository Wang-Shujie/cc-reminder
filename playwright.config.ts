// Playwright browser-UI acceptance config (Task 21).
//
// The webServer always serves the deterministic browser test backend: the
// VITE_CC_REMINDER_TEST_BACKEND=1 env var makes the app boot against an
// in-memory Backend fake instead of the Tauri bridge (see src/main.tsx and
// src/test/browser-backend.tsx). Production builds never define this var, so
// the branch is dead-code eliminated from dist.
import { defineConfig } from "@playwright/test";

export default defineConfig({
  testDir: "tests/e2e",
  fullyParallel: true,
  forbidOnly: !!process.env.CI,
  retries: process.env.CI ? 2 : 0,
  reporter: process.env.CI ? [["list"], ["html", { open: "never" }]] : [["list"]],
  use: {
    baseURL: "http://127.0.0.1:1420",
    // The Tauri window minimum; every page must fit here without overlap.
    viewport: { width: 960, height: 640 },
    locale: "zh-CN",
    timezoneId: "Asia/Shanghai",
    colorScheme: "light",
    trace: "on-first-retry",
  },
  projects: [{ name: "chromium", use: { browserName: "chromium" } }],
  webServer: {
    command: "pnpm exec vite --host 127.0.0.1 --port 1420",
    url: "http://127.0.0.1:1420",
    reuseExistingServer: !process.env.CI,
    env: { VITE_CC_REMINDER_TEST_BACKEND: "1" },
    timeout: 120_000,
  },
  expect: {
    toHaveScreenshot: {
      animations: "disabled",
      scale: "css",
      caret: "hide",
    },
  },
});
