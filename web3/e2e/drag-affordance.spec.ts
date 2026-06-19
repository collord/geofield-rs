// T060 [US5] — the two drop affordances are visually distinct (FR-015, SC-006). Rather
// than simulate a full drag (flaky), assert the two affordance styles differ on the
// dimensions the design fixes: the container insertion bar is thick + accent + full-width;
// the within-group bar is thin + light + indented. We inject one of each and compare
// computed styles — pinning the *visual contract* the dragResolver's two semantics rely on.

import { test, expect } from "@playwright/test";

test("container vs within-group drop affordances are visually distinct", async ({ page }) => {
  await page.goto("/");
  await page.getByTestId("seed-template").selectOption("conformal-column");
  await expect(page.getByText("status: done")).toBeVisible({ timeout: 30_000 });

  // The hierarchy.css classes are loaded; render one of each affordance and read back the
  // computed styles that the CSS deliberately makes different.
  const styles = await page.evaluate(() => {
    const mk = (cls: string) => {
      const el = document.createElement("div");
      el.className = cls;
      document.body.appendChild(el);
      const cs = getComputedStyle(el);
      const out = { height: cs.height, marginLeft: cs.marginLeft, opacity: cs.opacity };
      el.remove();
      return out;
    };
    return { container: mk("drop-indicator-container"), within: mk("drop-indicator-within") };
  });

  // Container bar is thicker than the within-group bar.
  expect(parseFloat(styles.container.height)).toBeGreaterThan(parseFloat(styles.within.height));
  // Within-group bar is indented (non-zero left margin); container bar is not.
  expect(parseFloat(styles.within.marginLeft)).toBeGreaterThan(parseFloat(styles.container.marginLeft));
  // The affordances are not identical on every axis (the whole point).
  expect(styles.container).not.toEqual(styles.within);
});
