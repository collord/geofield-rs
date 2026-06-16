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
    octree_dual_contour_scene, AxisBox, Boundary, ContactHalfSpace, OctreeConfig, Scene,
};
use crate::Model;

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
/// series beneath it. The series' interfaces (and their isovalues) live in
/// `model.lithology()`.
pub struct StackedSeries {
    pub model: Model,
    pub relation: Relation,
}

impl StackedSeries {
    pub fn new(model: Model, relation: Relation) -> Self {
        Self { model, relation }
    }

    /// The series' bounding/unconformity isovalue — its TOP interface (highest
    /// isovalue). The series is "present" where `field < iso_top`. `None` if the
    /// series has no interfaces (an empty/degenerate fit).
    pub fn iso_top(&self) -> Option<f64> {
        self.model.lithology().interfaces.last().map(|iv| iv.isovalue)
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
/// series is conventionally a [`Relation::Basement`].
pub struct Stack {
    pub series: Vec<StackedSeries>,
}

impl Stack {
    pub fn new(series: Vec<StackedSeries>) -> Self {
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

    /// The younger series (indices `< i`) that CLIP series `i`'s surfaces: every
    /// younger `Erode` series. Onlap/Conformable/Basement younger series do not clip.
    fn erode_maskers_for(&self, i: usize) -> Vec<ContactHalfSpace<'_>> {
        let mut maskers = Vec::new();
        for j in 0..i {
            let sj = &self.series[j];
            if sj.relation == Relation::Erode {
                if let Some(iso_top) = sj.iso_top() {
                    // Series `i` survives where it is OUTSIDE younger `j`'s body, i.e.
                    // `field_j >= iso_top(j)` — the ABOVE side (`below = false`).
                    maskers.push(ContactHalfSpace::new(&sj.model, iso_top, false));
                }
            }
        }
        maskers
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
        let s = self.series.get(i)?;
        let iv = *s.model.lithology().interfaces.get(interface_idx)?;
        let box_b = AxisBox::new(min, size);
        let maskers = self.erode_maskers_for(i);
        let mut contributors: Vec<&dyn Boundary> = Vec::with_capacity(maskers.len() + 1);
        contributors.push(&box_b);
        for m in &maskers {
            contributors.push(m);
        }
        let scene = Scene::new(&s.model, iv.isovalue, contributors);
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
            let interfaces = s
                .model
                .lithology()
                .interfaces
                .iter()
                .enumerate()
                .map(|(k, iv)| {
                    let mesh = self
                        .mesh_one(i, k, min, size, cfg)
                        .expect("i,k in range by construction");
                    (iv.interface_id, mesh)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assemble::KernelOptions;
    use crate::data::{Orientation, Series, SurfacePoint};
    use crate::mesh::normalize;
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
    fn assert_stack_watertight(build: impl Fn() -> Stack, label: &str) {
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
    fn two_series_stack(younger: Relation) -> Stack {
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
    fn stack_present(stack: &Stack, i: usize, p: [f64; 3]) -> bool {
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
}
