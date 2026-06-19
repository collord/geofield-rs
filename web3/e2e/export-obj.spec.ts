// Feature 002 (enhanced-authoring) T033 [US6] — OBJ export. Behavior gate (Constitution I)
// for quickstart.md V5: a fit + not-dirty enables export; the OBJ has non-trivial v/f counts
// and per-surface groups; export is unavailable while dirty.

import { test, expect } from "@playwright/test";

test("Export OBJ downloads a file with non-trivial geometry and per-surface groups", async ({ page }) => {
  await page.goto("/");
  await page.getByTestId("seed-template").selectOption("conformal-column");
  await expect(page.getByText("status: done")).toBeVisible({ timeout: 30_000 });

  // With a fit and not dirty, the OBJ export is enabled.
  await expect(page.getByTestId("export-obj")).toBeEnabled();

  const [download] = await Promise.all([
    page.waitForEvent("download"),
    page.getByTestId("export-obj").click(),
  ]);
  expect(download.suggestedFilename()).toMatch(/\.obj$/);

  // Parse the blob: count vertices and faces.
  const stream = await download.createReadStream();
  let text = "";
  for await (const chunk of stream) text += chunk.toString();
  const vCount = (text.match(/^v /gm) ?? []).length;
  const fCount = (text.match(/^f /gm) ?? []).length;
  const gCount = (text.match(/^g /gm) ?? []).length;
  expect(vCount).toBeGreaterThan(0);
  expect(fCount).toBeGreaterThan(0);
  expect(gCount).toBeGreaterThan(0); // per-surface groups
});

test("OBJ export is unavailable while there are unapplied changes", async ({ page }) => {
  await page.goto("/");
  await page.getByTestId("seed-template").selectOption("conformal-column");
  await expect(page.getByText("status: done")).toBeVisible({ timeout: 30_000 });
  await expect(page.getByTestId("export-obj")).toBeEnabled();

  // A geometry edit makes the view stale -> export disabled (you export what's rendered).
  await page.getByTestId("add-surface").click();
  await page.locator('.add-surface-choice[data-type="erosional"]').click();
  await page.getByTestId("addrow-points").click();
  await page.getByTestId("modal-apply").click();
  await expect(page.getByTestId("export-obj")).toBeDisabled();
});
