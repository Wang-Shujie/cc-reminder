import react from "@vitejs/plugin-react";
import { configDefaults, defineConfig } from "vitest/config";

export default defineConfig({
  plugins: [react()],
  server: {
    port: 1420,
    strictPort: true,
  },
  test: {
    environment: "jsdom",
    setupFiles: ["./src/test/setup.ts"],
    clearMocks: true,
    globals: true,
    // Playwright specs live in tests/e2e and run via `pnpm test:e2e`, never
    // under vitest. Nested copies under git worktrees (.worktrees/) and the
    // local pnpm store must never be scanned either — they fail to load as
    // vitest files when developing on main while a worktree exists.
    exclude: [
      ...configDefaults.exclude,
      "tests/e2e/**",
      ".worktrees/**",
      ".pnpm-store/**",
    ],
  },
});
