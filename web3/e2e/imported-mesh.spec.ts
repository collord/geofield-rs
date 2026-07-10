// Feature 004 (imported mesh surfaces) — US1 behavior gate (Constitution I). Importing an .obj via
// the DISTINCT "Import mesh" control (not the project-JSON "Import") adds an imported-surface element
// that fits + meshes: the model reaches "done" and renders. (Assert on state/affordances, not pixels.)

import { test, expect } from "@playwright/test";
import { fileURLToPath } from "node:url";

const FIXTURE = fileURLToPath(new URL("./fixtures/mesh-span-domain.obj", import.meta.url));

test("importing an .obj adds a mesh surface that fits and renders", async ({ page }) => {
  await page.goto("/");

  // The mesh-import control is distinct from the project-JSON Import (FR-013).
  const meshInput = page.getByTestId("import-mesh-input");
  await expect(meshInput).toBeAttached();

  // Import the span-domain bed. A geometry edit → Apply re-fits.
  await meshInput.setInputFiles(FIXTURE);
  await expect(page.getByTestId("apply")).toBeVisible();
  await page.getByTestId("apply").click();

  // The engine parses the .obj, fits a field through it, and meshes → status reaches done.
  await expect(page.getByText("status: done")).toBeVisible({ timeout: 30_000 });
  await expect(page.getByTestId("stale-scrim")).toHaveCount(0, { timeout: 30_000 });

  // The imported surface appears as an element in the hierarchy (named from the file).
  await expect(page.getByText("mesh-span-domain")).toBeVisible();

  // US3 role parity: the imported surface exposes a role selector; switching it to erosion is a
  // geometry edit that re-fits and re-meshes (the clip changes) — status returns to done.
  const role = page.getByTestId("import-role");
  await expect(role).toBeVisible();
  await role.selectOption("erosion");
  await expect(page.getByTestId("apply")).toBeVisible();
  await page.getByTestId("apply").click();
  await expect(page.getByText("status: done")).toBeVisible({ timeout: 30_000 });
  await expect(page.getByTestId("stale-scrim")).toHaveCount(0, { timeout: 30_000 });

  // US2 extend-to-bounds: off by default; turning it on warns (a confirm dialog) then projects the
  // surface to the domain box — a geometry edit that re-fits.
  const extend = page.getByTestId("import-extend").locator("input[type=checkbox]");
  await expect(extend).not.toBeChecked();
  page.once("dialog", (d) => d.accept()); // accept the extrapolation warning
  await extend.check();
  await expect(extend).toBeChecked();
  await page.getByTestId("apply").click();
  await expect(page.getByText("status: done")).toBeVisible({ timeout: 30_000 });
  await expect(page.getByTestId("stale-scrim")).toHaveCount(0, { timeout: 30_000 });
});
