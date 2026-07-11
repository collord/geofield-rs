// Regression for the reported bug: "the volume view isn't working at all with the imported mesh with
// bounds within (laterally) that imported mesh." A curved imported contact whose footprint laterally
// ENCLOSES the domain (the lidar case) rendered NOTHING in Solids mode: the below-contact primitive was
// contoured from a mesh-derived field, and nearest-triangle signing on a curved TIN left ~1750 open
// edges → Manifold.ofMesh rejected it → the unit dropped. The fix seals the mesh's OWN triangles into a
// watertight prism (no field, no contour), which Manifold accepts, so the below-terrain unit renders.
//
// This daylights only through the wasm + main-thread-Manifold path (executeRecipe), which the native
// invariant tests cannot reach — hence an e2e. It builds a CURVED sheet (the flat mesh-span-domain
// fixture is monotone and never triggered the bug) larger than the domain and sets the domain inside it.

import { test, expect } from "@playwright/test";

/** A domed, curved triangle sheet on an n×n grid over [0, ext]², z bulging up in the middle. Curvature
 *  is what broke nearest-triangle signing; the footprint (ext) exceeds the domain set below. */
function curvedSheetObj(ext: number, zBase: number, zBulge: number, n = 12): string {
  const lines: string[] = ["# curved dome sheet, wound CCW from +Z"];
  const idx = (i: number, j: number) => j * (n + 1) + i + 1; // OBJ is 1-indexed
  for (let j = 0; j <= n; j++) {
    for (let i = 0; i <= n; i++) {
      const x = (i / n) * ext;
      const y = (j / n) * ext;
      const u = i / n - 0.5;
      const v = j / n - 0.5;
      const z = zBase + zBulge * Math.cos(Math.PI * u) * Math.cos(Math.PI * v); // dome
      lines.push(`v ${x} ${y} ${z}`);
    }
  }
  for (let j = 0; j < n; j++) {
    for (let i = 0; i < n; i++) {
      lines.push(`f ${idx(i, j)} ${idx(i + 1, j)} ${idx(i + 1, j + 1)}`);
      lines.push(`f ${idx(i, j)} ${idx(i + 1, j + 1)} ${idx(i, j + 1)}`);
    }
  }
  return lines.join("\n") + "\n";
}

test("imported contact in Solids mode renders when the domain sits inside the mesh footprint", async ({ page }) => {
  // Mesh spans [0, 3000]² (curved, z≈4000..5500); domain [800, 2200]² × z[2000, 7000] — laterally
  // INSIDE the footprint, and taller than the mesh (mirrors the reported lidar project exactly).
  const objText = curvedSheetObj(3000, 4000, 1500);
  const project = {
    version: 1 as const,
    name: "domain-inside-footprint",
    extent: [800, 2200, 800, 2200, 2000, 7000] as [number, number, number, number, number, number],
    elements: [
      {
        kind: "importedSurface" as const,
        id: "el-1",
        name: "dome",
        role: "contact" as const,
        relation: "conformable" as const,
        objText,
        extendToBounds: false,
      },
    ],
    view: { camera: [1500, 1500, 12000] as [number, number, number], target: [1500, 1500, 4500] as [number, number, number], up: [0, 0, 1] as [number, number, number], fov: 50, renderMode: "solid" as const },
  };

  await page.goto("/");
  await page.waitForFunction(() => typeof (window as any).__loadProject === "function");
  await page.evaluate((p) => (window as any).__loadProject(p), project);
  await expect(page.getByText("status: done")).toBeVisible({ timeout: 30_000 });
  await expect(page.getByTestId("stale-scrim")).toHaveCount(0, { timeout: 30_000 });

  // THE assertion: Solids mode produced non-empty rock units. Before the fix NOTHING rendered —
  // Manifold rejected the contoured primitive and executeRecipe dropped every unit. A single contact
  // yields TWO units: the below-terrain solid (unit 0, floor → dome) and the void above it (unit 1,
  // dome → box top). We check the BELOW unit specifically tops out at the dome, not the box ceiling.
  const stats = await page.evaluate(() => {
    const surfaces = (window as any).__surfaces as { positions: Float32Array; indices: Uint32Array; unit?: number }[];
    const perUnit = new Map<number, { tris: number; zmin: number; zmax: number }>();
    let solids = 0;
    for (const s of surfaces) {
      if (s.indices.length === 0) continue;
      solids++;
      const u = s.unit ?? -1;
      const rec = perUnit.get(u) ?? { tris: 0, zmin: Infinity, zmax: -Infinity };
      rec.tris += s.indices.length / 3;
      for (const i of new Set<number>(s.indices)) {
        const z = s.positions[i * 3 + 2];
        if (z < rec.zmin) rec.zmin = z;
        if (z > rec.zmax) rec.zmax = z;
      }
      perUnit.set(u, rec);
    }
    const below = perUnit.get(0);
    return { solids, unitCount: perUnit.size, below };
  });

  expect(stats.solids, "rock unit solids render (the volume view is non-empty)").toBeGreaterThan(0);
  expect(stats.below, "the below-terrain unit (unit 0) meshed").toBeTruthy();
  expect(stats.below!.tris, "the below-terrain unit has geometry").toBeGreaterThan(0);
  // The below unit fills from the domain floor (z0=2000) up to the dome — not a sliver, not the box top.
  expect(stats.below!.zmin, `below unit reaches the domain floor: zmin=${stats.below!.zmin}`).toBeLessThan(2100);
  expect(stats.below!.zmax, `below unit tops out at the dome, under the box top (z1=7000): zmax=${stats.below!.zmax}`).toBeLessThan(6000);
});
