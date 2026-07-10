// Feature: draw model bounds. Behavior gate — the "Draw bounds" button enters top-down 2-point
// digitization; two canvas clicks + a Z range set project.extent; the bounds wireframe toggles.
// (Asserts on affordances/state, not pixels.)

import { test, expect } from "@playwright/test";

test("draw model bounds: two clicks + Z range set the extent; wireframe toggles", async ({ page }) => {
  await page.goto("/");
  // Bounds authoring works on a blank project (no meshing needed) — wait for the toolbar, not "done".
  const boundsToggle = page.getByTestId("bounds-toggle");
  await expect(boundsToggle).toBeVisible({ timeout: 30_000 });
  await expect(boundsToggle).not.toBeChecked();

  // Enter draw mode; the viewer canvas is the digitization surface.
  await page.getByTestId("draw-bounds").click();
  const canvas = page.locator("main canvas");
  await expect(canvas).toBeVisible();
  const box = await canvas.boundingBox();
  if (!box) throw new Error("no canvas");

  // Two clicks (opposite corners of the rectangle, well inside the canvas).
  await page.mouse.click(box.x + box.width * 0.3, box.y + box.height * 0.3);
  await page.mouse.click(box.x + box.width * 0.7, box.y + box.height * 0.7);

  // The Z-range form appears; setting it commits the bounds and shows the wireframe.
  const zform = page.getByTestId("z-range-form");
  await expect(zform).toBeVisible();
  await page.getByTestId("min-z").fill("100");
  await page.getByTestId("max-z").fill("900");
  await page.getByTestId("confirm-bounds").click();

  await expect(zform).toHaveCount(0); // form dismissed
  await expect(boundsToggle).toBeChecked(); // wireframe auto-shown after drawing

  // The toggle hides/shows the wireframe (display-only; no crash, stays interactive).
  await boundsToggle.uncheck();
  await expect(boundsToggle).not.toBeChecked();
  await boundsToggle.check();
  await expect(boundsToggle).toBeChecked();
});
