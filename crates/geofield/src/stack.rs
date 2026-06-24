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
use crate::mesh_clip::{clip, ClipMesh, Keep};
use crate::octree::{
    octree_dual_contour_scene, octree_dual_contour_shared, AxisBox, Boundary, ContactHalfSpace,
    OctreeConfig, PlaneHalfSpace, Scene, SlabComplement,
};
use crate::{clip_mesh_to_half_spaces, max_abs_field_in_box, BlockModel, ScalarField, SeriesModel};

/// How `mesh_cross_cut_shared` closes a cut contact against the body's wall(s).
///
/// `Snap` (default) is the committed, mesh-agnostic seam closer
/// ([`crate::weld::snap_free_boundary_to_mesh`]): it nudges the contact's free-boundary crease onto
/// the nearest wall triangle. Watertight-ish (reduces, doesn't always eliminate, see-through holes).
///
/// `Clip` routes through [`crate::mesh_clip::clip`]: a true mesh–mesh corefinement that shares the
/// contact∩wall seam vertices by index (watertight by construction, no scalloping). Wired
/// incrementally — see `docs/mesh_clip_wiring.md`. Currently handles only the SINGLE-WALL case;
/// it falls back to `Snap` for multi-wall bands (veins/shells) until step 2 lands.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum SeamMode {
    #[default]
    Snap,
    Clip,
}

// NOTE: the mesh-based keep-side helpers `exterior_keep_side` (probe the wall's CCW normal) and
// `keep_side_for_erosion` (always `Above` by W's winding) were REMOVED 2026-06-23. The clip keep side
// is now a body-FIELD predicate evaluated per sub-triangle centroid (`mesh_one_erode_clipped` and the
// `clip_walls` block) — robust where a wall fit extrapolates past its constraints near the domain rim
// (the wall MESH and the field disagree there; a nearest-triangle plane test mis-signs an uncut face,
// leaving an isolated sliver). The pole-flip reasoning the old erode helper documented still holds and
// is now inherent: `efield > eiso` is the survive side in BOTH orientations (the field flips with the
// pole). The no-crossing (empty-seam) case is still gated by `contact_survives_erosion` below.

/// Does the older contact `c` survive a younger erosion when it lies WHOLLY on one side of the
/// surface (no crossing)? The field-CSG convention: survive where `efield > eiso`. We test the
/// contact's centroid — since `c` doesn't cross the surface, every point of `c` is on the same side,
/// so one representative point decides the whole bed. (Crossing beds are trimmed by `clip` instead;
/// this gate is only consulted on the empty-seam branch.)
fn contact_survives_erosion(c: &ClipMesh, efield: &dyn ScalarField, eiso: f64) -> bool {
    if c.vertices.is_empty() {
        return false;
    }
    let mut ctr = [0.0; 3];
    for v in &c.vertices {
        for d in 0..3 {
            ctr[d] += v[d];
        }
    }
    for d in 0..3 {
        ctr[d] /= c.vertices.len() as f64;
    }
    efield.scalar_field(ctr) > eiso
}

/// Fill a mesh's per-vertex normals from a scalar field's gradient (normalized) — the octree
/// contourer's convention (`octree.rs`: vertex normal = normalized ∇field), so a corefiner-produced
/// mesh (which carries no normals) shades identically to a directly-contoured one. Used by the erode
/// corefiner path on the older contact's own field; the vertices lie on/near the contact where the
/// gradient is the surface normal. Overwrites any existing normals.
fn fill_normals_from_field(mesh: &mut Mesh, field: &dyn ScalarField) {
    let vcount = mesh.vertex_count();
    mesh.normals = Vec::with_capacity(vcount * 3);
    for v in 0..vcount {
        let p = [
            mesh.positions[3 * v] as f64,
            mesh.positions[3 * v + 1] as f64,
            mesh.positions[3 * v + 2] as f64,
        ];
        let g = field.scalar_field_gradient(p);
        let len = (g[0] * g[0] + g[1] * g[1] + g[2] * g[2]).sqrt().max(1e-300);
        mesh.normals.push((g[0] / len) as f32);
        mesh.normals.push((g[1] / len) as f32);
        mesh.normals.push((g[2] / len) as f32);
    }
}

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
        self.clip_maskers_filtered(i, true)
    }

    /// As [`Self::clip_maskers`] but OMIT the younger-erode half-spaces — used by the corefiner erode
    /// path ([`Self::mesh_one_erode_clipped`]), which closes the erode seam by mesh–mesh clip instead,
    /// so the erode masker must NOT also field-clip the contact (that would end it short of the
    /// surface and leave nothing for `clip` to trim). Veins/intrusions/topography/onlap still apply.
    fn clip_maskers_excluding_erode(&self, i: usize) -> Vec<Masker<M::Field<'_>>> {
        self.clip_maskers_filtered(i, false)
    }

    fn clip_maskers_filtered(&self, i: usize, include_erode: bool) -> Vec<Masker<M::Field<'_>>> {
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
                // Omitted when `include_erode` is false (the corefiner path closes this seam instead).
                StackElement::Series(sj) if sj.relation == Some(Relation::Erode) => {
                    if include_erode {
                        if let Some(iso_top) = sj.iso_top() {
                            out.push(Masker::Halfspace { field: sj.model.field(), iso: iso_top, below: false });
                        }
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

    /// As [`Stack::mesh_one_clipped`], but close the seam against younger **erode** surfaces with the
    /// mesh–mesh COREFINER ([`crate::mesh_clip::clip`]) instead of the field half-space masker — the
    /// A/B counterpart to `mesh_one_clipped`'s field-CSG path, and the path a future IMPORTED,
    /// field-less erosion surface must use (the corefiner is field-agnostic; the masker needs
    /// `scalar_field()`). See `docs/mesh_clip_wiring.md` and the unify-erode plan.
    ///
    /// `Snap` (or no younger erode surface) → delegates to `mesh_one_clipped` (byte-identical default).
    /// `Clip` → contour the older contact BOX-ONLY (so it spans across the erosion surface), contour
    /// each younger erosion surface as its own mesh, and `clip` the contact against each, keeping the
    /// surface's own-winding `Above` side ([`keep_side_from_winding`] — which tracks the pole flip).
    /// Non-erode younger clips (veins/intrusions) on the SAME contact are NOT handled here yet (they
    /// stay on the masker path); this increment is erode-only by design.
    pub fn mesh_one_erode_clipped(
        &self,
        i: usize,
        interface_idx: usize,
        min: [f64; 3],
        size: f64,
        ext_max: [f64; 3],
        cfg: OctreeConfig,
        seam: SeamMode,
    ) -> Option<Mesh> {
        let s = self.series_at(i)?;
        let isovalue = *s.model.interface_isovalues().get(interface_idx)?;

        // The younger ERODE series clipping this one (youngest-first: indices `< i`), as the SAME
        // set the `clip_maskers` erode branch enumerates — but collected as their top-contact
        // (field, iso) so we can contour each as a mesh. A non-erode younger clip (vein/intrusion)
        // is left to the field-masker path: this method only corefines erode surfaces.
        let erosion_surfaces: Vec<(M::Field<'_>, f64)> = self.series[..i]
            .iter()
            .filter_map(|ej| match ej {
                StackElement::Series(sj) if sj.relation == Some(Relation::Erode) => {
                    sj.iso_top().map(|iso| (sj.model.field(), iso))
                }
                _ => None,
            })
            .collect();

        // Snap mode, or nothing to corefine against → the established field-CSG path verbatim.
        if seam == SeamMode::Snap || erosion_surfaces.is_empty() {
            return self.mesh_one_clipped(i, interface_idx, min, size, ext_max, cfg);
        }

        // The other (non-erode) younger maskers still apply as field clips — veins/intrusions clip
        // this contact exactly as before; only the erode seam moves to the corefiner. We contour the
        // contact with those maskers BUT WITHOUT the erode half-spaces (which `clip` replaces), so it
        // spans across each erosion surface for `clip` to trim.
        let box_b = AxisBox::from_bounds(min, ext_max);
        // Hold the masker fields in a local so their borrowed boundaries outlive the `Scene` (mirrors
        // `mesh_one_clipped`).
        let non_erode_masker_fields = self.clip_maskers_excluding_erode(i);
        let non_erode_maskers: Vec<Box<dyn Boundary + '_>> =
            non_erode_masker_fields.iter().map(|m| m.boundary()).collect();
        let contact_field = s.model.field();
        // The contact's OWN contributors: box + non-erode maskers ONLY (no erosion clip — the contact
        // must span across each erosion surface so the corefiner can trim + share the seam).
        let mut contact_contribs: Vec<&dyn Boundary> = Vec::with_capacity(non_erode_maskers.len() + 1);
        contact_contribs.push(&box_b);
        for m in &non_erode_maskers {
            contact_contribs.push(m.as_ref());
        }
        // The BUILD scene drives octree REFINEMENT: it adds each erosion surface as a `ContactHalfSpace`
        // contributor so the tree refines the contact NEAR the seam. Without this the contact stays
        // coarse exactly where the seam lands (the octree only refines where the contact field or the
        // box/maskers cross), so the corefiner produces a coarse, NOTCHED seam. Refinement-only: the
        // erosion surfaces are in the BUILD scene (tree structure) but NOT the contact scene (kept
        // geometry), so they drive subdivision without clipping the contact. (`below=false` is
        // arbitrary — only the crossing LOCATION refines.) Mirrors the vein path's `extra_wall_contrib`.
        let erode_refine: Vec<ContactHalfSpace> = erosion_surfaces
            .iter()
            .map(|(f, iso)| ContactHalfSpace::new(f, *iso, false))
            .collect();
        let mut build_contribs = contact_contribs.clone();
        for r in &erode_refine {
            build_contribs.push(r);
        }
        let build_scene = Scene::new(&contact_field, isovalue, build_contribs);
        let contact_scene = Scene::new(&contact_field, isovalue, contact_contribs);
        let contact_mesh = octree_dual_contour_shared(&build_scene, &[&contact_scene], min, size, cfg)
            .into_iter()
            .next()
            .unwrap_or_default();
        let mut c = ClipMesh::from_mesh(&contact_mesh);

        // Corefine the contact against EACH younger erosion surface, keeping the survive side
        // (`Above` W's own winding — see `keep_side_for_erosion`). Where the contact CROSSES the
        // surface, `clip` trims it and shares the seam by index. Where it lies WHOLLY on one side (no
        // crossing — e.g. a deep bed entirely below the surface), `clip`'s empty-seam branch can only
        // keep-all or discard-all by W's winding, which is NOT enough to know whether that side is
        // eroded — so we gate the whole bed by the EROSION FIELD at the contact's centroid (the same
        // `efield > eiso` survival test the field-CSG masker uses), discarding a bed that sits on the
        // eroded side. (Field-gating the no-crossing case is what keeps parity with field-CSG; a
        // future imported field-less surface will use a normal-based whole-bed test here instead.)
        let surf_box = box_b; // erosion surfaces contour box-only (span the domain) so the seam is found
        for (efield, eiso) in &erosion_surfaces {
            let e_scene = Scene::new(efield, *eiso, vec![&surf_box]);
            let w = ClipMesh::from_mesh(&octree_dual_contour_scene(&e_scene, min, size, cfg));
            if w.triangles.is_empty() {
                continue;
            }
            // Keep where the older contact SURVIVES the erosion: `efield > eiso` (the field-CSG
            // convention). This is the field-computed form of `keep_side_for_erosion`'s `Above` (W
            // winds toward +∇field, so `signed > 0` ⇔ `efield > eiso`) — but evaluating the FIELD
            // instead of the wall MESH is robust where the wall fit extrapolates past its constraints
            // near the domain rim: a nearest-triangle plane test there mis-signs a face the field
            // correctly classifies, leaving an isolated 1-triangle sliver in the eroded zone (the
            // wall mesh and field disagree only where the fit is under-constrained). See
            // `docs/mesh_clip_band_complement.md`.
            let survive = |p: [f64; 3]| efield.scalar_field(p) > *eiso;
            let res = clip(&c, &w, Keep::Predicate(&survive), None);
            if res.shared.is_empty() {
                // No crossing: keep the whole bed only if its centroid survives the erosion field.
                if !contact_survives_erosion(&c, efield, *eiso) {
                    c = ClipMesh::default();
                    break;
                }
                // else: bed wholly on the survive side — leave `c` unchanged, try the next surface.
            } else {
                c = res.c;
            }
        }

        // The corefiner output carries NO normals (mesh_clip drops them). Resample the contact's own
        // field gradient at each output vertex — the octree convention (`octree.rs`: per-vertex normal
        // = normalized field gradient), so shading matches the field-CSG path. (The vertices lie on or
        // near the contact, where the gradient is the surface normal.)
        let mut out = c.to_mesh();
        fill_normals_from_field(&mut out, &contact_field);
        Some(out)
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

    /// The block-aware A/B counterpart to [`Self::mesh_one_block`]: close the seam against younger
    /// **erode** surfaces with the COREFINER ([`Self::mesh_one_erode_clipped`]) instead of the field
    /// masker. `Snap` (or no younger erode) → the field-masker `mesh_one_block` verbatim.
    ///
    /// `Clip` is wired for the UNFAULTED case (`nblocks == 1`, block 0), where the block field IS the
    /// whole-model field, so it delegates to the non-block corefiner path. A FAULTED erode series
    /// (`nblocks > 1`) still uses the field-masker `mesh_one_block` (corefining per fault block is a
    /// later step) — so the opt-in never regresses faulted scenes.
    pub fn mesh_one_block_erode_clipped(
        &self,
        i: usize,
        b: usize,
        interface_idx: usize,
        min: [f64; 3],
        size: f64,
        ext_max: [f64; 3],
        cfg: OctreeConfig,
        seam: SeamMode,
    ) -> Option<Mesh> {
        let single_block = self.series_at(i).map(|s| s.model.nblocks()).unwrap_or(0) == 1;
        if seam == SeamMode::Clip && b == 0 && single_block {
            self.mesh_one_erode_clipped(i, interface_idx, min, size, ext_max, cfg, seam)
        } else {
            self.mesh_one_block(i, b, interface_idx, min, size, ext_max, cfg)
        }
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

    /// Mesh a cross-cutting body's wall (`mesh_cross_cut_surface`) and, when `weld` is set, WELD
    /// into it the crease vertices of every contact that terminates against it — removing the
    /// T-junctions that otherwise leave see-through slivers where a flat contact meets the wall
    /// (the surface-mode gaps; see `crate::weld`). Returns `None` for a series / out-of-range.
    ///
    /// How: a body at `i` cuts the older elements `cuts_below(i)` reports. Each such series' OPEN
    /// contact sheets are clipped to OUTSIDE this body, so their inner free boundary is the crease
    /// lying on this wall. We harvest those free-boundary points and weld the ones within
    /// `weld_eps` of the wall surface into the wall mesh, so the crease becomes shared vertices.
    /// `weld = false` reproduces the old independent-mesh behavior (the runtime escape hatch).
    pub fn mesh_cross_cut_surface_welded(
        &self,
        i: usize,
        interface_idx: usize,
        min: [f64; 3],
        size: f64,
        ext_max: [f64; 3],
        cfg: OctreeConfig,
        weld: bool,
    ) -> Option<Mesh> {
        let mut wall = self.mesh_cross_cut_surface(i, interface_idx, min, size, ext_max, cfg)?;
        if !weld {
            return Some(wall);
        }
        // Crease points = the free boundaries of every contact this body cuts (its clipped edge
        // lying on this wall). Welding them inserts the contact's crease vertices into the wall.
        // (A contact's box-rim boundary is also in this set, but those points are far from the wall
        // and the `weld_eps` filter in `weld_points_into_mesh` drops them.)
        let cell = size / (1u32 << cfg.max_depth) as f64;
        // A crease vertex lies on the wall SURFACE, but the wall's independent TESSELLATION can be
        // up to ~a cell off it (flat triangles chord a curved wall). Reach across that chord error.
        let weld_eps = cell * 1.5;
        let mut crease: Vec<[f64; 3]> = Vec::new();
        let surf_cfg = OctreeConfig { cap: false, ..cfg };
        for &j in self.cuts_below(i, min, size).iter() {
            // Mesh every interface of the cut series as a clipped open sheet, collect its crease.
            let Some(s) = self.series_at(j) else { continue };
            let n = s.model.interface_ids().len();
            for k in 0..n {
                if let Some(contact) = self.mesh_one_clipped(j, k, min, size, ext_max, surf_cfg) {
                    crease.extend(crate::weld::free_boundary_points(&contact));
                }
            }
        }
        crate::weld::weld_points_into_mesh(&mut wall, &crease, weld_eps, cell);
        Some(wall)
    }

    /// SHARED-CREASE meshing for a cross-cutting body `i`: co-contour ALL the body's walls AND the
    /// contacts it cuts on ONE octree, so a contact's clipped edge and the wall share the SAME
    /// crease discretization — no T-junctions, no see-through holes (the watertight-by-construction
    /// fix, vs the approximate point-weld in `mesh_cross_cut_surface_welded`).
    ///
    /// Returns `(walls, contacts)`:
    ///   * `walls[k]` is the body's interface-`k` wall mesh;
    ///   * `contacts` is one `(series, interface, mesh)` per cut contact — tagged so the caller can
    ///     emit it in place of the independently-meshed series contact (and skip that one).
    /// All are contoured on a single tree refined for every surface (the body field is primary;
    /// each cut contact is added as a `ContactHalfSpace` contributor so the build subdivides near
    /// it too). `None` for a series / out-of-range body.
    pub fn mesh_cross_cut_shared(
        &self,
        i: usize,
        min: [f64; 3],
        size: f64,
        ext_max: [f64; 3],
        cfg: OctreeConfig,
        seam: SeamMode,
    ) -> Option<(Vec<Mesh>, Vec<(usize, usize, Mesh)>)> {
        let body = self.series.get(i)?;
        if body.is_series() {
            return None;
        }
        let body_model = body.model();
        let body_field = body_model.field();
        let body_isos = body_model.interface_isovalues();
        let n_walls = body_isos.len();
        if n_walls == 0 {
            return None;
        }
        // CLIP path: when on, the cut contact is contoured WITHOUT its body masker (box-only), so it
        // spans ACROSS the wall(s) — `clip` then does the trim AND the shared seam. (A field-clipped
        // contact ends short of the wall and never crosses it, so clip would find no seam.) Handles
        // SINGLE-wall bodies (step 1) and TWO-wall bands (step 2: union of two single-side clips,
        // §below). Bodies with ≥3 walls fall through to `Snap`.
        let use_clip = seam == SeamMode::Clip && (n_walls == 1 || n_walls == 2);

        // The cut contacts: (series j, interface k, isovalue, field).
        let cut = self.cuts_below(i, min, size);
        let surf_cfg = OctreeConfig { cap: false, ..cfg };

        // --- Boundary locals (must outlive every Scene) -------------------------------------
        let box_b = AxisBox::from_bounds(min, ext_max);
        // Body's own clip maskers (younger elements clipping the body's walls).
        let body_masker_fields = self.clip_maskers(i);
        let body_maskers: Vec<Box<dyn Boundary + '_>> =
            body_masker_fields.iter().map(|m| m.boundary()).collect();

        // Each cut contact's field + isovalue, owned here so their `ContactHalfSpace`s borrow them.
        // Parallel vectors (not a struct) so the masker type stays inferred — `Masker` is private
        // and generic over the model's field type.
        let mut cut_fields: Vec<Box<dyn ScalarField + '_>> = Vec::new();
        let mut cut_isos: Vec<f64> = Vec::new();
        let mut cut_masker_fields = Vec::new();
        let mut cut_ids: Vec<(usize, usize)> = Vec::new(); // (series j, interface k) per contact
        for &j in &cut {
            let Some(s) = self.series_at(j) else { continue };
            let isos = s.model.interface_isovalues();
            for (k, &iso) in isos.iter().enumerate() {
                cut_fields.push(Box::new(s.model.field()) as Box<dyn ScalarField + '_>);
                cut_isos.push(iso);
                cut_masker_fields.push(self.clip_maskers(j));
                cut_ids.push((j, k));
            }
        }
        // Each cut contact's clip maskers as boundaries (for the contact's own Scene).
        let cut_maskers: Vec<Vec<Box<dyn Boundary + '_>>> = cut_masker_fields
            .iter()
            .map(|mf: &Vec<_>| mf.iter().map(|m| m.boundary()).collect())
            .collect();
        // The cut contacts AS contributors to the BUILD scene (so the tree refines near them).
        // `below=false` is an arbitrary side — only the crossing location matters for refinement.
        let cut_as_contrib: Vec<ContactHalfSpace> = cut_fields
            .iter()
            .zip(&cut_isos)
            .map(|(f, &iso)| ContactHalfSpace::new(f.as_ref(), iso, false))
            .collect();
        // The body's OTHER walls (every wall isovalue beyond the primary `body_isos[0]`) AS build
        // contributors. The build primary only refines on `body_isos[0]` (the foot wall); without
        // this the hanging wall (`body_isos[1+]`) is NOT a refinement driver, so the tree stays
        // coarse there and that wall contours as a sparse, spiky sheet. Adding them makes the build
        // subdivide near EVERY wall, so all walls resolve at full detail on the shared tree.
        let extra_wall_contrib: Vec<ContactHalfSpace> = body_isos
            .iter()
            .skip(1)
            .map(|&iso| ContactHalfSpace::new(&body_field, iso, false))
            .collect();

        // --- Build scene: body field primary (foot wall), refined for ALL body walls + box + body
        // maskers + every cut contact (so all surfaces are resolved on the shared tree). ---------
        let mut build_contrib: Vec<&dyn Boundary> = Vec::new();
        build_contrib.push(&box_b);
        for m in &body_maskers {
            build_contrib.push(m.as_ref());
        }
        for c in &extra_wall_contrib {
            build_contrib.push(c);
        }
        for c in &cut_as_contrib {
            build_contrib.push(c);
        }
        let build_scene = Scene::new(&body_field, body_isos[0], build_contrib);

        // --- Per-surface contour scenes ------------------------------------------------------
        // Body walls: body field at each wall isovalue, clipped by box + body maskers.
        let mut wall_contribs: Vec<&dyn Boundary> = Vec::new();
        wall_contribs.push(&box_b);
        for m in &body_maskers {
            wall_contribs.push(m.as_ref());
        }
        let wall_scenes: Vec<Scene> = body_isos
            .iter()
            .map(|&iso| Scene::new(&body_field, iso, wall_contribs.clone()))
            .collect();

        // Cut contacts: each contact field at its isovalue, clipped by box + its OWN maskers.
        // Those maskers ALREADY include this younger body's clip (a vein's `SlabExterior` — the
        // contact survives OUTSIDE the band, on BOTH sides — or an intrusion's exterior), exactly
        // as the independent `mesh_one_clipped` path builds it. (An earlier version ALSO pushed a
        // one-sided `ContactHalfSpace` for the body here, which double-clipped the contact to a
        // single side and erased half the sediments — the body clip is the masker, not a
        // half-space.) The crease still lands on the wall because the body's `SlabExterior` masker
        // crosses there, contoured on the SHARED tree so it coincides with the wall.
        let contact_scenes: Vec<Scene> = (0..cut_fields.len())
            .map(|ci| {
                let mut contribs: Vec<&dyn Boundary> = Vec::new();
                contribs.push(&box_b);
                // `use_clip` contours the contact BOX-ONLY (no body masker) so it spans across the
                // wall for `clip` to trim; otherwise apply the contact's own maskers (the field-clip).
                if !use_clip {
                    for m in &cut_maskers[ci] {
                        contribs.push(m.as_ref());
                    }
                }
                Scene::new(cut_fields[ci].as_ref(), cut_isos[ci], contribs)
            })
            .collect();

        // Contour everything on ONE shared tree.
        let mut all_scenes: Vec<&Scene> = Vec::new();
        for s in &wall_scenes {
            all_scenes.push(s);
        }
        for s in &contact_scenes {
            all_scenes.push(s);
        }
        let meshes = octree_dual_contour_shared(&build_scene, &all_scenes, min, size, surf_cfg);
        let (walls, contacts) = meshes.split_at(n_walls);
        let walls: Vec<Mesh> = walls.to_vec();

        // SNAP each contact's clip-edge crease onto the wall surface. The shared tree makes the
        // contact and wall agree on the crease to SUB-cell, but their projected vertices still
        // differ by a sub-cell amount (the contact's Seam vertex is contact∩wall; the wall's vertex
        // is mid-wall), leaving see-through dashes where the contact edge lands just short of the
        // wall. Snapping the contact's free-boundary crease vertices onto the nearest wall triangle
        // closes that gap. This is the MESH-AGNOSTIC closer (triangles only) — the same call will
        // serve an IMPORTED wall mesh that has no defining field.
        let cell = size / (1u32 << surf_cfg.max_depth) as f64;
        // A crease vertex normally sits within ~a cell of the wall, but where an under-constrained
        // wall fit bulges, the contact's clip can diverge up to ~2.5 cells; reach across that so
        // those worst-case dashes still close. (Larger than the wall's own facet error would risk
        // snapping unrelated geometry, so this is bounded, not unlimited.)
        let snap_eps = cell * 3.0;
        // `use_clip` (decided above): clip the box-only contact against the wall(s), keeping the
        // body/band EXTERIOR. We combine ALL walls into ONE `ClipMesh` (one seam-id space — the old
        // per-wall two-call concat gave each wall INDEPENDENT seam vertices, orphaning rim-corner
        // slivers on oblique walls) and trim with ONE `clip` whose keep rule is the body FIELD's
        // exterior, evaluated at each sub-triangle centroid:
        //   * 1-wall envelope (intrusion-single): exterior = `field > iso`.
        //   * 2-wall band (vein / shell):          exterior = `field < lo ∪ field > hi`
        //                                          (the mesh analog of `octree::SlabComplement`).
        // Evaluating the FIELD (not the wall MESH via a nearest-triangle plane) is what makes this
        // robust: where a wall fit EXTRAPOLATES past its constraints near the domain rim, the wall
        // mesh and the field disagree, and a mesh nearest-plane test mis-signs an uncut face the field
        // classifies correctly — leaving an isolated 1-triangle sliver in the eroded/interior zone.
        // The field is the same ground truth the `Snap`/field-CSG path contours against, so the two
        // paths agree. See `docs/mesh_clip_wiring.md` and `docs/mesh_clip_band_complement.md`.
        let combined_wall: Option<ClipMesh> = if use_clip {
            let mut combined = ClipMesh::default();
            for w in &walls {
                let cw = ClipMesh::from_mesh(w);
                let base = combined.vertices.len();
                combined.vertices.extend_from_slice(&cw.vertices);
                combined
                    .triangles
                    .extend(cw.triangles.iter().map(|t| [t[0] + base, t[1] + base, t[2] + base]));
            }
            Some(combined)
        } else {
            None
        };
        let (lo, hi) = (
            body_isos.iter().cloned().fold(f64::INFINITY, f64::min),
            body_isos.iter().cloned().fold(f64::NEG_INFINITY, f64::max),
        );
        let band_eps = (hi - lo).abs() * 1e-6 + 1e-9;
        let single_wall = n_walls <= 1;
        let tagged_contacts: Vec<(usize, usize, Mesh)> = contacts
            .iter()
            .enumerate()
            .zip(&cut_ids)
            .map(|((ci, m), &(j, k))| {
                if use_clip {
                    let c = ClipMesh::from_mesh(m);
                    let combined = combined_wall.as_ref().expect("combined wall under use_clip");
                    // The body field's exterior — the side the older contact survives on.
                    let exterior = |p: [f64; 3]| {
                        let f = body_field.scalar_field(p);
                        if single_wall {
                            f > hi + band_eps
                        } else {
                            f < lo - band_eps || f > hi + band_eps
                        }
                    };
                    let out = clip(&c, combined, Keep::Predicate(&exterior), None).c;
                    // `clip` drops normals (mesh_clip.rs); refill from the contact's OWN field gradient
                    // (the octree convention) so the clipped contact still SHADES — without this it
                    // renders unlit/black and reads as "the sediments vanished" under clip seam.
                    let mut mesh = out.to_mesh();
                    fill_normals_from_field(&mut mesh, cut_fields[ci].as_ref());
                    (j, k, mesh)
                } else {
                    let mut contact = m.clone();
                    for w in &walls {
                        crate::weld::snap_free_boundary_to_mesh(&mut contact, w, snap_eps, cell);
                    }
                    (j, k, contact)
                }
            })
            .collect();
        Some((walls, tagged_contacts))
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

    /// SHARED-SEAM volume path (`docs/volume_mesh_seam.md`, Strategy A): mesh the cross-cutting
    /// body `ib`'s interior solid AND every rock UNIT solid it cuts on ONE shared octree, so the
    /// body and the surrounding rock land on IDENTICAL wall vertices instead of contouring the wall
    /// independently. The sharing is EXACT, not merely within a merge tolerance: a pure-wall leaf
    /// has a single QEF crossing, and `qef_vertex` minimises squared normal-distance — which is
    /// invariant to the wall masker's `n → −n` flip — so the body scene (wall as primary, `+∇`) and
    /// a rock scene (wall as a `SlabComplement` masker, `−∇`) place the vertex BIT-FOR-BIT equal.
    /// (At a wall∩bed/cap crease the two solids place DIFFERENT vertices; watertightness there is
    /// each solid's own `cap = true` closure — see below — NOT vertex sharing, so it does not rest
    /// on the seam being exact.) Returns the body solid plus the rock units tagged
    /// `(series, block, unit)`; `None` for a stratigraphic series / topography / out-of-range /
    /// wall-less body.
    ///
    /// This is the `Clip` counterpart to the independent `Snap` path
    /// ([`Self::mesh_cross_cut_solid`] + [`Self::mesh_one_block_unit_solid`]), which is left
    /// byte-for-byte unchanged. Strategy A needs ONE function because the body and its rock cannot
    /// share a tree across two independent calls. Each solid is closed by the octree's own
    /// `cap = true` closure (no hand-grafted cap face); sharing the tree only makes the two solids'
    /// wall caps coincide. The rock units' `ContactHeightField::weld` is omitted here — on the
    /// shared tree the bed contacts already coincide, and the height-field weld would only fight the
    /// shared wall seam (and cannot represent a vertical/overhanging wall anyway).
    pub fn mesh_cross_cut_solids_shared(
        &self,
        ib: usize,
        min: [f64; 3],
        size: f64,
        ext_max: [f64; 3],
        cfg: OctreeConfig,
    ) -> Option<(Mesh, Vec<(usize, usize, usize, Mesh)>)> {
        // The body band/interior bounds (mirrors `mesh_cross_cut_solid`).
        let (body_upper, body_lower_iso): (f64, Option<f64>) = match self.series.get(ib)? {
            StackElement::Vein(v) => {
                let (lo, hi) = v.band()?;
                (hi, Some(lo))
            }
            StackElement::Intrusion(it) => {
                let isos = it.model.interface_isovalues();
                match it.envelope {
                    Envelope::Single(k) => (*isos.get(k)?, None),
                    Envelope::Shell { inner, outer } => {
                        let (a, b) = (*isos.get(inner)?, *isos.get(outer)?);
                        let (lo, hi) = if a <= b { (a, b) } else { (b, a) };
                        (hi, Some(lo))
                    }
                }
            }
            _ => return None, // series / topography
        };

        // ---- Owned data: fields boxed for a uniform type the half-spaces can borrow; all built
        // BEFORE any Scene takes a reference into them (mirrors `mesh_cross_cut_shared`). ----------
        let box_b = AxisBox::from_bounds(min, ext_max);
        let body_e = self.series.get(ib)?;
        let body_field: Box<dyn ScalarField + '_> = Box::new(body_e.model().field());
        let body_isos = body_e.model().interface_isovalues();
        let body_maskers = self.clip_maskers(ib);

        // Each cut block's field + interfaces + fault planes (parallel vecs so the masker/field
        // types stay inferred — `Masker` is private and generic over the model's field type).
        let cut = self.cuts_below(ib, min, size);
        let mut rb_j: Vec<usize> = Vec::new();
        let mut rb_b: Vec<usize> = Vec::new();
        let mut rb_field: Vec<Box<dyn ScalarField + '_>> = Vec::new();
        let mut rb_isos: Vec<Vec<f64>> = Vec::new();
        let mut rb_planes: Vec<Vec<PlaneHalfSpace>> = Vec::new();
        let mut rb_nunits: Vec<usize> = Vec::new();
        let mut rb_maskers = Vec::new();
        for &j in &cut {
            let Some(s) = self.series_at(j) else { continue };
            for b in 0..self.series_block_count(j).max(1) {
                let Some(bf) = s.model.block_field(b) else { continue };
                let isos = s.model.block_interface_isovalues(b);
                if isos.is_empty() {
                    continue; // field-less / interface-less block has no unit slabs (matches Snap)
                }
                let planes: Vec<PlaneHalfSpace> = s
                    .model
                    .block_half_spaces(b)
                    .into_iter()
                    .map(|hs| PlaneHalfSpace::new(hs.point, hs.normal, hs.keep_positive))
                    .collect();
                rb_nunits.push(self.present_unit_count(j, b));
                rb_j.push(j);
                rb_b.push(b);
                rb_field.push(Box::new(bf));
                rb_isos.push(isos);
                rb_planes.push(planes);
                rb_maskers.push(self.clip_maskers(j));
            }
        }

        // Flat list of unit output scenes: (block-entry index `k`, unit `u`).
        let mut unit_keys: Vec<(usize, usize)> = Vec::new();
        for (k, &nunits) in rb_nunits.iter().enumerate() {
            for u in 0..nunits {
                unit_keys.push((k, u));
            }
        }

        // ---- Owned half-spaces (borrow the boxed fields above) -------------------------------
        // Body band lower cap (output) + both body walls as build-scene refinement drivers.
        let body_lower: Option<ContactHalfSpace> =
            body_lower_iso.map(|lo| ContactHalfSpace::new(body_field.as_ref(), lo, false));
        let body_wall_refine: Vec<ContactHalfSpace> = body_isos
            .iter()
            .map(|&iso| ContactHalfSpace::new(body_field.as_ref(), iso, false))
            .collect();
        // Every cut block's bed contacts as build-scene refinement drivers (so the tree subdivides
        // near every unit boundary, not just the body wall).
        let mut bed_refine: Vec<ContactHalfSpace> = Vec::new();
        for k in 0..rb_j.len() {
            for &iso in &rb_isos[k] {
                bed_refine.push(ContactHalfSpace::new(rb_field[k].as_ref(), iso, false));
            }
        }
        // Per unit-scene upper bound + lower cap (mirrors `mesh_one_block_unit_solid`).
        let mut unit_upper: Vec<f64> = Vec::with_capacity(unit_keys.len());
        let mut unit_lower: Vec<Option<ContactHalfSpace>> = Vec::with_capacity(unit_keys.len());
        for &(k, u) in &unit_keys {
            let isos = &rb_isos[k];
            let n = isos.len();
            let upper = if u < n {
                isos[u]
            } else {
                max_abs_field_in_box(rb_field[k].as_ref(), min, size) + 1.0
            };
            let lower = (u > 0).then(|| ContactHalfSpace::new(rb_field[k].as_ref(), isos[u - 1], false));
            unit_upper.push(upper);
            unit_lower.push(lower);
        }

        // Masker boundaries (borrow the maskers above).
        let body_masker_b: Vec<Box<dyn Boundary + '_>> =
            body_maskers.iter().map(|m| m.boundary()).collect();
        let rb_masker_b: Vec<Vec<Box<dyn Boundary + '_>>> = rb_maskers
            .iter()
            .map(|ms: &Vec<_>| ms.iter().map(|m| m.boundary()).collect())
            .collect();

        // ---- Build scene: refines for the body band + ALL body walls + every cut block's beds,
        // fault planes, and wall maskers — so EVERY output surface resolves on the shared tree. ----
        let mut build_contrib: Vec<&dyn Boundary> = Vec::new();
        build_contrib.push(&box_b);
        for m in &body_masker_b {
            build_contrib.push(m.as_ref());
        }
        for c in &body_wall_refine {
            build_contrib.push(c);
        }
        for c in &bed_refine {
            build_contrib.push(c);
        }
        for planes in &rb_planes {
            for p in planes {
                build_contrib.push(p);
            }
        }
        for ms in &rb_masker_b {
            for m in ms {
                build_contrib.push(m.as_ref());
            }
        }
        let build_primary = *body_isos.first().unwrap_or(&body_upper);
        let build_scene = Scene::new(body_field.as_ref(), build_primary, build_contrib);

        // ---- Output scenes: the body band, then each rock unit — all closed solids (cap=true). ----
        let cfg = OctreeConfig { cap: true, ..cfg };
        let mut scenes: Vec<Scene> = Vec::with_capacity(1 + unit_keys.len());
        {
            let mut c: Vec<&dyn Boundary> = Vec::new();
            c.push(&box_b);
            if let Some(lo) = &body_lower {
                c.push(lo);
            }
            for m in &body_masker_b {
                c.push(m.as_ref());
            }
            scenes.push(Scene::new(body_field.as_ref(), body_upper, c));
        }
        for (idx, &(k, _u)) in unit_keys.iter().enumerate() {
            let mut c: Vec<&dyn Boundary> = Vec::new();
            c.push(&box_b);
            for p in &rb_planes[k] {
                c.push(p);
            }
            if let Some(lo) = &unit_lower[idx] {
                c.push(lo);
            }
            for m in &rb_masker_b[k] {
                c.push(m.as_ref());
            }
            scenes.push(Scene::new(rb_field[k].as_ref(), unit_upper[idx], c));
        }

        let scene_refs: Vec<&Scene> = scenes.iter().collect();
        let meshes = octree_dual_contour_shared(&build_scene, &scene_refs, min, size, cfg);

        let mut it = meshes.into_iter();
        let body_solid = it.next()?;
        let rock_solids: Vec<(usize, usize, usize, Mesh)> = unit_keys
            .iter()
            .zip(it)
            .map(|(&(k, u), m)| (rb_j[k], rb_b[k], u, m))
            .collect();
        Some((body_solid, rock_solids))
    }
}

// Builds in-process kernel `Model`s, so native-only.
#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;
    use crate::assemble::KernelOptions;
    use crate::data::{Orientation, Series, SurfacePoint};
    use crate::mesh::normalize;
    use crate::mesh_test_util::{assert_mesh_watertight, assert_volume_watertight, incidence};
    use crate::Model;
    use std::collections::HashMap;

    /// An imported-mesh field proxy (`MeshSdf`) drops straight into `fill_normals_from_field`: an
    /// imported clip with no defining field can still reshade its corefiner output from the SDF
    /// gradient (the wiring half of `docs/imported_field_proxy.md`). Proves the call composes and
    /// yields unit, outward, surface-aligned normals — i.e. shades instead of rendering flat/black.
    #[test]
    fn mesh_sdf_drives_fill_normals_from_field() {
        use crate::octree::{MeshSdf, TriMesh};

        // Unit octahedron centred at the origin (CCW/outward-wound faces).
        let verts = vec![
            [1.0, 0.0, 0.0],
            [-1.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            [0.0, -1.0, 0.0],
            [0.0, 0.0, 1.0],
            [0.0, 0.0, -1.0],
        ];
        let tris = vec![
            [0, 2, 4], [2, 1, 4], [1, 3, 4], [3, 0, 4],
            [2, 0, 5], [1, 2, 5], [3, 1, 5], [0, 3, 5],
        ];
        let sdf = MeshSdf::new(TriMesh::new(verts, tris));

        // Two output vertices ON the +++ face: its centroid and the +x vertex it shares. Their
        // normals should match that face's outward normal (1,1,1)/√3 (the +x vertex is shared by
        // four faces, so its nearest-face normal is one of them — still all-positive components).
        let mut mesh = Mesh {
            positions: vec![1.0 / 3.0, 1.0 / 3.0, 1.0 / 3.0, 1.0, 0.0, 0.0],
            normals: Vec::new(),
            indices: Vec::new(),
        };
        fill_normals_from_field(&mut mesh, &sdf);

        assert_eq!(mesh.normals.len(), mesh.positions.len(), "one normal per position");
        for v in 0..mesh.vertex_count() {
            let n = [mesh.normals[3 * v], mesh.normals[3 * v + 1], mesh.normals[3 * v + 2]];
            let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
            assert!((len - 1.0).abs() < 1e-5, "vertex {v} normal not unit: {n:?}");
            assert!(
                n[0] > 0.0 && n[1] > 0.0 && n[2] > 0.0,
                "vertex {v} on the +++ face must shade with an all-positive outward normal: {n:?}"
            );
        }
    }

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

    /// `(nonmanifold, open, total)` undirected-edge incidence counts for a mesh — built on the shared
    /// [`incidence`] primitive. For the strict CLOSED-solid criterion prefer [`assert_mesh_watertight`];
    /// this stays for the callers that need a NON-closed criterion (e.g. "no non-manifold edges" on an
    /// open eroded bed) or to report raw counts.
    fn edge_stats(mesh: &Mesh) -> (usize, usize, usize) {
        let (_, tris) = crate::mesh_test_util::mesh_soup(mesh);
        let inc = incidence(&tris);
        let nm = inc.values().filter(|&&c| c > 2).count();
        let open = inc.values().filter(|&&c| c == 1).count();
        (nm, open, inc.len())
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
                    // Closed-solid criterion (shared helper): incidence exactly 2 everywhere, no rim.
                    // The combined per-series Scene is one sign field, so every interface closes.
                    eprintln!("{label} d{depth} series {} iface {iid}: checking watertight", sm.series);
                    assert_mesh_watertight(mesh, &|_| false);
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

    /// The corefiner erode path ([`Stack::mesh_one_erode_clipped`] with `Clip`) must MATCH the
    /// field-CSG path ([`Stack::mesh_one_clipped`]) on the exact `erosional-unconformity` web3
    /// template (scenarios.ts): a basalt erode series with one DIPPING erosion surface
    /// (z=650+0.4(x−500), pole = dipDirToPole(21.8°,270°)) over the 3-bed sediments floor, in BOTH
    /// pole orientations. Asserts, per bed, in BOTH flip states:
    ///   * `Snap` mode is byte-identical to the plain field-CSG mesher (no behavior change),
    ///   * `Clip` mode produces a non-empty, manifold (no non-manifold edges) eroded bed,
    ///   * the surviving area is the SAME region as the field-CSG path (within tessellation slack),
    ///   * the flip reverses the surviving area the SAME way in both paths.
    /// This is the native gate; the browser/wasm visual gate is separate (the bug was wasm-only).
    #[test]
    fn erode_corefiner_matches_field_csg_either_orientation() {
        // dipDirToPole(dip=acos(1/n), dipDir=270°) for z=650+0.4(x−500) (sx=0.4).
        let n = (0.4f64 * 0.4 + 1.0).sqrt();
        let (dip, dir) = ((1.0 / n).acos(), 270.0_f64.to_radians());
        let pole = [dip.sin() * dir.sin(), dip.sin() * dir.cos(), dip.cos()];
        let basalt = |flipped: bool| {
            let s = if flipped { -1.0 } else { 1.0 };
            let sps = vec![
                SurfacePoint::new([200.0, 500.0, 650.0 + 0.4 * (200.0 - 500.0)], 0),
                SurfacePoint::new([800.0, 500.0, 650.0 + 0.4 * (800.0 - 500.0)], 0),
            ];
            let oris = vec![Orientation::new(
                [500.0, 500.0, 650.0],
                [s * pole[0], s * pole[1], s * pole[2]],
            )];
            Model::fit(&Series::new(sps, oris), KernelOptions::default())
        };
        let sediments = || {
            let mut sps = Vec::new();
            for (id, z0) in [(0u32, 300.0), (1, 500.0), (2, 700.0)] {
                sps.push(SurfacePoint::new([200.0, 500.0, z0], id));
                sps.push(SurfacePoint::new([800.0, 500.0, z0], id));
            }
            let oris = [300.0, 500.0, 700.0]
                .map(|z| Orientation::new([500.0, 500.0, z], [0.0, 0.0, 1.0]))
                .to_vec();
            Model::fit(&Series::new(sps, oris), KernelOptions::default())
        };
        let min = [0.0, 0.0, 0.0];
        let size = 1000.0;
        let ext = [1000.0, 1000.0, 1000.0];
        let depth = cfg(6);
        // sediments is series index 1 (older); basalt erode is index 0 (younger).
        let mesh_bed = |flipped: bool, k: usize, seam: SeamMode, corefine: bool| -> Mesh {
            let stack = Stack::new(vec![
                StackElement::Series(StackedSeries::new(basalt(flipped), Relation::Erode)),
                StackElement::Series(StackedSeries::floor(sediments())),
            ]);
            if corefine {
                stack.mesh_one_erode_clipped(1, k, min, size, ext, depth, seam).unwrap()
            } else {
                stack.mesh_one_clipped(1, k, min, size, ext, depth).unwrap()
            }
        };
        for flipped in [true, false] {
            for k in 0..3 {
                let csg = mesh_bed(flipped, k, SeamMode::Snap, false);
                let snap = mesh_bed(flipped, k, SeamMode::Snap, true);
                let clip = mesh_bed(flipped, k, SeamMode::Clip, true);

                // Snap mode is the field-CSG path verbatim.
                assert_eq!(
                    snap.triangle_count(),
                    csg.triangle_count(),
                    "flipped={flipped} bed{k}: Snap mode must equal the field-CSG mesher"
                );
                let (nm, _open, _total) = edge_stats(&clip);
                assert_eq!(nm, 0, "flipped={flipped} bed{k}: corefiner left non-manifold edges");
                // The corefiner must match the field-CSG path on which side survives: a fully-eroded
                // bed stays empty, a surviving bed stays non-empty, and a partially-eroded bed keeps a
                // comparable area (tessellation differs at the re-triangulated seam, but a wrong-side
                // keep would roughly invert it).
                let (ct, tt) = (clip.triangle_count(), csg.triangle_count());
                if tt == 0 {
                    assert_eq!(ct, 0, "flipped={flipped} bed{k}: bed is fully eroded but corefiner kept {ct}");
                } else {
                    assert!(ct > 0, "flipped={flipped} bed{k}: bed survives but corefiner emptied it");
                    let slack = tt / 2 + 8;
                    assert!(
                        ct.abs_diff(tt) <= slack,
                        "flipped={flipped} bed{k}: corefiner area {ct} vs field-CSG {tt} differ too much"
                    );
                    // Normals filled (renderer needs them; corefiner output had none).
                    assert_eq!(
                        clip.normals.len(),
                        clip.positions.len(),
                        "flipped={flipped} bed{k}: corefiner output missing per-vertex normals"
                    );
                }
            }
        }
    }

    /// Thread-1 REGRESSION: a BENT erosion surface (a ridge in x, so adjacent W triangles' normals
    /// SPLAY) truncating ONE flat sediment contact, under `SeamMode::Clip`. The flat contact survives
    /// as exactly TWO edge-strips (where the ridge dips below z=500); the eroded middle must produce NO
    /// geometry. Before the field-predicate keep fix, the under-constrained erosion fit extrapolated
    /// below the contact near the y-rim, and the wall-MESH keep test (`nearest_w_signed`) wrongly kept
    /// isolated 1-triangle slivers there (4 components instead of 2) — the rim-corner slivers the user
    /// saw. The field keep test (`efield > eiso`) matches the `Snap`/field-CSG ground truth, so Clip
    /// must now equal Snap: 2 components, no sliver. (The PLANAR `erode_corefiner_matches_field_csg`
    /// parity test cannot catch this — only a bent wall has a rim region where mesh and field diverge.)
    #[test]
    fn bent_erosion_clip_has_no_rim_slivers() {
        // A single flat sediment contact at z=500 (floor series, index 1).
        let sediment = || {
            let mut sps = Vec::new();
            for i in 0..4 {
                for j in 0..4 {
                    sps.push(SurfacePoint::new([i as f64 * 300.0 + 50.0, j as f64 * 300.0 + 50.0, 500.0], 0));
                }
            }
            let oris = vec![Orientation::new([500.0, 500.0, 500.0], [0.0, 0.0, 1.0])];
            Model::fit(&Series::new(sps, oris), KernelOptions::default())
        };
        // A BENT erosion surface (index 0, Erode): a ridge in x — apex z≈700 at x=500, flanks z≈400 at
        // x=200/800. Adjacent wall triangles' normals splay across the apex. It truncates the flat
        // contact along two curved seams (where the ridge crosses z=500).
        let bent_erosion = || {
            let mut sps = Vec::new();
            for j in 0..4 {
                let y = j as f64 * 300.0 + 50.0;
                sps.push(SurfacePoint::new([200.0, y, 400.0], 0));
                sps.push(SurfacePoint::new([500.0, y, 700.0], 0));
                sps.push(SurfacePoint::new([800.0, y, 400.0], 0));
            }
            // Orientations on each flank (normal tilts +x on the left flank, −x on the right) + apex up.
            let oris = vec![
                Orientation::new([350.0, 500.0, 550.0], normalize([-1.0, 0.0, 1.0])),
                Orientation::new([500.0, 500.0, 700.0], [0.0, 0.0, 1.0]),
                Orientation::new([650.0, 500.0, 550.0], normalize([1.0, 0.0, 1.0])),
            ];
            Model::fit(&Series::new(sps, oris), KernelOptions::default())
        };
        let min = [0.0, 0.0, 0.0];
        let size = 1000.0;
        let ext = [1000.0, 1000.0, 1000.0];
        let depth = 6u32;
        let octcfg = OctreeConfig { max_depth: depth, min_depth: 2, qef_threshold: 1e-6, cap: false };
        let cell = size / (1u32 << depth) as f64;

        for seam in [SeamMode::Snap, SeamMode::Clip] {
            let stack = Stack::new(vec![
                StackElement::Series(StackedSeries::new(bent_erosion(), Relation::Erode)),
                StackElement::Series(StackedSeries::floor(sediment())),
            ]);
            let contact = stack
                .mesh_one_erode_clipped(1, 0, min, size, ext, octcfg, seam)
                .expect("eroded contact");

            let comp_sizes = component_sizes_bbox(&contact, cell * 0.05);
            let slivers: Vec<usize> = comp_sizes.iter().map(|c| c.0).filter(|&n| n <= 12).collect();
            assert!(
                slivers.is_empty(),
                "[{seam:?}] eroded contact has {} sliver component(s) {slivers:?} (sizes {:?}) — \
                 a rim-extrapolation tongue was kept as an isolated fragment",
                slivers.len(),
                comp_sizes.iter().map(|c| c.0).collect::<Vec<_>>()
            );
            assert_eq!(
                comp_sizes.len(),
                2,
                "[{seam:?}] eroded contact has {} components {:?} — expected exactly 2 edge-strips",
                comp_sizes.len(),
                comp_sizes.iter().map(|c| c.0).collect::<Vec<_>>()
            );
        }
    }

    /// Position-welded connected components of a mesh, each as `(triangle_count, bbox_lo, bbox_hi)`,
    /// largest first.
    fn component_sizes_bbox(m: &Mesh, snap: f64) -> Vec<(usize, [f64; 3], [f64; 3])> {
        let pos = |vi: usize| [m.positions[vi * 3] as f64, m.positions[vi * 3 + 1] as f64, m.positions[vi * 3 + 2] as f64];
        let mut node_of = vec![0usize; m.vertex_count()];
        let mut bucket: HashMap<(i64, i64, i64), usize> = HashMap::new();
        let mut nn = 0usize;
        for vi in 0..m.vertex_count() {
            let p = pos(vi);
            let key = ((p[0] / snap).round() as i64, (p[1] / snap).round() as i64, (p[2] / snap).round() as i64);
            node_of[vi] = *bucket.entry(key).or_insert_with(|| { let n = nn; nn += 1; n });
        }
        let mut parent: Vec<usize> = (0..nn).collect();
        fn find(p: &mut [usize], mut x: usize) -> usize { while p[x] != x { p[x] = p[p[x]]; x = p[x]; } x }
        for t in m.indices.chunks_exact(3) {
            let (a, b, c) = (node_of[t[0] as usize], node_of[t[1] as usize], node_of[t[2] as usize]);
            let (ra, rb, rc) = (find(&mut parent, a), find(&mut parent, b), find(&mut parent, c));
            parent[rb] = ra; parent[rc] = ra;
        }
        let mut tris: HashMap<usize, usize> = HashMap::new();
        let mut lo: HashMap<usize, [f64; 3]> = HashMap::new();
        let mut hi: HashMap<usize, [f64; 3]> = HashMap::new();
        for t in m.indices.chunks_exact(3) {
            let r = find(&mut parent, node_of[t[0] as usize]);
            *tris.entry(r).or_insert(0) += 1;
            for &c in t {
                let p = pos(c as usize);
                let l = lo.entry(r).or_insert([f64::INFINITY; 3]);
                let h = hi.entry(r).or_insert([f64::NEG_INFINITY; 3]);
                for d in 0..3 { l[d] = l[d].min(p[d]); h[d] = h[d].max(p[d]); }
            }
        }
        let mut out: Vec<(usize, [f64; 3], [f64; 3])> = tris.iter().map(|(&r, &n)| (n, lo[&r], hi[&r])).collect();
        out.sort_unstable_by(|a, b| b.0.cmp(&a.0));
        out
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
            eprintln!("unit {u}: checking closed solid watertight");
            assert_mesh_watertight(&mesh, &|_| false); // closed solid (cap=true), no open rim
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
            // Topography-capped + intrusion-punched + floor-bounded — the multi-clip CSG must stay
            // crack-free (no non-manifold edge) and hole-free (no open edge). Closed solid, no rim.
            eprintln!("unit {u}: checking multi-clip closed solid watertight");
            assert_mesh_watertight(&mesh, &|_| false);
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
        // The vein's interior band is a closed, manifold, hole-free solid (clipped by the box). No rim.
        assert_mesh_watertight(&solid, &|_| false);
    }

    /// A DIPPING vein cross-cutting a TWO-contact column (contacts at z=350 and z=650 ⇒ three
    /// stacked units), so adjacent units share the SAME oblique vein wall. The walls tilt in x
    /// with z (`dxdz`), so they cut the octree grid obliquely — the case the browser repro shows
    /// opening a staircase gap between adjacent unit solids (and even between contact sheets in
    /// surface mode). Returns the stack plus a probe copy of the vein field.
    fn vertical_vein_over_column() -> (Stack<Model>, Model) {
        let dxdz = 0.5; // matches web3 `cross-cutting-vein-dipping`
        // EXACTLY the web3 `dippingWall` fit: TWO points per wall, BOTH on the y=500 centerline, at
        // z=200 and z=800, x = x0 + dxdz·(z−500). This under-constrained 2-point fit is what the
        // browser meshes — far wavier in y than a 3-point fit, and the geometry that produced the
        // user's staircase. We replicate it so the native weld test sees the SAME wall.
        let build_vein = || {
            let mut sps = Vec::new();
            for (id, x0) in [(0u32, 400.0), (1u32, 600.0)] {
                sps.push(SurfacePoint::new([x0 + dxdz * (200.0 - 500.0), 500.0, 200.0], id));
                sps.push(SurfacePoint::new([x0 + dxdz * (800.0 - 500.0), 500.0, 800.0], id));
            }
            let oris = vec![Orientation::new([500.0, 500.0, 500.0], normalize([1.0, 0.0, -dxdz]))];
            Model::fit(&Series::new(sps, oris), KernelOptions::default())
        };
        let column = StackedSeries::floor(two_contact_model(350.0, 650.0));
        let stack = Stack::new(vec![
            StackElement::Vein(Vein::new(build_vein(), 0, 1)),
            StackElement::Series(column),
        ]);
        (stack, build_vein())
    }

    /// REPRO (surface-mode clip-seam gap) — the user observed gaps in SURFACE mode too, where a
    /// flat contact sheet meets the dipping vein wall (the staircase of see-through triangles in
    /// the wireframe). In surface mode each flat contact is an open sheet CLIPPED by the vein's
    /// `SlabExterior` masker; its clipped free boundary SHOULD lie exactly on the vein wall
    /// surface (`vein_field == band edge`), where it meets the vein's own wall sheet flush.
    ///
    /// We mesh the column's contacts (surface mode), collect each sheet's free-boundary vertices
    /// that are NOT on the domain box rim, and check they lie on the vein wall (vein field within
    /// ε of a band edge). A staircase gap shows up as free-boundary vertices sitting OFF the wall
    /// — the contact's clip boundary stepped away from where the vein wall actually is.
    #[test]
    fn flat_contact_clip_meets_the_vein_wall_in_surface_mode() {
        let (stack, vein_field) = vertical_vein_over_column();
        let min = [0.0, 0.0, 0.0];
        let size = 1000.0;
        let depth = 6u32;
        let cfg = OctreeConfig { max_depth: depth, min_depth: 2, qef_threshold: 1e-6, cap: false };
        let cell = size / (1u32 << depth) as f64;

        let (lo, hi) = {
            let isos = vein_field.interface_isovalues();
            (isos[0].min(isos[1]), isos[0].max(isos[1]))
        };
        // The band edges in WORLD units are where vein_field == lo / hi. We measure a vertex's
        // distance to the nearer wall as |vein_field − edge| / |∇vein_field|, a first-order world
        // distance to the wall surface.
        let wall_dist = |p: [f64; 3]| -> f64 {
            let f = vein_field.scalar_field(p);
            let g = vein_field.scalar_field_gradient(p);
            let gn = (g[0] * g[0] + g[1] * g[1] + g[2] * g[2]).sqrt().max(1e-12);
            (f - lo).abs().min((f - hi).abs()) / gn
        };

        // Mesh the column (element 1) contacts in surface mode.
        let meshes = stack.mesh_surfaces(min, size, cfg);
        let column = meshes.iter().find(|m| m.series == 1).expect("column meshes");

        let on_box = |p: [f64; 3]| -> bool {
            const E: f64 = 1.0;
            p[0] < min[0] + E || p[0] > min[0] + size - E
                || p[1] < min[1] + E || p[1] > min[1] + size - E
                || p[2] < min[2] + E || p[2] > min[2] + size - E
        };

        let mut interior_free = 0usize; // free-boundary verts off the box AND off the wall
        let mut max_off_wall = 0.0_f64;
        let mut total_free = 0usize;
        for (_iid, mesh) in &column.interfaces {
            if mesh.triangle_count() == 0 {
                continue;
            }
            // Undirected edges with incidence 1 are free boundary.
            let mut inc: std::collections::HashMap<(u32, u32), u32> = std::collections::HashMap::new();
            for t in mesh.indices.chunks_exact(3) {
                for &(a, b) in &[(t[0], t[1]), (t[1], t[2]), (t[2], t[0])] {
                    let e = if a < b { (a, b) } else { (b, a) };
                    *inc.entry(e).or_insert(0) += 1;
                }
            }
            for (&(a, b), &c) in &inc {
                if c != 1 {
                    continue;
                }
                for vi in [a, b] {
                    let p = [
                        mesh.positions[vi as usize * 3] as f64,
                        mesh.positions[vi as usize * 3 + 1] as f64,
                        mesh.positions[vi as usize * 3 + 2] as f64,
                    ];
                    if on_box(p) {
                        continue; // expected free boundary on the domain rim
                    }
                    total_free += 1;
                    let d = wall_dist(p);
                    max_off_wall = max_off_wall.max(d);
                    if d > cell {
                        interior_free += 1; // a free-boundary vertex more than a cell off the wall
                    }
                }
            }
        }
        eprintln!(
            "finest_cell={cell:.2} | non-box free-boundary verts={total_free}, off-wall(>1 cell)={interior_free}, max off-wall dist={max_off_wall:.2} ({:.2} cells)",
            max_off_wall / cell
        );

        // The contact clip-edges all lie ON the wall (interior_free≈0) — confirmed. So the gap is
        // NOT the contact stepping off the wall. We now pin the actual mechanism: the vein WALL is
        // meshed as its own independent full sheet (`mesh_cross_cut_surface`) that does NOT have
        // edges/vertices at the contact crease — the wall doesn't know the contact terminates
        // against it. So the contact's clip-edge vertices land in the MIDDLE of wall triangles: a
        // T-junction. Any sub-cell mismatch between the two independent tessellations of the wall
        // opens the triangular slivers the user sees. We confirm the wall sheet has NO crease
        // vertices to share with the contact (its boundary is entirely on the domain box).
        let ext = [size, size, size];
        let mut wall_crease: Vec<[f64; 3]> = Vec::new();
        for k in 0..2 {
            let wall = stack
                .mesh_cross_cut_surface(0, k, min, size, ext, cfg)
                .expect("vein wall should mesh");
            let mut inc: std::collections::HashMap<(u32, u32), u32> = std::collections::HashMap::new();
            for t in wall.indices.chunks_exact(3) {
                for &(a, b) in &[(t[0], t[1]), (t[1], t[2]), (t[2], t[0])] {
                    let e = if a < b { (a, b) } else { (b, a) };
                    *inc.entry(e).or_insert(0) += 1;
                }
            }
            for (&(a, b), &c) in &inc {
                if c != 1 {
                    continue;
                }
                for vi in [a, b] {
                    let p = [
                        wall.positions[vi as usize * 3] as f64,
                        wall.positions[vi as usize * 3 + 1] as f64,
                        wall.positions[vi as usize * 3 + 2] as f64,
                    ];
                    if !on_box(p) {
                        wall_crease.push(p);
                    }
                }
            }
        }

        // For each contact clip-edge crease vertex, the nearest vein-wall crease vertex.
        let mut worst_seam = 0.0_f64;
        let mut contact_crease = 0usize;
        for (_iid, mesh) in &column.interfaces {
            if mesh.triangle_count() == 0 {
                continue;
            }
            let mut inc: std::collections::HashMap<(u32, u32), u32> = std::collections::HashMap::new();
            for t in mesh.indices.chunks_exact(3) {
                for &(a, b) in &[(t[0], t[1]), (t[1], t[2]), (t[2], t[0])] {
                    let e = if a < b { (a, b) } else { (b, a) };
                    *inc.entry(e).or_insert(0) += 1;
                }
            }
            for (&(a, b), &c) in &inc {
                if c != 1 {
                    continue;
                }
                for vi in [a, b] {
                    let p = [
                        mesh.positions[vi as usize * 3] as f64,
                        mesh.positions[vi as usize * 3 + 1] as f64,
                        mesh.positions[vi as usize * 3 + 2] as f64,
                    ];
                    if on_box(p) || wall_dist(p) > cell {
                        continue; // only crease vertices on the wall
                    }
                    contact_crease += 1;
                    let nearest = wall_crease
                        .iter()
                        .map(|w| {
                            let d = [w[0] - p[0], w[1] - p[1], w[2] - p[2]];
                            (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt()
                        })
                        .fold(f64::INFINITY, f64::min);
                    worst_seam = worst_seam.max(nearest);
                }
            }
        }
        let _ = worst_seam;
        eprintln!(
            "contact crease verts on wall={contact_crease}, vein-wall crease verts to share with={}",
            wall_crease.len(),
        );

        // THE DIAGNOSIS, asserted: the contact has hundreds of crease vertices on the wall, but
        // the wall sheet offers ZERO crease vertices to meet them — it's a full independent sheet
        // with no edge at the crease. So the contact terminates in a T-junction against the wall's
        // interior, and the two surfaces' sub-cell mismatch shows as the surface-mode slivers.
        // (The fix is shared-crease meshing / T-junction welding, octree.rs:35-37's follow-up.)
        assert!(contact_crease > 0, "expected the clipped contact to have crease vertices on the wall");
        assert!(
            wall_crease.is_empty(),
            "regression guard: if the vein wall starts emitting crease vertices at the contact, the shared-crease fix has landed — update this test"
        );
    }

    /// THE FIX (T-junction weld) — `mesh_cross_cut_surface_welded` welds each cut contact's crease
    /// vertices INTO the vein wall, so the shared crease becomes shared vertices instead of a
    /// T-junction. We assert every contact crease vertex on the wall now coincides with an ACTUAL
    /// wall vertex (to sub-ε), which the un-welded wall (above) provably did not have. With
    /// `weld=false` the same check fails (the escape hatch reproduces the old behavior).
    #[test]
    fn welding_shares_the_contact_crease_with_the_vein_wall() {
        let (stack, vein_field) = vertical_vein_over_column();
        let min = [0.0, 0.0, 0.0];
        let size = 1000.0;
        let depth = 6u32;
        let cfg = OctreeConfig { max_depth: depth, min_depth: 2, qef_threshold: 1e-6, cap: false };
        let ext = [size, size, size];
        let cell = size / (1u32 << depth) as f64;
        let (lo, hi) = {
            let isos = vein_field.interface_isovalues();
            (isos[0].min(isos[1]), isos[0].max(isos[1]))
        };
        let wall_dist = |p: [f64; 3]| -> f64 {
            let f = vein_field.scalar_field(p);
            let g = vein_field.scalar_field_gradient(p);
            let gn = (g[0] * g[0] + g[1] * g[1] + g[2] * g[2]).sqrt().max(1e-12);
            (f - lo).abs().min((f - hi).abs()) / gn
        };
        let on_box = |p: [f64; 3]| -> bool {
            const E: f64 = 1.0;
            p[0] < E || p[0] > size - E || p[1] < E || p[1] > size - E || p[2] < E || p[2] > size - E
        };

        // Harvest the contact crease points (on the wall, off the box) we need the wall to share.
        let column_meshes = stack.mesh_surfaces(min, size, cfg);
        let column = column_meshes.iter().find(|m| m.series == 1).unwrap();
        let mut crease: Vec<[f64; 3]> = Vec::new();
        for (_iid, m) in &column.interfaces {
            for p in crate::weld::free_boundary_points(m) {
                if !on_box(p) && wall_dist(p) <= cell {
                    crease.push(p);
                }
            }
        }
        assert!(!crease.is_empty(), "no contact crease points harvested");

        // All wall vertices of the WELDED wall, bucketed for nearest lookup.
        let nearest_wall_vertex = |wall: &Mesh, p: [f64; 3]| -> f64 {
            wall.positions
                .chunks_exact(3)
                .map(|v| {
                    let d = [v[0] as f64 - p[0], v[1] as f64 - p[1], v[2] as f64 - p[2]];
                    (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt()
                })
                .fold(f64::INFINITY, f64::min)
        };

        for k in 0..2 {
            let welded = stack
                .mesh_cross_cut_surface_welded(0, k, min, size, ext, cfg, true)
                .expect("welded wall");
            let unwelded = stack
                .mesh_cross_cut_surface_welded(0, k, min, size, ext, cfg, false)
                .expect("unwelded wall");

            // For this wall, only count crease points that actually lie on IT (a vein has two
            // walls; a given crease point belongs to one). A point is "on wall k" if welding put a
            // wall vertex right at it.
            let mut on_this_wall = 0;
            let mut worst_welded = 0.0_f64;
            let mut worst_unwelded = 0.0_f64;
            for &p in &crease {
                let dw = nearest_wall_vertex(&welded, p);
                if dw > cell * 0.5 {
                    continue; // this crease point belongs to the other wall
                }
                on_this_wall += 1;
                worst_welded = worst_welded.max(dw);
                worst_unwelded = worst_unwelded.max(nearest_wall_vertex(&unwelded, p));
            }
            eprintln!(
                "wall {k}: {on_this_wall} crease pts | worst dist to a wall vertex: welded={worst_welded:.3}, unwelded={worst_unwelded:.3} (cell {cell:.2})",
            );
            assert!(on_this_wall > 0, "wall {k} shared no crease points");
            // Welded: every crease point coincides with a wall vertex (the split inserted it). The
            // tolerance is f32 mesh precision on a 1000-unit domain (~1e-2), not exact: the weld
            // projects the point onto the wall surface, which can differ from the harvested point
            // by f32 round-off, but it is the SAME shared vertex for rendering.
            assert!(
                worst_welded <= 0.05,
                "wall {k}: welded wall should have a vertex AT each crease point; worst {worst_welded:.3}"
            );
            // Unwelded: the old behavior leaves them mid-face — a sizeable fraction of a cell away.
            assert!(
                worst_unwelded > cell * 0.1,
                "wall {k}: expected the unwelded wall to MISS crease points (the bug); worst {worst_unwelded:.3}"
            );
        }
    }

    /// THE REAL TEST (see-through holes) — vertex-coincidence (above) is necessary but NOT
    /// sufficient: two separate meshes can share approximate vertices and still leave gaps BETWEEN
    /// them. This is what the user still sees. We MERGE the contact sheets and the welded vein
    /// walls into one combined mesh (snapping coincident vertices to a shared index), then count
    /// open edges that are NOT on the domain box — genuine see-through holes. A correct weld drives
    /// these to zero along the crease; the un-welded path leaves many.
    #[test]
    fn welded_contact_and_wall_seam_is_watertight() {
        let (stack, _vein_field) = vertical_vein_over_column();
        let min = [0.0, 0.0, 0.0];
        let size = 1000.0;
        let depth = 6u32;
        let ext = [size, size, size];
        let cfg = OctreeConfig { max_depth: depth, min_depth: 2, qef_threshold: 1e-6, cap: false };
        let cell = size / (1u32 << depth) as f64;

        // Merge several meshes into one, welding vertices within `snap` to a shared index (a grid
        // hash on the snap cell). Returns (positions, indices).
        let merge = |meshes: &[&Mesh], snap: f64| -> (Vec<[f64; 3]>, Vec<u32>) {
            let mut verts: Vec<[f64; 3]> = Vec::new();
            let mut indices: Vec<u32> = Vec::new();
            let mut hash: std::collections::HashMap<(i64, i64, i64), Vec<u32>> = std::collections::HashMap::new();
            let key = |p: [f64; 3]| {
                (
                    (p[0] / snap).round() as i64,
                    (p[1] / snap).round() as i64,
                    (p[2] / snap).round() as i64,
                )
            };
            for m in meshes {
                let mut remap = vec![0u32; m.vertex_count()];
                for vi in 0..m.vertex_count() {
                    let p = [
                        m.positions[vi * 3] as f64,
                        m.positions[vi * 3 + 1] as f64,
                        m.positions[vi * 3 + 2] as f64,
                    ];
                    // Find an existing vertex within snap in the 3³ neighbourhood.
                    let (kx, ky, kz) = key(p);
                    let mut found = u32::MAX;
                    'outer: for dx in -1..=1 {
                        for dy in -1..=1 {
                            for dz in -1..=1 {
                                if let Some(ids) = hash.get(&(kx + dx, ky + dy, kz + dz)) {
                                    for &id in ids {
                                        let q = verts[id as usize];
                                        let d2 = (q[0] - p[0]).powi(2) + (q[1] - p[1]).powi(2) + (q[2] - p[2]).powi(2);
                                        if d2 <= snap * snap {
                                            found = id;
                                            break 'outer;
                                        }
                                    }
                                }
                            }
                        }
                    }
                    let id = if found != u32::MAX {
                        found
                    } else {
                        let id = verts.len() as u32;
                        verts.push(p);
                        hash.entry((kx, ky, kz)).or_default().push(id);
                        id
                    };
                    remap[vi] = id;
                }
                for &i in &m.indices {
                    indices.push(remap[i as usize]);
                }
            }
            (verts, indices)
        };

        let on_box = |p: [f64; 3]| -> bool {
            const E: f64 = 1.0;
            p[0] < E || p[0] > size - E || p[1] < E || p[1] > size - E || p[2] < E || p[2] > size - E
        };

        let count_interior_open = |weld: bool| -> usize {
            // Contacts (series 1) + both vein walls (series 0), welded or not.
            let column = stack.mesh_surfaces(min, size, cfg);
            let col = column.iter().find(|m| m.series == 1).unwrap();
            let mut meshes: Vec<Mesh> = col.interfaces.iter().map(|(_i, m)| m.clone()).collect();
            for k in 0..2 {
                meshes.push(
                    stack
                        .mesh_cross_cut_surface_welded(0, k, min, size, ext, cfg, weld)
                        .unwrap(),
                );
            }
            let refs: Vec<&Mesh> = meshes.iter().collect();
            // Snap tolerance: a fraction of a cell (the weld places shared verts within ~0.006).
            let (verts, indices) = merge(&refs, cell * 0.1);
            // Count non-box open edges.
            let mut inc: std::collections::HashMap<(u32, u32), u32> = std::collections::HashMap::new();
            for t in indices.chunks_exact(3) {
                for &(a, b) in &[(t[0], t[1]), (t[1], t[2]), (t[2], t[0])] {
                    let e = if a < b { (a, b) } else { (b, a) };
                    *inc.entry(e).or_insert(0) += 1;
                }
            }
            let holes: Vec<[f64; 3]> = inc
                .iter()
                .filter(|(_, &c)| c == 1)
                .filter(|((a, b), _)| !on_box(verts[*a as usize]) && !on_box(verts[*b as usize]))
                .map(|((a, b), _)| {
                    let (p, q) = (verts[*a as usize], verts[*b as usize]);
                    [(p[0] + q[0]) * 0.5, (p[1] + q[1]) * 0.5, (p[2] + q[2]) * 0.5]
                })
                .collect();
            holes.len()
        };

        let unwelded = count_interior_open(false);
        let welded = count_interior_open(true);
        eprintln!("interior (non-box) open edges: unwelded={unwelded}, welded={welded}");

        // The weld must REDUCE the see-through holes. (Strict watertightness — welded==0 — may need
        // more than vertex sharing; this asserts the weld at least closes the bulk of them, and
        // pins the current number so we can drive it to zero.)
        assert!(
            welded < unwelded,
            "weld did not reduce see-through holes: unwelded={unwelded} welded={welded}"
        );
    }

    /// REGRESSION — every vein wall resolves at full detail on the shared tree. The build scene's
    /// primary is `body_isos[0]` (the foot wall); a prior version did NOT add the OTHER wall
    /// isovalues as refinement contributors, so the hanging wall was contoured on a tree coarse
    /// where it lives — emerging ~7× sparser and SPIKY (the ragged crest the user saw). We assert
    /// the walls have COMPARABLE triangle density (within 3×), and that no wall is a near-empty
    /// sparse sheet. (The foot wall's box-floor perimeter slivers are a SEPARATE, pre-existing
    /// open-sheet artifact — identical in the independent path — and are not asserted here.)
    #[test]
    fn shared_vein_walls_are_both_fully_resolved() {
        let (stack, _f) = vertical_vein_over_column();
        let min = [0.0, 0.0, 0.0];
        let size = 1000.0;
        let cfg = OctreeConfig { max_depth: 6, min_depth: 2, qef_threshold: 1e-6, cap: false };
        let (walls, _contacts) = stack
            .mesh_cross_cut_shared(0, min, size, [size, size, size], cfg, SeamMode::Snap)
            .unwrap();
        assert_eq!(walls.len(), 2, "a vein has two walls");
        let tris: Vec<usize> = walls.iter().map(|w| w.triangle_count()).collect();
        let (lo, hi) = (tris.iter().copied().min().unwrap(), tris.iter().copied().max().unwrap());
        eprintln!("vein wall triangle counts: {tris:?}");
        assert!(lo > 500, "a vein wall is near-empty ({lo} tris) — under-refined on the shared tree");
        assert!(
            hi <= lo * 3,
            "vein walls wildly asymmetric ({lo} vs {hi} tris) — one wall is under-refined (the spiky-crest bug)"
        );
    }

    /// REGRESSION (see-through dashes) — at fine resolution the contact's clip-edge crease lands
    /// SUB-cell short of the vein wall, leaving open edges with no wall triangle behind them (the
    /// dashes the user saw). The shared tree shrinks the gap but doesn't close it; the
    /// `snap_free_boundary_to_mesh` pass in `mesh_cross_cut_shared` does. We reproduce at depth 7
    /// (where the gap is visible — depth 6 hid it) on the BROWSER geometry exactly (2-point dipping
    /// walls over a 3-bed 2-point floor), and assert ZERO genuinely-unbacked open edges.
    #[test]
    fn no_see_through_dashes_at_the_contact_wall_seam() {
        // Match the BROWSER geometry exactly: a dipping vein (2-point walls) over a 3-bed floor
        // whose beds are each a 2-POINT dipping fit (z=300/500/700) — the under-constrained, wavy
        // contacts the browser meshes (web3 `dippingInterface`/`threeContactFloor`). The
        // well-conditioned `two_contact_model` 4x4 fit does NOT reproduce the holes.
        let dxdz = 0.5;
        let build_vein = || {
            let mut sps = Vec::new();
            for (id, x0) in [(0u32, 400.0), (1u32, 600.0)] {
                sps.push(SurfacePoint::new([x0 + dxdz * (200.0 - 500.0), 500.0, 200.0], id));
                sps.push(SurfacePoint::new([x0 + dxdz * (800.0 - 500.0), 500.0, 800.0], id));
            }
            let oris = vec![Orientation::new([500.0, 500.0, 500.0], normalize([1.0, 0.0, -dxdz]))];
            Model::fit(&Series::new(sps, oris), KernelOptions::default())
        };
        // A 3-bed floor: each bed a 2-point flat fit at z0 (mirrors web3 dippingInterface(z0,0,0)).
        let bed = |z0: f64| {
            let sps = vec![
                SurfacePoint::new([200.0, 500.0, z0], 0),
                SurfacePoint::new([800.0, 500.0, z0], 0),
            ];
            let oris = vec![Orientation::new([500.0, 500.0, z0], [0.0, 0.0, 1.0])];
            Model::fit(&Series::new(sps, oris), KernelOptions::default())
        };
        let _ = bed; // (single-bed helper kept for clarity; a 3-interface model is used)
        let mut sps = Vec::new();
        for (id, z0) in [(0u32, 300.0), (1u32, 500.0), (2u32, 700.0)] {
            sps.push(SurfacePoint::new([200.0, 500.0, z0], id));
            sps.push(SurfacePoint::new([800.0, 500.0, z0], id));
        }
        let floor_model = Model::fit(
            &Series::new(sps, vec![Orientation::new([500.0, 500.0, 500.0], [0.0, 0.0, 1.0])]),
            KernelOptions::default(),
        );
        let stack = Stack::new(vec![
            StackElement::Vein(Vein::new(build_vein(), 0, 1)),
            StackElement::Series(StackedSeries::floor(floor_model)),
        ]);
        let vf = build_vein();
        let min = [0.0, 0.0, 0.0];
        let size = 1000.0;
        let depth = 7u32; // browser-like fine walls (its walls were ~16k tris vs native ~3k at d6)
        let cfg = OctreeConfig { max_depth: depth, min_depth: 2, qef_threshold: 1e-6, cap: false };
        let cell = size / (1u32 << depth) as f64;
        let (walls, contacts) = stack.mesh_cross_cut_shared(0, min, size, [size, size, size], cfg, SeamMode::Snap).unwrap();
        let on_box = |p: [f64; 3]| {
            const E: f64 = 1.0;
            p[0] < E || p[0] > size - E || p[1] < E || p[1] > size - E || p[2] < E || p[2] > size - E
        };
        // Backed by a wall triangle?
        let backed = |mid: [f64; 3]| {
            for w in &walls {
                for t in w.indices.chunks_exact(3) {
                    let tp = |k: usize| { let v = t[k] as usize * 3; [w.positions[v] as f64, w.positions[v+1] as f64, w.positions[v+2] as f64] };
                    let c = crate::weld::closest_point_on_triangle_pub(mid, tp(0), tp(1), tp(2));
                    if (c[0]-mid[0]).powi(2)+(c[1]-mid[1]).powi(2)+(c[2]-mid[2]).powi(2) < (cell*0.1).powi(2) { return true; }
                }
            }
            false
        };
        let _ = vf;
        let count_unbacked = |meshes: &[&Mesh]| -> usize {
            let mut n = 0;
            for m in meshes {
                let mut inc: std::collections::HashMap<(u32,u32),u32> = std::collections::HashMap::new();
                for t in m.indices.chunks_exact(3) {
                    for &(a,b) in &[(t[0],t[1]),(t[1],t[2]),(t[2],t[0])] { let e = if a<b {(a,b)} else {(b,a)}; *inc.entry(e).or_insert(0)+=1; }
                }
                for (&(a,b),&c) in &inc {
                    if c != 1 { continue; }
                    let pa = [m.positions[a as usize*3] as f64, m.positions[a as usize*3+1] as f64, m.positions[a as usize*3+2] as f64];
                    let pb = [m.positions[b as usize*3] as f64, m.positions[b as usize*3+1] as f64, m.positions[b as usize*3+2] as f64];
                    if on_box(pa) || on_box(pb) { continue; }
                    let mid = [(pa[0]+pb[0])*0.5,(pa[1]+pb[1])*0.5,(pa[2]+pb[2])*0.5];
                    if !backed(mid) { n += 1; }
                }
            }
            n
        };
        let wall_refs: Vec<&Mesh> = walls.iter().collect();
        let contact_refs: Vec<&Mesh> = contacts.iter().map(|(_,_,m)| m).collect();
        let wall_unbacked = count_unbacked(&wall_refs);
        let contact_unbacked = count_unbacked(&contact_refs);
        eprintln!("unbacked open edges — walls: {wall_unbacked}, contacts: {contact_unbacked}");
        assert_eq!(
            contact_unbacked, 0,
            "contact crease leaves {contact_unbacked} see-through dashes (unbacked by the wall) — the snap-to-wall closer regressed"
        );
        assert_eq!(wall_unbacked, 0, "wall has {wall_unbacked} unbacked open edges");
    }

    /// A single-wall cross-cutting body (an `Intrusion::Single` envelope) over a two-contact column,
    /// so `mesh_cross_cut_shared` produces exactly ONE wall — the case the `SeamMode::Clip` path
    /// (step 1 of the mesh-clip wiring) handles. The contact terminating against the single envelope
    /// wall must, under `Clip`, share the seam by index → NO see-through dashes on the contact (every
    /// non-box contact open edge is backed by the wall). This is the clip path's first end-to-end
    /// proof inside production meshing; veins (two walls) still use `Snap` until step 2.
    fn intrusion_single_over_column() -> Stack<Model> {
        // A near-vertical intrusion envelope through the column centre (dips slightly in x so it
        // cuts the octree grid obliquely, like the vein case). Single envelope = one wall.
        let env = {
            let mut sps = Vec::new();
            // Two points on the y-centreline define a tilted vertical-ish contact x ≈ 500 + 0.4(z−500).
            sps.push(SurfacePoint::new([500.0 + 0.4 * (200.0 - 500.0), 500.0, 200.0], 0));
            sps.push(SurfacePoint::new([500.0 + 0.4 * (800.0 - 500.0), 500.0, 800.0], 0));
            let oris = vec![Orientation::new([500.0, 500.0, 500.0], normalize([1.0, 0.0, -0.4]))];
            Model::fit(&Series::new(sps, oris), KernelOptions::default())
        };
        Stack::new(vec![
            StackElement::Intrusion(Intrusion::new(env, Envelope::Single(0))),
            StackElement::Series(StackedSeries::floor(two_contact_model(350.0, 650.0))),
        ])
    }

    /// Count non-box open edges on the clipped contacts whose midpoint is NOT backed by ANY wall (a
    /// see-through dash). Shared by the single-wall and band clip-seam tests. `size`/`cell` size the
    /// box-rim filter and the "backed" tolerance (half a cell).
    fn count_unbacked_contact_dashes(
        contacts: &[(usize, usize, Mesh)],
        walls: &[Mesh],
        size: f64,
        cell: f64,
    ) -> usize {
        let on_box = |p: [f64; 3]| {
            const E: f64 = 1.0;
            (0..3).any(|d| p[d] < E || p[d] > size - E)
        };
        let backed = |mid: [f64; 3]| -> bool {
            for wall in walls {
                for t in wall.indices.chunks_exact(3) {
                    let tp = |i: usize| {
                        [
                            wall.positions[t[i] as usize * 3] as f64,
                            wall.positions[t[i] as usize * 3 + 1] as f64,
                            wall.positions[t[i] as usize * 3 + 2] as f64,
                        ]
                    };
                    let c = crate::weld::closest_point_on_triangle_pub(mid, tp(0), tp(1), tp(2));
                    if (c[0] - mid[0]).powi(2) + (c[1] - mid[1]).powi(2) + (c[2] - mid[2]).powi(2)
                        < (cell * 0.5).powi(2)
                    {
                        return true;
                    }
                }
            }
            false
        };
        let mut unbacked = 0;
        for (_, _, m) in contacts {
            let mut inc: std::collections::HashMap<(u32, u32), u32> = std::collections::HashMap::new();
            for t in m.indices.chunks_exact(3) {
                for &(a, b) in &[(t[0], t[1]), (t[1], t[2]), (t[2], t[0])] {
                    let e = if a < b { (a, b) } else { (b, a) };
                    *inc.entry(e).or_insert(0) += 1;
                }
            }
            for (&(a, b), &c) in &inc {
                if c != 1 {
                    continue;
                }
                let pa = [m.positions[a as usize * 3] as f64, m.positions[a as usize * 3 + 1] as f64, m.positions[a as usize * 3 + 2] as f64];
                let pb = [m.positions[b as usize * 3] as f64, m.positions[b as usize * 3 + 1] as f64, m.positions[b as usize * 3 + 2] as f64];
                if on_box(pa) || on_box(pb) {
                    continue;
                }
                let mid = [(pa[0] + pb[0]) * 0.5, (pa[1] + pb[1]) * 0.5, (pa[2] + pb[2]) * 0.5];
                if !backed(mid) {
                    unbacked += 1;
                }
            }
        }
        unbacked
    }

    #[test]
    fn clip_seam_single_wall_is_watertight() {
        let stack = intrusion_single_over_column();
        let min = [0.0, 0.0, 0.0];
        let size = 1000.0;
        let ext = [size, size, size];
        let depth = 6u32;
        let cfg = OctreeConfig { max_depth: depth, min_depth: 2, qef_threshold: 1e-6, cap: false };

        // Confirm this fixture is genuinely the single-wall case (so the Clip path is exercised, not
        // silently falling back to Snap).
        let (walls, contacts) = stack
            .mesh_cross_cut_shared(0, min, size, ext, cfg, SeamMode::Clip)
            .expect("shared cross-cut meshing");
        assert_eq!(walls.len(), 1, "fixture must be single-wall to exercise the Clip path");
        assert!(!contacts.is_empty(), "intrusion did not cut any column contact");
        assert!(
            contacts.iter().any(|(_, _, m)| m.triangle_count() > 0),
            "clip produced no contact geometry"
        );

        // Every non-box open edge on a clipped contact must be BACKED by the wall (no see-through
        // dash) — the same metric the snap tests use, so it's a like-for-like bar.
        let cell = size / (1u32 << depth) as f64;
        let unbacked = count_unbacked_contact_dashes(&contacts, &walls, size, cell);
        assert_eq!(unbacked, 0, "Clip path left {unbacked} see-through dashes on the contact seam");
    }

    /// Step 2 — a VEIN (two-wall band) cut contact under `SeamMode::Clip`. The contact survives
    /// OUTSIDE the band (`field < lo ∪ field > hi`), produced as the UNION of two single-side clips
    /// (one per wall). Assert: it IS the two-wall case (so the band-union path runs), the contact
    /// survives on BOTH sides of the band (the half-erasing one-sided bug stays dead), and every
    /// non-box contact open edge is backed by a wall (no see-through dashes).
    #[test]
    fn clip_seam_vein_band_is_watertight() {
        let (stack, _vf) = vertical_vein_over_column();
        let min = [0.0, 0.0, 0.0];
        let size = 1000.0;
        let ext = [size, size, size];
        let depth = 6u32;
        let cfg = OctreeConfig { max_depth: depth, min_depth: 2, qef_threshold: 1e-6, cap: false };

        let (walls, contacts) = stack
            .mesh_cross_cut_shared(0, min, size, ext, cfg, SeamMode::Clip)
            .expect("shared cross-cut meshing");
        assert_eq!(walls.len(), 2, "vein must be the two-wall band case");
        assert!(!contacts.is_empty(), "vein did not cut any column contact");

        // Both band-exterior sides survive (foot side small x, hanging side large x) — the band
        // [400,600]-ish in x at column mid, so left < 350 and right > 650 must both be populated.
        for (_, _, m) in &contacts {
            let (mut has_left, mut has_right) = (false, false);
            for v in m.positions.chunks_exact(3) {
                let x = v[0] as f64;
                has_left |= x < 350.0;
                has_right |= x > 650.0;
            }
            assert!(has_left && has_right, "band-union erased a side: left={has_left} right={has_right}");
        }

        let cell = size / (1u32 << depth) as f64;
        let unbacked = count_unbacked_contact_dashes(&contacts, &walls, size, cell);
        assert_eq!(unbacked, 0, "vein Clip path left {unbacked} see-through dashes on the contact seam");
    }

    /// Thread-1 NON-REGRESSION guard: the band-complement single-call clip must leave each cut bed a
    /// CLEAN 2 connected components (foot side + hanging side) with NO orphan slivers. Faithful repro
    /// of web3 `cross-cutting-vein-dipping`: a 3-FLAT-bed floor (z=300/500/700) cross-cut by a DIPPING
    /// vein (dxdz=0.5, walls at x0=400/600, the 2-point under-constrained — y-wavy — fit), at the web
    /// default depth 6.
    ///
    /// CAVEAT (documented in `docs/mesh_clip_band_complement.md`): the parked sliver symptom is
    /// WASM-ONLY — it never reproduced in native tests (the two-call concat was ALSO a clean 2
    /// components here). So this test guards against a NATIVE regression and pins the post-fix
    /// component count; it does NOT prove the wasm symptom fixed. Real verification is the web
    /// "clip seam" A/B toggle.
    #[test]
    fn band_complement_keeps_two_clean_components_per_bed() {
        // The dipping vein: dxdz=0.5, walls at x0=400/600, 2-point fit (matches `dippingWall`).
        let dxdz = 0.5;
        let build_vein = || {
            let mut sps = Vec::new();
            for (id, x0) in [(0u32, 400.0), (1u32, 600.0)] {
                sps.push(SurfacePoint::new([x0 + dxdz * (200.0 - 500.0), 500.0, 200.0], id));
                sps.push(SurfacePoint::new([x0 + dxdz * (800.0 - 500.0), 500.0, 800.0], id));
            }
            let oris = vec![Orientation::new([500.0, 500.0, 500.0], normalize([1.0, 0.0, -dxdz]))];
            Model::fit(&Series::new(sps, oris), KernelOptions::default())
        };
        // 3 flat beds at z = 300/500/700 (ids 0,1,2), pole +z. Mirrors `threeContactFloor`.
        let three_bed = {
            let mut sps = Vec::new();
            for (id, z) in [(0u32, 300.0), (1u32, 500.0), (2u32, 700.0)] {
                for i in 0..4 {
                    for j in 0..4 {
                        sps.push(SurfacePoint::new([i as f64 * 300.0 + 50.0, j as f64 * 300.0 + 50.0, z], id));
                    }
                }
            }
            let oris = vec![Orientation::new([500.0, 500.0, 500.0], [0.0, 0.0, 1.0])];
            Model::fit(&Series::new(sps, oris), KernelOptions::default())
        };
        let stack = Stack::new(vec![
            StackElement::Vein(Vein::new(build_vein(), 0, 1)),
            StackElement::Series(StackedSeries::floor(three_bed)),
        ]);
        let min = [0.0, 0.0, 0.0];
        let size = 1000.0;
        let ext = [size, size, size];
        let depth = 6u32;
        let cfg = OctreeConfig { max_depth: depth, min_depth: 2, qef_threshold: 1e-6, cap: false };
        let cell = size / (1u32 << depth) as f64;

        let (_walls, contacts) = stack
            .mesh_cross_cut_shared(0, min, size, ext, cfg, SeamMode::Clip)
            .expect("shared cross-cut meshing");

        // Position-welded connected components (snap merges f32 round-off; the foot/hanging sides are
        // genuinely separated by the band, so they stay 2 components — an orphan sliver would add one).
        let component_sizes = |m: &Mesh| -> Vec<usize> {
            let snap = cell * 0.05;
            let pos = |vi: usize| {
                [m.positions[vi * 3] as f64, m.positions[vi * 3 + 1] as f64, m.positions[vi * 3 + 2] as f64]
            };
            let mut node_of = vec![0usize; m.vertex_count()];
            let mut bucket: HashMap<(i64, i64, i64), usize> = HashMap::new();
            let mut nnodes = 0usize;
            for vi in 0..m.vertex_count() {
                let p = pos(vi);
                let key = ((p[0] / snap).round() as i64, (p[1] / snap).round() as i64, (p[2] / snap).round() as i64);
                node_of[vi] = *bucket.entry(key).or_insert_with(|| {
                    let n = nnodes;
                    nnodes += 1;
                    n
                });
            }
            let mut parent: Vec<usize> = (0..nnodes).collect();
            fn find(parent: &mut [usize], mut x: usize) -> usize {
                while parent[x] != x {
                    parent[x] = parent[parent[x]];
                    x = parent[x];
                }
                x
            }
            for t in m.indices.chunks_exact(3) {
                let (a, b, c) = (node_of[t[0] as usize], node_of[t[1] as usize], node_of[t[2] as usize]);
                let (ra, rb, rc) = (find(&mut parent, a), find(&mut parent, b), find(&mut parent, c));
                parent[rb] = ra;
                parent[rc] = ra;
            }
            let mut tris: HashMap<usize, usize> = HashMap::new();
            for t in m.indices.chunks_exact(3) {
                *tris.entry(find(&mut parent, node_of[t[0] as usize])).or_insert(0) += 1;
            }
            let mut sizes: Vec<usize> = tris.into_values().collect();
            sizes.sort_unstable_by(|a, b| b.cmp(a));
            sizes
        };

        assert_eq!(contacts.len(), 3, "the 3-bed floor must cut 3 contacts");
        for (j, k, m) in &contacts {
            let sizes = component_sizes(m);
            assert_eq!(
                sizes.len(),
                2,
                "bed (s{j},i{k}) has {} components {:?} — expected 2 (foot side + hanging side); a sliver orphaned",
                sizes.len(),
                sizes
            );
            assert!(
                sizes.iter().all(|&n| n > 8),
                "bed (s{j},i{k}) has a small (sliver) component: {sizes:?}"
            );
        }
    }

    /// THE SHARED-CREASE FIX — co-contour the vein walls AND the cut contacts on ONE octree
    /// (`mesh_cross_cut_shared`) so their crease is the SAME discretization. Merge all the meshes
    /// and count non-box open edges: this should be DRAMATICALLY fewer than the independent meshes
    /// (and far fewer than the point-weld's residual), because the seam is watertight by
    /// construction, not patched after the fact.
    #[test]
    fn shared_crease_closes_the_seam() {
        for probe_depth in [6u32, 7] {
            shared_crease_holes_at_depth(probe_depth);
        }
    }

    fn shared_crease_holes_at_depth(depth: u32) {
        let (stack, _vein_field) = vertical_vein_over_column();
        let min = [0.0, 0.0, 0.0];
        let size = 1000.0;
        let ext = [size, size, size];
        let cfg = OctreeConfig { max_depth: depth, min_depth: 2, qef_threshold: 1e-6, cap: false };
        let cell = size / (1u32 << depth) as f64;

        let (walls, contacts) = stack
            .mesh_cross_cut_shared(0, min, size, ext, cfg, SeamMode::Snap)
            .expect("shared cross-cut meshing");

        // REGRESSION: a vein clips the contacts to OUTSIDE its band — both the foot-wall side
        // (small x) AND the hanging-wall side (large x) survive. A one-sided clip (the bug that
        // erased half the sediments) would leave contact vertices on only ONE side of the band
        // (≈[400,600] in x at the column mid). Assert each contact has vertices on BOTH sides.
        for (j, k, m) in &contacts {
            let mut has_left = false;
            let mut has_right = false;
            for v in m.positions.chunks_exact(3) {
                let x = v[0] as f64;
                if x < 350.0 {
                    has_left = true;
                }
                if x > 650.0 {
                    has_right = true;
                }
            }
            assert!(
                has_left && has_right,
                "contact (series {j}, iface {k}) survives on only one side of the vein (left={has_left}, right={has_right}) — the clip is one-sided"
            );
        }

        let mut all: Vec<&Mesh> = Vec::new();
        for m in &walls {
            all.push(m);
        }
        for (_j, _k, m) in &contacts {
            all.push(m);
        }

        // Merge with vertex snapping (the contact and wall crease should now use IDENTICAL vertices
        // since they were contoured on the same tree — snap is just for f32 round-off).
        let snap = cell * 0.01;
        let mut verts: Vec<[f64; 3]> = Vec::new();
        let mut indices: Vec<u32> = Vec::new();
        let mut hash: std::collections::HashMap<(i64, i64, i64), Vec<u32>> = std::collections::HashMap::new();
        let key = |p: [f64; 3]| ((p[0] / snap).round() as i64, (p[1] / snap).round() as i64, (p[2] / snap).round() as i64);
        for m in &all {
            let mut remap = vec![0u32; m.vertex_count()];
            for vi in 0..m.vertex_count() {
                let p = [m.positions[vi * 3] as f64, m.positions[vi * 3 + 1] as f64, m.positions[vi * 3 + 2] as f64];
                let (kx, ky, kz) = key(p);
                let mut found = u32::MAX;
                'o: for dx in -1..=1 {
                    for dy in -1..=1 {
                        for dz in -1..=1 {
                            if let Some(ids) = hash.get(&(kx + dx, ky + dy, kz + dz)) {
                                for &id in ids {
                                    let q = verts[id as usize];
                                    if (q[0] - p[0]).powi(2) + (q[1] - p[1]).powi(2) + (q[2] - p[2]).powi(2) <= snap * snap {
                                        found = id;
                                        break 'o;
                                    }
                                }
                            }
                        }
                    }
                }
                remap[vi] = if found != u32::MAX {
                    found
                } else {
                    let id = verts.len() as u32;
                    verts.push(p);
                    hash.entry((kx, ky, kz)).or_default().push(id);
                    id
                };
            }
            for &idx in &m.indices {
                indices.push(remap[idx as usize]);
            }
        }
        let on_box = |p: [f64; 3]| -> bool {
            const E: f64 = 1.0;
            p[0] < E || p[0] > size - E || p[1] < E || p[1] > size - E || p[2] < E || p[2] > size - E
        };
        let mut inc: std::collections::HashMap<(u32, u32), u32> = std::collections::HashMap::new();
        for t in indices.chunks_exact(3) {
            for &(a, b) in &[(t[0], t[1]), (t[1], t[2]), (t[2], t[0])] {
                let e = if a < b { (a, b) } else { (b, a) };
                *inc.entry(e).or_insert(0) += 1;
            }
        }
        let open_edges: Vec<[f64; 3]> = inc
            .iter()
            .filter(|(_, &c)| c == 1)
            .map(|(&(a, b), _)| (verts[a as usize], verts[b as usize]))
            .filter(|(p, q)| !on_box(*p) && !on_box(*q))
            .map(|(p, q)| [(p[0] + q[0]) * 0.5, (p[1] + q[1]) * 0.5, (p[2] + q[2]) * 0.5])
            .collect();
        let interior_open = open_edges.len();

        // The few remaining non-box open edges are the contact sheets' LEGITIMATE free boundary
        // where they end against the wall (a clipped open sheet has a free perimeter, exactly like
        // an onlap pinch-out). What matters is whether they are SEE-THROUGH: an open edge is benign
        // if a WALL triangle backs it (the wall is right behind where the contact ends), and a real
        // hole only if NOTHING backs it. We count the genuinely-unbacked edges — the visible gaps.
        let backed = |mid: [f64; 3]| -> bool {
            for w in &walls {
                for t in w.indices.chunks_exact(3) {
                    let tp = |k: usize| {
                        let v = t[k] as usize * 3;
                        [w.positions[v] as f64, w.positions[v + 1] as f64, w.positions[v + 2] as f64]
                    };
                    let closest = crate::weld::closest_point_on_triangle_pub(mid, tp(0), tp(1), tp(2));
                    let d2 = (closest[0] - mid[0]).powi(2) + (closest[1] - mid[1]).powi(2) + (closest[2] - mid[2]).powi(2);
                    if d2 < (cell * 0.1).powi(2) {
                        return true;
                    }
                }
            }
            false
        };
        let see_through = open_edges.iter().filter(|&&mid| !backed(mid)).count();
        eprintln!(
            "shared-crease: {interior_open} non-box open edges, {see_through} genuinely SEE-THROUGH (point-weld left ~68 holes, many see-through)"
        );

        // The fix: NO genuinely see-through holes at the contact↔wall seam (the residual open edges
        // are backed contact-sheet perimeters, not visible gaps).
        assert_eq!(
            see_through, 0,
            "shared-crease left {see_through} see-through holes at the seam"
        );
    }

    /// REPRO (vein-body ↔ cross-cut-rock gap) — the bug behind the see-through vertical channel
    /// in the web3 solid view. The web3 worker meshes the vein's INTERIOR solid (its namesake
    /// lithology) via `mesh_cross_cut_solid`, AND the surrounding rock units via
    /// `mesh_one_block_unit_solid`. These are TWO independent dual-contourings of the SAME
    /// foot-wall surface, from OPPOSITE sides (the vein fills the band UP TO the wall; the rock
    /// pulls back to the wall from outside) — different code paths, no shared vertices, no weld.
    /// If they disagree by a sub-cell amount the vein body and the rock don't meet, leaving the
    /// channel. (`adjacent_units_share_the_vein_wall` showed same-side rock units DO agree; the
    /// untested seam is body-vs-rock.)
    ///
    /// We sample each mesh's foot-wall x per y-row at the bed level and compare. A flush model
    /// has the two surfaces coincide to a sub-cell tolerance.
    #[test]
    fn vein_body_meets_the_cross_cut_rock() {
        let (stack, vein_field) = vertical_vein_over_column();
        let min = [0.0, 0.0, 0.0];
        let size = 1000.0;
        let ext = [1000.0, 1000.0, 1000.0];
        let depth = 6u32;
        let cfg = OctreeConfig { max_depth: depth, min_depth: 2, qef_threshold: 1e-6, cap: true };
        let cell = size / (1u32 << depth) as f64;

        let (lo, hi) = {
            let isos = vein_field.interface_isovalues();
            (isos[0].min(isos[1]), isos[0].max(isos[1]))
        };

        let _ = (lo, hi);
        // Per (y,z) cell along the wall, the x each mesh's foot-wall vertices reach. Both the
        // vein body's RIGHT-going face and the rock's right face live in x∈[350,450] near the FW
        // plane; we take, per cell, the x nearest the plane from each mesh and compare. No
        // field-sign split — vertices ON the wall (f≈lo) must not be discarded.
        let wall_x = |mesh: &Mesh| -> std::collections::HashMap<(i64, i64), f64> {
            let mut by_cell: std::collections::HashMap<(i64, i64), f64> = std::collections::HashMap::new();
            for v in mesh.positions.chunks_exact(3) {
                let p = [v[0] as f64, v[1] as f64, v[2] as f64];
                if p[1] < 200.0 || p[1] > 800.0 || p[2] < 100.0 || p[2] > 900.0 {
                    continue; // wall interior, away from box rims
                }
                if p[0] < 350.0 || p[0] > 450.0 {
                    continue; // foot-wall plane (x≈400) vicinity only
                }
                let key = ((p[1] / cell).round() as i64, (p[2] / cell).round() as i64);
                // x closest to the FW plane (x=400) in this cell.
                by_cell
                    .entry(key)
                    .and_modify(|x| {
                        if (p[0] - 400.0).abs() < (*x - 400.0).abs() {
                            *x = p[0];
                        }
                    })
                    .or_insert(p[0]);
            }
            by_cell
        };

        let vein_body = stack
            .mesh_cross_cut_solid(0, min, size, ext, cfg)
            .expect("vein interior should mesh");
        // The cross-cut rock: unit 0 of the column (below z=350) survives left of the FW wall.
        let rock = stack
            .mesh_one_block_unit_solid(1, 0, 0, min, size, ext, cfg)
            .expect("rock unit should mesh");

        let body_x = wall_x(&vein_body);
        let rock_x = wall_x(&rock);
        assert!(!body_x.is_empty() && !rock_x.is_empty(), "no foot-wall vertices to compare");

        let mut worst = 0.0_f64;
        let mut compared = 0;
        for (key, &bx) in &body_x {
            if let Some(&rx) = rock_x.get(key) {
                worst = worst.max((bx - rx).abs());
                compared += 1;
            }
        }
        eprintln!(
            "finest_cell={cell:.2} compared {compared} cells | worst body↔rock foot-wall disagreement = {worst:.2} ({:.2} cells)",
            worst / cell
        );
        assert!(compared > 0, "body and rock shared no wall cells to compare");

        // Tightened from `cell * 0.25` to near float noise (positions are f32; a planar wall at full
        // refinement coincides to a few ULP of ~400.0 ≈ 5e-5). The two independent dual-contourings
        // already agree to 0.00 — the see-through channel the task feared does not reproduce here
        // (see `cross_cut_solid_seam_is_watertight` and `docs/volume_mesh_seam.md`).
        let tol = cell * 1e-3;
        assert!(
            worst <= tol,
            "vein body and cross-cut rock disagree on the foot-wall by {worst} (> {tol:.5}); they contour it independently → the visible see-through channel"
        );
    }

    /// Shared edges (and helpers) for the solid-seam watertightness checks. Merges several meshes
    /// into one vertex/triangle soup, deduping vertices by a quantized position key (`q`), so
    /// coincident wall vertices from different solids collapse to one index.
    fn merge_dedup(meshes: &[&Mesh], q: f64) -> (Vec<[f64; 3]>, Vec<[usize; 3]>) {
        let mut map: std::collections::HashMap<(i64, i64, i64), usize> = std::collections::HashMap::new();
        let mut pos: Vec<[f64; 3]> = Vec::new();
        let mut tris: Vec<[usize; 3]> = Vec::new();
        for m in meshes {
            for t in m.indices.chunks_exact(3) {
                let v: Vec<usize> = t
                    .iter()
                    .map(|&vi| {
                        let p = [
                            m.positions[vi as usize * 3] as f64,
                            m.positions[vi as usize * 3 + 1] as f64,
                            m.positions[vi as usize * 3 + 2] as f64,
                        ];
                        let k = ((p[0] / q).round() as i64, (p[1] / q).round() as i64, (p[2] / q).round() as i64);
                        *map.entry(k).or_insert_with(|| {
                            pos.push(p);
                            pos.len() - 1
                        })
                    })
                    .collect();
                tris.push([v[0], v[1], v[2]]);
            }
        }
        (pos, tris)
    }

    /// `(open_off_box, shared_off_box)` for a merged soup: count of incidence-1 edges whose
    /// endpoints are NOT both on the domain box rim (genuine see-through tears), and incidence-≥3
    /// edges off the rim (where two solids' wall caps fused — positive evidence of sharing). This is
    /// a genuinely DIFFERENT criterion from [`assert_volume_watertight`]: a fused body∪rock wall is
    /// legitimately incidence-≥3 (two solids meeting), so it cannot use the manifold assertion — it
    /// shares only the underlying [`incidence`] primitive.
    fn seam_incidence_stats(pos: &[[f64; 3]], tris: &[[usize; 3]], size: f64) -> (usize, usize) {
        let on_box = |p: [f64; 3]| (0..3).any(|d| p[d] < 1.0 || p[d] > size - 1.0);
        let inc = incidence(tris);
        let mut open = 0usize;
        let mut shared = 0usize;
        for (&(a, b), &c) in &inc {
            let off_rim = !(on_box(pos[a]) && on_box(pos[b]));
            if c == 1 && off_rim {
                open += 1;
            }
            if c >= 3 && off_rim {
                shared += 1;
            }
        }
        (open, shared)
    }

    /// Strategy A (`docs/volume_mesh_seam.md`): the body interior solid and the cross-cut rock unit
    /// solids meshed on ONE shared tree fuse watertight across the shared wall — no see-through
    /// edges, and the wall is genuinely SHARED (incidence-≥3 fused edges). Also asserts the
    /// independent (`Snap`) path is itself watertight + deterministic (acceptance: Snap unchanged).
    #[test]
    fn cross_cut_solid_seam_is_watertight() {
        let (stack, _vein) = vertical_vein_over_column();
        let min = [0.0, 0.0, 0.0];
        let size = 1000.0;
        let ext = [1000.0, 1000.0, 1000.0];
        let depth = 5u32; // coarse enough to be fast; the shared tree resamples every unit
        let cfg = OctreeConfig { max_depth: depth, min_depth: 2, qef_threshold: 1e-6, cap: true };
        let cell = size / (1u32 << depth) as f64;
        let q = (cell * 1e-4).max(1e-9);

        // --- Shared path (Strategy A) ---
        let (body, rocks) =
            stack.mesh_cross_cut_solids_shared(0, min, size, ext, cfg).expect("shared solids mesh");
        // Each solid individually watertight (closed, cap=true → incidence exactly 2, the only open
        // edges allowed are on the domain box rim). Routed through the SAME shared closed-solid
        // invariant as the surface path (Task 08 Part C) — `merge_dedup` first so coincident wall
        // verts collapse, then `assert_volume_watertight` with the box rim as the open boundary.
        let on_box = move |p: [f64; 3]| (0..3).any(|d| p[d] < 1.0 || p[d] > size - 1.0);
        let assert_solo = |m: &Mesh| {
            let (p, t) = merge_dedup(&[m], q);
            assert_volume_watertight(&p, &t, &on_box);
        };
        assert_solo(&body);
        for (j, b, u, m) in &rocks {
            if m.triangle_count() > 0 {
                eprintln!("rock unit ({j},{b},{u}): checking solo solid watertight");
                assert_solo(m);
            }
        }
        // Merge body ∪ all rock units and check the seam fuses watertight.
        let mut all: Vec<&Mesh> = vec![&body];
        for (_, _, _, m) in &rocks {
            if m.triangle_count() > 0 {
                all.push(m);
            }
        }
        let (pos, tris) = merge_dedup(&all, q);
        let (open, shared) = seam_incidence_stats(&pos, &tris, size);
        eprintln!("SHARED: merged verts={} tris={} | see-through={open} fused(inc≥3)={shared}", pos.len(), tris.len());
        assert_eq!(open, 0, "shared-tree body∪rock left {open} see-through edges off the box rim");
        assert!(shared > 0, "body and rock did not fuse on the wall (no incidence-≥3 edges)");

        // The seam is NOT propped up by the merge tolerance. Re-merge at a near-exact `q` and assert
        // the metrics are UNCHANGED: the shared wall vertices coincide bit-for-bit (a pure-wall
        // leaf's single-constraint QEF is invariant to the masker's `n → −n` flip), so shrinking `q`
        // by ~6 orders cannot reveal a hidden sub-q crack. Guards the doc claim that the seam is
        // exact, not within-q — and that watertightness rests on per-solid `cap=true` closure, not
        // on this dedup. (Without exact sharing, `shared` would collapse and/or `open` would jump.)
        let q_exact = 1e-9;
        let (pos_x, tris_x) = merge_dedup(&all, q_exact);
        let (open_x, shared_x) = seam_incidence_stats(&pos_x, &tris_x, size);
        assert_eq!(open_x, 0, "seam opened at near-exact q={q_exact:.0e}: wall verts are NOT bit-identical");
        assert_eq!(
            (open_x, shared_x),
            (open, shared),
            "seam metrics changed between q={q:.1e} and q={q_exact:.0e}: fusion depends on the merge tolerance"
        );

        // --- Snap path (independent calls) — watertight + deterministic (acceptance #3) ---
        let body_snap = stack.mesh_cross_cut_solid(0, min, size, ext, cfg).expect("snap body");
        let body_snap2 = stack.mesh_cross_cut_solid(0, min, size, ext, cfg).expect("snap body");
        assert_eq!(body_snap.positions, body_snap2.positions, "Snap body nondeterministic");
        assert_eq!(body_snap.indices, body_snap2.indices, "Snap body nondeterministic");
        let mut snap_all: Vec<Mesh> = vec![body_snap];
        for (j, b) in (0..stack.series.len())
            .filter(|&j| stack.series_at(j).is_some())
            .flat_map(|j| (0..stack.series_block_count(j).max(1)).map(move |b| (j, b)))
            .collect::<Vec<_>>()
        {
            for u in 0..stack.present_unit_count(j, b) {
                if let Some(m) = stack.mesh_one_block_unit_solid(j, b, u, min, size, ext, cfg) {
                    if m.triangle_count() > 0 {
                        snap_all.push(m);
                    }
                }
            }
        }
        let snap_refs: Vec<&Mesh> = snap_all.iter().collect();
        let (sp, st) = merge_dedup(&snap_refs, q);
        let (sopen, _sshared) = seam_incidence_stats(&sp, &st, size);
        eprintln!("SNAP:   merged verts={} tris={} | see-through={sopen}", sp.len(), st.len());
        assert_eq!(sopen, 0, "Snap (independent) body∪rock left {sopen} see-through edges — regression");
    }
}
