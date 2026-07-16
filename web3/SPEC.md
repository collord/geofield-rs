# web3 — Stack Authoring Harness · Functional Specification

**Status:** prototype / testbed (not production)
**Audience:** users driving the app (geologists, modelers, and engineers exercising the geofield engine)
**Scope:** the browser application in `web3/` — its authoring model, controls, and behavior. Seed
templates and the underlying meshing engine are out of scope except where the UI exposes them.

---

## 1. Purpose

The web3 harness is an interactive **stratigraphic stack authoring tool**. A user builds an
in-browser geological model as an ordered stack of surfaces and bodies, edits the data that defines
each surface, and — on an explicit **Apply** — fits and meshes the model with the geofield engine,
then inspects the result in a live 3D view as either open contact surfaces or closed lithology
solids. Projects can be named, saved, exported, and re-imported, so a specific model and viewpoint
can be shared and reproduced.

It is a **prototype**: it favors direct manipulation and fast iteration over polish, and it exposes
a few engine-diagnostic toggles alongside the authoring controls. It is the functional gate for the
engine — if a geological configuration can be authored here and it renders correctly, the engine
behavior is considered demonstrated.

### 1.1 What the user works with

- **A Project** — a named model with a 3D **extent** (an axis-aligned box), an ordered list of
  **elements** (youngest at top), an optional **lithology catalog**, and an optional saved **view**.
- **Elements** — the building blocks of the stack. Each is one of the types below; **§3.1.2 explains
  in plain language exactly what each does** to the surfaces around it.
  - **Surface / bed** (a *conformable*, *erosional*, or *onlap* contact),
  - **Vein** (a thin sheet-like body that slices across the layers it passes through),
  - **Intrusion** (a blob-like body that displaces what was there — a *solid* stock or a hollow *shell*),
  - **Topography** (the single ground surface capping the model; empty Air above it),
  - **Imported surface** (an external `.obj` triangle mesh used in any of the above roles).
- **Surfaces** — the interpolation data behind a parametric element: **contact points** (XYZ) and
  optional **orientations** (dip / dip-direction). Imported surfaces have no such data — their
  geometry *is* the mesh.
- **Units & lithologies** — the model's rock volumes are *derived* from the stack (not authored). A
  user names units and assigns each a **lithology** (a named, colored rock type) from a catalog.

### 1.2 Axis & geology conventions (as presented in the UI)

- **X = East, Y = North, Z = Up.** The gnomon in the 3D view is labeled accordingly.
- **Dip** is degrees from horizontal; **dip-direction** is the azimuth of steepest descent, clockwise
  from North (+Y). These are what the user types; the engine's internal pole is derived from them.
- **Stack order is youngest-first** (top of the list = youngest). Relations describe how an element
  meets what is older/below it.

---

## 2. Core interaction model

### 2.1 The two-level edit-and-apply loop

The single most important behavior: **geometry-affecting edits are deferred; nothing re-fits until
the user clicks Apply.** This lets the user make many changes and pay the (potentially slow) fit
cost once.

- A **geometry-affecting edit** (adding/deleting/reordering a surface, editing its point or
  orientation table, flipping it, changing an imported role, changing bounds) marks the project
  **dirty**. A grey scrim over the 3D view signals that what's shown is stale.
- **Apply** re-fits and re-meshes the whole stack; **Cancel** reverts to the last-applied state.
- A few **cheap declared-state edits** (changing a series' relation seam, or lithology
  names/colors/assignments) take effect **immediately without a re-fit** and do not set dirty.
- **Surface data editing has its own inner Apply**: the point/orientation modal's *Apply* commits the
  table edits into the project (still deferred); the panel-level *Apply* is what re-fits. So a user
  edits several surfaces, then fits once.

**US-EDIT-1.** *As a user, I edit several surfaces and then click Apply once, so I fit the whole
model in a single pass rather than re-fitting after each change.*

**US-EDIT-2.** *As a user, I see a grey scrim whenever my edits haven't been applied, so I never
mistake a stale render for the current model.*

**US-EDIT-3.** *As a user, I click Cancel to discard un-applied edits and return to the last model
I fit.*

### 2.2 Status & feedback

While a fit/mesh runs, the app streams a **status** line (e.g. "fitting stack…", "done", or an
error). Fitting happens off the UI thread; a new edit cancels an in-flight fit. Errors surface as a
readable status message rather than a blank view.

**US-STATUS-1.** *As a user, I watch a status line so I know whether the model is fitting, done, or
errored, and I get a clear message when a fit fails (e.g. an unmeshable surface).*

### 2.3 Validation

The panel continuously validates the authored model and **enables Apply only when the model can be
fit**. Hard errors (e.g. a surface with no usable data) block Apply and show the reason; soft
warnings inform without blocking.

**US-VALID-1.** *As a user, when Apply is disabled I see why (the first blocking reason), so I know
what to fix.*

---

## 3. Authoring the stack

### 3.1 Building the stack

**US-STACK-1.** *As a user, I add a surface and choose its type — Conformable, Erosional, Onlap,
Vein, Intrusion, or Topography — and the app applies that type's rules for how it interacts with the
surfaces above, below, and crossing it, so I don't have to encode the geology by hand.*

#### 3.1.1 The shared mental model (read this first)

You don't have to know geology to use the harness, but you do need three rules that every type
builds on. Think of the model as a **stack of layers, listed youngest (top of the list) to oldest
(bottom)**, filling a 3D box (the extent).

1. **A surface divides space into "below it" and "above it."** Each ordinary surface marks the
   **top of one rock layer**; that layer fills the space *below* the surface, down to the next-older
   surface beneath it. (So a plain bed is the slab *between its own surface and the one below it*.)
2. **Where two layers claim the same point, the younger one wins.** The color/rock you see at any
   point is whichever layer is *youngest among those present there*. This one rule decides every
   "who shows here" question — the difference between the types below is **not** who wins a point,
   it's **which surfaces get cut (trimmed) and where**.
3. **"Cut" means trimmed away, not deleted.** When a type "cuts" a surface, that surface simply
   stops — it is trimmed along the line where it meets the cutting surface and only survives on the
   surviving side. Nothing is destroyed; the geometry just ends there.

With that, here is exactly what each type does.

#### 3.1.2 What each type does to the surfaces around it

- **Conformable** *(a plain, matched bed)* — the neutral case. It **cuts nothing** and is **cut by
  nothing** by virtue of being conformable; it just stacks in place as the slab between its surface
  and the next-older one. Use it for beds that were laid down in an unbroken sequence, parallel to
  their neighbors. (Menu gloss: "stacks conformably; shares a field with its neighbours.")

- **Erosional** *(a surface that shaved off what was there before)* — represents a gap in time where
  older rock was eroded away and later buried. An erosional surface **trims every older surface it
  crosses**: below the erosional surface the older layers survive, but where an older surface would
  have poked *up through* the erosional surface, it is cut off flush along it. Younger layers then
  sit on top of the erosion. Net effect: **younger-on-top wins, and older surfaces are truncated at
  the erosion line.** (Menu gloss: "truncates older surfaces below it.")

- **Onlap** *(a surface that laps up against a pre-existing slope and thins to nothing)* — the mirror
  image of erosional. Instead of cutting the *older* surfaces, an onlap surface **cuts itself**: it
  exists only on the side where it was deposited (up against and above the older surface it laps
  onto) and **pinches out to a feather edge** along the line where it meets that older surface — it
  does *not* pass through it. The older surfaces are left untouched. Use it for sediments that
  gradually filled in against a standing hillside or basin margin. (Menu gloss: "laps onto and
  pinches out against older topography.")

  *(Erosional and onlap are deliberate opposites: erosional trims the OLD surfaces at the contact;
  onlap trims the NEW surface at the contact. In both cases the same younger-wins rule still colors
  the volumes — the two differ only in which surface gets the free, trimmed edge.)*

- **Vein** *(a thin sheet-like body that slices across the layers it passes through)* — a
  **cross-cutting** body: it is defined by two walls (a foot-wall and a hanging-wall surface) and
  occupies the thin slab *between* them. Wherever it passes, **every older surface it crosses is cut
  out inside the vein's band** — older rock survives *outside* the two walls, and the vein rock fills
  between them. Unlike a bed, a vein isn't confined to one slot in the stack: it can slice through
  many older layers at once (like a crack later filled with mineral). It cuts only what is *older*
  than it, so its position in the list still sets how much it crosses. (Menu gloss: "tabular body
  that cross-cuts what it passes through.")

- **Intrusion** *(a blob of rock that pushed in and displaced what was there)* — also a
  cross-cutting body, but volumetric rather than sheet-like. **Older surfaces are cut out *inside*
  the intrusion's envelope**, and the intrusion fills that interior; older rock survives *outside*
  it. Two shapes:
  - **Solid** — a filled body bounded by a single envelope (a stock/pluton — a big blob).
  - **Shell** — a hollow-cored body: rock fills the space *between* an inner and an outer envelope,
    leaving the core to whatever else occupies it (a ring dike / cored pluton).

  (Menu gloss: "volumetric body that cross-cuts; fills its interior.")

- **Topography** *(the ground surface — the top of the whole model)* — the single surface that
  **caps the model against Air**. Everything **below** it is rock (resolved by the younger-wins rule
  among the layers); everything **above** it is empty (Air) and is not meshed. It applies to the
  whole model regardless of where it sits in the list, and there is **at most one per model** (the
  menu option is disabled once one exists). Use it to give the model a realistic ground/erosion
  surface as its ceiling. (Menu gloss: "the ground surface; caps the model — Air above it.")

**A note on ordering:** because "younger wins" and "who cuts whom" both depend on the youngest→oldest
order, **the position of an element in the list is itself geologically meaningful** — moving a vein
or an erosional surface up or down the list changes what it cuts. See US-STACK-2.

**US-STACK-2.** *As a user, I reorder elements (and reorder interfaces within a multi-interface
series) by drag or by move-up/move-down, so I set the youngest-to-oldest chronology — which
determines both which layer wins where they overlap and which surfaces each type cuts.*

**US-STACK-3.** *As a user, I rename or delete any element or surface, and duplicate a surface, so I
manage the stack directly.*

**US-STACK-4.** *As a user, a series with a single interface is shown flat as a plain surface; only
when it has multiple interfaces does it nest — so simple beds stay simple.*

### 3.2 Editing surface data (parametric surfaces)

**US-DATA-1.** *As a user, I open a surface's data modal and edit two tables — contact **points**
(XYZ) and **orientations** (X, Y, Z, dip, dip-direction) — then Apply the modal to commit the edits
into the project.*

**US-DATA-2.** *As a user, I flip a surface's orientation (its "which side is up" sense) as a
reversible toggle that never rewrites the dip/dip-direction values I typed.*

**US-DATA-3.** *As a user, I set a series' relation (conformable / onlap / erode) and it takes
effect immediately without re-fitting, so I can compare relations quickly.*

### 3.3 Cross-cutting bodies

*(Behavior is defined in §3.1.2 — a vein cuts older surfaces inside its band; an intrusion cuts them
inside its envelope. This section is about how you define one.)*

**US-BODY-1.** *As a user, I add a **vein** defined by its two bounding surfaces — a foot-wall and a
hanging-wall — and the vein occupies the slab between them, slicing through the layers it crosses.*

**US-BODY-2.** *As a user, I add an **intrusion** and choose **Solid** (a filled blob bounded by one
envelope surface — a stock/pluton) or **Shell** (a hollow-cored body, rock filling the space between
an inner and an outer envelope — a ring dike), so I model both filled and cored bodies.*

### 3.4 The model extent (bounds)

**US-BOUNDS-1.** *As a user, I set the model bounds by switching to a top-down view, clicking two
corners of the rectangle, then entering the Z (elevation) range — so I frame the domain box the
model is meshed within.*

---

## 4. Imported mesh surfaces

**US-IMPORT-1.** *As a user, I import an external `.obj` triangle mesh (e.g. a lidar terrain or a
digitized contact) as a model surface, distinct from importing a project file.*

**US-IMPORT-2.** *As a user, I assign the imported surface a **role** — contact, erosion, vein-wall,
or topography — so the same mesh can play any stratigraphic part a fitted surface can.*

**US-IMPORT-3.** *As a user, I honor the mesh exactly inside its own footprint and optionally
**extend it smoothly to the domain box** beyond that footprint (a toggle, off by default, that warns
before extrapolating).*

**US-IMPORT-4.** *As a user, a large imported mesh is stored as an external file reference on
export (so my project JSON stays small) and re-fetched on load; a mesh with no external copy travels
inline so the project is self-contained.*

*(An imported mesh has no editable point/orientation data — its geometry is the mesh itself, so the
data modal and the points/normals overlay do not apply to it.)*

---

## 5. Lithologies, units, and volumes

**US-LITHO-1.** *As a user, I maintain a **lithology catalog**: Air and Unknown are fixed; bodies
(veins/intrusions) contribute an inferred entry (color-editable); and I can add my own named,
colored rock types.*

**US-LITHO-2.** *As a user, each derived rock **unit** exposes a lithology dropdown; I assign a
catalog lithology to it, and the assignment takes effect immediately (it recolors the solids without
a re-fit).*

**US-LITHO-3.** *As a user, I rename derived units for readability (the units themselves are derived
from the stack; only their display name is mine to set).*

**US-LITHO-4.** *As a user, adding a catalog entry makes it available in every unit's dropdown, so I
build up a consistent palette as I go.*

---

## 6. The 3D view

### 6.1 Render modes

**US-VIEW-1.** *As a user, I toggle between **Surfaces** (open contact sheets, colored by element)
and **Solids** (closed lithology-unit solids, colored by lithology), so I inspect either the
contacts or the filled rock volumes.*

### 6.2 Camera & framing

**US-VIEW-2.** *As a user, I orbit, pan, and zoom the model with the mouse; Z is up (geological
convention) so the view stays upright.*

**US-VIEW-3.** *As a user, I click **Fit view** to frame the camera on the data from
above-south-east, fit to the model extent.*

**US-VIEW-4.** *As a user, a labeled **X/Y/Z gnomon** (East/North/Up) sits just outside the model so
I always know my orientation.*

### 6.3 Inspection aids

**US-VIEW-5.** *As a user, I toggle a mesh **wireframe** (triangle edges) to inspect tessellation.*

**US-VIEW-6.** *As a user, I toggle a **points/normals overlay** that shows the authored contact
points (spheres) and surface normals (arrows), labeled by table row, for parametric surfaces — and I
scale the point, arrow, and label sizes independently so the overlay reads at any model size.
(Imported meshes have no such overlay.)*

**US-VIEW-7.** *As a user, I toggle a **model-bounds** wireframe box so I see the domain the model is
meshed within.*

**US-VIEW-8.** *(Diagnostic) As a user, I toggle individual rendered volumes on and off, one at a
time, to isolate what I'm looking at.*

### 6.4 Engine-diagnostic toggles (prototype)

These expose engine behavior for testing; they are not part of authoring the geology:

- **Clip seam** — chooses the cross-cut seam closer (legacy snap vs. mesh–mesh clip corefine);
  re-meshes on toggle.
- **Manifold** (Solids only, default on) — chooses the cross-cut volume mesher (the Manifold boolean
  engine vs. the legacy field-domain shared tree); re-meshes on toggle.

---

## 7. Projects: new, save, export, import

**US-PROJ-1.** *As a user, I start a **New** blank project, or **seed** one from a built-in template,
and I'm warned before discarding un-applied edits.*

**US-PROJ-2.** *As a user, my project **autosaves** to the browser so I don't lose work on reload.*

**US-PROJ-3.** *As a user, I **rename** the project.*

**US-PROJ-4.** *As a user, I **export** the project as a `.geoproj.json` file — including my current
**view** (camera + render toggles) — so I can archive it or hand it to someone else, and **import**
it later to restore both the model and exactly what I was looking at.*

**US-PROJ-5.** *As a user, I **export the rendered geometry as OBJ** (grouped by lithology in Solids
mode), available once the model has been applied and there is geometry in the scene (it is disabled
while edits are pending, so I only ever export the current, fit model).*

**US-PROJ-6.** *As a user, when I import a project whose large meshes were exported by reference, the
app re-fetches them and fails with a clear, path-naming error if a mesh is missing — rather than
loading a silently-blank model.*

---

## 8. Non-goals / out of scope

- **Not a production modeler.** No undo/redo history, no multi-user or cloud persistence, no units of
  measure / georeferencing / CRS handling.
- **No in-app engine tuning.** Kernel/fit parameters (range, nugget, decimation, etc.) are not
  user-exposed; the harness uses engine defaults.
- **No mesh editing.** Imported meshes are used as-is; the app does not edit their triangles, nor
  does it edit derived units' geometry (only their names and lithology assignments).
- **Diagnostic toggles are not authoring.** The clip-seam, manifold, wireframe, bounds, and
  per-volume-visibility controls exist to exercise/inspect the engine, not to define geology.
- **Seed templates are illustrative, not part of this spec.** They seed a starting project; the
  authoring model is what this document specifies.

---

## 9. Acceptance summary

The harness meets this spec when a user can, end to end:

1. Start from blank or a seed, set the model extent, and build an ordered stack mixing beds,
   cross-cutting bodies, topography, and imported meshes.
2. Edit surface data across several surfaces, then Apply once and watch it fit to "done" (or get a
   clear error), with a stale-scrim whenever edits are pending.
3. Inspect the result in both Surfaces and Solids modes, frame it, and use the overlay/wireframe/
   bounds aids to verify it.
4. Build a lithology palette and assign lithologies to units, recoloring solids live.
5. Export the project (with view) and OBJ geometry, then re-import the project and land on the same
   model and viewpoint.
