import { defineConfig } from "vitest/config";

/**
 * T342 — the end-to-end harness, run apart from everything else.
 *
 * A separate configuration because nothing it needs is shared: no jsdom (there is a real
 * window), no React plugin (the frontend is already built into the binary), no setup file.
 * Above all, no default include — running these with the fast checks would put a
 * minute-long, three-process test into every `npm test`.
 *
 * One file at a time, deliberately: the harness starts the real application, and two of them
 * would race for the same driver port and the same window.
 */
export default defineConfig({
  test: {
    include: ["tests/e2e/**/*.e2e.ts"],
    fileParallelism: false,
    // A build, a driver and a window: the default five seconds is not the unit here.
    testTimeout: 60_000,
    hookTimeout: 120_000,
  },
});
