// T058 [US5] — smoke gate: the page loads, each scenario selects and renders into the
// hierarchy panel, and the 3D viewer canvas mounts. (SC-001, FR-021.)

import { test, expect } from "@playwright/test";

test("page loads with the hierarchy heading and scenario selector", async ({ page }) => {
  await page.goto("/");
  await expect(page.getByRole("heading", { name: "Stratigraphic hierarchy" })).toBeVisible();
  await expect(page.locator("select")).toBeVisible();
  // The three.js viewer mounts a canvas.
  await expect(page.locator("canvas")).toBeVisible();
});

test("each scenario loads container(s) into the panel", async ({ page }) => {
  await page.goto("/");
  const select = page.locator("select");
  const options = await select.locator("option").allTextContents();
  expect(options.length).toBeGreaterThanOrEqual(2); // conformal-column + erosional-unconformity

  for (const label of options) {
    await select.selectOption({ label });
    // Wait for the worker fit+mesh to complete and the panel to populate.
    await expect(page.locator(".hierarchy .container-node").first()).toBeVisible({ timeout: 30_000 });
    await expect(page.getByText("status: done")).toBeVisible({ timeout: 30_000 });
  }
});
