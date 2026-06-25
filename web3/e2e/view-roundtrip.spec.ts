// Export captures the VIEW (camera + render toggles) so a viewpoint round-trips and an exact
// view can be shared for repro; Import/load re-applies it.
import { test, expect } from "@playwright/test";

test("Export carries camera + render mode; load restores them", async ({ page }) => {
  await page.goto("/");
  await page.getByTestId("seed-template").selectOption("cross-cutting-vein-dipping");
  await expect(page.getByText("status: done")).toBeVisible({ timeout: 30_000 });
  // Go to Solids.
  await page.getByTestId("render-toggle").click();
  await expect(page.getByText("status: done")).toBeVisible({ timeout: 30_000 });
  // Orbit a bit so the camera is off its default.
  const canvas = page.locator("main canvas");
  const box = (await canvas.boundingBox())!;
  await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
  await page.mouse.down();
  await page.mouse.move(box.x + box.width / 2 + 120, box.y + box.height / 2 - 80, { steps: 8 });
  await page.mouse.up();
  await page.waitForTimeout(2000); // let OrbitControls damping settle (residual would skew setView)

  // Export → intercept the download → parse.
  const [dl] = await Promise.all([
    page.waitForEvent("download"),
    page.getByRole("button", { name: "Export", exact: true }).click(),
  ]);
  const fs = await import("node:fs/promises");
  const proj = JSON.parse(await fs.readFile(await dl.path(), "utf8"));

  expect(proj.view, "view block present").toBeTruthy();
  expect(proj.view.renderMode, "render mode captured").toBe("solid");
  expect(proj.view.seamClip).toBe(true);
  expect(proj.view.edges).toBe(false);
  for (const k of ["camera", "target", "up"]) {
    expect(Array.isArray(proj.view[k]) && proj.view[k].length === 3, `${k} is xyz`).toBe(true);
  }
  expect(typeof proj.view.fov).toBe("number");

  // Round-trip: pin a known top-down camera, reload via the test hook, confirm mode restores.
  proj.view.camera = [500, 500, 3000];
  proj.view.target = [500, 500, 500];
  proj.view.up = [0, 1, 0];
  await page.evaluate((p) => (window as unknown as { __loadProject: (p: unknown) => void }).__loadProject(p), proj);
  await expect(page.getByText("status: done")).toBeVisible({ timeout: 30_000 });
  await page.waitForTimeout(800); // settle
  // Solid mode restored (toggle label flips to "Solids ⇄ Surfaces" only in solid mode).
  await expect(page.getByTestId("render-toggle")).toHaveText("Solids ⇄ Surfaces");
  // ...and the camera was applied (the pinned top-down position).
  const cam = await page.evaluate(() => (window as unknown as { __getView: () => { camera: number[]; target: number[] } }).__getView());
  expect(cam.camera.map((n) => Math.round(n))).toEqual([500, 500, 3000]);
  expect(cam.target.map((n) => Math.round(n))).toEqual([500, 500, 500]);
});
