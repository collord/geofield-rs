// Feature 005 (cross-sections) §11.4 validation gate (SC-007): every enumerated HARD case
// blocks Apply with a stated reason; every SOFT case warns without blocking; the
// "flip this line to match" one-action fix clears the opposed-younging warning.

import { test, expect, type Page } from "@playwright/test";

type Hooks = {
  __addSectionSet: (s: object) => string;
  __digitizeLine: (
    e: string | null,
    i: string | null,
    s: string,
    k: number,
    v: [number, number][],
  ) => { elementId: string; interfaceId: string; lineId: string } | null;
};

async function openBlank(page: Page): Promise<string> {
  await page.goto("/");
  await expect(page.getByTestId("section-new")).toBeVisible({ timeout: 30_000 });
  return page.evaluate(() =>
    (window as unknown as Hooks).__addSectionSet({ a: [500, 200], b: [500, 800], spacing: 250, count: 3, offset: -250 }),
  );
}

test("hard: a 1-vertex line blocks Apply with a stated reason", async ({ page }) => {
  const setId = await openBlank(page);
  await page.evaluate(
    (id) => (window as unknown as Hooks).__digitizeLine(null, null, id, 0, [[100, 400]]),
    setId,
  );
  await expect(page.getByTestId("apply")).toBeDisabled();
  await expect(page.getByTestId("apply-reason")).toContainText("needs at least 2");
});

test("hard: all digitized data on ONE plane (and nothing else) blocks Apply; a second section clears it to a soft warning", async ({ page }) => {
  const setId = await openBlank(page);
  const r = await page.evaluate(
    (id) => (window as unknown as Hooks).__digitizeLine(null, null, id, 0, [[100, 400], [300, 430], [500, 460]]),
    setId,
  );
  if (!r) throw new Error("digitize failed");
  await expect(page.getByTestId("apply")).toBeDisabled();
  await expect(page.getByTestId("apply-reason")).toContainText("ONE section plane");

  // A line on a SECOND parallel section makes it fittable (hard error clears)…
  await page.evaluate(
    ({ id, el, ifid }) =>
      (window as unknown as Hooks).__digitizeLine(el, ifid, id, 1, [[100, 420], [300, 455], [500, 480]]),
    { id: setId, el: r.elementId, ifid: r.interfaceId },
  );
  await expect(page.getByTestId("apply")).toBeEnabled();
});

test("soft: opposed drawn directions warn and 'flip this line to match' fixes it", async ({ page }) => {
  const setId = await openBlank(page);
  const r = await page.evaluate(
    (id) => (window as unknown as Hooks).__digitizeLine(null, null, id, 0, [[100, 400], [500, 460]]),
    setId,
  );
  if (!r) throw new Error("digitize failed");
  // The sibling on slice 1 is drawn BACKWARD (the classic digitizing slip).
  await page.evaluate(
    ({ id, el, ifid }) =>
      (window as unknown as Hooks).__digitizeLine(el, ifid, id, 1, [[500, 490], [100, 430]]),
    { id: setId, el: r.elementId, ifid: r.interfaceId },
  );
  await expect(page.getByTestId("soft-warnings")).toContainText("OPPOSITE way");
  await expect(page.getByTestId("apply")).toBeEnabled(); // warns, never blocks

  // One action fixes it: the line's vertex order reverses, the warning clears.
  await page.getByTestId("flip-line-to-match").click();
  await expect(page.getByTestId("soft-warnings")).not.toContainText("OPPOSITE way");
});

test("soft: a self-crossing line warns (ambiguous direction) without blocking", async ({ page }) => {
  const setId = await openBlank(page);
  const r = await page.evaluate(
    (id) =>
      // A bow-tie: segments (0→1) and (2→3) cross in-plane.
      (window as unknown as Hooks).__digitizeLine(null, null, id, 0, [
        [100, 400], [500, 500], [500, 400], [100, 500],
      ]),
    setId,
  );
  if (!r) throw new Error("digitize failed");
  // Needs a second slice so the one-plane HARD error doesn't mask the soft warning.
  await page.evaluate(
    ({ id, el, ifid }) =>
      (window as unknown as Hooks).__digitizeLine(el, ifid, id, 1, [[100, 420], [500, 480]]),
    { id: setId, el: r.elementId, ifid: r.interfaceId },
  );
  await expect(page.getByTestId("soft-warnings")).toContainText("self-crosses");
  await expect(page.getByTestId("apply")).toBeEnabled();
});
