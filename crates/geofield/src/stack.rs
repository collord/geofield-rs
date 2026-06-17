//! Multi-series stacking with unconformity relationships.
//!
//! A [`Stack`] is an ordered list of geological series, **youngest first** (index
//! 0) **→ oldest last**, each an independently-fitted [`Model`] (one per-series
//! cokriging field — the same per-series solve as [`crate::Model::fit`]; there is
//! no global multi-field solve). Each series carries a [`Relation`] describing how
//! it sits on the (older) series beneath it — the GemPy `StackRelationType`:
//! `Erode`, `Onlap`, `Conformable`, `Basement`.
//!
//! # What the relation controls
//!
//! Two things derive from the stack: the per-point **lithology** ([`Stack::global_unit`])
//! and the **clipped surface meshes** ([`Stack::mesh_surfaces`]).
//!
//! Sign convention (matching [`crate::octree::ContactSurface`]): within a series,
//! `field < iso` means "this series' rock is present here" (below its own contact).
//! A series' bounding/unconformity contact is its **top** isovalue
//! (`iso_top = interfaces.last()`); the series is present where `field < iso_top`.
//!
//! **Lithology is youngest-present-wins**: walk youngest→oldest, return the first
//! series present at the point (a `Basement` is present everywhere). This single
//! rule is correct for ALL relations — erode and onlap do NOT differ in the
//! lithology lookup.
//!
//! **The erode/onlap difference is in surface clipping.** When meshing an OLDER
//! series' contact:
//!   * a younger **Erode** series clips it — the older contact is cut along the
//!     younger's surface (visible only on the `field >= iso` side, outside the
//!     younger body). The eroding younger series is added as a clip half-space to
//!     the older series' [`Scene`].
//!   * a younger **Onlap** series does NOT clip it — the onlapping series laps onto
//!     and pinches out against the older surface, which is preserved as the contact.
//!     The onlapping younger series is OMITTED from the older series' `Scene`.
//!   * `Conformable` / `Basement` never clip older series.
//!
//! So: **erode maskers go into older series' Scenes; onlap maskers do not.** That
//! one wiring difference is the whole erode-vs-onlap semantics. (If both clipped
//! identically the two relations would be indistinguishable — see the
//! `onlap_vs_erode_differ` test, which pins this.)
//!
//! Each per-series-per-interface mesh is still a SINGLE [`Scene`] (the series'
//! contact intersected with the domain box and any younger erosions), contoured by
//! [`octree_dual_contour_scene`] in one watertight pass — no new contour algorithm.

use crate::mesh::Mesh;
use crate::octree::{
    octree_dual_contour_scene, AxisBox, Boundary, ContactHalfSpace, OctreeConfig, PlaneHalfSpace,
    Scene,
};
use crate::{clip_mesh_to_half_spaces, max_abs_field_in_box, BlockModel, SeriesModel};

/// How a series relates to the (older) series beneath it (GemPy `StackRelationType`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Relation {
    /// Removes everything older wherever this series is present (truncation): the
    /// younger contact cuts the older surfaces along it.
    Erode,
    /// Laps onto the older surface and pinches out against it: present only on its
    /// deposited side, does NOT cut the older series where it pinches out.
    Onlap,
    /// Stacks normally (no unconformity); its interfaces subdivide within its own
    /// visible region but it does not clip older series.
    Conformable,
    /// The oldest series; never masks anything and is present everywhere not claimed
    /// by a younger series.
    Basement,
}

/// One series in a [`Stack`]: its own fitted field plus its relationship to the
/// series beneath it. Generic over the fitted model `M` so the SAME stack logic drives
/// the native kernel [`Model`] and the wasm [`RemoteModel`] (see [`SeriesModel`]). The
/// series' interfaces (and their isovalues) come from `model.interface_isovalues()`.
pub struct StackedSeries<M> {
    pub model: M,
    pub relation: Relation,
}

impl<M: SeriesModel> StackedSeries<M> {
    pub fn new(model: M, relation: Relation) -> Self {
        Self { model, relation }
    }

    /// The series' bounding/unconformity isovalue — its TOP interface (highest
    /// isovalue). The series is "present" where `field < iso_top`. `None` if the
    /// series has no interfaces (an empty/degenerate fit).
    pub fn iso_top(&self) -> Option<f64> {
        self.model.interface_isovalues().last().copied()
    }

    /// Is this series' rock present at `p` (below its bounding contact)? A series
    /// with no interfaces is treated as present nowhere (it contributes nothing).
    pub fn present_at(&self, p: [f64; 3]) -> bool {
        match self.iso_top() {
            Some(iso) => self.model.scalar_field(p) - iso < 0.0,
            None => false,
        }
    }
}

/// One series' clipped contact meshes — one [`Mesh`] per interface of the series,
/// tagged with the interface id, all clipped to the series' visible region.
pub struct SeriesMeshes {
    /// Index of this series in the stack (0 = youngest).
    pub series: usize,
    /// `(interface_id, mesh)` for each of the series' interfaces (ascending isovalue).
    pub interfaces: Vec<(u32, Mesh)>,
}

/// An ordered stack of series, **youngest (index 0) → oldest (last)**. The oldest
/// series is conventionally a [`Relation::Basement`]. Generic over the fitted model `M`
/// ([`SeriesModel`]) so the native and wasm paths share one implementation.
pub struct Stack<M> {
    pub series: Vec<StackedSeries<M>>,
}

impl<M: SeriesModel> Stack<M> {
    pub fn new(series: Vec<StackedSeries<M>>) -> Self {
        Self { series }
    }

    pub fn len(&self) -> usize {
        self.series.len()
    }

    pub fn is_empty(&self) -> bool {
        self.series.is_empty()
    }

    /// The lithology at `p`: `(series_index, unit_within_series)`.
    ///
    /// Youngest-present-wins — the youngest series present at `p` claims it, and the
    /// unit within that series is its own single-field assignment
    /// ([`Model::lithology_unit`], thresholding the series' field at all its
    /// isovalues). A `Basement` is present everywhere. Returns `(len, 0)` only if no
    /// series is present (a stack with no basement, a point above everything).
    pub fn global_unit(&self, p: [f64; 3]) -> (usize, usize) {
        for (i, s) in self.series.iter().enumerate() {
            if s.relation == Relation::Basement || s.present_at(p) {
                return (i, s.model.lithology_unit(p));
            }
        }
        (self.series.len(), 0)
    }

    /// The younger-`Erode` series (indices `< i`) that CLIP series `i`'s surfaces, as
    /// `(field, iso_top)` pairs. Onlap/Conformable/Basement younger series do not clip.
    /// Returned as owned fields (not yet `ContactHalfSpace`s) so the caller can keep them
    /// alive while the half-spaces borrow them — a `ContactHalfSpace` borrows its field.
    fn erode_masker_fields(&self, i: usize) -> Vec<(M::Field<'_>, f64)> {
        let mut out = Vec::new();
        for sj in &self.series[..i] {
            if sj.relation == Relation::Erode {
                if let Some(iso_top) = sj.iso_top() {
                    out.push((sj.model.field(), iso_top));
                }
            }
        }
        out
    }

    /// Mesh ONE interface of series `i`, clipped to that series' visible region (the
    /// domain box ∩ outside every younger erosion). Returns `None` if `i` or
    /// `interface_idx` is out of range. The single-`Scene` watertight guarantee holds.
    ///
    /// This is the single-surface primitive [`Stack::mesh_surfaces`] iterates, also
    /// used by the WASM binding to mesh on demand without rebuilding the whole stack.
    pub fn mesh_one(
        &self,
        i: usize,
        interface_idx: usize,
        min: [f64; 3],
        size: f64,
        cfg: OctreeConfig,
    ) -> Option<Mesh> {
        self.mesh_one_clipped(i, interface_idx, min, size, [min[0] + size, min[1] + size, min[2] + size], cfg)
    }

    /// As [`Stack::mesh_one`], but clip the contour to the TRUE extent box `[min, ext_max]`
    /// rather than the octree cube `[min, min+size]³`. When the cube balloons one axis past
    /// the geology (a wide-flat domain), this drops the spurious floor/ceiling lobes the
    /// cokriging drift produces in the empty extrapolated space.
    pub fn mesh_one_clipped(
        &self,
        i: usize,
        interface_idx: usize,
        min: [f64; 3],
        size: f64,
        ext_max: [f64; 3],
        cfg: OctreeConfig,
    ) -> Option<Mesh> {
        let s = self.series.get(i)?;
        let isovalue = *s.model.interface_isovalues().get(interface_idx)?;
        let box_b = AxisBox::from_bounds(min, ext_max);
        // Hold the masker fields in a local so the half-spaces (which borrow them) and the
        // field reference below all outlive the `Scene`.
        let masker_fields = self.erode_masker_fields(i);
        let maskers: Vec<ContactHalfSpace<'_>> = masker_fields
            .iter()
            // Series `i` survives where it is OUTSIDE younger `j`'s body, i.e.
            // `field_j >= iso_top(j)` — the ABOVE side (`below = false`).
            .map(|(field, iso_top)| ContactHalfSpace::new(field, *iso_top, false))
            .collect();
        let mut contributors: Vec<&dyn Boundary> = Vec::with_capacity(maskers.len() + 1);
        contributors.push(&box_b);
        for m in &maskers {
            contributors.push(m);
        }
        let field = s.model.field();
        let scene = Scene::new(&field, isovalue, contributors);
        Some(octree_dual_contour_scene(&scene, min, size, cfg))
    }

    /// One watertight contact mesh per interface per series, each clipped to where
    /// that series is visible (the domain box ∩ outside every younger erosion).
    ///
    /// `[min, min+size]³` is the octree cube. Each mesh is produced by
    /// [`octree_dual_contour_scene`] on a single [`Scene`] (the series' contact at
    /// that interface's isovalue, intersected with the box and the younger-erosion
    /// clip half-spaces), so the watertight single-sign-field guarantee is preserved.
    pub fn mesh_surfaces(&self, min: [f64; 3], size: f64, cfg: OctreeConfig) -> Vec<SeriesMeshes> {
        let mut out = Vec::with_capacity(self.series.len());
        for (i, s) in self.series.iter().enumerate() {
            let ids = s.model.interface_ids();
            let interfaces = ids
                .iter()
                .enumerate()
                .map(|(k, &iid)| {
                    let mesh = self
                        .mesh_one(i, k, min, size, cfg)
                        .expect("i,k in range by construction");
                    (iid, mesh)
                })
                .collect();
            out.push(SeriesMeshes {
                series: i,
                interfaces,
            });
        }
        out
    }
}

impl<M: SeriesModel + BlockModel> Stack<M> {
    /// Number of fault blocks in series `i` (1 if that series is not faulted). `0` if `i`
    /// is out of range.
    pub fn series_block_count(&self, i: usize) -> usize {
        self.series.get(i).map(|s| s.model.nblocks()).unwrap_or(0)
    }

    /// Mesh ONE interface of series `i` **within a single fault block `b`** — the
    /// per-block open-sheet view that matches the `web/` harness: each fault block's
    /// contact is contoured from that block's OWN smooth field and clipped flush to the
    /// block's fault half-spaces, so the offset reads as two clean separate sheets rather
    /// than one surface stretched across the discontinuity.
    ///
    /// The block's contact is still clipped to the series' visible region (the domain box
    /// ∩ outside every younger erosion), exactly as [`Stack::mesh_one`] — only the field
    /// being contoured is the block field instead of the whole-model field, plus the
    /// post-hoc fault-plane clip. Returns `None` if `i`/`b`/`interface_idx` is out of
    /// range or block `b` is field-less.
    pub fn mesh_one_block(
        &self,
        i: usize,
        b: usize,
        interface_idx: usize,
        min: [f64; 3],
        size: f64,
        ext_max: [f64; 3],
        cfg: OctreeConfig,
    ) -> Option<Mesh> {
        let s = self.series.get(i)?;
        // The block carries the same interfaces as the series, at this block's (offset)
        // isovalue — use the block's own isovalue so the contour lands on the block field.
        let isovalue = *s.model.block_interface_isovalues(b).get(interface_idx)?;
        let block_field = s.model.block_field(b)?;
        // Clip to the TRUE extent box, not the (possibly ballooned) octree cube.
        let box_b = AxisBox::from_bounds(min, ext_max);
        // Younger-erosion clip half-spaces, exactly as `mesh_one` (whole-model fields —
        // erosion is geological and dispatches per-block correctly).
        let masker_fields = self.erode_masker_fields(i);
        let maskers: Vec<ContactHalfSpace<'_>> = masker_fields
            .iter()
            .map(|(field, iso_top)| ContactHalfSpace::new(field, *iso_top, false))
            .collect();
        let mut contributors: Vec<&dyn Boundary> = Vec::with_capacity(maskers.len() + 1);
        contributors.push(&box_b);
        for m in &maskers {
            contributors.push(m);
        }
        let scene = Scene::new(&block_field, isovalue, contributors);
        let mut mesh = octree_dual_contour_scene(&scene, min, size, cfg);
        // Cut the sheet flush at the fault plane(s) bounding this block.
        let half_spaces = s.model.block_half_spaces(b);
        if !half_spaces.is_empty() {
            clip_mesh_to_half_spaces(&mut mesh, &half_spaces);
        }
        Some(mesh)
    }

    /// Number of lithology UNIT solids in block `b` of series `i` (`interfaces + 1`, 0 if
    /// field-less or out of range). This counts EVERY unit including the topmost — for the
    /// renderable (rock-present) units use [`Stack::present_unit_count`].
    pub fn block_unit_count(&self, i: usize, b: usize) -> usize {
        self.series.get(i).map(|s| s.model.block_unit_count(b)).unwrap_or(0)
    }

    /// Number of PRESENT (rock-bearing) lithology units in block `b` of series `i` — the
    /// units that are actually this series' rock, to render as closed solids.
    ///
    /// A series is present where `field < iso_top` (below its bounding/unconformity
    /// contact). With `N` interfaces there are `N + 1` units `0..=N`; the present side
    /// `field < iso[N-1]` is units `0..N` — the topmost unit `N` (`field ≥ iso[N-1]`) is
    /// the VOID above the series' bounding contact, owned by whatever younger series sits
    /// there, NOT by this series. So the renderable count is `block_unit_count - 1`
    /// (`= interfaces`). Returns 0 for a field-less block.
    pub fn present_unit_count(&self, i: usize, b: usize) -> usize {
        match self.block_unit_count(i, b) {
            0 => 0,
            n => n - 1, // drop the topmost (above-the-bounding-contact) void unit
        }
    }

    /// Mesh ONE lithology UNIT of block `b` of series `i` as a watertight CLOSED solid —
    /// the slab between consecutive contacts `{ iso[unit-1] ≤ field_b < iso[unit] }`,
    /// capped by the domain box, the block's fault planes, AND every younger erosion (so
    /// the slab is truncated where a younger Erode series removed it — the same clip the
    /// open-sheet [`Stack::mesh_one_block`] applies, here as a real cap face that daylights
    /// against the erosion surface instead of being dropped).
    ///
    /// `unit` is `0..=interfaces` (bottom unit `0` has no lower contact; top unit
    /// `interfaces` has no upper contact, the box caps it). Returns `None` if `i`/`b`/`unit`
    /// is out of range or block `b` is field-less; the mesh may still be empty if the unit
    /// has no volume in this block's visible region.
    pub fn mesh_one_block_unit_solid(
        &self,
        i: usize,
        b: usize,
        unit: usize,
        min: [f64; 3],
        size: f64,
        ext_max: [f64; 3],
        cfg: OctreeConfig,
    ) -> Option<Mesh> {
        let s = self.series.get(i)?;
        let isovalues = s.model.block_interface_isovalues(b);
        let n = isovalues.len();
        if n == 0 || unit > n {
            return None;
        }
        let block_field = s.model.block_field(b)?;

        // Upper bound of the slab: the unit's upper contact, or (top unit) a value above
        // every field in the box so the box top caps it. The primary isosurface.
        let upper_iso = if unit < n {
            isovalues[unit]
        } else {
            max_abs_field_in_box(&block_field, min, size) + 1.0
        };
        // Lower bound: the ABOVE side of the unit's lower contact, if any.
        let lower: Option<ContactHalfSpace> = if unit > 0 {
            Some(ContactHalfSpace::new(&block_field, isovalues[unit - 1], false))
        } else {
            None
        };

        // Clip to the TRUE extent box, not the (possibly ballooned) octree cube.
        let box_b = AxisBox::from_bounds(min, ext_max);
        // The block's fault planes (kept as real caps so slabs stitch to the fault wall).
        let planes: Vec<PlaneHalfSpace> = s
            .model
            .block_half_spaces(b)
            .into_iter()
            .map(|hs| PlaneHalfSpace::new(hs.point, hs.normal, hs.keep_positive))
            .collect();
        // Younger-erosion clips (whole-model fields), so the closed slab respects erosion
        // exactly like the open sheet — but here the cut is a kept cap face.
        let masker_fields = self.erode_masker_fields(i);
        let maskers: Vec<ContactHalfSpace<'_>> = masker_fields
            .iter()
            .map(|(field, iso_top)| ContactHalfSpace::new(field, *iso_top, false))
            .collect();

        let mut contributors: Vec<&dyn Boundary> =
            Vec::with_capacity(planes.len() + maskers.len() + 2);
        contributors.push(&box_b);
        for p in &planes {
            contributors.push(p);
        }
        if let Some(lo) = &lower {
            contributors.push(lo);
        }
        for m in &maskers {
            contributors.push(m);
        }

        let cfg = OctreeConfig { cap: true, ..cfg };
        let scene = Scene::new(&block_field, upper_iso, contributors);
        Some(octree_dual_contour_scene(&scene, min, size, cfg))
    }
}

// Builds in-process kernel `Model`s, so native-only.
#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;
    use crate::assemble::KernelOptions;
    use crate::data::{Orientation, Series, SurfacePoint};
    use crate::mesh::normalize;
    use crate::Model;
    use std::collections::HashMap;

    /// Fit a planar contact `z = z0 + sx*(x-500) + sy*(y-500)` over a [0,1000]² grid
    /// as a single-interface `Model` (field increases upward). Mirrors the octree
    /// tests' `dipping_plane` helper.
    fn dipping_plane(z0: f64, sx: f64, sy: f64) -> Model {
        let mut sps = Vec::new();
        for i in 0..4 {
            for j in 0..4 {
                let x = i as f64 * 300.0 + 50.0;
                let y = j as f64 * 300.0 + 50.0;
                let z = z0 + sx * (x - 500.0) + sy * (y - 500.0);
                sps.push(SurfacePoint::new([x, y, z], 0));
            }
        }
        let oris = vec![Orientation::new([500.0, 500.0, z0], normalize([-sx, -sy, 1.0]))];
        Model::fit(&Series::new(sps, oris), KernelOptions::default())
    }

    fn cfg(depth: u32) -> OctreeConfig {
        OctreeConfig {
            max_depth: depth,
            min_depth: 2,
            qef_threshold: 1e-6,
            cap: true,
        }
    }

    /// `(nonmanifold, open, total)` undirected-edge incidence counts for a mesh.
    fn edge_stats(mesh: &Mesh) -> (usize, usize, usize) {
        let mut undirected: HashMap<(u32, u32), u32> = HashMap::new();
        for t in mesh.indices.chunks_exact(3) {
            for &(a, b) in &[(t[0], t[1]), (t[1], t[2]), (t[2], t[0])] {
                let e = if a < b { (a, b) } else { (b, a) };
                *undirected.entry(e).or_insert(0) += 1;
            }
        }
        let nm = undirected.values().filter(|&&c| c > 2).count();
        let open = undirected.values().filter(|&&c| c == 1).count();
        (nm, open, undirected.len())
    }

    /// Assert every interface mesh of every series is strictly watertight
    /// (incidence == 2), at several depths. The combined per-series Scene is a single
    /// sign field, so the watertight guarantee carries over from the octree CSG tests.
    fn assert_stack_watertight(build: impl Fn() -> Stack<Model>, label: &str) {
        for depth in [4u32, 5, 6] {
            let stack = build();
            let meshes = stack.mesh_surfaces([0.0, 0.0, 0.0], 1000.0, cfg(depth));
            for sm in &meshes {
                for (iid, mesh) in &sm.interfaces {
                    if mesh.triangle_count() == 0 {
                        continue; // a fully-clipped-away interface emits nothing — fine
                    }
                    let (nm, open, total) = edge_stats(mesh);
                    assert_eq!(
                        nm, 0,
                        "{label} d{depth} series {} iface {iid}: non-manifold {nm}/{total}",
                        sm.series
                    );
                    assert_eq!(
                        open, 0,
                        "{label} d{depth} series {} iface {iid}: open {open}/{total}",
                        sm.series
                    );
                }
            }
        }
    }

    /// A 2-series stack: a younger contact (`Erode` or `Onlap`) dipping steeply so it
    /// is present over the LEFT of the domain and absent over the RIGHT, over an older
    /// flat bed at z=500 (`Basement`). Same geometry for both relations — only the
    /// younger relation changes.
    fn two_series_stack(younger: Relation) -> Stack<Model> {
        // Younger contact ~ z = 600 + 0.4*(x-500): below z≈400 at x=0, z≈800 at x=1000.
        let young = dipping_plane(600.0, 0.4, 0.0);
        // Older flat bed at z=500.
        let old = dipping_plane(500.0, 0.0, 0.0);
        Stack::new(vec![
            StackedSeries::new(young, younger),
            StackedSeries::new(old, Relation::Basement),
        ])
    }

    #[test]
    fn stack_erode_surfaces_are_watertight() {
        assert_stack_watertight(|| two_series_stack(Relation::Erode), "erode");
    }

    #[test]
    fn stack_onlap_surfaces_are_watertight() {
        assert_stack_watertight(|| two_series_stack(Relation::Onlap), "onlap");
    }

    /// THE discriminating test: erode and onlap must produce DIFFERENT older-series
    /// surface meshes. The younger series is present over the left half; under ERODE
    /// it clips the older bed there (older mesh confined to the right), under ONLAP it
    /// does NOT (older bed meshed across the full domain). A no-op onlap aliased to
    /// erode would make these identical and fail this test.
    #[test]
    fn onlap_vs_erode_differ() {
        let depth = 6;
        let erode = two_series_stack(Relation::Erode).mesh_surfaces([0.0, 0.0, 0.0], 1000.0, cfg(depth));
        let onlap = two_series_stack(Relation::Onlap).mesh_surfaces([0.0, 0.0, 0.0], 1000.0, cfg(depth));

        // Series 1 is the older bed in both; compare its first interface's mesh extent.
        let older_xrange = |meshes: &[SeriesMeshes]| -> (f64, f64) {
            let (_iid, mesh) = &meshes[1].interfaces[0];
            let mut lo = f64::INFINITY;
            let mut hi = f64::NEG_INFINITY;
            // Only consider CONTACT vertices (those near z=500, the bed), not box-cap
            // vertices — the open contact extent is what erode clips.
            for v in mesh.positions.chunks_exact(3) {
                let (x, z) = (v[0] as f64, v[2] as f64);
                if (z - 500.0).abs() < 60.0 {
                    lo = lo.min(x);
                    hi = hi.max(x);
                }
            }
            (lo, hi)
        };

        let (e_lo, e_hi) = older_xrange(&erode);
        let (o_lo, o_hi) = older_xrange(&onlap);
        eprintln!("erode older bed x∈[{e_lo:.0},{e_hi:.0}]  onlap older bed x∈[{o_lo:.0},{o_hi:.0}]");

        // The younger contact (z=600+0.4(x-500)) rises above the older bed (z=500) for
        // x>250, so the younger body covers the bed on the RIGHT. Under ERODE the bed
        // is clipped there (older mesh confined to small x); under ONLAP the bed is
        // meshed across the full domain. So onlap's right extent far exceeds erode's.
        let _ = (e_lo, o_lo);
        assert!(
            o_hi > e_hi + 100.0,
            "onlap older bed should extend right of the erode-clipped one: onlap_hi={o_hi:.0} erode_hi={e_hi:.0}"
        );
    }

    /// Lithology is youngest-present-wins and is IDENTICAL for erode vs onlap (the
    /// relation changes meshing, not the per-point unit). At a point inside the
    /// younger body, the younger series wins regardless of relation; in the younger's
    /// absence zone, the older series shows through.
    #[test]
    fn lithology_is_youngest_present_wins() {
        let erode = two_series_stack(Relation::Erode);
        let onlap = two_series_stack(Relation::Onlap);

        // Right/low point: inside the younger body (young contact at x=900 is ~z=760,
        // so a point at z=300 is below it ⇒ younger present). Younger (series 0) wins,
        // and lithology is identical for erode vs onlap.
        let young_present = [900.0, 500.0, 300.0];
        assert_eq!(
            erode.global_unit(young_present).0,
            0,
            "younger should claim a point inside its body"
        );
        assert_eq!(
            erode.global_unit(young_present),
            onlap.global_unit(young_present),
            "lithology must not depend on erode vs onlap"
        );

        // Left/high point where the younger has pinched out: young contact at x=100 is
        // ~z=440, so a point at z=470 is ABOVE it ⇒ younger absent; it is below the
        // older bed (z=500) ⇒ older basement (series 1) shows through.
        let young_absent = [100.0, 500.0, 470.0];
        assert!(
            !stack_present(&erode, 0, young_absent),
            "younger should be absent at the pinch-out probe"
        );
        assert_eq!(
            erode.global_unit(young_absent).0,
            1,
            "older basement should claim the point where younger is absent"
        );
    }

    /// Helper: is series `i` of `stack` present at `p`?
    fn stack_present(stack: &Stack<Model>, i: usize, p: [f64; 3]) -> bool {
        stack.series[i].present_at(p)
    }

    /// A `Basement` claims every point not taken by a younger series — the stack
    /// partitions the domain with no unclaimed points.
    #[test]
    fn basement_partitions_domain() {
        let stack = two_series_stack(Relation::Erode);
        for i in 0..6 {
            for j in 0..6 {
                for k in 0..6 {
                    let p = [
                        i as f64 * 200.0 + 50.0,
                        j as f64 * 200.0 + 50.0,
                        k as f64 * 200.0 + 50.0,
                    ];
                    let (s, _u) = stack.global_unit(p);
                    assert!(s < stack.len(), "point {p:?} unclaimed (no basement coverage)");
                }
            }
        }
    }

    /// The canonical 4-series tutorial setup (seafloor/right/onlap/left) reduced to
    /// fitted planes — every interface mesh of every series watertight.
    #[test]
    fn stack_four_series_watertight() {
        let build = || {
            Stack::new(vec![
                StackedSeries::new(dipping_plane(750.0, 0.1, 0.0), Relation::Erode),
                StackedSeries::new(dipping_plane(650.0, -0.2, 0.05), Relation::Onlap),
                StackedSeries::new(dipping_plane(550.0, 0.15, 0.0), Relation::Erode),
                StackedSeries::new(dipping_plane(400.0, 0.0, 0.0), Relation::Basement),
            ])
        };
        assert_stack_watertight(build, "four-series");
    }

    /// A single-series stack whose one series is FAULTED: a flat contact offset across the
    /// plane x=500 (z=600 on the left, z=700 on the right — a 100-unit throw), the same
    /// geometry the kernel's per-block tests use. `nblocks()` is 2.
    fn faulted_single_series_stack() -> Stack<Model> {
        use crate::{OneFault, PlanarFault};
        use crate::data::{Orientation, Series, SurfacePoint};
        let mut sps = Vec::new();
        for i in 0..5 {
            for j in 0..3 {
                let x = i as f64 * 200.0 + 100.0; // 100,300,500,700,900
                let y = j as f64 * 300.0 + 200.0;
                let z = if x < 500.0 { 600.0 } else { 700.0 };
                sps.push(SurfacePoint::new([x, y, z], 0));
            }
        }
        let oris = vec![
            Orientation::new([300.0, 500.0, 600.0], [0.0, 0.0, 1.0]),
            Orientation::new([700.0, 500.0, 700.0], [0.0, 0.0, 1.0]),
        ];
        let fault = Box::new(OneFault::new(PlanarFault::new([500.0, 0.0, 0.0], [1.0, 0.0, 0.0])));
        let model = Model::fit_faulted(&Series::new(sps, oris), fault, KernelOptions::default());
        Stack::new(vec![StackedSeries::new(model, Relation::Basement)])
    }

    /// Per-block meshing honors the fault: a faulted series has 2 blocks, and each block's
    /// contact sheet stays on its OWN side of the fault plane at its OWN offset elevation —
    /// the offset is rendered as two separate sheets, not one surface stretched across the
    /// discontinuity. (This is the `web/`-parity property `mesh_one_block` exists to give;
    /// the whole-domain `mesh_one` would instead bridge the step.)
    #[test]
    fn faulted_series_meshes_per_block() {
        let stack = faulted_single_series_stack();
        let cfg = OctreeConfig { max_depth: 6, min_depth: 2, qef_threshold: 1e-6, cap: false };
        assert_eq!(stack.series_block_count(0), 2, "a one-fault series has 2 blocks");

        // Block 0 = left (x<500), contact at z=600; block 1 = right (x>=500), contact z=700.
        for (b, side_lo, side_hi, expect_z) in [(0usize, 0.0, 500.0, 600.0), (1, 500.0, 1000.0, 700.0)] {
            let mesh = stack
                .mesh_one_block(0, b, 0, [0.0, 0.0, 0.0], 1000.0, [1000.0, 1000.0, 1000.0], cfg)
                .expect("block in range");
            assert!(mesh.triangle_count() > 0, "block {b} produced an empty mesh");
            // Contact vertices (near the bed, not box-cap walls) must sit on this block's
            // side of the fault AND at this block's offset elevation — proving the sheet is
            // cut at the plane and carries the right block's field. Only check vertices
            // actually REFERENCED by a triangle: the Sutherland–Hodgman clip leaves the
            // pre-clip vertices in the buffer unreferenced (harmless), but the live surface
            // is the referenced set. Allow a one-cell slop at the plane for the cut edge.
            let cell = 1000.0 / 2f64.powi(6) + 1.0; // depth-6 leaf size + epsilon
            let referenced: std::collections::HashSet<u32> = mesh.indices.iter().copied().collect();
            let mut contact_vs = 0;
            for &vi in &referenced {
                let base = vi as usize * 3;
                let (x, z) = (mesh.positions[base] as f64, mesh.positions[base + 2] as f64);
                if (z - expect_z).abs() < 50.0 {
                    contact_vs += 1;
                    assert!(
                        x >= side_lo - cell && x <= side_hi + cell,
                        "block {b} contact vertex x={x:.1} crossed the fault (expected x∈[{side_lo},{side_hi}] ±{cell:.1})"
                    );
                }
            }
            assert!(contact_vs > 0, "block {b} has no contact vertices near z={expect_z}");
        }
    }

    /// The closed-solid path (`mesh_one_block_unit_solid`) produces watertight lithology
    /// slabs, and — for an ERODED older series — the slab DAYLIGHTS against the erosion
    /// surface: its closed solid reaches up to the eroding contact (a kept cap face),
    /// where the open-sheet view (`mesh_one_block`) is instead trimmed back. This is the
    /// difference the web2 "closed solids" toggle exposes.
    #[test]
    fn closed_solids_daylight_against_erosion() {
        let stack = two_series_stack(Relation::Erode); // older bed eroded on the left
        let cfg = OctreeConfig { max_depth: 6, min_depth: 2, qef_threshold: 1e-6, cap: true };

        // The older series (index 1) is non-faulted → one block. block_unit_count = 2 (one
        // interface ⇒ units below/above the bed). Every unit solid must be watertight.
        assert_eq!(stack.series_block_count(1), 1, "non-faulted series has one block");
        let units = stack.block_unit_count(1, 0);
        assert!(units >= 2, "older series should have ≥2 unit slabs, got {units}");

        let mut total_solid_tris = 0usize;
        for u in 0..units {
            let mesh = stack
                .mesh_one_block_unit_solid(1, 0, u, [0.0, 0.0, 0.0], 1000.0, [1000.0, 1000.0, 1000.0], cfg)
                .expect("unit in range");
            if mesh.triangle_count() == 0 {
                continue; // a unit with no volume in the visible region
            }
            total_solid_tris += mesh.triangle_count();
            let (nm, open, _total) = edge_stats(&mesh);
            assert_eq!(nm, 0, "unit {u} closed solid non-manifold: {nm}");
            assert_eq!(open, 0, "unit {u} closed solid not watertight: open {open}");
        }
        assert!(total_solid_tris > 0, "closed solids produced no geometry");

        // Daylighting: the closed solid of the older series carries cap faces lying ON the
        // erosion contact (the younger series' top isovalue surface), which the open sheet
        // drops. Probe the younger erosion elevation at a point where it cuts the older bed
        // — the closed solids must include vertices up near that contact, proving they
        // reach the erosion surface rather than stopping short of it.
        let younger = &stack.series[0];
        let iso_top = younger.iso_top().unwrap();
        // A point where the younger contact (z=600+0.4(x-500)) is near the older bed z=500:
        // 600+0.4(x-500)=500 ⇒ x≈250. Around there the erosion truncates the older bed.
        let mut reaches_erosion = false;
        for u in 0..units {
            let mesh = stack
                .mesh_one_block_unit_solid(1, 0, u, [0.0, 0.0, 0.0], 1000.0, [1000.0, 1000.0, 1000.0], cfg)
                .unwrap();
            for v in mesh.positions.chunks_exact(3) {
                let p = [v[0] as f64, v[1] as f64, v[2] as f64];
                // On the erosion contact, the younger field ≈ its top isovalue.
                if (younger.model.scalar_field(p) - iso_top).abs() < 0.05 * iso_top.abs().max(1.0) {
                    reaches_erosion = true;
                    break;
                }
            }
            if reaches_erosion {
                break;
            }
        }
        assert!(
            reaches_erosion,
            "closed solids should daylight ON the erosion contact (cap faces there), not stop short"
        );
    }

    /// The extent-box clip bounds the contour to the TRUE extent, not the octree cube. With
    /// a cube `size` much taller than `ext_max`, a contact must not produce geometry above
    /// `ext_max` — this is what keeps the cokriging drift's spurious lobes (which form out
    /// in the empty extrapolated column beyond the data) from being meshed. Without the
    /// clip the field re-crosses its isovalue far above the bed and grows a ceiling lobe.
    #[test]
    fn extent_clip_bounds_contour_to_true_extent() {
        // One flat bed at z=500 over a 1000-cube octree, but clip the TRUE extent z to 650.
        let stack = Stack::new(vec![StackedSeries::new(dipping_plane(500.0, 0.0, 0.0), Relation::Basement)]);
        let cfg = OctreeConfig { max_depth: 6, min_depth: 2, qef_threshold: 1e-6, cap: false };
        let ext_max = [1000.0, 1000.0, 650.0]; // z capped well below the cube top (1000)

        let mesh = stack
            .mesh_one_clipped(0, 0, [0.0, 0.0, 0.0], 1000.0, ext_max, cfg)
            .expect("interface in range");
        assert!(mesh.triangle_count() > 0, "produced no geometry");

        // No REFERENCED vertex may exceed the clipped extent (allow a one-cell slop at the
        // cut). The contact at z=500 is inside; any z≫650 vertex would be a cube-driven lobe.
        let cell = 1000.0 / 2f64.powi(6) + 1.0;
        let referenced: std::collections::HashSet<u32> = mesh.indices.iter().copied().collect();
        for &vi in &referenced {
            let z = mesh.positions[vi as usize * 3 + 2] as f64;
            assert!(
                z <= ext_max[2] + cell,
                "vertex z={z:.1} exceeds clipped extent {} (+{cell:.1}) — cube lobe not clipped",
                ext_max[2]
            );
        }
    }
}
