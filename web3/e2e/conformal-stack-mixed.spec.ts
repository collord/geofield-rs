// The "Conformal stack (mixed imported + parametric)" seed template — a 5-surface conformable stack
// with element types INTERLEAVED: 3 imported ~100k-vert meshes (top bed / bed 3 / bed 1) and 2
// single-interface parametric series (upper bed / bed 2). Pins that the imported↔parametric stacking
// interaction fits and renders a clean 5-volume layer cake in Solids (each surface contributes its
// own below-rock slab). The 3 imported meshes must be present (gitignored per-machine; regenerate
// with `cargo test -p geofield --test gen_erosion_meshes -- --ignored`); skipped if absent.

import { test, expect } from "@playwright/test";
import { existsSync } from "node:fs";
import { fileURLToPath } from "node:url";

const MESH_DIR = fileURLToPath(new URL("../public/meshes/", import.meta.url));
const MESHES = ["erosion-stack-bed1.obj", "erosion-stack-bed3.obj", "erosion-stack-cover.obj"];

test("the mixed imported+parametric conformal stack fits and renders 5 slab volumes", async ({ page }) => {
  test.skip(
    !MESHES.every((m) => existsSync(MESH_DIR + m)),
    "erosion-stack-*.obj fixtures absent — regenerate with `cargo test -p geofield --test gen_erosion_meshes -- --ignored`",
  );
  test.setTimeout(300_000);

  await page.goto("/");
  await page.getByTestId("seed-template").selectOption("conformal-stack-mixed");
  await expect(page.getByText("status: done")).toBeVisible({ timeout: 150_000 });
  await expect(page.getByTestId("stale-scrim")).toHaveCount(0, { timeout: 150_000 });

  // The interleaved element types both appear in the hierarchy.
  await expect(page.getByText("top bed")).toBeVisible();
  await expect(page.getByText("upper bed (parametric)")).toBeVisible();
  await expect(page.getByText("bed 2 (parametric)")).toBeVisible();

  // Surfaces mode: all five surfaces (each its own series) render geometry.
  const seriesWithGeometry = await page.evaluate(() => {
    const s = (window as { __surfaces?: { series: number; indices: Uint32Array }[] }).__surfaces ?? [];
    return new Set(s.filter((x) => x.indices.length > 0).map((x) => x.series)).size;
  });
  expect(seriesWithGeometry, "all five interleaved surfaces render").toBe(5);

  // Solids: each surface contributes its own below-rock (unit 0) slab — a 5-volume layer cake.
  await page.getByTestId("render-toggle").click();
  await page
    .waitForFunction(() => (window as { __statusLog?: string[] }).__statusLog?.slice(-1)[0] !== "done", { timeout: 10_000 })
    .catch(() => {});
  await expect(page.getByText("status: done")).toBeVisible({ timeout: 120_000 });
  await expect(page.getByTestId("stale-scrim")).toHaveCount(0, { timeout: 120_000 });
  await expect
    .poll(
      () =>
        page.evaluate(() => {
          const s = (window as { __surfaces?: { unit?: number }[] }).__surfaces ?? [];
          return s.length > 0 && s.every((x) => x.unit !== undefined && x.unit !== null);
        }),
      { timeout: 120_000 },
    )
    .toBe(true);

  const seriesWithSlab = await page.evaluate(() => {
    const s = (window as { __surfaces?: { series: number; unit?: number; indices: Uint32Array }[] }).__surfaces ?? [];
    return new Set(s.filter((x) => x.indices.length > 0 && x.unit === 0).map((x) => x.series)).size;
  });
  expect(seriesWithSlab, "all five interleaved surfaces yield a below-rock slab in Solids").toBe(5);

  // Partition proxy: the five slabs must TILE the sub-topography column, not stack on top of each
  // other — so their Manifold volumes sum to at most the domain box (1000³), with slack for the
  // known thin edge/contact film. The regression this pins measured 60.8% grid overlap (each slab
  // flooding its full below-column), which roughly doubles the sum — a decisive failure.
  const volumeSum = await page.evaluate(() => {
    const s = (window as { __surfaces?: { unit?: number; volume?: number }[] }).__surfaces ?? [];
    return s.filter((x) => x.unit === 0).reduce((acc, x) => acc + (x.volume ?? 0), 0);
  });
  const DOMAIN = 1000 ** 3;
  expect(volumeSum, "slab volumes must not sum past the domain (overlapping columns)").toBeLessThan(DOMAIN * 1.05);
  expect(volumeSum, "slab volumes must fill a substantial share of the domain (vanished slabs)").toBeGreaterThan(DOMAIN * 0.2);
});
