// Feature 002 (enhanced-authoring) T009 — the three former canned scenarios, re-exposed as
// SEED TEMPLATES that each return a `Project` (not the auto-loaded dropdown of SeriesJson[]).
// Plus a "blank" template. These seed the authoring document; the user then edits/adds/
// reorders and re-fits via Apply. (Feature 001's scenarios.ts built SeriesJson[] directly;
// the authoring layer compiles a Project down to that shape via project.ts::toSeriesJson.)
//
// The dipping-plane / multi-bed-floor geology is preserved so a seeded template fits to a
// meaningful picture — the round-trip anchor in quickstart.md.

import type { InterfaceData, OrientationRow, Project, ProjectElement } from "./HierarchyPanel/project";
import { newId } from "./HierarchyPanel/project";

export interface Template {
  id: string;
  name: string;
  description: string;
  build: () => Project;
}

const EXTENT: [number, number, number, number, number, number] = [0, 1000, 0, 1000, 0, 1000];

/** A 4×4 grid of contact points on a dipping plane z = z0 + sx·(x−500) + sy·(y−500), with
 *  one matching orientation at the center. dip/dipDir are derived from the plane's up-pole
 *  (−sx,−sy,1): flat (sx=sy=0) ⇒ dip 0, pole straight up.
 *
 *  `flipped` negates that pole at compile time — i.e. the unit sits ABOVE its contact rather
 *  than below (an erosional deposit caps the unconformity from above). This is the authoring
 *  equivalent of feature-001's `depositSide:"above"`; without it an erosion surface fits with
 *  an inverted field, which the octree mesher renders with a hole/tear (the user can also fix
 *  any surface via the per-surface "Flip orientation" action). */
function dippingInterface(
  name: string,
  z0: number,
  sx: number,
  sy: number,
  flipped = false,
): InterfaceData {
  // A plane `z = z0 + sx·(x−500) + sy·(y−500)` — TWO points + ONE orientation. Two points, not
  // one: a single surface point leaves the dual-kriging system ill-conditioned (the field's
  // SCALE is undetermined — |grad| blows up to ~2e10 while keeping the right direction, and the
  // octree then can't extract a coherent isosurface, collapsing the sheet to a vertical sliver).
  // A second point pins the scale (|grad| → ~1e-3, healthy). The orientation is kept so `flipped`
  // still controls the field's "up" sense (it negates the pole at compile time, project.ts).
  // (Diagnosed 2026-06-20: 1pt → |grad| 2e10; 2pt → 3e-3.)
  const z = (x: number, y: number) => z0 + sx * (x - 500) + sy * (y - 500);
  const n = Math.hypot(sx, sy, 1);
  const dip = (Math.acos(1 / n) * 180) / Math.PI;
  let dipDir = (Math.atan2(-sx / n, -sy / n) * 180) / Math.PI;
  if (dipDir < 0) dipDir += 360;
  return {
    id: newId("iface"),
    name,
    points: [
      { x: 200, y: 500, z: z(200, 500) },
      { x: 800, y: 500, z: z(800, 500) },
    ],
    orientations: [{ x: 500, y: 500, z: z0, dip, dipDir }],
    flipped,
  };
}

/** A VERTICAL planar wall at constant x = `x0` (the plane `x = x0`), for a vein's
 *  foot-wall/hanging-wall pair so the tabular body is vertical, not flat-lying. TWO points
 *  (both on the plane, at different z) + ONE orientation (pole `(1,0,0)`, dip 90°, dipDir 90°
 *  = facing +X). Two points, not one: a single point leaves the kriging ill-conditioned
 *  (|grad| ~2e10, the sheet collapses) — see `dippingInterface`. */
function verticalWall(name: string, x0: number): InterfaceData {
  return {
    id: newId("iface"),
    name,
    points: [
      { x: x0, y: 500, z: 200 },
      { x: x0, y: 500, z: 800 },
    ],
    orientations: [{ x: x0, y: 500, z: 500, dip: 90, dipDir: 90 }],
  };
}

/** A DIPPING planar wall: the plane through `(x0, *, 500)` tilted by `dxdz` (Δx per Δz), so the
 *  vein walls are oblique to the octree grid instead of axis-aligned. Two points at different z
 *  (so the wall's x shifts with depth) + one orientation whose pole tilts with the dip. This is
 *  the diagnostic variant: an oblique cut-wall is the case a grid-aligned vertical wall can't
 *  exercise, and the one most likely to expose a sub-cell seam between the vein body and the
 *  cross-cut rock (the suspected see-through channel). */
function dippingWall(name: string, x0: number, dxdz: number): InterfaceData {
  // x(z) = x0 + dxdz·(z − 500). Pole faces +X tilted by the dip: (1, 0, −dxdz) normalized.
  const x = (z: number) => x0 + dxdz * (z - 500);
  const n = Math.hypot(1, 0, dxdz);
  const dip = (Math.acos(1 / n) * 180) / Math.PI; // 90° when vertical (dxdz→∞ would be flat)
  return {
    id: newId("iface"),
    name,
    points: [
      { x: x(200), y: 500, z: 200 },
      { x: x(800), y: 500, z: 800 },
    ],
    // dipDir 90° = facing +X; the dip angle off-vertical is governed by the tilt.
    orientations: [{ x: x0, y: 500, z: 500, dip: 90 - dip, dipDir: 90 }],
  };
}

/** The 3-contact floor (z = 300/500/700) as one conformable, unbounded-above series. */
function threeContactFloor(): ProjectElement {
  return {
    kind: "series",
    id: newId("el"),
    name: "sediments",
    relation: "conformable",
    unboundedAbove: true,
    interfaces: [
      dippingInterface("bed 1", 300, 0, 0),
      dippingInterface("bed 2", 500, 0, 0),
      dippingInterface("bed 3", 700, 0, 0),
    ],
  };
}

/** An interface from explicit XYZ points + one upward orientation at the centroid (a conformable
 *  bed fits field-increasing-upward). Used for a CURVED bed (>2 points, non-planar). */
function bedFromPoints(name: string, pts: { x: number; y: number; z: number }[]): InterfaceData {
  const c = pts.reduce((a, p) => ({ x: a.x + p.x, y: a.y + p.y, z: a.z + p.z }), { x: 0, y: 0, z: 0 });
  const n = pts.length;
  return {
    id: newId("iface"),
    name,
    points: pts,
    orientations: [{ x: c.x / n, y: c.y / n, z: c.z / n, dip: 0, dipDir: 0 }], // dip 0 ⇒ pole up
  };
}

/** Like `threeContactFloor` but bed 3 is CURVED (z 620→780 across x,y) — the case a flat AABB box
 *  slab can't represent (its box would overlap the unit below). Exercises the Manifold path's
 *  curved-bed-following host slabs (docs/13 §5 follow-up). */
function curvedContactFloor(): ProjectElement {
  return {
    kind: "series",
    id: newId("el"),
    name: "sediments",
    relation: "conformable",
    unboundedAbove: true,
    interfaces: [
      dippingInterface("bed 1", 300, 0, 0),
      dippingInterface("bed 2", 500, 0, 0),
      bedFromPoints("bed 3 (curved)", [
        { x: 200, y: 500, z: 700 },
        { x: 800, y: 500, z: 680 },
        { x: 200, y: 200, z: 620 },
        { x: 800, y: 800, z: 780 },
      ]),
    ],
  };
}

/** A near-CLOSED spherical envelope for a SOLID intrusion: `count` points sampled on a sphere of
 *  `radius` about `center`, each carrying an OUTWARD orientation (dip/dipDir from the radial pole),
 *  so the co-kriged field's `field < iso` interior is the blob. A single-envelope (solid) intrusion
 *  reads this as its sole bounding surface. A Fibonacci lattice gives an even, deterministic spread. */
function sphereEnvelope(
  name: string,
  center: { x: number; y: number; z: number },
  radius: number,
  count = 24,
): InterfaceData {
  const points: { x: number; y: number; z: number }[] = [];
  const orientations: OrientationRow[] = [];
  const golden = Math.PI * (3 - Math.sqrt(5)); // ~2.399963 rad
  for (let i = 0; i < count; i++) {
    const w = 1 - (2 * i + 1) / count; // z in (−1, 1), centred
    const r = Math.sqrt(Math.max(0, 1 - w * w));
    const theta = golden * i;
    const nx = Math.cos(theta) * r;
    const ny = Math.sin(theta) * r;
    const nz = w;
    const p = { x: center.x + radius * nx, y: center.y + radius * ny, z: center.z + radius * nz };
    points.push(p);
    // Outward radial pole (nx,ny,nz) → dip off vertical, dipDir clockwise from +Y.
    const dip = (Math.acos(Math.max(-1, Math.min(1, nz))) * 180) / Math.PI;
    let dipDir = (Math.atan2(nx, ny) * 180) / Math.PI;
    if (dipDir < 0) dipDir += 360;
    orientations.push({ x: p.x, y: p.y, z: p.z, dip, dipDir });
  }
  return { id: newId("iface"), name, points, orientations };
}

export const TEMPLATES: Template[] = [
  {
    id: "blank",
    name: "Blank project",
    description: "An empty project — add surfaces to build a model.",
    build: () => ({ version: 1, name: "untitled", extent: EXTENT, elements: [] }),
  },
  {
    id: "conformal-column",
    name: "Conformal column",
    description: "A 3-contact floor with no younger unconformity — the beds stack conformably.",
    build: () => ({
      version: 1,
      name: "conformal-column",
      extent: EXTENT,
      elements: [threeContactFloor()],
    }),
  },
  {
    id: "erosional-unconformity",
    name: "Erosional unconformity",
    description:
      "A dipping erosion surface (a younger erode contact) truncates the 3-contact floor below it.",
    build: () => ({
      version: 1,
      name: "erosional-unconformity",
      extent: EXTENT,
      elements: [
        {
          kind: "series",
          id: newId("el"),
          name: "basalt",
          relation: "erode",
          unboundedAbove: false,
          // UNflipped: the `erode` relation already implies deposit-above (it carries the
          // inverted "up" sense vs. a conformable bed — see project.ts flattenInterfaces), so the
          // erosion contact renders correct out of the box with no explicit flip.
          interfaces: [dippingInterface("erosion surface", 650, 0.4, 0)],
        },
        threeContactFloor(),
      ],
    }),
  },
  {
    id: "cross-cutting-vein",
    name: "Cross-cutting vein",
    description:
      "A tabular vein (vertical foot-wall/hanging-wall pair at x≈400/600) cross-cuts the floor it passes through.",
    build: () => ({
      version: 1,
      name: "cross-cutting-vein",
      elements: [
        {
          kind: "vein",
          id: newId("el"),
          name: "quartz vein",
          footWall: verticalWall("foot wall", 400),
          hangingWall: verticalWall("hanging wall", 600),
        },
        threeContactFloor(),
      ],
      extent: EXTENT,
    }),
  },
  {
    id: "combined-geohistory",
    name: "Combined geo-history",
    description:
      "The full story in one model: a 3-bed conformable stack, a solid intrusion punched through it, an oblique erosion surface truncating the top, a fresh bed deposited on that unconformity, and a near-vertical vein cutting across everything. Toggle to Solids to see the units; 'manifold' seals the vein↔rock seam.",
    build: () => {
      // Element order is YOUNGEST FIRST (matches add_* order). The vein (cross-cutter) is youngest,
      // then the post-erosion cover package, the intrusion, and the basal stack oldest.
      const stack: ProjectElement = {
        kind: "series",
        id: newId("el"),
        name: "basal strata",
        relation: "conformable",
        unboundedAbove: true,
        // A tight 3-bed stack low in the box, leaving headroom for the erosion (~600) and the
        // post-erosion cover package above it.
        interfaces: [
          dippingInterface("bed 1", 200, 0, 0),
          dippingInterface("bed 2", 320, 0, 0),
          dippingInterface("bed 3", 440, 0, 0),
        ],
      };
      const intrusion: ProjectElement = {
        kind: "intrusion",
        id: newId("el"),
        name: "stock",
        mode: "solid",
        // Solid mode: `envelopeOuter` is the sole bounding surface; `envelopeInner` is retained but
        // unused (we point it at the same sphere so the data is well-formed if toggled to shell).
        envelopeInner: sphereEnvelope("core (unused in solid)", { x: 480, y: 500, z: 380 }, 175),
        envelopeOuter: sphereEnvelope("envelope", { x: 480, y: 500, z: 380 }, 175),
      };
      // The post-erosion package: an ERODE series whose LOWER interface is the oblique unconformity
      // (truncating everything below) and whose UPPER interface is a bed WITHIN the new deposits — so
      // "a layer deposited above the erosional contact" is a visible second unit, not a separate
      // element. `erode` already implies deposit-above (the package fills UP from the unconformity).
      const erosion: ProjectElement = {
        kind: "series",
        id: newId("el"),
        name: "post-erosion cover",
        relation: "erode",
        // The `erode` relation already fills upward to the box ceiling (it negates the fit pole,
        // so the field increases upward and `present_at` is an upward-unbounded half-space above
        // the erosion surface — see project.ts flattenInterfaces). Authoring `unboundedAbove: true`
        // here would additionally trip global_unit's fill-upward short-circuit and make the cover
        // claim BELOW its own unconformity, burying the older basal stack. Bounded, like the
        // validated `erosional-unconformity` scenario.
        unboundedAbove: false,
        interfaces: [
          dippingInterface("erosion surface", 600, 0.35, 0), // the unconformity (oblique)
          dippingInterface("cover bed", 800, 0.1, 0), // a bed within the cover, gently dipping
        ],
      };
      const vein: ProjectElement = {
        kind: "vein",
        id: newId("el"),
        name: "late vein",
        // Near-vertical, slightly dipping so it's oblique to the grid (the robust cross-cut case).
        footWall: dippingWall("foot wall", 470, 0.12),
        hangingWall: dippingWall("hanging wall", 560, 0.12),
      };

      // Bake a lithology catalog so the demo renders with distinct colours out of the box (unassigned
      // units render grey "Unknown"). Keys are `${elementId}:${unitIndex}`; the basal stack's 4 units
      // are the 3 beds + its truncated top, the cover + intrusion + vein each one body unit.
      const litho = (id: string, name: string, color: string) =>
        ({ id, name, color, kind: "user" as const });
      const catalog = {
        lithologies: [
          { id: "air", name: "Air", color: "#ffffff", kind: "air" as const },
          { id: "unknown", name: "Unknown", color: "#c8c8cd", kind: "unknown" as const },
          litho("l-ss", "Sandstone", "#d9b06a"),
          litho("l-sh", "Shale", "#6f7d55"),
          litho("l-ls", "Limestone", "#9fb1c4"),
          litho("l-base", "Basement", "#7a6b5d"),
          litho("l-int", "Granite (stock)", "#c0554e"),
          litho("l-cov", "Cover sand", "#e6d18a"),
          litho("l-cov2", "Cover marl", "#b7c99a"),
          litho("l-vein", "Quartz vein", "#8a63b8"),
        ],
        assignment: {
          [`${stack.id}:0`]: "l-base", // below bed 1
          [`${stack.id}:1`]: "l-ss", //   bed 1 → bed 2
          [`${stack.id}:2`]: "l-sh", //   bed 2 → bed 3
          [`${stack.id}:3`]: "l-ls", //   bed 3 → erosion (truncated top)
          [`${intrusion.id}:0`]: "l-int",
          [`${erosion.id}:0`]: "l-cov", //  unconformity → cover bed (lower post-erosion unit)
          [`${erosion.id}:1`]: "l-cov2", // cover bed → top (upper post-erosion unit)
          [`${vein.id}:0`]: "l-vein",
        } as Record<string, string>,
      };

      return {
        version: 1,
        name: "combined-geohistory",
        extent: EXTENT,
        elements: [vein, erosion, intrusion, stack],
        catalog,
      };
    },
  },
  {
    id: "erosion-over-stack",
    name: "Erosion over conformal stack — repro",
    description:
      "DIAGNOSTIC: a 2-unit ERODE cover over a 3-bed conformal stack, no cross-cutting bodies. Isolates the multi-interface-erode-over-conformal-stack composition: the basal stack's top unit (Limestone) MUST truncate at the erosion contact and NOT reach the model top; the cover's two units sit ABOVE it. If Limestone daylights above the erosion, the erode-clip of an older conformal series' TOP unit is the bug.",
    build: () => {
      const stack: ProjectElement = {
        kind: "series",
        id: newId("el"),
        name: "basal strata",
        relation: "conformable",
        unboundedAbove: true,
        interfaces: [
          dippingInterface("bed 1", 200, 0, 0),
          dippingInterface("bed 2", 320, 0, 0),
          dippingInterface("bed 3", 440, 0, 0),
        ],
      };
      const erosion: ProjectElement = {
        kind: "series",
        id: newId("el"),
        name: "post-erosion cover",
        relation: "erode",
        unboundedAbove: false,
        interfaces: [
          dippingInterface("erosion surface", 600, 0.35, 0),
          dippingInterface("cover bed", 800, 0.1, 0),
        ],
      };
      const litho = (id: string, name: string, color: string) =>
        ({ id, name, color, kind: "user" as const });
      const catalog = {
        lithologies: [
          { id: "air", name: "Air", color: "#ffffff", kind: "air" as const },
          { id: "unknown", name: "Unknown", color: "#c8c8cd", kind: "unknown" as const },
          litho("l-ss", "Sandstone", "#d9b06a"),
          litho("l-sh", "Shale", "#6f7d55"),
          litho("l-ls", "Limestone", "#9fb1c4"),
          litho("l-base", "Basement", "#7a6b5d"),
          litho("l-cov", "Cover sand", "#e6d18a"),
          litho("l-cov2", "Cover marl", "#b7c99a"),
        ],
        assignment: {
          [`${stack.id}:0`]: "l-base",
          [`${stack.id}:1`]: "l-ss",
          [`${stack.id}:2`]: "l-sh",
          [`${stack.id}:3`]: "l-ls",
          [`${erosion.id}:0`]: "l-cov",
          [`${erosion.id}:1`]: "l-cov2",
        } as Record<string, string>,
      };
      return {
        version: 1,
        name: "erosion-over-stack",
        extent: EXTENT,
        elements: [erosion, stack],
        catalog,
      };
    },
  },
  {
    id: "erosion-over-stack-imported",
    name: "Erosion over stack (imported meshes)",
    description:
      "The SAME geology as 'Erosion over conformal stack' — a 2-unit erode cover over a 3-bed conformal stack — but every surface is an IMPORTED ~100k-vertex .obj mesh (value-noise-perturbed) instead of a fitted parametric surface. Exercises feature-004 role parity (contact + erosion roles, erode/conformable relations) against realistic irregular surfaces. Meshes live under public/meshes/ (regenerate with gen_erosion_meshes.rs); the erosion surface must still truncate the beds below it.",
    build: () => {
      const litho = (id: string, name: string, color: string) =>
        ({ id, name, color, kind: "user" as const });
      // Each surface is an importedSurface referencing an .obj by path; objText is hydrated on
      // seed (main.tsx onSeed → hydrateMeshPaths). Order is youngest-first, mirroring the
      // parametric template: cover bed, erosion surface, then the three basal beds.
      const imported = (
        name: string,
        file: string,
        role: "contact" | "erosion",
        relation: "erode" | "conformable",
      ) => ({
        kind: "importedSurface" as const,
        id: newId("el"),
        name,
        role,
        relation,
        objText: "",
        objPath: `meshes/${file}`,
        // Meshes already span the full [0,1000]² footprint — no smooth extrapolation needed.
        extendToBounds: false,
      });
      const cover = imported("cover bed", "erosion-stack-cover.obj", "contact", "conformable");
      const erosion = imported("erosion surface", "erosion-stack-erosion.obj", "erosion", "erode");
      const bed3 = imported("bed 3", "erosion-stack-bed3.obj", "contact", "conformable");
      const bed2 = imported("bed 2", "erosion-stack-bed2.obj", "contact", "conformable");
      const bed1 = imported("bed 1", "erosion-stack-bed1.obj", "contact", "conformable");
      const catalog = {
        lithologies: [
          { id: "air", name: "Air", color: "#ffffff", kind: "air" as const },
          { id: "unknown", name: "Unknown", color: "#c8c8cd", kind: "unknown" as const },
          litho("l-ss", "Sandstone", "#d9b06a"),
          litho("l-sh", "Shale", "#6f7d55"),
          litho("l-ls", "Limestone", "#9fb1c4"),
          litho("l-base", "Basement", "#7a6b5d"),
          litho("l-cov", "Cover sand", "#e6d18a"),
          litho("l-cov2", "Cover marl", "#b7c99a"),
        ],
        // Each imported contact has one unit (index 0) — the rock BELOW its surface.
        assignment: {
          [`${bed1.id}:0`]: "l-base",
          [`${bed2.id}:0`]: "l-ss",
          [`${bed3.id}:0`]: "l-sh",
          [`${erosion.id}:0`]: "l-cov",
          [`${cover.id}:0`]: "l-cov2",
        } as Record<string, string>,
      };
      return {
        version: 1,
        name: "erosion-over-stack-imported",
        extent: EXTENT,
        elements: [cover, erosion, bed3, bed2, bed1],
        catalog,
      };
    },
  },
  {
    id: "conformal-stack-mixed",
    name: "Conformal stack (mixed imported + parametric)",
    description:
      "A 5-surface CONFORMABLE stack with the element types INTERLEAVED: three surfaces are imported ~100k-vertex meshes (the top bed, bed 3, bed 1) and two are single-interface PARAMETRIC (fitted) series (an upper bed, bed 2). Exercises the imported↔parametric stacking interaction — a parametric bed slabbing between two imported contacts and vice versa — which the volume mesher must tile into a clean layer cake regardless of each surface's type. (Erode interleaving of a parametric DEPOSIT series with imported contacts is a known-open follow-up; see the imported-mesh-volume-partition-residual memory.)",
    build: () => {
      const litho = (id: string, name: string, color: string) =>
        ({ id, name, color, kind: "user" as const });
      const imported = (name: string, file: string) => ({
        kind: "importedSurface" as const,
        id: newId("el"),
        name,
        role: "contact" as const,
        relation: "conformable" as const,
        objText: "",
        objPath: `meshes/${file}`,
        extendToBounds: false,
      });
      // A single PARAMETRIC surface as a standalone 1-interface CONFORMABLE series (its own stack
      // element, interleaved with the imported ones). `dippingInterface` matches the imported meshes'
      // analytic z/dip so the surfaces line up into a coherent stack.
      const paramSurface = (name: string, iface: InterfaceData): ProjectElement => ({
        kind: "series",
        id: newId("el"),
        name,
        relation: "conformable",
        unboundedAbove: false,
        interfaces: [iface],
      });

      // Youngest-first, matching the imported template's order and elevations. The "erosion" surface
      // slot is here a plain CONFORMABLE upper bed (parametric) — the erode-deposit interleaving is
      // deferred (see description).
      const cover = imported("top bed", "erosion-stack-cover.obj");
      const upper = paramSurface(
        "upper bed (parametric)",
        dippingInterface("upper bed", 600, 0.35, 0),
      );
      const bed3 = imported("bed 3", "erosion-stack-bed3.obj");
      const bed2 = paramSurface("bed 2 (parametric)", dippingInterface("bed 2", 320, 0, 0));
      const bed1 = imported("bed 1", "erosion-stack-bed1.obj");

      const catalog = {
        lithologies: [
          { id: "air", name: "Air", color: "#ffffff", kind: "air" as const },
          { id: "unknown", name: "Unknown", color: "#c8c8cd", kind: "unknown" as const },
          litho("l-ss", "Sandstone", "#d9b06a"),
          litho("l-sh", "Shale", "#6f7d55"),
          litho("l-ls", "Limestone", "#9fb1c4"),
          litho("l-base", "Basement", "#7a6b5d"),
          litho("l-cov", "Cover sand", "#e6d18a"),
          litho("l-cov2", "Cover marl", "#b7c99a"),
        ],
        // Imported contacts assign the below-rock at unit index 0; a bounded parametric 1-interface
        // series' below-rock is also unit 0 (§ feature 003 unit ladder).
        assignment: {
          [`${bed1.id}:0`]: "l-base",
          [`${bed2.id}:0`]: "l-ss",
          [`${bed3.id}:0`]: "l-sh",
          [`${upper.id}:0`]: "l-cov",
          [`${cover.id}:0`]: "l-cov2",
        } as Record<string, string>,
      };
      return {
        version: 1,
        name: "conformal-stack-mixed",
        extent: EXTENT,
        elements: [cover, upper, bed3, bed2, bed1],
        catalog,
      };
    },
  },
  {
    id: "cross-cutting-vein-dipping",
    name: "Cross-cutting vein (dipping) — repro",
    description:
      "DIAGNOSTIC: the walls DIP (oblique to the grid) AND bed 3 is CURVED — the case neither a vertical wall nor a flat-box slab can exercise. Toggle to Solids; turn 'manifold' on to seal the vein body↔bed junction the field path leaves as a peek-a-boo.",
    build: () => ({
      version: 1,
      name: "cross-cutting-vein-dipping",
      elements: [
        {
          kind: "vein",
          id: newId("el"),
          name: "quartz vein",
          footWall: dippingWall("foot wall", 400, 0.5),
          hangingWall: dippingWall("hanging wall", 600, 0.5),
        },
        curvedContactFloor(),
      ],
      extent: EXTENT,
    }),
  },
];

export function templateById(id: string): Template | undefined {
  return TEMPLATES.find((t) => t.id === id);
}

/** The default starting project when there is no autosaved one. */
export function blankProject(): Project {
  return templateById("blank")!.build();
}
