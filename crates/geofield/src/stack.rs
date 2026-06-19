//! Multi-series stacking with unconformity relationships.
//!
//! A [`Stack`] is an ordered list of geological series, **youngest first** (index
//! 0) **→ oldest last**, each an independently-fitted [`Model`] (one per-series
//! cokriging field — the same per-series solve as [`crate::Model::fit`]; there is
//! no global multi-field solve). Each series carries **two independent axes**
//! (`docs/stack_hierarchy_spec.md` §1–§2):
//!   * **Axis A — [`Relation`]** (`relation: Option<Relation>`): how it clips the
//!     (older) series beneath it — `Erode`, `Onlap`, `Conformable`, or `None` for
//!     the floor (no series below, so no clip is representable);
//!   * **Axis B — `unbounded_above: bool`**: whether the series is bounded by its
//!     own top contact or fills upward until a younger series claims the space.
//!
//! These were previously conflated in a single `Relation::Basement` variant, which
//! was a category error — `Basement` overloaded "present-everywhere",
//! "never-clips", and "keep-topmost-unit", three behaviours that all follow from
//! the one property `unbounded_above`. Splitting them makes the branch in
//! `present_unit_count` semantically honest (it asks `unbounded_above`, not
//! `matches!(relation, Basement)`) and removes the latent bug where mislabelling
//! the floor `Conformable` silently dropped its top unit. The old floor
//! (`Relation::Basement`) is now `relation: None` + `unbounded_above: true`.
//!
//! The ancestor of this scheme is the Lajaunie–Calcagno potential-field formalism
//! (Lajaunie et al. 1997; Calcagno et al. 2008): a model is an ordered stack of
//! series, each modelled as isovalues of one co-kriged scalar field, carrying a
//! single inter-series relationship drawn from the binary `erode | onlap`. GemPy's
//! `StackRelationType` (`BASEMENT/ERODE/ONLAP/FAULT`) and LoopStructural's
//! `UnconformityType` (`ERODE/ONLAP`) are both implementations of that formalism;
//! Leapfrog surfaces the SAME degree of freedom per-surface as its Deposit (onlap-
//! like) vs Erosion toggle. Our `Erode`/`Onlap` are Calcagno's binary; `Basement`
//! is GemPy's floor-of-the-model relation. `Conformable`, by contrast, is NOT a
//! relation in any of those tools — in the formalism family conformity is EMERGENT
//! (two surfaces are conformable iff they are isovalues of the same series field),
//! so we model `Conformable` as a series that simply never clips older series (the
//! same no-op an unoverridden Leapfrog deposit produces). See `docs/contact_grammar`.
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
//! series present at the point (an `unbounded_above` series is present
//! everywhere not already claimed). This single rule is correct for ALL relations
//! — erode and onlap do NOT differ in the lithology lookup.
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
//!   * `Conformable` (and the floor, `relation: None`) never clip older series.
//!
//! So: **erode maskers go into older series' Scenes; onlap maskers do not.** That
//! one wiring difference is the whole erode-vs-onlap semantics — Calcagno's "erode
//! truncates the older side it reaches, onlap laps against and pinches out" stated
//! as a clip vs no-clip. LoopStructural implements the same asymmetry as region
//! masks (`add_unconformity` walks back masking older features and stops at the next
//! unconformity; `add_onlap_unconformity` skips past them and inverts the mask),
//! and Leapfrog as the Erosion-cuts / Deposit-doesn't toggle on a surface. (If both
//! clipped identically the two relations would be indistinguishable — see the
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

/// How a series relates to the (older) series beneath it. `Erode`/`Onlap` are the
/// Lajaunie–Calcagno binary; GemPy's `StackRelationType` and LoopStructural's
/// `UnconformityType` are the same two values under different names, and Leapfrog's
/// Erosion/Deposit surface toggle is the same choice per-surface (see module docs).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Relation {
    /// Removes everything older wherever this series is present (truncation): the
    /// younger contact cuts the older surfaces along it. Calcagno's `erode`; GemPy
    /// `ERODE` (its default); LoopStructural `UnconformityType.ERODE`; Leapfrog's
    /// Erosion surface type.
    Erode,
    /// Laps onto the older surface and pinches out against it: present only on its
    /// deposited side, does NOT cut the older series where it pinches out. Calcagno's
    /// `onlap`; GemPy `ONLAP`; LoopStructural `ONLAP`; Leapfrog's Deposit (onlap-like)
    /// surface type.
    Onlap,
    /// Stacks normally (no unconformity); its interfaces subdivide within its own
    /// visible region but it does not clip older series. NOT a relation in Calcagno /
    /// GemPy / LoopStructural — there conformity is EMERGENT from a shared series
    /// field — so this is modelled as a non-clipping series (an unoverridden Leapfrog
    /// deposit produces the same no-op).
    Conformable,
}

/// One series in a [`Stack`]: its own fitted field plus its two independent axes —
/// how it clips the series beneath it ([`StackedSeries::relation`], Axis A) and
/// whether it is bounded by its own top contact ([`StackedSeries::unbounded_above`],
/// Axis B). Generic over the fitted model `M` so the SAME stack logic drives the
/// native kernel [`Model`] and the wasm [`RemoteModel`] (see [`SeriesModel`]). The
/// series' interfaces (and their isovalues) come from `model.interface_isovalues()`.
pub struct StackedSeries<M> {
    pub model: M,
    /// Axis A — how this series clips the (older) series beneath it. `None` for the
    /// floor (oldest series), which has no series below and so no representable clip.
    pub relation: Option<Relation>,
    /// Axis B — is this series bounded by its own top contact, or does it fill upward
    /// until a younger series claims the space? The floor is conventionally `true`.
    /// This is the single honest bit the old `Relation::Basement` variant smuggled in.
    pub unbounded_above: bool,
}

impl<M: SeriesModel> StackedSeries<M> {
    /// A normally-bounded series with an explicit Axis-A relation to the series
    /// beneath it (`unbounded_above = false`).
    pub fn new(model: M, relation: Relation) -> Self {
        Self { model, relation: Some(relation), unbounded_above: false }
    }

    /// The floor (oldest) series: no series beneath it (`relation = None`) and it
    /// fills upward until a younger series claims the space (`unbounded_above = true`).
    /// Replaces the old `Relation::Basement`.
    pub fn floor(model: M) -> Self {
        Self { model, relation: None, unbounded_above: true }
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

/// A cross-cutting **vein**: a tabular body occupying the region BETWEEN a co-kriged
/// hanging-wall/foot-wall interface pair (`docs/stack_hierarchy_spec.md` §3.2). Unlike a
/// stratigraphic series (present *below* one top contact), a vein is present in a *band*
/// (`foot_wall_iso < field < hanging_wall_iso`) and cross-cuts: it clips everything older
/// it intersects, but only WITHIN that band — two ANDed `ContactHalfSpace`s, not one.
///
/// `hanging_wall` / `foot_wall` are indices into `model.interface_isovalues()` (ascending);
/// the band is `[iso[foot_wall], iso[hanging_wall]]`.
pub struct Vein<M> {
    pub model: M,
    pub hanging_wall: usize,
    pub foot_wall: usize,
}

impl<M: SeriesModel> Vein<M> {
    pub fn new(model: M, foot_wall: usize, hanging_wall: usize) -> Self {
        Self { model, hanging_wall, foot_wall }
    }

    /// `(foot_wall_iso, hanging_wall_iso)` — the band bounds, lower first. `None` if either
    /// index is out of range. (We normalize lower≤upper so callers needn't assume order.)
    pub fn band(&self) -> Option<(f64, f64)> {
        let isos = self.model.interface_isovalues();
        let fw = *isos.get(self.foot_wall)?;
        let hw = *isos.get(self.hanging_wall)?;
        Some(if fw <= hw { (fw, hw) } else { (hw, fw) })
    }

    /// Present BETWEEN the foot-wall and hanging-wall isovalues (the tabular body), not
    /// below a single top. Out-of-range indices ⇒ present nowhere.
    pub fn present_at(&self, p: [f64; 3]) -> bool {
        match self.band() {
            Some((lo, hi)) => {
                let f = self.model.scalar_field(p);
                f > lo && f < hi
            }
            None => false,
        }
    }
}

/// The closed surface(s) bounding an [`Intrusion`]. `Single` is one envelope interface (the
/// body is the side of it the geometry deems interior); `Shell` is an inner/outer pair (the
/// body is between them — a hollow-cored intrusion). Indices into `interface_isovalues()`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Envelope {
    Single(usize),
    Shell { inner: usize, outer: usize },
}

/// A cross-cutting **intrusion**: a closed body present INSIDE its envelope, cross-cutting
/// everything older it intersects (`docs/stack_hierarchy_spec.md` §3.3). Geometry comes from
/// the body model's field thresholded at the envelope isovalue(s), not from a stratigraphic
/// top. The interior convention matches the field sign used elsewhere: `field < iso` is the
/// inside of the body (below the envelope contact).
pub struct Intrusion<M> {
    pub model: M,
    pub envelope: Envelope,
}

impl<M: SeriesModel> Intrusion<M> {
    pub fn new(model: M, envelope: Envelope) -> Self {
        Self { model, envelope }
    }

    /// Present inside the envelope: for `Single(i)`, `field < iso[i]`; for `Shell`, between
    /// the inner and outer isovalues. Out-of-range indices ⇒ present nowhere.
    pub fn present_at(&self, p: [f64; 3]) -> bool {
        let isos = self.model.interface_isovalues();
        let f = self.model.scalar_field(p);
        match self.envelope {
            Envelope::Single(i) => match isos.get(i) {
                Some(&iso) => f < iso,
                None => false,
            },
            Envelope::Shell { inner, outer } => match (isos.get(inner), isos.get(outer)) {
                (Some(&a), Some(&b)) => {
                    let (lo, hi) = if a <= b { (a, b) } else { (b, a) };
                    f > lo && f < hi
                }
                _ => false,
            },
        }
    }
}

/// One element of a [`Stack`]'s chronology: a stratigraphic [`StackedSeries`], or a
/// cross-cutting [`Vein`] / [`Intrusion`] (`docs/stack_hierarchy_spec.md` §3.1, Option A).
/// Lithology ([`Stack::global_unit`]) is uniform across kinds — youngest-present-wins —
/// so the only kind-specific behaviour is `present_at` (band/interior vs. below-top) and
/// the masker geometry a cross-cutting body contributes to older elements' Scenes.
pub enum StackElement<M> {
    Series(StackedSeries<M>),
    Vein(Vein<M>),
    Intrusion(Intrusion<M>),
}

impl<M: SeriesModel> StackElement<M> {
    /// Is this element's rock present at `p`? Dispatches to the kind's own test.
    pub fn present_at(&self, p: [f64; 3]) -> bool {
        match self {
            StackElement::Series(s) => s.present_at(p),
            StackElement::Vein(v) => v.present_at(p),
            StackElement::Intrusion(it) => it.present_at(p),
        }
    }

    /// The fitted model, for lithology lookup (uniform across kinds).
    pub fn model(&self) -> &M {
        match self {
            StackElement::Series(s) => &s.model,
            StackElement::Vein(v) => &v.model,
            StackElement::Intrusion(it) => &it.model,
        }
    }

    /// `true` if this is a stratigraphic series (vs. a cross-cutting body). Cross-cutting
    /// bodies are never the `unbounded_above` floor and carry no Axis-A relation.
    pub fn is_series(&self) -> bool {
        matches!(self, StackElement::Series(_))
    }

    /// Borrow as a stratigraphic series, if it is one.
    pub fn as_series(&self) -> Option<&StackedSeries<M>> {
        match self {
            StackElement::Series(s) => Some(s),
            _ => None,
        }
    }

    /// Axis B — `true` only for an unbounded-above series; always `false` for a
    /// cross-cutting body (which is bounded by its band/envelope).
    pub fn unbounded_above(&self) -> bool {
        matches!(self, StackElement::Series(s) if s.unbounded_above)
    }

    /// Axis A — the seam-below relation of a series (`None` for the floor or a
    /// cross-cutting body, which has no seam-below).
    pub fn relation(&self) -> Option<Relation> {
        match self {
            StackElement::Series(s) => s.relation,
            _ => None,
        }
    }
}

impl<M> From<StackedSeries<M>> for StackElement<M> {
    fn from(s: StackedSeries<M>) -> Self {
        StackElement::Series(s)
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
/// series is conventionally the floor ([`StackedSeries::floor`]: `relation = None`,
/// `unbounded_above = true`). Generic over the fitted model `M` ([`SeriesModel`]) so
/// the native and wasm paths share one implementation.
pub struct Stack<M> {
    /// The chronology, youngest first. Each entry is a stratigraphic series or a
    /// cross-cutting body ([`StackElement`]).
    pub series: Vec<StackElement<M>>,
}

impl<M: SeriesModel> Stack<M> {
    pub fn new(series: Vec<StackElement<M>>) -> Self {
        Self { series }
    }

    pub fn len(&self) -> usize {
        self.series.len()
    }

    pub fn is_empty(&self) -> bool {
        self.series.is_empty()
    }

    /// Borrow element `i` as a stratigraphic series, if it is one (`None` for a
    /// cross-cutting body or out of range). Used by the series-specific meshing/unit logic.
    fn series_at(&self, i: usize) -> Option<&StackedSeries<M>> {
        match self.series.get(i)? {
            StackElement::Series(s) => Some(s),
            _ => None,
        }
    }

    /// The lithology at `p`: `(element_index, unit_within_element)`.
    ///
    /// Youngest-present-wins — the youngest element present at `p` claims it, and the
    /// unit within it is its own single-field assignment ([`Model::lithology_unit`]).
    /// An `unbounded_above` series is present everywhere (it fills upward until claimed);
    /// a cross-cutting body is present in its band/interior. This rule is uniform across
    /// element kinds (`docs/stack_hierarchy_spec.md` §3.4 — cross-cutting needs no special
    /// lithology case). Returns `(len, 0)` only if no element is present.
    pub fn global_unit(&self, p: [f64; 3]) -> (usize, usize) {
        for (i, e) in self.series.iter().enumerate() {
            let unbounded = matches!(e, StackElement::Series(s) if s.unbounded_above);
            if unbounded || e.present_at(p) {
                return (i, e.model().lithology_unit(p));
            }
        }
        (self.series.len(), 0)
    }

    /// The younger CROSS-CUTTING contributions (indices `< i`) that CLIP element `i`'s
    /// surfaces, as owned-field + half-space-descriptor pairs. A younger `Erode` series
    /// contributes its top contact's ABOVE side (one half-space); a younger vein/intrusion
    /// contributes its band/interior (the body's own clip). Onlap/Conformable series (and
    /// the floor) contribute nothing. Returned as owned fields so the caller can keep them
    /// alive while the borrowed `ContactHalfSpace`s reference them.
    ///
    /// Each entry is `(field, isovalue, below)`: a `ContactHalfSpace::new(field, isovalue,
    /// below)`. An erode masker is the single `(iso_top, below=false)`; a vein is TWO entries
    /// (above FW, below HW); an intrusion the envelope interior.
    fn erode_masker_fields(&self, i: usize) -> Vec<(M::Field<'_>, f64, bool)> {
        let mut out = Vec::new();
        for ej in &self.series[..i] {
            match ej {
                // A younger erode series: its top contact's ABOVE side clips older surfaces.
                StackElement::Series(sj) if sj.relation == Some(Relation::Erode) => {
                    if let Some(iso_top) = sj.iso_top() {
                        out.push((sj.model.field(), iso_top, false));
                    }
                }
                // A younger vein: present BETWEEN fw/hw — clips older surfaces inside that
                // band = two ANDed half-spaces (above the lower iso, below the upper iso).
                StackElement::Vein(v) => {
                    if let Some((lo, hi)) = v.band() {
                        out.push((v.model.field(), lo, false)); // above the foot wall
                        out.push((v.model.field(), hi, true)); // below the hanging wall
                    }
                }
                // A younger intrusion: present inside its envelope — clips older surfaces
                // there. Single(i): interior is `field < iso` (below=true). Shell: between.
                StackElement::Intrusion(it) => {
                    let isos = it.model.interface_isovalues();
                    match it.envelope {
                        Envelope::Single(k) => {
                            if let Some(&iso) = isos.get(k) {
                                out.push((it.model.field(), iso, true));
                            }
                        }
                        Envelope::Shell { inner, outer } => {
                            if let (Some(&a), Some(&b)) = (isos.get(inner), isos.get(outer)) {
                                let (lo, hi) = if a <= b { (a, b) } else { (b, a) };
                                out.push((it.model.field(), lo, false));
                                out.push((it.model.field(), hi, true));
                            }
                        }
                    }
                }
                _ => {}
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
        // This contact mesher is for stratigraphic series; a cross-cutting body has its own
        // present region and is meshed via its own path. Series-only here.
        let s = self.series_at(i)?;
        let isovalue = *s.model.interface_isovalues().get(interface_idx)?;
        let box_b = AxisBox::from_bounds(min, ext_max);
        // Hold the masker fields in a local so the half-spaces (which borrow them) and the
        // field reference below all outlive the `Scene`.
        let masker_fields = self.erode_masker_fields(i);
        let maskers: Vec<ContactHalfSpace<'_>> = masker_fields
            .iter()
            // Each younger cross-cutting contribution is clipped on the side that leaves
            // element `i` visible OUTSIDE the younger body (erode: above its top; vein:
            // outside its band; intrusion: outside its envelope).
            .map(|(field, iso, below)| ContactHalfSpace::new(field, *iso, *below))
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
        for (i, e) in self.series.iter().enumerate() {
            // `mesh_one` is the stratigraphic-contact mesher and returns `None` for a
            // cross-cutting body (which has its own present region); such elements emit no
            // SeriesMeshes here. Series contacts mesh as before.
            let ids = e.model().interface_ids();
            let interfaces = ids
                .iter()
                .enumerate()
                .filter_map(|(k, &iid)| self.mesh_one(i, k, min, size, cfg).map(|m| (iid, m)))
                .collect();
            out.push(SeriesMeshes {
                series: i,
                interfaces,
            });
        }
        out
    }

    /// Older elements (indices `> i`, i.e. OLDER in the youngest-first order) whose present
    /// region a cross-cutting element at `i` spatially intersects — DERIVED from chronology
    /// + geometry, never declared (`docs/stack_hierarchy_spec.md` §3.4). Returns `[]` for a
    /// stratigraphic series (only cross-cutting bodies have a cut-set) and for a body that
    /// reaches nothing. Sampled on a coarse grid over `[min, min+size]³` — this is a derived
    /// reporting/UI quantity (which older elements a body cuts), not a meshing input.
    pub fn cuts_below(&self, i: usize, min: [f64; 3], size: f64) -> Vec<usize> {
        let body = match self.series.get(i) {
            Some(e) if !e.is_series() => e,
            _ => return Vec::new(), // series have no cut-set; out of range → none
        };
        const N: usize = 12; // coarse sample grid per axis
        let mut hits = Vec::new();
        for j in (i + 1)..self.series.len() {
            let older = &self.series[j];
            let mut overlaps = false;
            'scan: for a in 0..N {
                for b in 0..N {
                    for c in 0..N {
                        let p = [
                            min[0] + size * (a as f64 + 0.5) / N as f64,
                            min[1] + size * (b as f64 + 0.5) / N as f64,
                            min[2] + size * (c as f64 + 0.5) / N as f64,
                        ];
                        if body.present_at(p) && older.present_at(p) {
                            overlaps = true;
                            break 'scan;
                        }
                    }
                }
            }
            if overlaps {
                hits.push(j);
            }
        }
        hits
    }
}

impl<M: SeriesModel + BlockModel> Stack<M> {
    /// Number of fault blocks in series `i` (1 if that series is not faulted). `0` if `i`
    /// is out of range.
    pub fn series_block_count(&self, i: usize) -> usize {
        self.series_at(i).map(|s| s.model.nblocks()).unwrap_or(0)
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
        let s = self.series_at(i)?; // per-block contact mesher is series-only
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
            .map(|(field, iso, below)| ContactHalfSpace::new(field, *iso, *below))
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
        self.series_at(i).map(|s| s.model.block_unit_count(b)).unwrap_or(0)
    }

    /// Number of PRESENT (rock-bearing) lithology units in block `b` of series `i` — the
    /// units that are actually this series' rock, to render as closed solids.
    ///
    /// With `N` interfaces there are `N + 1` units `0..=N`. The topmost unit `N`
    /// (`field ≥ iso[N-1]`, above the series' highest contact) is rock IFF this series owns
    /// the space above that contact:
    ///
    ///   * a series BOUNDED above (`unbounded_above = false`) is present only BELOW its
    ///     top interface (`field < iso_top` — see [`StackedSeries::present_at`]); its
    ///     topmost unit is the VOID above the bounding/unconformity contact, owned by a
    ///     younger series, so it is dropped → `block_unit_count - 1` (`= interfaces`).
    ///   * an `unbounded_above` series (the floor) is present EVERYWHERE not claimed by a
    ///     younger series ([`Stack::global_unit`]), so its top interface is just an internal
    ///     subdivision, NOT a bounding contact — its topmost unit is real rock (truncated by
    ///     younger erosions / the box top, where it daylights). Dropping it leaves a hole:
    ///     the slab between the highest internal contact and the erosion surface vanishes,
    ///     and with it the volume that daylights against that erosion. So an unbounded-above
    ///     series keeps ALL its units → `block_unit_count`.
    ///
    /// This branches on `unbounded_above` — the one honest property — not on a relation
    /// variant. The old code keyed on `Relation::Basement`, which coincided with
    /// `unbounded_above` only at the bottom of the stack; keying on it directly removes
    /// the latent bug where a floor mislabelled `Conformable` dropped its top unit.
    ///
    /// Returns 0 for a field-less block.
    pub fn present_unit_count(&self, i: usize, b: usize) -> usize {
        let n = self.block_unit_count(i, b);
        if n == 0 {
            return 0;
        }
        match self.series_at(i) {
            // An unbounded-above series owns the column above its top contact, so its
            // topmost unit is rock — keep every unit.
            Some(s) if s.unbounded_above => n,
            // A bounded series' topmost unit is the void above its bounding contact.
            _ => n - 1,
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
        let s = self.series_at(i)?; // per-block unit-solid mesher is series-only
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
            .map(|(field, iso, below)| ContactHalfSpace::new(field, *iso, *below))
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
    /// flat bed at z=500 (the floor). Same geometry for both relations — only the
    /// younger relation changes.
    fn two_series_stack(younger: Relation) -> Stack<Model> {
        // Younger contact ~ z = 600 + 0.4*(x-500): below z≈400 at x=0, z≈800 at x=1000.
        let young = dipping_plane(600.0, 0.4, 0.0);
        // Older flat bed at z=500.
        let old = dipping_plane(500.0, 0.0, 0.0);
        Stack::new(vec![
            StackedSeries::new(young, younger).into(),
            StackedSeries::floor(old).into(),
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

    /// T011 [US1] — the three seam states (Erode / Onlap / Conformable) must produce
    /// THREE distinct older-series meshes, not two. `onlap_vs_erode_differ` pins erode≠onlap;
    /// this extends it to conformable. Onlap and Conformable both DON'T clip the older bed, so
    /// their older-mesh extents match each other and exceed the erode-clipped one — the
    /// discriminator is "erode clips, the other two don't," which is exactly the Axis-A
    /// grammar. (SC-002.)
    #[test]
    fn three_seam_states_differ() {
        let depth = 6;
        let extent = |stack: Stack<Model>| -> (f64, f64) {
            let meshes = stack.mesh_surfaces([0.0, 0.0, 0.0], 1000.0, cfg(depth));
            let (_iid, mesh) = &meshes[1].interfaces[0];
            let (mut lo, mut hi) = (f64::INFINITY, f64::NEG_INFINITY);
            for v in mesh.positions.chunks_exact(3) {
                let (x, z) = (v[0] as f64, v[2] as f64);
                if (z - 500.0).abs() < 60.0 {
                    lo = lo.min(x);
                    hi = hi.max(x);
                }
            }
            (lo, hi)
        };
        let (_, e_hi) = extent(two_series_stack(Relation::Erode));
        let (_, o_hi) = extent(two_series_stack(Relation::Onlap));
        let (_, c_hi) = extent(two_series_stack(Relation::Conformable));
        eprintln!("erode_hi={e_hi:.0} onlap_hi={o_hi:.0} conformable_hi={c_hi:.0}");

        // Erode clips the older bed on the right (small extent); onlap and conformable do
        // NOT clip (full extent). So both non-clipping relations extend well past erode.
        assert!(o_hi > e_hi + 100.0, "onlap must not be erode-clipped: o_hi={o_hi:.0} e_hi={e_hi:.0}");
        assert!(c_hi > e_hi + 100.0, "conformable must not be erode-clipped: c_hi={c_hi:.0} e_hi={e_hi:.0}");
        // The grammar's discriminating axis is clip-vs-no-clip; erode is the singular
        // clipping state, distinct from the two that leave the older bed intact.
        assert!(
            e_hi < o_hi && e_hi < c_hi,
            "erode (the clipping state) must yield a SMALLER older-bed extent than the two non-clipping states"
        );
    }

    /// T012 [US1] — a floor (`unbounded_above`) series must NEVER have its top unit dropped,
    /// regardless of what Axis-A relation a YOUNGER series carries above it. This generalizes
    /// the missing-volume regression past the single erode case: the keep is a property of the
    /// floor's `unbounded_above`, independent of the younger seam. (FR-003.)
    #[test]
    fn floor_top_unit_kept_under_any_younger_relation() {
        let make = |younger: Relation| -> Stack<Model> {
            // 3-contact floor (4 units), a younger series above it with the given relation.
            let mut sps = Vec::new();
            for (id, z0) in [(0u32, 300.0), (1, 500.0), (2, 700.0)] {
                for i in 0..4 {
                    for j in 0..4 {
                        sps.push(SurfacePoint::new([i as f64 * 300.0 + 50.0, j as f64 * 300.0 + 50.0, z0], id));
                    }
                }
            }
            let oris = vec![Orientation::new([500.0, 500.0, 500.0], [0.0, 0.0, 1.0])];
            let floor = Model::fit(&Series::new(sps, oris), KernelOptions::default());
            Stack::new(vec![
                StackedSeries::new(dipping_plane(650.0, 0.4, 0.0), younger).into(),
                StackedSeries::floor(floor).into(),
            ])
        };
        for younger in [Relation::Erode, Relation::Onlap, Relation::Conformable] {
            let stack = make(younger);
            assert!(stack.series[1].unbounded_above(), "series 1 is the floor");
            assert_eq!(
                stack.present_unit_count(1, 0),
                stack.block_unit_count(1, 0),
                "floor must keep ALL units under younger {younger:?} (top unit is rock, never dropped)"
            );
        }
    }

    /// A realistic erosional unconformity: the younger erode deposit sits ABOVE its dipping
    /// contact (pole pointing down), so the older floor survives BELOW the unconformity and
    /// is truncated at it — NOT erased down the whole column. This pins the geology the web3
    /// `erosional-unconformity` scenario relies on (the deposit-above fix). Contrast
    /// `floor_keeps_topmost_unit_which_daylights`, which uses the deposit-BELOW basalt.
    #[test]
    fn erode_deposit_above_truncates_floor_at_unconformity() {
        // Basalt contact z = 650 + 0.4(x-500); pole DOWN so basalt is present ABOVE it.
        let mut sps = Vec::new();
        for i in 0..4 {
            for j in 0..4 {
                let x = i as f64 * 300.0 + 50.0;
                let y = j as f64 * 300.0 + 50.0;
                sps.push(SurfacePoint::new([x, y, 650.0 + 0.4 * (x - 500.0)], 0));
            }
        }
        let nrm = (0.4f64 * 0.4 + 1.0).sqrt();
        // pole = (+sx, 0, -1)/|..| → field increases downward → present above the contact.
        let oris = vec![Orientation::new([500.0, 500.0, 650.0], [0.4 / nrm, 0.0, -1.0 / nrm])];
        let basalt = Model::fit(&Series::new(sps, oris), KernelOptions::default());

        // 3-contact floor at z = 300/500/700.
        let mut fsps = Vec::new();
        for (id, z0) in [(0u32, 300.0), (1, 500.0), (2, 700.0)] {
            for i in 0..4 {
                for j in 0..4 {
                    fsps.push(SurfacePoint::new([i as f64 * 300.0 + 50.0, j as f64 * 300.0 + 50.0, z0], id));
                }
            }
        }
        let floor = Model::fit(
            &Series::new(fsps, vec![Orientation::new([500.0, 500.0, 500.0], [0.0, 0.0, 1.0])]),
            KernelOptions::default(),
        );
        let stack = Stack::new(vec![
            StackElement::Series(StackedSeries::new(basalt, Relation::Erode)),
            StackElement::Series(StackedSeries::floor(floor)),
        ]);

        // RIGHT (x=900): contact ≈ z=810. The floor must survive BELOW it (all three beds at
        // 350/550/750 are floor), and basalt only caps ABOVE (z=950).
        for (z, want_floor) in [(350.0, true), (550.0, true), (750.0, true), (950.0, false)] {
            let (s, _u) = stack.global_unit([900.0, 500.0, z]);
            if want_floor {
                assert_eq!(s, 1, "x=900 z={z}: floor should survive below the unconformity");
            } else {
                assert_eq!(s, 0, "x=900 z={z}: basalt deposit caps above the unconformity");
            }
        }
        // LEFT (x=100): contact ≈ z=490. Floor below (z=350), basalt above (z=750).
        assert_eq!(stack.global_unit([100.0, 500.0, 350.0]).0, 1, "x=100 z=350: floor below");
        assert_eq!(stack.global_unit([100.0, 500.0, 750.0]).0, 0, "x=100 z=750: basalt above");
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
                StackedSeries::new(dipping_plane(750.0, 0.1, 0.0), Relation::Erode).into(),
                StackedSeries::new(dipping_plane(650.0, -0.2, 0.05), Relation::Onlap).into(),
                StackedSeries::new(dipping_plane(550.0, 0.15, 0.0), Relation::Erode).into(),
                StackedSeries::floor(dipping_plane(400.0, 0.0, 0.0)).into(),
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
        Stack::new(vec![StackedSeries::floor(model).into()])
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
        let younger = stack.series[0].as_series().unwrap();
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

    /// A multi-contact `Basement` under a younger `Erode` series — the scale scenario's
    /// shape (3 sediment contacts as a basement, an erosion plane above). Its three
    /// interfaces (ids 0,1,2) cut four units; the basement is present everywhere not claimed
    /// by the younger series, so ALL four units are rock.
    fn basement_under_erosion() -> Stack<Model> {
        let mut sps = Vec::new();
        for (id, z0) in [(0u32, 300.0), (1, 500.0), (2, 700.0)] {
            for i in 0..4 {
                for j in 0..4 {
                    let x = i as f64 * 300.0 + 50.0;
                    let y = j as f64 * 300.0 + 50.0;
                    sps.push(SurfacePoint::new([x, y, z0], id));
                }
            }
        }
        let oris = vec![Orientation::new([500.0, 500.0, 500.0], [0.0, 0.0, 1.0])];
        let basement = Model::fit(&Series::new(sps, oris), KernelOptions::default());
        // Younger erosion dipping z = 650 + 0.4(x-500), basalt above, eroding the floor.
        Stack::new(vec![
            StackedSeries::new(dipping_plane(650.0, 0.4, 0.0), Relation::Erode).into(),
            StackedSeries::floor(basement).into(),
        ])
    }

    /// THE missing-volume regression, re-keyed onto `unbounded_above` (the two-axis
    /// migration). An `unbounded_above` floor's topmost lithology unit is real rock (the
    /// slab between its highest internal contact and the eroding surface), so
    /// [`Stack::present_unit_count`] must KEEP it — for a BOUNDED series the topmost unit is
    /// a void and is dropped, but applying that drop to the floor deletes the very slab
    /// that daylights against the erosion, which read as "missing volumes" in the web2
    /// solid view. This test pins the keep on `unbounded_above` directly — NOT on the old
    /// `Relation::Basement` proxy — and guards the latent bug the migration closes: a floor
    /// mislabelled as bounded would silently drop its top unit.
    #[test]
    fn floor_keeps_topmost_unit_which_daylights() {
        let stack = basement_under_erosion();
        let cfg = OctreeConfig { max_depth: 6, min_depth: 2, qef_threshold: 1e-6, cap: true };
        let ext = [1000.0, 1000.0, 1000.0];

        // Series 1 is the floor: it is the honest property under test.
        assert!(stack.series[1].unbounded_above(), "series 1 must be the unbounded-above floor");
        assert_eq!(stack.series[1].relation(), None, "the floor has no Axis-A relation");

        // 3 interfaces ⇒ 4 units, and ALL are present (none is a void). A bounded series
        // drops its top unit; an unbounded-above floor must not.
        assert_eq!(stack.block_unit_count(1, 0), 4, "3-contact floor has 4 units");
        assert_eq!(
            stack.present_unit_count(1, 0),
            4,
            "an unbounded-above floor keeps ALL units (its top unit is rock, not a void)"
        );
        // The younger Erode series (index 0) DOES drop its top void unit.
        assert_eq!(
            stack.present_unit_count(0, 0),
            stack.block_unit_count(0, 0) - 1,
            "a younger-bounded series still drops its topmost (void) unit"
        );

        // The basement's topmost unit (index 3) is the slab that daylights against the
        // erosion — it must mesh with volume AND carry vertices on the erosion contact.
        let younger = stack.series[0].as_series().unwrap();
        let iso_top = younger.iso_top().unwrap();
        let top = stack
            .mesh_one_block_unit_solid(1, 0, 3, [0.0, 0.0, 0.0], 1000.0, ext, cfg)
            .expect("top unit in range");
        assert!(top.triangle_count() > 0, "the top basement unit must have volume");
        let on_erosion = top.positions.chunks_exact(3).filter(|v| {
            let p = [v[0] as f64, v[1] as f64, v[2] as f64];
            let f = younger.model.scalar_field(p) - iso_top;
            let g = younger.model.scalar_field_gradient(p);
            let gm = (g[0] * g[0] + g[1] * g[1] + g[2] * g[2]).sqrt().max(1e-9);
            (f / gm).abs() < 15.0
        }).count();
        assert!(on_erosion > 0, "the top basement unit must daylight ON the erosion contact");
    }

    /// The extent-box clip bounds the contour to the TRUE extent, not the octree cube. With
    /// a cube `size` much taller than `ext_max`, a contact must not produce geometry above
    /// `ext_max` — this is what keeps the cokriging drift's spurious lobes (which form out
    /// in the empty extrapolated column beyond the data) from being meshed. Without the
    /// clip the field re-crosses its isovalue far above the bed and grows a ceiling lobe.
    #[test]
    fn extent_clip_bounds_contour_to_true_extent() {
        // One flat bed at z=500 over a 1000-cube octree, but clip the TRUE extent z to 650.
        let stack = Stack::new(vec![StackedSeries::floor(dipping_plane(500.0, 0.0, 0.0)).into()]);
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

    /// A 2-interface model: two stacked flat contacts (ids 0,1) at z0 and z1, so a [`Vein`]
    /// can occupy the band between them. Field increases upward (pole +z).
    fn two_contact_model(z0: f64, z1: f64) -> Model {
        let mut sps = Vec::new();
        for (id, z) in [(0u32, z0), (1u32, z1)] {
            for i in 0..4 {
                for j in 0..4 {
                    sps.push(SurfacePoint::new([i as f64 * 300.0 + 50.0, j as f64 * 300.0 + 50.0, z], id));
                }
            }
        }
        let oris = vec![Orientation::new([500.0, 500.0, (z0 + z1) * 0.5], [0.0, 0.0, 1.0])];
        Model::fit(&Series::new(sps, oris), KernelOptions::default())
    }

    /// T029 [US2] — a vein is present ONLY between its foot-wall/hanging-wall isovalues (a
    /// band), not below a single top. Probe a point inside the band (present) and points
    /// above/below it (absent). (FR-006.)
    #[test]
    fn vein_is_present_only_within_its_band() {
        // FW at z≈400 (id 0), HW at z≈600 (id 1). The vein occupies the slab 400<z<600.
        let v = Vein::new(two_contact_model(400.0, 600.0), 0, 1);
        assert!(v.present_at([500.0, 500.0, 500.0]), "mid-band point is in the vein");
        assert!(!v.present_at([500.0, 500.0, 300.0]), "below the foot wall is outside the vein");
        assert!(!v.present_at([500.0, 500.0, 700.0]), "above the hanging wall is outside the vein");
    }

    /// T029 [US2] — a vein clips an older bed ONLY within its band, not as an unbounded
    /// half-space. We check the masker contribution: a younger vein over a floor contributes
    /// TWO half-spaces (above FW, below HW), so the floor's mesh survives OUTSIDE the band
    /// and is removed INSIDE it. Verify by lithology: a point in the band is claimed by the
    /// vein; a point below the band (still in the floor) is claimed by the floor.
    #[test]
    fn vein_cross_cuts_only_within_band() {
        let vein = Vein::new(two_contact_model(400.0, 600.0), 0, 1);
        let floor = StackedSeries::floor(dipping_plane(500.0, 0.0, 0.0));
        let stack = Stack::new(vec![StackElement::Vein(vein), StackElement::Series(floor)]);

        // In-band point ⇒ youngest-present-wins gives the vein (element 0).
        assert_eq!(stack.global_unit([500.0, 500.0, 500.0]).0, 0, "vein claims its band");
        // Below the band but inside the domain ⇒ the floor (element 1) shows through.
        assert_eq!(stack.global_unit([500.0, 500.0, 200.0]).0, 1, "floor shows below the vein band");
        // Above the band ⇒ also the floor (it is unbounded-above / present everywhere not
        // claimed), confirming the vein does NOT clip as an unbounded half-space.
        assert_eq!(stack.global_unit([500.0, 500.0, 800.0]).0, 1, "floor shows above the vein band");
    }

    /// T030 [US2] — an intrusion is present only INSIDE its envelope, and lithology needs no
    /// special case (youngest-present-wins already routes interior points to it). (FR-007,
    /// FR-009.)
    #[test]
    fn intrusion_claims_only_its_interior() {
        // Envelope = a single contact at z≈500; interior is `field < iso` (below it).
        let intr = Intrusion::new(dipping_plane(500.0, 0.0, 0.0), Envelope::Single(0));
        assert!(intr.present_at([500.0, 500.0, 300.0]), "below the envelope contact is interior");
        assert!(!intr.present_at([500.0, 500.0, 700.0]), "above the envelope contact is exterior");

        // In a stack over a floor, the intrusion claims its interior (element 0).
        let floor = StackedSeries::floor(dipping_plane(200.0, 0.0, 0.0));
        let stack = Stack::new(vec![StackElement::Intrusion(intr), StackElement::Series(floor)]);
        assert_eq!(stack.global_unit([500.0, 500.0, 300.0]).0, 0, "intrusion claims its interior");
    }

    /// T031 [US2] — `cuts_below` is DERIVED from chronology + spatial overlap, never declared:
    /// it returns exactly the older elements a cross-cutting body actually intersects, an
    /// empty set when it reaches none, and `[]` for a stratigraphic series. (FR-008.)
    #[test]
    fn cuts_below_is_derived_from_overlap() {
        let min = [0.0, 0.0, 0.0];
        let size = 1000.0;

        // A vein (band 400–600) over a floor it overlaps → cuts the floor (index 1).
        let vein = Vein::new(two_contact_model(400.0, 600.0), 0, 1);
        let floor = StackedSeries::floor(dipping_plane(500.0, 0.0, 0.0));
        let stack = Stack::new(vec![StackElement::Vein(vein), StackElement::Series(floor)]);
        assert_eq!(stack.cuts_below(0, min, size), vec![1], "vein cuts the floor it overlaps");
        // A stratigraphic series has no cut-set.
        assert_eq!(stack.cuts_below(1, min, size), Vec::<usize>::new(), "a series has no cut-set");

        // A vein whose band sits ABOVE the domain's data (band 1400–1600, outside [0,1000])
        // reaches nothing → empty cut-set, not an error.
        let high_vein = Vein::new(two_contact_model(1400.0, 1600.0), 0, 1);
        let floor2 = StackedSeries::floor(dipping_plane(500.0, 0.0, 0.0));
        let stack2 = Stack::new(vec![StackElement::Vein(high_vein), StackElement::Series(floor2)]);
        assert_eq!(stack2.cuts_below(0, min, size), Vec::<usize>::new(), "a body reaching nothing cuts nothing");
    }

    /// Where a younger erode deposit (present ABOVE its contact) fully covers an older
    /// contact, that older contact's open sheet must be cleanly truncated at the
    /// unconformity — emitting NO referenced geometry far below it on the eroded side, and
    /// (the stronger guarantee, checked separately) no referenced vertex inside the eroding
    /// body. This pins the deposit-above erosional-unconformity clip the web3 scenario uses.
    ///
    /// NB: this checks the field-PENETRATION guarantee (no referenced vertex inside the
    /// eroding body). It does NOT catch lateral overshoot ALONG the crease — verts that sit
    /// on the contact (`field ≈ iso`) but past the intersection — which was the visible
    /// "brown through blue" artifact; that is pinned separately by
    /// `eroded_open_sheet_does_not_overshoot_the_crease`. Both are now clean after the
    /// open-sheet termination fix (the `inside_trunc` face gate in `octree::contour_edges`).
    #[test]
    fn eroded_contact_is_cleanly_truncated_at_unconformity() {
        // Basalt deposited ABOVE a dipping contact z=650+0.4(x-500); pole points down.
        let mut sps = Vec::new();
        for i in 0..4 {
            for j in 0..4 {
                let x = i as f64 * 300.0 + 50.0;
                let y = j as f64 * 300.0 + 50.0;
                sps.push(SurfacePoint::new([x, y, 650.0 + 0.4 * (x - 500.0)], 0));
            }
        }
        let nrm = (0.4f64 * 0.4 + 1.0).sqrt();
        let basalt = Model::fit(
            &Series::new(sps, vec![Orientation::new([500.0, 500.0, 650.0], [0.4 / nrm, 0.0, -1.0 / nrm])]),
            KernelOptions::default(),
        );
        // Floor top bed at z≈700 (single interface suffices for the artifact).
        let floor = dipping_plane(700.0, 0.0, 0.0);
        let stack = Stack::new(vec![
            StackElement::Series(StackedSeries::new(basalt, Relation::Erode)),
            StackElement::Series(StackedSeries::floor(floor)),
        ]);

        let basalt_iso = match &stack.series[0] {
            StackElement::Series(s) => s.iso_top().unwrap(),
            _ => unreachable!(),
        };
        let cfg = OctreeConfig { max_depth: 6, min_depth: 2, qef_threshold: 1e-6, cap: false };
        let mesh = stack
            .mesh_one_clipped(1, 0, [0.0, 0.0, 0.0], 1000.0, [1000.0, 1000.0, 1000.0], cfg)
            .expect("floor interface in range");

        let referenced: std::collections::HashSet<u32> = mesh.indices.iter().copied().collect();
        let basalt_field = match &stack.series[0] {
            StackElement::Series(s) => &s.model,
            _ => unreachable!(),
        };
        let cell = 1000.0 / 2f64.powi(6) + 1.0;
        let (mut skirt, mut inside) = (0usize, 0usize);
        for &vi in &referenced {
            let o = vi as usize * 3;
            let p = [mesh.positions[o] as f64, mesh.positions[o + 1] as f64, mesh.positions[o + 2] as f64];
            // (a) No skirt: left of the seam (x<600, basalt covers the z=700 bed) AND far
            // below the contact = a spurious closing skirt.
            if p[0] < 600.0 - cell && p[2] < 600.0 {
                skirt += 1;
            }
            // (b) No penetration: the basalt is present where field < iso; a floor vertex
            // meaningfully inside that region would be brown-through-blue at the model level.
            if basalt_field.scalar_field(p) - basalt_iso < -0.03 {
                inside += 1;
            }
        }
        assert_eq!(skirt, 0, "eroded floor contact emitted {skirt} skirt vertices below the unconformity");
        assert_eq!(inside, 0, "eroded floor contact emitted {inside} vertices INSIDE the eroding basalt body");
    }

    /// REGRESSION (brown-through-blue): the open contact sheet must TERMINATE on the
    /// floor∩erosion intersection, not overshoot LATERALLY past it. The earlier
    /// penetration test misses this: the overshoot verts sit ON the floor contact (field
    /// ≈ iso) but on the eroded SIDE of the crease, so they pass a field-penetration check
    /// while still poking past the rendered erosion surface.
    ///
    /// Geometry = the web3 erosional-unconformity scenario: a deposit-above dipping basalt
    /// over a 3-contact floor (beds at z=300/500/700). The crease (where the floor∩erosion
    /// surfaces meet) is set by the FITTED basalt field, not the nominal plane: at z=500 the
    /// basalt field crosses its isovalue near x≈150 (the cokriged + deposit-above field is
    /// well left of the nominal `650+0.4(x-500)=500` ⇒ x=375). The open sheet must terminate
    /// at that field crease — where `inside_trunc` flips — NOT overshoot left of it.
    ///
    /// We pin the fix two ways: (1) the surviving contact must not extend into the eroded
    /// region where the basalt is meaningfully present (`field−iso < −ε`) — the lateral
    /// overshoot the field-penetration test missed; and (2) it is depth-STABLE (the
    /// pre-fix overshoot was angle-determined, depth-invariant).
    #[test]
    fn eroded_open_sheet_does_not_overshoot_the_crease() {
        let make = || {
            // Deposit-above basalt: z=650+0.4(x-500), pole down (present above the contact).
            let mut bsps = Vec::new();
            for i in 0..4 {
                for j in 0..4 {
                    let x = i as f64 * 300.0 + 50.0;
                    let y = j as f64 * 300.0 + 50.0;
                    bsps.push(SurfacePoint::new([x, y, 650.0 + 0.4 * (x - 500.0)], 0));
                }
            }
            let nrm = (0.4f64 * 0.4 + 1.0).sqrt();
            let basalt = Model::fit(
                &Series::new(bsps, vec![Orientation::new([500.0, 500.0, 650.0], [0.4 / nrm, 0.0, -1.0 / nrm])]),
                KernelOptions::default(),
            );
            // 3-contact floor (ids 0,1,2 at z=300/500/700), the web3 threeContactFloor.
            let mut fsps = Vec::new();
            for (id, z0) in [(0u32, 300.0), (1, 500.0), (2, 700.0)] {
                for i in 0..4 {
                    for j in 0..4 {
                        fsps.push(SurfacePoint::new([i as f64 * 300.0 + 50.0, j as f64 * 300.0 + 50.0, z0], id));
                    }
                }
            }
            let floor = Model::fit(
                &Series::new(fsps, vec![Orientation::new([500.0, 500.0, 500.0], [0.0, 0.0, 1.0])]),
                KernelOptions::default(),
            );
            Stack::new(vec![
                StackElement::Series(StackedSeries::new(basalt, Relation::Erode)),
                StackElement::Series(StackedSeries::floor(floor)),
            ])
        };
        // Middle bed (interface idx 1, z≈500). The basalt field defines the crease; assert
        // the surviving sheet does not reach meaningfully into the basalt-present region.
        // The crease at z=500 is where the basalt FIELD crosses its isovalue (NOT the
        // nominal plane `650+0.4(x-500)=500`⇒x=375 — the cokriged + deposit-above field
        // crosses well left of that). Locate it by bisection so the test pins true geometry.
        let basalt_iso = match &make().series[0] {
            StackElement::Series(s) => s.iso_top().unwrap(),
            _ => unreachable!(),
        };
        let crease_x = {
            let stack = make();
            let f = |x: f64| match &stack.series[0] {
                StackElement::Series(s) => s.model.scalar_field([x, 500.0, 500.0]) - basalt_iso,
                _ => unreachable!(),
            };
            // f<0 (basalt present) left, f>0 right; bisect the zero in [0,1000].
            let (mut lo, mut hi) = (0.0, 1000.0);
            for _ in 0..50 {
                let mid = (lo + hi) * 0.5;
                if f(lo) * f(mid) <= 0.0 { hi = mid } else { lo = mid }
            }
            (lo + hi) * 0.5
        };

        // The surviving contact must not extend left of the crease by more than ~a cell.
        // Pre-fix the sheet reached x≈94 (crease≈130) — fails; post-fix it sits at the
        // crease. Checked at two depths for depth-stability (the overshoot was
        // angle-determined / depth-invariant, so a too-lenient slop would still catch it).
        let mut min_x_at_depth = Vec::new();
        for depth in [6u32, 7] {
            let stack = make();
            let cfg = OctreeConfig { max_depth: depth, min_depth: 2, qef_threshold: 1e-6, cap: false };
            let mesh = stack
                .mesh_one_clipped(1, 1, [0.0, 0.0, 0.0], 1000.0, [1000.0, 1000.0, 1000.0], cfg)
                .expect("floor interface in range");
            let referenced: std::collections::HashSet<u32> = mesh.indices.iter().copied().collect();
            let cell = 1000.0 / 2f64.powi(depth as i32);
            let mut min_x = f64::INFINITY;
            for &vi in &referenced {
                let o = vi as usize * 3;
                let (x, z) = (mesh.positions[o] as f64, mesh.positions[o + 2] as f64);
                if (z - 500.0).abs() < 60.0 {
                    min_x = min_x.min(x);
                }
            }
            assert!(
                min_x >= crease_x - 1.5 * cell,
                "depth {depth}: middle bed open sheet overshoots the field crease — survives from \
                 x={min_x:.0}, crease at x≈{crease_x:.0} (allowed ≥ {:.0})",
                crease_x - 1.5 * cell
            );
            min_x_at_depth.push(min_x);
        }
        // Depth-stable: the termination is at the field crease, not a resolution staircase
        // that would march left as cells shrink. The two depths agree within a couple cells.
        let (a, b) = (min_x_at_depth[0], min_x_at_depth[1]);
        assert!(
            (a - b).abs() < 40.0,
            "termination should be depth-stable (at the crease), got x={a:.0} (d6) vs x={b:.0} (d7)"
        );
    }
}
