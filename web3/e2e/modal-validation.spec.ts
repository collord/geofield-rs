// Feature 002 (enhanced-authoring) T015 [US1] — author-time validation. Behavior gate
// (Constitution I) for quickstart.md V6: zero points hard-blocks; adding points enables;
// no-orientation is a soft advisory that still applies.

import { test, expect } from "@playwright/test";

test("zero-point surface blocks Apply; adding points enables it", async ({ page }) => {
  await page.goto("/");

  // Add a surface but enter NO points, then cancel the modal — the surface exists with zero
  // points, which is hard-invalid.
  await page.getByTestId("add-surface").click();
  await page.locator('.add-surface-choice[data-type="conformable"]').click();
  await expect(page.getByTestId("modal-apply")).toBeDisabled(); // zero points
  await page.getByTestId("modal-cancel").click();

  // The surface is in the project (dirty) but the panel Apply is BLOCKED with a reason.
  await expect(page.getByTestId("apply-bar")).toBeVisible();
  await expect(page.getByTestId("apply")).toBeDisabled();
  await expect(page.getByTestId("apply-reason")).toContainText("no points");

  // Add points via the modal -> Apply becomes available.
  await page.getByTestId("surface-menu").first().click();
  await page.getByTestId("surface-edit").first().click();
  await page.getByTestId("addrow-points").click();
  await page.getByTestId("modal-apply").click();
  await expect(page.getByTestId("apply")).toBeEnabled();
});

test("a series with no orientation shows the amber advisory but still applies", async ({ page }) => {
  await page.goto("/");

  // Add a conformable surface with points but NO orientation rows.
  await page.getByTestId("add-surface").click();
  await page.locator('.add-surface-choice[data-type="conformable"]').click();
  // Add three DISTINCT, in-domain points (>=3 to avoid the <3-points advisory; distinct so
  // the kriging solve is well-conditioned). This isolates the no-orientation soft advisory.
  const coords = [
    [100, 100, 500],
    [700, 100, 500],
    [400, 700, 520],
  ];
  for (const [x, y, z] of coords) {
    await page.getByTestId("addrow-points").click();
    const row = page.getByTestId("table-points").locator("tbody tr").last();
    const cells = row.locator('input[type="number"]');
    await cells.nth(0).fill(String(x));
    await cells.nth(1).fill(String(y));
    await cells.nth(2).fill(String(z));
  }
  await page.getByTestId("modal-apply").click();

  // Apply is allowed (no hard error); the fit can attempt it.
  await expect(page.getByTestId("apply")).toBeEnabled();
  await page.getByTestId("apply").click();
  await expect(page.getByText("status: done")).toBeVisible({ timeout: 30_000 });
});
