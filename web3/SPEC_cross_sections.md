# web3 — Spec fragment: Cross-sections & digitized surfaces

**Status:** proposed extension to `web3/SPEC.md` (prototype / testbed)
**Relationship:** additive. This fragment introduces two new sections that slot after the current
`SPEC.md §9`, and reuses — does not change — the existing edit-and-apply loop (`§2.1`), surface data
model (`§3.2`), render modes (`§6`), and flip/younging semantics (`US-DATA-2`). It makes concrete the
roadmap items **B7 (on-demand cross-sections)** and **B8 (polyline-on-section surface editing)**.
**Reference example:** Leapfrog Geo's *Serial Sections*, *Cross Sections / Fence Sections*, and the
*Polyline Editor* (draw on a slice locked to the section plane), plus its per-surface *younging
direction* — the workflow this fragment adapts.

---

## 10. Cross-section views

### 10.1 What a cross-section is (read this first)

A **cross-section** is a **vertical plane cut through the model**. Looking at it face-on gives a 2D
picture — the *traces* where the model's surfaces meet that plane — the way a geologist reads a
section drawing. It is both an **inspection aid** (see the layering along a line) and, in §11, a
**drawing surface** (digitize new geology directly on it).

A section plane is defined by a **baseline**: two points picked in plan (top-down) view, `A → B`.
The plane passes vertically through that line (**X = East, Y = North, Z = Up**, per `SPEC.md §1.2`),
spanning the full model Z-range. The baseline direction `A → B` is the section's **in-plane
horizontal axis (u)**; world **+Z** is its **in-plane vertical axis (w)**; the horizontal direction
perpendicular to the baseline, `s = u × w`, is the plane's **out-of-plane normal** (which way you are
"looking through" the section).

**Serial sections** are a set of **parallel** section planes: one baseline, then `N` copies offset
along `s` at a fixed **spacing**, optionally with **front/back offsets** bounding the fence. Parallel
sections are the substrate for §11 — you digitize the same surface on each and the app lofts them
into one 3D surface.

### 10.2 User stories — creating and viewing sections

**US-SECT-1.** *As a user, I create a cross-section by switching to top-down view and clicking two
points to set its baseline, so I frame a vertical slice through the model.* (Mirrors the two-corner
bounds gesture in `US-BOUNDS-1`.)

**US-SECT-2.** *As a user, I create a **serial set** of parallel sections from one baseline by
entering a spacing and a count (and optional front/back offsets), so I get evenly-spaced slices to
read and to digitize across.*

**US-SECT-3.** *As a user, I open a section in a **section view**: the camera locks orthographic and
face-on to the plane (u right, Z up), and I see the **traces** of the currently-applied model
surfaces where they intersect the plane, plus the authored contact points lying near the plane
(within a slab tolerance), as a backdrop.* The traces are the current fit intersected with the plane;
they carry the stale-scrim (`US-EDIT-2`) when edits are pending, exactly like the 3D view.

**US-SECT-4.** *As a user, I **step** between parallel sections in a serial set (next/previous)
without leaving section view, so I can compare and digitize slice by slice while keeping my tool
active.*

**US-SECT-5.** *As a user, I toggle a section's plane on/off in the 3D view as a translucent quad
with its baseline and normal shown, so I always know where the slice sits in space.*

**US-SECT-6 (diagnostic).** *As a user, the section view shares the render-mode toggle: it shows
surface **traces** (contact mode) or filled **unit intervals** (solid mode), so a section reads the
same geology the 3D view does.*

### 10.3 Notes & non-goals for sections

- Sections are **view/authoring constructs, not geology.** Creating, moving, or deleting a section
  never sets the project dirty and never triggers a re-fit — it changes only what is drawn. (Digitized
  *content* on a section, §11, is geometry and does set dirty.)
- Sections are **vertical planes only** in this fragment (baseline + full Z-range). Arbitrary tilted
  or folded (fence-polyline) sections are out of scope here.
- The saved **view** (`US-PROJ-4`) is extended to persist a project's sections (baselines, spacing,
  active section) so an exported project reopens on the same slices.

---

## 11. Digitizing surfaces on parallel sections

### 11.1 The idea (read this first)

On a section you **digitize a line** — an ordered chain of points traced along where a contact runs
in that slice. Do the same contact on several **parallel** sections and the app has a line per slice;
it **lofts those lines into one 3D surface** and adds it to the model as an ordinary parametric
surface (points + orientations feeding the same fit as any other surface — see `SPEC.md §3.2`).

Two things make a digitized line more than a set of points:

1. **The points are contacts.** Each vertex is a **contact point** at its world position on the
   section plane — the same kind of datum the data modal's points table holds. Several sections
   contribute their vertices to the **same** surface element.
2. **The direction you draw sets the younging (the surface normal).** The order of the vertices
   defines a **tangent** along the trace; combined with the section's out-of-plane normal `s`, that
   fixes which side of the surface is **younger** (equivalently, which way its **pole** points). This
   is how a hand-drawn line carries "up-section" information without the user typing dip numbers.

Concretely, walking the line in the drawn direction with the section's normal `s` pointing **toward
you** and **Z up**, the surface **youngs to your left**: the derived pole points to the younger
(hanging) side. The app shows this live as a **younging affordance** — an arrow along the line and
hachures/shading on the younger side — and it is **reversible** with the same flip toggle as
`US-DATA-2` (flipping never rewrites the drawn vertices; it only negates the derived pole). *(Left is
a fixed convention; the affordance means the user reads and flips it visually rather than reasoning
about the rule.)*

### 11.2 How a line becomes surface data (integration, informative)

This reuses the existing pipeline with **no new engine capability** required:

- Each digitized **vertex** `→` one entry in the target surface's **contact points** (`sp_xyz`).
- Each vertex (or each line segment) `→` one **orientation** whose pole is the in-plane normal
  `m = normalize(t × s)`, where `t` is the local tangent (drawn direction) and `s` the section
  normal; the drawn direction fixes `m`'s sign, and the per-surface **flip** bit negates it. The pole
  is handed to the fit exactly as `dipDirToPole` output is today (`project.ts`), so the digitized
  surface participates in `Model::fit` identically to a typed surface. (The full 3D dip is recovered
  by the co-kriging across the parallel sections; a single section constrains only the in-plane
  component, which is why §11.5 warns on one section.)
- On **Apply**, the lofted point+orientation set is fit and meshed with the rest of the stack — one
  pass, deferred like every geometry edit (`§2.1`). The resulting surface then takes any **role**
  (contact / erosion / onlap / vein-wall / topography) and stacks by its list position exactly like a
  fitted or imported surface.

### 11.3 User stories — digitizing

**US-DIG-1.** *As a user, in a section view I pick a **target surface** (an existing element, or a
new one I create for this purpose) and **digitize a line** on the plane: click to add ordered
vertices, with click-to-extend, drag-to-move, and delete-vertex, so I trace a contact where I see it
in the slice.*

**US-DIG-2.** *As a user, the **direction** I digitize sets the surface's **younging/normal**, shown
live as an arrow-and-hachure **younging affordance** on the drawn side; I can **flip** it (reversible,
never rewrites my vertices) if the younger side is the other way.* (Same semantics as `US-DATA-2`.)

**US-DIG-3.** *As a user, I digitize the **same** surface on several **parallel** sections — stepping
between slices (`US-SECT-4`) with the tool active — and each slice's line is added to that one
surface, so the collection of parallel traces defines a 3D surface.*

**US-DIG-4.** *As a user, on **Apply** the app lofts my digitized lines into a fitted surface and
meshes it in the stack, and I inspect the result in both the 3D view and the section views (my traces
should land on the new surface), with the stale-scrim shown until I Apply.*

**US-DIG-5.** *As a user, my digitized lines are **editable data**: I can reopen a surface, move or
delete vertices on any section, add a line on a new section, or re-digitize a slice — each is a
deferred geometry edit that re-fits only on the next Apply.* Digitized vertices also appear in the
surface's **points table** (`US-DATA-1`) and in the **points/normals overlay** (`US-VIEW-6`), so the
two authoring routes (type a table / draw on a section) are two views of the same contact data.

**US-DIG-6.** *As a user, I set the digitized surface's **role and relation** (contact/erosion/onlap,
vein-wall, topography; conformable/onlap/erode) like any other element, so a surface I drew composes
with beds, bodies, and imported meshes under the normal cut rules (`SPEC.md §3.1.2`).*

### 11.4 Validation (extends `SPEC.md §2.3`)

The panel validates digitized input and blocks Apply with the first blocking reason when it cannot
fit:

- **Hard (blocks Apply):** a line with fewer than 2 vertices; a surface whose entire digitized set
  lies on a **single** section plane *and* has no other data (a single coplanar trace cannot define a
  bounded 3D surface — see §11.5); a vertex that does not lie on its section plane (should be
  impossible via the tool, guarded on import).
- **Soft (warns, does not block):**
  - *Under-constrained loft:* all digitized lines lie on **one** section, or on **parallel** sections
    with **no across-section variation** — the fit is cylindrical/extrapolated off-section; warn that
    ≥2 sections (ideally 3) give a determined surface.
  - *Inconsistent younging:* one line's drawn direction implies the **opposite** younger side from its
    siblings on the same surface (the classic digitizing slip). Warn and offer "flip this line to
    match," since a mixed-younging point set fits a self-contradictory field.
  - *Sparse/!ordered:* a line whose vertices back-track or self-cross on the plane (an ambiguous
    tangent for the pole derivation).

### 11.5 Notes & non-goals

- **Reuses the engine as-is.** Deriving poles from in-section tangents can be done entirely UI-side
  (TypeScript) and passed through the existing `ori_xyz` / `ori_pole` ABI; the kernel and stack need
  no change. (An optional engine-side helper could centralize the derivation, but is not required.)
- **Plane-locked drawing only.** Digitizing is constrained to the active section plane; free 3D
  drawing on surfaces, and drawing on tilted/fence sections, are out of scope here (roadmap, later).
- **Manual digitizing only.** No auto-tracing from images, grids, or drillhole intercepts in this
  fragment (there is no drillhole data model yet — roadmap B11). A digitized line is authored by hand.
- **No mesh editing.** As in `SPEC.md §8`, this authors *input data* for the fit; it never edits the
  resulting triangles.
- A digitized surface is **indistinguishable downstream** from a typed one: same points/orientations,
  same fit, same stacking — so everything in `SPEC.md §3–§6` applies to it unchanged.

### 11.6 Acceptance

This fragment is met when a user can, end to end:

1. Create a serial set of parallel vertical sections through the model and open them face-on, seeing
   the current model's traces as a backdrop.
2. Digitize a contact line on each of several parallel sections into one target surface, with the
   drawn direction driving a visible younging affordance that can be flipped.
3. Apply once, watch the lines loft into a fitted surface that meshes in the stack (traces land back
   on the new surface in section view), with a stale-scrim while edits pend.
4. Give the digitized surface a role/relation and see it compose with the rest of the stack under the
   normal cut rules, and edit/redraw its lines as ordinary deferred geometry edits.
