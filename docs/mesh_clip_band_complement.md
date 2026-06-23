# `mesh_clip` keep classification: field predicate, not wall-mesh plane (Thread 1 fix)

**Status:** implemented + native-verified 2026-06-23. Root cause found via a bent-erosion repro;
fix is broader than the original "band complement" framing (which is one case of it).

## Root cause (the real one)

`clip`'s keep test decided which side of the wall W a contact face survives on by sampling the wall
**MESH** — `nearest_w_signed` (nearest W triangle's plane) for an uncut face, or the per-cutter plane
for a cut face. That is fragile exactly where a wall fit **extrapolates past its constraints near the
domain rim**: there the wall MESH and the body/erosion FIELD disagree, so a nearest-triangle plane
test mis-signs an uncut contact face the field would classify correctly — and a face mostly on the
discard side whose centroid lands on the keep side is kept WHOLE as an **isolated 1-triangle sliver**
in the eroded/interior zone. (The `Snap`/field-CSG path never shows this because it contours the
masked FIELD directly.)

**Reproduced natively** (`bent_erosion_clip_has_no_rim_slivers`): a BENT erosion ridge truncating one
flat contact → Clip gave **4 components** (the 2 correct edge-strips + 2 single-triangle slivers at
the y-rim, x≈ridge-apex) vs Snap's correct **2**. The two sliver faces were the *only* two kept
"uncut" faces that actually straddled the wall — the fit dipped the wall below the contact near the
rim, so they should have been trimmed, but the wall mesh there didn't intersect them. The dipping
VEIN did not reproduce natively (its walls don't extrapolate as wildly), which is why the earlier
3-bed vein repro looked clean — but the mechanism is the same and shows in the browser.

## The fix: classify keep with the FIELD, everywhere a field exists

The body/erosion field is the same ground truth `Snap`/field-CSG contours against, and an off-seam
centroid is well clear of the iso, so a field test is robust where the mesh extrapolation is not.
`clip`'s keep parameter is generalized to `Keep { Side(KeepSide), Predicate(&dyn Fn([f64;3])->bool) }`,
and every stack call site that has a field now passes a `Predicate`:

| Path | keep predicate | was |
|---|---|---|
| Erode (`mesh_one_erode_clipped`) | `efield > eiso` (survive side) | `Keep::Side(keep_side_for_erosion)` → wall mesh |
| Intrusion-single (`clip_walls`, 1 wall) | `field > iso` (exterior) | `Keep::Side(exterior_keep_side)` → wall mesh |
| Vein/shell band (`clip_walls`, 2 walls) | `field < lo \|\| field > hi` (exterior) | two-call concat + wall-mesh sides |

`efield > eiso` is the field form of the old `Above`-by-winding (W winds toward +∇field) — same
criterion, both pole orientations (the `erode_corefiner_matches_field_csg` parity test still passes).
The two now-dead mesh keep-side helpers (`exterior_keep_side`, `keep_side_for_erosion`) were removed.

Two structural improvements ride along:
- **One seam-id space for the band:** the 2-wall band combines both walls into ONE `ClipMesh` and one
  `clip` call (was two independent calls concatenated — independent seam vertices), the mesh analog of
  `octree::SlabComplement` (express the union as one complement, not two ANDed half-spaces).
- **Robust `nearest_w_signed`:** for the field-less `Keep::Side` path (future imported meshes), it now
  takes the CLAMPED closest point on W and signs by that triangle's normal, instead of the nearest
  centroid's infinite plane.

## Imported, field-less keep: the geometric predicate (Task 03)

An imported wall has no field, so it cannot use the `Predicate` forms above — it must classify by
the **mesh** itself. Task 03 (`docs/03_clip_keepside_curved_wall_robustness.md`) makes that path as
bent-wall-robust as the field path and gives it the same closure-shaped API, so the field and
field-less keeps read as exact parallels:

| Path | field predicate (generated) | geometric predicate (imported, field-less) |
|---|---|---|
| Single wall / erode | `Keep::Predicate(\|p\| field(p) - iso >= 0)` | `Keep::Predicate(&keep_side_of_mesh(w, side))` |
| Vein / shell band | `Keep::Predicate(\|p\| f(p) < lo \|\| f(p) > hi)` | `Keep::Predicate(&keep_outside_band(&lo, &hi))` |

`keep_side_of_mesh` is `\|p\| side_kept(nearest_w_signed(w, p), side)`; `keep_outside_band` is
`\|p\| nearest_w_signed(lo, p) < 0 \|\| nearest_w_signed(hi, p) > 0` — the field-less mirror of the
band complement (a UNION of two half-spaces a single `KeepSide` cannot express). Both **assume a
winding-normalized W** (`ClipMesh::winding_normalized`) so the CCW-normal sense is meaningful, and
both `debug_assert!` it; the band's two walls must additionally be **co-oriented** (both normals
toward increasing band coordinate).

**The cut-face `Keep::Side` fix that backs this.** `nearest_w_signed`'s clamped-closest-point test
covered only the *uncut* path; the cut-face path still kept a sub-triangle only if its centroid lay
on the keep side of **every** cutting W plane (`.all()` over infinite planes). Against a curved/bent
W those cutters splay, so a sub-triangle on the keep side of its OWN nearest cutter was discarded for
falling behind a splayed co-cutter's extended plane — carving a wedge out of the kept surface (the
wedge-gap bug, previously only defended on the field path). The cut-face path now signs each
sub-triangle by the **nearest** of its face's cutters (`nearest_w_signed_among`), the same clamped
test the uncut path uses. So `Keep::Side` — and therefore `keep_side_of_mesh` — is now bent-wall-safe
end to end. Verified field-agnostically by `keep_side_curved_wall_no_wedge_carve_either_bend` (torn
seam in both bend orientations under the old `.all()`, watertight under the fix) and
`keep_side_of_mesh_predicate_curved_wall_watertight_no_scallop`.

## Verification

Native: `bent_erosion_clip_has_no_rim_slivers` (Clip == Snap, 2 components, 0 slivers),
`band_complement_keeps_two_clean_components_per_bed`, `erode_corefiner_matches_field_csg_either_orientation`,
full geofield suite, wasm32 check. **Browser (the symptom is most visible there):** rebuild the wasm
engine and check `cross-cutting-vein-dipping` with "clip seam" ON for the rim slivers AND any holes.

## The change

### `mesh_clip::clip` — generalize the keep parameter

```rust
/// How `clip` decides which side of W to keep for C.
#[derive(Clone, Copy)]
pub enum Keep<'a> {
    /// Keep the chosen side of every cutting W plane (single wall / erosion). Existing behavior.
    Side(KeepSide),
    /// Keep where `pred(centroid)` is true — the caller's rule over a sub-triangle's centroid.
    /// For a vein band the caller passes `|p| f(p) < lo || f(p) > hi` (the SlabComplement union,
    /// evaluated on the body field: robust, off-seam centroids are well clear of the band edges).
    /// `clip` stays field-agnostic — it only calls the closure; an imported field-less wall band
    /// supplies a geometric predicate instead.
    Predicate(&'a dyn Fn([f64; 3]) -> bool),
}

pub fn clip(c: &ClipMesh, w: &ClipMesh, keep: Keep, refine: Option<&dyn ScalarField>) -> ClipResult
```

Three keep-classification sites switch on `Keep`:
- **cut C face** (`retriangulate_c_face`): `Side` → `.all()` over the face's cut W planes (unchanged);
  `Predicate` → `pred(sub_centroid)`.
- **uncut C face** (main loop `None` arm): `Side` → `side_kept(nearest_w_signed(...))` (unchanged);
  `Predicate` → `pred(face_centroid)`.
- **empty seam** (`empty_seam_result`): same switch on the representative centroid.

Per-cutter signed distances of *only the current face's cutters* cannot express the band complement
(a face cut by one wall still needs the other wall's side), so the predicate is evaluated at the
**centroid**, letting the caller carry the full rule (the field). This is why `Predicate` is a point
test, not the `Fn(&[SignedDist])` form.

### `stack.rs` `clip_walls` — one call for the 2-wall band

- **1 wall** (intrusion-single / erode): unchanged — one `clip` call, `Keep::Side(exterior_keep_side(..))`.
- **2 walls** (vein / shell band): **combine** foot ∪ hanging into one `ClipMesh` (disjoint index
  ranges → still consistently wound) and call `clip` **once** with
  `Keep::Predicate(|p| { let f = body_field.scalar_field(p); f < lo - eps || f > hi + eps })`,
  where `lo`/`hi` and `eps` are exactly `exterior_keep_side`'s band bounds. Both walls' seams now
  live in one `SeamBuilder` (one quantum, one on-edge-insertion pass over all seam points), so a
  C face crossed by both walls is re-triangulated **once** with both seams as constraints.

The two-call concat path is removed once the single-call path passes the native non-regression test.
The erode call site (`stack.rs:850`) becomes `Keep::Side(keep_side_for_erosion(..))`; the ~8
`mesh_clip` test call sites become `Keep::Side(KeepSide::Above)`.

## Tests

- **Native non-regression** (new): the faithful dipping-vein band → exactly 2 components per bed,
  0 size-1 components. Guards the floor; does **not** prove the wasm fix.
- **Keep green:** Milestone 1–2 (`mesh_clip`), `clip_seam_vein_band_is_watertight`,
  `shared_crease_closes_the_seam`, the vertical-vein cases.
- **Real verification (manual):** web "clip seam" toggle on `cross-cutting-vein-dipping` — beds
  should drop to 2 components with no slivers.

## Out of scope (Thread 2, follow-ups)

Combinatorial seam keying (the quantization-keying honesty item — the *most likely* wasm culprit if
orphans are seam-id-based), cdt-vs-spade doc reconciliation, and the `cdt_face` silent-skip hardening
are tracked separately and done after this. Thread-2 item-2 (curved-`W` keep robustness) is **done** —
see the imported field-less section above; the cut-face path now uses the same clamped nearest-cutter
classification as the uncut path, and `keep_side_of_mesh` / `keep_outside_band` are the imported
analogs of the field predicates. A full winding-number sign for `nearest_w_signed` (vs nearest-facet
normal) is the remaining refinement, only relevant for a closed imported solid, not an open wall.
