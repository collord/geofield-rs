import { defineConfig, devices } from "@playwright/test";

// Functional gate for the web3 testbed (constitution v1.1.0 Principle I). Serves the
// PRODUCTION build via `vite preview` and drives it with chromium. Run: `npm run test:e2e`
// (assumes `npm run wasm && npm run build` have produced dist/ + the wasm artifacts).
export default defineConfig({
  testDir: "./e2e",
  fullyParallel: true,
  forbidOnly: !!process.env.CI,
  retries: 0,
  reporter: "list",
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
