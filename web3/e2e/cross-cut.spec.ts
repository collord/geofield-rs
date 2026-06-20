// T037 [US2] — the cross-cutting-vein scenario: the panel shows the read-only "cross-cuts"
// chip for the vein (effectiveRelation = crossCut, derived by the core), and the chip's
// tooltip records the syn-depositional punt (T038). (FR-016, SC-003.)

import { test, expect } from "@playwright/test";

test("a cross-cutting vein shows the derived cross-cuts chip", async ({ page }) => {
  await page.goto("/");
  await page.getByTestId("seed-template").selectOption("cross-cutting-vein");
  await expect(page.getByText("status: done")).toBeVisible({ timeout: 30_000 });

  // The vein container's seam shows the read-only cross-cuts chip (not the 3-state control).
  const chip = page.locator(".cross-cut-chip").first();
  await expect(chip).toBeVisible();
  await expect(chip).toContainText("cross-cuts");

  // The chip is read-only: there is no seam-state button inside it.
  await expect(chip.locator("button")).toHaveCount(0);

  // T038: the syn-depositional limitation is recorded in the tooltip.
  await expect(chip).toHaveAttribute("title", /syn-depositional/i);

  // The vein's derived display reads crossCut (sourced from CoreUpdate.derived).
  await expect(page.locator(".derived-summary").first()).toContainText("crossCut");

  // The vein's OWN walls now render (the feature-001 punt is lifted): the cross-cutting body
  // (series 0) contributes at least one wall surface, not just the clipped older beds.
  const veinSurfaces = await page.evaluate(() => {
    const surfaces = (window as any).__surfaces as { series: number; indices: Uint32Array }[];
    return surfaces.filter((s) => s.series === 0 && s.indices.length > 0).length;
  });
  expect(veinSurfaces, "the vein contributes rendered wall surfaces").toBeGreaterThan(0);
});
