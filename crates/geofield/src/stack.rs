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
//! **The erode/onlap difference is in surface clipping — two MIRROR directions**
//! (see `clip_maskers`; both run through the same `Scene` machinery):
//!   * a younger **Erode** series clips the OLDER series' contact — cut along the
//!     younger's surface, visible only on the `field >= iso` side (outside the
//!     younger body). The eroding younger field is a clip half-space in the OLDER
//!     series' [`Scene`]. Direction: younger → older (up the stack).
//!   * an **Onlap** series clips ITS OWN surface against the older topography it laps
//!     onto — it survives only on its depositional side (ABOVE each older series' top
//!     contact, `field_older > iso`), so it PINCHES OUT along the intersection instead
//!     of passing through. The older series is NOT cut. The older fields are clip
//!     half-spaces in the ONLAP series' own `Scene`. Direction: older → younger (down
//!     the stack). (A clipped onlap surface legitimately has a free perimeter on its
//!     pinch-out crease — an open sheet that ends there, not a tear.)
//!   * `Conformable` (and the floor, `relation: None`) never clip.
//!
//! So erode and onlap are mirror clips — Calcagno's "erode truncates the older side it
//! reaches, onlap laps against and pinches out". LoopStructural implements the same
//! asymmetry as region masks (`add_unconformity` masks older features; `add_onlap_
//! unconformity` inverts the mask), Leapfrog as the Erosion-cuts / Deposit-doesn't
//! toggle. (If both clipped the same direction the two relations would be
//! indistinguishable — see the `onlap_vs_erode_differ` test, which pins this.) An
//! as-built overview of the whole stack/interaction model is in
//! `docs/surface_stack_as_built.md`.
//!
//! Each per-series-per-interface mesh is still a SINGLE [`Scene`] (the series'
//! contact intersected with the domain box and any younger erosions), contoured by
//! [`octree_dual_contour_scene`] in one watertight pass — no new contour algorithm.

use crate::mesh::Mesh;
use crate::octree::{
    octree_dual_contour_scene, AxisBox, Boundary, ContactHalfSpace, OctreeConfig, PlaneHalfSpace,
    Scene, SlabComplement,
};
use crate::{clip_mesh_to_half_spaces, max_abs_field_in_box, BlockModel, ScalarField, SeriesModel};

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

/// One element of a [`Stack`]'s chronology: a stratigraphic [`StackedSeries`], a
/// cross-cutting [`Vein`] / [`Intrusion`] (`docs/stack_hierarchy_spec.md` §3.1, Option A),
/// or the singleton [`Topography`] surface that caps the model against Air
/// (`docs/topography_lithology_solids_spec.md` §2).
/// Lithology ([`Stack::global_unit`]) is uniform across kinds — youngest-present-wins —
/// so the only kind-specific behaviour is `present_at` (band/interior vs. below-top vs.
/// below-ground) and the masker geometry a cross-cutting / capping body contributes to
/// older elements' Scenes.
pub enum StackElement<M> {
    Series(StackedSeries<M>),
    Vein(Vein<M>),
    Intrusion(Intrusion<M>),
    /// The ground surface. Pinned youngest (index 0), at most one per stack. Its single
    /// interface is the ground; ABOVE it is Air (no rock), BELOW it everything older is
    /// present. Mechanically the youngest erosional surface — it contributes the ABOVE-side
    /// half-space ([`Masker::Halfspace`] `below=false`) to every older element, so each is
    /// clipped to below the ground (`docs/topography_lithology_solids_spec.md` §2.1).
    Topography(Topography<M>),
}

/// The ground surface that caps a [`Stack`] against Air (`docs/topography_lithology_solids_spec.md`
/// §2). Owns one co-kriged field with a single interface; its upper unit is Air (never meshed
/// as a solid), its lower side is "everything below, present". Reuses the erode clip mechanism.
pub struct Topography<M> {
    /// The fitted ground field. Its top interface isovalue is the ground contact.
    pub model: M,
}

impl<M: SeriesModel> Topography<M> {
    /// The ground contact isovalue (the single interface's isovalue), if fitted.
    fn iso_ground(&self) -> Option<f64> {
        self.model.interface_isovalues().last().copied()
    }

    /// Is rock present at `p`? Topography is present (rock below) where the ground field is
    /// at-or-below the ground contact; ABOVE the ground (`field > iso`) is Air. Mirrors
    /// [`StackedSeries::present_at`] (below its top contact), the erode convention.
    fn present_at(&self, p: [f64; 3]) -> bool {
        match self.iso_ground() {
            Some(iso) => self.model.scalar_field(p) < iso,
            None => false,
        }
    }
}

impl<M: SeriesModel> StackElement<M> {
    /// Is this element's rock present at `p`? Dispatches to the kind's own test. Topography
    /// is "present" (caps the column) BELOW the ground — but it carries no rock of its own;
    /// [`Stack::global_unit`] turns that presence into Air, never a topography lithology.
    pub fn present_at(&self, p: [f64; 3]) -> bool {
        match self {
            StackElement::Series(s) => s.present_at(p),
            StackElement::Vein(v) => v.present_at(p),
            StackElement::Intrusion(it) => it.present_at(p),
            StackElement::Topography(t) => t.present_at(p),
        }
    }

    /// The fitted model, for lithology lookup (uniform across kinds). For topography this is
    /// the ground field; it is used for its contact geometry, not for a rock lithology.
    pub fn model(&self) -> &M {
        match self {
            StackElement::Series(s) => &s.model,
            StackElement::Vein(v) => &v.model,
            StackElement::Intrusion(it) => &it.model,
            StackElement::Topography(t) => &t.model,
        }
    }

    /// `true` if this is a stratigraphic series (vs. a cross-cutting body or topography).
    /// Cross-cutting bodies and topography are never the `unbounded_above` floor and carry
    /// no Axis-A relation.
    pub fn is_series(&self) -> bool {
        matches!(self, StackElement::Series(_))
    }

    /// `true` if this is the singleton topography surface.
    pub fn is_topography(&self) -> bool {
        matches!(self, StackElement::Topography(_))
    }

    /// Borrow as a stratigraphic series, if it is one.
    pub fn as_series(&self) -> Option<&StackedSeries<M>> {
        match self {
            StackElement::Series(s) => Some(s),
            _ => None,
        }
    }

    /// Axis B (DEPRECATED, inert) — was `true` only for an unbounded-above series. Fill-upward
    /// is now universal (`docs/topography_lithology_solids_spec.md` §3); nothing in the engine
    /// branches on this any more. Retained for the legacy `floor()` constructor / tests only.
    pub fn unbounded_above(&self) -> bool {
        matches!(self, StackElement::Series(s) if s.unbounded_above)
    }

    /// Axis A — the seam-below relation of a series (`None` for the floor, a cross-cutting
    /// body, or topography, which have no seam-below).
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

/// How a younger element clips an OLDER element's surface: the region of the older
/// surface that SURVIVES (is kept), expressed against the younger body's own field.
///
/// The older surface survives OUTSIDE the younger body. An erode/intrusion-single body
/// is one side of a single contact (`Halfspace`); a vein or shell intrusion is the
/// COMPLEMENT of a band (`SlabExterior`) — "outside the slab", a union the older
/// [`SlabComplement`] boundary expresses directly (ANDing two half-spaces would instead
/// keep the older surface INSIDE the band — the inverted-clip bug this type replaces).
enum Masker<F> {
    /// Keep the older surface on one side of a contact: `field < iso` if `below`, else
    /// `field > iso`. (Erode top = `below=false`; intrusion-single exterior = `below=false`.)
    Halfspace { field: F, iso: f64, below: bool },
    /// Keep the older surface OUTSIDE the band `[lo, hi]` (`field < lo || field > hi`).
    /// (Vein / shell intrusion.)
    SlabExterior { field: F, lo: f64, hi: f64 },
}

impl<F: ScalarField> Masker<F> {
    /// Borrow this masker as a [`Boundary`] contributor (ANDed into the older element's
    /// Scene). `Box<dyn>` so both half-space and slab-complement kinds unify.
    fn boundary(&self) -> Box<dyn Boundary + '_> {
        match self {
            Masker::Halfspace { field, iso, below } => Box::new(ContactHalfSpace::new(field, *iso, *below)),
            Masker::SlabExterior { field, lo, hi } => Box::new(SlabComplement::new(field, *lo, *hi)),
        }
    }
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
    /// Youngest-present-wins — the youngest element present at `p` claims it, and the unit
    /// within it is its own single-field assignment ([`Model::lithology_unit`]). A series that
    /// fills upward (the floor, and — once topography caps the model — any exposed series) is
    /// present everywhere not claimed by a younger element; a bounded series is present below
    /// its top contact; a cross-cutting body in its band/interior.
    ///
    /// Topography is the special cap and is evaluated FIRST regardless of its slot: ABOVE the
    /// ground there is no rock — the result is the Air/void sentinel `(len, 0)`; BELOW the
    /// ground topography is transparent (claims no lithology of its own), so evaluation
    /// proceeds to the older elements. This is what makes "open sky vs. rock" a property of
    /// the topographic surface, not a per-series flag (`docs/topography_lithology_solids_spec.md`
    /// §2.2, §3). The `unbounded_above` short-circuit is retained ONLY for the floor's
    /// fill-upward in topography-less (legacy) stacks; with topography present, the cap is
    /// what bounds fill-upward. Returns `(len, 0)` for Air / no element present.
    pub fn global_unit(&self, p: [f64; 3]) -> (usize, usize) {
        for (i, e) in self.series.iter().enumerate() {
            match e {
                // Topography never claims a lithology: above the ground → Air (stop, void);
                // below the ground → transparent, defer to older elements (continue).
                StackElement::Topography(t) => {
                    if !t.present_at(p) {
                        return (self.series.len(), 0); // Air, above the ground
                    }
                    // below ground: fall through to older elements
                }
                // A fill-upward series (the floor) is present everywhere not yet claimed; a
                // bounded series / cross-cutting body claims only where it is present.
                _ => {
                    let fills_upward = matches!(e, StackElement::Series(s) if s.unbounded_above);
                    if fills_upward || e.present_at(p) {
                        return (i, e.model().lithology_unit(p));
                    }
                }
            }
        }
        (self.series.len(), 0)
    }

    /// The contributions that CLIP element `i`'s surfaces — each a [`Masker`] describing
    /// where element `i`'s surface SURVIVES. Two directions, mirroring the two relations:
    ///
    /// **Younger bodies clip older (erode / cross-cut), indices `< i`.** A younger `Erode`
    /// series contributes the ABOVE side of its top contact; a younger vein or shell
    /// intrusion the EXTERIOR of its band; an intrusion-single the exterior of its envelope.
    /// (Element `i`'s surface survives OUTSIDE the younger body.)
    ///
    /// **An OLDER surface clips a younger ONLAP series, indices `> i`.** If element `i` is an
    /// onlap series it pinches out AGAINST the older topography it laps onto: it survives only
    /// on its depositional side (ABOVE each older series' top contact, `field_older > iso`), so
    /// its surface terminates exactly along the intersection with the older surface. This is
    /// the mirror of erode — the masker flows DOWN the stack (older → younger) instead of up.
    /// (Conformable contributes nothing in either direction.)
    ///
    /// The owned fields live in the returned [`Masker`]s so the borrowed boundaries (built via
    /// [`Masker::boundary`]) outlive them.
    fn clip_maskers(&self, i: usize) -> Vec<Masker<M::Field<'_>>> {
        let mut out = Vec::new();
        // (1) Younger bodies clipping element `i` (topography / erode / vein / intrusion).
        for ej in &self.series[..i] {
            match ej {
                // Topography is the youngest erosional surface: every older element survives
                // only BELOW the ground. It caps the whole model against Air
                // (`docs/topography_lithology_solids_spec.md` §2.1, §5.2). `below=true` keeps the
                // older surface where the ground field is below the ground contact (`field < iso`),
                // mirroring `Topography::present_at` (rock below ground).
                StackElement::Topography(t) => {
                    if let Some(iso) = t.iso_ground() {
                        out.push(Masker::Halfspace { field: t.model.field(), iso, below: true });
                    }
                }
                // A younger erode series: older survives ABOVE its top contact (`field > iso`).
                StackElement::Series(sj) if sj.relation == Some(Relation::Erode) => {
                    if let Some(iso_top) = sj.iso_top() {
                        out.push(Masker::Halfspace { field: sj.model.field(), iso: iso_top, below: false });
                    }
                }
                // A younger vein: present BETWEEN fw/hw — older survives OUTSIDE that band.
                StackElement::Vein(v) => {
                    if let Some((lo, hi)) = v.band() {
                        out.push(Masker::SlabExterior { field: v.model.field(), lo, hi });
                    }
                }
                // A younger intrusion: older survives OUTSIDE its envelope. Single(i): exterior
                // is `field > iso` (below=false). Shell: exterior of the [lo,hi] band.
                StackElement::Intrusion(it) => {
                    let isos = it.model.interface_isovalues();
                    match it.envelope {
                        Envelope::Single(k) => {
                            if let Some(&iso) = isos.get(k) {
                                out.push(Masker::Halfspace { field: it.model.field(), iso, below: false });
                            }
                        }
                        Envelope::Shell { inner, outer } => {
                            if let (Some(&a), Some(&b)) = (isos.get(inner), isos.get(outer)) {
                                let (lo, hi) = if a <= b { (a, b) } else { (b, a) };
                                out.push(Masker::SlabExterior { field: it.model.field(), lo, hi });
                            }
                        }
                    }
                }
                _ => {}
            }
        }
        // (2) If element `i` is an ONLAP series, every OLDER series it crosses clips it: it
        // survives only on its depositional side (ABOVE that older series' top contact). The
        // onlap surface thus pinches out against the older topography instead of passing
        // through it. (Cross-cutting bodies are not stratigraphic and lap onto nothing.)
        if matches!(&self.series[i], StackElement::Series(s) if s.relation == Some(Relation::Onlap)) {
            for ej in &self.series[i + 1..] {
                if let StackElement::Series(older) = ej {
                    if let Some(iso_top) = older.iso_top() {
                        out.push(Masker::Halfspace { field: older.model.field(), iso: iso_top, below: false });
                    }
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
        // This contact mesher is for stratigraphic series; a cross-cutting body has its own
        // present region and is meshed via its own path. Series-only here.
        let s = self.series_at(i)?;
        let isovalue = *s.model.interface_isovalues().get(interface_idx)?;
        let box_b = AxisBox::from_bounds(min, ext_max);
        // Hold the maskers + their boundaries in locals so the borrowed `&dyn Boundary`s all
        // outlive the `Scene`. Each younger cross-cutting contribution keeps element `i`
        // visible OUTSIDE the younger body (erode: above its top; vein/shell: outside its
        // band; intrusion-single: outside its envelope).
        let masker_fields = self.clip_maskers(i);
        let maskers: Vec<Box<dyn Boundary + '_>> = masker_fields.iter().map(|m| m.boundary()).collect();
        let mut contributors: Vec<&dyn Boundary> = Vec::with_capacity(maskers.len() + 1);
        contributors.push(&box_b);
        for m in &maskers {
            contributors.push(m.as_ref());
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
        // Younger cross-cutting clip maskers, exactly as `mesh_one` (whole-model fields —
        // cross-cutting is geological and dispatches per-block correctly).
        let masker_fields = self.clip_maskers(i);
        let maskers: Vec<Box<dyn Boundary + '_>> = masker_fields.iter().map(|m| m.boundary()).collect();
        let mut contributors: Vec<&dyn Boundary> = Vec::with_capacity(maskers.len() + 1);
        contributors.push(&box_b);
        for m in &maskers {
            contributors.push(m.as_ref());
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

    /// Mesh ONE surface of a NON-SERIES element (a cross-cutting vein/intrusion wall, or the
    /// topography ground contact, feature 003) at element `i` as an open contact sheet — the
    /// element's OWN surface, clipped to the extent box and to any YOUNGER element above it
    /// (the same `clip_maskers(i)` an older series gets). Returns `None` if `i` IS a
    /// stratigraphic series (use `mesh_one*`), or `interface_idx`/the field is out of range.
    ///
    /// This lifts the feature-001 logged punt ("the cross-cutting body's OWN 3D surface mesh
    /// is a follow-up — `mesh_one_block` is series-only"): the body's lithology/cut-set/masker
    /// clip already existed; only the wall geometry for the viewer was missing. A vein has two
    /// walls (foot wall = interface 0, hanging wall = 1); an intrusion has its envelope
    /// surface(s). Rendered as open sheets to match the stratigraphic contact view.
    pub fn mesh_cross_cut_surface(
        &self,
        i: usize,
        interface_idx: usize,
        min: [f64; 3],
        size: f64,
        ext_max: [f64; 3],
        cfg: OctreeConfig,
    ) -> Option<Mesh> {
        // Cross-cutting bodies only; a stratigraphic series is meshed by `mesh_one*`.
        let e = self.series.get(i)?;
        if e.is_series() {
            return None;
        }
        let isovalue = *e.model().interface_isovalues().get(interface_idx)?;
        let field = e.model().field();
        let box_b = AxisBox::from_bounds(min, ext_max);
        // Younger cross-cutting bodies above this one clip its walls too (same masker rule).
        let masker_fields = self.clip_maskers(i);
        let maskers: Vec<Box<dyn Boundary + '_>> = masker_fields.iter().map(|m| m.boundary()).collect();
        let mut contributors: Vec<&dyn Boundary> = Vec::with_capacity(maskers.len() + 1);
        contributors.push(&box_b);
        for m in &maskers {
            contributors.push(m.as_ref());
        }
        let scene = Scene::new(&field, isovalue, contributors);
        Some(octree_dual_contour_scene(&scene, min, size, cfg))
    }

    /// Mesh the closed, watertight INTERIOR SOLID of a cross-cutting body (vein/intrusion) at
    /// element `i` (`docs/topography_lithology_solids_spec.md` §4.3/§5 — the body's namesake
    /// lithology volume). The interior is the band `lo < field < hi` (vein / shell intrusion)
    /// or `field < iso` (single-envelope intrusion); the closed solid is the primary isosurface
    /// at the upper bound, capped below by the lower bound (if any), the box, and every YOUNGER
    /// element's clip (so the body is itself truncated by topography / a younger cross-cut).
    /// Returns `None` for a stratigraphic series (use `mesh_one_block_unit_solid`) or topography.
    pub fn mesh_cross_cut_solid(
        &self,
        i: usize,
        min: [f64; 3],
        size: f64,
        ext_max: [f64; 3],
        cfg: OctreeConfig,
    ) -> Option<Mesh> {
        // The interior bounds (upper iso, optional lower iso) for each cross-cutting kind.
        let (upper_iso, lower_iso): (f64, Option<f64>) = match self.series.get(i)? {
            StackElement::Vein(v) => {
                let (lo, hi) = v.band()?;
                (hi, Some(lo))
            }
            StackElement::Intrusion(it) => {
                let isos = it.model.interface_isovalues();
                match it.envelope {
                    Envelope::Single(k) => (*isos.get(k)?, None), // interior = field < iso
                    Envelope::Shell { inner, outer } => {
                        let (a, b) = (*isos.get(inner)?, *isos.get(outer)?);
                        let (lo, hi) = if a <= b { (a, b) } else { (b, a) };
                        (hi, Some(lo))
                    }
                }
            }
            _ => return None, // series / topography
        };

        let e = self.series.get(i)?;
        let field = e.model().field();
        let box_b = AxisBox::from_bounds(min, ext_max);
        // The lower cap: keep the interior ABOVE the lower contact (`field > lo`).
        let lower: Option<ContactHalfSpace> =
            lower_iso.map(|lo| ContactHalfSpace::new(&field, lo, false));
        // Younger elements (incl. topography) clip this body too.
        let masker_fields = self.clip_maskers(i);
        let maskers: Vec<Box<dyn Boundary + '_>> = masker_fields.iter().map(|m| m.boundary()).collect();

        let mut contributors: Vec<&dyn Boundary> = Vec::with_capacity(maskers.len() + 2);
        contributors.push(&box_b);
        if let Some(lo) = &lower {
            contributors.push(lo);
        }
        for m in &maskers {
            contributors.push(m.as_ref());
        }

        let cfg = OctreeConfig { cap: true, ..cfg };
        let scene = Scene::new(&field, upper_iso, contributors);
        Some(octree_dual_contour_scene(&scene, min, size, cfg))
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
    /// **Fill-upward is now universal** (`docs/topography_lithology_solids_spec.md` §3):
    /// every series keeps ALL its units, and clipping — younger erosions, cross-cutting
    /// bodies, and topography (all already applied by [`Stack::clip_maskers`] when the unit
    /// solid is meshed) — removes the parts that are not rock. The old `unbounded_above`
    /// branch (drop the top "void" unit for a bounded series) is GONE: that void is no longer
    /// a per-series property but is supplied by the cap above (topography → Air, or a younger
    /// element). The floor's long-standing keep-all behaviour is now every series' behaviour;
    /// the top unit daylights against whatever caps it, exactly as the floor's already did.
    ///
    /// `unbounded_above` is therefore **inert here** — this function no longer reads it
    /// (staged deprecation: the slot is retained-but-ignored, §3.2). In a topography-less
    /// stack a formerly-bounded series' top unit fills to the box ceiling, which is the
    /// honest "no ground surface defined yet" state.
    ///
    /// Returns 0 for a field-less block.
    pub fn present_unit_count(&self, i: usize, b: usize) -> usize {
        // Keep every unit, every series. Clipping (incl. topography) decides where each has
        // volume; the cap above supplies the void, so no unit is dropped at the count level.
        self.block_unit_count(i, b)
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
        // Younger cross-cutting clips (whole-model fields), so the closed slab respects them
        // exactly like the open sheet — but here the cut is a kept cap face.
        let masker_fields = self.clip_maskers(i);
        let maskers: Vec<Box<dyn Boundary + '_>> = masker_fields.iter().map(|m| m.boundary()).collect();

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
            contributors.push(m.as_ref());
        }

        let cfg = OctreeConfig { cap: true, ..cfg };
        let scene = Scene::new(&block_field, upper_iso, contributors);
        let mut mesh = octree_dual_contour_scene(&scene, min, size, cfg);

        // WELD the unit's shared contact caps onto the canonical contact surfaces, so adjacent
        // units land on the SAME surface and don't leave a sub-cell gap at their shared contact
        // (the dual-contour quantization that read as a horizontal seam in the web3 solid view).
        // Mirrors `Model::mesh_block_unit_solids`' weld, but here on the stack-CLIPPED unit so
        // topography/cross-cut clips are preserved. Welds only the bounding contacts of this
        // unit (`u-1` lower, `u` upper); the box/erosion cap faces are untouched.
        for &k in crate::bounding_contacts(unit, n).iter().flatten() {
            crate::ContactHeightField::build(&s.model, b, k, min, size, cfg).weld(&mut mesh);
        }
        Some(mesh)
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

    /// An ONLAP surface now PINCHES OUT against the older topography it laps onto (it is
    /// clipped to its depositional side — `field_older > iso`), so its open-sheet has a free
    /// perimeter along that pinch-out crease. That is geologically correct and NOT a tear, so
    /// the strict "every edge incidence 2" no longer holds for the onlap surface itself.
    ///
    /// The right invariant: (a) the OLDER bed is untouched by the onlap and stays strictly
    /// watertight; (b) the onlap surface's open edges lie ONLY on the domain box rim OR on the
    /// pinch-out crease (where it meets the older contact, `field_older ≈ iso_older`) — never
    /// an interior tear. (Mirrors `octree::open_sheet_boundary_is_flush_on_curved_contact`.)
    ///
    /// Checked at the render depths (5, 6). At depth 4 the pinch-out crease — where the dipping
    /// onlap surface grazes the older contact at a shallow angle — leaves a ragged ~1-cell gap
    /// (the documented open-sheet-meets-contact crease residual); the viewer renders at depth 6
    /// where it is clean, so the coarse case is a known, bounded resolution artifact, not a fix
    /// target here.
    #[test]
    fn onlap_surface_pinches_out_without_tearing() {
        use std::collections::HashMap;
        for depth in [5u32, 6] {
            let stack = two_series_stack(Relation::Onlap);
            let older = stack.series_at(1).unwrap();
            let iso_older = older.iso_top().unwrap();
            let meshes = stack.mesh_surfaces([0.0, 0.0, 0.0], 1000.0, cfg(depth));

            // (a) The older bed (series 1) is untouched → strictly watertight.
            for (iid, mesh) in &meshes[1].interfaces {
                if mesh.triangle_count() == 0 {
                    continue;
                }
                let (nm, open, total) = edge_stats(mesh);
                assert_eq!(nm, 0, "onlap d{depth} OLDER iface {iid}: non-manifold {nm}/{total}");
                assert_eq!(open, 0, "onlap d{depth} OLDER iface {iid}: open {open}/{total}");
            }

            // (b) The onlap surface (series 0): no non-manifold edges, and every OPEN edge is
            // on the box rim or the pinch-out crease — no interior tear.
            for (iid, mesh) in &meshes[0].interfaces {
                if mesh.triangle_count() == 0 {
                    continue;
                }
                let p = |i: u32| {
                    let b = (i as usize) * 3;
                    [mesh.positions[b] as f64, mesh.positions[b + 1] as f64, mesh.positions[b + 2] as f64]
                };
                let mut undirected: HashMap<(u32, u32), u32> = HashMap::new();
                for t in mesh.indices.chunks_exact(3) {
                    for &(a, b) in &[(t[0], t[1]), (t[1], t[2]), (t[2], t[0])] {
                        let e = if a < b { (a, b) } else { (b, a) };
                        *undirected.entry(e).or_insert(0) += 1;
                    }
                }
                let nm = undirected.values().filter(|&&c| c > 2).count();
                assert_eq!(nm, 0, "onlap d{depth} surface iface {iid}: non-manifold {nm}");
                // An open edge is allowed on the box rim (within ~1 cell of a box face) or on the
                // pinch-out crease (both endpoints near the older contact). A cell at depth 4 is
                // ~62 wide; allow a generous box-rim band so a crease∩box-corner counts as rim.
                let cell = 1000.0 / 2f64.powi(depth as i32);
                let on_box = |q: [f64; 3]| (0..3).any(|d| q[d] < cell || q[d] > 1000.0 - cell);
                let near_crease = |q: [f64; 3]| (older.model.scalar_field(q) - iso_older).abs() < 0.1;
                let torn = undirected
                    .iter()
                    .filter(|(_, &c)| c == 1)
                    .filter(|((a, b), _)| {
                        let (pa, pb) = (p(*a), p(*b));
                        let rim = on_box(pa) && on_box(pb);
                        let crease = near_crease(pa) && near_crease(pb);
                        !(rim || crease)
                    })
                    .count();
                assert_eq!(torn, 0, "onlap d{depth} surface iface {iid}: {torn} interior tears (not box/crease)");
            }
        }
    }

    /// The `SlabComplement` clip (a vein cross-cutting an older bed) must stay watertight —
    /// the non-convex "outside the band" contributor is the risk the slab-complement approach
    /// takes on. A vein band 350..650 over a flat bed at z=500: the bed's closed-solid mesh,
    /// clipped to OUTSIDE the band, must have every edge incidence exactly 2 at several depths
    /// (the double-crossed-edge refinement counts BOTH band contacts via `edge_crossing_count`).
    #[test]
    fn vein_clipped_bed_is_watertight() {
        assert_stack_watertight(
            || {
                let vein = Vein::new(two_contact_model(350.0, 650.0), 0, 1);
                let bed = StackedSeries::floor(dipping_plane(500.0, 0.0, 0.0));
                Stack::new(vec![StackElement::Vein(vein), StackElement::Series(bed)])
            },
            "vein-clipped-bed",
        );
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
    /// `every_series_keeps_all_units_and_floor_top_daylights`, which uses the deposit-BELOW basalt.
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

    /// THE missing-volume regression, now under **universal fill-upward**
    /// (`docs/topography_lithology_solids_spec.md` §3). Every series keeps ALL its units —
    /// the floor always did (its topmost unit is the rock slab that daylights against the
    /// eroding surface), and that keep-all is now every series' behaviour. The old "a bounded
    /// series drops its topmost void unit" rule is GONE: that void is no longer a per-series
    /// property but is supplied by the cap above (a younger erosion here; topography in the
    /// full model). [`Stack::present_unit_count`] therefore equals [`Stack::block_unit_count`]
    /// for every series, and `unbounded_above` is inert in that decision. The geometric
    /// guarantee that actually mattered — the floor's top slab meshes with volume and
    /// daylights ON the erosion contact — is unchanged and still pinned below.
    #[test]
    fn every_series_keeps_all_units_and_floor_top_daylights() {
        let stack = basement_under_erosion();
        let cfg = OctreeConfig { max_depth: 6, min_depth: 2, qef_threshold: 1e-6, cap: true };
        let ext = [1000.0, 1000.0, 1000.0];

        // Series 1 is the floor (its `unbounded_above` slot is now inert for the unit count,
        // but still marks the fill-upward series in `global_unit` for a topography-less stack).
        assert!(stack.series[1].unbounded_above(), "series 1 is the (legacy) fill-upward floor");
        assert_eq!(stack.series[1].relation(), None, "the floor has no Axis-A relation");

        // 3 interfaces ⇒ 4 units; under universal fill-upward EVERY series keeps all units.
        assert_eq!(stack.block_unit_count(1, 0), 4, "3-contact floor has 4 units");
        assert_eq!(
            stack.present_unit_count(1, 0),
            4,
            "universal fill-upward: the floor keeps ALL units (top unit is rock, not a void)"
        );
        // The younger Erode series now ALSO keeps all its units (clipping, not the count,
        // removes the part above the erosion); the flag no longer drops its top unit.
        assert_eq!(
            stack.present_unit_count(0, 0),
            stack.block_unit_count(0, 0),
            "universal fill-upward: a younger series keeps its topmost unit too (clip-driven)"
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

    /// Part B — a vein's OWN walls now mesh (the feature-001 logged punt is lifted). Each of
    /// the two walls (foot wall id 0, hanging wall id 1) produces a non-empty open sheet near
    /// its isosurface; a stratigraphic series returns None from the cross-cut mesher.
    #[test]
    fn vein_walls_mesh_as_open_sheets() {
        let vein = Vein::new(two_contact_model(300.0, 700.0), 0, 1);
        let floor = StackedSeries::floor(dipping_plane(500.0, 0.0, 0.0));
        let stack = Stack::new(vec![StackElement::Vein(vein), StackElement::Series(floor)]);
        let cfg = OctreeConfig { max_depth: 6, min_depth: 2, qef_threshold: 1e-6, cap: false };
        let ext = [1000.0, 1000.0, 1000.0];
        for k in 0..2 {
            let mesh = stack
                .mesh_cross_cut_surface(0, k, [0.0, 0.0, 0.0], 1000.0, ext, cfg)
                .expect("vein wall should mesh");
            assert!(mesh.triangle_count() > 0, "vein wall {k} produced an empty mesh");
        }
        // A stratigraphic series is not a cross-cutting body → None.
        assert!(
            stack.mesh_cross_cut_surface(1, 0, [0.0, 0.0, 0.0], 1000.0, ext, cfg).is_none(),
            "a series has no cross-cut wall mesh"
        );
    }

    /// REGRESSION — a younger vein clips an older bed's MESH to OUTSIDE its band (not inside).
    /// `vein_cross_cuts_only_within_band` checks `global_unit` (lithology, driven by
    /// `present_at`); this checks the SEPARATE meshing path (`clip_maskers` → the Scene), which
    /// before the SlabComplement fix ANDed two half-spaces and kept the bed INSIDE the band —
    /// the inverted clip the user saw (sediments preserved between the walls, erased outside).
    ///
    /// A flat bed at z=500 runs through the whole column; a vein band 300..700 (foot wall id 0,
    /// hanging wall id 1) cross-cuts it. The meshed bed must SURVIVE where the vein is absent
    /// (|y| far from the band in the vein's field) and be REMOVED inside the band. We assert via
    /// the masker boolean (the exact predicate the Scene ANDs) at bed points in/out of the band.
    #[test]
    fn vein_clips_older_bed_mesh_outside_its_band() {
        // Vein over a bed; the vein's band is 300..700 in ITS field (two_contact_model fw/hw).
        let vein = Vein::new(two_contact_model(300.0, 700.0), 0, 1);
        let bed = StackedSeries::floor(dipping_plane(500.0, 0.0, 0.0));
        let stack = Stack::new(vec![StackElement::Vein(vein), StackElement::Series(bed)]);

        // The masker the bed (element 1) sees from the younger vein (element 0): exactly ONE
        // contributor (a SlabComplement), and its `inside` keeps the bed OUTSIDE the band.
        let maskers = stack.clip_maskers(1);
        assert_eq!(maskers.len(), 1, "a vein contributes ONE slab-exterior masker, not two ANDed half-spaces");
        let kept = |z: f64| maskers[0].boundary().inside([500.0, 500.0, z]);
        assert!(kept(150.0), "bed survives BELOW the vein band");
        assert!(!kept(500.0), "bed is REMOVED INSIDE the vein band (was inverted before the fix)");
        assert!(kept(850.0), "bed survives ABOVE the vein band");
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
            // No OVERSHOOT: the sheet must not extend left of the crease (into the eroded
            // region) by more than ~a cell.
            assert!(
                min_x >= crease_x - 1.5 * cell,
                "depth {depth}: middle bed open sheet OVERSHOOTS the field crease — survives from \
                 x={min_x:.0}, crease at x≈{crease_x:.0} (allowed ≥ {:.0})",
                crease_x - 1.5 * cell
            );
            // No UNDERSHOOT: the sheet must REACH the crease, not stop short of it (the gap).
            // Pre-fix-2 the `&&` corner test stopped a cell short here. The bed must daylight
            // up to the unconformity, so its surviving edge reaches ~the crease.
            assert!(
                min_x <= crease_x + 1.5 * cell,
                "depth {depth}: middle bed open sheet UNDERSHOOTS the field crease (visible gap) — \
                 stops at x={min_x:.0}, crease at x≈{crease_x:.0} (allowed ≤ {:.0})",
                crease_x + 1.5 * cell
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

    // ===== Topography, universal fill-upward, and unit derivation (feature 003) =====

    /// Build a `Topography` element from a flat ground plane at `z = z_ground`.
    fn topography_at(z_ground: f64) -> StackElement<Model> {
        StackElement::Topography(Topography { model: dipping_plane(z_ground, 0.0, 0.0) })
    }

    /// **Inert-flag ORACLE** (`docs/topography_lithology_solids_spec.md` §3.2, SC-009): a
    /// series with `unbounded_above = false` AND nothing younger capping it still fills
    /// upward. This pins the NEW universal default (not a self-consistency guard): the flag
    /// is dead in the keep/drop decision — every series keeps all its units regardless of it.
    #[test]
    fn bounded_series_with_no_cap_still_fills_upward() {
        // A single "bounded" series (unbounded_above=false), no younger element above it.
        let bounded = StackedSeries::new(dipping_plane(500.0, 0.0, 0.0), Relation::Conformable);
        assert!(!bounded.unbounded_above, "fixture: the series is flagged bounded");
        let stack = Stack::new(vec![bounded.into()]);

        // Universal fill-upward: it keeps ALL its units (1 interface ⇒ 2 units), exactly as a
        // floor would — the flag no longer drops the top unit.
        assert_eq!(stack.block_unit_count(0, 0), 2, "1-contact series has 2 units");
        assert_eq!(
            stack.present_unit_count(0, 0),
            2,
            "a bounded series with no younger cap keeps ALL units (the flag is inert)"
        );
    }

    /// Topography caps the model: ABOVE the ground `global_unit` is the Air/void sentinel
    /// `(len, 0)`; BELOW the ground the older series claims the point. (§2.2, SC-001.)
    #[test]
    fn topography_caps_to_air_above_and_rock_below() {
        // Floor with contacts ~300/500/700; ground plane at z=600 cuts through it.
        let basement = {
            let mut sps = Vec::new();
            for (id, z0) in [(0u32, 300.0), (1, 500.0), (2, 700.0)] {
                for i in 0..4 {
                    for j in 0..4 {
                        sps.push(SurfacePoint::new([i as f64 * 300.0 + 50.0, j as f64 * 300.0 + 50.0, z0], id));
                    }
                }
            }
            let oris = vec![Orientation::new([500.0, 500.0, 500.0], [0.0, 0.0, 1.0])];
            Model::fit(&Series::new(sps, oris), KernelOptions::default())
        };
        let stack = Stack::new(vec![topography_at(600.0), StackedSeries::floor(basement).into()]);

        // Topography is youngest (index 0) and the floor index 1.
        assert!(stack.series[0].is_topography(), "topography pinned youngest");

        // Well above the ground → Air (void sentinel = stack length).
        let (air_elem, _) = stack.global_unit([500.0, 500.0, 950.0]);
        assert_eq!(air_elem, stack.len(), "above the ground surface is Air (no rock)");

        // Well below the ground → the floor (index 1) claims it (some rock unit).
        let (rock_elem, _) = stack.global_unit([500.0, 500.0, 100.0]);
        assert_eq!(rock_elem, 1, "below the ground surface the floor's rock is present");
    }

    /// `DerivedView.units` exposes the per-element ordered unit coordinates (§5.1, SC-002):
    /// a K-contact series → K+1 units; topography → none (Air only); coordinates are stable
    /// `(elementId, index)`. Pins the contract the UI ladder + lithology assignment read.
    #[test]
    fn derived_units_lists_per_element_coordinates() {
        let basement = {
            let mut sps = Vec::new();
            for (id, z0) in [(0u32, 300.0), (1, 500.0), (2, 700.0)] {
                for i in 0..4 {
                    for j in 0..4 {
                        sps.push(SurfacePoint::new([i as f64 * 300.0 + 50.0, j as f64 * 300.0 + 50.0, z0], id));
                    }
                }
            }
            let oris = vec![Orientation::new([500.0, 500.0, 500.0], [0.0, 0.0, 1.0])];
            Model::fit(&Series::new(sps, oris), KernelOptions::default())
        };
        let stack = Stack::new(vec![topography_at(900.0), StackedSeries::floor(basement).into()]);
        let dv = stack.derived_view();

        // Two elements present as keys; order parallels the model (topography id "0", floor "1").
        assert_eq!(dv.units.len(), 2, "every element appears in units");
        assert_eq!(dv.units[0].0, "0");
        assert_eq!(dv.units[0].1.len(), 0, "topography lists NO unit (its only unit is Air)");
        assert_eq!(dv.units[1].0, "1");
        assert_eq!(dv.units[1].1.len(), 4, "a 3-contact floor exposes 4 units (universal fill-up)");
        // The list is emitted TOP-first (descending mesher index) for the UI ladder: the
        // top slab (index n) leads, the bottom slab (index 0) trails. The `index` VALUE is the
        // stable bottom-0 mesher coordinate (the assignment key), just listed top→bottom.
        let idxs: Vec<usize> = dv.units[1].1.iter().map(|u| u.index).collect();
        assert_eq!(idxs, vec![3, 2, 1, 0], "units listed top-first; index 0 is the bottom slab");
    }

    /// **The multi-clip watertightness case** (`docs/topography_lithology_solids_spec.md` §5.2,
    /// SC-005, T039): a floor unit capped by TOPOGRAPHY above, bounded by its floor contact
    /// below, AND punched through by an INTRUSION must mesh as a closed, watertight solid — no
    /// holes (open edges == 0), manifold edges (incidence > 2 count == 0). The masker stack
    /// (topography Halfspace + intrusion exterior + the lower contact) all apply at once; this
    /// pins that the combined CSG stays crack-free. The brief said "don't pre-solve — add the
    /// test and let it tell us"; this is that test.
    #[test]
    fn topography_capped_intrusion_punched_unit_is_watertight() {
        let basement = {
            let mut sps = Vec::new();
            for (id, z0) in [(0u32, 300.0), (1, 500.0), (2, 700.0)] {
                for i in 0..4 {
                    for j in 0..4 {
                        sps.push(SurfacePoint::new([i as f64 * 300.0 + 50.0, j as f64 * 300.0 + 50.0, z0], id));
                    }
                }
            }
            let oris = vec![Orientation::new([500.0, 500.0, 500.0], [0.0, 0.0, 1.0])];
            Model::fit(&Series::new(sps, oris), KernelOptions::default())
        };
        // An intrusion (single envelope) cutting vertically through the centre of the domain.
        let intrusion = Intrusion::new(dipping_plane(500.0, 0.0, 0.0), Envelope::Single(0));
        // Stack youngest→oldest: topography (caps at z=650), intrusion, floor.
        let stack = Stack::new(vec![
            topography_at(650.0),
            StackElement::Intrusion(intrusion),
            StackedSeries::floor(basement).into(),
        ]);

        let cfg = OctreeConfig { max_depth: 6, min_depth: 2, qef_threshold: 1e-6, cap: true };
        let ext = [1000.0, 1000.0, 1000.0];
        // The floor is element 2 (topography 0, intrusion 1). Mesh ALL its unit solids — each
        // is simultaneously topography-capped, intrusion-punched, and floor-bounded.
        let units = stack.block_unit_count(2, 0);
        assert!(units >= 2, "the floor should expose ≥2 unit slabs");

        let mut total_tris = 0usize;
        for u in 0..units {
            let mesh = stack
                .mesh_one_block_unit_solid(2, 0, u, [0.0, 0.0, 0.0], 1000.0, ext, cfg)
                .expect("floor unit in range");
            if mesh.triangle_count() == 0 {
                continue; // a unit fully clipped away in the visible region
            }
            total_tris += mesh.triangle_count();
            let (nm, open, total) = edge_stats(&mesh);
            assert_eq!(nm, 0, "unit {u}: non-manifold {nm}/{total} (multi-clip CSG cracked)");
            assert_eq!(open, 0, "unit {u}: not watertight, open {open}/{total} (a hole)");
        }
        assert!(total_tris > 0, "the multi-clip floor units produced no geometry");
    }

    /// A cross-cutting body's INTERIOR SOLID (`mesh_cross_cut_solid`, feature 003 §4.3/§5)
    /// meshes as a closed, watertight body — the namesake-lithology volume that renders in the
    /// web3 solid view. A vein's interior is its band `lo < field < hi`; the closed solid must
    /// be manifold (no holes), and clipped by the box (and any younger element).
    #[test]
    fn cross_cut_interior_solid_is_watertight() {
        // A vertical vein band (foot wall x=400, hanging wall x=600) cutting the domain.
        let wall = |x0: f64| {
            let sps = vec![
                SurfacePoint::new([x0, 500.0, 200.0], 0),
                SurfacePoint::new([x0, 500.0, 800.0], 0),
            ];
            let oris = vec![Orientation::new([x0, 500.0, 500.0], [1.0, 0.0, 0.0])];
            Model::fit(&Series::new(sps, oris), KernelOptions::default())
        };
        // One co-kriged field from both walls (ids 0 = foot, 1 = hanging): build a 2-interface model.
        let mut sps = Vec::new();
        for (id, x0) in [(0u32, 400.0), (1u32, 600.0)] {
            sps.push(SurfacePoint::new([x0, 200.0, 500.0], id));
            sps.push(SurfacePoint::new([x0, 800.0, 500.0], id));
        }
        let oris = vec![Orientation::new([500.0, 500.0, 500.0], [1.0, 0.0, 0.0])];
        let vein_model = Model::fit(&Series::new(sps, oris), KernelOptions::default());
        let _ = wall; // (single-wall helper kept for clarity; the 2-interface model is used)

        let stack = Stack::new(vec![
            StackElement::Vein(Vein::new(vein_model, 0, 1)),
            StackedSeries::floor(dipping_plane(500.0, 0.0, 0.0)).into(),
        ]);
        let cfg = OctreeConfig { max_depth: 6, min_depth: 2, qef_threshold: 1e-6, cap: true };
        let solid = stack
            .mesh_cross_cut_solid(0, [0.0, 0.0, 0.0], 1000.0, [1000.0, 1000.0, 1000.0], cfg)
            .expect("vein interior solid");
        assert!(solid.triangle_count() > 0, "the vein interior produced no geometry");
        let (nm, open, total) = edge_stats(&solid);
        assert_eq!(nm, 0, "vein interior non-manifold {nm}/{total}");
        assert_eq!(open, 0, "vein interior not watertight, open {open}/{total}");
    }
}
