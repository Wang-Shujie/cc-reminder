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
    // under vitest.
    exclude: [...configDefaults.exclude, "tests/e2e/**"],
  },
});
