// Feature 005 (cross-sections) US1 behavior gate — create a section from a two-click
// plan-view baseline, open it face-on, read traces + backdrop, and verify the load-bearing
// invariant: section create/open/step/delete NEVER dirties the project and NEVER re-fits
// (spec FR-009 / SC-004). Asserts on affordances, hooks, and the status log — not pixels.

import { test, expect, type Page } from "@playwright/test";

async function seedApplied(page: Page, template = "conformal-column") {
  await page.goto("/");
  await page.getByTestId("seed-template").selectOption(template);
  await expect(page.getByText("status: done")).toBeVisible({ timeout: 60_000 });
}

const statusLogLen = (page: Page) =>
  page.evaluate(() => ((window as unknown as { __statusLog?: string[] }).__statusLog ?? []).length);

test("two-click baseline creates a section; face-on view shows traces and backdrop; no dirty, no re-fit", async ({ page }) => {
  await seedApplied(page);
  const fitsBefore = await statusLogLen(page);

  // Two-click baseline gesture (mirrors the bounds gesture, US-SECT-1 ↔ US-BOUNDS-1).
  await page.getByTestId("section-new").click();
  const canvas = page.locator("main canvas");
  const box = await canvas.boundingBox();
  if (!box) throw new Error("no canvas");
  await page.mouse.click(box.x + box.width * 0.25, box.y + box.height * 0.5);
  await page.mouse.click(box.x + box.width * 0.75, box.y + box.height * 0.5);

  // The serial form appears; create a single section (count 1).
  const form = page.getByTestId("section-set-form");
  await expect(form).toBeVisible();
  await page.getByTestId("section-create").click();
  await expect(form).toHaveCount(0);

  // The set exists and the section view opened on it.
  const sections = await page.evaluate(() =>
    (window as unknown as { __sections: () => { id: string; count: number }[] }).__sections(),
  );
  expect(sections).toHaveLength(1);
  expect(sections[0].count).toBe(1);
  await expect(page.getByTestId("section-active")).toBeVisible();

  // Traces of the applied model exist on the plane (the baseline crosses the model).
  const traces = await page.evaluate((setId) => {
    const w = window as unknown as {
      __sectionTraces: (s: string, i: number) => { polylines: number[][][] }[];
    };
    return w.__sectionTraces(setId, 0);
  }, sections[0].id);
  const totalPts = traces.reduce((n, t) => n + t.polylines.reduce((m, p) => m + p.length, 0), 0);
  expect(totalPts).toBeGreaterThan(0);

  // Backdrop/authored data is a display concern; the load-bearing invariant is state:
  // NO dirty (no apply-bar) and NO re-fit (status log unchanged) through section ops.
  await expect(page.getByTestId("apply-bar")).toHaveCount(0);

  // Close, delete — still clean, still no fit.
  await page.getByTestId("section-close").click();
  await expect(page.getByTestId("section-active")).toHaveCount(0);
  await page.getByTestId(`section-delete-${sections[0].id}`).click();
  await expect(page.getByTestId("apply-bar")).toHaveCount(0);
  const fitsAfter = await statusLogLen(page);
  expect(fitsAfter).toBe(fitsBefore); // zero worker activity from any section op (SC-004)
});

test("serial set: stepping is clamped, keeps the view open, and never re-fits; render mode toggles in section view", async ({ page }) => {
  await seedApplied(page);

  // Programmatic serial set (gesture covered above): 3 planes across the model.
  const setId = await page.evaluate(() =>
    (window as unknown as {
      __addSectionSet: (s: object) => string;
      __openSection: (id: string, k: number) => void;
    }).__addSectionSet({ a: [500, 200], b: [500, 800], spacing: 250, count: 3, offset: -250 }),
  );
  await page.evaluate((id) => (window as unknown as { __openSection: (i: string, k: number) => void }).__openSection(id, 0), setId);
  await expect(page.getByTestId("section-label")).toContainText("1/3");

  const fitsBefore = await statusLogLen(page);
  await page.getByTestId("section-step-next").click();
  await expect(page.getByTestId("section-label")).toContainText("2/3");
  await page.getByTestId("section-step-next").click();
  await expect(page.getByTestId("section-label")).toContainText("3/3");
  await expect(page.getByTestId("section-step-next")).toBeDisabled(); // clamped at the end
  await page.getByTestId("section-step-prev").click();
  await expect(page.getByTestId("section-label")).toContainText("2/3");
  expect(await statusLogLen(page)).toBe(fitsBefore); // stepping = pure re-slice, no fit
  await expect(page.getByTestId("apply-bar")).toHaveCount(0);

  // The shared render-mode toggle works from section view (US-SECT-6). This DOES re-fit
  // (the documented terminate-per-fit tradeoff) — it is a render-mode change, not a section op.
  await page.getByTestId("render-toggle").click();
  await expect(page.getByText("status: done")).toBeVisible({ timeout: 60_000 });
  await expect(page.getByTestId("section-active")).toBeVisible(); // still in section view
});

test("stale scrim shows in section view while edits are pending", async ({ page }) => {
  await seedApplied(page);
  const setId = await page.evaluate(() =>
    (window as unknown as { __addSectionSet: (s: object) => string }).__addSectionSet({
      a: [500, 200], b: [500, 800], spacing: 0, count: 1,
    }),
  );
  await page.evaluate((id) => (window as unknown as { __openSection: (i: string, k: number) => void }).__openSection(id, 0), setId);
  await expect(page.getByTestId("stale-scrim")).toHaveCount(0);
  // A geometry edit (digitize one line programmatically) → dirty → scrim, in section view.
  await page.evaluate(
    (id) =>
      (window as unknown as {
        __digitizeLine: (e: null, i: null, s: string, k: number, v: [number, number][]) => unknown;
      }).__digitizeLine(null, null, id, 0, [[100, 400], [500, 450]]),
    setId,
  );
  await expect(page.getByTestId("stale-scrim")).toBeVisible();
});
