// Feature 005 (cross-sections) US3+US5 behavior gate — THE END-TO-END PAYOFF (SC-002):
// digitize one contact on 3 parallel sections, Apply ONCE, and the lines loft into a
// fitted surface whose trace on every digitized slice passes within ε = 0.1·spacing of
// every drawn vertex. Then US5 parity: the lofted surface takes the erode relation
// (immediate, non-dirty — US-DATA-3) and derives solids like any typed surface.

import { test, expect, type Page } from "@playwright/test";

type Hooks = {
  __addSectionSet: (s: object) => string;
  __openSection: (id: string, k: number) => void;
  __digitizeLine: (
    e: string | null,
    i: string | null,
    s: string,
    k: number,
    v: [number, number][],
  ) => { elementId: string; interfaceId: string; lineId: string } | null;
  __sectionTraces: (id: string, k: number) => { series: number; polylines: number[][][] }[];
  __surfaces: { series: number; unit?: number }[];
};

const SPACING = 250;
const EPS = 0.1 * SPACING; // the loft-fidelity tolerance (quickstart.md)

// The drawn lines: a gently NE-dipping planar contact — z varies along the line (du) AND
// across sections (k), so the loft is fully determined (no under-constrained warning).
const lineFor = (k: number): [number, number][] => [
  [50, 400 + 30 * k],
  [300, 430 + 30 * k],
  [550, 460 + 30 * k],
];

async function digitizeThreeSlices(page: Page): Promise<string> {
  await page.goto("/");
  await expect(page.getByTestId("section-new")).toBeVisible({ timeout: 30_000 });
  // Serial set: baseline S→N at x=500, planes at x = 250, 500, 750 (offset −250).
  const setId = await page.evaluate(
    (spacing) =>
      (window as unknown as Hooks).__addSectionSet({
        a: [500, 200], b: [500, 800], spacing, count: 3, offset: -spacing,
      }),
    SPACING,
  );
  // Slice 0 creates the target surface; slices 1–2 add to the SAME surface (US-DIG-3).
  const first = await page.evaluate(
    ({ id, v }) => (window as unknown as Hooks).__digitizeLine(null, null, id, 0, v),
    { id: setId, v: lineFor(0) },
  );
  if (!first) throw new Error("digitize failed");
  for (const k of [1, 2]) {
    await page.evaluate(
      ({ id, v, el, ifid, k }) => (window as unknown as Hooks).__digitizeLine(el, ifid, id, k, v),
      { id: setId, v: lineFor(k), el: first.elementId, ifid: first.interfaceId, k },
    );
  }
  return setId;
}

test("three parallel digitized lines loft into one fitted surface; traces land on the drawn vertices", async ({ page }) => {
  const setId = await digitizeThreeSlices(page);

  // One deferred Apply (US-DIG-4): the scrim shows until it lands.
  await expect(page.getByTestId("stale-scrim")).toBeVisible();
  await page.getByTestId("apply").click();
  await expect(page.getByText("status: done")).toBeVisible({ timeout: 60_000 });
  await expect(page.getByTestId("stale-scrim")).toHaveCount(0);

  // On EVERY digitized slice, the re-fit surface's trace passes through the drawn
  // vertices within ε (point-to-segment distance against the trace polylines).
  for (const k of [0, 1, 2]) {
    const traces = await page.evaluate(
      ({ id, k }) => (window as unknown as Hooks).__sectionTraces(id, k),
      { id: setId, k },
    );
    const polys = traces.flatMap((t) => t.polylines);
    expect(polys.length).toBeGreaterThan(0);
    const planeX = 250 + 250 * k;
    for (const [du, dw] of lineFor(k)) {
      // world position of the drawn vertex: baseline a=[500,200] + u·du (u=+Y), plane x
      const v = [planeX, 200 + du, dw];
      let best = Infinity;
      for (const poly of polys) {
        for (let i = 0; i + 1 < poly.length; i++) {
          const a = poly[i];
          const b = poly[i + 1];
          const ab = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
          const av = [v[0] - a[0], v[1] - a[1], v[2] - a[2]];
          const len2 = ab[0] ** 2 + ab[1] ** 2 + ab[2] ** 2;
          const t = len2 > 0 ? Math.max(0, Math.min(1, (av[0] * ab[0] + av[1] * ab[1] + av[2] * ab[2]) / len2)) : 0;
          const d = Math.hypot(v[0] - (a[0] + ab[0] * t), v[1] - (a[1] + ab[1] * t), v[2] - (a[2] + ab[2] * t));
          if (d < best) best = d;
        }
      }
      expect(best).toBeLessThan(EPS);
    }
  }
});

test("US5 parity: the lofted surface takes the erode relation (immediate, no dirty), truncates older beds, and derives solids", async ({ page }) => {
  test.slow(); // three full fits (apply, erode refit, solids refit) — tight under parallel-worker CPU contention
  // An OLDER stack to compose against: the conformal column (one 3-contact series).
  await page.goto("/");
  await page.getByTestId("seed-template").selectOption("conformal-column");
  await expect(page.getByText("status: done")).toBeVisible({ timeout: 60_000 });

  // Digitize a steeply dipping surface through the column on 3 serial slices.
  const setId = await page.evaluate(
    (spacing) =>
      (window as unknown as Hooks).__addSectionSet({
        a: [500, 200], b: [500, 800], spacing, count: 3, offset: -spacing,
      }),
    SPACING,
  );
  const first = await page.evaluate(
    ({ id }) => (window as unknown as Hooks).__digitizeLine(null, null, id, 0, [[50, 250], [550, 700]]),
    { id: setId },
  );
  if (!first) throw new Error("digitize failed");
  for (const k of [1, 2]) {
    await page.evaluate(
      ({ id, el, ifid, k }) =>
        (window as unknown as Hooks).__digitizeLine(el, ifid, id, k, [[50, 250 + 30 * k], [550, 700 + 30 * k]]),
      { id: setId, el: first.elementId, ifid: first.interfaceId, k },
    );
  }
  // The digitized series lands at the OLDEST slot (the floor shows no seam control) —
  // move it above the column so it has a boundary below it, like any reorderable element.
  await page.getByTestId("reorder-up").last().click();
  await page.getByTestId("apply").click();
  // "status: done" alone races with the PREVIOUS fit's text (the stale-preview gotcha) —
  // the apply-bar unmounting is the true "this Apply landed" signal (dirty cleared on done).
  await expect(page.getByTestId("apply-bar")).toHaveCount(0, { timeout: 60_000 });

  // Truncation metric: how far the OLDER column's sheets poke ABOVE the digitized surface
  // (≈ the plane z = 250 + 0.9·(y−200) + 0.12·(x−250) through the drawn lines). Series 0
  // is the digitized element (youngest after the reorder).
  const maxAbove = () =>
    page.evaluate(() => {
      const older = (window as unknown as { __surfaces: { series: number; positions: Float32Array }[] }).__surfaces
        .filter((s) => s.series !== 0);
      let m = -Infinity;
      for (const p of older) {
        for (let i = 0; i + 2 < p.positions.length; i += 3) {
          const above = p.positions[i + 2] - (250 + 0.9 * (p.positions[i + 1] - 200) + 0.12 * (p.positions[i] - 250));
          if (above > m) m = above;
        }
      }
      return m;
    });
  // Conformable cuts nothing (§3.1.2): the flat beds cross far above the dipping surface.
  expect(await maxAbove()).toBeGreaterThan(100);

  // Relation is the standard series control (US-DIG-6): switching to erode re-fits
  // immediately WITHOUT dirtying (US-DATA-3 — same as any typed series). Wait for the NEW
  // fit via status-log growth (no dirty/apply-bar signal exists on this path).
  const len0 = await page.evaluate(() => ((window as unknown as { __statusLog?: string[] }).__statusLog ?? []).length);
  await page.locator(".seam-state", { hasText: "erode" }).first().click();
  await expect
    .poll(async () => page.evaluate(() => ((window as unknown as { __statusLog?: string[] }).__statusLog ?? []).length), {
      timeout: 60_000,
    })
    .toBeGreaterThanOrEqual(len0 + 2);
  await expect(page.getByText("status: done")).toBeVisible({ timeout: 60_000 });
  await expect(page.getByTestId("apply-bar")).toHaveCount(0);

  // …and the digitized erosion TRUNCATES the older beds flush along it (the erode cut
  // rule, indistinguishable from a typed erosion): nothing survives above the surface
  // beyond mesh-cell tolerance (~extent/2^depth ≈ 16).
  expect(await maxAbove()).toBeLessThan(25);

  // Solids mode: the digitized surface derives lithology units like a typed one.
  const len1 = await page.evaluate(() => ((window as unknown as { __statusLog?: string[] }).__statusLog ?? []).length);
  await page.getByTestId("render-toggle").click();
  await expect
    .poll(async () => page.evaluate(() => ((window as unknown as { __statusLog?: string[] }).__statusLog ?? []).length), {
      timeout: 60_000,
    })
    .toBeGreaterThanOrEqual(len1 + 2);
  await expect(page.getByText("status: done")).toBeVisible({ timeout: 60_000 });
  const solidUnits = await page.evaluate(
    () => (window as unknown as Hooks).__surfaces.filter((s) => s.unit !== undefined).length,
  );
  expect(solidUnits).toBeGreaterThan(0);
});
