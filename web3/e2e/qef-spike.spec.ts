// Engine regression (Constitution I — geometric invariant, NOT a GemPy reference; and the
// native↔wasm parity it protects). A single FLIPPED (negated-pole) erosion plane must mesh
// cleanly: every rendered contact vertex ON the surface (none flung to the domain ceiling),
// AND no interior hole.
//
// The historical "QEF ceiling spike": one dual-contour vertex at (625,375,z1=1000) forming a
// tall thin triangular spike. ROOT CAUSE: the dipping contact runs EXACTLY along a grid plane
// (z=750 at x=750), so `field − iso` at that corner is at the round-off floor (~1e-17). Its
// discrete sign (`<= 0`) differs between native and wasm32 in the last ULPs, flipping the
// corner between below/above the contact — which flipped the contact-face classification and,
// on wasm only, kept a contact face stretched up to a domain-cap vertex at the ceiling. The
// fix (octree.rs `CONTACT_GRAZE_EPS` in `sample_corners`) snaps a near-zero corner to a fixed
// side so the contour is identical across builds. This spec is the live-app behaviour gate;
// it daylights only through the wasm path, which the native invariant tests cannot reproduce.
//
// It guards BOTH failure modes: the spike (a ceiling vertex) AND a hole (an interior open
// edge) — an earlier wrong fix that DELETED the spike triangles passed the ceiling check but
// tore a hole, so both are asserted here.

import { test, expect } from "@playwright/test";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

const fixture = JSON.parse(
  readFileSync(fileURLToPath(new URL("./fixtures/qef-spike-repro.geoproj.json", import.meta.url)), "utf8"),
);

test("flipped erosion plane meshes with no ceiling spike and no hole", async ({ page }) => {
  await page.goto("/");
  await page.waitForFunction(() => typeof (window as any).__loadProject === "function");
  await page.evaluate((p) => (window as any).__loadProject(p), fixture);
  await expect(page.getByText("status: done")).toBeVisible({ timeout: 30_000 });

  const stats = await page.evaluate(() => {
    const surfaces = (window as any).__surfaces as {
      positions: Float32Array;
      indices: Uint32Array;
    }[];
    let total = 0;
    let maxZ = -Infinity;
    let ceilingCount = 0;
    let interiorOpenEdges = 0;
    for (const s of surfaces) {
      const pos = s.positions;
      const idx = s.indices;
      // Highest z among vertices REFERENCED by a triangle. (The open sheet keeps the closed
      // solid's now-orphaned ceiling cap-vertex POSITIONS unreferenced by `indices`; exclude
      // them — the spike is a TRIANGLE that uses a ceiling vertex.)
      for (const i of new Set<number>(idx)) {
        const z = pos[i * 3 + 2];
        total++;
        if (z > maxZ) maxZ = z;
        if (z > 950) ceilingCount++;
      }
      // Interior open edges = holes. An edge with incidence 1 is allowed only on the box rim
      // (the open sheet's legitimate perimeter); an interior one is a tear.
      const onBox = (i: number) => {
        const x = pos[i * 3];
        const y = pos[i * 3 + 1];
        return x < 1 || x > 999 || y < 1 || y > 999;
      };
      const inc = new Map<string, number>();
      for (let t = 0; t < idx.length; t += 3) {
        const tri = [idx[t], idx[t + 1], idx[t + 2]];
        for (const [a, b] of [[tri[0], tri[1]], [tri[1], tri[2]], [tri[2], tri[0]]]) {
          const k = a < b ? `${a},${b}` : `${b},${a}`;
          inc.set(k, (inc.get(k) ?? 0) + 1);
        }
      }
      for (const [k, c] of inc) {
        if (c !== 1) continue;
        const [a, b] = k.split(",").map(Number);
        if (!(onBox(a) && onBox(b))) interiorOpenEdges++;
      }
    }
    return { total, maxZ, ceilingCount, interiorOpenEdges };
  });

  expect(stats.total).toBeGreaterThan(0);
  // No spike: the contact tops at z≈830 (x=950 row); the dipping rise stays well under 900.
  expect(stats.ceilingCount, `${stats.ceilingCount} rendered vertices flung near the z=1000 ceiling`).toBe(0);
  expect(stats.maxZ, `highest rendered vertex z=${stats.maxZ} (contact tops at ~830)`).toBeLessThan(900);
  // No hole: every interior edge is shared by two triangles.
  expect(stats.interiorOpenEdges, `${stats.interiorOpenEdges} interior open edges (a torn hole)`).toBe(0);
});
