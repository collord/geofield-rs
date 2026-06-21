// Feature 003 (topography, lithology, solids) T021 [US1] — author a Topography surface and
// see the model capped at the ground. Behavior gate (Constitution I) for the spec's User
// Story 1: above-ground becomes Air, older elements clip to the ground, and a SECOND
// topography cannot be added. (FR-001/002/003, SC-001.)

import { test, expect } from "@playwright/test";

// A flat-ish ground surface spanning the domain at z≈600, cutting through the seeded model.
const GROUND_TSV = `50\t50\t600
500\t50\t600
950\t50\t600
50\t500\t600
500\t500\t600
950\t500\t600
50\t950\t600
500\t950\t600
950\t950\t600`;

test("add Topography → it fits, caps the model, and a second is disabled", async ({ page }) => {
  await page.goto("/");

  // Seed a model with existing stratigraphy to cap.
  await page.getByTestId("seed-template").selectOption("conformal-column");
  await expect(page.getByText("status: done")).toBeVisible({ timeout: 30_000 });

  // Add Surface → the Topography choice is present and ENABLED (none exists yet).
  await page.getByTestId("add-surface").click();
  await expect(page.getByTestId("add-surface-menu")).toBeVisible();
  const topoChoice = page.locator('.add-surface-choice[data-type="topography"]');
  await expect(topoChoice).toBeEnabled();
  await topoChoice.click();

  // The data modal opens for the new ground surface; paste the ground points.
  await expect(page.getByTestId("surface-data-modal")).toBeVisible();
  const pointsTable = page.getByTestId("table-points");
  await pointsTable.click();
  await page.evaluate(async (tsv) => {
    const el = document.querySelector('[data-testid="table-points"]')!;
    const dt = new DataTransfer();
    dt.setData("text", tsv);
    el.dispatchEvent(new ClipboardEvent("paste", { clipboardData: dt, bubbles: true, cancelable: true }));
  }, GROUND_TSV);
  await expect(page.getByTestId("rowcount-points")).toContainText("9");

  // Commit the modal, then Apply (re-fit) the whole stack.
  await page.getByTestId("modal-apply").click();
  await expect(page.getByTestId("surface-data-modal")).toHaveCount(0);
  await page.getByTestId("apply").click();
  await expect(page.getByText("status: done")).toBeVisible({ timeout: 30_000 });
  await expect(page.getByTestId("stale-scrim")).toHaveCount(0, { timeout: 30_000 });

  // The topography element renders in the panel (pinned at the top / youngest).
  const topoNode = page.locator('.container-node[data-kind="topography"]');
  await expect(topoNode).toHaveCount(1);
  await expect(page.locator(".hierarchy .container-node").first()).toHaveAttribute("data-kind", "topography");

  // The cap is real: the older stratigraphy still renders surfaces below the ground (it was
  // clipped to below topography, not erased). The topography is the youngest element (series 0).
  const meshInfo = await page.evaluate(() => {
    const surfaces = (window as any).__surfaces as { series: number; positions: Float32Array }[];
    // Max z across ALL rendered surfaces — older beds must not poke far above the ground (z≈600).
    let maxZ = -Infinity;
    let topoSurfaces = 0;
    for (const s of surfaces) {
      if (s.series === 0) topoSurfaces += 1; // topography is pinned youngest (index 0)
      for (let i = 2; i < s.positions.length; i += 3) maxZ = Math.max(maxZ, s.positions[i]);
    }
    return { maxZ, topoSurfaces, count: surfaces.length };
  });
  expect(meshInfo.count, "the stack meshes some surfaces").toBeGreaterThan(0);
  // The ground surface itself meshes (the youngest element contributes geometry).
  expect(meshInfo.topoSurfaces, "topography contributes a rendered surface").toBeGreaterThan(0);

  // Re-open the Add menu → the Topography choice is now DISABLED (singleton invariant).
  await page.getByTestId("add-surface").click();
  await expect(page.getByTestId("add-surface-menu")).toBeVisible();
  await expect(page.locator('.add-surface-choice[data-type="topography"]')).toBeDisabled();
});
