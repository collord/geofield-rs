// Feature (points/normals overlay) — a viewer toggle overlays the authored contact POINTS (spheres)
// and orientation NORMALS (arrows) that define each parametric surface, labeled by row number, with
// three global size sliders. Behavior gate (Constitution I): assert on the built overlay geometry
// (via the __overlayCounts hook) and the toggle/slider affordances — not pixels.

import { test, expect } from "@playwright/test";

type Counts = { spheres: number; arrows: number } | undefined;
const overlayCounts = (page: import("@playwright/test").Page) =>
  page.evaluate(() => (window as unknown as { __overlayCounts?: () => Counts }).__overlayCounts?.());

test("toggle overlays authored points (spheres) + normals (arrows); off clears them", async ({ page }) => {
  await page.goto("/");

  // A conformal column = 3 beds, each with 2 points + 1 orientation → 6 spheres, 3 arrows.
  await page.getByTestId("seed-template").selectOption("conformal-column");
  await expect(page.getByText("status: done")).toBeVisible({ timeout: 30_000 });
  await expect(page.getByTestId("stale-scrim")).toHaveCount(0);

  // Off by default: no overlay geometry, and the size sliders are hidden.
  await expect(page.getByTestId("overlay-size-controls")).toHaveCount(0);
  await expect.poll(() => overlayCounts(page)).toEqual({ spheres: 0, arrows: 0 });

  // Toggle ON: spheres for every point, arrows for every orientation; sliders appear.
  await page.getByTestId("points-normals-toggle").check();
  await expect(page.getByTestId("overlay-size-controls")).toBeVisible();
  await expect.poll(() => overlayCounts(page)).toEqual({ spheres: 6, arrows: 3 });

  // Sizing sliders exist (point / arrow / label). Changing one keeps the geometry (re-sized, not
  // rebuilt-away) — the counts are unchanged.
  await expect(page.getByTestId("overlay-point-size")).toBeVisible();
  await expect(page.getByTestId("overlay-arrow-size")).toBeVisible();
  await expect(page.getByTestId("overlay-label-size")).toBeVisible();
  await page.getByTestId("overlay-point-size").fill("3");
  await expect.poll(() => overlayCounts(page)).toEqual({ spheres: 6, arrows: 3 });

  // Toggle OFF: overlay disposed, sliders hidden.
  await page.getByTestId("points-normals-toggle").uncheck();
  await expect(page.getByTestId("overlay-size-controls")).toHaveCount(0);
  await expect.poll(() => overlayCounts(page)).toEqual({ spheres: 0, arrows: 0 });
});
