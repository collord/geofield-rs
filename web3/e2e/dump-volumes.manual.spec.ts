// Diagnostic: seed the imported-mesh erosion-over-stack template, toggle Solids, and dump each
// rendered Manifold volume solid as a SEPARATE .obj (keyed series-unit) plus a manifest, so a
// native Rust test can read the ACTUAL booleaned output back and check the volumes partition space
// (each grid point inside exactly one volume). Not a gate — a throwaway data-export harness.
import { test, expect } from "@playwright/test";
import { mkdirSync, writeFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

// Which template + output dir. Defaults to the all-imported template; override via env for the
// mixed one: DUMP_TEMPLATE=conformal-stack-mixed DUMP_DIR=imported_volumes_mixed.
const TEMPLATE = process.env.DUMP_TEMPLATE ?? "erosion-over-stack-imported";
const DIR = process.env.DUMP_DIR ?? "imported_volumes";
const OUT = fileURLToPath(new URL(`../../crates/geofield/tests/fixtures/${DIR}/`, import.meta.url));

test("dump erosion-over-stack volumes", async ({ page }) => {
  test.setTimeout(300_000);
  await page.goto("/");
  await page.getByTestId("seed-template").selectOption(TEMPLATE);
  await expect(page.getByText("status: done")).toBeVisible({ timeout: 150_000 });
  await expect(page.getByTestId("stale-scrim")).toHaveCount(0, { timeout: 150_000 });

  await page.getByTestId("render-toggle").click();
  await page
    .waitForFunction(() => (window as { __statusLog?: string[] }).__statusLog?.slice(-1)[0] !== "done", { timeout: 10_000 })
    .catch(() => {});
  await expect(page.getByText("status: done")).toBeVisible({ timeout: 150_000 });
  await expect(page.getByTestId("stale-scrim")).toHaveCount(0, { timeout: 150_000 });
  await expect
    .poll(() => page.evaluate(() => {
      const s = (window as { __surfaces?: { unit?: number }[] }).__surfaces ?? [];
      return s.length > 0 && s.every((x) => x.unit !== undefined && x.unit !== null);
    }), { timeout: 150_000 })
    .toBe(true);

  // Pull each volume's positions/indices out of the render hook.
  const vols = await page.evaluate(() => {
    const s = (window as {
      __surfaces?: { series: number; unit?: number; indices: Uint32Array; positions: Float32Array; lithologyId?: string }[];
    }).__surfaces ?? [];
    return s.map((x) => ({
      series: x.series,
      unit: x.unit ?? 0,
      lithologyId: x.lithologyId ?? null,
      positions: Array.from(x.positions),
      indices: Array.from(x.indices),
    }));
  });

  mkdirSync(OUT, { recursive: true });
  const manifest: { file: string; series: number; unit: number; lithologyId: string | null; tris: number }[] = [];
  for (const v of vols) {
    const file = `vol_s${v.series}_u${v.unit}.obj`;
    const lines = [`# volume series=${v.series} unit=${v.unit} lithology=${v.lithologyId}`];
    for (let i = 0; i < v.positions.length; i += 3) {
      lines.push(`v ${v.positions[i]} ${v.positions[i + 1]} ${v.positions[i + 2]}`);
    }
    for (let i = 0; i < v.indices.length; i += 3) {
      lines.push(`f ${v.indices[i] + 1} ${v.indices[i + 1] + 1} ${v.indices[i + 2] + 1}`);
    }
    writeFileSync(OUT + file, lines.join("\n") + "\n");
    manifest.push({ file, series: v.series, unit: v.unit, lithologyId: v.lithologyId, tris: v.indices.length / 3 });
  }
  writeFileSync(OUT + "manifest.json", JSON.stringify(manifest, null, 2));
  console.log(`DUMPED ${vols.length} volumes to ${OUT}`);
});
