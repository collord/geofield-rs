// Smoke gate (updated for feature 002, enhanced-authoring): the page loads, the authoring
// shell mounts (heading + Add Surface + the seed-template selector), the 3D viewer canvas
// mounts, and seeding each template populates the hierarchy panel and fits. (SC-001.)

import { test, expect } from "@playwright/test";

test("page loads with the authoring shell and a viewer canvas", async ({ page }) => {
  await page.goto("/");
  await expect(page.getByRole("heading", { name: "Stack authoring" })).toBeVisible();
  await expect(page.getByTestId("add-surface")).toBeVisible();
  await expect(page.getByTestId("seed-template")).toBeVisible();
  // The three.js viewer mounts a canvas.
  await expect(page.locator("canvas")).toBeVisible();
});

test("each seed template loads container(s) into the panel and fits", async ({ page }) => {
  await page.goto("/");
  const select = page.getByTestId("seed-template");
  // Skip the disabled placeholder; the real templates include blank + the three scenarios.
  const values = (await select.locator("option").evaluateAll((opts) =>
    opts.map((o) => (o as HTMLOptionElement).value).filter((v) => v.length > 0),
  )) as string[];
  expect(values.length).toBeGreaterThanOrEqual(3);

  for (const value of values) {
    if (value === "blank") continue; // blank has nothing to fit
    await select.selectOption(value);
    await expect(page.locator(".hierarchy .container-node").first()).toBeVisible({ timeout: 30_000 });
    await expect(page.getByText("status: done")).toBeVisible({ timeout: 30_000 });
  }
});
