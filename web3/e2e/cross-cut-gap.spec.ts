// Regression (docs/10): in SOLID mode the cross-cutting-vein's rock units must (1) keep their full
// block on BOTH sides of the vein — the body-primary shared build used to drop the thin middle unit's
// far block — and (2) meet with no inter-unit gap at the binary-aligned z=500 bed, whose grid-aligned
// cap floats off the contact until the bed-cap projection lands it. Guards the wasm render path
// (the native guard is `cross_cut_vertical_three_beds_keeps_left_block`).
import { test, expect } from "@playwright/test";

async function waitForSolids(page: import("@playwright/test").Page) {
  await expect
    .poll(() =>
      page.evaluate(() => {
        const s = (window as { __surfaces?: { unit?: number }[] }).__surfaces ?? [];
        return s.length > 0 && s.every((x) => x.unit !== undefined && x.unit !== null);
      }),
      { timeout: 25_000 },
    )
    .toBe(true);
}

test("cross-cut solids have no inter-unit gap on either side of the vein", async ({ page }) => {
  await page.goto("/");
  await page.getByTestId("seed-template").selectOption("cross-cutting-vein");
  await expect(page.getByText("status: done")).toBeVisible({ timeout: 30_000 });
  await page.getByTestId("render-toggle").click();
  await expect(page.getByText("status: done")).toBeVisible({ timeout: 30_000 });
  await waitForSolids(page);

  // For the rock units (series 1) on the LEFT (x<340) and RIGHT (x>660) of the vein, gather each
  // unit's interior z-extent + vertex count, and confirm consecutive units meet (top_i ≈ bottom_{i+1}).
  const sides = await page.evaluate(() => {
    const s = (window as { __surfaces?: { unit: number; series?: number; positions: Float32Array }[] }).__surfaces ?? [];
    const extent = (xlo: number, xhi: number) =>
      s
        .filter((p) => p.unit !== undefined && p.unit !== null)
        .map((p) => {
          let zmin = Infinity, zmax = -Infinity, n = 0;
          for (let i = 0; i + 2 < p.positions.length; i += 3) {
            const x = p.positions[i], z = p.positions[i + 2];
            if (x > xlo && x < xhi) { zmin = Math.min(zmin, z); zmax = Math.max(zmax, z); n++; }
          }
          return { unit: p.unit, zmin, zmax, n };
        })
        .filter((u) => Number.isFinite(u.zmin))
        .sort((a, b) => a.zmin - b.zmin);
    return { left: extent(60, 340), right: extent(660, 940) };
  });

  for (const [name, units] of [["left", sides.left], ["right", sides.right]] as const) {
    expect(units.length, `${name} has all units`).toBeGreaterThanOrEqual(4);
    for (let i = 1; i < units.length; i++) {
      // Each rock block is a real slab on this side (not a dropped-to-sliver block — docs/10).
      expect(units[i].n, `${name} unit ${units[i].unit} present`).toBeGreaterThan(40);
      // No inter-unit gap: unit i's bottom meets unit i-1's top (within a fraction of a unit).
      expect(
        Math.abs(units[i].zmin - units[i - 1].zmax),
        `${name} gap between unit ${units[i - 1].unit} (top ${units[i - 1].zmax.toFixed(1)}) and unit ${units[i].unit} (bottom ${units[i].zmin.toFixed(1)})`,
      ).toBeLessThan(12);
    }
  }
});
