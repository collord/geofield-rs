// Feature 002 (enhanced-authoring) T009 — the three former canned scenarios, re-exposed as
// SEED TEMPLATES that each return a `Project` (not the auto-loaded dropdown of SeriesJson[]).
// Plus a "blank" template. These seed the authoring document; the user then edits/adds/
// reorders and re-fits via Apply. (Feature 001's scenarios.ts built SeriesJson[] directly;
// the authoring layer compiles a Project down to that shape via project.ts::toSeriesJson.)
//
// The dipping-plane / multi-bed-floor geology is preserved so a seeded template fits to a
// meaningful picture — the round-trip anchor in quickstart.md.

import type { InterfaceData, Project, ProjectElement } from "./HierarchyPanel/project";
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
          // Seed FLIPPED: the basalt deposit caps the unconformity from above (the field's
          // "up" sense is inverted vs. a conformable bed). Renders correct out of the box.
          interfaces: [dippingInterface("erosion surface", 650, 0.4, 0, true)],
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
