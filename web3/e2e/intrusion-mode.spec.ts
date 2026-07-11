// Feature (imported mesh surfaces) — intrusion Solid vs Shell mode. An intrusion is EITHER a
// solid body bounded by one envelope (engine `Single`) OR a hollow shell between an inner and
// outer envelope (engine `Shell`). This gates the authoring UI: a new intrusion defaults to
// solid (one envelope row); switching to shell reveals the inner (core) row; switching back
// hides it without losing data. (Compile-side: solid emits equal envelope indices — the worker's
// documented single-envelope encoding — so no engine change is needed.)

import { test, expect } from "@playwright/test";

test("intrusion defaults to solid (one envelope) and toggles to shell (two)", async ({ page }) => {
  await page.goto("/");
  await expect(page.getByTestId("add-surface")).toBeVisible();

  // Add Surface -> Intrusion. On add, the data modal opens for the sole envelope; dismiss it —
  // we're testing the panel's structure, not authoring points here.
  await page.getByTestId("add-surface").click();
  await expect(page.getByTestId("add-surface-menu")).toBeVisible();
  await page.locator('.add-surface-choice[data-type="intrusion"]').click();
  await expect(page.getByTestId("surface-data-modal")).toBeVisible();
  await page.getByTestId("modal-cancel").click();
  await expect(page.getByTestId("surface-data-modal")).toHaveCount(0);

  const intrusion = page.locator('.container-node[data-kind="intrusion"]');
  const modeSelect = intrusion.getByTestId("intrusion-mode");
  const envelopeRows = intrusion.locator("li.surface-node");

  // Defaults to solid: one envelope contact row.
  await expect(modeSelect).toHaveValue("solid");
  await expect(envelopeRows).toHaveCount(1);

  // Switch to shell: the inner (core) envelope row appears -> two contact rows.
  await modeSelect.selectOption("shell");
  await expect(modeSelect).toHaveValue("shell");
  await expect(envelopeRows).toHaveCount(2);

  // Switch back to solid: inner row hides again.
  await modeSelect.selectOption("solid");
  await expect(modeSelect).toHaveValue("solid");
  await expect(envelopeRows).toHaveCount(1);
});
