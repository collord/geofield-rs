// T059 [US5] — the mutation→derive round-trip: changing a seam state in the panel goes
// through the WASM core and the panel's derived display reflects it. (FR-023, SC-005.)

import { test, expect } from "@playwright/test";

test("toggling the seam updates the derived display via the core", async ({ page }) => {
  await page.goto("/");
  await page.getByTestId("seed-template").selectOption("erosional-unconformity");

  // Wait for the initial fit+mesh.
  await expect(page.getByText("status: done")).toBeVisible({ timeout: 30_000 });

  // The younger (top) container carries the three-state seam control.
  const seam = page.locator(".seam-control").first();
  await expect(seam).toBeVisible();

  // The derived summary for the younger series initially reads its effective relation.
  const derived = page.locator(".derived-summary").first();
  const before = await derived.textContent();

  // Click "onlap" and "conformable" in turn; each must re-fit (status cycles) and the
  // derived display must change away from the prior value — proving the round-trip, not a
  // UI-local echo.
  await seam.getByRole("button", { name: "onlap" }).click();
  await expect(page.getByText("status: done")).toBeVisible({ timeout: 30_000 });
  await expect(derived).not.toHaveText(before ?? "", { timeout: 30_000 });
  await expect(derived).toContainText("onlap");

  await seam.getByRole("button", { name: "conformable" }).click();
  await expect(page.getByText("status: done")).toBeVisible({ timeout: 30_000 });
  await expect(derived).toContainText("conformable");
});
