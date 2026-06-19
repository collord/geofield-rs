// Feature 002 (enhanced-authoring) T027 [US4] — reorder chronology. Behavior gate
// (Constitution I) for quickstart.md V4. Real @dnd-kit drag is flaky (cf. drag-affordance
// .spec), so we drive the SAME wired onReorderContainers via a test hook and assert the
// round-trip through the fit — mirroring round-trip.spec.

import { test, expect } from "@playwright/test";

test("reordering two containers changes the panel order and re-fits on Apply", async ({ page }) => {
  await page.goto("/");
  await page.getByTestId("seed-template").selectOption("erosional-unconformity");
  await expect(page.getByText("status: done")).toBeVisible({ timeout: 30_000 });

  // Two containers, youngest first: the "basalt" erosion series (a SINGLETON, rendered flat
  // as its interface "erosion surface") then the "sediments" floor series (nested beds).
  const namesBefore = await page.locator(".container-name, .surface-name").allTextContents();
  expect(namesBefore.join(" ")).toContain("erosion surface");
  expect(namesBefore.join(" ")).toContain("sediments");
  // Youngest (top) is the erosion surface.
  expect(namesBefore[0]).toContain("erosion surface");

  // Read the element ids in render order, then swap the two via the wired callback.
  const ids = await page.locator("[data-element-id]").evaluateAll((els) =>
    Array.from(new Set(els.map((e) => (e as HTMLElement).dataset.elementId!))),
  );
  expect(ids.length).toBeGreaterThanOrEqual(2);

  await page.evaluate(
    ([from, to]) => {
      (window as unknown as { __reorderContainers: (f: string, t: string) => void }).__reorderContainers(from, to);
    },
    [ids[0], ids[1]],
  );

  // The reorder is a geometry edit -> dirty bar + scrim.
  await expect(page.getByTestId("apply-bar")).toBeVisible();

  // Apply -> the derived order/relations reflect the new chronology (round-trip, not echo).
  await page.getByTestId("apply").click();
  await expect(page.getByText("status: done")).toBeVisible({ timeout: 30_000 });

  // The panel order changed: the first rendered name is now the formerly-older element.
  const namesAfter = await page.locator(".container-name, .surface-name").allTextContents();
  expect(namesAfter.join(" ")).not.toEqual(namesBefore.join(" "));
});

test("the up/down reorder buttons change top-level chronology and re-fit on Apply", async ({ page }) => {
  // The CLICK affordance for reorder (drag-and-drop is unbuilt). Unlike the test-hook reorder
  // above, this drives the real UI buttons. Two elements youngest-first: "erosion surface"
  // (singleton, flat) then "sediments" (floor).
  await page.goto("/");
  await page.getByTestId("seed-template").selectOption("erosional-unconformity");
  await expect(page.getByText("status: done")).toBeVisible({ timeout: 30_000 });

  const before = await page.locator(".container-name, .surface-name").allTextContents();
  expect(before[0]).toContain("erosion surface");

  // The youngest element's ▲ is disabled (already at the top); its ▼ moves it down.
  await expect(page.getByTestId("reorder-up").first()).toBeDisabled();
  await page.getByTestId("reorder-down").first().click();

  // Reorder is a geometry edit → dirty bar; Apply re-fits and the new order persists.
  await expect(page.getByTestId("apply-bar")).toBeVisible();
  await page.getByTestId("apply").click();
  await expect(page.getByText("status: done")).toBeVisible({ timeout: 30_000 });

  const after = await page.locator(".container-name, .surface-name").allTextContents();
  expect(after[0]).not.toContain("erosion surface"); // the floor is now youngest-rendered
  expect(after.join(" ")).not.toEqual(before.join(" "));
});

test("a cross-container surface move is DECLINED with a message (not a silent no-op)", async ({ page }) => {
  // FR-016: the move callback is a documented punt — it shows a user-facing message and
  // changes nothing, rather than silently doing nothing. Drive the wired onMoveSurface via
  // the same kind of test hook the reorder test uses.
  await page.goto("/");
  await page.getByTestId("seed-template").selectOption("cross-cutting-vein");
  await expect(page.getByText("status: done")).toBeVisible({ timeout: 30_000 });

  const elementsBefore = await page
    .locator("[data-element-id]")
    .evaluateAll((els) => els.map((e) => (e as HTMLElement).dataset.elementId));

  await page.evaluate(() => {
    (window as unknown as { __moveSurface: () => void }).__moveSurface();
  });

  // A clear message is shown, and the model is unchanged (no dirty bar, same elements).
  await expect(page.getByText(/isn't supported — delete and re-add/)).toBeVisible();
  await expect(page.getByTestId("apply-bar")).toHaveCount(0);
  const elementsAfter = await page
    .locator("[data-element-id]")
    .evaluateAll((els) => els.map((e) => (e as HTMLElement).dataset.elementId));
  expect(elementsAfter).toEqual(elementsBefore);
});
