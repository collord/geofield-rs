// Drillhole import (prototype): pasting a post-desurvey lithology interval table auto-builds the
// conformable stack — contacts extracted at unit transitions ("top of <unit>", grouped under the
// lower unit), chronology from superposition votes (transitive: hole D's Sand pinch-out still
// orders Sand vs Shale), lithologies from unit names, extent refit to the data bbox — and the
// panel Apply then fits at the new extent. Re-import REPLACES the generated surfaces.

import { test, expect } from "@playwright/test";
import type { Project } from "../src/HierarchyPanel/project";

// 4 vertical holes × Cover/Sand/Shale/Basement at x≈5000–5300, y≈8000–8300 (far outside the
// default [0,1000]³ extent, so the data-fit extent is observable). Hole D pinches out Sand.
// Hole B's Sand/Shale boundary endpoints deliberately disagree by 0.5 (midpoint + warning path).
const TABLE = [
  "holeid\tstart_x\tstart_y\tstart_z\tend_x\tend_y\tend_z\tunitname",
  // Hole A (5000, 8000): full sequence.
  "A\t5000\t8000\t1000\t5000\t8000\t950\tCover",
  "A\t5000\t8000\t950\t5000\t8000\t880\tSand",
  "A\t5000\t8000\t880\t5000\t8000\t800\tShale",
  "A\t5000\t8000\t800\t5000\t8000\t700\tBasement",
  // Hole B (5300, 8000): full sequence; Sand→Shale boundary logged with a 0.5 gap.
  "B\t5300\t8000\t1000\t5300\t8000\t940\tCover",
  "B\t5300\t8000\t940\t5300\t8000\t870.5\tSand",
  "B\t5300\t8000\t870\t5300\t8000\t790\tShale",
  "B\t5300\t8000\t790\t5300\t8000\t700\tBasement",
  // Hole C (5000, 8300): full sequence.
  "C\t5000\t8300\t1000\t5000\t8300\t930\tCover",
  "C\t5000\t8300\t930\t5000\t8300\t860\tSand",
  "C\t5000\t8300\t860\t5000\t8300\t780\tBasement",
  // Hole D (5300, 8300): Sand pinched out — Cover sits directly on Shale.
  "D\t5300\t8300\t1000\t5300\t8300\t900\tCover",
  "D\t5300\t8300\t900\t5300\t8300\t810\tShale",
  "D\t5300\t8300\t810\t5300\t8300\t700\tBasement",
].join("\n");

// Contact expectations: top of Sand = A,B,C Cover/Sand (3); top of Shale = A,B Sand/Shale + D
// Cover/Shale (3); top of Basement = A,B Shale/Basement + C Sand/Basement + D Shale/Basement (4).
const EXPECTED = [
  { name: "top of Sand", points: 3 },
  { name: "top of Shale", points: 3 },
  { name: "top of Basement", points: 4 },
];

const proj = (page: import("@playwright/test").Page) =>
  page.evaluate(() => (window as unknown as { __project: () => Project }).__project());

test("pasting an interval table auto-builds the stack, refits the extent, and fits", async ({ page }) => {
  test.setTimeout(180_000);
  await page.goto("/");

  await page.getByTestId("import-drillholes-btn").click();
  await page.getByTestId("drillhole-text").fill(TABLE);

  const preview = page.getByTestId("drillhole-preview");
  await expect(preview).toContainText("4 holes, 14 intervals, 10 contacts");
  await expect(preview).toContainText("youngest → oldest: Cover, Sand, Shale, Basement");
  await expect(preview).toContainText("using midpoint"); // hole B's 0.5 boundary gap

  await page.getByTestId("drillhole-apply").click();
  await expect(page.getByTestId("drillhole-import-modal")).toHaveCount(0);
  await expect(page.getByTestId("apply-bar")).toBeVisible();

  // The committed model: elements youngest-first with the expected points + one dip-0 orientation.
  const p = await proj(page);
  const series = p.elements.filter((e) => e.kind === "series");
  expect(series.map((e) => e.name)).toEqual(EXPECTED.map((e) => e.name));
  for (const [i, exp] of EXPECTED.entries()) {
    const el = series[i];
    if (el.kind !== "series") throw new Error("unreachable");
    expect(el.relation).toBe("conformable");
    expect(el.interfaces).toHaveLength(1);
    expect(el.interfaces[0].points).toHaveLength(exp.points);
    expect(el.interfaces[0].orientations).toHaveLength(1);
    expect(el.interfaces[0].orientations[0].dip).toBe(0);
    // Unit 0 (the below-contact slab) is assigned the lower unit's lithology by name.
    const lithoId = p.catalog?.assignment[`${el.id}:0`];
    const litho = p.catalog?.lithologies.find((l) => l.id === lithoId);
    expect(litho?.name).toBe(exp.name.replace("top of ", ""));
  }
  // All four unit lithologies exist (Cover too, though its volume has no element yet).
  for (const name of ["Cover", "Sand", "Shale", "Basement"]) {
    expect(p.catalog?.lithologies.some((l) => l.name === name)).toBe(true);
  }
  // Extent refit to the padded data bbox (not the default [0,1000]³).
  const [x0, x1, y0, y1, z0, z1] = p.extent;
  expect(x0).toBeLessThan(5000);
  expect(x1).toBeGreaterThan(5300);
  expect(y0).toBeLessThan(8000);
  expect(y1).toBeGreaterThan(8300);
  expect(z0).toBeLessThan(700);
  expect(z1).toBeGreaterThan(1000);
  // Raw intervals persisted for round-trip.
  expect(p.drillholes).toHaveLength(14);

  // Panel Apply: the one fit, at the new extent.
  await page.getByTestId("apply").click();
  await expect(page.getByTestId("stale-scrim")).toHaveCount(0, { timeout: 60_000 });
  await expect(page.getByTestId("apply-bar")).toHaveCount(0, { timeout: 60_000 });
  await expect(page.getByText("status: done")).toBeVisible({ timeout: 60_000 });
});

test("re-importing the same table replaces the generated surfaces (no duplicates)", async ({ page }) => {
  test.setTimeout(120_000);
  await page.goto("/");

  for (let round = 0; round < 2; round++) {
    await page.getByTestId("import-drillholes-btn").click();
    await page.getByTestId("drillhole-text").fill(TABLE);
    await page.getByTestId("drillhole-apply").click();
    await expect(page.getByTestId("drillhole-import-modal")).toHaveCount(0);
  }
  const p = await proj(page);
  expect(p.elements.filter((e) => e.kind === "series")).toHaveLength(EXPECTED.length);
  // Lithologies matched by name on the second pass — no duplicate catalog entries either.
  const names = p.catalog?.lithologies.map((l) => l.name) ?? [];
  expect(names.filter((n) => n === "Sand")).toHaveLength(1);
});
