# Wiring `mesh_clip` into `stack.rs` (incremental, flag-gated)

**Status:** plan, pre-implementation. Decided with the user 2026-06-22: `clip` replaces the
field-based contact clip (not just the seam snap), applied **per wall**, sequenced **incrementally
behind a flag** with the snap path as fallback. This note pins the shape before any `stack.rs` edit.

## The integration shape (what we learned from the map)

The cut contact **C** is currently *field-clipped* to the band exterior by a `SlabExterior` /
`Halfspace` masker in `mesh_cross_cut_shared` (`stack.rs:924–933`), then its free boundary is
**snapped** onto the wall (`stack.rs:966`) to close sub-cell see-through dashes. Two consequences
for wiring `clip`:

1. **`clip` trims C itself**, so to use it we must contour C *unclipped* (box-only, no body masker)
   and let `clip` do the carving. Keeping the field-clip AND `clip` would double-clip.
2. **A vein is a band (two walls: foot + hanging)**, so a cut contact terminates against BOTH. The
   `clip` API is one wall + one `KeepSide`, so a band needs **two `clip` calls** in sequence
   (`C` → clip vs foot wall → clip vs hanging wall), each keeping the band-exterior side.

The guarding tests (`welded_contact_and_wall_seam_is_watertight`,
`no_see_through_dashes_at_the_contact_wall_seam`, `shared_crease_closes_the_seam`) assert via
**merge-then-count-interior-open-edges**, NOT raw "incidence == 2". So the clip's intentional 3-way
seam junction (C terminates ON W) is COMPATIBLE — those tests want zero see-through holes, which a
clean clip delivers. (Verified by reading the test bodies: they weld within `cell*0.1` and count
incidence-1 interior edges.)

## KeepSide derivation (per wall)

`clip(C, W, keep, None)` keeps the side of W its CCW normal points into (`Above`) or away from
(`Below`). The wall meshes here are contoured with normals toward **increasing body field**.
The masker tells us where C survives:

| Body kind (younger, clipping C) | C survives where | Wall(s) | KeepSide per wall |
|---|---|---|---|
| Intrusion-single (envelope at `iso`) | `field > iso` (exterior) | the one envelope wall | side the normal points toward → **Above** |
| Vein / shell (band `[lo,hi]`) | `field < lo` OR `field > hi` (exterior) | foot wall (`lo`), hanging wall (`hi`) | foot: keep `field < lo` = **Below**; hanging: keep `field > hi` = **Above** |
| Erode (top contact `iso`) | `field > iso` (above) | the top-contact wall | **Above** |

The band is the union of two half-spaces, expressed as two sequential single-side clips: clip C to
`field < lo` (Below the foot wall) on the foot side, AND separately keep the `field > hi` piece
(Above the hanging wall). Because C-exterior-of-band is `< lo ∪ > hi` (disconnected), the correct
composition is: the surviving C = (C clipped Below foot) ∪ (C clipped Above hanging) — NOT a single
chained clip (chaining would intersect, giving the empty band-interior). **Two independent clips,
union the kept C's, with the seam shared on each wall.** (This is the one subtlety the two-wall step
must get right; the single-wall step avoids it entirely.)

## Sequence (each step verified before the next; snap is fallback throughout)

A `SeamMode { Snap, Clip }` parameter selects the path; `Snap` is the unchanged default so nothing
regresses until a step is proven.

- **Step 1 — single-wall contacts. ✅ DONE (`clip_seam_single_wall_is_watertight`, green; full
  suite 87 pass; wasm32 verified).** `SeamMode { Snap (default), Clip }` selects the path.
  `use_clip = Clip && n_walls == 1`. **Key confirmed learning:** the cut contact MUST be contoured
  box-only (no body masker) on the `Clip` branch — a *field-clipped* contact ends ~a cell short of
  the wall and never crosses it, so `clip` finds NO seam (shared=0) and the dashes survive. (Tried
  the shortcut of clipping the already-clipped contact first; it failed exactly this way — 45 dashes
  on the short-ending contact, 0 on the overshooting one.) Fix: condition `contact_scenes` to skip
  `cut_maskers` when `use_clip`, so C spans the wall; `clip(C, wall, KeepSide::Above, None)` then
  trims + shares. Fixture: `Intrusion::Single` over a two-contact column (no single-wall cross-cut
  fixture existed — all prior tests are the two-wall vein). Snap still serves veins.
- **Step 2 — two-wall bands (veins/shells). ✅ DONE (`clip_seam_vein_band_is_watertight`, green;
  full suite 88 pass; wasm32 verified).** `use_clip = Clip && (n_walls == 1 || n_walls == 2)`. The
  band exterior `field < lo ∪ field > hi` is the UNION of two single-side clips — clip the box-only
  C against each wall and concatenate the kept pieces (disjoint, so no merge needed); each piece
  carries its own shared seam. **`KeepSide` is derived from the BODY FIELD, not an assumed winding**
  (`exterior_keep_side`: probe a short step along the wall's CCW normal, keep the side where the
  field is band-exterior). Test asserts both band sides survive (the one-sided-erasure bug stays
  dead) AND no see-through dashes. ≥3-wall bodies still fall to `Snap`.
- **Step 2.5 — in-harness A/B toggle. ✅ DONE.** Before flipping the default, the `Clip` path is
  reachable from web3 via a visible **"clip seam"** checkbox (`data-testid=seam-clip-toggle`, next to
  the wireframe toggle) so the human can compare visually. Wiring: checkbox → `RunCommand.seamClip`
  → worker dispatches `mesh_cross_cut_shared_octree_clip` vs `mesh_cross_cut_shared_octree`.
  Verified live: `cross-cutting-vein-dipping` contacts go 2302t (snap) → 4051t (clip) → 2302t
  (reversible). Snap is still the default everywhere.
  - **Toolchain gotcha (cost me a long debug):** a TRAILING flag arg on the wasm binding (tried both
    `bool` and `u32`) silently arrived `0`/`false` in Rust across this wasm-bindgen/wasm-pack 0.14
    toolchain — an ABI desync on the last arg of a 12-arg method. Fix: **two separate no-flag
    methods** (snap + `_clip`), the worker picks one. Do NOT add a trailing flag arg to these
    bindings. Second gotcha: the clip mesh is heavier, so the worker re-run takes longer — a test
    that reads `__surfaces` right after toggling sees the STALE snap mesh; wait for the status to
    leave "done" and return before reading. Also: the tmpfs/ENOSPC in this env corrupted cargo's
    wasm32 incremental cache repeatedly (`cargo clean --target wasm32` fixed it); rebuild from a
    clean target if wasm output looks stale.

- **Step 3 — drop the field-clip per verified case + flip the default.** Once the human OKs the
  visual, make `Clip` the default (worker calls `_clip` unconditionally, or the binding defaults to
  it), then remove each case's `SlabExterior`/`Halfspace` masker from the contact scene (the contact
  contours unclipped and `clip` is the sole trimmer). Snap removed last, per case.

## Risks / open checks
- **Refinement:** the shared tree currently refines near the masker crossings (`cut_as_contrib`).
  With the contact contoured box-only, the seam location still needs refinement — keep
  `cut_as_contrib` (it refines on the contact field crossing, independent of the masker).
- **Winding precondition:** `clip` asserts W consistently wound. Octree wall meshes are (toward
  increasing field) — fine. Watch the foot wall's box-floor sliver artifact (noted in
  `shared_vein_walls_are_both_fully_resolved`); it's pre-existing, not clip's.
- **Normals:** `clip` output carries no per-vertex normals; the wasm/render path must face-average
  or re-derive. Check `WasmTaggedMesh` / the OBJ path doesn't assume normals are present.
- **Non-vein bodies through this path:** intrusions/shells/erode also flow through
  `mesh_cross_cut_shared`; the table covers them but only step-2/3 exercises them.
