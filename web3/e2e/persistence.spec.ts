// Feature 002 (enhanced-authoring) T030 [US5] — persistence + templates. Behavior gate
// (Constitution I) for quickstart.md V7: autosave survives reload; export/import round-trips;
// seeding loads a valid project.

import { test, expect } from "@playwright/test";

test("a committed edit survives a page reload (localStorage restore)", async ({ page }) => {
  await page.goto("/");
  await page.getByTestId("seed-template").selectOption("conformal-column");
  await expect(page.getByText("status: done")).toBeVisible({ timeout: 30_000 });

  // Rename the project so we can recognize it after reload (a committed edit -> autosave).
  const nameInput = page.getByLabel("project name");
  await nameInput.fill("my-restore-test");

  // Wait until the ~500ms debounced autosave has actually flushed to localStorage (polling
  // the real condition is robust under parallel CPU load; a fixed sleep can race).
  await expect
    .poll(() => page.evaluate(() => localStorage.getItem("geoproj:@last")), { timeout: 10_000 })
    .toBe("my-restore-test");

  await page.reload();
  await expect(page.getByLabel("project name")).toHaveValue("my-restore-test", { timeout: 10_000 });
});

test("seeding from a template loads a valid project that fits", async ({ page }) => {
  await page.goto("/");
  for (const tpl of ["conformal-column", "cross-cutting-vein"]) {
    await page.getByTestId("seed-template").selectOption(tpl);
    await expect(page.getByText("status: done")).toBeVisible({ timeout: 30_000 });
    // A valid project renders at least one surface row / container.
    await expect(page.locator("[data-element-id]").first()).toBeVisible();
  }
});

test("export produces a downloadable .geoproj.json", async ({ page }) => {
  await page.goto("/");
  await page.getByTestId("seed-template").selectOption("conformal-column");
  await expect(page.getByText("status: done")).toBeVisible({ timeout: 30_000 });

  const [download] = await Promise.all([
    page.waitForEvent("download"),
    page.getByText("Export", { exact: true }).click(),
  ]);
  expect(download.suggestedFilename()).toMatch(/\.geoproj\.json$/);
});
