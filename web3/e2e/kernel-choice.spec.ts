// Feature: project-wide interpolation-kernel choice (gempy's cubic / matern_5_2 / exponential).
// Behavior gate (Constitution I): picking a kernel re-fits immediately (no Apply gate), each
// kernel produces a DIFFERENT model (so the choice really reaches the kernel wasm, not just the
// UI), switching back to cubic reproduces the original model exactly (the kernel is the only
// thing that changed), and the choice persists across a reload. The kernel math itself is
// gated against gempy in validation/ (dump_kernel.py --kernel, run_model1_validation.py --kernel).

import { test, expect, type Page } from "@playwright/test";

/** A cheap, order-sensitive digest of every rendered surface's vertex positions. */
function fingerprint(page: Page): Promise<string> {
  return page.evaluate(() => {
    const surfaces = (window as { __surfaces?: { positions: Float32Array }[] }).__surfaces ?? [];
    let acc = 0;
    let n = 0;
    for (const s of surfaces) {
      const p = s.positions;
      for (let i = 0; i < p.length; i++) acc += p[i] * ((i % 13) + 1);
      n += p.length;
    }
    return `${n}:${acc.toFixed(4)}`;
  });
}

/** Wait until the rendered model differs from `before` and the run has finished. */
async function waitForRefit(page: Page, before: string): Promise<string> {
  await expect.poll(() => fingerprint(page), { timeout: 30_000 }).not.toBe(before);
  await expect(page.getByText("status: done")).toBeVisible({ timeout: 30_000 });
  return fingerprint(page);
}

test("choosing a kernel re-fits with that kernel; cubic round-trips exactly", async ({ page }) => {
  await page.goto("/");
  await page.getByTestId("seed-template").selectOption("erosional-unconformity");
  await expect(page.getByText("status: done")).toBeVisible({ timeout: 30_000 });

  const select = page.getByTestId("kernel-select");
  await expect(select).toHaveValue("cubic"); // absent project.kernel ⇒ the cubic default
  const cubic = await fingerprint(page);
  expect(cubic.startsWith("0:")).toBe(false); // something was actually meshed

  // A kernel switch is a declared model choice: it re-fits immediately, never via Apply.
  await select.selectOption("matern_5_2");
  await expect(page.getByTestId("apply-bar")).toHaveCount(0);
  const matern = await waitForRefit(page, cubic);

  await select.selectOption("exponential");
  const gaussian = await waitForRefit(page, matern);
  expect(gaussian).not.toBe(cubic);

  // Back to cubic: the fit is deterministic, so the original model returns bit-for-bit.
  await select.selectOption("cubic");
  await waitForRefit(page, gaussian);
  expect(await fingerprint(page)).toBe(cubic);
});

test("the kernel choice persists across a reload", async ({ page }) => {
  await page.goto("/");
  await page.getByTestId("seed-template").selectOption("conformal-column");
  await expect(page.getByText("status: done")).toBeVisible({ timeout: 30_000 });

  await page.getByTestId("kernel-select").selectOption("matern_5_2");
  // Wait for the autosave to flush the new kernel (debounced write; poll the real condition).
  await expect
    .poll(
      () =>
        page.evaluate(() => {
          const last = localStorage.getItem("geoproj:@last");
          const raw = last ? localStorage.getItem("geoproj:" + last) : null;
          return raw ? (JSON.parse(raw) as { kernel?: string }).kernel : undefined;
        }),
      { timeout: 10_000 },
    )
    .toBe("matern_5_2");

  await page.reload();
  await expect(page.getByTestId("kernel-select")).toHaveValue("matern_5_2", { timeout: 10_000 });
});
