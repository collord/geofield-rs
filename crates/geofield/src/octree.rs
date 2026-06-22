//! Adaptive octree dual contouring.
//!
//! Refines the octree where either surface has detail (sign-changing cells whose
//! single-QEF-vertex residual exceeds a threshold), 2:1-balances the tree, then
//! contours it by **minimal-edge iteration**: every surface-crossing minimal edge
//! is, in a 2:1-balanced tree, shared by exactly 4 leaves, located by point
//! sampling the 4 quadrant centers around the edge; each emits one quad. This is
//! simple point-location rather than the recursive cellProc/faceProc/edgeProc of
//! Ju et al. (2002) — the recursion's transition tables are notoriously error-
//! prone to reproduce, and an earlier attempt left ~30% boundary edges (holes) on
//! genuinely refined surfaces. The edge-iteration form is crack-free and
//! consistently wound (regression-tested), at the cost of per-edge leaf lookups.
//!
//! # Closing the surface against a domain boundary (CSG union)
//!
//! A bounded geological model is the solid `{ field < iso } ∩ { inside the domain
//! }`. We mesh its boundary by contouring the **combined CSG field**
//!
//! ```text
//!   g = max(field − iso, b)        (g < 0  ⟺  field < iso  AND  b < 0)
//! ```
//!
//! where `b` is the domain boundary function (`b < 0` inside). The intersection of
//! two solids is the `max` of their signed fields (the standard SDF-CSG identity).
//! Because the result is a SINGLE sign field, plain dual contouring meshes it
//! watertight — the seam where the contact meets the boundary is just an ordinary
//! crease, with no separate "cap" surface to stitch. The single QEF vertex per
//! cell already minimizes the *joint* QEF of the field AND boundary Hermite
//! samples, so the crease vertex snaps to the contact/boundary intersection.
//!
//! (An earlier design contoured the field surface and the boundary cap as two
//! separate passes and tried to reconcile their seam; that cannot be made manifold
//! for boundaries not aligned to the grid — the two passes discretize the seam
//! curve into different edges. The CSG-union reframing removes the problem at the
//! root. The one case it does not cover is a true two-components-in-one-cell, e.g.
//! a sharp boundary crease, which needs Manifold Dual Contouring's per-cell vertex
//! split — Schaefer/Ju/Warren 2007 — left as a follow-up.)
//!
//! With the cap disabled (`OctreeConfig::cap = false`) only `field == iso` is
//! contoured (clipped to the domain) — an OPEN bounded sheet.
//!
//! The domain boundary is reached only through the [`Boundary`] trait (sign at a
//! point; Hermite data at a sign-changing edge). The contourer never references
//! box geometry directly — that is the load-bearing abstraction that lets tilted
//! faults and triangulated fault meshes drop in as new `Boundary` impls with zero
//! contouring-code changes.
//!
//! Field Hermite data (zero-crossings + analytic gradients) comes from
//! `Model::scalar_field` / `scalar_field_gradient`. The vertex/QEF machinery is
//! shared with `mesh.rs`.
//!
//! Corner numbering throughout: corner index `c` has offsets
//! `(c&1, (c>>1)&1, (c>>2)&1)` = (x, y, z) ∈ {0,1}³ (same as `mesh::corner_offset`).

use crate::mesh::{corner_offset, normalize, qef_vertex, Mesh, CELL_EDGES};

/// The contourer samples a scalar field through the [`ScalarField`] oracle contract
/// (value + analytic gradient at a world point) — defined in the permissive
/// `geofield-types` interface crate. In-process it is the kernel's `Model`/`BlockField`;
/// in the browser it is a `RemoteField` forwarding to the kernel wasm module. Either way
/// the octree is generic over `&dyn ScalarField` and unchanged.
pub use geofield_types::ScalarField;

// ---------------------------------------------------------------------------
// Boundary: the domain wall as just-another-implicit-surface.
//
// The contourer asks a Boundary only two things:
//   * the SIGN at a point — inside (`b < 0`) or outside (`b >= 0`) the domain;
//   * given a cell edge that changes sign, the crossing POINT and the surface
//     NORMAL there (Hermite data).
//
// Crucially there is NO "gradient anywhere in space" method: normals are queried
// only at crossing points. An implicit boundary returns its (analytic or finite-
// difference) gradient there; an explicit triangulated boundary returns the
// intersected triangle's normal. The joint QEF that pins seam vertices takes its
// boundary normals from `edge_crossing`, never from a free-space gradient.
// ---------------------------------------------------------------------------

/// The domain boundary, contoured as a second implicit surface (`b == 0`).
///
/// Implementors answer point-membership and per-edge Hermite queries. They must
/// NOT assume a globally defined analytic gradient — `edge_crossing` is the only
/// place a normal is ever produced, so an explicit-mesh boundary can supply it
/// from the intersected triangle.
pub trait Boundary {
    /// Is `p` inside the domain? Inside means `b(p) < 0`.
    fn inside(&self, p: [f64; 3]) -> bool;

    /// Hermite data for a cell edge `a → b` that changes boundary sign.
    ///
    /// Returns the crossing point on `b == 0` and the (unit) boundary normal
    /// there, pointing OUT of the domain (toward increasing `b`). Called only
    /// when `inside(a) != inside(b)`; may return `None` if the implementor
    /// cannot resolve a crossing (treated as "no boundary detail on this edge").
    fn edge_crossing(&self, a: [f64; 3], b: [f64; 3]) -> Option<([f64; 3], [f64; 3])>;

    /// The (unit) outward normal of the boundary surface at a point `p` ON or near
    /// `b == 0`. Used to project a seam vertex (a contact-meets-boundary vertex)
    /// ALONG the boundary onto the contact, smoothing it without leaving the wall —
    /// the field gradient is projected into this normal's tangent plane. Returns
    /// `None` if the implementor cannot give a normal there (then the seam vertex is
    /// left where the QEF placed it).
    fn surface_normal(&self, p: [f64; 3]) -> Option<[f64; 3]>;

    /// Unsigned world distance from `p` to THIS boundary's surface (approximate is fine).
    /// A seam vertex lies on whichever contributor's surface it is ACTUALLY on; the scene
    /// uses this to constrain the seam projection to the NEAREST contributor's tangent plane
    /// rather than the first one in the list (the domain box answers `surface_normal` for
    /// every point, so first-responder picks it wrongly when the real wall is a truncating
    /// contact). `None` if the implementor cannot estimate a distance — then it is not a
    /// candidate for the nearest-surface choice. Defaults to `None`.
    fn surface_distance(&self, _p: [f64; 3]) -> Option<f64> {
        None
    }

    /// How many DISTINCT surfaces of this boundary cross the edge `a → b` (for the
    /// double-crossed-edge refinement). Defaults to `edge_crossing().is_some()` (0 or
    /// 1) — correct for a single level set. A multi-contact boundary (a
    /// [`SlabComplement`], whose two contacts can both cross one coarse edge) overrides
    /// this so such a cell is subdivided until each edge crosses at most one contact,
    /// preserving the watertight single-crossing-per-edge contour.
    fn edge_crossing_count(&self, a: [f64; 3], b: [f64; 3]) -> usize {
        self.edge_crossing(a, b).is_some() as usize
    }
}

/// A geologic contact (a `Model` field thresholded at an isovalue) expressed as a
/// [`Boundary`] contributor, so it can join other surfaces in a [`Scene`]'s CSG
/// combine. "Inside" is below the contact (`field < iso`); the crossing normal is
/// the field gradient. This is how a second contact (e.g. an erosion surface that
/// truncates a stack) enters the combined field as just-another-`Boundary`.
pub struct ContactSurface<'a> {
    model: &'a dyn ScalarField,
    isovalue: f64,
}

impl<'a> ContactSurface<'a> {
    pub fn new(model: &'a dyn ScalarField, isovalue: f64) -> Self {
        Self { model, isovalue }
    }
}

impl Boundary for ContactSurface<'_> {
    fn inside(&self, p: [f64; 3]) -> bool {
        self.model.scalar_field(p) - self.isovalue < 0.0
    }

    fn edge_crossing(&self, a: [f64; 3], b: [f64; 3]) -> Option<([f64; 3], [f64; 3])> {
        let fa = self.model.scalar_field(a) - self.isovalue;
        let fb = self.model.scalar_field(b) - self.isovalue;
        if (fa < 0.0) == (fb < 0.0) {
            return None;
        }
        let t = fa / (fa - fb);
        let p = [
            a[0] + (b[0] - a[0]) * t,
            a[1] + (b[1] - a[1]) * t,
            a[2] + (b[2] - a[2]) * t,
        ];
        Some((p, normalize(self.model.scalar_field_gradient(p))))
    }

    fn surface_normal(&self, p: [f64; 3]) -> Option<[f64; 3]> {
        Some(normalize(self.model.scalar_field_gradient(p)))
    }

    fn surface_distance(&self, p: [f64; 3]) -> Option<f64> {
        let df = (self.model.scalar_field(p) - self.isovalue).abs();
        let g = self.model.scalar_field_gradient(p);
        let gn = (g[0] * g[0] + g[1] * g[1] + g[2] * g[2]).sqrt();
        Some(if gn > 1e-12 { df / gn } else { df })
    }
}

/// One side of a `Model` field's contact `field == iso`, as a [`Boundary`].
///
/// `inside` is the BELOW side (`field < iso`) when `below`, else the ABOVE side
/// (`field > iso`); the crossing normal is the field gradient. Unlike
/// [`ContactSurface`] (which is fixed to the below side), this carries an explicit
/// side flag — needed by the multi-series [`crate::stack`] to clip an older series'
/// surface to the half OUTSIDE a younger eroding body (`below = false`, the
/// `field >= iso` side), and to build slabs (`below(hi) ∩ above(lo)`) from one field.
pub struct ContactHalfSpace<'a> {
    model: &'a dyn ScalarField,
    isovalue: f64,
    /// `true` keeps the below side (`field < iso`) inside; `false` the above side.
    below: bool,
}

impl<'a> ContactHalfSpace<'a> {
    /// A half-space of `model`'s contact at `isovalue`. `below = true` → the
    /// `field < iso` side is inside; `below = false` → the `field > iso` side.
    pub fn new(model: &'a dyn ScalarField, isovalue: f64, below: bool) -> Self {
        Self {
            model,
            isovalue,
            below,
        }
    }
}

impl Boundary for ContactHalfSpace<'_> {
    fn inside(&self, p: [f64; 3]) -> bool {
        let f = self.model.scalar_field(p) - self.isovalue;
        if self.below {
            f < 0.0
        } else {
            f > 0.0
        }
    }

    fn edge_crossing(&self, a: [f64; 3], b: [f64; 3]) -> Option<([f64; 3], [f64; 3])> {
        let fa = self.model.scalar_field(a) - self.isovalue;
        let fb = self.model.scalar_field(b) - self.isovalue;
        if (fa < 0.0) == (fb < 0.0) {
            return None;
        }
        let t = fa / (fa - fb);
        let p = [
            a[0] + (b[0] - a[0]) * t,
            a[1] + (b[1] - a[1]) * t,
            a[2] + (b[2] - a[2]) * t,
        ];
        Some((p, normalize(self.model.scalar_field_gradient(p))))
    }

    fn surface_normal(&self, p: [f64; 3]) -> Option<[f64; 3]> {
        Some(normalize(self.model.scalar_field_gradient(p)))
    }

    fn surface_distance(&self, p: [f64; 3]) -> Option<f64> {
        let df = (self.model.scalar_field(p) - self.isovalue).abs();
        let g = self.model.scalar_field_gradient(p);
        let gn = (g[0] * g[0] + g[1] * g[1] + g[2] * g[2]).sqrt();
        Some(if gn > 1e-12 { df / gn } else { df })
    }
}

/// The region OUTSIDE a field's slab `lo <= field <= hi`, as a [`Boundary`] — i.e.
/// `inside(p)` is `field(p) < lo OR field(p) > hi` (the complement of the slab).
///
/// This is the correct clip a cross-cutting **vein** (or shell intrusion) contributes
/// to an OLDER element it cross-cuts: the older surface survives OUTSIDE the body's
/// band and is removed INSIDE it. "Outside a slab" is a UNION of two half-spaces, which
/// ANDed [`ContactHalfSpace`] contributors cannot express (ANDing `above-lo` with
/// `below-hi` gives the band INTERIOR — the inverted clip). A single `SlabComplement`
/// contributor expresses the union directly, ANDed with the rest of the Scene as usual.
///
/// `edge_crossing` reports whichever of the two contacts (`lo`/`hi`) the edge crosses;
/// the [`Scene`]'s double-crossed-edge refinement guarantees each minimal edge crosses
/// at most one contact, so a single crossing per edge is sufficient for the contour.
/// The outward normal points INTO the slab (out of the kept region): `+∇field` at the
/// `lo` contact (kept side `field < lo`), `−∇field` at the `hi` contact (kept side
/// `field > hi`).
pub struct SlabComplement<'a> {
    field: &'a dyn ScalarField,
    lo: f64,
    hi: f64,
}

impl<'a> SlabComplement<'a> {
    /// Outside the slab `[lo, hi]` of `field`. `lo`/`hi` are normalized so `lo <= hi`.
    pub fn new(field: &'a dyn ScalarField, lo: f64, hi: f64) -> Self {
        let (lo, hi) = if lo <= hi { (lo, hi) } else { (hi, lo) };
        Self { field, lo, hi }
    }
}

impl Boundary for SlabComplement<'_> {
    fn inside(&self, p: [f64; 3]) -> bool {
        let f = self.field.scalar_field(p);
        f < self.lo || f > self.hi
    }

    fn edge_crossing(&self, a: [f64; 3], b: [f64; 3]) -> Option<([f64; 3], [f64; 3])> {
        let fa = self.field.scalar_field(a);
        let fb = self.field.scalar_field(b);
        // The kept-region membership flips when the edge crosses either contact. Find the
        // contact this edge crosses (at most one, after double-crossed-edge refinement); the
        // outward normal points into the slab (out of the kept region).
        for (iso, sign) in [(self.lo, 1.0_f64), (self.hi, -1.0_f64)] {
            let (ga, gb) = (fa - iso, fb - iso);
            if (ga < 0.0) != (gb < 0.0) {
                let t = ga / (ga - gb);
                let p = [
                    a[0] + (b[0] - a[0]) * t,
                    a[1] + (b[1] - a[1]) * t,
                    a[2] + (b[2] - a[2]) * t,
                ];
                let g = normalize(self.field.scalar_field_gradient(p));
                return Some((p, [sign * g[0], sign * g[1], sign * g[2]]));
            }
        }
        None
    }

    fn surface_normal(&self, p: [f64; 3]) -> Option<[f64; 3]> {
        // The nearer contact determines the local wall orientation.
        let f = self.field.scalar_field(p);
        let sign = if (f - self.lo).abs() <= (f - self.hi).abs() { 1.0 } else { -1.0 };
        let g = normalize(self.field.scalar_field_gradient(p));
        Some([sign * g[0], sign * g[1], sign * g[2]])
    }

    fn surface_distance(&self, p: [f64; 3]) -> Option<f64> {
        // First-order world distance to the NEARER band wall: |field − nearer edge| / |∇field|.
        let f = self.field.scalar_field(p);
        let df = (f - self.lo).abs().min((f - self.hi).abs());
        let g = self.field.scalar_field_gradient(p);
        let gn = (g[0] * g[0] + g[1] * g[1] + g[2] * g[2]).sqrt();
        Some(if gn > 1e-12 { df / gn } else { df })
    }

    fn edge_crossing_count(&self, a: [f64; 3], b: [f64; 3]) -> usize {
        let fa = self.field.scalar_field(a);
        let fb = self.field.scalar_field(b);
        // Count each of the two band contacts the edge crosses — a thin band relative to
        // the cell can cross both on one edge, which must force a subdivision.
        let lo_cross = (fa < self.lo) != (fb < self.lo);
        let hi_cross = (fa < self.hi) != (fb < self.hi);
        lo_cross as usize + hi_cross as usize
    }
}

/// A pure geometric plane half-space, as a [`Boundary`]. `inside` keeps the chosen
/// side of the plane (`(p − point)·normal >= 0` when `keep_positive`, else the
/// negative side); `edge_crossing` is the exact segment/plane intersection. Unlike
/// [`ContactHalfSpace`] (a `Model`-field level set) this is a fixed plane — used to
/// clip a per-block contact mesh to one side of a planar fault, so a faulted contact
/// renders as separate, unconnected sheets. Built from a [`crate::fault::HalfSpace`].
pub struct PlaneHalfSpace {
    point: [f64; 3],
    normal: [f64; 3],
    keep_positive: bool,
}

impl PlaneHalfSpace {
    pub fn new(point: [f64; 3], normal: [f64; 3], keep_positive: bool) -> Self {
        Self {
            point,
            normal: normalize(normal),
            keep_positive,
        }
    }

    /// Signed perpendicular distance `(p − point)·normal` (positive on the +normal side).
    fn signed(&self, p: [f64; 3]) -> f64 {
        (p[0] - self.point[0]) * self.normal[0]
            + (p[1] - self.point[1]) * self.normal[1]
            + (p[2] - self.point[2]) * self.normal[2]
    }
}

impl Boundary for PlaneHalfSpace {
    fn inside(&self, p: [f64; 3]) -> bool {
        let s = self.signed(p);
        // Inside the KEPT side; `b < 0` is the inside convention, so "kept" maps to
        // s>=0 (keep_positive) or s<=0 (else).
        if self.keep_positive {
            s >= 0.0
        } else {
            s <= 0.0
        }
    }

    fn edge_crossing(&self, a: [f64; 3], b: [f64; 3]) -> Option<([f64; 3], [f64; 3])> {
        let sa = self.signed(a);
        let sb = self.signed(b);
        if (sa < 0.0) == (sb < 0.0) {
            return None;
        }
        let t = sa / (sa - sb);
        let p = [
            a[0] + (b[0] - a[0]) * t,
            a[1] + (b[1] - a[1]) * t,
            a[2] + (b[2] - a[2]) * t,
        ];
        // Outward normal points OUT of the kept side.
        let n = if self.keep_positive {
            self.normal
        } else {
            [-self.normal[0], -self.normal[1], -self.normal[2]]
        };
        Some((p, n))
    }

    fn surface_normal(&self, _p: [f64; 3]) -> Option<[f64; 3]> {
        Some(if self.keep_positive {
            self.normal
        } else {
            [-self.normal[0], -self.normal[1], -self.normal[2]]
        })
    }
}

/// The combined implicit solid the contourer meshes: the PRIMARY contact (`model`
/// at `isovalue`) intersected with a list of additional [`Boundary`] contributors
/// (the domain box, erosion/truncation surfaces, …). A point is INSIDE the solid
/// iff it is below the primary contact AND inside every contributor — the CSG
/// intersection (`g = max` of the signed fields), expressed purely at the sign +
/// edge-crossing level so no contributor needs a free-space scalar.
///
/// The primary contact is distinguished only because its gradient orients the open
/// sheet (`cap = false`); for the closed solid all contributors are symmetric.
pub struct Scene<'a> {
    model: &'a dyn ScalarField,
    isovalue: f64,
    /// Additional surfaces intersected with the primary contact. The first is
    /// conventionally the domain boundary; further entries are truncating contacts.
    contributors: Vec<&'a dyn Boundary>,
}

impl<'a> Scene<'a> {
    pub fn new(model: &'a dyn ScalarField, isovalue: f64, contributors: Vec<&'a dyn Boundary>) -> Self {
        Self {
            model,
            isovalue,
            contributors,
        }
    }

    /// Field-minus-isovalue of the primary contact at `p` (for the open-sheet sign
    /// and clip). Negative = below the contact.
    fn primary(&self, p: [f64; 3]) -> f64 {
        self.model.scalar_field(p) - self.isovalue
    }

    /// Is `p` inside every contributor (the domain ∩ all truncating surfaces)?
    fn contributors_inside(&self, p: [f64; 3]) -> bool {
        self.contributors.iter().all(|c| c.inside(p))
    }

    /// Is `p` inside every TRUNCATING contributor — i.e. every contributor EXCEPT the
    /// domain box (conventionally contributor 0)? `true` when there are no truncators
    /// (box-only scene). Used to classify open-sheet faces: a contact face is kept only
    /// where its primary crossing is on the un-eroded side of the truncators, so the open
    /// sheet terminates exactly on the contact∩truncator crease — WITHOUT excluding the
    /// box-daylight faces (which must stay flush on the domain wall).
    fn contributors_inside_truncators(&self, p: [f64; 3]) -> bool {
        // contributor 0 is the domain box (see `octree_dual_contour_bounded` and the
        // stack's `mesh_one_clipped`, both of which push the box first).
        self.contributors.iter().skip(1).all(|c| c.inside(p))
    }

    /// Primary-contact gradient at `p`.
    fn primary_gradient(&self, p: [f64; 3]) -> [f64; 3] {
        self.model.scalar_field_gradient(p)
    }

    /// Outward normal of the contributor (domain) surface a seam vertex at `p` sits
    /// on — for constraining seam-vertex projection to stay on the boundary. Picks
    /// the first contributor that can supply a normal (the convention is contributor
    /// 0 is the domain). `None` if none can.
    fn seam_boundary_normal(&self, p: [f64; 3]) -> Option<[f64; 3]> {
        // Constrain the seam to the contributor whose surface `p` is ACTUALLY on — the NEAREST one
        // by `surface_distance`, not the first responder. (The domain box answers `surface_normal`
        // for every point, so first-responder wrongly slides a contact↔vein-wall seam vertex along
        // a box face instead of the vein wall, leaving the contact crease off the wall — the
        // see-through dashes.) Fall back to first-responder when no contributor reports a distance.
        let nearest = self
            .contributors
            .iter()
            .filter_map(|c| c.surface_distance(p).map(|d| (d, c)))
            .min_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
        if let Some((_, c)) = nearest {
            if let Some(n) = c.surface_normal(p) {
                return Some(n);
            }
        }
        self.contributors.iter().find_map(|c| c.surface_normal(p))
    }

    /// All Hermite crossings on the cell edge `pa..pb` (each `(point, unit
    /// normal)`): the primary contact crossing (if the contact field changes sign,
    /// `va`/`vb` being its corner values) plus every contributor's `edge_crossing`.
    /// Together these pin a component's QEF vertex to the contact, a contributor
    /// surface, or their crease. Appended into `out`.
    fn edge_crossings(
        &self,
        pa: [f64; 3],
        pb: [f64; 3],
        va: f64,
        vb: f64,
        out: &mut Vec<([f64; 3], [f64; 3])>,
    ) {
        if (va <= 0.0) != (vb <= 0.0) {
            let t = va / (va - vb);
            let p = [
                pa[0] + (pb[0] - pa[0]) * t,
                pa[1] + (pb[1] - pa[1]) * t,
                pa[2] + (pb[2] - pa[2]) * t,
            ];
            out.push((p, normalize(self.primary_gradient(p))));
        }
        for c in &self.contributors {
            if let Some((p, n)) = c.edge_crossing(pa, pb) {
                out.push((p, normalize(n)));
            }
        }
    }

    /// Does the cell edge `pa..pb` carry crossings from TWO OR MORE different
    /// surfaces of the scene (primary contact + contributors)?
    ///
    /// Two distinct surfaces crossing one edge means two sheets of the solid
    /// boundary can pass through this one cell; a single dual vertex would average
    /// them into a non-pairing point (a crack). `build` subdivides such cells so
    /// each edge has at most one surface crossing, after which the single-vertex
    /// minimal-edge contour is watertight.
    ///
    /// We deliberately count crossings of ALL surfaces, not only those currently on
    /// the solid boundary: a buried crossing (inside or outside the solid here) can
    /// become a boundary crossing one cell over, and subdividing conservatively is
    /// cheap and keeps the criterion local and monotone. (Filtering to on-boundary
    /// crossings under-subdivides and leaves cracks — measured.)
    fn edge_double_crossed(&self, pa: [f64; 3], pb: [f64; 3]) -> bool {
        let mut count = 0;
        let fa = self.primary(pa);
        let fb = self.primary(pb);
        if (fa <= 0.0) != (fb <= 0.0) {
            count += 1;
        }
        for c in &self.contributors {
            count += c.edge_crossing_count(pa, pb);
            if count >= 2 {
                return true;
            }
        }
        count >= 2
    }
}

/// Axis-aligned box boundary: the domain is the cube `[min, min+size]³` (an
/// extruded rectangle in Z, matching the original hardcoded extent). `b < 0`
/// strictly inside; the cap `b == 0` is the six faces.
///
/// This is the first `Boundary` impl purely so behavior matches the prior
/// box-extent closure. The contourer treats it like any other boundary.
pub struct AxisBox {
    min: [f64; 3],
    max: [f64; 3],
}

impl AxisBox {
    pub fn new(min: [f64; 3], size: f64) -> Self {
        Self {
            min,
            max: [min[0] + size, min[1] + size, min[2] + size],
        }
    }

    /// An arbitrary (non-cube) axis-aligned box from explicit bounds. Used to clip a mesh
    /// to the TRUE data extent when the octree cube (side = max span) balloons one axis far
    /// beyond the geology — without this, the cokriging field's linear drift re-crosses the
    /// isovalue out in the empty extrapolated space and the contour grows spurious lobes on
    /// the cube's floor/ceiling.
    pub fn from_bounds(min: [f64; 3], max: [f64; 3]) -> Self {
        Self { min, max }
    }

    /// Signed "box field": negative strictly inside, zero on a face, positive
    /// outside. The classic box SDF (max of per-axis slab distances) — its sign
    /// is the membership test and its zero set is the six faces.
    fn b(&self, p: [f64; 3]) -> f64 {
        let mut d = f64::NEG_INFINITY;
        for ax in 0..3 {
            d = d.max(self.min[ax] - p[ax]).max(p[ax] - self.max[ax]);
        }
        d
    }
}

impl Boundary for AxisBox {
    fn inside(&self, p: [f64; 3]) -> bool {
        self.b(p) < 0.0
    }

    fn edge_crossing(&self, a: [f64; 3], b: [f64; 3]) -> Option<([f64; 3], [f64; 3])> {
        // The segment a→b crosses some face plane. Find the crossing parameter by
        // root-bracketing the box field along the segment (it is piecewise-linear
        // and monotone across a single face here, since cells are small relative
        // to the box). Then the normal is the outward normal of whichever face
        // the crossing point lies on.
        let fa = self.b(a);
        let fb = self.b(b);
        if (fa < 0.0) == (fb < 0.0) {
            return None;
        }
        let t = fa / (fa - fb);
        let p = [
            a[0] + (b[0] - a[0]) * t,
            a[1] + (b[1] - a[1]) * t,
            a[2] + (b[2] - a[2]) * t,
        ];
        // Outward face normal: the axis/side whose slab distance is largest at p
        // (i.e. the face p sits on). Ties (box edges/corners) are vanishingly
        // unlikely on a crossing edge; pick the dominant axis.
        let mut best = (0usize, 0.0_f64, 1.0_f64); // (axis, |dist|, sign)
        for ax in 0..3 {
            let lo = self.min[ax] - p[ax];
            let hi = p[ax] - self.max[ax];
            if lo.abs() >= best.1 {
                best = (ax, lo.abs(), -1.0);
            }
            if hi.abs() >= best.1 {
                best = (ax, hi.abs(), 1.0);
            }
        }
        let mut n = [0.0; 3];
        n[best.0] = best.2;
        Some((p, n))
    }

    fn surface_normal(&self, p: [f64; 3]) -> Option<[f64; 3]> {
        // Outward normal of the box face `p` is closest to (the dominant slab axis).
        let mut best = (0usize, f64::NEG_INFINITY, 1.0_f64);
        for ax in 0..3 {
            let lo = (self.min[ax] - p[ax]).abs();
            let hi = (p[ax] - self.max[ax]).abs();
            if -lo > best.1 {
                best = (ax, -lo, -1.0);
            }
            if -hi > best.1 {
                best = (ax, -hi, 1.0);
            }
        }
        let mut n = [0.0; 3];
        n[best.0] = best.2;
        Some(n)
    }

    fn surface_distance(&self, p: [f64; 3]) -> Option<f64> {
        // Distance to the nearest box FACE (not the box interior): min over axes of the distance
        // to either face plane. A point deep inside the domain is far from every face.
        let mut best = f64::INFINITY;
        for ax in 0..3 {
            best = best
                .min((self.min[ax] - p[ax]).abs())
                .min((p[ax] - self.max[ax]).abs());
        }
        Some(best)
    }
}

/// Triangulated-mesh boundary: the domain is the inside of a closed triangle
/// mesh (e.g. a fault delivered as a mesh). This is the second `Boundary` impl,
/// and the one that proves the trait carries its weight — it answers membership
/// and Hermite queries with NO closed-form gradient anywhere:
///
///   * `inside` is a generalized winding number (robust to imperfect meshes, not
///     a parity ray cast), so a not-quite-watertight input still classifies
///     correctly;
///   * `edge_crossing` is a segment/triangle intersection, with the normal taken
///     from the intersected triangle — exactly the "normal only at the crossing"
///     contract the trait was designed around.
///
/// Triangles are expected wound counter-clockwise as seen from OUTSIDE (outward
/// normals), so the winding number is positive inside and the hit-triangle normal
/// already points out of the domain.
pub struct TriMesh {
    vertices: Vec<[f64; 3]>,
    triangles: Vec<[usize; 3]>,
}

impl TriMesh {
    pub fn new(vertices: Vec<[f64; 3]>, triangles: Vec<[usize; 3]>) -> Self {
        Self {
            vertices,
            triangles,
        }
    }

    /// The three world-space corners of triangle `t`.
    fn tri(&self, t: usize) -> ([f64; 3], [f64; 3], [f64; 3]) {
        let [i, j, k] = self.triangles[t];
        (self.vertices[i], self.vertices[j], self.vertices[k])
    }

    /// Outward (CCW) geometric normal of triangle `t`, normalized.
    fn tri_normal(&self, t: usize) -> [f64; 3] {
        let (a, b, c) = self.tri(t);
        let ab = sub(b, a);
        let ac = sub(c, a);
        normalize(cross(ab, ac))
    }

    /// Generalized winding number of the mesh at `p`, in turns (≈ +1 strictly
    /// inside a closed outward-wound mesh, ≈ 0 outside). Sum of per-triangle
    /// signed solid angles (Van Oosterom–Strackee) over 4π.
    fn winding(&self, p: [f64; 3]) -> f64 {
        let mut omega = 0.0;
        for t in 0..self.triangles.len() {
            let (va, vb, vc) = self.tri(t);
            let a = sub(va, p);
            let b = sub(vb, p);
            let c = sub(vc, p);
            let la = norm(a);
            let lb = norm(b);
            let lc = norm(c);
            if la == 0.0 || lb == 0.0 || lc == 0.0 {
                continue; // p coincides with a vertex; treat as no contribution
            }
            let numer = det3(a, b, c);
            let denom = la * lb * lc
                + dot(a, b) * lc
                + dot(b, c) * la
                + dot(c, a) * lb;
            omega += 2.0 * numer.atan2(denom);
        }
        omega / (4.0 * std::f64::consts::PI)
    }
}

impl Boundary for TriMesh {
    fn inside(&self, p: [f64; 3]) -> bool {
        // Inside ⟺ winding number near 1 (use 0.5 as the robust threshold). This
        // is the `b < 0` convention: `b` is "outside-ness" and its zero set is the
        // mesh surface.
        self.winding(p) > 0.5
    }

    fn edge_crossing(&self, a: [f64; 3], b: [f64; 3]) -> Option<([f64; 3], [f64; 3])> {
        // Nearest segment/triangle intersection along a→b. Normal is the hit
        // triangle's outward normal — the only place this boundary produces one.
        let dir = sub(b, a);
        let mut best_t = f64::INFINITY;
        let mut hit: Option<([f64; 3], [f64; 3])> = None;
        for ti in 0..self.triangles.len() {
            let (v0, v1, v2) = self.tri(ti);
            if let Some(t) = ray_triangle(a, dir, v0, v1, v2) {
                if (0.0..=1.0).contains(&t) && t < best_t {
                    best_t = t;
                    let p = [a[0] + dir[0] * t, a[1] + dir[1] * t, a[2] + dir[2] * t];
                    hit = Some((p, self.tri_normal(ti)));
                }
            }
        }
        hit
    }

    fn surface_normal(&self, p: [f64; 3]) -> Option<[f64; 3]> {
        // Normal of the nearest triangle (by centroid distance — enough to pick the
        // facet a near-surface seam vertex sits on).
        let mut best = (f64::INFINITY, 0usize);
        for ti in 0..self.triangles.len() {
            let (a, b, c) = self.tri(ti);
            let ctr = [
                (a[0] + b[0] + c[0]) / 3.0,
                (a[1] + b[1] + c[1]) / 3.0,
                (a[2] + b[2] + c[2]) / 3.0,
            ];
            let d = norm(sub(p, ctr));
            if d < best.0 {
                best = (d, ti);
            }
        }
        if best.0.is_finite() {
            Some(self.tri_normal(best.1))
        } else {
            None
        }
    }
}

// Small vector helpers for the TriMesh boundary (kept local — the rest of the
// contourer works in raw [f64; 3] and doesn't need a vec type).
fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
fn norm(a: [f64; 3]) -> f64 {
    dot(a, a).sqrt()
}
/// Determinant of the matrix with rows a, b, c — i.e. the scalar triple product
/// a · (b × c). Used for the solid-angle numerator.
fn det3(a: [f64; 3], b: [f64; 3], c: [f64; 3]) -> f64 {
    dot(a, cross(b, c))
}

/// Möller–Trumbore ray/triangle intersection. Returns the ray parameter `t`
/// (so the hit is `origin + t*dir`) for a front- or back-facing hit, or None if
/// the ray misses the triangle's plane within its bounds. `t` is not clamped —
/// the caller checks `t ∈ [0, 1]` for a segment hit.
fn ray_triangle(
    origin: [f64; 3],
    dir: [f64; 3],
    v0: [f64; 3],
    v1: [f64; 3],
    v2: [f64; 3],
) -> Option<f64> {
    const EPS: f64 = 1e-12;
    let e1 = sub(v1, v0);
    let e2 = sub(v2, v0);
    let pvec = cross(dir, e2);
    let det = dot(e1, pvec);
    if det.abs() < EPS {
        return None; // ray parallel to triangle
    }
    let inv_det = 1.0 / det;
    let tvec = sub(origin, v0);
    let u = dot(tvec, pvec) * inv_det;
    if !(0.0..=1.0).contains(&u) {
        return None;
    }
    let qvec = cross(tvec, e1);
    let v = dot(dir, qvec) * inv_det;
    if v < 0.0 || u + v > 1.0 {
        return None;
    }
    Some(dot(e2, qvec) * inv_det)
}

/// A node in the dual-contouring octree.
enum Node {
    /// Internal node: 8 children indexed by corner number (x + 2y + 4z).
    Internal(Box<[Option<NodeId>; 8]>),
    /// Leaf with a surface crossing (field and/or boundary): holds its dual
    /// vertex (index into the output mesh, filled during contouring) and the
    /// corner sign data for both surfaces.
    Leaf(Leaf),
}

struct Leaf {
    /// Field-minus-isovalue at the 8 cell corners.
    corners: [f64; 8],
    /// Boundary membership at the 8 cell corners: `true` = inside (`b < 0`).
    inside: [bool; 8],
    /// Output mesh vertex index per surface COMPONENT (one sheet → one entry; a
    /// two-sheet cell → two). Filled in `assign_vertices` (which knows `cap`).
    /// Empty until then / for non-surface leaves.
    out: Vec<u32>,
    /// For each of the 12 cell edges that the contoured surface crosses, the index
    /// into `out` of the component that owns it (the component of the edge's
    /// minority-sign endpoint). `u8::MAX` for non-crossing edges. Lets quad emission
    /// route a shared edge to the right component's vertex.
    edge_owner: [u8; 12],
}

type NodeId = usize;

/// Arena-backed octree (nodes in a Vec; children referenced by index, so the
/// recursion borrows the arena immutably).
struct Octree {
    nodes: Vec<Node>,
    /// Min corner + uniform size of each node's cube, parallel to `nodes`.
    bounds: Vec<([f64; 3], f64)>,
    root: NodeId,
}

/// Refinement / contouring configuration.
#[derive(Debug, Clone, Copy)]
pub struct OctreeConfig {
    /// Maximum subdivision depth (root is depth 0). The finest cell size is
    /// `extent / 2^max_depth`.
    pub max_depth: u32,
    /// Minimum depth to force everywhere (uniform floor before adaptivity).
    pub min_depth: u32,
    /// QEF residual above which a sign-changing cell is subdivided.
    pub qef_threshold: f64,
    /// Whether to contour the domain-boundary cap (`b == 0`, kept where
    /// `field < iso`). When `true`, the output is a watertight closed surface
    /// (field sheet + cap). When `false`, only the field sheet is emitted —
    /// an OPEN surface clipped to the domain, with its perimeter flush on the
    /// boundary. The harness uses `false` to show the bounded contact surface
    /// and `true` to show the closed solid.
    pub cap: bool,
}

impl Default for OctreeConfig {
    fn default() -> Self {
        Self {
            max_depth: 6,
            min_depth: 2,
            qef_threshold: 1e-4,
            cap: true,
        }
    }
}

/// Dual-contour the isosurface `field == isovalue` over the cube `[min, max]`,
/// closed against the axis-aligned-box domain (the original behavior). Adaptive:
/// fine near detail, coarse in flat regions.
pub fn octree_dual_contour(
    model: &dyn ScalarField,
    min: [f64; 3],
    size: f64,
    isovalue: f64,
    cfg: OctreeConfig,
) -> Mesh {
    let boundary = AxisBox::new(min, size);
    octree_dual_contour_bounded(model, &boundary, min, size, isovalue, cfg)
}

/// Dual-contour `field == isovalue`, closed against an arbitrary [`Boundary`].
///
/// With `cfg.cap = true` (the default) this contours the **combined solid field**
/// `g = max(field − iso, b)` as a SINGLE dual-contouring pass — the standard CSG
/// intersection-of-SDFs identity (`g < 0` ⟺ `field < iso` AND inside the domain).
/// Because there is one sign field, the seam where the contact meets the boundary
/// is just an ordinary crease that DC meshes watertight — no field/cap split, no
/// seam reconciliation. (This replaced an earlier two-pass field+cap scheme whose
/// seam could not be made manifold for non-grid-aligned boundaries; see the module
/// note and `docs/`.) The single QEF vertex per cell already carries both the
/// field and the boundary Hermite samples, so the crease vertex snaps to the
/// contact/boundary intersection.
///
/// With `cfg.cap = false` this contours just the **field surface clipped to the
/// domain** — an OPEN sheet (the bounded contact surface), no cap.
///
/// `[min, min+size]³` is the octree's sampling cube.
pub fn octree_dual_contour_bounded(
    model: &dyn ScalarField,
    boundary: &dyn Boundary,
    min: [f64; 3],
    size: f64,
    isovalue: f64,
    cfg: OctreeConfig,
) -> Mesh {
    let scene = Scene::new(model, isovalue, vec![boundary]);
    octree_dual_contour_scene(&scene, min, size, cfg)
}

/// Dual-contour the combined [`Scene`] solid (primary contact ∩ all contributors)
/// over the cube `[min, min+size]³`. This is the general entry point: a single
/// contributor is the bounded-contact case; adding a truncating `ContactSurface`
/// contributor meshes an erosion/unconformity cutting the primary contact. The
/// two-sheet cells where surfaces intersect are split per component (Manifold DC)
/// so the result stays watertight.
pub fn octree_dual_contour_scene(
    scene: &Scene,
    min: [f64; 3],
    size: f64,
    cfg: OctreeConfig,
) -> Mesh {
    let mut tree = Octree {
        nodes: Vec::new(),
        bounds: Vec::new(),
        root: 0,
    };
    tree.root = build(&mut tree, scene, min, size, 0, &cfg);
    // Subdivide cells where two surfaces cross one edge (two sheets through one
    // cell) before balancing — a single dual vertex there averages the sheets into
    // a crack. Refining to a fixpoint (capped at max_depth) makes each edge
    // single-crossed, after which the single-vertex minimal-edge contour is
    // watertight.
    refine_double_crossed(&mut tree, scene, &cfg);
    balance(&mut tree, scene, &cfg);

    // Always contour the watertight closed SOLID — its boundary closure handles the
    // contact-meets-domain seam cleanly (a bare open sheet has no such closure and
    // tears at the boundary). For the open-sheet view (`cap = false`) we then keep
    // only the contact (field-crossing) faces, leaving the contact surface flush
    // along the boundary. Each triangle is tagged at emission as a contact face
    // (the primary field crosses its minimal edge) or a cap face (a contributor
    // surface crosses it) — unambiguous, unlike a post-hoc geometric test.
    let mut mesh = Mesh::default();
    let mut contact_face: Vec<bool> = Vec::new();
    assign_vertices(&mut tree, scene, &mut mesh, true);
    contour_edges(&tree, scene, &mut mesh, &mut contact_face);
    orient_closed_outward(&mut mesh);
    if !cfg.cap {
        keep_contact_faces(&mut mesh, &contact_face);
    }
    mesh
}

/// Re-sample every leaf's cached corner data (`corners`, `inside`) from `scene`, and clear the
/// per-contour output (`out`, `edge_owner`). Used by [`octree_dual_contour_shared`] to contour a
/// DIFFERENT scene on a tree whose topology was built for another — so both contours run on the
/// SAME cell subdivision and their shared-edge crossings coincide.
fn resample_leaves(tree: &mut Octree, scene: &Scene) {
    for id in 0..tree.nodes.len() {
        let (min, size) = tree.bounds[id];
        if let Node::Leaf(_) = &tree.nodes[id] {
            let (corners, inside) = sample_corners(scene, min, size);
            if let Node::Leaf(l) = &mut tree.nodes[id] {
                l.corners = corners;
                l.inside = inside;
                l.out = Vec::new();
                l.edge_owner = [u8::MAX; 12];
            }
        }
    }
}

/// Contour `scene` on an already-built `tree` (its leaves re-sampled for `scene`), returning the
/// mesh. Mirrors the tail of [`octree_dual_contour_scene`] but takes the tree as given — no
/// build/refine/balance, so the subdivision is whatever the caller built.
fn contour_on_tree(tree: &mut Octree, scene: &Scene, cfg: &OctreeConfig) -> Mesh {
    let mut mesh = Mesh::default();
    let mut contact_face: Vec<bool> = Vec::new();
    assign_vertices(tree, scene, &mut mesh, true);
    contour_edges(tree, scene, &mut mesh, &mut contact_face);
    orient_closed_outward(&mut mesh);
    if !cfg.cap {
        keep_contact_faces(&mut mesh, &contact_face);
    }
    mesh
}

/// SHARED-CREASE contouring. Build ONE octree from `build_scene` (which must carry every other
/// surface as a contributor, so refinement subdivides wherever ANY of them crosses — `is_surface`
/// and `edge_double_crossed` already consider contributors), then contour each scene in `scenes`
/// on that SAME subdivision. Because both passes evaluate `edge_crossings` on identical cell edges,
/// a crease shared by two surfaces lands on the SAME edge crossings in both meshes — no T-junctions
/// or see-through holes at the seam (the octree.rs:35-37 follow-up, solved by sharing the tree).
///
/// Returns one mesh per scene, in order. `build_scene` is typically `scenes[0]` (the primary), but
/// is passed separately so the caller can use a richer refinement scene than any single contour.
pub fn octree_dual_contour_shared(
    build_scene: &Scene,
    scenes: &[&Scene],
    min: [f64; 3],
    size: f64,
    cfg: OctreeConfig,
) -> Vec<Mesh> {
    let mut tree = Octree {
        nodes: Vec::new(),
        bounds: Vec::new(),
        root: 0,
    };
    tree.root = build(&mut tree, build_scene, min, size, 0, &cfg);
    refine_double_crossed(&mut tree, build_scene, &cfg);
    balance(&mut tree, build_scene, &cfg);

    scenes
        .iter()
        .map(|s| {
            resample_leaves(&mut tree, s);
            contour_on_tree(&mut tree, s, &cfg)
        })
        .collect()
}

/// Flood-fill a globally consistent winding across a closed mesh's shared edges,
/// then flip the whole mesh outward by signed volume. No per-face probe, so no
/// back-face band.
fn orient_closed_outward(mesh: &mut Mesh) {
    let tris = mesh.indices.len() / 3;
    if tris == 0 {
        return;
    }
    let tri = |t: usize| {
        [
            mesh.indices[3 * t],
            mesh.indices[3 * t + 1],
            mesh.indices[3 * t + 2],
        ]
    };

    // Undirected edge → triangles sharing it, each with the directed pair as that
    // triangle currently traverses it.
    let mut edge_tris: std::collections::HashMap<(u32, u32), Vec<(usize, (u32, u32))>> =
        std::collections::HashMap::new();
    for t in 0..tris {
        let v = tri(t);
        for &(a, b) in &[(v[0], v[1]), (v[1], v[2]), (v[2], v[0])] {
            let e = if a < b { (a, b) } else { (b, a) };
            edge_tris.entry(e).or_default().push((t, (a, b)));
        }
    }

    // BFS each connected component, flipping a neighbor when it traverses a shared
    // edge in the SAME direction as us (inconsistent winding).
    let mut flip = vec![false; tris];
    let mut visited = vec![false; tris];
    for start in 0..tris {
        if visited[start] {
            continue;
        }
        visited[start] = true;
        let mut stack = vec![start];
        while let Some(t) = stack.pop() {
            let v = tri(t);
            for &(a, b) in &[(v[0], v[1]), (v[1], v[2]), (v[2], v[0])] {
                let e = if a < b { (a, b) } else { (b, a) };
                for &(nt, (na, nb)) in &edge_tris[&e] {
                    if nt == t || visited[nt] {
                        continue;
                    }
                    let our = if flip[t] { (b, a) } else { (a, b) };
                    let theirs = if flip[nt] { (nb, na) } else { (na, nb) };
                    // Consistent iff we traverse the edge opposite to them.
                    if our != (theirs.1, theirs.0) {
                        flip[nt] = true;
                    }
                    visited[nt] = true;
                    stack.push(nt);
                }
            }
        }
    }

    // Signed volume (×6) under the now-consistent winding; flip all if it's < 0.
    let pos = |i: u32| {
        let b = (i as usize) * 3;
        [
            mesh.positions[b] as f64,
            mesh.positions[b + 1] as f64,
            mesh.positions[b + 2] as f64,
        ]
    };
    let mut vol6 = 0.0;
    for t in 0..tris {
        let v0 = tri(t);
        let v = if flip[t] {
            [v0[0], v0[2], v0[1]]
        } else {
            v0
        };
        let a = pos(v[0]);
        let b = pos(v[1]);
        let c = pos(v[2]);
        vol6 += a[0] * (b[1] * c[2] - b[2] * c[1]) - a[1] * (b[0] * c[2] - b[2] * c[0])
            + a[2] * (b[0] * c[1] - b[1] * c[0]);
    }
    let global_flip = vol6 < 0.0;

    for t in 0..tris {
        if flip[t] ^ global_flip {
            mesh.indices.swap(3 * t + 1, 3 * t + 2);
        }
    }
}

/// Magnitude (in `field − iso` units) below which a corner is treated as a
/// DEGENERATE graze — the contact passing through the corner exactly — and snapped
/// to a single canonical sign. A geological contact aligned to a grid plane (e.g. a
/// dipping plane that hits `z = z0` exactly at a cell boundary) lands `field − iso`
/// at the round-off floor (~1e-17), whose discrete sign (`<= 0`) is otherwise
/// build-dependent: native and wasm32 disagree in the last ULPs, flipping a corner
/// between "below" and "above" the contact. That flip propagates into both the
/// closed-solid topology (`corner_sign`) and the contact-face classification
/// (`is_contact`), and produced the "ceiling spike" — a contact face wrongly kept
/// and stretched to a domain-cap vertex on wasm only. Snapping any near-zero corner
/// to a fixed side makes the contour IDENTICAL across builds (the parity the engine
/// requires). `1e-9` sits ~1e8× above the round-off floor yet far below any genuine
/// per-cell field variation (a depth-6 cell changes `field − iso` by ~1e-2), so it
/// never reclassifies a real crossing.
const CONTACT_GRAZE_EPS: f64 = 1e-9;

/// Sample the scene at a node's 8 corners: `corners[c]` = primary contact field −
/// isovalue (negative below the contact); `inside[c]` = inside EVERY contributor
/// (the domain ∩ all truncating surfaces). The combined solid sign at a corner is
/// `corners[c] <= 0 && inside[c]` (see `corner_sign`).
///
/// A corner within [`CONTACT_GRAZE_EPS`] of the contact is snapped to a single
/// canonical sign (just ABOVE the contact, `+eps`) so a grid-aligned contact graze
/// classifies identically across native/wasm — the same world point sampled by every
/// cell that shares it gets the same value, so the snap preserves watertightness.
fn sample_corners(scene: &Scene, min: [f64; 3], size: f64) -> ([f64; 8], [bool; 8]) {
    let mut c = [0.0; 8];
    let mut ins = [false; 8];
    for i in 0..8 {
        let (dx, dy, dz) = corner_offset(i);
        let p = [
            min[0] + dx as f64 * size,
            min[1] + dy as f64 * size,
            min[2] + dz as f64 * size,
        ];
        let f = scene.primary(p);
        // Snap a degenerate graze (contact through the corner) to a fixed side so the
        // sign is build-independent. See `CONTACT_GRAZE_EPS`.
        c[i] = if f.abs() < CONTACT_GRAZE_EPS { CONTACT_GRAZE_EPS } else { f };
        ins[i] = scene.contributors_inside(p);
    }
    (c, ins)
}

/// Does any of the 12 cell edges change FIELD sign?
fn has_field_crossing(corners: &[f64; 8]) -> bool {
    CELL_EDGES
        .iter()
        .any(|&(a, b)| (corners[a] <= 0.0) != (corners[b] <= 0.0))
}

/// Does any of the 12 cell edges change BOUNDARY sign?
fn has_boundary_crossing(inside: &[bool; 8]) -> bool {
    CELL_EDGES.iter().any(|&(a, b)| inside[a] != inside[b])
}

/// Is this a surface leaf — does either surface cross it?
fn is_surface(corners: &[f64; 8], inside: &[bool; 8]) -> bool {
    has_field_crossing(corners) || has_boundary_crossing(inside)
}

/// Connected components of the cell's MINORITY-sign corners — one per surface sheet
/// passing through the cell (Manifold Dual Contouring, Schaefer/Ju/Warren 2007).
///
/// `sign[c]` is the contoured sign at corner `c` (`true` = inside the solid). We
/// take the rarer sign as the "minority" (ties → `true`), then group minority
/// corners that are connected through cube edges (`CELL_EDGES`). Each group is one
/// sheet: a cell with two disconnected minority groups is pierced by two sheets and
/// must emit two vertices (else the single-vertex rule pinches them non-manifold).
///
/// Returns `(comp, ncomp)`: `comp[c]` is the component id of corner `c` if it is a
/// minority corner, else `u8::MAX`; `ncomp` is the number of components. A surface-
/// crossing cell always has `ncomp >= 1`; the common case is exactly 1.
///
/// LATENT for smooth implicit geology. Empirically, CSG combinations of smooth
/// cokriging fields (erosion truncating a contact, faults offsetting a contact,
/// crossing contacts) produce a CREASE — one connected component per cell — which
/// the CSG-union contour already meshes watertight; they do NOT produce the two-
/// disconnected-sheets-in-one-cell case (`ncomp >= 2`). That case arises from
/// marching-cubes ambiguous faces and from sharp creases in an explicit INPUT mesh
/// boundary (e.g. a faceted fault / the test octahedron's edges). This split is
/// kept as the correct, unit-tested handling for those — but for the smooth-field
/// path it (correctly) never fires. The thin-grazing-layer case that DOES break is
/// sub-cell undersampling, addressed by refinement, not by this split.
fn minority_components(sign: &[bool; 8]) -> ([u8; 8], usize) {
    let inside_count = sign.iter().filter(|&&b| b).count();
    let minority = inside_count <= 4; // rarer sign; tie → inside

    // Cube-edge adjacency from CELL_EDGES.
    let mut adj = [[false; 8]; 8];
    for &(a, b) in CELL_EDGES.iter() {
        adj[a][b] = true;
        adj[b][a] = true;
    }

    let mut comp = [u8::MAX; 8];
    let mut ncomp = 0u8;
    for start in 0..8 {
        if sign[start] != minority || comp[start] != u8::MAX {
            continue;
        }
        // Flood-fill this component over cube edges.
        let mut stack = vec![start];
        comp[start] = ncomp;
        while let Some(c) = stack.pop() {
            for (n, &connected) in adj[c].iter().enumerate() {
                if connected && sign[n] == minority && comp[n] == u8::MAX {
                    comp[n] = ncomp;
                    stack.push(n);
                }
            }
        }
        ncomp += 1;
    }
    (comp, ncomp as usize)
}

/// The contoured sign at corner `c` of a leaf.
///
///   * Solid (`cap = true`): the combined CSG field `g = max(field − iso, b)`.
///     `g < 0` ⟺ `field < iso` (below the contact) AND inside the domain — i.e.
///     in the rock. The contoured surface is the boundary of that solid, and a
///     single sign field means its seam is just a crease (watertight by DC).
///   * Sheet (`cap = false`): the field sign alone (`field < iso`); the surface
///     is `field == iso`, later clipped to inside the domain at quad emission.
#[inline]
fn corner_sign(corners: &[f64; 8], inside: &[bool; 8], c: usize, cap: bool) -> bool {
    if cap {
        corners[c] <= 0.0 && inside[c]
    } else {
        corners[c] <= 0.0
    }
}

/// Does any of the cell's 12 edges carry two-or-more boundary crossings from
/// different surfaces (the two-sheets-per-edge case)? Such a cell is subdivided in
/// `build` so the single-vertex contour stays watertight.
fn cell_has_double_crossed_edge(scene: &Scene, min: [f64; 3], size: f64) -> bool {
    CELL_EDGES.iter().any(|&(a, b)| {
        scene.edge_double_crossed(corner_pos(min, size, a), corner_pos(min, size, b))
    })
}

/// Does the contoured sign change across this leaf (any of its 12 edges)?
fn has_contour_crossing(corners: &[f64; 8], inside: &[bool; 8], cap: bool) -> bool {
    CELL_EDGES
        .iter()
        .any(|&(a, b)| corner_sign(corners, inside, a, cap) != corner_sign(corners, inside, b, cap))
}

/// World position of corner `c` of the cell `(min, size)`.
fn corner_pos(min: [f64; 3], size: f64, c: usize) -> [f64; 3] {
    let (dx, dy, dz) = corner_offset(c);
    [
        min[0] + dx as f64 * size,
        min[1] + dy as f64 * size,
        min[2] + dz as f64 * size,
    ]
}

/// Joint Hermite crossings + QEF vertex for a cell. Gathers field-edge crossings
/// (point + field gradient normal) AND boundary-edge crossings (point + boundary
/// normal from `edge_crossing`), so a seam cell's single vertex is pinned to the
/// intersection of both surfaces. Returns `(vertex, error)` or `None` if neither
/// surface crosses.
fn cell_vertex(
    scene: &Scene,
    min: [f64; 3],
    size: f64,
    corners: &[f64; 8],
) -> Option<([f64; 3], f64)> {
    let mut crossings: Vec<([f64; 3], [f64; 3])> = Vec::new();
    for &(a, b) in CELL_EDGES.iter() {
        let pa = corner_pos(min, size, a);
        let pb = corner_pos(min, size, b);
        scene.edge_crossings(pa, pb, corners[a], corners[b], &mut crossings);
    }
    if crossings.is_empty() {
        return None;
    }
    let hi = [min[0] + size, min[1] + size, min[2] + size];
    Some(qef_vertex(&crossings, min, hi))
}

/// Per-component QEF vertices + the edge→component map for a cell, under the
/// contoured (combined) sign for this `cap`. Splits a two-sheet cell into one
/// vertex per sheet (Manifold DC); a single-sheet cell returns exactly one vertex,
/// identical to `cell_vertex`'s placement.
///
/// Returns `(verts, edge_owner)` where `verts[cid]` is the QEF vertex of component
/// `cid` and `edge_owner[e]` is the component owning crossing edge `e` (`u8::MAX`
/// if edge `e` does not change the contoured sign). `verts` is empty if the cell
/// has no contoured crossing.
fn cell_vertices(
    scene: &Scene,
    min: [f64; 3],
    size: f64,
    corners: &[f64; 8],
    inside: &[bool; 8],
    cap: bool,
) -> (Vec<[f64; 3]>, [u8; 12], Vec<VertKind>) {
    // Combined sign at the 8 corners, then its connected components.
    let mut sign = [false; 8];
    for c in 0..8 {
        sign[c] = corner_sign(corners, inside, c, cap);
    }
    let (comp, ncomp) = minority_components(&sign);
    let mut edge_owner = [u8::MAX; 12];
    if ncomp == 0 {
        return (Vec::new(), edge_owner, Vec::new());
    }

    // Gather Hermite crossings per component. The owner of a crossing edge is the
    // component of that edge's MINORITY-sign endpoint (a crossing edge has exactly
    // one endpoint of each sign, so exactly one is the minority).
    let inside_count = sign.iter().filter(|&&b| b).count();
    let minority = inside_count <= 4;
    let mut buckets: Vec<Vec<([f64; 3], [f64; 3])>> = vec![Vec::new(); ncomp];
    // Per component, two flags: does the PRIMARY CONTACT field cross an edge, and
    // does a CONTRIBUTOR (domain boundary) cross an edge? A PURE-contact component
    // (contact yes, boundary no) gets its vertex projected onto the contact — the
    // QEF can place it off-surface on a coarse cell spanning a steep ramp, which
    // terraces. A component touching the boundary (a cap or seam vertex) must NOT be
    // projected, or it would leave the domain wall and tear the flush closure.
    let mut has_contact = vec![false; ncomp];
    let mut has_boundary = vec![false; ncomp];

    for (ei, &(a, b)) in CELL_EDGES.iter().enumerate() {
        if sign[a] == sign[b] {
            continue; // contoured sign doesn't change here
        }
        let m = if sign[a] == minority { a } else { b };
        let cid = comp[m] as usize;
        edge_owner[ei] = comp[m];
        if (corners[a] <= 0.0) != (corners[b] <= 0.0) {
            has_contact[cid] = true;
        }
        if inside[a] != inside[b] {
            has_boundary[cid] = true;
        }

        let pa = corner_pos(min, size, a);
        let pb = corner_pos(min, size, b);
        // All surface crossings on this edge (primary contact + every contributor)
        // pin this component's QEF — to the contact, a contributor surface, or
        // their crease.
        scene.edge_crossings(pa, pb, corners[a], corners[b], &mut buckets[cid]);
    }

    let hi = [min[0] + size, min[1] + size, min[2] + size];
    let center = [min[0] + size * 0.5, min[1] + size * 0.5, min[2] + size * 0.5];
    let verts: Vec<[f64; 3]> = buckets
        .iter()
        .map(|cr| {
            if cr.is_empty() {
                center
            } else {
                qef_vertex(cr, min, hi).0
            }
        })
        .collect();
    // Project only pure-contact components (contact crossing, no boundary crossing).
    // Per-component projection kind: `Cap` (boundary only — leave it), `Interior`
    // (contact only — project freely onto the contact), `Seam` (contact AND boundary
    // — project onto the contact CONSTRAINED to the boundary surface, so it smooths
    // without leaving the wall).
    let kind: Vec<VertKind> = (0..ncomp)
        .map(|c| match (has_contact[c], has_boundary[c]) {
            (true, false) => VertKind::Interior,
            (true, true) => VertKind::Seam,
            _ => VertKind::Cap,
        })
        .collect();
    (verts, edge_owner, kind)
}

/// How a component's vertex relates to the surfaces, deciding how it may be moved
/// onto the contact for smoothing (see `project_to_contact`).
#[derive(Clone, Copy, PartialEq)]
enum VertKind {
    /// Boundary only (a domain-cap vertex) — leave it on the boundary.
    Cap,
    /// Contact only — project freely onto the contact.
    Interior,
    /// Contact AND boundary (a seam vertex) — project onto the contact CONSTRAINED
    /// to the boundary surface (stays flush on the wall).
    Seam,
}

/// How far past a cell face the projection's dominant-axis coordinate may travel,
/// in cell sizes. The contour TOPOLOGY (4-leaves-per-edge, edge ownership, winding)
/// is fixed by `corner_sign` on the cell's unmoved corners, so a vertex leaving its
/// cell along the surface-normal direction creates no crack — proven by the
/// watertight/flush/winding tests staying green with the dominant axis unclamped.
/// This bound is only a runaway rail for an ill-conditioned Newton step on a coarse
/// cell; a contact this resolution genuinely rises <2 cells per cell, so `2.0` never
/// binds in practice.
const PROJECT_OOB_CELLS: f64 = 2.0;

/// Project a contact vertex onto the contact (`field == iso`) with a few Newton
/// steps along the field gradient.
///
/// The QEF places a vertex at the joint minimizer, which on a COARSE cell straddling
/// a ramp lands off the contact — and the naive fix of clamping the projected vertex
/// to its own cell `[min, min+size]³` *re-quantizes* it: the true contact point can
/// require a coordinate OUTSIDE the cell (a ramp rising >1 cell of height across the
/// cell), so the clamp pins the vertex to the cell face — one terraced step per cell.
///
/// Instead we let the vertex leave its cell ALONG the gradient's dominant axis (the
/// direction Newton actually steps — the contact's facing direction), while clamping
/// the two OFF-axis coordinates to the cell. The contact is single-valued along its
/// normal locally, so the dominant-axis coordinate is the one that must be free to
/// reach the surface; the off-axis clamp keeps the lateral vertex lattice intact.
/// Travel past a face is capped at [`PROJECT_OOB_CELLS`] cells as a runaway rail.
///
/// For a `Seam` vertex the field gradient is first projected into the boundary's
/// tangent plane (the `boundary.surface_normal` component removed), so the dominant
/// axis is necessarily IN the wall's tangent plane — the vertex slides ALONG the
/// boundary onto the contact without leaving it, preserving the flush domain closure.
fn project_to_contact(
    scene: &Scene,
    v: [f64; 3],
    min: [f64; 3],
    size: f64,
    kind: VertKind,
) -> [f64; 3] {
    let lo = min;
    let hi = [min[0] + size, min[1] + size, min[2] + size];
    // Out-of-bounds slack the dominant axis may use past either cell face.
    let slack = size * PROJECT_OOB_CELLS;
    let mut p = v;
    for _ in 0..6 {
        let f = scene.primary(p);
        let mut g = scene.primary_gradient(p);
        // Seam: remove the boundary-normal component so the step stays on the wall.
        if kind == VertKind::Seam {
            if let Some(n) = scene.seam_boundary_normal(p) {
                let dot = g[0] * n[0] + g[1] * n[1] + g[2] * n[2];
                g = [g[0] - dot * n[0], g[1] - dot * n[1], g[2] - dot * n[2]];
            }
        }
        let g2 = g[0] * g[0] + g[1] * g[1] + g[2] * g[2];
        if g2 < 1e-12 {
            break;
        }
        // Dominant axis of the (seam-adjusted) gradient — the surface-normal
        // direction along which the contact is locally single-valued, and the only
        // axis allowed to leave the cell.
        let ax = (0..3)
            .max_by(|&a, &b| g[a].abs().partial_cmp(&g[b].abs()).unwrap())
            .unwrap();
        let s = f / g2;
        let np = [p[0] - s * g[0], p[1] - s * g[1], p[2] - s * g[2]];
        p = [
            np[0].clamp(lo[0], hi[0]),
            np[1].clamp(lo[1], hi[1]),
            np[2].clamp(lo[2], hi[2]),
        ];
        // Re-allow the dominant axis to leave the cell, up to the runaway rail.
        p[ax] = np[ax].clamp(lo[ax] - slack, hi[ax] + slack);
    }
    p
}

/// Recursively build the octree. A cell becomes a leaf when: it is at max depth,
/// or (past min depth) neither surface crosses it, or its single-vertex joint-QEF
/// error is under threshold.
fn build(
    tree: &mut Octree,
    scene: &Scene,
    min: [f64; 3],
    size: f64,
    depth: u32,
    cfg: &OctreeConfig,
) -> NodeId {
    let (corners, inside) = sample_corners(scene, min, size);
    let crossing = is_surface(&corners, &inside);

    let make_leaf = |tree: &mut Octree| -> NodeId {
        let id = tree.nodes.len();
        // Per-component vertices are computed later in `assign_vertices` (which knows
        // `cap` and so the combined sign that defines the components).
        tree.nodes.push(Node::Leaf(Leaf {
            corners,
            inside,
            out: Vec::new(),
            edge_owner: [u8::MAX; 12],
        }));
        tree.bounds.push((min, size));
        id
    };

    if depth >= cfg.max_depth {
        return make_leaf(tree);
    }
    if depth >= cfg.min_depth {
        // Below the forced floor: stop where there's no detail on either surface.
        if !crossing {
            return make_leaf(tree);
        }
        if let Some((_, err)) = cell_vertex(scene, min, size, &corners) {
            if err <= cfg.qef_threshold {
                return make_leaf(tree);
            }
        }
    }

    // Subdivide into 8 children.
    let half = size * 0.5;
    let mut children: [Option<NodeId>; 8] = [None; 8];
    for c in 0..8 {
        let (dx, dy, dz) = corner_offset(c);
        let cmin = [
            min[0] + dx as f64 * half,
            min[1] + dy as f64 * half,
            min[2] + dz as f64 * half,
        ];
        children[c] = Some(build(tree, scene, cmin, half, depth + 1, cfg));
    }
    let id = tree.nodes.len();
    tree.nodes.push(Node::Internal(Box::new(children)));
    tree.bounds.push((min, size));
    id
}

/// 2:1 balance: ensure no surface leaf is more than one level coarser than a leaf
/// touching it, so each minimal surface edge is shared by exactly 4 leaves.
/// Implemented as a fixpoint: repeatedly subdivide offending leaves until stable.
fn balance(tree: &mut Octree, scene: &Scene, cfg: &OctreeConfig) {
    loop {
        let mut to_split: Vec<NodeId> = Vec::new();
        let mut leaf_depth: std::collections::HashMap<NodeId, u32> =
            std::collections::HashMap::new();
        collect_leaf_depths(tree, tree.root, 0, &mut leaf_depth);

        for (&leaf_id, &d) in leaf_depth.iter() {
            let (min, size) = tree.bounds[leaf_id];
            // Only surface leaves (field OR boundary crossing) matter for cracks.
            if let Node::Leaf(l) = &tree.nodes[leaf_id] {
                if !is_surface(&l.corners, &l.inside) {
                    continue;
                }
            }
            let eps = size * 1e-3;
            let face_pts = [
                [min[0] - eps, min[1] + size * 0.5, min[2] + size * 0.5],
                [min[0] + size + eps, min[1] + size * 0.5, min[2] + size * 0.5],
                [min[0] + size * 0.5, min[1] - eps, min[2] + size * 0.5],
                [min[0] + size * 0.5, min[1] + size + eps, min[2] + size * 0.5],
                [min[0] + size * 0.5, min[1] + size * 0.5, min[2] - eps],
                [min[0] + size * 0.5, min[1] + size * 0.5, min[2] + size + eps],
            ];
            for fp in face_pts {
                if let Some(nd) = leaf_depth_at(tree, tree.root, 0, fp) {
                    if nd >= d + 2 {
                        to_split.push(leaf_id);
                        break;
                    }
                }
            }
        }

        if to_split.is_empty() {
            break;
        }
        to_split.sort_unstable();
        to_split.dedup();
        for leaf_id in to_split {
            split_leaf(tree, scene, leaf_id, leaf_depth[&leaf_id], cfg);
        }
    }
}

fn collect_leaf_depths(
    tree: &Octree,
    id: NodeId,
    depth: u32,
    out: &mut std::collections::HashMap<NodeId, u32>,
) {
    match &tree.nodes[id] {
        Node::Leaf(_) => {
            out.insert(id, depth);
        }
        Node::Internal(ch) => {
            for c in ch.iter().flatten() {
                collect_leaf_depths(tree, *c, depth + 1, out);
            }
        }
    }
}

/// Depth of the leaf containing point `p`, if inside the tree.
fn leaf_depth_at(tree: &Octree, id: NodeId, depth: u32, p: [f64; 3]) -> Option<u32> {
    let (min, size) = tree.bounds[id];
    if p[0] < min[0] || p[1] < min[1] || p[2] < min[2] {
        return None;
    }
    if p[0] > min[0] + size || p[1] > min[1] + size || p[2] > min[2] + size {
        return None;
    }
    match &tree.nodes[id] {
        Node::Leaf(_) => Some(depth),
        Node::Internal(ch) => {
            for c in ch.iter().flatten() {
                if let Some(d) = leaf_depth_at(tree, *c, depth + 1, p) {
                    return Some(d);
                }
            }
            None
        }
    }
}

/// Replace a leaf with an internal node of 8 freshly-built children one level
/// finer (used by balancing).
fn split_leaf(tree: &mut Octree, scene: &Scene, leaf_id: NodeId, depth: u32, cfg: &OctreeConfig) {
    let (min, size) = tree.bounds[leaf_id];
    let half = size * 0.5;
    let mut children: [Option<NodeId>; 8] = [None; 8];
    for c in 0..8 {
        let (dx, dy, dz) = corner_offset(c);
        let cmin = [
            min[0] + dx as f64 * half,
            min[1] + dy as f64 * half,
            min[2] + dz as f64 * half,
        ];
        children[c] = Some(build(tree, scene, cmin, half, depth + 1, cfg));
    }
    tree.nodes[leaf_id] = Node::Internal(Box::new(children));
}

/// Subdivide, to a fixpoint, every leaf below `max_depth` that has a double-crossed
/// edge (two surfaces of the scene crossing one edge → two boundary sheets through
/// the cell). Splitting these makes each edge single-crossed so the single-vertex
/// minimal-edge contour stays watertight. A genuine grazing/tangent intersection
/// can stay double-crossed at `max_depth`; that residual is capped, not infinite.
fn refine_double_crossed(tree: &mut Octree, scene: &Scene, cfg: &OctreeConfig) {
    loop {
        let mut depths: std::collections::HashMap<NodeId, u32> =
            std::collections::HashMap::new();
        collect_leaf_depths(tree, tree.root, 0, &mut depths);

        let mut to_split: Vec<(NodeId, u32)> = Vec::new();
        for (&id, &d) in &depths {
            if d >= cfg.max_depth {
                continue;
            }
            let (min, size) = tree.bounds[id];
            if cell_has_double_crossed_edge(scene, min, size) {
                to_split.push((id, d));
            }
        }
        if to_split.is_empty() {
            break;
        }
        // Stable order so the arena indices we split are deterministic.
        to_split.sort_unstable();
        for (id, d) in to_split {
            // `id` is still a leaf (we only split leaves and split_leaf replaces in
            // place; new children are appended, not renumbering `id`).
            if matches!(tree.nodes[id], Node::Leaf(_)) {
                split_leaf(tree, scene, id, d, cfg);
            }
        }
    }
}

/// Compute and emit each surface leaf's per-component QEF vertices, recording their
/// output mesh indices in `leaf.out` and the edge→component map in `leaf.edge_owner`
/// (used by quad emission to route a shared edge to the owning component's vertex).
/// A single-sheet cell gets exactly one vertex (identical to the old single-vertex
/// behavior); a two-sheet cell gets one per sheet (Manifold DC).
fn assign_vertices(tree: &mut Octree, scene: &Scene, mesh: &mut Mesh, cap: bool) {
    for id in 0..tree.nodes.len() {
        let (min, size) = tree.bounds[id];
        let (corners, inside) = match &tree.nodes[id] {
            Node::Leaf(l) if has_contour_crossing(&l.corners, &l.inside, cap) => {
                (l.corners, l.inside)
            }
            _ => continue,
        };
        let (verts, edge_owner, kinds) = cell_vertices(scene, min, size, &corners, &inside, cap);
        let mut out = Vec::with_capacity(verts.len());
        for (ci, v) in verts.iter().enumerate() {
            // Project contact vertices onto the contact (smoothing the terraced steps
            // from coarse-cell QEF placement). Interior vertices project freely; seam
            // vertices project CONSTRAINED to the boundary (stay flush on the wall);
            // cap-only vertices are left untouched. See `project_to_contact`.
            let kind = kinds.get(ci).copied().unwrap_or(VertKind::Cap);
            let p = match kind {
                VertKind::Cap => *v,
                _ => project_to_contact(scene, *v, min, size, kind),
            };
            let idx = (mesh.positions.len() / 3) as u32;
            mesh.positions.push(p[0] as f32);
            mesh.positions.push(p[1] as f32);
            mesh.positions.push(p[2] as f32);
            // Exact per-vertex normal from the analytic field gradient (the vertex
            // lies on the contact). Smooth shading without face averaging.
            let g = normalize(scene.primary_gradient(p));
            mesh.normals.push(g[0] as f32);
            mesh.normals.push(g[1] as f32);
            mesh.normals.push(g[2] as f32);
            out.push(idx);
        }
        if let Node::Leaf(l) = &mut tree.nodes[id] {
            l.out = out;
            l.edge_owner = edge_owner;
        }
    }
}

// ---------------------------------------------------------------------------
// Crack-free contouring by minimal-edge iteration.
//
// With a 2:1-balanced octree, every minimal edge that crosses the contoured sign
// field is shared by exactly 4 leaves; we locate them by point sampling the 4
// quadrant centers around the edge and emit one quad. One pass over the combined
// sign (`corner_sign`) handles both the solid (`g = max(field−iso, b)`) and the
// open sheet (`field`), so there is no field/cap split to reconcile.
// ---------------------------------------------------------------------------

/// Find the leaf node containing point `p` (or None if outside the tree).
fn leaf_at(tree: &Octree, id: NodeId, p: [f64; 3]) -> Option<NodeId> {
    let (min, size) = tree.bounds[id];
    if p[0] < min[0] || p[1] < min[1] || p[2] < min[2] {
        return None;
    }
    if p[0] > min[0] + size || p[1] > min[1] + size || p[2] > min[2] + size {
        return None;
    }
    match &tree.nodes[id] {
        Node::Leaf(_) => Some(id),
        Node::Internal(ch) => {
            for c in ch.iter().flatten() {
                if let Some(l) = leaf_at(tree, *c, p) {
                    return Some(l);
                }
            }
            None
        }
    }
}

/// Quantize a world coordinate to an integer grid at the finest cell size, for
/// stable edge-deduplication keys.
fn qkey(v: f64, finest: f64) -> i64 {
    (v / finest).round() as i64
}

fn finest_leaf_size(tree: &Octree) -> Option<f64> {
    let mut finest = f64::INFINITY;
    for id in 0..tree.nodes.len() {
        if matches!(tree.nodes[id], Node::Leaf(_)) {
            finest = finest.min(tree.bounds[id].1);
        }
    }
    finest.is_finite().then_some(finest)
}

/// Center point offset from edge midpoint `mid` by `du,dv` along in-plane axes.
fn quad_center(mid: [f64; 3], u: usize, v: usize, du: f64, dv: f64) -> [f64; 3] {
    let mut p = mid;
    p[u] += du;
    p[v] += dv;
    p
}

/// The 4 quadrant leaves around a minimal edge whose midpoint is `mid` on `axis`.
/// Returns `(leaves, ok, this_is_finest)`: `ok` is false if any quadrant is
/// outside the tree; `this_is_finest` is false if a quadrant is a finer leaf
/// (so a finer owner will process this edge instead).
fn edge_quad_leaves(
    tree: &Octree,
    mid: [f64; 3],
    axis: usize,
    lsize: f64,
    finest: f64,
) -> ([NodeId; 4], bool, bool) {
    let (u, v) = in_plane_axes(axis);
    let off = finest * 0.5;
    let quad_centers = [
        quad_center(mid, u, v, -off, -off),
        quad_center(mid, u, v, off, -off),
        quad_center(mid, u, v, off, off),
        quad_center(mid, u, v, -off, off),
    ];
    let mut leaves = [usize::MAX; 4];
    let mut ok = true;
    let mut this_is_finest = true;
    for (qi, qc) in quad_centers.iter().enumerate() {
        match leaf_at(tree, tree.root, *qc) {
            Some(lf) => {
                leaves[qi] = lf;
                if tree.bounds[lf].1 < lsize - 1e-9 {
                    this_is_finest = false;
                }
            }
            None => {
                ok = false;
                break;
            }
        }
    }
    (leaves, ok, this_is_finest)
}

fn in_plane_axes(axis: usize) -> (usize, usize) {
    match axis {
        0 => (1, 2),
        1 => (0, 2),
        _ => (0, 1),
    }
}

/// Map an edge's corner-pair XOR to its axis (None if not an axis-aligned edge).
fn edge_axis(ca: usize, cb: usize) -> Option<usize> {
    match ca ^ cb {
        1 => Some(0),
        2 => Some(1),
        4 => Some(2),
        _ => None,
    }
}

/// Contour the combined-solid sign field (`g`) by minimal-edge iteration: one quad
/// per minimal edge across which the sign changes, whose 4 quadrant leaves all
/// exist (interior edge). Meshes the watertight closed solid. Each emitted triangle
/// is tagged in `contact_face`: `true` if the PRIMARY contact field crosses this
/// minimal edge (a contact-surface face), `false` if only a contributor surface
/// crosses it (a domain-cap face). Winding is provisional — orientation fixes it.
fn contour_edges(tree: &Octree, scene: &Scene, mesh: &mut Mesh, contact_face: &mut Vec<bool>) {
    let Some(finest) = finest_leaf_size(tree) else {
        return;
    };
    let mut seen: std::collections::HashSet<(usize, i64, i64, i64)> =
        std::collections::HashSet::new();

    for id in 0..tree.nodes.len() {
        let (lmin, lsize) = tree.bounds[id];
        let (corners, inside) = match &tree.nodes[id] {
            Node::Leaf(l) if has_contour_crossing(&l.corners, &l.inside, true) => {
                (l.corners, l.inside)
            }
            _ => continue,
        };

        for &(ca, cb) in CELL_EDGES.iter() {
            if corner_sign(&corners, &inside, ca, true) == corner_sign(&corners, &inside, cb, true) {
                continue;
            }
            let Some(axis) = edge_axis(ca, cb) else {
                continue;
            };
            let p0 = corner_pos(lmin, lsize, ca);
            let mut mid = p0;
            mid[axis] += lsize * 0.5;
            let key = (
                axis,
                qkey(mid[0], finest),
                qkey(mid[1], finest),
                qkey(mid[2], finest),
            );

            let (leaves, ok, this_is_finest) = edge_quad_leaves(tree, mid, axis, lsize, finest);
            if !ok || !this_is_finest || !seen.insert(key) {
                continue;
            }

            // A contact face: the primary contact field crosses this minimal edge AND the
            // PRIMARY CROSSING POINT is on the un-eroded side of the TRUNCATING contributors.
            // Deciding at the crossing point (not the two corners) makes the open sheet
            // terminate EXACTLY on the contact∩truncator crease: the crossing-point's
            // truncator membership flips precisely at the crease, so the last kept face is
            // the one whose primary crossing sits on the crease — neither undershooting (a
            // corner-flag `&&` stops a cell short) nor overshooting (`||` keeps a cell too
            // many). The domain box is excluded from the truncator test, so box-daylight
            // faces stay flush. A combined-sign flip with NO primary crossing is a cap face.
            let primary_crosses = (corners[ca] <= 0.0) != (corners[cb] <= 0.0);
            let is_contact = primary_crosses && {
                let pa = corner_pos(lmin, lsize, ca);
                let pb = corner_pos(lmin, lsize, cb);
                let (va, vb) = (corners[ca], corners[cb]);
                let t = va / (va - vb); // primary crossing parameter (va != vb when crossing)
                let cross = [
                    pa[0] + (pb[0] - pa[0]) * t,
                    pa[1] + (pb[1] - pa[1]) * t,
                    pa[2] + (pb[2] - pa[2]) * t,
                ];
                scene.contributors_inside_truncators(cross)
            };
            emit_quad(tree, &leaves, axis, mid, mesh, is_contact, contact_face);
        }
    }
}

/// Which of a leaf's 12 edges is the axis-parallel edge passing through world point
/// `mid` (the shared minimal edge's midpoint)? Identified by the two in-plane
/// (perpendicular) coordinates: each names whether the edge sits at the leaf's low
/// or high side in that axis. Returns the `CELL_EDGES` index, or None if `mid` is
/// off this leaf's edge lattice (shouldn't happen for a true shared edge).
fn leaf_local_edge(lmin: [f64; 3], lsize: f64, axis: usize, mid: [f64; 3]) -> Option<usize> {
    let (u, v) = in_plane_axes(axis);
    let bit = |coord: f64, lo: f64| -> Option<usize> {
        let tol = lsize * 1e-3;
        if (coord - lo).abs() < tol {
            Some(0)
        } else if (coord - (lo + lsize)).abs() < tol {
            Some(1)
        } else {
            None
        }
    };
    let ub = bit(mid[u], lmin[u])?;
    let vb = bit(mid[v], lmin[v])?;
    // The edge's lower corner: axis bit 0, with u/v bits set. Find the CELL_EDGES
    // entry along `axis` whose lower corner has those u,v bits.
    let target_lo = (ub << u) | (vb << v); // axis bit is 0 (lower endpoint)
    for (ei, &(a, b)) in CELL_EDGES.iter().enumerate() {
        if edge_axis(a, b) != Some(axis) {
            continue;
        }
        let lo_c = if a < b { a } else { b }; // lower endpoint along axis (axis bit 0)
        if lo_c == target_lo {
            return Some(ei);
        }
    }
    None
}

/// The 4 shared vertices of a quad's leaves, routed to the COMPONENT that owns the
/// shared edge in each leaf (Manifold DC). For a single-component leaf this is just
/// its one vertex. None if any leaf lacks a vertex for that edge.
fn quad_vertices(
    tree: &Octree,
    leaves: &[NodeId; 4],
    axis: usize,
    mid: [f64; 3],
) -> Option<[u32; 4]> {
    let mut vtx = [0u32; 4];
    for qi in 0..4 {
        let (lmin, lsize) = tree.bounds[leaves[qi]];
        let l = match &tree.nodes[leaves[qi]] {
            Node::Leaf(l) => l,
            _ => return None,
        };
        if l.out.is_empty() {
            return None;
        }
        // Single-component leaf (the common case, incl. all coarse neighbors at a
        // 2:1 transition): one vertex, no routing needed — use it. This also avoids
        // edge-lattice lookup on a coarse leaf where the fine `mid` falls mid-face.
        if l.out.len() == 1 {
            vtx[qi] = l.out[0];
            continue;
        }
        // Multi-component leaf: route the shared edge to its owning component.
        let ei = leaf_local_edge(lmin, lsize, axis, mid)?;
        let owner = l.edge_owner[ei];
        if owner == u8::MAX {
            return None;
        }
        vtx[qi] = *l.out.get(owner as usize)?;
    }
    Some(vtx)
}

/// Emit the quad (two triangles) from the 4 quadrant leaves' per-component vertices.
fn emit_quad(
    tree: &Octree,
    leaves: &[NodeId; 4],
    axis: usize,
    mid: [f64; 3],
    mesh: &mut Mesh,
    is_contact: bool,
    contact_face: &mut Vec<bool>,
) {
    let Some(vtx) = quad_vertices(tree, leaves, axis, mid) else {
        return;
    };
    for tri in [[vtx[0], vtx[1], vtx[2]], [vtx[0], vtx[2], vtx[3]]] {
        if tri[0] != tri[1] && tri[1] != tri[2] && tri[0] != tri[2] {
            mesh.indices.extend_from_slice(&tri);
            contact_face.push(is_contact);
        }
    }
}

/// Keep only the contact-surface faces (drop domain-cap faces), turning the closed
/// solid into the bounded open contact sheet. `contact_face[t]` tags triangle `t`.
fn keep_contact_faces(mesh: &mut Mesh, contact_face: &[bool]) {
    let mut kept: Vec<u32> = Vec::new();
    for (t, &is_contact) in contact_face.iter().enumerate() {
        if is_contact {
            kept.extend_from_slice(&mesh.indices[3 * t..3 * t + 3]);
        }
    }
    mesh.indices = kept;
}

// Builds an in-process kernel `Model`, so native-only (the wasm engine has no kernel).
#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;
    use crate::assemble::KernelOptions;
    use crate::data::{Orientation, Series, SurfacePoint};
    use crate::Model;

    fn flat_model() -> Model {
        let sps = vec![
            SurfacePoint::new([0.0, 0.0, 1.0], 0),
            SurfacePoint::new([1.0, 0.0, 1.0], 0),
            SurfacePoint::new([0.0, 1.0, 1.0], 0),
            SurfacePoint::new([1.0, 1.0, 1.0], 0),
        ];
        let oris = vec![Orientation::new([0.5, 0.5, 1.0], [0.0, 0.0, 1.0])];
        Model::fit(&Series::new(sps, oris), KernelOptions::default())
    }

    #[test]
    fn octree_produces_a_sheet_near_z1() {
        let model = flat_model();
        // Open-sheet view (no cap): just the contact surface, so footprint
        // vertices sit near the contact at z=1.
        let cfg = OctreeConfig {
            max_depth: 5,
            min_depth: 2,
            qef_threshold: 1e-5,
            cap: false,
        };
        let mesh = octree_dual_contour(&model, [-0.5, -0.5, 0.0], 2.0, 0.0, cfg);
        assert!(mesh.vertex_count() > 0, "no vertices");
        assert!(mesh.triangle_count() > 0, "no triangles");

        let vmax = mesh.vertex_count() as u32;
        assert!(mesh.indices.iter().all(|&i| i < vmax), "dangling index");

        // REFERENCED vertices within the data footprint should sit near z=1. (The
        // sheet is derived from the closed solid by keeping contact faces, so the
        // solid's now-orphaned cap-vertex positions remain in `positions` but aren't
        // referenced — count only referenced ones.)
        let referenced: std::collections::HashSet<u32> = mesh.indices.iter().copied().collect();
        let near: Vec<f64> = referenced
            .iter()
            .map(|&i| {
                let b = (i as usize) * 3;
                [
                    mesh.positions[b] as f64,
                    mesh.positions[b + 1] as f64,
                    mesh.positions[b + 2] as f64,
                ]
            })
            .filter(|p| p[0] >= 0.0 && p[0] <= 1.0 && p[1] >= 0.0 && p[1] <= 1.0)
            .map(|p| p[2])
            .collect();
        assert!(!near.is_empty(), "no vertices in data footprint");
        let mean = near.iter().sum::<f64>() / near.len() as f64;
        assert!((mean - 1.0).abs() < 0.2, "sheet not near z=1: mean={mean}");
    }

    #[test]
    fn empty_when_no_crossing() {
        let model = flat_model();
        // A box entirely above the contact (z in [1.5, 2.0]) has no field
        // crossing; the box boundary alone produces no cap inside the rock here.
        let mesh = octree_dual_contour(&model, [0.0, 0.0, 1.5], 0.5, 0.0, OctreeConfig::default());
        assert_eq!(mesh.triangle_count(), 0);
    }

    /// The closed solid (`cap = true`) is STRICTLY watertight for a planar contact
    /// meeting the box — every edge has incidence exactly 2: zero non-manifold,
    /// zero open edges, anywhere on the mesh including the box faces.
    ///
    /// This is the gempy model1 case the web harness renders (a flat contact at
    /// z=600 in a [0,1000]³ box). The incidence-aware cap dedup (drop a cap
    /// triangle only on an INTERIOR field edge, pair on a field BOUNDARY/seam edge)
    /// closes the field/cap seam that the earlier "drop on any shared edge" rule
    /// left open. Asserted at several depths since the seam config is resolution-
    /// dependent.
    ///
    /// (A CURVED contact meeting the box walls still leaves seam open edges — see
    /// the note in `curved_surface_is_crack_free_wound_and_oriented`. That is the
    /// remaining manifold-DC residual; this test guards the flat case it solved.)
    #[test]
    fn flat_contact_closed_solid_is_watertight() {
        use std::collections::HashMap;
        // gempy model1: a flat contact at z=600 in a [0,1000]³ box.
        let mut sps = Vec::new();
        for &(x, y) in &[
            (100.0, 200.0),
            (500.0, 200.0),
            (900.0, 200.0),
            (100.0, 800.0),
            (500.0, 800.0),
            (900.0, 800.0),
        ] {
            sps.push(SurfacePoint::new([x, y, 600.0], 0));
        }
        let oris = vec![Orientation::new([500.0, 500.0, 600.0], [0.0, 0.0, 1.0])];
        let model = Model::fit(&Series::new(sps, oris), KernelOptions::default());
        let iso = model.lithology().interfaces[0].isovalue;

        for depth in [3, 4, 5, 6] {
            let cfg = OctreeConfig {
                max_depth: depth,
                min_depth: 2,
                qef_threshold: 1e-6,
                cap: true,
            };
            let mesh = octree_dual_contour(&model, [0.0, 0.0, 0.0], 1000.0, iso, cfg);
            assert!(mesh.triangle_count() > 0, "depth {depth}: empty mesh");

            let mut undirected: HashMap<(u32, u32), u32> = HashMap::new();
            for t in mesh.indices.chunks_exact(3) {
                for &(a, b) in &[(t[0], t[1]), (t[1], t[2]), (t[2], t[0])] {
                    let e = if a < b { (a, b) } else { (b, a) };
                    *undirected.entry(e).or_insert(0) += 1;
                }
            }
            // Watertight closed solid: EVERY edge incidence is exactly 2.
            let nonmanifold = undirected.values().filter(|&&c| c > 2).count();
            let open = undirected.values().filter(|&&c| c == 1).count();
            assert_eq!(nonmanifold, 0, "depth {depth}: non-manifold edges {nonmanifold}");
            assert_eq!(open, 0, "depth {depth}: open edges {open}");
        }
    }

    #[test]
    fn minority_components_groups_sheets() {
        // No minority corners (all same sign) → 0 components.
        let (_c, n) = minority_components(&[false; 8]);
        assert_eq!(n, 0, "uniform-outside cell has no components");
        let (_c, n) = minority_components(&[true; 8]);
        assert_eq!(n, 0, "uniform-inside cell has no components");

        // One inside corner → one component (a single sheet cutting off a corner).
        let mut s = [false; 8];
        s[0] = true;
        let (comp, n) = minority_components(&s);
        assert_eq!(n, 1);
        assert_eq!(comp[0], 0);
        assert_eq!(comp[1], u8::MAX, "majority corner has no component");

        // One face inside (corners 0,1,2,3 = z==0 face) → one connected component
        // (a planar sheet) — the common single-sheet case.
        let mut s = [false; 8];
        for c in [0, 1, 2, 3] {
            s[c] = true;
        }
        let (_comp, n) = minority_components(&s);
        assert_eq!(n, 1, "a flat sheet is one component");

        // TWO opposite corners inside (0 and 7, diagonal) → TWO components: they are
        // not cube-edge adjacent, so two separate sheets pass through the cell. This
        // is the two-sheets-in-one-cell case that needs two vertices.
        let mut s = [false; 8];
        s[0] = true;
        s[7] = true;
        let (comp, n) = minority_components(&s);
        assert_eq!(n, 2, "two diagonal corners are two sheets");
        assert_ne!(comp[0], comp[7], "diagonal corners in different components");
    }

    /// Regression for the missing-triangle / winding bugs: on a CURVED surface
    /// that forces refinement (so coarse/fine transitions exist), the mesh must
    /// be (a) crack-free, (b) consistently wound, and (c) watertight where it
    /// meets the domain box — the field surface and the boundary cap share seam
    /// vertices, leaving zero interior holes.
    #[test]
    fn curved_surface_is_crack_free_wound_and_oriented() {
        use std::collections::HashMap;
        // A paraboloid bowl interface — strong curvature → adaptive refinement.
        let mut sps = Vec::new();
        for i in 0..5 {
            for j in 0..5 {
                let x = i as f64 * 250.0;
                let y = j as f64 * 250.0;
                let z = 500.0 + 0.0008 * ((x - 500.0).powi(2) + (y - 500.0).powi(2));
                sps.push(SurfacePoint::new([x, y, z], 0));
            }
        }
        let oris = vec![Orientation::new([500.0, 500.0, 500.0], [0.0, 0.0, 1.0])];
        let model = Model::fit(&Series::new(sps, oris), KernelOptions::default());

        let cfg = OctreeConfig {
            max_depth: 5,
            min_depth: 2,
            qef_threshold: 1e-6,
            cap: true,
        };
        let mesh = octree_dual_contour(&model, [0.0, 0.0, 0.0], 1000.0, 0.0, cfg);
        assert!(mesh.triangle_count() > 100, "expected a refined mesh");

        // Boundary closure: the surface must reach the domain faces — some
        // vertices lie ON the box (x or y at 0 or 1000), and none escape it.
        let posb = |i: usize| {
            [
                mesh.positions[3 * i] as f64,
                mesh.positions[3 * i + 1] as f64,
            ]
        };
        let nverts = mesh.positions.len() / 3;
        let mut on_face = 0;
        for i in 0..nverts {
            let [x, y] = posb(i);
            assert!(
                x >= -1e-6 && x <= 1000.0 + 1e-6 && y >= -1e-6 && y <= 1000.0 + 1e-6,
                "vertex {x},{y} escaped the domain box"
            );
            if x.abs() < 1.0 || (x - 1000.0).abs() < 1.0 || y.abs() < 1.0 || (y - 1000.0).abs() < 1.0
            {
                on_face += 1;
            }
        }
        assert!(
            on_face > 0,
            "surface did not reach any domain face (boundary not closed)"
        );

        // Edge stats.
        let mut dir: HashMap<(u32, u32), i32> = HashMap::new();
        let mut undirected: HashMap<(u32, u32), u32> = HashMap::new();
        for t in mesh.indices.chunks_exact(3) {
            for &(a, b) in &[(t[0], t[1]), (t[1], t[2]), (t[2], t[0])] {
                *dir.entry((a, b)).or_insert(0) += 1;
                let e = if a < b { (a, b) } else { (b, a) };
                *undirected.entry(e).or_insert(0) += 1;
            }
        }
        // STRICTLY watertight closed solid: every edge incidence is exactly 2 —
        // zero non-manifold (>2), zero open (1). Contouring the single combined
        // field `g = max(field − iso, b)` makes the CURVED contact-meets-box seam
        // a plain crease, so it closes watertight (no field/cap seam to reconcile).
        let nonmanifold = undirected.values().filter(|&&c| c > 2).count();
        let open = undirected.values().filter(|&&c| c == 1).count();
        let total = undirected.len();
        assert_eq!(nonmanifold, 0, "non-manifold edges: {nonmanifold}/{total}");
        assert_eq!(open, 0, "open edges (not watertight): {open}/{total}");

        let _ = &dir;
    }

    /// An outward-wound octahedron of "radius" `r` about `center` — a coarse
    /// closed polyhedron with 6 vertices and 8 tilted (non-axis-aligned) faces,
    /// for exercising the `TriMesh` boundary. Each face is wound CCW as seen from
    /// outside, so winding number is +1 inside and face normals point outward.
    fn octahedron(center: [f64; 3], r: f64) -> TriMesh {
        let [cx, cy, cz] = center;
        // 0:+x 1:-x 2:+y 3:-y 4:+z 5:-z
        let verts = vec![
            [cx + r, cy, cz],
            [cx - r, cy, cz],
            [cx, cy + r, cz],
            [cx, cy - r, cz],
            [cx, cy, cz + r],
            [cx, cy, cz + 0.0 - r],
        ];
        // 8 faces, each a (+x/−x, +y/−y, +z/−z) octant corner. Winding chosen so
        // the normal points away from center; verified by the unit test below.
        let tris = vec![
            [0, 2, 4],
            [2, 1, 4],
            [1, 3, 4],
            [3, 0, 4],
            [2, 0, 5],
            [1, 2, 5],
            [3, 1, 5],
            [0, 3, 5],
        ];
        TriMesh::new(verts, tris)
    }

    #[test]
    fn trimesh_winding_and_crossing() {
        let m = octahedron([0.0, 0.0, 0.0], 1.0);

        // Every face normal must point away from the (interior) centroid.
        for t in 0..m.triangles.len() {
            let (a, b, c) = m.tri(t);
            let ctr = [
                (a[0] + b[0] + c[0]) / 3.0,
                (a[1] + b[1] + c[1]) / 3.0,
                (a[2] + b[2] + c[2]) / 3.0,
            ];
            let n = m.tri_normal(t);
            assert!(
                dot(n, ctr) > 0.0,
                "face {t} normal points inward: n={n:?} ctr={ctr:?}"
            );
        }

        // Inside near the center, outside well beyond the radius.
        assert!(m.inside([0.0, 0.0, 0.0]), "center should be inside");
        assert!(m.inside([0.1, -0.2, 0.3]), "near-center should be inside");
        assert!(!m.inside([2.0, 0.0, 0.0]), "far point should be outside");
        assert!(!m.inside([0.7, 0.7, 0.7]), "outside the octant face");

        // An edge from the inside out through the +++ octant face (the plane
        // x+y+z = r): aim at a face interior, not a vertex/edge.
        let hit = m.edge_crossing([0.1, 0.1, 0.1], [2.0, 1.5, 1.2]);
        let (p, n) = hit.expect("segment from inside to outside must cross");
        // The crossing sits on that face plane (x+y+z ≈ r = 1) and the outward
        // normal of the +++ face is (1,1,1)/√3 — all components positive.
        assert!(
            (p[0] + p[1] + p[2] - 1.0).abs() < 1e-9,
            "crossing not on the x+y+z=1 face: {p:?}"
        );
        assert!(
            n[0] > 0.0 && n[1] > 0.0 && n[2] > 0.0,
            "+++ face outward normal should be all-positive: {n:?}"
        );

        // A segment entirely inside must NOT report a crossing.
        assert!(
            m.edge_crossing([0.0, 0.0, 0.0], [0.1, 0.0, 0.0]).is_none(),
            "interior segment should not cross the boundary"
        );
    }

    /// The `Boundary` trait generalizes off the box with ZERO contouring-code
    /// changes — a `TriMesh` (closed polyhedron) drops in as a new impl. A tilted-
    /// plane contact is closed against an octahedron mesh the plane cuts through (a
    /// real seam on tilted, grid-misaligned faces).
    ///
    /// Contouring the combined field `g = max(field − iso, b)` (one DC pass) makes
    /// the contact-meets-boundary seam a watertight crease — ZERO open edges, even
    /// on the tilted faces that the earlier two-pass scheme could not stitch.
    ///
    /// Residual: the octahedron's sharp CREASES (where two coarse mesh faces meet
    /// at a hard edge) are a genuine "two surface components in one cell" case that
    /// a single QEF vertex per cell cannot resolve — it leaves a tiny number of
    /// non-manifold edges (a handful out of thousands). That is the one case the
    /// CSG-union approach does NOT cover; the textbook fix is Manifold Dual
    /// Contouring's per-cell vertex split (Schaefer/Ju/Warren 2007), tracked as a
    /// possible follow-up. We assert a small ceiling, not zero, and log the count.
    #[test]
    fn trimesh_boundary_generalizes_off_the_box() {
        use std::collections::HashMap;
        // Tilted-plane contact through z ≈ 500 — guaranteed to cut the octahedron.
        let mut sps = Vec::new();
        for i in 0..4 {
            for j in 0..4 {
                let x = i as f64 * 300.0;
                let y = j as f64 * 300.0;
                let z = 500.0 + 0.1 * (x - 450.0) + 0.05 * (y - 450.0);
                sps.push(SurfacePoint::new([x, y, z], 0));
            }
        }
        let oris = vec![Orientation::new([450.0, 450.0, 500.0], [-0.1, -0.05, 1.0])];
        let model = Model::fit(&Series::new(sps, oris), KernelOptions::default());

        let boundary = octahedron([500.0, 500.0, 500.0], 400.0);
        let cfg = OctreeConfig {
            max_depth: 6,
            min_depth: 3,
            qef_threshold: 1e-6,
            cap: true,
        };
        let mesh =
            octree_dual_contour_bounded(&model, &boundary, [0.0, 0.0, 0.0], 1000.0, 0.0, cfg);
        assert!(
            mesh.triangle_count() > 100,
            "expected a non-trivial mesh, got {}",
            mesh.triangle_count()
        );

        let mut undirected: HashMap<(u32, u32), u32> = HashMap::new();
        for t in mesh.indices.chunks_exact(3) {
            for &(a, b) in &[(t[0], t[1]), (t[1], t[2]), (t[2], t[0])] {
                let e = if a < b { (a, b) } else { (b, a) };
                *undirected.entry(e).or_insert(0) += 1;
            }
        }
        let nonmanifold = undirected.values().filter(|&&c| c > 2).count();
        let open = undirected.values().filter(|&&c| c == 1).count();
        let total = undirected.len();

        // The smooth contact/boundary seam closes watertight. The ONLY residual is
        // at the octahedron's sharp CREASES (two flat mesh faces meeting at a hard
        // dihedral) — a true two-components-in-one-cell case at coarse, grid-
        // misaligned mesh edges, a few of which don't route cleanly yet (manifold-DC
        // routing where a crease coincides with a 2:1 transition). Bound it small —
        // a few edges out of thousands, not a band. The smooth two-field truncation
        // (the geologic target) is asserted strictly watertight in
        // `csg_truncation_is_watertight`; this coarse-polyhedron crease is a
        // synthetic worst case kept as a small non-zero ceiling on purpose.
        assert!(
            open + nonmanifold <= 8,
            "crease residual grew beyond a handful: open={open} nm={nonmanifold} / {total}"
        );
        eprintln!("[trimesh] tris={} open={open} crease_nm={nonmanifold}", mesh.triangle_count());
    }

    /// A planar contact `z = z0 + sx*(x-500) + sy*(y-500)` as a fitted Model. The
    /// field increases upward, so `field < iso0` (iso0 = the recovered isovalue) is
    /// below the contact.
    fn dipping_plane(z0: f64, sx: f64, sy: f64) -> (Model, f64) {
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
        let model = Model::fit(&Series::new(sps, oris), KernelOptions::default());
        let iso = model.lithology().interfaces[0].isovalue;
        (model, iso)
    }

    /// An erosion/unconformity contact truncating a conformable contact — two
    /// surfaces intersecting inside the domain, combined as one CSG `Scene` solid
    /// `{ below primary } ∩ { below erosion } ∩ { in box }` and contoured in one
    /// pass. The mesh must be watertight (every edge incidence exactly 2) at both a
    /// STEEP and a LOW-ANGLE crossing.
    ///
    /// NOTE on what this does and does NOT exercise: the boundary of the CSG
    /// intersection at the contacts' meeting line is a CREASE (one sheet bending),
    /// not two disconnected sheets in a cell — so the combined-sign cells stay
    /// single-component and the CSG-union contour already closes them watertight
    /// (confirmed: this geometry produces zero multi-component cells at this
    /// resolution). The per-component Manifold-DC split (`minority_components` /
    /// per-component `cell_vertices`) is therefore NOT what makes this pass; it is
    /// latent here. (Smooth-geologic CSG combinations were found not to produce the
    /// two-disconnected-sheets-in-one-cell case the split fixes — that arises from
    /// MC ambiguous faces and sharp INPUT-MESH creases, e.g. a faceted fault. The
    /// genuinely hard thin-grazing-layer case is sub-cell undersampling, addressed
    /// by refinement, not vertex-splitting.) This test guards the CSG-truncation
    /// watertightness that geology actually relies on.
    #[test]
    fn csg_truncation_is_watertight() {
        use std::collections::HashMap;

        // Two contacts crossing near the domain center. `primary` dips gently one
        // way; `erosion` dips the OTHER way by `slope`, so they intersect along a
        // line through z≈600. Small `slope` → low crossing angle (wider two-sheet
        // band); large `slope` → steep.
        let check = |slope: f64, label: &str| {
            let (m_primary, iso_p) = dipping_plane(600.0, 0.05, 0.0);
            let (m_erosion, iso_e) = dipping_plane(600.0, -slope, 0.0);
            let box_b = AxisBox::new([0.0, 0.0, 0.0], 1000.0);
            let erosion = ContactSurface::new(&m_erosion, iso_e);
            let scene = Scene::new(&m_primary, iso_p, vec![&box_b, &erosion]);

            let cfg = OctreeConfig {
                max_depth: 6,
                min_depth: 3,
                qef_threshold: 1e-6,
                cap: true,
            };
            let mesh = octree_dual_contour_scene(&scene, [0.0, 0.0, 0.0], 1000.0, cfg);
            assert!(
                mesh.triangle_count() > 100,
                "{label}: trivial mesh ({} tris) — surfaces didn't intersect in-domain",
                mesh.triangle_count()
            );

            let mut undirected: HashMap<(u32, u32), u32> = HashMap::new();
            for t in mesh.indices.chunks_exact(3) {
                for &(a, b) in &[(t[0], t[1]), (t[1], t[2]), (t[2], t[0])] {
                    let e = if a < b { (a, b) } else { (b, a) };
                    *undirected.entry(e).or_insert(0) += 1;
                }
            }
            let nonmanifold = undirected.values().filter(|&&c| c > 2).count();
            let open = undirected.values().filter(|&&c| c == 1).count();
            let total = undirected.len();
            eprintln!(
                "[truncation {label}] slope={slope} tris={} nm={nonmanifold} open={open}/{total}",
                mesh.triangle_count()
            );
            assert_eq!(nonmanifold, 0, "{label}: non-manifold edges {nonmanifold}/{total}");
            assert_eq!(open, 0, "{label}: open edges {open}/{total}");
        };

        check(0.7, "steep");
        check(0.08, "low-angle");
    }

    /// Test alias for the public [`ContactHalfSpace`]: `inside` is below
    /// (`field < iso`) when `below`, else above. Used to build a slab
    /// (`below(hi) ∩ above(lo)`) from one field.
    fn half_space(m: &Model, iso: f64, below: bool) -> ContactHalfSpace<'_> {
        ContactHalfSpace::new(m, iso, below)
    }

    #[test]
    fn edge_double_crossed_detects_two_surfaces() {
        // A slab (below-hi ∩ above-lo) plus the box. An edge that pierces BOTH the
        // lo and hi faces is double-crossed; one piercing a single face is not.
        let (m, _) = dipping_plane(500.0, 0.0, 0.0); // flat, field ∝ z
        let lo = m.scalar_field([500.0, 500.0, 480.0]);
        let hi = m.scalar_field([500.0, 500.0, 520.0]);
        let below_hi = half_space(&m, hi, true);
        let above_lo = half_space(&m, lo, false);
        let box_b = AxisBox::new([0.0, 0.0, 0.0], 1000.0);
        let scene = Scene::new(&m, hi, vec![&box_b, &above_lo]);
        let _ = below_hi;

        // Vertical edge spanning the whole slab (z 400→600) crosses BOTH faces.
        assert!(
            scene.edge_double_crossed([500.0, 500.0, 400.0], [500.0, 500.0, 600.0]),
            "edge through both slab faces should be double-crossed"
        );
        // A short edge inside/near one face only — single crossing.
        assert!(
            !scene.edge_double_crossed([500.0, 500.0, 515.0], [500.0, 500.0, 525.0]),
            "edge through one face only should not be double-crossed"
        );
        // An edge entirely outside the slab in z but the primary(hi) still bounds —
        // below 480 is above-lo=false → not in solid → no boundary crossings here.
        assert!(
            !scene.edge_double_crossed([500.0, 500.0, 100.0], [500.0, 500.0, 200.0]),
            "edge far from both faces should not be double-crossed"
        );
    }

    /// A THICK slab (`below-hi ∩ above-lo ∩ box`) — two contacts bounding a layer —
    /// must mesh watertight. Before the double-crossed-edge refinement, the lo/hi
    /// faces cracked where they grazed the box wall (a cell edge pierced by two
    /// surfaces averaged into one vertex). The refinement subdivides those cells so
    /// each edge has ≤1 boundary crossing; the result is every edge incidence 2.
    #[test]
    fn csg_slab_is_watertight() {
        use std::collections::HashMap;
        let (m, _) = dipping_plane(500.0, 0.05, 0.03);
        let f_at = |z: f64| m.scalar_field([500.0, 500.0, z]);
        let lo = f_at(500.0).min(f_at(620.0));
        let hi = f_at(500.0).max(f_at(620.0));
        let above_lo = half_space(&m, lo, false);
        let box_b = AxisBox::new([0.0, 0.0, 0.0], 1000.0);
        // primary = below-hi (Model m at iso hi); contributors = box ∩ above-lo.
        let scene = Scene::new(&m, hi, vec![&box_b, &above_lo]);

        let cfg = OctreeConfig {
            max_depth: 7,
            min_depth: 2,
            qef_threshold: 1e-6,
            cap: true,
        };
        let mesh = octree_dual_contour_scene(&scene, [0.0, 0.0, 0.0], 1000.0, cfg);
        assert!(mesh.triangle_count() > 100, "trivial slab mesh");

        let mut undirected: HashMap<(u32, u32), u32> = HashMap::new();
        for t in mesh.indices.chunks_exact(3) {
            for &(a, b) in &[(t[0], t[1]), (t[1], t[2]), (t[2], t[0])] {
                let e = if a < b { (a, b) } else { (b, a) };
                *undirected.entry(e).or_insert(0) += 1;
            }
        }
        let nonmanifold = undirected.values().filter(|&&c| c > 2).count();
        let open = undirected.values().filter(|&&c| c == 1).count();
        let total = undirected.len();
        eprintln!("[slab] tris={} nm={nonmanifold} open={open}/{total}", mesh.triangle_count());
        assert_eq!(nonmanifold, 0, "slab non-manifold edges {nonmanifold}/{total}");
        assert_eq!(open, 0, "slab open edges {open}/{total}");
    }

    /// The headline geologic case: an EROSIONAL UNCONFORMITY (a contact) truncating
    /// a conformable BED (a slab between two contacts) at a LOW angle. Three surfaces
    /// — the bed's lower and upper contacts and the erosion — bound the solid, and
    /// they meet at low-angle intersection lines (the double-crossed-edge case). Must
    /// mesh watertight (every edge incidence exactly 2).
    #[test]
    fn erosion_truncating_a_bed_is_watertight() {
        use std::collections::HashMap;
        // Bed: a flat-lying slab between two contacts of field `bed`.
        let (bed, _) = dipping_plane(500.0, 0.02, 0.0);
        let f_at = |z: f64| bed.scalar_field([500.0, 500.0, z]);
        let lo = f_at(450.0).min(f_at(550.0));
        let hi = f_at(450.0).max(f_at(550.0));
        let above_lo = half_space(&bed, lo, false);
        // Erosion: a separate contact dipping the OTHER way at low angle, cutting the
        // bed near the middle of the domain.
        let (ero, iso_e) = dipping_plane(520.0, -0.06, 0.02);
        let erosion = half_space(&ero, iso_e, true);
        let box_b = AxisBox::new([0.0, 0.0, 0.0], 1000.0);
        // primary = below-hi (the bed's upper contact); contributors = box, above-lo
        // (bed's lower contact), erosion.
        let scene = Scene::new(&bed, hi, vec![&box_b, &above_lo, &erosion]);

        let cfg = OctreeConfig {
            max_depth: 7,
            min_depth: 2,
            qef_threshold: 1e-6,
            cap: true,
        };
        let mesh = octree_dual_contour_scene(&scene, [0.0, 0.0, 0.0], 1000.0, cfg);
        assert!(mesh.triangle_count() > 100, "trivial truncated-bed mesh");

        let mut undirected: HashMap<(u32, u32), u32> = HashMap::new();
        for t in mesh.indices.chunks_exact(3) {
            for &(a, b) in &[(t[0], t[1]), (t[1], t[2]), (t[2], t[0])] {
                let e = if a < b { (a, b) } else { (b, a) };
                *undirected.entry(e).or_insert(0) += 1;
            }
        }
        let nonmanifold = undirected.values().filter(|&&c| c > 2).count();
        let open = undirected.values().filter(|&&c| c == 1).count();
        let total = undirected.len();
        eprintln!("[erosion-bed] tris={} nm={nonmanifold} open={open}/{total}", mesh.triangle_count());
        assert_eq!(nonmanifold, 0, "truncated-bed non-manifold {nonmanifold}/{total}");
        assert_eq!(open, 0, "truncated-bed open edges {open}/{total}");
    }

    /// The closed solid (`cap = true`) must be CONSISTENTLY WOUND OUTWARD: every
    /// shared edge is traversed once per direction (no two neighbors disagree), and
    /// the overall winding is outward (positive signed volume). A curved contact
    /// previously left a band of inward-wound triangles (the per-face ε-probe
    /// mis-flipped near the box edges), rendering as back-faces / "missing"
    /// triangles. Flood-fill orientation makes inconsistency zero by construction.
    #[test]
    fn closed_solid_is_consistently_wound_outward() {
        use std::collections::HashMap;
        // Curved (paraboloid) contact in a box — the case that showed the back-face
        // band in the harness.
        let mut sps = Vec::new();
        for i in 0..5 {
            for j in 0..5 {
                let x = i as f64 * 250.0;
                let y = j as f64 * 250.0;
                let z = 500.0 + 0.0008 * ((x - 500.0).powi(2) + (y - 500.0).powi(2));
                sps.push(SurfacePoint::new([x, y, z], 0));
            }
        }
        let model = Model::fit(
            &Series::new(sps, vec![Orientation::new([500.0, 500.0, 500.0], [0.0, 0.0, 1.0])]),
            KernelOptions::default(),
        );
        let cfg = OctreeConfig {
            max_depth: 6,
            min_depth: 2,
            qef_threshold: 1e-4,
            cap: true,
        };
        let mesh = octree_dual_contour(&model, [0.0, 0.0, 0.0], 1000.0, 0.0, cfg);
        let tris = mesh.indices.len() / 3;
        assert!(tris > 100);

        // Consistent winding: for every directed edge, the reverse appears the same
        // number of times (a closed, consistently-wound manifold).
        let mut dir: HashMap<(u32, u32), i32> = HashMap::new();
        for t in 0..tris {
            let v = [mesh.indices[3 * t], mesh.indices[3 * t + 1], mesh.indices[3 * t + 2]];
            for &(a, b) in &[(v[0], v[1]), (v[1], v[2]), (v[2], v[0])] {
                *dir.entry((a, b)).or_insert(0) += 1;
            }
        }
        let inconsistent = dir
            .iter()
            .filter(|(&(a, b), &n)| a < b && n != *dir.get(&(b, a)).unwrap_or(&0))
            .count();
        assert_eq!(inconsistent, 0, "inconsistently-wound edges (back-face band): {inconsistent}");

        // Outward: signed volume positive.
        let p = |i: u32| {
            let b = (i as usize) * 3;
            [
                mesh.positions[b] as f64,
                mesh.positions[b + 1] as f64,
                mesh.positions[b + 2] as f64,
            ]
        };
        let mut vol6 = 0.0;
        for t in 0..tris {
            let v = [mesh.indices[3 * t], mesh.indices[3 * t + 1], mesh.indices[3 * t + 2]];
            let a = p(v[0]);
            let b = p(v[1]);
            let c = p(v[2]);
            vol6 += a[0] * (b[1] * c[2] - b[2] * c[1]) - a[1] * (b[0] * c[2] - b[2] * c[0])
                + a[2] * (b[0] * c[1] - b[1] * c[0]);
        }
        assert!(vol6 > 0.0, "closed solid wound inward (negative signed volume)");
    }

    /// The OPEN sheet (`cap = false`) of a CURVED contact must terminate flush along
    /// the domain box with no torn perimeter. (The sheet is derived from the closed
    /// solid by keeping contact faces, so it inherits the solid's clean boundary
    /// closure instead of tearing where curved minimal-edges meet the wall.) The
    /// only boundary edges allowed are the perimeter where the contact daylights on
    /// a box face — both endpoints on the box.
    #[test]
    fn open_sheet_boundary_is_flush_on_curved_contact() {
        use std::collections::HashMap;
        let mut sps = Vec::new();
        for i in 0..5 {
            for j in 0..5 {
                let x = i as f64 * 250.0;
                let y = j as f64 * 250.0;
                let z = 500.0 + 0.0008 * ((x - 500.0).powi(2) + (y - 500.0).powi(2));
                sps.push(SurfacePoint::new([x, y, z], 0));
            }
        }
        let model = Model::fit(
            &Series::new(sps, vec![Orientation::new([500.0, 500.0, 500.0], [0.0, 0.0, 1.0])]),
            KernelOptions::default(),
        );
        let cfg = OctreeConfig {
            max_depth: 6,
            min_depth: 2,
            qef_threshold: 1e-4,
            cap: false,
        };
        let mesh = octree_dual_contour(&model, [0.0, 0.0, 0.0], 1000.0, 0.0, cfg);
        assert!(mesh.triangle_count() > 100);

        let p = |i: u32| {
            let b = (i as usize) * 3;
            [
                mesh.positions[b] as f64,
                mesh.positions[b + 1] as f64,
                mesh.positions[b + 2] as f64,
            ]
        };
        let on_box = |q: [f64; 3]| {
            (0..3).any(|d| q[d].abs() < 1.0 || (q[d] - 1000.0).abs() < 1.0)
        };
        let mut undirected: HashMap<(u32, u32), u32> = HashMap::new();
        for t in mesh.indices.chunks_exact(3) {
            for &(a, b) in &[(t[0], t[1]), (t[1], t[2]), (t[2], t[0])] {
                let e = if a < b { (a, b) } else { (b, a) };
                *undirected.entry(e).or_insert(0) += 1;
            }
        }
        // A boundary edge (incidence 1) is allowed only on the box rim (both
        // endpoints on a box face). An interior boundary edge is a torn perimeter.
        let torn = undirected
            .iter()
            .filter(|(_, &c)| c == 1)
            .filter(|((a, b), _)| !(on_box(p(*a)) && on_box(p(*b))))
            .count();
        assert_eq!(torn, 0, "open sheet has a torn (non-flush) perimeter: {torn}");
    }

    /// REGRESSION (the flipped-erosion "QEF ceiling spike"): a single dipping erosion plane
    /// with a NEGATED orientation pole (the authoring "Flip orientation" bit), meshed through
    /// the exact web3 path (`Stack::mesh_one_block`, `cap=false`, depth 6), must place every
    /// referenced contact vertex ON the contact — none flung to the domain ceiling — and leave
    /// no interior hole.
    ///
    /// The artifact: the dipping plane runs EXACTLY along the grid plane `z=750` at `x=750`,
    /// so `field − iso` at that corner is at the round-off floor (~1e-17). Its discrete sign
    /// (`<= 0`) is build-dependent — native and wasm32 land on opposite ULPs — flipping the
    /// corner below/above the contact and, on wasm only, keeping a contact face stretched up
    /// to a ceiling cap vertex. The `CONTACT_GRAZE_EPS` snap in `sample_corners` makes that
    /// sign deterministic across builds. This native assertion was ALREADY green pre-fix (the
    /// spike daylights only on wasm — see `web3/e2e/qef-spike.spec.ts`, the behaviour gate of
    /// record); it is kept so the documented repro travels with the engine and the off-contact
    /// invariant is pinned natively for the flipped case.
    #[test]
    fn flipped_erosion_contact_vertices_lie_on_the_contact() {
        use crate::stack::{Relation, Stack, StackedSeries};
        // The repro fixture (web3/e2e/fixtures/qef-spike-repro.geoproj.json): 16 contact points
        // rising in x (z=470/590/710/830 at x=50/350/650/950) and one orientation at
        // (500,500,650) with a FLIPPED pole (dip 21.8°, dipDir 270° → (sin d, 0, −cos d)).
        let mut sps = Vec::new();
        let zs = [470.0, 590.0, 710.0, 830.0];
        let xs = [50.0, 350.0, 650.0, 950.0];
        let ys = [50.0, 350.0, 650.0, 950.0];
        for (xi, &x) in xs.iter().enumerate() {
            for &y in &ys {
                sps.push(SurfacePoint::new([x, y, zs[xi]], 0));
            }
        }
        let d = 21.801409486351833_f64.to_radians();
        let oris = vec![Orientation::new([500.0, 500.0, 650.0], [d.sin(), 0.0, -d.cos()])];
        let model = Model::fit(&Series::new(sps, oris), KernelOptions::default());
        // One bounded erode series — exactly what `toSeriesJson` emits for the repro.
        let stack = Stack::new(vec![StackedSeries::new(model, Relation::Erode).into()]);
        let model = stack.series[0].model();
        let iso = model.lithology().interfaces[0].isovalue;
        let cfg = OctreeConfig { max_depth: 6, min_depth: 2, qef_threshold: 1e-6, cap: false };
        // web3 worker: min=[0,0,0], size=max extent dim=1000, ext_max=[1000;3].
        let mesh = stack
            .mesh_one_block(0, 0, 0, [0.0, 0.0, 0.0], 1000.0, [1000.0, 1000.0, 1000.0], cfg)
            .expect("mesh_one_block returned None");
        assert!(mesh.triangle_count() > 0, "empty mesh");

        let p = |i: u32| {
            let b = (i as usize) * 3;
            [mesh.positions[b] as f64, mesh.positions[b + 1] as f64, mesh.positions[b + 2] as f64]
        };
        // Every REFERENCED vertex sits near the contact (distance = |field-iso|/|grad|), a
        // fraction of a cell, and none is flung to the ceiling. A spike fails both.
        let referenced: std::collections::HashSet<u32> = mesh.indices.iter().copied().collect();
        let mut off_max = 0.0_f64;
        let mut z_max = 0.0_f64;
        for &i in &referenced {
            let pv = p(i);
            z_max = z_max.max(pv[2]);
            let f = model.scalar_field(pv) - iso;
            let g = model.scalar_field_gradient(pv);
            let gm = (g[0] * g[0] + g[1] * g[1] + g[2] * g[2]).sqrt().max(1e-9);
            off_max = off_max.max((f / gm).abs());
        }
        // depth-6 cell ≈ 15.6; the contact tops at z≈830 (the dipping rise stays under 900).
        assert!(off_max < 8.0, "flipped-erosion sheet vertices stray off the contact: {off_max:.1}");
        assert!(z_max < 900.0, "flipped-erosion vertex flung toward the ceiling: z_max={z_max:.1}");
    }

    /// Contact vertices are projected onto the contact (`field == iso`) — interior
    /// freely, seam vertices constrained to the boundary — so the sheet hugs the
    /// smooth surface instead of terracing from coarse-cell QEF placement. Assert the
    /// referenced sheet vertices sit close to the contact AND the flush boundary is
    /// preserved (seam projection stays on the wall). Uses the harness data (one
    /// raised corner) at a moderate depth.
    #[test]
    fn contact_vertices_lie_on_the_contact() {
        use std::collections::{HashMap, HashSet};
        let pts = [
            (100.0, 200.0, 600.0),
            (500.0, 200.0, 600.0),
            (900.0, 200.0, 800.0), // raised corner
            (100.0, 800.0, 600.0),
            (500.0, 800.0, 600.0),
            (900.0, 800.0, 600.0),
        ];
        let sps: Vec<_> = pts
            .iter()
            .map(|&(x, y, z)| SurfacePoint::new([x, y, z], 0))
            .collect();
        let model = Model::fit(
            &Series::new(sps, vec![Orientation::new([500.0, 500.0, 600.0], [0.0, 0.0, 1.0])]),
            KernelOptions::default(),
        );
        let iso = model.lithology().interfaces[0].isovalue;
        let cfg = OctreeConfig {
            max_depth: 6,
            min_depth: 2,
            qef_threshold: 1e-6,
            cap: false,
        };
        let mesh = octree_dual_contour(&model, [0.0, 0.0, 0.0], 1000.0, iso, cfg);

        let p = |i: u32| {
            let b = (i as usize) * 3;
            [
                mesh.positions[b] as f64,
                mesh.positions[b + 1] as f64,
                mesh.positions[b + 2] as f64,
            ]
        };
        // Every referenced sheet vertex sits near the contact (distance = |field-iso|
        // / |grad|). With projection this is a fraction of a cell.
        let referenced: HashSet<u32> = mesh.indices.iter().copied().collect();
        let mut off_max = 0.0_f64;
        for &i in &referenced {
            let pv = p(i);
            let f = model.scalar_field(pv) - iso;
            let g = model.scalar_field_gradient(pv);
            let gm = (g[0] * g[0] + g[1] * g[1] + g[2] * g[2]).sqrt().max(1e-9);
            off_max = off_max.max((f / gm).abs());
        }
        // cell at depth 6 ≈ 15.6; vertices should be within ~half a cell of the contact.
        assert!(off_max < 8.0, "sheet vertices stray off the contact: {off_max:.1}");

        // Flush boundary preserved (seam projection stayed on the wall): no torn edge.
        let on_box = |q: [f64; 3]| (0..3).any(|d| q[d].abs() < 1.0 || (q[d] - 1000.0).abs() < 1.0);
        let mut undirected: HashMap<(u32, u32), u32> = HashMap::new();
        for t in mesh.indices.chunks_exact(3) {
            for &(a, b) in &[(t[0], t[1]), (t[1], t[2]), (t[2], t[0])] {
                let e = if a < b { (a, b) } else { (b, a) };
                *undirected.entry(e).or_insert(0) += 1;
            }
        }
        let torn = undirected
            .iter()
            .filter(|(_, &c)| c == 1)
            .filter(|((a, b), _)| !(on_box(p(*a)) && on_box(p(*b))))
            .count();
        assert_eq!(torn, 0, "seam projection un-flushed the boundary: {torn} torn");
    }

    /// Terracing regression at COARSE depth. The raised-corner contact is a smooth
    /// single-valued height field whose steepest ramp rises <1 cell of HEIGHT across
    /// one depth-3 cell (size 125) — fully resolved in (x,y), NOT a sub-cell feature.
    /// Yet before the dominant-axis projection fix, the cell-clamped Newton step
    /// pinned ramp/seam vertices to cell z-faces (the contact point needed a z OUTSIDE
    /// the cell), landing them ~19–24 units off the contact — the visible stair-steps.
    ///
    /// This asserts the smoothness bound the artifact violates: every referenced sheet
    /// vertex sits within a small fraction of a cell of the true contact, at depth 3
    /// AND depth 4. It FAILS on the pre-fix cell-clamped projection (off-contact ≈ 23,
    /// i.e. ~0.18 cells at depth 3) — the artifact is caught here, not hidden by depth.
    /// Raising `max_depth` is explicitly NOT the fix; this test is coarse on purpose.
    ///
    /// The flush-boundary assertion is kept so the steps can't be "smoothed" by simply
    /// detaching seam vertices from the wall (which would tear the open-sheet rim).
    #[test]
    fn contact_is_not_terraced_at_coarse_depth() {
        use std::collections::{HashMap, HashSet};
        let pts = [
            (100.0, 200.0, 600.0),
            (500.0, 200.0, 600.0),
            (900.0, 200.0, 800.0), // raised corner
            (100.0, 800.0, 600.0),
            (500.0, 800.0, 600.0),
            (900.0, 800.0, 600.0),
        ];
        let sps: Vec<_> = pts
            .iter()
            .map(|&(x, y, z)| SurfacePoint::new([x, y, z], 0))
            .collect();
        let model = Model::fit(
            &Series::new(sps, vec![Orientation::new([500.0, 500.0, 600.0], [0.0, 0.0, 1.0])]),
            KernelOptions::default(),
        );
        let iso = model.lithology().interfaces[0].isovalue;

        let on_box = |q: [f64; 3]| (0..3).any(|d| q[d].abs() < 1.0 || (q[d] - 1000.0).abs() < 1.0);

        for depth in [3u32, 4] {
            let cfg = OctreeConfig {
                max_depth: depth,
                min_depth: 2,
                qef_threshold: 1e-6,
                cap: false,
            };
            let mesh = octree_dual_contour(&model, [0.0, 0.0, 0.0], 1000.0, iso, cfg);
            assert!(mesh.triangle_count() > 0, "depth {depth}: empty mesh");

            let p = |i: u32| {
                let b = (i as usize) * 3;
                [
                    mesh.positions[b] as f64,
                    mesh.positions[b + 1] as f64,
                    mesh.positions[b + 2] as f64,
                ]
            };
            // Off-contact distance |field-iso|/|grad| of every referenced sheet vertex.
            let referenced: HashSet<u32> = mesh.indices.iter().copied().collect();
            let mut off_max = 0.0_f64;
            for &i in &referenced {
                let pv = p(i);
                let f = model.scalar_field(pv) - iso;
                let g = model.scalar_field_gradient(pv);
                let gm = (g[0] * g[0] + g[1] * g[1] + g[2] * g[2]).sqrt().max(1e-9);
                off_max = off_max.max((f / gm).abs());
            }
            let cell = 1000.0 / 2f64.powi(depth as i32);
            // Bound: < 1/12 of a cell. At depth 3 (cell 125) that is ≈10.4 — far below
            // the pre-fix strays (≈23) and far above the post-fix residual (≈0).
            let bound = cell / 12.0;
            assert!(
                off_max < bound,
                "depth {depth}: contact terraced — max off-contact {off_max:.1} ≥ {bound:.1} (cell {cell:.0})"
            );

            // Flush boundary preserved (the smoothing did not detach seam vertices from
            // the wall): the only boundary edges are on the box rim.
            let mut undirected: HashMap<(u32, u32), u32> = HashMap::new();
            for t in mesh.indices.chunks_exact(3) {
                for &(a, b) in &[(t[0], t[1]), (t[1], t[2]), (t[2], t[0])] {
                    let e = if a < b { (a, b) } else { (b, a) };
                    *undirected.entry(e).or_insert(0) += 1;
                }
            }
            let torn = undirected
                .iter()
                .filter(|(_, &c)| c == 1)
                .filter(|((a, b), _)| !(on_box(p(*a)) && on_box(p(*b))))
                .count();
            assert_eq!(torn, 0, "depth {depth}: seam projection un-flushed the boundary: {torn} torn");
        }
    }
}
