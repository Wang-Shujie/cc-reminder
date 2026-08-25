// Playwright browser-UI acceptance config (Task 21, CI split added Task 22).
//
// The webServer always serves the deterministic browser test backend: the
// VITE_CC_REMINDER_TEST_BACKEND=1 env var makes the app boot against an
// in-memory Backend fake instead of the Tauri bridge (see src/App.tsx and
// src/test/browser-backend.tsx). Production builds never define this var, so
// the branch is dead-code eliminated from dist — CI greps dist to keep this
// honest.
import { defineConfig } from "@playwright/test";

/**
 * Screenshot-bearing tests compare pixels against baselines committed as
 * `<name>-<projectName>-<platform>.png` (Playwright composes the suffix from
 * the project name and process.platform). The committed baselines are
 * `*-chromium-darwin.png`, so pixel comparison is only meaningful on macOS
 * hosts. CI therefore runs these four tests darwin-only while every other
 * check (workflow coverage, keyboard access, layout invariants, axe-core,
 * 200% zoom) runs on all operating systems. The project stays named
 * "chromium" on every OS so baseline resolution is unchanged wherever the
 * screenshots DO run.
 */
const SCREENSHOT_TEST_PATTERNS = [
  /all primary pages fit at the minimum window without overlap/,
  /hook rules fit at 1280×800 and 1440×900/,
  /light and dark themes both render the shell/,
  /English longest nav label renders without clipping/,
];

const ON_MACOS = process.platform === "darwin";

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
  projects: [
    {
      name: "chromium",
      use: { browserName: "chromium" },
      ...(ON_MACOS ? {} : { grepInvert: SCREENSHOT_TEST_PATTERNS }),
    },
  ],
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
