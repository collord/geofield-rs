// Verifies the cross-cut seam-closer A/B toggle ("clip seam"): toggling it re-meshes through the
// mesh_clip corefine codepath (`SeamMode::Clip`) instead of the legacy snap, and produces a
// non-trivial, DIFFERENT contact tessellation. This is the in-harness gate for OK'ing the clip
// wiring — the human reviews the visual result; this test pins that the toggle actually drives the
// new codepath end-to-end (UI → worker → wasm binding → Stack::mesh_cross_cut_shared).

import { test, expect, type Page } from "@playwright/test";

// Total triangle count across the CUT-CONTACT surfaces (series != 0; series 0 is the vein body).
// Read out of the page as a plain number so nothing typed-array crosses the boundary.
function contactTriCount(page: Page): Promise<number> {
  return page.evaluate(() => {
    const s = (window as unknown as { __surfaces?: { series: number; indices: Uint32Array }[] })
      .__surfaces ?? [];
    return s
      .filter((x) => x.series !== 0)
      .reduce((n, x) => n + x.indices.length / 3, 0);
  });
}

// Count cut-contact vertices with a DEGENERATE (all-zero) normal. The corefiner drops normals; if the
// clip path doesn't refill them the contacts render unlit/black and read as "vanished". This guards
// that regression: it must be 0.
function contactZeroNormalVerts(page: Page): Promise<number> {
  return page.evaluate(() => {
    const s = (window as unknown as { __surfaces?: { series: number; normals: Float32Array }[] })
      .__surfaces ?? [];
    let zero = 0;
    for (const x of s.filter((y) => y.series !== 0)) {
      const n = x.normals;
      for (let i = 0; i < n.length; i += 3) {
        if (n[i] === 0 && n[i + 1] === 0 && n[i + 2] === 0) zero++;
      }
    }
    return zero;
  });
}

test("the clip-seam toggle re-meshes the cross-cut contacts through the clip codepath", async ({ page }) => {
  await page.goto("/");
  // A dipping vein cross-cutting a column — the two-wall band case (step 2 of the clip wiring).
  await page.getByTestId("seed-template").selectOption("cross-cutting-vein-dipping");
  await expect(page.getByText("status: done")).toBeVisible({ timeout: 30_000 });

  // Clip is now the DEFAULT codepath: the toggle starts CHECKED and the initial mesh is the clip mesh.
  const toggle = page.getByTestId("seam-clip-toggle");
  await expect(toggle).toBeChecked();
  const clipContactTris = await contactTriCount(page);
  expect(clipContactTris, "the clip default has cut-contact geometry").toBeGreaterThan(0);
  // The clip geometry has VALID normals (the corefiner drops normals; the clip path must refill them,
  // else the contacts render unlit/black and read as "vanished"). This guards that regression: 0.
  expect(
    await contactZeroNormalVerts(page),
    "clipped cut-contacts must have non-degenerate normals (else they render invisible)",
  ).toBe(0);

  // Toggle OFF → legacy snap. It re-meshes (the onChange re-runs the stack). Wait for the status to
  // LEAVE done (re-run started) before waiting for it to return — reading too early sees the stale mesh.
  await toggle.uncheck();
  await page
    .waitForFunction(() => (window as unknown as { __statusLog?: string[] }).__statusLog?.slice(-1)[0] !== "done", { timeout: 5_000 })
    .catch(() => {});
  await expect(page.getByText("status: done")).toBeVisible({ timeout: 60_000 });
  await expect(toggle).not.toBeChecked();
  const snapContactTris = await contactTriCount(page);

  // Snap produced non-trivial geometry, and it is a DIFFERENT tessellation than clip (the corefine
  // re-triangulates the seam row). Equal counts would mean the toggle didn't take effect.
  expect(snapContactTris, "the snap path produced cut-contact geometry").toBeGreaterThan(0);
  expect(
    snapContactTris,
    "snap is a different tessellation than clip (the toggle took effect)",
  ).not.toBe(clipContactTris);

  // Toggling back on restores the clip mesh (round-trip), proving it's a live A/B, not one-way.
  await toggle.check();
  await page
    .waitForFunction(() => (window as unknown as { __statusLog?: string[] }).__statusLog?.slice(-1)[0] !== "done", { timeout: 5_000 })
    .catch(() => {});
  await expect(page.getByText("status: done")).toBeVisible({ timeout: 60_000 });
  const backContactTris = await contactTriCount(page);
  expect(backContactTris, "toggling clip back on returns to the clip mesh").toBe(clipContactTris);
});
