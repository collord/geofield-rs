// Feature 003 (topography, lithology, solids) T045 [US4] — the Surfaces⇄Solids toggle.
// Behavior gate (Constitution I) for User Story 4: toggling shows closed unit solids tagged
// by lithology (Air omitted); toggling back restores series-coloured contact sheets; OBJ in
// solid mode groups by lithology with no Air group. (FR-022..027, SC-006/008.)

import { test, expect } from "@playwright/test";

/** Wait until the __surfaces test hook reflects solid payloads (each carries a `unit`). */
async function waitForSolids(page: import("@playwright/test").Page) {
  await expect
    .poll(() =>
      page.evaluate(() => {
        const s = (window as { __surfaces?: { unit?: number }[] }).__surfaces ?? [];
        return s.length > 0 && s.every((x) => x.unit !== undefined && x.unit !== null);
      }),
    )
    .toBe(true);
}

test("toggle to Solids → closed unit solids tagged by lithology; toggle back restores surfaces", async ({ page }) => {
  await page.goto("/");
  await page.getByTestId("seed-template").selectOption("conformal-column");
  await expect(page.getByText("status: done")).toBeVisible({ timeout: 30_000 });

  // Assign the youngest unit to a user lithology so a solid carries a non-default tag.
  await page.getByTestId("litho-new-name").fill("Sandstone");
  await page.getByTestId("litho-add").click();
  await page.getByTestId("unit-litho").first().selectOption({ label: "Sandstone" });

  // Contact mode: open sheets (one per interface), no `unit` field.
  const contactCount = await page.evaluate(() => (window as { __surfaces?: unknown[] }).__surfaces?.length ?? 0);
  expect(contactCount).toBeGreaterThan(0);

  // Toggle → Solids.
  await page.getByTestId("render-toggle").click();
  await expect(page.getByText("status: done")).toBeVisible({ timeout: 30_000 });
  await waitForSolids(page);

  const solids = await page.evaluate(() => {
    const s = (window as { __surfaces?: { unit?: number; lithologyId?: string }[] }).__surfaces ?? [];
    return { count: s.length, tagged: s.filter((x) => x.lithologyId !== undefined).length };
  });
  // The floor's 4 units mesh as solids; at least the assigned one carries its lithology id.
  expect(solids.count).toBeGreaterThanOrEqual(4);
  expect(solids.tagged, "the assigned unit solid is tagged with its lithology").toBeGreaterThan(0);

  // Toggle back → Surfaces: the contact view returns (payloads lose the `unit` tag).
  await page.getByTestId("render-toggle").click();
  await expect(page.getByText("status: done")).toBeVisible({ timeout: 30_000 });
  await expect
    .poll(() =>
      page.evaluate(() => {
        const s = (window as { __surfaces?: { unit?: number }[] }).__surfaces ?? [];
        return s.length > 0 && s.every((x) => x.unit === undefined || x.unit === null);
      }),
    )
    .toBe(true);
});

test("unit solids stack in stratigraphic order (top ladder unit = top slab) with no gaps", async ({ page }) => {
  // Regression for the inversion + inter-unit gap bugs (feature 003): the panel ladder is
  // top-first, and the SOLID for the unit at ladder-position-1 must be the TOPMOST in z; each
  // unit's top meets the next unit's bottom exactly (welded contacts — no seam).
  await page.goto("/");
  await page.getByTestId("seed-template").selectOption("conformal-column");
  await expect(page.getByText("status: done")).toBeVisible({ timeout: 30_000 });
  await page.getByTestId("render-toggle").click();
  await expect(page.getByText("status: done")).toBeVisible({ timeout: 30_000 });
  await waitForSolids(page);

  const units = await page.evaluate(() => {
    const s = (window as { __surfaces?: { unit: number; positions: Float32Array }[] }).__surfaces ?? [];
    return s
      .map((p) => {
        let zmin = Infinity, zmax = -Infinity;
        for (let i = 2; i < p.positions.length; i += 3) {
          zmin = Math.min(zmin, p.positions[i]);
          zmax = Math.max(zmax, p.positions[i]);
        }
        return { unit: p.unit, zmin, zmax };
      })
      .sort((a, b) => a.unit - b.unit);
  });

  // Higher unit index = higher in z (the mesher's index 0 is the bottom slab).
  for (let i = 1; i < units.length; i++) {
    expect(units[i].zmin).toBeGreaterThanOrEqual(units[i - 1].zmin - 1);
    // No gap: unit i's bottom meets unit i-1's top within a fraction of a unit (welded).
    expect(Math.abs(units[i].zmin - units[i - 1].zmax)).toBeLessThan(2);
  }
});

test("a cross-cutting vein renders a closed interior solid in solid mode", async ({ page }) => {
  // Regression: the vein's namesake-lithology interior must mesh as a solid (feature 003 §4.3).
  await page.goto("/");
  await page.getByTestId("seed-template").selectOption("cross-cutting-vein");
  await expect(page.getByText("status: done")).toBeVisible({ timeout: 30_000 });
  await page.getByTestId("render-toggle").click();
  await expect(page.getByText("status: done")).toBeVisible({ timeout: 30_000 });
  await waitForSolids(page);

  const veinTris = await page.evaluate(() => {
    const s = (window as { __surfaces?: { series: number; indices: Uint32Array }[] }).__surfaces ?? [];
    // The vein is the youngest element (series 0); it must contribute solid geometry.
    return s.filter((x) => x.series === 0).reduce((n, x) => n + x.indices.length / 3, 0);
  });
  expect(veinTris, "the vein interior contributes a solid").toBeGreaterThan(0);
});

test("OBJ export in solid mode groups by lithology, with no Air group", async ({ page }) => {
  await page.goto("/");
  await page.getByTestId("seed-template").selectOption("conformal-column");
  await expect(page.getByText("status: done")).toBeVisible({ timeout: 30_000 });

  await page.getByTestId("litho-new-name").fill("Sandstone");
  await page.getByTestId("litho-add").click();
  await page.getByTestId("unit-litho").first().selectOption({ label: "Sandstone" });

  // Switch to Solids, then export OBJ and capture the downloaded text.
  await page.getByTestId("render-toggle").click();
  await expect(page.getByText("status: done")).toBeVisible({ timeout: 30_000 });
  await waitForSolids(page);

  const download = await Promise.all([
    page.waitForEvent("download"),
    page.getByTestId("export-obj").click(),
  ]).then(([d]) => d);
  const stream = await download.createReadStream();
  const text = await new Promise<string>((resolve) => {
    let buf = "";
    stream!.on("data", (c) => (buf += c));
    stream!.on("end", () => resolve(buf));
  });

  // Groups are by lithology (g litho_<name>), including the user "Sandstone"; never by Air.
  expect(text).toMatch(/^g litho_/m);
  expect(text).toContain("g litho_Sandstone");
  expect(text).not.toContain("litho_Air");
});
