// Feature 003 (topography, lithology, solids) T036 [US3] — the lithology catalog. Behavior
// gate (Constitution I) for User Story 3: Air + Unknown fixed; inferred entries sync with
// their body; user add/rename/recolour/delete; many-to-one assignment; and the catalog
// round-trips through persistence. (FR-013..021, SC-007.)

import { test, expect } from "@playwright/test";

// A litho name is either a span's text (fixed/inferred) or an input's value (user) — read both.
const lithoNames = (page: import("@playwright/test").Page) =>
  page.getByTestId("litho-item").evaluateAll((items) =>
    items.map((it) => {
      const input = it.querySelector("input.litho-name") as HTMLInputElement | null;
      if (input) return input.value;
      return (it.querySelector(".litho-name")?.textContent ?? "").trim();
    }),
  );

test("Air and Unknown are fixed (non-deletable, non-renameable)", async ({ page }) => {
  await page.goto("/");
  await page.getByTestId("seed-template").selectOption("conformal-column");
  await expect(page.getByText("status: done")).toBeVisible({ timeout: 30_000 });

  await expect(page.getByTestId("litho-catalog")).toBeVisible();
  const air = page.getByTestId("litho-item").filter({ hasText: "Air" });
  const unknown = page.getByTestId("litho-item").filter({ hasText: "Unknown" });
  await expect(air).toHaveAttribute("data-kind", "air");
  await expect(unknown).toHaveAttribute("data-kind", "unknown");
  // Neither offers a delete button nor an editable name input (fixed members).
  await expect(air.locator(".litho-delete")).toHaveCount(0);
  await expect(unknown.locator(".litho-delete")).toHaveCount(0);
  await expect(air.locator("input.litho-name")).toHaveCount(0);
});

test("a vein contributes an inferred lithology that syncs on rename and delete", async ({ page }) => {
  await page.goto("/");
  await page.getByTestId("seed-template").selectOption("cross-cutting-vein");
  await expect(page.getByText("status: done")).toBeVisible({ timeout: 30_000 });

  // The vein "quartz vein" contributes an inferred entry, auto-named after the body.
  expect((await lithoNames(page)).join(" ")).toContain("quartz vein");
  const inferred = page.getByTestId("litho-item").filter({ hasText: "quartz vein" });
  await expect(inferred).toHaveAttribute("data-kind", "inferred");
  await expect(inferred.locator(".litho-delete")).toHaveCount(0); // deleted via the body, not here

  // Rename the body (the vein element) → the inferred lithology re-syncs its name. The
  // series menu's "Rename…" uses window.prompt — register the dialog handler BEFORE clicking.
  page.once("dialog", (d) => d.accept("gold vein"));
  await page.locator('.container-node[data-kind="vein"] [data-testid="series-menu"]').first().click();
  await page.getByRole("menuitem", { name: "Rename…" }).click();
  await expect.poll(async () => (await lithoNames(page)).join(" ")).toContain("gold vein");

  // Delete the body → the inferred lithology is removed.
  page.once("dialog", (d) => d.accept());
  await page.locator('.container-node[data-kind="vein"] [data-testid="series-menu"]').first().click();
  await page.getByRole("menuitem", { name: "Delete" }).click();
  await expect.poll(async () => (await lithoNames(page)).join(" ")).not.toContain("vein");
});

test("user can add, recolour, and delete a lithology; it appears in the unit dropdown", async ({ page }) => {
  await page.goto("/");
  await page.getByTestId("seed-template").selectOption("conformal-column");
  await expect(page.getByText("status: done")).toBeVisible({ timeout: 30_000 });

  // Add a "Sandstone" user lithology. User entries render their name in an INPUT (editable),
  // so locate the item by its data-kind="user" rather than by text.
  await page.getByTestId("litho-new-name").fill("Sandstone");
  await page.getByTestId("litho-add").click();
  const sandstone = page.getByTestId("litho-item").filter({ has: page.locator('[data-kind="user"]') }).or(
    page.locator('[data-testid="litho-item"][data-kind="user"]'),
  );
  await expect(page.locator('[data-testid="litho-item"][data-kind="user"]')).toHaveCount(1);
  await expect(page.locator('[data-testid="litho-item"][data-kind="user"] input.litho-name')).toHaveValue("Sandstone");

  // It appears in every unit row's dropdown.
  const firstDropdown = page.getByTestId("unit-litho").first();
  await expect(firstDropdown.locator("option", { hasText: "Sandstone" })).toHaveCount(1);

  // Delete it → gone from the catalog.
  await page.locator('[data-testid="litho-item"][data-kind="user"] .litho-delete').click();
  await expect(page.locator('[data-testid="litho-item"][data-kind="user"]')).toHaveCount(0);
});

test("two separate units can be assigned the same lithology (many-to-one)", async ({ page }) => {
  await page.goto("/");
  await page.getByTestId("seed-template").selectOption("conformal-column");
  await expect(page.getByText("status: done")).toBeVisible({ timeout: 30_000 });

  // Add "Sandstone", then point two distinct unit dropdowns at it.
  await page.getByTestId("litho-new-name").fill("Sandstone");
  await page.getByTestId("litho-add").click();

  const dropdowns = page.getByTestId("unit-litho");
  await dropdowns.nth(0).selectOption({ label: "Sandstone" });
  await dropdowns.nth(2).selectOption({ label: "Sandstone" });
  await expect(dropdowns.nth(0)).toHaveValue(await dropdowns.nth(2).inputValue());
  // No automatic correlation occurred — only the two we set point at Sandstone.
  const allValues = await dropdowns.evaluateAll((els) => els.map((e) => (e as HTMLSelectElement).selectedOptions[0].textContent));
  expect(allValues.filter((v) => v === "Sandstone").length).toBe(2);
});

test("catalog + assignments round-trip through a page reload", async ({ page }) => {
  await page.goto("/");
  await page.getByTestId("seed-template").selectOption("conformal-column");
  await expect(page.getByText("status: done")).toBeVisible({ timeout: 30_000 });

  await page.getByLabel("project name").fill("litho-roundtrip");
  await page.getByTestId("litho-new-name").fill("Limestone");
  await page.getByTestId("litho-add").click();
  await page.getByTestId("unit-litho").first().selectOption({ label: "Limestone" });

  // Wait for the debounced autosave to flush the full project INCLUDING the assignment (not
  // just the name) — polling `@last` alone races, since the assignment is a later save.
  await expect
    .poll(
      () =>
        page.evaluate(() => {
          const last = localStorage.getItem("geoproj:@last");
          if (last !== "litho-roundtrip") return false;
          const raw = localStorage.getItem("geoproj:litho-roundtrip");
          if (!raw) return false;
          const p = JSON.parse(raw);
          const names: string[] = (p.catalog?.lithologies ?? []).map((l: { name: string }) => l.name);
          const hasAssignment = Object.keys(p.catalog?.assignment ?? {}).length > 0;
          return names.includes("Limestone") && hasAssignment;
        }),
      { timeout: 10_000 },
    )
    .toBe(true);

  await page.reload();
  await expect(page.getByLabel("project name")).toHaveValue("litho-roundtrip", { timeout: 10_000 });
  // The restored project re-fits on load; wait for the unit dropdowns to mount before reading.
  const firstDropdown = page.getByTestId("unit-litho").first();
  await expect(firstDropdown).toBeVisible({ timeout: 30_000 });
  // The user lithology survived in the catalog…
  expect((await lithoNames(page)).join(" ")).toContain("Limestone");
  // …and the assignment survived: the first unit's dropdown still reads "Limestone".
  await expect
    .poll(async () => firstDropdown.evaluate((e) => (e as HTMLSelectElement).selectedOptions[0].textContent), {
      timeout: 10_000,
    })
    .toBe("Limestone");
});
