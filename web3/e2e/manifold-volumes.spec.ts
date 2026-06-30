// Manifold boolean volume mesher A/B (docs/13 §5 SPIKE). On the cross-cutting-vein-dipping scene
// in SOLID mode, toggling `manifold` ON must mesh the vein's rock units with the Manifold engine —
// the wall is shared by construction, so the rock units are WATERTIGHT (no wall∩bed peek-a-boo)
// where the field path leaves a gap. This guards the browser/wasm-in-worker integration (the
// node harness scratch-verify/verify-manifold.mjs proves the geometry independently).
import { test, expect } from "@playwright/test";

type Surf = { series?: number; unit?: number; positions: Float32Array; indices: Uint32Array };

async function waitForSolids(page: import("@playwright/test").Page) {
  await expect
    .poll(
      () =>
        page.evaluate(() => {
          const s = (window as { __surfaces?: { unit?: number }[] }).__surfaces ?? [];
          return s.length > 0 && s.every((x) => x.unit !== undefined && x.unit !== null);
        }),
      { timeout: 30_000 },
    )
    .toBe(true);
}

// Engine-independent watertight check on a triangle soup: weld coincident verts, every undirected
// edge must have incidence exactly 2 (no open boundary, no non-manifold edge).
function watertightCheck() {
  return (window as { __surfaces?: Surf[] }).__surfaces!.map((p) => {
    // Manifold's getMesh (the worker payload's source) is ALREADY vertex-merged, so check
    // incidence on the raw index buffer — re-welding by quantized position would INVENT false
    // coincidences (over-merging distinct verts) and report phantom open edges.
    const inc = new Map<string, number>();
    const bump = (a: number, b: number) => {
      const e = a < b ? `${a}-${b}` : `${b}-${a}`;
      inc.set(e, (inc.get(e) ?? 0) + 1);
    };
    for (let t = 0; t + 2 < p.indices.length; t += 3) {
      const a = p.indices[t], b = p.indices[t + 1], c = p.indices[t + 2];
      bump(a, b); bump(b, c); bump(c, a);
    }
    let open = 0, nonManifold = 0;
    for (const c of inc.values()) {
      if (c === 1) open++;
      else if (c > 2) nonManifold++;
    }
    return { series: p.series, unit: p.unit, tris: p.indices.length / 3, open, nonManifold };
  });
}

test("manifold-toggle seals the dipping cross-cut vein's rock units (watertight, no peek-a-boo)", async ({ page }) => {
  await page.goto("/");
  await page.getByTestId("seed-template").selectOption("cross-cutting-vein-dipping");
  await expect(page.getByText("status: done")).toBeVisible({ timeout: 30_000 });

  // Switch to Solids. The unified Manifold recipe mesher is the DEFAULT volume mesher, so the very
  // first solid render already meshes through it (status passes through the "(Manifold)" phase) —
  // no toggle needed. Assert the toggle reflects that default-on state.
  await page.getByTestId("render-toggle").click();
  await expect(page.getByText(/Manifold/)).toBeVisible({ timeout: 30_000 });
  await expect(page.getByText("status: done")).toBeVisible({ timeout: 30_000 });
  await expect(page.getByTestId("manifold-toggle")).toBeChecked();
  await waitForSolids(page);
  // The Manifold band is distinctly smaller than the field band; poll until the Manifold output is
  // actually in __surfaces (the band has no per-vertex normals — its normals array is empty).
  await expect
    .poll(
      () =>
        page.evaluate(() => {
          const s = (window as { __surfaces?: { series?: number; normals?: Float32Array }[] }).__surfaces ?? [];
          const band = s.find((x) => x.series === 0);
          return !!band && (band.normals?.length ?? 0) === 0;
        }),
      { timeout: 30_000 },
    )
    .toBe(true);

  const report = await page.evaluate(watertightCheck);
  // Every emitted solid (band + rock units) is watertight: incidence-2 everywhere.
  expect(report.length, "solids present").toBeGreaterThan(1);
  for (const r of report) {
    expect(r.tris, `series ${r.series} unit ${r.unit} non-empty`).toBeGreaterThan(0);
    expect(r.open, `series ${r.series} unit ${r.unit} open edges`).toBe(0);
    expect(r.nonManifold, `series ${r.series} unit ${r.unit} non-manifold edges`).toBe(0);
  }

  // No CONNECTED sliver FLAPS at the domain border (docs/14). The earlier "tiny *components*" check
  // was INSUFFICIENT: the flaps are thin slivers ATTACHED to the main solid, so a component count
  // can't see them (it passed while the flaps remained). Detect them geometrically instead: a flap
  // is a thin needle triangle (min-altitude h = 2·area / longest-edge → 0) lying against a domain
  // box face. The overhang-the-primitives + one-clean-domain-clip fix makes every boundary face a
  // transversal cut, so there must be NO collapsed slivers (h < 1e-3). (A few FLOAT-scale
  // micro-triangles at vein∩wall∩bed triple corners survive as h∈[1e-3,1e-2] "needles" — the
  // boolean engine's documented triple-point limit; they are sub-visual and, since every solid is
  // watertight (asserted above), not see-through. The gate is collapsed slivers, not those.)
  const flaps = await page.evaluate(() => {
    const surfs = (window as { __surfaces?: { series?: number; unit?: number; positions: Float32Array; indices: Uint32Array }[] }).__surfaces ?? [];
    // Domain box = tight bbox over ALL solids (they partition it), so the extent needn't be hardcoded.
    const lo = [Infinity, Infinity, Infinity], hi = [-Infinity, -Infinity, -Infinity];
    for (const p of surfs) for (let i = 0; i < p.positions.length; i += 3)
      for (let d = 0; d < 3; d++) { lo[d] = Math.min(lo[d], p.positions[i + d]); hi[d] = Math.max(hi[d], p.positions[i + d]); }
    const eps = 0.5; // a vert "on" a box face within 0.5 unit (domain ~1000)
    // distance of a point to each of the 6 box faces, in fixed order.
    const faceDists = (pt: number[]) => [
      Math.abs(pt[0] - lo[0]), Math.abs(pt[0] - hi[0]), Math.abs(pt[1] - lo[1]),
      Math.abs(pt[1] - hi[1]), Math.abs(pt[2] - lo[2]), Math.abs(pt[2] - hi[2]),
    ];
    const out: { series?: number; unit?: number; slivers: number; needles: number; minH: number; at: number[] }[] = [];
    for (const p of surfs) {
      const P = (i: number) => [p.positions[3 * i], p.positions[3 * i + 1], p.positions[3 * i + 2]];
      let slivers = 0, needles = 0, minH = Infinity, at = [-1, -1, -1];
      for (let t = 0; t + 2 < p.indices.length; t += 3) {
        const a = P(p.indices[t]), b = P(p.indices[t + 1]), c = P(p.indices[t + 2]);
        // ≥2 verts within eps of the SAME box face → a boundary triangle (a triangle with verts on
        // two DIFFERENT faces is a legitimate edge-spanning triangle, not a border flap).
        const da = faceDists(a), db = faceDists(b), dc = faceDists(c);
        let onSameFace = false;
        for (let f = 0; f < 6; f++) {
          if ((da[f] < eps ? 1 : 0) + (db[f] < eps ? 1 : 0) + (dc[f] < eps ? 1 : 0) >= 2) { onSameFace = true; break; }
        }
        if (!onSameFace) continue;
        const ab = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
        const ac = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
        const bc = [c[0] - b[0], c[1] - b[1], c[2] - b[2]];
        const cr = [ab[1] * ac[2] - ab[2] * ac[1], ab[2] * ac[0] - ab[0] * ac[2], ab[0] * ac[1] - ab[1] * ac[0]];
        const area = 0.5 * Math.hypot(cr[0], cr[1], cr[2]);
        const L = Math.max(Math.hypot(...ab), Math.hypot(...bc), Math.hypot(...ac));
        const h = L > 0 ? (2 * area) / L : 0;
        if (h < 1e-3) slivers++;
        if (h < 1e-2) { needles++; if (h < minH) { minH = h; at = [(a[0] + b[0] + c[0]) / 3, (a[1] + b[1] + c[1]) / 3, (a[2] + b[2] + c[2]) / 3]; } }
      }
      out.push({ series: p.series, unit: p.unit, slivers, needles, minH: Number.isFinite(minH) ? minH : 1, at: at.map((v) => Math.round(v * 10) / 10) });
    }
    return out;
  });
  for (const f of flaps) {
    expect(f.slivers, `series ${f.series} unit ${f.unit} collapsed boundary sliver flaps (needles=${f.needles}, minH=${f.minH}, at=${JSON.stringify(f.at)})`).toBe(0);
  }
});
