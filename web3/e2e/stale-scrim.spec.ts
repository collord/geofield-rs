// Feature 002 (enhanced-authoring) T025 [US3] — the stale-view scrim. Behavior gate
// (Constitution I) for quickstart.md V3. Assert on the overlay ELEMENT, not pixels.

import { test, expect } from "@playwright/test";

test("scrim appears on a geometry edit, clears on Apply", async ({ page }) => {
  await page.goto("/");
  await page.getByTestId("seed-template").selectOption("conformal-column");
  await expect(page.getByText("status: done")).toBeVisible({ timeout: 30_000 });

  // No scrim with a clean (just-applied) view.
  await expect(page.getByTestId("stale-scrim")).toHaveCount(0);

  // A geometry-affecting edit (add a surface) sets dirty -> scrim + apply bar. Give the new
  // surface a few distinct, in-domain points so the re-fit on Apply stays well-conditioned.
  await page.getByTestId("add-surface").click();
  await page.locator('.add-surface-choice[data-type="erosional"]').click();
  for (const [x, y, z] of [
    [100, 100, 820],
    [700, 100, 820],
    [400, 700, 840],
  ]) {
    await page.getByTestId("addrow-points").click();
    const row = page.getByTestId("table-points").locator("tbody tr").last();
    const cells = row.locator('input[type="number"]');
    await cells.nth(0).fill(String(x));
    await cells.nth(1).fill(String(y));
    await cells.nth(2).fill(String(z));
  }
  await page.getByTestId("modal-apply").click();
  await expect(page.getByTestId("stale-scrim")).toBeVisible();
  await expect(page.getByTestId("apply-bar")).toBeVisible();

  // Apply clears the scrim (the dirty=false transition fires on the new fit's done; assert
  // on the scrim clearing rather than "status: done", which is already showing from the seed).
  await page.getByTestId("apply").click();
  await expect(page.getByTestId("stale-scrim")).toHaveCount(0, { timeout: 30_000 });
});

test("a cheap seam-relation edit re-fits immediately with NO scrim", async ({ page }) => {
  await page.goto("/");
  await page.getByTestId("seed-template").selectOption("erosional-unconformity");
  await expect(page.getByText("status: done")).toBeVisible({ timeout: 30_000 });
  await expect(page.getByTestId("stale-scrim")).toHaveCount(0);

  // Toggle a seam relation — a cheap declared-state edit: immediate re-fit, never dirty.
  const seam = page.locator(".seam-control").first();
  await expect(seam).toBeVisible();
  await seam.getByRole("button", { name: "onlap" }).click();
  await expect(page.getByText("status: done")).toBeVisible({ timeout: 30_000 });
  // No scrim and no apply bar — the view is never stale for cheap edits.
  await expect(page.getByTestId("stale-scrim")).toHaveCount(0);
  await expect(page.getByTestId("apply-bar")).toHaveCount(0);
});
