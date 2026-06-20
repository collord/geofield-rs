# Surface stack & interaction logic — as built

**Status:** as-built description of what is *currently implemented* in `crates/geofield/`
(the BSD engine) and `web3/` (the authoring UI), as of the vein-clip / onlap-clip /
grid-graze fixes. This is a description of reality, not a spec — where it and the design
docs (`docs/stack_hierarchy_spec.md`, `docs/contact_grammar.docx`) disagree on intent, the
spec states the goal and this states what the code does. Source of truth for behaviour is
the Rust in `crates/geofield/src/stack.rs` + `octree.rs` and the tests there.

This document exists partly to record one thing the model **does not** have (a per-surface
"top of unit / bottom of unit" role) and why the controls that look like they might encode
it actually don't — see §6.

---

## 1. The core primitive: a series is one co-kriged field

A **series** is a single cokriging field (`crate::Model`) carrying `N` interface isovalues.
There is no global multi-field solve — each series is fitted independently
(`Model::fit`), and the stack is an *ordered list* of these fields.

- A series' "units" are the slabs between consecutive level sets of its own field.
- "Up/down" within a series is defined entirely by the **sign of that field**, by
  convention: `field < iso` is the side where the series' rock is present ("below" the
  contact). A series is *present* where `field < iso_top`, where `iso_top` is its highest
  (top) interface isovalue.
- The only knob that re-orients a field is the per-interface **flip** bit (negates the
  orientation pole at compile time — `project.ts::flippedDirToPole` / `InterfaceData.flipped`).
  It changes which physical side is "below"; it does not introduce a top/bottom-of-unit role.

A `Stack` is `Vec<StackElement>`, **youngest first (index 0) → oldest last**. A
`StackElement` is one of: `Series`, `Vein`, or `Intrusion` (the latter two are
cross-cutting bodies, §5).

## 2. The two declared axes of a series

Each `Series` carries exactly two free bits (everything else is derived). They are
*independent* — this separation is deliberate; the old `Relation::Basement` variant
conflated them and bred a latent bug.

### Axis A — `relation: Option<Relation>` (how it clips the series BELOW it)

`Erode | Onlap | Conformable`, or `None` for the floor (no series below ⇒ no clip to
declare). This is the one free bit per stratigraphic boundary. See §3.

### Axis B — `unbounded_above: bool` (does it stop at its top contact, or fill upward)

The web3 control labelled **"fills upward (floor)"**. It answers: is this series bounded
by its own top contact, or does it fill upward until a younger series claims the space?
It is **NOT** a surface polarity flag and **NOT** a relation. It changes three things:

1. **Lithology presence** (`Stack::global_unit`): an `unbounded_above` series is treated
   as present *everywhere not already claimed by a younger series* — it does not stop at
   its top contact. A bounded series is present only `field < iso_top`.
2. **Which units render** (`Stack::present_unit_count`): a bounded series **drops its
   topmost unit** (the void above its bounding contact, owned by a younger series); a
   floor series **keeps every unit** (its top interface is an internal subdivision, not a
   bound — the slab above it is real rock that daylights against younger erosions / the
   box top). Dropping it on a floor would leave a hole.
3. **Floor convention**: the oldest series should normally be the floor so the bottom of
   the model is solid.

Practical heuristic: check it for the **basement / oldest** unit; leave it off for any
series that should be capped above its own top contact. Nothing *enforces* that only the
oldest series has it — checking it mid-stack gives fill-upward behaviour there, which the
"(floor)" label implies but the mechanism does not require. (Known label ambiguity.)

## 3. Erode vs. onlap vs. conformable — the clip directions (as implemented)

Lithology (`global_unit`) is **youngest-present-wins** and is identical for all relations.
The relations differ only in **surface clipping**, expressed as `Masker`s collected by
`Stack::clip_maskers(i)` and ANDed into element `i`'s `Scene` as `Boundary` contributors.
There are **two clip directions**, mirror images of each other:

| Relation | Direction | What is clipped | Survives where |
|---|---|---|---|
| **Erode** (younger) | younger → older (up the stack) | the OLDER series' contact | `field_younger > iso_top` (above/outside the eroding body) |
| **Onlap** (younger) | older → younger (down the stack) | the ONLAP surface ITSELF | `field_older > iso_older` for every older series it crosses (its depositional side) |
| **Conformable** | none | nothing | — |

- **Erode**: a younger erode series cuts everything older where it is present; the older
  contact survives only on the un-eroded side. (Masker added to the *older* series' Scene.)
- **Onlap**: the onlapping (younger) surface laps onto and **pinches out against** the older
  topography — it is clipped to its depositional side and terminates along its intersection
  with each older surface. The older beds are NOT cut. (Masker added to the *onlap* series'
  own Scene, sourced from every older series.) This is the recent change: onlap was
  previously a pure no-op (neither surface clipped, full interpenetration); it now clips the
  onlapping surface. A clipped onlap surface legitimately has a free perimeter along its
  pinch-out crease — it is an open sheet that ends there, not a tear (asserted by
  `onlap_surface_pinches_out_without_tearing`; the older bed stays strictly watertight).
- **Conformable** (and the floor, `relation: None`): never clips.

The discriminator `onlap_vs_erode_differ` pins that erode and onlap produce different
older/younger meshes; if both clipped identically they would be indistinguishable.

Lineage: this is the Lajaunie–Calcagno potential-field formalism (erode truncates the side
it reaches; onlap laps against and pinches out), the same asymmetry as GemPy's
`ERODE`/`ONLAP`, LoopStructural's `add_unconformity` vs `add_onlap_unconformity`, and
Leapfrog's Erosion-cuts / Deposit-doesn't toggle.

## 4. Meshing path

Each per-series, per-interface surface is meshed as a **single `Scene`** — the series'
contact at one isovalue, intersected with the domain box and every applicable `Masker`
boundary — contoured by `octree_dual_contour_scene` in one watertight pass (no separate
contour algorithm). Open sheets for the viewer (`cap = false`); closed unit solids for the
solid view (`cap = true`). Maskers are `Boundary` contributors ANDed via
`Scene::contributors_inside`.

`Masker` kinds (the "where element `i` survives" descriptor):
- `Halfspace { iso, below }` — one side of a contact (erode top; intrusion-single exterior;
  onlap's "above each older contact").
- `SlabExterior { lo, hi }` — OUTSIDE a band (`field < lo || field > hi`), realised by the
  `SlabComplement` boundary. Used by a **vein** / shell intrusion: "outside the band" is a
  *union*, which ANDed half-spaces cannot express — ANDing `above-lo` with `below-hi` yields
  the band INTERIOR, which was the historical inverted-clip bug (older beds kept *between*
  the walls instead of outside). `SlabComplement` expresses the union as one contributor and
  participates in the double-crossed-edge refinement (`edge_crossing_count`) so the
  non-convex region stays watertight.

A degenerate robustness note: when a contact runs exactly along a grid plane,
`field − iso` at that corner is at the floating-point round-off floor and its discrete sign
differed between native and wasm builds (a parity violation that flung a dual-contour vertex
to the domain ceiling). `sample_corners` snaps such near-zero corners to a fixed side
(`CONTACT_GRAZE_EPS`) so the contour is identical across builds.

## 5. Cross-cutting bodies (vein / intrusion)

A **vein** is a 2-interface body present in the *band* between its foot-wall and hanging-wall
isovalues; an **intrusion** is present inside its envelope (`Single` contact, or a `Shell`
band). Both **cross-cut**: they clip everything older they intersect, but bounded to their
band/interior (not a half-space). Their cut-set (`cuts_below`) is *derived* from chronology +
sampled spatial overlap, never declared.

Their own walls now mesh (`Stack::mesh_cross_cut_surface` / the wasm
`mesh_cross_cut_surface_octree` binding; the web3 worker routes elements with a `crossCut`
tag to it). This lifted a feature-001 logged punt ("the body's own 3D surface mesh is a
follow-up — `mesh_one_block` is series-only"). Walls render as open sheets, matching the
stratigraphic contact view.

## 6. What is NOT modelled: per-surface "top of unit / bottom of unit"

There is **no concept of a surface being "the top of unit X" or "the bottom of unit Y."** A
unit is never defined as "the rock between a declared top surface and a declared bottom
surface." Instead a unit's extent is computed from (a) youngest-present-wins ownership and
(b) the slabs between a series field's own level sets. Polarity is implicit in the field's
sign convention and only nudgeable via the per-interface `flipped` bit.

This matters because `unbounded_above` ("fills upward (floor)") *looks* like it might be a
top/bottom-of-unit role and is not — and asking whether the two collide surfaced a real
modelling gap. The three things that sound entangled are actually distinct axes, and the
engine has only the first two:

1. `relation` (Axis A) — how a series clips the one below it.
2. `unbounded_above` (Axis B) — does a series fill upward or stop at its top contact.
3. surface polarity / top-vs-bottom-of-unit — **does not exist as a declared concept**;
   implicit in field sign, nudgeable only via `flipped`.

They do not *collide* in the current code only because #3 is absent. But #2 leans on a
global "up" that an explicit per-surface top/bottom role would make first-class — so #2 and
#3 *would* overlap if #3 were ever added. The spec already names this direction as future
work: lift surfaces to **referenced entities** so one surface can be simultaneously "the
erode top of the stack below AND the onlap floor of the stack above" (correlative
conformity) — the surface-as-entity model in which top/bottom roles become explicit. The
current code deliberately punts that; if it is built, `unbounded_above` and the field-sign
polarity convention must be reconciled with the new role rather than left as a third,
partly-redundant bit.

## 7. Authoring layer (web3) — the interaction mapping

The browser holds a client-side **Project document** (`project.ts`) that compiles to the
existing `SeriesJson[]` via a single seam (`toSeriesJson`); the core stays the source of
truth for all geology and is read back via `CoreUpdate.derived`. The panel
(`HierarchyPanel.tsx`) renders elements youngest-at-top.

Controls and what they set:
- **erode / onlap / conformable** seam control → `relation` (Axis A), on the boundary
  *below* a series. Shown only for a bounded, non-floor series.
- **"fills upward (floor)"** checkbox → `unbounded_above` (Axis B). §2.
- **Flip orientation** (per-surface "…" menu) → `InterfaceData.flipped`, negates the
  orientation pole.
- **▲/▼ reorder buttons** (per top-level element) → chronology reorder via
  `reorderContainers`. (Drag-and-drop scaffolding exists — `DndContext`, the drop-indicator
  CSS — but no node is registered draggable; the buttons are the working affordance.)
- A **vein/intrusion** shows a read-only "cross-cuts" chip (derived `effectiveRelation =
  crossCut`), not the 3-state seam control.

The single source of truth for the erode/onlap/cross-cut grammar is Rust; the UI never
recomputes geology — it only declares state and displays `CoreUpdate.derived`.
