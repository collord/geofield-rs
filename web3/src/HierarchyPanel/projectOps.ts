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
  type Lithology,
  type LithologyCatalog,
  type OrientationRow,
  type PointRow,
  type Project,
  type ProjectElement,
  type SurfaceType,
  AIR_ID,
  UNKNOWN_ID,
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
/** Whether the project already has the singleton topography surface (feature 003 §2). */
export function hasTopography(project: Project): boolean {
  return project.elements.some((el) => el.kind === "topography");
}

export function addSurface(
  project: Project,
  type: SurfaceType,
): { project: Project; openInterface: { elementId: string; interfaceId: string } } {
  const name = defaultSurfaceName(project);

  if (type === "topography") {
    // Singleton, pinned youngest (front of the list). A second add is a no-op (the picker
    // disables it, but guard here too so the invariant holds structurally).
    if (hasTopography(project)) {
      return { project, openInterface: { elementId: "", interfaceId: "" } };
    }
    const ground = emptyInterface("ground");
    const el: ProjectElement = { kind: "topography", id: newId("el"), name: "Topography", ground };
    return {
      project: { ...project, elements: [el, ...project.elements] },
      openInterface: { elementId: el.id, interfaceId: ground.id },
    };
  }

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

  // intrusion — defaults to a SOLID body (one envelope, the common stock/pluton). The inner
  // envelope is created but unused until the user switches to shell (ring dike / cored pluton).
  const envelopeInner = emptyInterface("inner envelope");
  const envelopeOuter = emptyInterface("outer envelope");
  const el: ProjectElement = {
    kind: "intrusion",
    id: newId("el"),
    name: `Intrusion ${name}`,
    mode: "solid",
    envelopeInner,
    envelopeOuter,
  };
  return {
    project: { ...project, elements: [...project.elements, el] },
    openInterface: { elementId: el.id, interfaceId: envelopeOuter.id },
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
  if (el.kind === "topography") return [{ iface: el.ground, role: `${el.name} · ground` }];
  // importedSurface has no editable InterfaceData (its geometry is the mesh).
  if (el.kind === "importedSurface") return [];
  // "solid" mode edits only the sole envelope; "shell" exposes both walls.
  if ((el.mode ?? "shell") === "solid")
    return [{ iface: el.envelopeOuter, role: `${el.name} · envelope` }];
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
  if (el.kind === "topography") {
    return { ...el, ground: el.ground.id === interfaceId ? fn(el.ground) : el.ground };
  }
  // importedSurface carries no editable InterfaceData — nothing to map.
  if (el.kind === "importedSurface") return el;
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

/** Set the role of an IMPORTED surface (feature 004): contact/erosion/veinWall/topography. A
 *  geometry-affecting edit (the role changes how it clips) — the caller commits + re-fits. */
export function setImportRole(
  project: Project,
  elementId: string,
  role: "contact" | "erosion" | "veinWall" | "topography",
): Project {
  return mapElement(project, elementId, (el) =>
    el.kind === "importedSurface" ? { ...el, role } : el,
  );
}

/** Toggle whether an imported surface is projected to the domain bounds (feature 004 US2). Turning
 *  it ON runs the co-kriged extrapolation beyond the data footprint — a geometry edit that re-fits. */
export function setImportExtend(project: Project, elementId: string, extend: boolean): Project {
  return mapElement(project, elementId, (el) =>
    el.kind === "importedSurface" ? { ...el, extendToBounds: extend } : el,
  );
}

/** Switch an intrusion between a SOLID body (one envelope — engine `Single`) and a hollow SHELL
 *  (inner + outer envelope — engine `Shell`). A geometry edit that re-fits: solid compiles the
 *  outer envelope alone as a single-envelope body; shell compiles both walls as a band. The
 *  retained inner envelope's authored points survive a round-trip through solid mode. */
export function setIntrusionMode(project: Project, elementId: string, mode: "solid" | "shell"): Project {
  return mapElement(project, elementId, (el) => (el.kind === "intrusion" ? { ...el, mode } : el));
}

// --- unit names (feature 003 §1) — units are DERIVED; only their display name is authored ---

/** The `(element, unit)` coordinate key shared by unit names and lithology assignment. */
export function unitKey(elementId: string, unitIndex: number): string {
  return `${elementId}:${unitIndex}`;
}

/** The display name for a derived unit: the user's name if set, else "Unit {displayPos}".
 *  `displayPos` is the VISUAL ladder position (1 = top); the stored name is keyed by the
 *  stable mesher `unitIndex`. Defaults `displayPos` to `unitIndex + 1` if not given. */
export function unitNameOf(project: Project, elementId: string, unitIndex: number, displayPos?: number): string {
  return project.unitNames?.[unitKey(elementId, unitIndex)] ?? `Unit ${displayPos ?? unitIndex + 1}`;
}

/** Set (or clear, with "") a derived unit's display name. */
export function renameUnit(project: Project, elementId: string, unitIndex: number, name: string): Project {
  const names = { ...(project.unitNames ?? {}) };
  const key = unitKey(elementId, unitIndex);
  if (name.trim() === "") delete names[key];
  else names[key] = name;
  return { ...project, unitNames: names };
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

/** Reorder containers by id (chronology change). `fromId` moves to `toId`'s position.
 *  Guards the topography singleton/youngest invariant (feature 003 §2.1, FR-002): topography
 *  cannot be moved, and nothing may move above it into index 0. A blocked move is a no-op. */
export function reorderContainers(project: Project, fromId: string, toId: string): Project {
  const from = project.elements.findIndex((e) => e.id === fromId);
  const to = project.elements.findIndex((e) => e.id === toId);
  if (from < 0 || to < 0 || from === to) return project;
  // Topography is pinned youngest: refuse to move it, and refuse to move anything above it.
  const moving = project.elements[from];
  if (moving.kind === "topography") return project;
  const topoAt = project.elements.findIndex((e) => e.kind === "topography");
  if (topoAt === 0 && to <= topoAt) return project; // would land at/above topography
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

// ============================================================================
// Lithology catalog (feature 003 §4) — UI-side identity metadata. T029–T032.
// ============================================================================

/** The two fixed catalog members present in every catalog (data-model §3). */
function fixedLithologies(): Lithology[] {
  return [
    { id: AIR_ID, name: "Air", color: "#ffffff", kind: "air" },
    { id: UNKNOWN_ID, name: "Unknown", color: "#c8c8cd", kind: "unknown" },
  ];
}

/** A fresh catalog: Air + Unknown fixed, no user/inferred entries, empty assignment. */
export function defaultCatalog(): LithologyCatalog {
  return { lithologies: fixedLithologies(), assignment: {} };
}

/** The catalog on a project, creating the default if absent (older saved projects). */
export function catalogOf(project: Project): LithologyCatalog {
  return project.catalog ?? defaultCatalog();
}

function withCatalog(project: Project, catalog: LithologyCatalog): Project {
  return { ...project, catalog };
}

/** The lithology assigned to a derived unit. Below-topography units default to Unknown; a
 *  dangling assignment (its lithology was removed) also resolves to Unknown (data-model D1). */
export function lithologyOf(project: Project, elementId: string, unitIndex: number): Lithology {
  const cat = catalogOf(project);
  const id = cat.assignment[unitKey(elementId, unitIndex)] ?? UNKNOWN_ID;
  return cat.lithologies.find((l) => l.id === id) ?? cat.lithologies.find((l) => l.id === UNKNOWN_ID)!;
}

/** Assign a catalog entry to a unit (many-to-one is allowed — FR-019). Assigning Unknown
 *  clears the explicit assignment (Unknown is the default). */
export function assignLithology(project: Project, elementId: string, unitIndex: number, lithologyId: string): Project {
  const cat = catalogOf(project);
  const assignment = { ...cat.assignment };
  const key = unitKey(elementId, unitIndex);
  if (lithologyId === UNKNOWN_ID) delete assignment[key];
  else assignment[key] = lithologyId;
  return withCatalog(project, { ...cat, assignment });
}

/** Add a user lithology (name + colour). Returns the new project; the entry appears in every
 *  unit dropdown (FR-018). */
export function addLithology(project: Project, name: string, color: string): Project {
  const cat = catalogOf(project);
  const litho: Lithology = { id: newId("litho"), name, color, kind: "user" };
  return withCatalog(project, { ...cat, lithologies: [...cat.lithologies, litho] });
}

/** Rename/recolour a user or inferred entry (Air/Unknown are fixed — no-op for them). */
export function updateLithology(project: Project, id: string, patch: { name?: string; color?: string }): Project {
  const cat = catalogOf(project);
  const lithologies = cat.lithologies.map((l) =>
    l.id === id && (l.kind === "user" || l.kind === "inferred") ? { ...l, ...patch } : l,
  );
  return withCatalog(project, { ...cat, lithologies });
}

/** Delete a USER entry (Air/Unknown fixed; inferred go with their body). Assignments pointing
 *  at it fall back to Unknown (data-model D1.3). */
export function deleteLithology(project: Project, id: string): Project {
  const cat = catalogOf(project);
  const target = cat.lithologies.find((l) => l.id === id);
  if (!target || target.kind !== "user") return project;
  const lithologies = cat.lithologies.filter((l) => l.id !== id);
  const assignment = Object.fromEntries(Object.entries(cat.assignment).filter(([, v]) => v !== id));
  return withCatalog(project, { lithologies, assignment });
}

// --- inferred-lithology sync (T031): a vein/intrusion body contributes a lithology ----------

/** The stable inferred-lithology id for a body element (one per body). */
function inferredId(elementId: string): string {
  return `inferred:${elementId}`;
}

/** Reconcile inferred lithologies with the project's bodies (vein/intrusion): one inferred
 *  entry per body, named after it, auto-assigned to the body's interior unit (index 0).
 *  Removes inferred entries whose body is gone; renames where the body was renamed. Called
 *  after any add/rename/delete of a body (data-model D2). Pure: returns a new project. */
export function syncInferredLithologies(project: Project): Project {
  const cat = catalogOf(project);
  const bodies = project.elements.filter((e) => e.kind === "vein" || e.kind === "intrusion");
  const bodyIds = new Set(bodies.map((b) => inferredId(b.id)));

  // Keep non-inferred entries + inferred entries whose body still exists (re-synced name).
  const kept = cat.lithologies.filter((l) => l.kind !== "inferred" || bodyIds.has(l.id));
  const lithologies = kept.map((l) => {
    if (l.kind !== "inferred") return l;
    const body = bodies.find((b) => inferredId(b.id) === l.id)!;
    return { ...l, name: body.name }; // re-sync the name to the body
  });

  // Add a fresh inferred entry for any body that doesn't have one yet, auto-assigned.
  const assignment = { ...cat.assignment };
  for (const b of bodies) {
    const id = inferredId(b.id);
    if (!lithologies.some((l) => l.id === id)) {
      lithologies.push({ id, name: b.name, color: "#a05bd4", kind: "inferred" });
    }
    // Auto-assign the body's interior unit (index 0) to its inferred lithology.
    assignment[unitKey(b.id, 0)] = id;
  }

  // Drop assignments that point at a removed inferred entry (→ fall back to Unknown).
  const liveIds = new Set(lithologies.map((l) => l.id));
  for (const [k, v] of Object.entries(assignment)) {
    if (!liveIds.has(v)) delete assignment[k];
  }

  return withCatalog(project, { lithologies, assignment });
}

/** Compile the project's `(elementId, unitIndex)` lithology assignment into the index form the
 *  worker needs: `${seriesIndex}:${unitIndex}` → lithologyId. The series index matches
 *  `toSeriesJson`'s output order (topography emitted youngest/first), so the worker can tag
 *  each solid by `(series, unit)` without knowing element ids (feature 003 §5.3). */
export function indexedAssignment(project: Project): Record<string, string> {
  // Mirror toSeriesJson's ordering: topography first, then the rest in declared order.
  const ordered = [...project.elements].sort((a, b) => {
    const at = a.kind === "topography" ? 0 : 1;
    const bt = b.kind === "topography" ? 0 : 1;
    return at - bt;
  });
  const cat = catalogOf(project);
  const out: Record<string, string> = {};
  ordered.forEach((el, si) => {
    for (const [key, lithoId] of Object.entries(cat.assignment)) {
      const [elId, unit] = key.split(":");
      if (elId === el.id) out[`${si}:${unit}`] = lithoId;
    }
  });
  return out;
}
