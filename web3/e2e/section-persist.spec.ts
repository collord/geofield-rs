// Feature 005 (cross-sections) US6 behavior gate (SC-005): an exported project restores
// its sections, serial parameters, digitized vertices (bit-exact), and the ACTIVE slice;
// a pre-005 file (no new fields) still loads; the 3D plane-quad toggle is view-state.

import { test, expect, type Page } from "@playwright/test";
import type { Project } from "../src/HierarchyPanel/project";

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
  __exportState: () => Project;
  __importProjectJson: (t: string) => Promise<Project>;
  __loadProject: (p: Project) => void;
  __sections: () => { id: string; count: number; spacing: number; showInScene?: boolean }[];
  __activeSection: () => { setId: string; index: number } | null;
  __project: () => Project;
};

async function openApp(page: Page) {
  await page.goto("/");
  await expect(page.getByTestId("section-new")).toBeVisible({ timeout: 30_000 });
}

test("export → import round-trips sections, digitized vertices (bit-exact), and the active slice", async ({ page }) => {
  await openApp(page);
  const setId = await page.evaluate(() =>
    (window as unknown as Hooks).__addSectionSet({
      a: [500, 200], b: [500, 800], spacing: 250, count: 3, offset: -250, name: "Fence A",
    }),
  );
  await page.evaluate(
    (id) => (window as unknown as Hooks).__digitizeLine(null, null, id, 1, [[100, 400], [300, 430], [500, 465.125]]),
    setId,
  );
  await page.evaluate((id) => (window as unknown as Hooks).__openSection(id, 2), setId);
  await expect(page.getByTestId("section-label")).toContainText("3/3");

  // Capture the exact export object (the same thing Export downloads), then wipe to a
  // fresh project and re-import it through the real import path (incl. sanitize guards).
  const exported = await page.evaluate(() => JSON.stringify((window as unknown as Hooks).__exportState()));
  const vertsBefore = JSON.stringify(
    (JSON.parse(exported) as Project).elements.flatMap((e) =>
      e.kind === "series" ? e.interfaces.flatMap((i) => i.digitized ?? []) : [],
    ),
  );
  await page.evaluate(async (json) => {
    const h = window as unknown as Hooks;
    h.__loadProject(await h.__importProjectJson(json));
  }, exported);

  // Sections + serial params + active slice restored (FR-010 / SC-005).
  const sections = await page.evaluate(() => (window as unknown as Hooks).__sections());
  expect(sections).toHaveLength(1);
  expect(sections[0].count).toBe(3);
  expect(sections[0].spacing).toBe(250);
  const active = await page.evaluate(() => (window as unknown as Hooks).__activeSection());
  expect(active).toEqual({ setId: sections[0].id, index: 2 });
  await expect(page.getByTestId("section-label")).toContainText("3/3");

  // Digitized vertices round-trip bit-exactly (JSON number fidelity).
  const vertsAfter = await page.evaluate(() =>
    JSON.stringify(
      (window as unknown as Hooks).__project().elements.flatMap((e) =>
        e.kind === "series" ? e.interfaces.flatMap((i) => i.digitized ?? []) : [],
      ),
    ),
  );
  expect(vertsAfter).toBe(vertsBefore);
});

test("a pre-005 project file (no sections/digitized fields) still imports cleanly", async ({ page }) => {
  await openApp(page);
  const pre005 = JSON.stringify({
    version: 1,
    name: "legacy",
    extent: [0, 1000, 0, 1000, 0, 1000],
    elements: [
      {
        kind: "series", id: "el-legacy", name: "Bed", relation: "conformable", unboundedAbove: false,
        interfaces: [{
          id: "if-legacy", name: "Bed",
          points: [{ x: 200, y: 500, z: 500 }, { x: 800, y: 500, z: 500 }],
          orientations: [{ x: 500, y: 500, z: 500, dip: 0, dipDir: 0 }],
        }],
      },
    ],
  });
  await page.evaluate(async (json) => {
    const h = window as unknown as Hooks;
    h.__loadProject(await h.__importProjectJson(json));
  }, pre005);
  await expect(page.getByText("status: done")).toBeVisible({ timeout: 60_000 });
  expect(await page.evaluate(() => (window as unknown as Hooks).__sections())).toHaveLength(0);
  expect(await page.evaluate(() => (window as unknown as Hooks).__activeSection())).toBeNull();
});

test("the 3D plane-quad toggle is per-set view state (no dirty)", async ({ page }) => {
  await openApp(page);
  const setId = await page.evaluate(() =>
    (window as unknown as Hooks).__addSectionSet({ a: [200, 200], b: [800, 800], spacing: 0, count: 1 }),
  );
  const toggle = page.getByTestId(`section-show-${setId}`);
  await expect(toggle).not.toBeChecked();
  await toggle.check();
  await expect(toggle).toBeChecked();
  await expect(page.getByTestId("apply-bar")).toHaveCount(0); // view-state, never dirty
  const shown = await page.evaluate(() => (window as unknown as Hooks).__sections()[0].showInScene);
  expect(shown).toBe(true);
});
