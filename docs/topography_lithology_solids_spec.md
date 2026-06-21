# Topography, Lithology Catalog & Solid Volumes

**Status:** design spec / build brief
**Baseline:** the as-built (`surface_stack_as_built.md`) + the current `web3/` authoring layer and `crates/geofield/` engine.
**Scope:** the web3 testbed + the minimum Rust/ABI changes to support it. Faults out of scope.
**Audience:** Claude Code.
**Tags:** `[EXISTS]` already built · `[EXTENDS]` change to existing · `[NEW]` net-new.

---

## 0. One-paragraph summary

This step turns the surface model into a **named-volume model**. It adds a dedicated **Topography** surface that caps the whole model (its upper side is Air), makes **fill-upward the universal, non-optional default** (staged-deprecating `unbounded_above`), introduces a **lithology catalog** whose entries are partly **inferred** (a vein/intrusion named "Vein A" contributes a lithology "Vein A"; the floor and Air are distinguished members) and partly **user-added**, **assigns lithologies to the volumes** the field already produces (the `(element, unit)` slabs from `global_unit`/`present_unit_count`, named by the *unit*, never by the contact), reuses the **existing capped-Scene solid meshing** to emit **watertight per-volume solids** (Air excluded), and adds a **surface/solid render toggle** to the web3 harness (driving the `RenderMode` the protocol already carries).

---

## 1. The conceptual shift: units are named, contacts are not

The current UI keys off **interfaces** (a series owns `surfaces: InterfaceData[]`; the panel renders one row per surface). This step inverts the presentation to key off **units** — the volumetric slabs the surfaces bound.

- A conformable series with **N interfaces defines N+1 units**. We do **not** try to name units from contacts (a contact is between two units; naming a unit after one of its bounding contacts is arbitrary and wrong). Instead the **unit itself** is the named thing.
- The panel renders an **alternating ladder**, youngest-at-top:

  ```
   ┌ unit  "Topsoil"          [lithology ▾]   …       ← nameable, litho-assignable
   ├ ────  (contact)                          …       ← point/orientation entry only
   ├ unit  "Sandstone"        [lithology ▾]   …
   ├ ────  (contact)                          …
   ├ unit  "Shale"            [lithology ▾]   …
   ├ ────  (contact)                          …
   └ unit  "Basement"         [lithology ▾]   …
  ```

- **Units are derived from the core, not authored.** The user cannot add/remove a unit directly — they add/remove **contacts** (interfaces), and the units re-derive (N interfaces → N+1 units, minus whatever clipping removes). The user only **names** and **assigns lithology to** the units the geometry produces. This is the honest expression of "we can't define the unit count, so we don't try": the UI never asserts how many units exist; it labels whatever the field yields.

The contact row keeps today's behaviour: its "…" opens the point/orientation modal; **no rename** (a contact has no meaningful user-facing name).

---

## 2. Topography — a dedicated surface that caps the model `[NEW]`

### 2.1 Rust: a dedicated `StackElement` variant

Topography is **not** a flagged youngest series; it is its own variant so its invariants are structural, not conventional:

```rust
enum StackElement {
    Series(StackedSeries),
    Vein(Vein),
    Intrusion(Intrusion),
    Topography(Topography),   // NEW — singleton, pinned youngest, never reorderable
}

struct Topography {
    model: Model,             // one co-kriged field, single interface (the ground surface)
    // upper side is ALWAYS Air; lower side is "everything below, present"
    // reuses the Erode clip mechanism internally (caps all older series)
}
```

Invariants the engine enforces (not the user):
- **At most one** `Topography` in a stack.
- **Always index 0** (youngest); cannot be reordered below any element; the reorder API refuses to move anything above it.
- Its clip is the **erode** half-space already implemented (`Masker::Halfspace`): every older element is clipped to *below* topography. This is mechanically the youngest erosional surface — the as-built already notes erosion is "younger truncates older," and topography is exactly that with the upper unit fixed to Air.
- Its **upper unit is Air** (§4.2), which is never meshed as a solid (§5).

### 2.2 Lithology presence with topography

`global_unit` is unchanged in spirit (youngest-present-wins) with one addition: above the topography surface, the result is the **Air** unit (no rock). Below it, the existing evaluation proceeds. Because topography is youngest and erodes, it naturally caps every series' upward fill at the ground — which is the whole point (see §3).

### 2.3 Authoring

Topography is authored like any other surface — XYZ points + optional orientations in the existing modal. In the panel it renders as a single contact `----` at the very top with the **Air** unit above it (Air shown greyed, not lithology-assignable, not renameable). Add it via the type picker as a new "Topography" choice; adding a second is disallowed (the picker hides/disables it when one exists).

---

## 3. Fill-upward becomes universal; `unbounded_above` is deprecated `[EXTENDS]`

### 3.1 The new default

Every series **fills upward by default** — present below its top contact and continuing upward until something younger clips it. Topography is what clips the exposed tops to the ground; younger erosions/units clip the buried ones. "Open sky vs rock" is therefore decided by **surfaces** (topography, younger contacts), never by a per-series flag. This removes the artifact where the domain-box ceiling stood in for topography.

### 3.2 Staged deprecation (the safe path, not a hard rip-out)

Do **not** delete `unbounded_above` from the data model/ABI in this step. Instead neutralize it in stages so existing serialized projects and the Rust test suite stay green:

1. **Engine:** force fill-upward behaviour for all series regardless of the flag. Concretely, `Stack::global_unit` and `Stack::present_unit_count` stop reading `unbounded_above`; the "keep vs drop top unit" decision is now derived from **whether a younger element (including topography) actually caps the series at that point**, not from the flag. The floor's top unit is kept because nothing younger caps it there; an exposed series' top unit is kept where topography is above it and dropped (→ Air/void) where topography cuts below it. This makes `present_unit_count`'s old branch fall out of the clip result.
2. **ABI/protocol/project:** keep the `unboundedAbove` slot in `SeriesJson` / `add_series` / `project.ts` **accepted but ignored** (document it as deprecated). Existing JSON scenarios still parse.
3. **UI:** **hide** the "fills upward (floor)" checkbox. The control is gone from the panel; the field is no longer authorable.
4. **Tests:** the existing tests that pin `present_unit_count` keep passing because the *behaviour* (floor keeps its top unit; bounded series drop theirs where capped) is preserved — it's now driven by the clip rather than the flag. Add a test that a series with `unbounded_above=false` **and** no younger cap still fills upward (the new universal default), proving the flag is inert.

Removal of the slot from the ABI is a **later** cleanup once nothing reads it; this step only makes it inert and invisible. (Rationale: a half-removed flag mid-refactor is worse than either end; staged-inert is the safe middle.)

> Note for reviewers: this is the change the as-built foresaw — `unbounded_above` was a one-bit shadow of a missing topographic surface. Adding topography supplies the surface; the flag becomes redundant and is retired behavior-first, slot-later.

---

## 4. Lithology catalog `[NEW]`

### 4.1 Model (lives in `web3/project.ts`; live-only, serialized into the downloadable scenario JSON)

```ts
export type LithologyId = string;

export interface Lithology {
  id: LithologyId;
  name: string;
  color: string;           // hex; drives solid mesh colour + swatch
  kind: "air" | "inferred" | "user";  // provenance (controls editability)
}

export interface LithologyCatalog {
  lithologies: Lithology[];          // includes the distinguished Air member
  // assignment: which catalog entry fills each derived volume.
  // keyed on (elementId, unitIndex) — the core's volume coordinate.
  assignment: Record<string, LithologyId>;  // key = `${elementId}:${unitIndex}`
}
```

Persistence: **live only** (in-memory), but **serialized into the project JSON** on export and restored on import (so a downloaded scenario round-trips its names/colours/assignments). No separate persistence.

### 4.2 Air — the distinguished member `[NEW]`

- A single **non-deletable, non-renameable** catalog entry `{ id: "air", name: "Air", kind: "air" }`, rendered transparent.
- It is the lithology of every volume **above topography** (and of any uncapped void).
- It is **never meshed as a solid** (§5).
- The UI shows it as a fixed greyed row in the catalog; its assignment is automatic (topography's upper unit), not user-editable.

### 4.3 Inferred entries `[NEW]`

Inference is **only** done where it is unambiguous — i.e. for the cross-cutting bodies whose volume *is* the named object:

- A **Vein** named "Vein A" contributes an inferred lithology "Vein A" (`kind: "inferred"`), auto-assigned to that vein's interior volume.
- An **Intrusion** named "Intrusion Bob" contributes "Intrusion Bob," auto-assigned to its interior.
- Renaming the body renames (or re-syncs) its inferred lithology; deleting the body removes it.

We do **not** infer names for series-internal units (a series' N+1 units have no natural names — §1). Those start **unassigned** (default to a placeholder that maps to Air-equivalent "unassigned/void" until the user names them, OR to a neutral "Unknown" member — see §4.5).

### 4.4 User-added entries `[NEW]`

The catalog UI (a small panel/section) lets the user **add** lithologies (name + colour, `kind: "user"`), **rename/recolour** user + inferred entries, and **delete** user entries (inferred ones are deleted via their body; Air is fixed). Added entries become available in every unit row's lithology dropdown.

### 4.5 "Unknown" default

Mirror Leapfrog: series-internal units default to a distinguished **"Unknown"** lithology (a fixed catalog member, `kind: "user"`-like but non-deletable, neutral grey) until the user assigns something. Unknown **is** meshed as a solid (unlike Air) — it's rock of unspecified type, not void. This keeps the model never-blocked: every below-topography volume has *a* lithology (Unknown at worst), every above-topography volume is Air.

---

## 5. Assigning lithologies to volumes & solid meshing `[EXTENDS]`

### 5.1 Volumes are the core's `(element, unit)` cells

The core already enumerates units (`present_unit_count`, `global_unit`). The derived `CoreUpdate` must expose, per element, its **ordered unit list with stable unit indices**, so the UI can render the ladder (§1) and key assignments on `(elementId, unitIndex)`. Add to `DerivedView`:

```ts
interface DerivedView {
  // …existing…
  units: [string /*elementId*/, UnitInfo[]][];   // NEW: ordered units per element
}
interface UnitInfo { index: number; /* youngest→oldest within the element */ }
```

(The core knows nothing about lithology *names* — it provides the unit *coordinates*; the catalog/assignment is the UI's, compiled back in for colouring.)

### 5.2 Solid meshing already exists — reuse it `[EXISTS]`

The as-built states solids are already produced via the same `octree_dual_contour_scene` path with `cap = true` (closed unit solids), and the non-convex `SlabComplement` path already keeps vein/shell volumes watertight. **Do not re-architect meshing.** A volume meshes as the existing capped Scene for that `(element, unit)` cell, with the masker set the surface path already assembles **plus topography as a universal top masker**. Cross-cutting interiors remain exclusions for the series volumes they punch through (already handled by `cuts_below` + the masker machinery).

The only additions:
- **Topography participates as the top masker** for every exposed volume (it's the youngest erode surface — already the mechanism).
- **Air volumes are skipped** — the solid mesher emits nothing for the above-topography unit.
- Each emitted solid carries its **lithology id** (from the assignment) so the viewer can colour/group by lithology (§6), replacing the current colour-by-series-index.

> If a multi-clip volume (a unit capped by topography above, its floor contact below, and punched by an intrusion) exposes a watertightness gap, that is the case to test — but the precedent (`SlabComplement`, capped Scenes) suggests it's covered. Don't pre-solve; add a watertightness assertion test for the topography-capped + intrusion-punched volume.

### 5.3 Compile path

`project.ts::toSeriesJson` (the single seam) gains: emit the topography element; emit the lithology assignment alongside the series (or as a parallel `assignment` payload the worker forwards), so the solid mesh output can be tagged by lithology id. The worker's solid-mode mesh collection tags each `SurfacePayload`/solid with its lithology id for the viewer.

---

## 6. web3 harness: surface/solid render toggle `[EXTENDS]`

`protocol.ts` already carries `RenderMode = "contact" | "solid"`. Wire it:

- A **toggle** in the project bar: "Surfaces ⇄ Solids."
- **Surfaces** (today's default): open contact sheets, coloured by series (unchanged).
- **Solids**: closed watertight unit solids, **coloured by lithology** (from the catalog), **Air omitted**. Uses `cap = true` and the lithology-tagged output (§5.2).
- The toggle re-runs the mesh collection in the chosen mode (no re-fit needed — same fitted stack, different mesh extraction). If cheap enough, switch without refit; otherwise treat as a normal apply.
- Viewer (`Viewer.tsx`): accept a `colorBy: "series" | "lithology"` and a per-surface/solid lithology id; in solid mode colour from the catalog's hex, skip Air. OBJ export (existing) groups by lithology in solid mode.

---

## 7. UI inventory (web3)

**New:**
- `LithologyCatalog.tsx` — the catalog panel: list (Air fixed, Unknown fixed, inferred from bodies, user-added), add/rename/recolour/delete (per provenance rules).
- Unit-row rendering in `HierarchyPanel.tsx` — the alternating unit/contact ladder (§1), unit "…" → rename + lithology dropdown; contact "…" → existing point modal.
- Topography choice in `AddSurfaceMenu.tsx` (disabled when one exists).
- Render-mode toggle + `colorBy` plumbing.

**Changed:**
- `HierarchyPanel.tsx` — render units (from `derived.units`) interleaved with contacts, instead of one row per interface. Seam control still on the boundary below a series; vein/intrusion still show the cross-cuts chip. **Hide** the "fills upward (floor)" checkbox.
- `project.ts` — `LithologyCatalog` type + assignment, serialize into project JSON; `toSeriesJson` emits topography + assignment; inferred-litho sync on body add/rename/delete; `unboundedAbove` retained-but-ignored.
- `dto.ts` — add `units` to `DerivedView`; lithology types if any cross the boundary (assignment stays UI-side; only unit coordinates come from core).
- `Viewer.tsx` — `colorBy`, lithology colours, skip Air, group OBJ by lithology in solid mode.
- `protocol.ts` / `worker.ts` — topography element; solid-mode mesh collection tags lithology ids; forward the assignment if the worker needs it for tagging.

**Rust (`crates/geofield/`):**
- `StackElement::Topography` variant + invariants (singleton, youngest, erode clip, Air above).
- `global_unit` / `present_unit_count` derive top-unit keep/drop from clipping (incl. topography) instead of `unbounded_above`.
- `DerivedView`/core-update JSON gains the per-element ordered unit list.
- Solid meshing: topography as universal top masker; skip Air; tag solids with the unit coordinate (lithology id applied UI-side, or passed through if assignment is forwarded).

---

## 8. Minimal correlative assignment (G) `[NEW]`

The per-unit lithology dropdown lets the user point **multiple `(element, unit)` cells at the same catalog entry** (e.g. two separated sandstone slabs both → "Sandstone"). That's the whole mechanism — a dropdown per unit row, many-to-one onto the catalog. **No automatic correlation detection.** This is the one thing that must stay explicit (the geometry can't know two slabs are the same formation), and it's exactly what a *catalog* (vs per-cell names) buys.

---

## 9. v1 scope & non-goals

**In scope:** topography variant + universal fill-upward (deprecating the flag behaviour-first); lithology catalog (Air + Unknown fixed, inferred bodies, user-added); per-unit assignment incl. many-to-one; solid meshing reuse with Air omitted + lithology colouring; surface/solid toggle; OBJ-by-lithology in solid mode.

**Non-goals:**
- **Faults / block model** — still out.
- **Hard removal of `unbounded_above` from the ABI** — deprecated/inert now, slot removed later.
- **Drillhole-label inheritance** — not in this step (units start Unknown; user names them). Can layer in later as an inference source.
- **Surface-as-entity / correlative conformity refactor** — still future; this step does not lift surfaces to shared referenced entities. (It does, however, retire `unbounded_above`, removing one of the two bits that future work needs to reconcile.)
- **Persistent storage** — live-only + project-JSON serialization; no localStorage/db.
- **Re-architecting the mesher** — reuse the capped-Scene path verbatim.

---

## 10. Why this is the right increment (for reviewers)

It closes the gap the as-built named: "open sky vs rock is defined by a topographic surface," not by a per-series flag. Adding topography as a dedicated youngest-erode variant supplies that surface, makes fill-upward the universal default, and lets `unbounded_above` be retired behaviour-first. It promotes the model from surfaces to **named volumes** by attaching identity to the `(element, unit)` cells the field already produces — naming the *units*, never the contacts, because a conformable series' N contacts genuinely cannot name its N+1 units. The catalog is inferred where it can be (a body's interior is its namesake lithology) and explicit where it must be (Air, Unknown, user rock types, and the correlative many-to-one). Solids reuse the existing capped-Scene mesher with Air omitted, and the harness toggle exercises the whole thing — surfaces coloured by series, solids coloured by lithology — without touching the fit. No faults, no mesher rewrite, no ABI break: the smallest step that yields watertight, lithology-labelled volumes.

---

## 11. As-built notes (implemented 2026-06-20 — corrections to the brief above)

The feature shipped behind the constitutional gates (65 native Rust tests incl. a multi-clip
watertightness invariant; 35 Playwright specs; wasm32 build + shared-impl native↔wasm parity).
Four places where the implementation **corrected the brief's model** — the code is right and
this section is the record (Principle II):

1. **`present_unit_count` keep/drop was net-new, and simpler than §3.2 implied.** The brief
   said making `unbounded_above` inert "falls out of the clip result." It does not — the old
   `present_unit_count` read the flag with NO clip-awareness. The honest rule turned out to be
   the simplest: **universal fill-upward → every series keeps ALL its units**
   (`present_unit_count == block_unit_count`); the `n-1` "drop the void unit" branch is deleted,
   and clipping (topography + younger erosions, already in `clip_maskers`) removes the non-rock
   parts. The floor's long-standing keep-all became every series' behaviour.

2. **`unbounded_above` is inert in the unit COUNT, but retained in `global_unit`** as the
   floor's fill-upward marker. Removing it entirely there breaks the floor's fill-upward in a
   topography-less stack (a series' `present_at` is `field < iso_top`, which does not fill
   upward). Retiring that last use belongs to the deferred surface-as-entity refactor. §3.2's
   "behaviour-first deprecation" holds for the count (what mattered); the flag's geometric role
   in `global_unit` is a smaller, separate retirement.

3. **No Rust reorder API exists** — the singleton/youngest topography invariant is split: the
   `add_topography` builder rejects a 2nd and inserts at index 0; the UI resolver + the
   `toSeriesJson` compile seam (emit topography first) keep it youngest. The engine enforces
   "at most one, at index 0"; the UI enforces "can't be dragged below."

4. **Topography meshes through the generic non-series surface path** (`mesh_cross_cut_surface`),
   not a new mesher — its single ground contact is an open sheet clipped to the box, identical
   to how a vein/intrusion wall meshes. The worker routes topography there.

**Coverage bounds logged (Principle IV):**
- **Render toggle re-FITS, not re-meshes.** The worker is terminate-and-refit per run (the
   existing hard-cancel design); keeping a fitted `WasmStack` alive across messages is a larger
   change deferred. The fit is deterministic so the result is identical — only the (one-time)
   cost differs. Lithology/metadata edits (assignment, catalog, unit names) do NOT set the
   geometry-dirty flag and, in solid mode, trigger a re-mesh (still a refit, per the above)
   rather than gating behind Apply.
- **Seed surfaces use 2 points + 1 orientation** (not 1): a single surface point leaves the
   dual-kriging system ill-conditioned (|grad| ~2e10 → the octree collapses the sheet to a
   vertical sliver). Two points pin the scale (|grad| ~1e-3). The author-time validator's
   "≥3 points to fit well" advisory is geologically sound and still fires on these seeds.

### 11.1 Solid-view fixes (post-review, same day)

Three correctness bugs in the first solid-view cut, found by inspecting the rendered scene and
fixed:

1. **Unit ladder was inverted vs. the 3D stack.** The mesher's unit index 0 is the BOTTOM slab
   (`isovalues` ascending), but `DerivedView.units` emitted ascending and the ladder rendered
   index 0 at the top — so "Unit 1" (top of panel) coloured the bottom slab. Fix: `units` is
   now emitted **top-first** (descending index) for the ladder, while `UnitInfo.index` stays
   the stable bottom-0 mesher coordinate (the assignment key). The default label uses the
   visual ladder position, so "Unit 1" is the top slab.

2. **Sub-cell gap between adjacent unit solids.** The `Stack`-level `mesh_one_block_unit_solid`
   meshed each unit's caps independently, so dual-contour quantization left a fraction-of-a-cell
   gap at a shared contact (a visible horizontal seam). Fix: lifted the canonical-contact WELD
   from `Model::mesh_block_unit_solids` into the stack-clipped path — adjacent units snap their
   shared-contact caps onto the same canonical surface. (A stale wasm artifact initially masked
   the fix; a clean `wasm:engine` rebuild was needed.)

3. **Cross-cutting bodies had no closed solid.** `collectSolids` skipped veins/intrusions
   (the per-unit path is series-only), so an assigned vein showed nothing in solid mode. Fix:
   added `Stack::mesh_cross_cut_solid` — the body's closed interior (vein/shell band
   `lo < field < hi`, or single-envelope `field < iso`), capped by the box + younger clips —
   exposed as `mesh_cross_cut_solid_octree` and meshed by the worker (tagged at unit index 0,
   matching the inferred-lithology auto-assignment). Watertight (its own invariant test).
