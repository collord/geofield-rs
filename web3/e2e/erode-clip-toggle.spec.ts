// Verifies the "clip seam" A/B toggle now ALSO drives the stratigraphic ERODE path: on an
// erosional-unconformity scene (no cross-cut body), toggling it re-meshes the eroded sediments
// through the mesh_clip corefiner (`SeamMode::Clip`, Stack::mesh_one_block_erode_clipped) instead of
// the field-half-space masker — a different tessellation. Before this wiring the toggle was a NO-OP
// on erode scenes (it only affected cross-cut bodies). The human reviews the visual result (the
// orientation-flip gap); this test pins that the toggle actually drives the new codepath end-to-end.

import { test, expect, type Page } from "@playwright/test";

// Total triangle count across the SEDIMENTS surfaces (series 1; series 0 is the basalt erode
// surface). Read as a plain number so no typed-array crosses the page boundary.
function sedimentTriCount(page: Page): Promise<number> {
  return page.evaluate(() => {
    const s = (window as unknown as { __surfaces?: { series: number; indices: Uint32Array }[] })
      .__surfaces ?? [];
    return s
      .filter((x) => x.series === 1)
      .reduce((n, x) => n + x.indices.length / 3, 0);
  });
}

test("the clip-seam toggle re-meshes the eroded contacts through the corefiner", async ({ page }) => {
  await page.goto("/");
  // The erosional-unconformity scene: a dipping erode surface truncating the 3-bed sediments floor.
  await page.getByTestId("seed-template").selectOption("erosional-unconformity");
  await expect(page.getByText("status: done")).toBeVisible({ timeout: 30_000 });

  // Clip is now the DEFAULT codepath: the toggle starts CHECKED, so the initial eroded sediments are
  // the corefiner mesh.
  const toggle = page.getByTestId("seam-clip-toggle");
  await expect(toggle).toBeChecked();
  const clipTris = await sedimentTriCount(page);
  expect(clipTris, "the corefiner erode default produced sediment geometry").toBeGreaterThan(0);

  // Toggle OFF → legacy field-masker erode path. It re-meshes. Wait for status to LEAVE done (re-run
  // started) before waiting for it to return, so we don't read the stale clip mesh.
  await toggle.uncheck();
  await page
    .waitForFunction(() => (window as unknown as { __statusLog?: string[] }).__statusLog?.slice(-1)[0] !== "done", { timeout: 5_000 })
    .catch(() => {});
  await expect(page.getByText("status: done")).toBeVisible({ timeout: 60_000 });
  await expect(toggle).not.toBeChecked();
  const maskerTris = await sedimentTriCount(page);

  // The masker path produced non-trivial geometry, and it is a DIFFERENT tessellation than the
  // corefiner (which re-triangulates the seam row). Equal counts would mean the toggle did NOT take
  // effect on the erode scene — the bug this wiring fixes (the toggle used to be a no-op here).
  expect(maskerTris, "the field-masker erode path produced sediment geometry").toBeGreaterThan(0);
  expect(
    maskerTris,
    "the field masker is a different tessellation than clip (the toggle took effect)",
  ).not.toBe(clipTris);

  // Toggling back on restores the corefiner mesh (round-trip), proving it's a live A/B.
  await toggle.check();
  await page
    .waitForFunction(() => (window as unknown as { __statusLog?: string[] }).__statusLog?.slice(-1)[0] !== "done", { timeout: 5_000 })
    .catch(() => {});
  await expect(page.getByText("status: done")).toBeVisible({ timeout: 60_000 });
  const backTris = await sedimentTriCount(page);
  expect(backTris, "toggling clip back on returns to the corefiner mesh").toBe(clipTris);
});
