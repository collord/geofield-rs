// Feature 002 (enhanced-authoring) T014/T021 [US1/US2] — author a new surface end to end,
// then edit it. Behavior gate (Constitution I) for quickstart.md V1 + V2.

import { test, expect } from "@playwright/test";

const POINTS_TSV = `50\t50\t650
350\t50\t770
650\t50\t650
350\t350\t710
500\t500\t700`;

test("add a surface, paste points, modal Apply, panel Apply re-fits and renders", async ({ page }) => {
  await page.goto("/");

  // Blank project to start (no autosaved project in a fresh context).
  await expect(page.getByTestId("add-surface")).toBeVisible();

  // Add Surface -> pick Erosional -> the data modal opens for the new surface.
  await page.getByTestId("add-surface").click();
  await expect(page.getByTestId("add-surface-menu")).toBeVisible();
  await page.locator('.add-surface-choice[data-type="erosional"]').click();
  await expect(page.getByTestId("surface-data-modal")).toBeVisible();

  // Modal Apply is disabled with zero points.
  await expect(page.getByTestId("modal-apply")).toBeDisabled();

  // Paste a TSV points block into the Points table -> one row per line.
  const pointsTable = page.getByTestId("table-points");
  await pointsTable.click();
  await page.evaluate(async (tsv) => {
    // Dispatch a synthetic paste with a DataTransfer carrying the TSV.
    const el = document.querySelector('[data-testid="table-points"]')!;
    const dt = new DataTransfer();
    dt.setData("text", tsv);
    el.dispatchEvent(new ClipboardEvent("paste", { clipboardData: dt, bubbles: true, cancelable: true }));
  }, POINTS_TSV);

  await expect(page.getByTestId("rowcount-points")).toContainText("5");

  // Modal Apply now enabled; commit -> modal closes, panel shows the surface + dirty.
  await expect(page.getByTestId("modal-apply")).toBeEnabled();
  await page.getByTestId("modal-apply").click();
  await expect(page.getByTestId("surface-data-modal")).toHaveCount(0);
  await expect(page.getByTestId("apply-bar")).toBeVisible();
  await expect(page.getByTestId("stale-scrim")).toBeVisible();

  // Panel Apply -> re-fit -> status done; the surface appears in the derived summary.
  await page.getByTestId("apply").click();
  await expect(page.getByText("status: done")).toBeVisible({ timeout: 30_000 });
  await expect(page.getByTestId("stale-scrim")).toHaveCount(0);
  await expect(page.locator(".derived-summary").first()).toBeVisible();
});

test("edit a surface round-trips; Cancel discards; one Apply fits several edits (V2)", async ({ page }) => {
  await page.goto("/");

  // Seed a model with existing surfaces.
  await page.getByTestId("seed-template").selectOption("conformal-column");
  await expect(page.getByText("status: done")).toBeVisible({ timeout: 30_000 });

  // Open Edit data… on the first surface — it shows current rows (>0 points).
  await page.getByTestId("surface-menu").first().click();
  await page.getByTestId("surface-edit").first().click();
  await expect(page.getByTestId("surface-data-modal")).toBeVisible();
  const count = await page.getByTestId("rowcount-points").textContent();
  expect(parseInt(count ?? "0", 10)).toBeGreaterThan(0);

  // Cancel leaves the project unchanged (no dirty bar appears).
  await page.getByTestId("modal-cancel").click();
  await expect(page.getByTestId("surface-data-modal")).toHaveCount(0);
  await expect(page.getByTestId("apply-bar")).toHaveCount(0);

  // Edit + Apply (modal) sets dirty without a re-fit yet. Add a row and give it REALISTIC
  // in-plane coordinates (a point on the z≈300 bed) — a (0,0,0) outlier would stress the
  // kriging solve and make the re-fit slow/ill-conditioned, which isn't what we're testing.
  await page.getByTestId("surface-menu").first().click();
  await page.getByTestId("surface-edit").first().click();
  await page.getByTestId("addrow-points").click();
  const newRow = page.getByTestId("table-points").locator("tbody tr").last();
  const cells = newRow.locator('input[type="number"]');
  await cells.nth(0).fill("500");
  await cells.nth(1).fill("500");
  await cells.nth(2).fill("300");
  await page.getByTestId("modal-apply").click();
  await expect(page.getByTestId("apply-bar")).toBeVisible();
  await expect(page.getByTestId("stale-scrim")).toBeVisible();

  // Panel Apply once -> a single re-fit. The scrim CLEARING is the signal that the re-fit
  // for THIS edit completed (waiting on "status: done" alone races, since done is already
  // showing from the seed fit). The dirty=false transition only happens on the new fit's done.
  await page.getByTestId("apply").click();
  await expect(page.getByTestId("stale-scrim")).toHaveCount(0, { timeout: 30_000 });
  await expect(page.getByTestId("apply-bar")).toHaveCount(0);
});
