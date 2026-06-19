// Feature 002 (enhanced-authoring) — per-surface "Flip orientation". Behavior gate
// (Constitution I). Flipping negates the field's "up" sense (fixes an inverted surface, e.g.
// an erosional deposit). It is a geometry edit: sets dirty, re-fits on Apply, and the menu
// reflects the toggled state. (Assert on state/affordances, not pixels.)

import { test, expect } from "@playwright/test";

test("flipping a surface's orientation is a geometry edit that re-fits on Apply", async ({ page }) => {
  await page.goto("/");
  await page.getByTestId("seed-template").selectOption("erosional-unconformity");
  await expect(page.getByText("status: done")).toBeVisible({ timeout: 30_000 });
  await expect(page.getByTestId("stale-scrim")).toHaveCount(0);

  // The youngest surface (the erosion contact) is seeded flipped — its menu shows the ✓.
  await page.getByTestId("surface-menu").first().click();
  await expect(page.getByTestId("surface-flip").first()).toContainText("✓");

  // Flip it -> dirty + scrim (a geometry-affecting edit).
  await page.getByTestId("surface-flip").first().click();
  await expect(page.getByTestId("apply-bar")).toBeVisible();
  await expect(page.getByTestId("stale-scrim")).toBeVisible();

  // The toggle state is reflected (no longer flipped -> no ✓).
  await page.getByTestId("surface-menu").first().click();
  await expect(page.getByTestId("surface-flip").first()).not.toContainText("✓");
  await page.getByTestId("surface-menu").first().click(); // close the menu

  // Apply re-fits and clears the scrim.
  await page.getByTestId("apply").click();
  await expect(page.getByTestId("stale-scrim")).toHaveCount(0, { timeout: 30_000 });
});
