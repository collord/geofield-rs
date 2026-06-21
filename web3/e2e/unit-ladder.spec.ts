// Feature 003 (topography, lithology, solids) T027 [US2] — the unit/contact ladder.
// Behavior gate (Constitution I) for User Story 2: a series with N contacts renders N+1
// unit rows interleaved with N contact rows (youngest at top); a unit row is renameable, a
// contact row is not; the per-series "fills upward (floor)" control is gone. (FR-009..012,
// SC-002.)

import { test, expect } from "@playwright/test";

test("a conformable series renders N+1 unit rows interleaved with N contacts", async ({ page }) => {
  await page.goto("/");
  // The conformal-column seed: one floor series with 3 contacts (bed 1/2/3) ⇒ 4 units.
  await page.getByTestId("seed-template").selectOption("conformal-column");
  await expect(page.getByText("status: done")).toBeVisible({ timeout: 30_000 });

  // The "sediments" container shows its derived units interleaved with its contacts.
  const container = page.locator('.container-node[data-kind="series"]').first();
  const unitRows = container.locator(".unit-node");
  const contactRows = container.locator(".surface-node");

  // 3 contacts ⇒ 4 units (universal fill-upward keeps the top unit).
  await expect(contactRows).toHaveCount(3);
  await expect(unitRows).toHaveCount(4);

  // Youngest at top: the FIRST row in the ladder is a unit, not a contact.
  const firstLadderChild = container.locator(".ladder > li").first();
  await expect(firstLadderChild).toHaveClass(/unit-node/);
});

test("a unit row is renameable; a contact row is not; the fills-upward control is gone", async ({ page }) => {
  await page.goto("/");
  await page.getByTestId("seed-template").selectOption("conformal-column");
  await expect(page.getByText("status: done")).toBeVisible({ timeout: 30_000 });

  // A unit row carries an editable name input — rename it and the value sticks.
  const unitName = page.getByTestId("unit-name").first();
  await unitName.fill("Topsoil");
  await expect(unitName).toHaveValue("Topsoil");

  // A contact (surface) row has NO name input — it offers the point/orientation modal only
  // (via its "…" menu), never an inline rename in the ladder.
  const contactRow = page.locator(".surface-node .surface-row").first();
  await expect(contactRow.locator("input.unit-name")).toHaveCount(0);

  // The per-series "fills upward (floor)" checkbox is ABSENT everywhere (FR-007).
  await expect(page.locator(".unbounded-toggle")).toHaveCount(0);
  await expect(page.getByText("fills upward")).toHaveCount(0);
});

test("removing a contact re-derives fewer units (the user never edits units directly)", async ({ page }) => {
  await page.goto("/");
  await page.getByTestId("seed-template").selectOption("conformal-column");
  await expect(page.getByText("status: done")).toBeVisible({ timeout: 30_000 });

  const container = page.locator('.container-node[data-kind="series"]').first();
  await expect(container.locator(".surface-node")).toHaveCount(3); // 3 contacts
  await expect(container.locator(".unit-node")).toHaveCount(4); // → 4 units

  // Delete one CONTACT via its "…" menu (the units are NOT directly editable — FR-011).
  page.on("dialog", (d) => d.accept()); // the menu confirms "Delete this surface?"
  await container.locator(".surface-node").first().locator('[data-testid="surface-menu"]').click();
  await page.getByRole("menuitem", { name: "Delete" }).click();
  await page.getByTestId("apply").click();
  await expect(page.getByText("status: done")).toBeVisible({ timeout: 30_000 });
  await expect(page.getByTestId("stale-scrim")).toHaveCount(0, { timeout: 30_000 });

  // 2 contacts now ⇒ 3 units — the count re-derived from the geometry, not authored.
  await expect(container.locator(".surface-node")).toHaveCount(2);
  await expect(container.locator(".unit-node")).toHaveCount(3);
});
