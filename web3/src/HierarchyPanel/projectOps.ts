// Feature 002 (enhanced-authoring) — pure operations on the Project document.
//
// All functions are PURE: they take a Project and return a NEW Project (or a read-only
// lookup), never mutating in place. The caller (main.tsx) marks dirty + autosaves. Split
// from project.ts (which holds the types + the compile seam) to keep the data model and the
// edit operations readable; both stay inside the reusable HierarchyPanel/ boundary.
//
//   T018 [US1] addSurface — the type->container DERIVATION (research.md R4)
//   T023 [US2] rename/duplicate/delete/add-interface, applyInterfaceData
//   T028 [US4] reorderContainers / reorderWithin

import {
  type InterfaceData,
  type OrientationRow,
  type PointRow,
  type Project,
  type ProjectElement,
  type SurfaceType,
  newId,
} from "./project";

function emptyInterface(name: string): InterfaceData {
  return { id: newId("iface"), name, points: [], orientations: [] };
}

/** A surface added flat (singleton series) needs a default name; count existing surfaces. */
function defaultSurfaceName(project: Project): string {
  let n = 0;
  for (const el of project.elements) {
    if (el.kind === "series") n += el.interfaces.length;
    else n += 2;
  }
  return `Surface ${n + 1}`;
}

/** Whether the OLDEST element (insert position = bottom) is a conformable series we can
 *  append to. v1 inserts at the oldest end, so the "adjacent conformable series" check
 *  looks at the last element. */
function oldestConformableSeriesIndex(project: Project): number {
  const i = project.elements.length - 1;
  const el = project.elements[i];
  if (el && el.kind === "series" && el.relation === "conformable") return i;
  return -1;
}

/**
 * T018 [US1] — the ONE type->container rule (research.md R4 / brief §1). Returns the new
 * project and which interface to open in the data modal next.
 *   conformable -> append to the adjacent conformable series if one is at the insert
 *                  position, else a new conformable series;
 *   erosional/onlap -> a NEW single-surface series with that relation;
 *   vein/intrusion -> a NEW cross-cutting container with the required pair/envelope.
 * Inserts at the OLDEST end (bottom). The user reorders afterwards (insert-at-selection is
 * out of scope for v1).
 */
export function addSurface(
  project: Project,
  type: SurfaceType,
): { project: Project; openInterface: { elementId: string; interfaceId: string } } {
  const name = defaultSurfaceName(project);

  if (type === "conformable") {
    const idx = oldestConformableSeriesIndex(project);
    if (idx >= 0) {
      const target = project.elements[idx];
      if (target.kind === "series") {
        const iface = emptyInterface(name);
        const elements = project.elements.map((el, i) =>
          i === idx && el.kind === "series" ? { ...el, interfaces: [...el.interfaces, iface] } : el,
        );
        return { project: { ...project, elements }, openInterface: { elementId: target.id, interfaceId: iface.id } };
      }
    }
    // no adjacent conformable series -> new conformable series
    const iface = emptyInterface(name);
    const el: ProjectElement = {
      kind: "series",
      id: newId("el"),
      name,
      relation: "conformable",
      unboundedAbove: false,
      interfaces: [iface],
    };
    return {
      project: { ...project, elements: [...project.elements, el] },
      openInterface: { elementId: el.id, interfaceId: iface.id },
    };
  }

  if (type === "erosional" || type === "onlap") {
    const iface = emptyInterface(name);
    const el: ProjectElement = {
      kind: "series",
      id: newId("el"),
      name,
      relation: type === "erosional" ? "erode" : "onlap",
      unboundedAbove: false,
      interfaces: [iface],
    };
    return {
      project: { ...project, elements: [...project.elements, el] },
      openInterface: { elementId: el.id, interfaceId: iface.id },
    };
  }

  if (type === "vein") {
    const footWall = emptyInterface("foot wall");
    const hangingWall = emptyInterface("hanging wall");
    const el: ProjectElement = { kind: "vein", id: newId("el"), name: `Vein ${name}`, footWall, hangingWall };
    return {
      project: { ...project, elements: [...project.elements, el] },
      openInterface: { elementId: el.id, interfaceId: footWall.id },
    };
  }

  // intrusion
  const envelopeInner = emptyInterface("inner envelope");
  const envelopeOuter = emptyInterface("outer envelope");
  const el: ProjectElement = {
    kind: "intrusion",
    id: newId("el"),
    name: `Intrusion ${name}`,
    envelopeInner,
    envelopeOuter,
  };
  return {
    project: { ...project, elements: [...project.elements, el] },
    openInterface: { elementId: el.id, interfaceId: envelopeInner.id },
  };
}

// --- interface lookup + per-interface mutation ----------------------------------------

/** All interfaces of an element, with their human role label (for the modal header). */
function interfacesOf(el: ProjectElement): { iface: InterfaceData; role: string }[] {
  if (el.kind === "series") return el.interfaces.map((iface) => ({ iface, role: el.name }));
  if (el.kind === "vein")
    return [
      { iface: el.footWall, role: `${el.name} · foot wall` },
      { iface: el.hangingWall, role: `${el.name} · hanging wall` },
    ];
  return [
    { iface: el.envelopeInner, role: `${el.name} · inner envelope` },
    { iface: el.envelopeOuter, role: `${el.name} · outer envelope` },
  ];
}

/** Read-only lookup: the interface + its name + parent label, for the data modal. */
export function findInterface(
  project: Project,
  elementId: string,
  interfaceId: string,
): { name: string; parentName: string; points: PointRow[]; orientations: OrientationRow[] } | null {
  const el = project.elements.find((e) => e.id === elementId);
  if (!el) return null;
  const hit = interfacesOf(el).find((x) => x.iface.id === interfaceId);
  if (!hit) return null;
  return {
    name: hit.iface.name,
    parentName: hit.role,
    points: hit.iface.points,
    orientations: hit.iface.orientations,
  };
}

/** Replace one interface within an element by id, via a mapping function. */
function mapInterface(
  el: ProjectElement,
  interfaceId: string,
  fn: (iface: InterfaceData) => InterfaceData,
): ProjectElement {
  if (el.kind === "series") {
    return { ...el, interfaces: el.interfaces.map((i) => (i.id === interfaceId ? fn(i) : i)) };
  }
  if (el.kind === "vein") {
    return {
      ...el,
      footWall: el.footWall.id === interfaceId ? fn(el.footWall) : el.footWall,
      hangingWall: el.hangingWall.id === interfaceId ? fn(el.hangingWall) : el.hangingWall,
    };
  }
  return {
    ...el,
    envelopeInner: el.envelopeInner.id === interfaceId ? fn(el.envelopeInner) : el.envelopeInner,
    envelopeOuter: el.envelopeOuter.id === interfaceId ? fn(el.envelopeOuter) : el.envelopeOuter,
  };
}

function mapElement(project: Project, elementId: string, fn: (el: ProjectElement) => ProjectElement): Project {
  return { ...project, elements: project.elements.map((el) => (el.id === elementId ? fn(el) : el)) };
}

/** Commit edited point/orientation tables back to an interface (the modal Apply target). */
export function applyInterfaceData(
  project: Project,
  elementId: string,
  interfaceId: string,
  points: PointRow[],
  orientations: OrientationRow[],
): Project {
  return mapElement(project, elementId, (el) => mapInterface(el, interfaceId, (i) => ({ ...i, points, orientations })));
}

export function renameInterface(project: Project, elementId: string, interfaceId: string, name: string): Project {
  return mapElement(project, elementId, (el) => mapInterface(el, interfaceId, (i) => ({ ...i, name })));
}

/** Toggle the orientation flip on one interface (negates its poles at compile time). The
 *  raw dip/dipDir are untouched; only the field's "up" sense flips. Useful when a fitted
 *  surface comes out inverted (e.g. an erosional deposit whose contact caps from above). */
export function flipOrientation(project: Project, elementId: string, interfaceId: string): Project {
  return mapElement(project, elementId, (el) => mapInterface(el, interfaceId, (i) => ({ ...i, flipped: !i.flipped })));
}

/** Duplicate an interface. Only meaningful for a series (vein/intrusion have fixed slots);
 *  for those, duplicating is a no-op. */
export function duplicateInterface(project: Project, elementId: string, interfaceId: string): Project {
  return mapElement(project, elementId, (el) => {
    if (el.kind !== "series") return el;
    const src = el.interfaces.find((i) => i.id === interfaceId);
    if (!src) return el;
    const copy: InterfaceData = {
      ...src,
      id: newId("iface"),
      name: `${src.name} copy`,
      points: src.points.map((p) => ({ ...p })),
      orientations: src.orientations.map((o) => ({ ...o })),
    };
    const at = el.interfaces.findIndex((i) => i.id === interfaceId);
    const interfaces = [...el.interfaces];
    interfaces.splice(at + 1, 0, copy);
    return { ...el, interfaces };
  });
}

/** Delete an interface from a series. If it empties the series, the series is dropped too.
 *  Vein/intrusion slots are fixed — deleting a constituent deletes the whole body. */
export function deleteInterface(project: Project, elementId: string, interfaceId: string): Project {
  const el = project.elements.find((e) => e.id === elementId);
  if (!el) return project;
  if (el.kind !== "series") {
    return { ...project, elements: project.elements.filter((e) => e.id !== elementId) };
  }
  const interfaces = el.interfaces.filter((i) => i.id !== interfaceId);
  if (interfaces.length === 0) {
    return { ...project, elements: project.elements.filter((e) => e.id !== elementId) };
  }
  return mapElement(project, elementId, (e) => (e.kind === "series" ? { ...e, interfaces } : e));
}

/** Append an empty interface to a series. */
export function addInterface(project: Project, elementId: string): Project {
  return mapElement(project, elementId, (el) =>
    el.kind === "series"
      ? { ...el, interfaces: [...el.interfaces, emptyInterface(`Surface ${el.interfaces.length + 1}`)] }
      : el,
  );
}

export function renameElement(project: Project, elementId: string, name: string): Project {
  return mapElement(project, elementId, (el) => ({ ...el, name }));
}

export function deleteElement(project: Project, elementId: string): Project {
  return { ...project, elements: project.elements.filter((e) => e.id !== elementId) };
}

// --- T028 [US4] reorder ---------------------------------------------------------------

function moveItem<T>(arr: T[], from: number, to: number): T[] {
  const next = [...arr];
  const [item] = next.splice(from, 1);
  next.splice(to, 0, item);
  return next;
}

/** Reorder containers by id (chronology change). `fromId` moves to `toId`'s position. */
export function reorderContainers(project: Project, fromId: string, toId: string): Project {
  const from = project.elements.findIndex((e) => e.id === fromId);
  const to = project.elements.findIndex((e) => e.id === toId);
  if (from < 0 || to < 0 || from === to) return project;
  return { ...project, elements: moveItem(project.elements, from, to) };
}

/** Reorder surfaces within a series (isovalue order of one shared field). */
export function reorderWithin(project: Project, elementId: string, from: number, to: number): Project {
  return mapElement(project, elementId, (el) =>
    el.kind === "series" && from >= 0 && to >= 0 && from < el.interfaces.length && to < el.interfaces.length
      ? { ...el, interfaces: moveItem(el.interfaces, from, to) }
      : el,
  );
}
