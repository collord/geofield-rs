// Verify the externalized-mesh round-trip: import → slim export (objText dropped, objPath kept) →
// re-load (hydrates objText by fetching /meshes/...) → the imported surface renders.
import { test, expect } from "@playwright/test";

test("imported mesh exports path-only and re-hydrates on load", async ({ page }) => {
  await page.goto("/");
  await page.waitForFunction(() => typeof (window as any).__loadProject === "function");

  // Import the mesh through the real UI control (reads file.text(), sets objText + objPath).
  await page.getByTestId("import-mesh-input").setInputFiles("public/meshes/mesh-span-domain.obj");
  await page.getByTestId("apply").click();
  await expect(page.getByText("status: done")).toBeVisible({ timeout: 30_000 });
  await page.waitForTimeout(700); // let the ~500ms debounced autosave flush to localStorage

  // Round-trip the pure serialization logic in-page (where fetch('/meshes/...') is live), via the
  // window test hooks the app exposes.
  const result = await page.evaluate(async () => {
    const w = window as any;
    // The live autosaved project (full objText) from localStorage (prefix `geoproj:`).
    const last = localStorage.getItem("geoproj:@last");
    const live = JSON.parse(localStorage.getItem("geoproj:" + last) || "null");
    const imp = live.elements.find((e: any) => e.kind === "importedSurface");

    // EXPORT slim: objText dropped, objPath kept.
    const slim = w.__slimForExport(live);
    const slimImp = slim.elements.find((e: any) => e.kind === "importedSurface");
    const exportedJson = JSON.stringify(slim);

    // RE-LOAD from the slim JSON via the real import path (re-fetches path-only meshes).
    const reloaded = await w.__importProjectJson(exportedJson);
    const reImp = reloaded.elements.find((e: any) => e.kind === "importedSurface");

    return {
      liveTextLen: imp.objText.length,
      liveObjPath: imp.objPath,
      slimTextLen: slimImp.objText.length,
      slimObjPath: slimImp.objPath,
      exportedBytes: exportedJson.length,
      rehydratedTextLen: reImp.objText.length,
      rehydratedMatchesLive: reImp.objText === imp.objText,
    };
  });

  expect(result.liveTextLen, "live project has the full mesh text").toBeGreaterThan(0);
  expect(result.liveObjPath).toBe("meshes/mesh-span-domain.obj");
  expect(result.slimTextLen, "export drops the inline mesh text").toBe(0);
  expect(result.slimObjPath, "export keeps the path reference").toBe("meshes/mesh-span-domain.obj");
  expect(result.rehydratedTextLen, "loader re-fetched the mesh").toBeGreaterThan(0);
  expect(result.rehydratedMatchesLive, "re-hydrated text matches the original").toBe(true);
  console.log("round-trip:", JSON.stringify(result));

  // And a genuinely-missing mesh path fails LOUDLY (not a silent blank).
  const err = await page.evaluate(async () => {
    const w = window as any;
    const bad = { version: 1, name: "x", extent: [0, 1, 0, 1, 0, 1], elements: [{ kind: "importedSurface", id: "e1", name: "missing", role: "contact", relation: "conformable", objText: "", objPath: "meshes/does-not-exist.obj", extendToBounds: false }] };
    try { await w.__importProjectJson(JSON.stringify(bad)); return null; } catch (e: any) { return e.message; }
  });
  expect(err, "a missing mesh path throws a clear error").toContain("does-not-exist.obj");
});
