import { defineConfig, devices } from "@playwright/test";

// Functional gate for the web3 testbed (constitution v1.1.0 Principle I). Serves the
// PRODUCTION build via `vite preview` and drives it with chromium. Run: `npm run test:e2e`
// (assumes `npm run wasm && npm run build` have produced dist/ + the wasm artifacts).
export default defineConfig({
  testDir: "./e2e",
  // `*.manual.spec.ts` are heavy diagnostic harnesses (e.g. dump-volumes: dumps ~100 MB of
  // booleaned solids for the native partition test). They are NOT part of the functional gate —
  // excluded by default; run them with `RUN_MANUAL=1 playwright test dump-volumes.manual`.
  testIgnore: process.env.RUN_MANUAL ? [] : "**/*.manual.spec.ts",
  fullyParallel: true,
  forbidOnly: !!process.env.CI,
  retries: 0,
  reporter: "list",
  // Each test drives a real WASM stack fit (the heaviest scenarios re-fit a 3-bed floor).
  // With unbounded workers, concurrent fits starve each other on CPU and slow tests can
  // blow their timeouts. Cap at 2 so the gate is reliable on a developer machine; the suite
  // still finishes in well under a minute. (Per-test timeout left at the 30s default, which
  // is ample once fits aren't competing 3-4 at a time.)
  workers: 2,
  use: {
    baseURL: "http://localhost:4173",
    trace: "on-first-retry",
  },
  projects: [{ name: "chromium", use: { ...devices["Desktop Chrome"] } }],
  webServer: {
    command: "npm run preview",
    url: "http://localhost:4173",
    reuseExistingServer: !process.env.CI,
    timeout: 60_000,
  },
});
