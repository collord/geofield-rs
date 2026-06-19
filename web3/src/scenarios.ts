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
  const points = [];
  for (let i = 0; i < 4; i++) {
    for (let j = 0; j < 4; j++) {
      const x = i * 300 + 50;
      const y = j * 300 + 50;
      points.push({ x, y, z: z0 + sx * (x - 500) + sy * (y - 500) });
    }
  }
  // up-pole = (−sx,−sy,1)/n  ⇒  dip = acos(nz), dipDir = atan2(nx, ny) (clockwise from +Y).
  const n = Math.hypot(sx, sy, 1);
  const nx = -sx / n,
    ny = -sy / n,
    nz = 1 / n;
  const dip = (Math.acos(nz) * 180) / Math.PI;
  let dipDir = (Math.atan2(nx, ny) * 180) / Math.PI;
  if (dipDir < 0) dipDir += 360;
  return {
    id: newId("iface"),
    name,
    points,
    orientations: [{ x: 500, y: 500, z: z0, dip, dipDir }],
    flipped,
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
      "A tabular vein (foot-wall/hanging-wall pair at z≈400/600) cross-cuts the floor it passes through.",
    build: () => ({
      version: 1,
      name: "cross-cutting-vein",
      extent: EXTENT,
      elements: [
        {
          kind: "vein",
          id: newId("el"),
          name: "quartz vein",
          footWall: dippingInterface("foot wall", 400, 0, 0),
          hangingWall: dippingInterface("hanging wall", 600, 0, 0),
        },
        threeContactFloor(),
      ],
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
