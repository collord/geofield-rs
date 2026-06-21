//! geofield — meshing, stacking, and faults for the cokriging scalar field.
//!
//! Built on top of [`geofield_kernel`], which owns the universal-cokriging
//! [`Model`]. This crate adds the geometry layers on top of that field: dual-contour
//! **meshing** of contacts and closed lithology solids (`mesh`, `octree`), the series
//! **stack** with erode/onlap masking (`stack`), and the **wasm** bindings.
//!
//! A faulted model meshes each fault-bounded block as separate, clipped sheets
//! (Leapfrog-style) and closed per-block unit solids capped by the fault — the
//! geometry that the kernel's per-block discontinuous field makes possible.
//!
//! Public entry point: [`Model::fit`] (re-exported from the kernel), then the
//! [`MeshModel`] extension trait for per-block meshing.

pub mod mesh;
pub mod octree;

// The multi-series stack (erode/onlap masking) is generic over `SeriesModel`, so it runs
// on BOTH targets: over the in-process kernel `Model` natively, and over `RemoteModel`
// (the wasm import bridge) in the browser.
pub mod stack;

// Hierarchy UI↔core DTOs (feature 001). The single source of truth for the grammar that
// crosses the boundary; generic over `SeriesModel` like `stack`, so it builds on both
// targets and is exercised by native DTO tests.
pub mod schema;

// Faults as an outer partition (feature 001 §4): the fault network instances the whole
// stack per block. Single-block stub for now (research R3); generic over `SeriesModel`.
pub mod faulted;

// Shared surface entities (feature 001 §5): a surface stored once, referenced by two
// boundaries (correlative conformity) without duplication. Data layer only in v1.
pub mod surface;

#[cfg(feature = "wasm")]
pub mod wasm;

// The field-oracle trait + boundary POD come from the permissive interface crate and
// are available on BOTH targets (native in-process kernel, and the wasm remote module).
pub use geofield_types::{HalfSpace, ScalarField};

pub use stack::{Relation, Stack, StackedSeries};

// NATIVE ONLY: the in-process kernel. Re-exported so `geofield::Model` / `geofield::data`
// etc. resolve for the CLI + native tests. The wasm build has NO kernel — it reaches the
// field oracle through the `remote` module's wasm imports instead.
#[cfg(not(target_arch = "wasm32"))]
pub use geofield_kernel::{
    assemble, data, evaluate, fault, geometry, kernel, litho, solve, FaultStack, Model,
    OneFault, PlanarFault,
};

// WASM ONLY: the remote field oracle (imports from the separately-loaded kernel module).
#[cfg(target_arch = "wasm32")]
pub mod remote;
#[cfg(target_arch = "wasm32")]
pub use remote::RemoteModel;

use mesh::{normalize, Mesh};
use octree::{
    octree_dual_contour, octree_dual_contour_scene, AxisBox, Boundary, ContactHalfSpace,
    OctreeConfig, PlaneHalfSpace, Scene,
};

/// What the meshing layer needs from a fitted model, abstracted so the SAME meshing code
/// serves the in-process kernel [`Model`] (native) and the [`RemoteModel`] that forwards
/// to the kernel wasm module (browser). Returns OWNED data (e.g. isovalues as a `Vec`),
/// because the remote model cannot hand out references into another wasm instance.
pub trait BlockModel {
    /// A per-block smooth-field oracle (one block's contact extrapolated over the box).
    type Field<'a>: ScalarField
    where
        Self: 'a;

    /// Number of fault-bounded blocks (1 if non-faulted).
    fn nblocks(&self) -> usize;
    /// Block `b`'s smooth field as a [`ScalarField`], or `None` if field-less.
    fn block_field(&self, b: usize) -> Option<Self::Field<'_>>;
    /// Block `b`'s interface isovalues, ascending (empty if field-less).
    fn block_interface_isovalues(&self, b: usize) -> Vec<f64>;
    /// Number of meshable interfaces in block `b`.
    fn block_interface_count(&self, b: usize) -> usize {
        self.block_interface_isovalues(b).len()
    }
    /// Number of lithology units in block `b` (`interfaces + 1`, 0 if field-less).
    fn block_unit_count(&self, b: usize) -> usize {
        match self.block_interface_count(b) {
            0 => 0,
            n => n + 1,
        }
    }
    /// The fault half-spaces bounding block `b` (for clipping its mesh to the block).
    fn block_half_spaces(&self, b: usize) -> Vec<HalfSpace>;
}

/// What the multi-series [`stack::Stack`] needs from a fitted model — the WHOLE-model
/// oracle (per-block dispatch across any fault) plus its representative interface set.
/// Abstracted so a `Stack` works over the native kernel [`Model`] and the wasm
/// [`RemoteModel`] alike. Like [`BlockModel`], returns OWNED interface data so it is safe
/// across separate wasm instances.
pub trait SeriesModel {
    /// The whole-model field oracle as a [`ScalarField`] (per-block dispatch).
    type Field<'a>: ScalarField
    where
        Self: 'a;

    /// Borrow the whole-model field oracle.
    fn field(&self) -> Self::Field<'_>;
    /// Scalar field at a world point (whole model, per-block dispatch).
    fn scalar_field(&self, p: [f64; 3]) -> f64;
    /// Lithology unit index at a world point (per-block classification).
    fn lithology_unit(&self, p: [f64; 3]) -> usize;
    /// Representative interface isovalues (ascending) — the whole-model set.
    fn interface_isovalues(&self) -> Vec<f64>;
    /// Representative interface ids, parallel to [`SeriesModel::interface_isovalues`].
    fn interface_ids(&self) -> Vec<u32>;
}

/// `geofield_kernel::Model` as a [`SeriesModel`] for the native (in-process) stack path.
#[cfg(not(target_arch = "wasm32"))]
impl SeriesModel for Model {
    type Field<'a> = &'a Model;
    fn field(&self) -> Self::Field<'_> {
        self
    }
    fn scalar_field(&self, p: [f64; 3]) -> f64 {
        Model::scalar_field(self, p)
    }
    fn lithology_unit(&self, p: [f64; 3]) -> usize {
        Model::lithology_unit(self, p)
    }
    fn interface_isovalues(&self) -> Vec<f64> {
        self.lithology().interfaces.iter().map(|iv| iv.isovalue).collect()
    }
    fn interface_ids(&self) -> Vec<u32> {
        self.lithology().interfaces.iter().map(|iv| iv.interface_id).collect()
    }
}

/// `geofield_kernel::Model` as a [`BlockModel`] for the native (in-process) meshing path.
#[cfg(not(target_arch = "wasm32"))]
impl BlockModel for Model {
    type Field<'a> = geofield_kernel::BlockField<'a>;
    fn nblocks(&self) -> usize {
        Model::nblocks(self)
    }
    fn block_field(&self, b: usize) -> Option<Self::Field<'_>> {
        Model::block_field(self, b)
    }
    fn block_interface_isovalues(&self, b: usize) -> Vec<f64> {
        match self.block_lithology(b) {
            Some(l) => l.interfaces.iter().map(|iv| iv.isovalue).collect(),
            None => Vec::new(),
        }
    }
    fn block_half_spaces(&self, b: usize) -> Vec<HalfSpace> {
        Model::block_half_spaces(self, b)
            .into_iter()
            .map(|h| HalfSpace { point: h.point, normal: h.normal, keep_positive: h.keep_positive })
            .collect()
    }
}

/// Per-block meshing for a fitted [`Model`] — an extension trait so the geometry layer
/// lives in this crate while the cokriging [`Model`] stays in `geofield-kernel`. Each
/// method contours ONE block's own smooth field (via [`Model::block_field`]) and clips
/// to that block's fault half-spaces ([`Model::block_half_spaces`]), so a faulted
/// model's contacts and unit solids are clean separate pieces stitched to the fault.
pub trait MeshModel {
    /// Mesh ONE interface of ONE block's contact, **clipped to that block's region**
    /// — the open-sheet (cap=false) view. For a faulted model this renders the
    /// rock1–rock2 contact in each block as a SEPARATE, unconnected sheet that stops
    /// at the fault plane (Leapfrog-style), instead of one mesh stitched across the
    /// discontinuity.
    ///
    /// Contour the block's OWN smooth field over the whole box (so the contact is
    /// continuous — no cross-fault discontinuity to crumple it), then drop the
    /// triangles whose centroid falls OUTSIDE the block's half-spaces. The contact ends
    /// flush at the fault as a clean cut, with no welded vertical seam wall (the clip
    /// is a pure post-hoc triangle drop, not a CSG boundary the contact bends toward).
    ///
    /// Returns `None` if `b` is field-less or `interface_idx` is out of range.
    /// `[min, min+size]³` is the octree cube.
    fn mesh_block_interface(
        &self,
        b: usize,
        interface_idx: usize,
        min: [f64; 3],
        size: f64,
        cfg: OctreeConfig,
    ) -> Option<Mesh>;

    /// A watertight CLOSED solid for one lithology UNIT of one fault block — the slab
    /// `{ iso[k-1] ≤ field_b < iso[k] }` intersected with the domain box and the
    /// block's fault half-spaces, contoured cap=true. The fault plane is a real cap
    /// face (the contact daylighting on the fault wall), so the block's layered solids
    /// stitch to the fault. Each is independently watertight (one combined CSG sign
    /// field), and a block's units tile its volume.
    ///
    /// `unit_idx` is `0..=interfaces` (bottom unit `0` has no lower contact; top unit
    /// `interfaces` has no upper contact). Returns `None` if `b` is field-less or
    /// `unit_idx` is out of range; the contour may still be empty if the unit has no
    /// volume in this block.
    fn mesh_block_unit_solid(
        &self,
        b: usize,
        unit_idx: usize,
        min: [f64; 3],
        size: f64,
        cfg: OctreeConfig,
    ) -> Option<Mesh>;

    /// All of block `b`'s lithology-unit solids (index = unit) for the closed-domain
    /// view, with their SHARED contacts WELDED so consecutive units coincide on their
    /// common boundary (no "green dips below blue").
    ///
    /// Each unit solid is meshed by the per-unit CSG path (watertight), then the
    /// vertices lying on a shared contact are snapped onto a CANONICAL height field for
    /// that contact — meshed ONCE via the surface contour (the projected/de-terraced
    /// position). Because both adjacent units snap their cap to the SAME canonical
    /// surface, the shared boundary matches exactly. (The per-unit re-contouring of a
    /// shared contact otherwise leaves a depth-independent ~½-cell gap; the robust fix
    /// is a single multi-label contour, tracked as a follow-up — this weld removes the
    /// visible gap without rewriting the contour.)
    fn mesh_block_unit_solids(
        &self,
        b: usize,
        min: [f64; 3],
        size: f64,
        cfg: OctreeConfig,
    ) -> Vec<Mesh>;

    /// One welded unit solid (the closed-domain view for a single lithology unit), with
    /// its shared contacts welded onto the canonical contact surfaces. Self-contained:
    /// builds only the 1–2 canonical contacts bounding this unit. `None` if the block is
    /// field-less or `unit_idx` is out of range. Used by the WASM per-unit binding.
    fn mesh_block_unit_solid_welded(
        &self,
        b: usize,
        unit_idx: usize,
        min: [f64; 3],
        size: f64,
        cfg: OctreeConfig,
    ) -> Option<Mesh>;
}

/// Blanket impl: any [`BlockModel`] gets the per-block meshing. Native uses the kernel
/// `Model`; wasm uses `RemoteModel`. The meshing code is written once against the trait.
impl<M: BlockModel> MeshModel for M {
    fn mesh_block_interface(
        &self,
        b: usize,
        interface_idx: usize,
        min: [f64; 3],
        size: f64,
        cfg: OctreeConfig,
    ) -> Option<Mesh> {
        let iso = *self.block_interface_isovalues(b).get(interface_idx)?;

        // Contour ONLY this block's field (smooth everywhere) as an open sheet.
        let field = self.block_field(b)?;
        let cfg = OctreeConfig { cap: false, ..cfg };
        let mut mesh = octree_dual_contour(&field, min, size, iso, cfg);

        // Drop triangles outside the block: a triangle is kept iff its centroid is on
        // the kept side of every fault half-space bounding this block. (Centroid test
        // is robust — a triangle straddling the fault is dropped, leaving a clean cut
        // edge within a cell of the plane.)
        let half_spaces = self.block_half_spaces(b);
        if !half_spaces.is_empty() {
            clip_mesh_to_half_spaces(&mut mesh, &half_spaces);
        }
        Some(mesh)
    }

    fn mesh_block_unit_solid(
        &self,
        b: usize,
        unit_idx: usize,
        min: [f64; 3],
        size: f64,
        cfg: OctreeConfig,
    ) -> Option<Mesh> {
        let isovalues = self.block_interface_isovalues(b);
        let n = isovalues.len();
        if n == 0 || unit_idx > n {
            return None;
        }
        let field = self.block_field(b)?;

        // Upper bound of the slab: the unit's UPPER contact, or — for the top unit —
        // a value above every field in the box, so `field < upper` holds everywhere
        // (the box top caps it). `primary` is always the "below the upper bound" side.
        let big = max_abs_field_in_box(&field, min, size) + 1.0;
        let upper_iso = if unit_idx < n {
            isovalues[unit_idx]
        } else {
            big // top unit: no upper contact, box caps it
        };

        // Lower bound (the ABOVE side of the unit's lower contact), if any.
        let lower: Option<ContactHalfSpace> = if unit_idx > 0 {
            Some(ContactHalfSpace::new(&field, isovalues[unit_idx - 1], false))
        } else {
            None
        };

        let box_b = AxisBox::new(min, size);
        let planes: Vec<PlaneHalfSpace> = self
            .block_half_spaces(b)
            .into_iter()
            .map(|hs| PlaneHalfSpace::new(hs.point, hs.normal, hs.keep_positive))
            .collect();

        let mut contributors: Vec<&dyn Boundary> = Vec::with_capacity(planes.len() + 2);
        contributors.push(&box_b);
        for p in &planes {
            contributors.push(p);
        }
        if let Some(lo) = &lower {
            contributors.push(lo);
        }

        let cfg = OctreeConfig { cap: true, ..cfg };
        let scene = Scene::new(&field, upper_iso, contributors);
        Some(octree_dual_contour_scene(&scene, min, size, cfg))
    }

    fn mesh_block_unit_solids(
        &self,
        b: usize,
        min: [f64; 3],
        size: f64,
        cfg: OctreeConfig,
    ) -> Vec<Mesh> {
        let n_interfaces = self.block_interface_count(b);
        if n_interfaces == 0 {
            return Vec::new();
        }
        // Canonical height field z(x,y) per contact, meshed ONCE (surface contour of
        // the block's smooth field, clipped to the block). Only valid for single-valued
        // (height-field) contacts — the geological norm; a near-vertical contact would
        // fall back to no weld (the column lookup just misses).
        let canon: Vec<ContactHeightField> = (0..n_interfaces)
            .map(|k| ContactHeightField::build(self, b, k, min, size, cfg))
            .collect();

        let mut out = Vec::with_capacity(self.block_unit_count(b));
        for u in 0..self.block_unit_count(b) {
            let mut mesh = self
                .mesh_block_unit_solid(b, u, min, size, cfg)
                .unwrap_or_default();
            for &k in bounding_contacts(u, n_interfaces).iter().flatten() {
                canon[k].weld(&mut mesh);
            }
            out.push(mesh);
        }
        out
    }

    fn mesh_block_unit_solid_welded(
        &self,
        b: usize,
        unit_idx: usize,
        min: [f64; 3],
        size: f64,
        cfg: OctreeConfig,
    ) -> Option<Mesh> {
        let n = self.block_interface_count(b);
        if n == 0 || unit_idx > n {
            return None;
        }
        let mut mesh = self.mesh_block_unit_solid(b, unit_idx, min, size, cfg)?;
        for &k in bounding_contacts(unit_idx, n).iter().flatten() {
            ContactHeightField::build(self, b, k, min, size, cfg).weld(&mut mesh);
        }
        Some(mesh)
    }
}

/// The interfaces bounding lithology unit `u` (of `n` interfaces): the lower
/// (`u-1`) and upper (`u`) contacts, each `None` at the bottom/top of the stack.
pub(crate) fn bounding_contacts(u: usize, n: usize) -> [Option<usize>; 2] {
    [u.checked_sub(1), (u < n).then_some(u)]
}

/// A value strictly larger than `|field|` anywhere in the box `[min, min+size]³`,
/// sampled on a coarse grid (the cokriging field is smooth, so a coarse sample
/// bounds it with margin). Used as the top lithology unit's "upper bound" so
/// `field < upper` holds everywhere and the box top caps the unit.
pub(crate) fn max_abs_field_in_box(field: &dyn ScalarField, min: [f64; 3], size: f64) -> f64 {
    let mut m = 0.0_f64;
    let steps = 6;
    for i in 0..=steps {
        for j in 0..=steps {
            for k in 0..=steps {
                let p = [
                    min[0] + size * i as f64 / steps as f64,
                    min[1] + size * j as f64 / steps as f64,
                    min[2] + size * k as f64 / steps as f64,
                ];
                let v = field.scalar_field(p);
                if v.is_finite() {
                    m = m.max(v.abs());
                }
            }
        }
    }
    m
}

/// Clip a triangle mesh to a fault block's region by SPLITTING triangles at each
/// bounding half-space plane (Sutherland–Hodgman per triangle), so the contact ends
/// FLUSH on the fault — a clean cut exactly at the plane, not a ragged per-triangle
/// keep/drop. New vertices created on a plane get interpolated positions AND normals
/// (so shading stays smooth up to the cut). Vertices that become unreferenced are left
/// in the buffers (harmless).
pub(crate) fn clip_mesh_to_half_spaces(mesh: &mut Mesh, half_spaces: &[HalfSpace]) {
    for hs in half_spaces {
        clip_mesh_to_plane(mesh, hs);
    }
}

/// One Sutherland–Hodgman pass: clip every triangle to the kept side of `hs`.
fn clip_mesh_to_plane(mesh: &mut Mesh, hs: &HalfSpace) {
    let has_normals = mesh.normals.len() == mesh.positions.len();
    let vpos = |m: &Mesh, i: u32| {
        let b = (i as usize) * 3;
        [m.positions[b] as f64, m.positions[b + 1] as f64, m.positions[b + 2] as f64]
    };
    let vnorm = |m: &Mesh, i: u32| {
        let b = (i as usize) * 3;
        [m.normals[b] as f64, m.normals[b + 1] as f64, m.normals[b + 2] as f64]
    };
    // Kept-signed distance: >= 0 on the kept side, < 0 outside.
    let kept_dist = |p: [f64; 3]| -> f64 {
        let d = (p[0] - hs.point[0]) * hs.normal[0]
            + (p[1] - hs.point[1]) * hs.normal[1]
            + (p[2] - hs.point[2]) * hs.normal[2];
        if hs.keep_positive {
            d
        } else {
            -d
        }
    };

    let old = std::mem::take(&mut mesh.indices);
    let mut out: Vec<u32> = Vec::with_capacity(old.len());
    for tri in old.chunks_exact(3) {
        let p = [vpos(mesh, tri[0]), vpos(mesh, tri[1]), vpos(mesh, tri[2])];
        let dk = [kept_dist(p[0]), kept_dist(p[1]), kept_dist(p[2])];
        // All inside → emit unchanged; all outside → drop. (Fast path, no new verts.)
        if dk.iter().all(|&d| d >= 0.0) {
            out.extend_from_slice(tri);
            continue;
        }
        if dk.iter().all(|&d| d < 0.0) {
            continue;
        }
        // Mixed: build the clipped polygon (kept corners + edge crossings), appending
        // a new vertex for each crossing. Collected as plain `mesh` reads first so the
        // borrow of `mesh.positions` ends before we push.
        let mut poly: Vec<u32> = Vec::with_capacity(4);
        for e in 0..3 {
            let (a, b) = (tri[e], tri[(e + 1) % 3]);
            let (da, db) = (dk[e], dk[(e + 1) % 3]);
            if da >= 0.0 {
                poly.push(a);
            }
            if (da >= 0.0) != (db >= 0.0) {
                let t = da / (da - db);
                let (pa, pb) = (vpos(mesh, a), vpos(mesh, b));
                let np = [
                    pa[0] + (pb[0] - pa[0]) * t,
                    pa[1] + (pb[1] - pa[1]) * t,
                    pa[2] + (pb[2] - pa[2]) * t,
                ];
                let nn = if has_normals {
                    let (na, nb) = (vnorm(mesh, a), vnorm(mesh, b));
                    Some(normalize([
                        na[0] + (nb[0] - na[0]) * t,
                        na[1] + (nb[1] - na[1]) * t,
                        na[2] + (nb[2] - na[2]) * t,
                    ]))
                } else {
                    None
                };
                let idx = (mesh.positions.len() / 3) as u32;
                mesh.positions.push(np[0] as f32);
                mesh.positions.push(np[1] as f32);
                mesh.positions.push(np[2] as f32);
                if let Some(n) = nn {
                    mesh.normals.push(n[0] as f32);
                    mesh.normals.push(n[1] as f32);
                    mesh.normals.push(n[2] as f32);
                }
                poly.push(idx);
            }
        }
        // Fan-triangulate the convex clipped polygon.
        for i in 1..poly.len().saturating_sub(1) {
            out.push(poly[0]);
            out.push(poly[i]);
            out.push(poly[i + 1]);
        }
    }
    mesh.indices = out;
}

/// A canonical mesh for ONE contact, meshed once via the surface contour (the
/// projected/de-terraced contact position). Used to WELD the per-unit closed solids'
/// caps onto a single shared surface so consecutive units coincide on their common
/// contact: a unit cap vertex near the canonical contact is snapped to its NEAREST
/// canonical vertex (full 3-D, so steep/raised-corner contacts weld correctly, not
/// just height fields). Both adjacent units snap to the same canonical vertices, so the
/// shared cap is identical.
pub(crate) struct ContactHeightField {
    /// Canonical contact vertices (the surface-contoured contact), in a coarse spatial
    /// hash for nearest-vertex lookup.
    verts: Vec<[f64; 3]>,
    /// `bucket(p) → indices into verts`, bucketed at `cell`.
    hash: std::collections::HashMap<(i64, i64, i64), Vec<u32>>,
    cell: f64,
    min: [f64; 3],
}

impl ContactHeightField {
    pub(crate) fn build<M: MeshModel + ?Sized>(
        model: &M,
        b: usize,
        k: usize,
        min: [f64; 3],
        size: f64,
        cfg: OctreeConfig,
    ) -> Self {
        // Bucket at ~one finest octree cell: a cap vertex's nearest canonical vertex is
        // within a cell, so a 3×3×3 bucket neighbourhood always contains it.
        let cell = (size / 2f64.powi(cfg.max_depth as i32)).max(1.0);
        let mut verts: Vec<[f64; 3]> = Vec::new();
        let mut hash: std::collections::HashMap<(i64, i64, i64), Vec<u32>> =
            std::collections::HashMap::new();
        if let Some(mesh) = model.mesh_block_interface(b, k, min, size, cfg) {
            for i in mesh.indices.iter().copied().collect::<std::collections::HashSet<_>>() {
                let o = (i as usize) * 3;
                let p = [
                    mesh.positions[o] as f64,
                    mesh.positions[o + 1] as f64,
                    mesh.positions[o + 2] as f64,
                ];
                let idx = verts.len() as u32;
                verts.push(p);
                hash.entry(Self::bucket(p, min, cell)).or_default().push(idx);
            }
        }
        Self { verts, hash, cell, min }
    }

    fn bucket(p: [f64; 3], min: [f64; 3], cell: f64) -> (i64, i64, i64) {
        (
            ((p[0] - min[0]) / cell).floor() as i64,
            ((p[1] - min[1]) / cell).floor() as i64,
            ((p[2] - min[2]) / cell).floor() as i64,
        )
    }

    /// Snap each vertex of `mesh` within ~one cell of a canonical contact vertex onto
    /// that nearest canonical vertex (so both adjacent units land on the same surface).
    pub(crate) fn weld(&self, mesh: &mut Mesh) {
        if self.verts.is_empty() {
            return;
        }
        let nverts = mesh.positions.len() / 3;
        for i in 0..nverts {
            let o = i * 3;
            let p = [
                mesh.positions[o] as f64,
                mesh.positions[o + 1] as f64,
                mesh.positions[o + 2] as f64,
            ];
            let (bx, by, bz) = Self::bucket(p, self.min, self.cell);
            let mut best = (self.cell * self.cell, usize::MAX); // weld only within a cell
            for dx in -1..=1 {
                for dy in -1..=1 {
                    for dz in -1..=1 {
                        if let Some(ids) = self.hash.get(&(bx + dx, by + dy, bz + dz)) {
                            for &vi in ids {
                                let c = self.verts[vi as usize];
                                let d2 = (c[0] - p[0]).powi(2)
                                    + (c[1] - p[1]).powi(2)
                                    + (c[2] - p[2]).powi(2);
                                if d2 < best.0 {
                                    best = (d2, vi as usize);
                                }
                            }
                        }
                    }
                }
            }
            if best.1 != usize::MAX {
                let c = self.verts[best.1];
                mesh.positions[o] = c[0] as f32;
                mesh.positions[o + 1] = c[1] as f32;
                mesh.positions[o + 2] = c[2] as f32;
            }
        }
    }
}

// These tests build an in-process kernel `Model`, so they only apply to the native path
// (the wasm engine has no kernel linked — it talks to the kernel module over imports).
#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;
    use super::MeshModel;
    use geofield_kernel::assemble::KernelOptions;
    use geofield_kernel::data::{Orientation, Series, SurfacePoint};
    use geofield_kernel::Model;

    /// A flat contact OFFSET by a fault: z=600 on the negative side of the fault
    /// plane x=500, z=700 on the positive side (a 100-unit throw). The surface points
    /// reflect the offset; fitting with a `OneFault` at x=500 must reproduce each
    /// block's own contact and make the field discontinuous across the plane.
    fn faulted_offset_series() -> Series {
        let mut sps = Vec::new();
        let mut oris = Vec::new();
        for i in 0..5 {
            for j in 0..3 {
                let x = i as f64 * 200.0 + 100.0; // 100,300,500,700,900
                let y = j as f64 * 300.0 + 200.0;
                // Left of fault (x<500): contact at z=600; right (x>=500): z=700.
                let z = if x < 500.0 { 600.0 } else { 700.0 };
                sps.push(SurfacePoint::new([x, y, z], 0));
            }
        }
        // One orientation per side so each block's gradient is anchored.
        oris.push(Orientation::new([300.0, 500.0, 600.0], [0.0, 0.0, 1.0]));
        oris.push(Orientation::new([700.0, 500.0, 700.0], [0.0, 0.0, 1.0]));
        Series::new(sps, oris)
    }

    fn fault_x500() -> Box<OneFault> {
        Box::new(OneFault::new(PlanarFault::new([500.0, 0.0, 0.0], [1.0, 0.0, 0.0])))
    }

    fn mesh_cfg(depth: u32) -> octree::OctreeConfig {
        octree::OctreeConfig {
            max_depth: depth,
            min_depth: 2,
            qef_threshold: 1e-6,
            cap: false,
        }
    }

    fn edge_incidence(mesh: &crate::mesh::Mesh) -> (usize, usize) {
        use std::collections::HashMap;
        let mut und: HashMap<(u32, u32), u32> = HashMap::new();
        for t in mesh.indices.chunks_exact(3) {
            for &(a, b) in &[(t[0], t[1]), (t[1], t[2]), (t[2], t[0])] {
                let e = if a < b { (a, b) } else { (b, a) };
                *und.entry(e).or_insert(0) += 1;
            }
        }
        let nm = und.values().filter(|&&c| c > 2).count();
        let open = und.values().filter(|&&c| c == 1).count();
        (nm, open)
    }

    /// Every (block, unit) closed solid is STRICTLY watertight — every edge incidence
    /// exactly 2 — at several depths. This is the faulted closed-domain view: each
    /// block's lithology slabs, capped by the box AND the fault plane.
    #[test]
    fn block_unit_solids_are_watertight() {
        for depth in [4u32, 5, 6] {
            let model = Model::fit_faulted(
                &faulted_offset_series(),
                fault_x500(),
                KernelOptions::default(),
            );
            let cfg = octree::OctreeConfig { cap: true, ..mesh_cfg(depth) };
            for b in 0..model.nblocks() {
                for u in 0..model.block_unit_count(b) {
                    let mesh = model
                        .mesh_block_unit_solid(b, u, [0.0, 0.0, 0.0], 1000.0, cfg)
                        .unwrap();
                    if mesh.triangle_count() == 0 {
                        continue; // a unit with no volume in this block
                    }
                    let (nm, open) = edge_incidence(&mesh);
                    assert_eq!(nm, 0, "d{depth} block {b} unit {u}: non-manifold {nm}");
                    assert_eq!(open, 0, "d{depth} block {b} unit {u}: open {open}");
                }
            }
        }
    }

    /// A faulted block's closed solids have the FAULT plane as a cap (faces lying in
    /// x=500) and none crossing to the far side — the layered solids stitch to the
    /// fault wall.
    #[test]
    fn block_unit_solid_has_fault_cap() {
        let model =
            Model::fit_faulted(&faulted_offset_series(), fault_x500(), KernelOptions::default());
        let cfg = octree::OctreeConfig { cap: true, ..mesh_cfg(5) };
        // Block 1 = positive side (x>=500); its solids must cap on x=500 and not cross.
        let mut total_fault_faces = 0;
        for u in 0..model.block_unit_count(1) {
            let mesh = model.mesh_block_unit_solid(1, u, [0.0, 0.0, 0.0], 1000.0, cfg).unwrap();
            let pos = |i: u32| {
                let k = (i as usize) * 3;
                [mesh.positions[k] as f64, mesh.positions[k + 1] as f64, mesh.positions[k + 2] as f64]
            };
            for &i in &mesh.indices {
                assert!(pos(i)[0] >= 500.0 - 0.5, "block 1 solid crossed the fault: {:?}", pos(i));
            }
            total_fault_faces += (0..mesh.triangle_count())
                .filter(|&t| {
                    let v = [mesh.indices[3*t], mesh.indices[3*t+1], mesh.indices[3*t+2]];
                    v.iter().all(|&j| (pos(j)[0] - 500.0).abs() < 0.5)
                })
                .count();
        }
        assert!(total_fault_faces > 0, "no fault cap faces on block 1");
    }

    /// A block's unit solids TILE the block: a point at a known structural position is
    /// inside exactly the unit its lithology says, and that unit's solid contains it
    /// (the slabs partition the block with no overlap/gap).
    #[test]
    fn block_unit_solids_partition_block() {
        let model =
            Model::fit_faulted(&faulted_offset_series(), fault_x500(), KernelOptions::default());
        // Block 1 (x>500). The offset data has the contact at z=700 on the right.
        // A point below it (z=650) is in a different unit than above it (z=750), by the
        // block's OWN lithology — confirming the slabs the closed solids fill are the
        // partition the unit count is built on.
        let lith = model.block_lithology(1).unwrap();
        let f = |p: [f64; 3]| model.scalar_field(p);
        let u_below = lith.unit_of_value(f([700.0, 500.0, 650.0]));
        let u_above = lith.unit_of_value(f([700.0, 500.0, 750.0]));
        assert_ne!(u_below, u_above, "below vs above the contact are different units");
        assert!(model.block_unit_count(1) >= 2, "two-contact block has >=2 units");
    }

    /// For a NON-faulted model, the per-block-unit solid equals the whole-domain
    /// closed solid at that unit's bounding isovalue — parity with the existing view.
    #[test]
    fn nonfaulted_block_unit_solid_watertight() {
        let model = Model::fit(&faulted_offset_series(), KernelOptions::default());
        assert_eq!(model.nblocks(), 1);
        let cfg = octree::OctreeConfig { cap: true, ..mesh_cfg(5) };
        for u in 0..model.block_unit_count(0) {
            let mesh = model.mesh_block_unit_solid(0, u, [0.0, 0.0, 0.0], 1000.0, cfg).unwrap();
            if mesh.triangle_count() == 0 {
                continue;
            }
            let (nm, open) = edge_incidence(&mesh);
            assert_eq!(nm, 0, "unit {u}: non-manifold");
            assert_eq!(open, 0, "unit {u}: open");
        }
    }

    /// THE zero-gap regression: consecutive lithology-unit solids must SHARE their
    /// common contact exactly (unit k's top cap == unit k+1's bottom cap), so there is
    /// no "green dips below blue". This holds only if the shared contact is contoured
    /// ONCE (the multi-label `mesh_block_unit_solids`), not twice (the per-unit CSG
    /// path, whose worst-case gap stays ~10 units at every depth). Assert the gap is
    /// near zero at a COARSE depth 3 AND a fine depth 6 — depth-independence is the
    /// whole point (a re-contoured boundary's gap does not shrink with depth).
    #[test]
    fn unit_solids_share_their_contact_exactly() {
        // Raised-corner seed: rock2 at z=600 (z=800 at one corner), rock1 at z=400.
        let pts = [
            (100.0, 200.0, 600.0, 0u32), (500.0, 200.0, 600.0, 0), (900.0, 200.0, 800.0, 0),
            (100.0, 800.0, 600.0, 0), (500.0, 800.0, 600.0, 0), (900.0, 800.0, 600.0, 0),
            (100.0, 200.0, 400.0, 1), (500.0, 200.0, 400.0, 1), (900.0, 200.0, 400.0, 1),
            (100.0, 800.0, 400.0, 1), (500.0, 800.0, 400.0, 1), (900.0, 800.0, 400.0, 1),
        ];
        let sps: Vec<_> = pts.iter().map(|&(x, y, z, id)| SurfacePoint::new([x, y, z], id)).collect();
        let oris = vec![
            Orientation::new([500.0, 500.0, 600.0], [0.0, 0.0, 1.0]),
            Orientation::new([500.0, 500.0, 400.0], [0.0, 0.0, 1.0]),
        ];
        let model = Model::fit(&Series::new(sps, oris), KernelOptions::default());

        for depth in [3u32, 6] {
            let cfg = octree::OctreeConfig { cap: true, ..mesh_cfg(depth) };
            let solids = model.mesh_block_unit_solids(0, [0.0, 0.0, 0.0], 1000.0, cfg);
            assert_eq!(solids.len(), model.block_unit_count(0), "one solid per unit");

            // Shared contact between unit 1 (blue) and unit 2 (green) is rock2 (iso[1]).
            // For every vertex of unit 2 ON that contact, the NEAREST vertex of unit 1
            // ON that contact must be ~coincident (3-D nearest, so steep/raised-corner
            // regions are handled). With welding both caps land on the same canonical
            // surface, so the nearest-vertex distance is ~0.
            let iso1 = model.lithology().interfaces[1].isovalue;
            let on_contact_pts = |m: &crate::mesh::Mesh| -> Vec<[f64; 3]> {
                m.indices
                    .iter()
                    .copied()
                    .collect::<std::collections::HashSet<_>>()
                    .into_iter()
                    .filter_map(|i| {
                        let b = (i as usize) * 3;
                        let p = [m.positions[b] as f64, m.positions[b + 1] as f64, m.positions[b + 2] as f64];
                        let f = model.scalar_field(p) - iso1;
                        let g = model.scalar_field_gradient(p);
                        let gm = (g[0] * g[0] + g[1] * g[1] + g[2] * g[2]).sqrt().max(1e-9);
                        ((f / gm).abs() < 12.0).then_some(p)
                    })
                    .collect()
            };
            let blue = on_contact_pts(&solids[1]);
            let green = on_contact_pts(&solids[2]);
            assert!(blue.len() > 10 && green.len() > 10, "depth {depth}: too few contact verts");
            let mut worst = 0.0_f64;
            for g in &green {
                let nearest = blue
                    .iter()
                    .map(|b| (b[0] - g[0]).powi(2) + (b[1] - g[1]).powi(2) + (b[2] - g[2]).powi(2))
                    .fold(f64::MAX, f64::min)
                    .sqrt();
                worst = worst.max(nearest);
            }
            assert!(
                worst < 2.0,
                "depth {depth}: shared contact gap {worst:.1} (units don't share the boundary)"
            );
        }
    }

    /// Welding the shared contact (snapping cap vertices onto the canonical contact)
    /// must NOT break watertightness — `mesh_block_unit_solids` solids stay manifold
    /// (every edge incidence 2, modulo the snap producing a degenerate triangle that is
    /// dropped). Asserts each welded unit solid has zero open edges. (A snap can merge
    /// two vertices into a degenerate tri; those are filtered, not left as a hole.)
    #[test]
    fn welded_unit_solids_stay_watertight() {
        let model =
            Model::fit_faulted(&faulted_offset_series(), fault_x500(), KernelOptions::default());
        for depth in [4u32, 5] {
            let cfg = octree::OctreeConfig { cap: true, ..mesh_cfg(depth) };
            for b in 0..model.nblocks() {
                let solids = model.mesh_block_unit_solids(b, [0.0, 0.0, 0.0], 1000.0, cfg);
                for (u, mesh) in solids.iter().enumerate() {
                    if mesh.triangle_count() == 0 {
                        continue;
                    }
                    let (nm, open) = edge_incidence(mesh);
                    assert_eq!(nm, 0, "d{depth} b{b} unit {u}: welded non-manifold {nm}");
                    assert_eq!(open, 0, "d{depth} b{b} unit {u}: welded open {open}");
                }
            }
        }
    }

    /// Per-block meshes are SEPARATE and CLIPPED: each block's contact mesh lies
    /// entirely on its own side of the fault plane (x=500), so the two never stitch
    /// across the discontinuity. This is the Leapfrog-style render — the fix for the
    /// crumpled fault-zone triangles.
    #[test]
    fn block_meshes_are_separate_and_clipped() {
        let model =
            Model::fit_faulted(&faulted_offset_series(), fault_x500(), KernelOptions::default());
        assert_eq!(model.nblocks(), 2);

        let pos = |m: &crate::mesh::Mesh, i: u32| {
            let b = (i as usize) * 3;
            [m.positions[b] as f64, m.positions[b + 1] as f64, m.positions[b + 2] as f64]
        };
        // Interface 0 of each block (both blocks have the rock2 + rock1 contacts).
        let m0 = model
            .mesh_block_interface(0, 0, [0.0, 0.0, 0.0], 1000.0, mesh_cfg(5))
            .expect("block 0 interface 0");
        let m1 = model
            .mesh_block_interface(1, 0, [0.0, 0.0, 0.0], 1000.0, mesh_cfg(5))
            .expect("block 1 interface 0");
        assert!(m0.triangle_count() > 0 && m1.triangle_count() > 0, "both blocks mesh");

        // Block 0 = negative side (x <= 500), block 1 = positive (x >= 500). With the
        // triangle-splitting clip the cut is FLUSH at x=500 (only f32 rounding slack).
        let referenced = |m: &crate::mesh::Mesh| -> std::collections::HashSet<u32> {
            m.indices.iter().copied().collect()
        };
        for &i in &referenced(&m0) {
            assert!(pos(&m0, i)[0] <= 500.0 + 0.1, "block 0 vertex crossed the fault: {:?}", pos(&m0, i));
        }
        for &i in &referenced(&m1) {
            assert!(pos(&m1, i)[0] >= 500.0 - 0.1, "block 1 vertex crossed the fault: {:?}", pos(&m1, i));
        }
    }

    /// No seam WALL at the fault: a per-block contact mesh must not contain near-
    /// vertical faces lying in the fault plane (the welded/snapped sawtooth the old
    /// CSG-clip produced). With the per-block smooth contour + triangle-split clip the
    /// contact ends flush as a clean cut, so there are zero faces parallel to the
    /// (vertical) fault plane. Uses the raised-corner screenshot data.
    #[test]
    fn no_seam_wall_at_the_fault() {
        let pts = [
            (100.0, 200.0, 500.0, 0u32), (500.0, 200.0, 600.0, 0), (900.0, 200.0, 800.0, 0),
            (100.0, 800.0, 600.0, 0), (500.0, 800.0, 600.0, 0), (900.0, 800.0, 600.0, 0),
            (100.0, 200.0, 400.0, 1), (500.0, 200.0, 400.0, 1), (900.0, 200.0, 400.0, 1),
            (100.0, 800.0, 400.0, 1), (500.0, 800.0, 400.0, 1), (900.0, 800.0, 400.0, 1),
        ];
        let sps: Vec<_> = pts.iter().map(|&(x, y, z, id)| SurfacePoint::new([x, y, z], id)).collect();
        let oris = vec![
            Orientation::new([500.0, 500.0, 600.0], [0.0, 0.0, 1.0]),
            Orientation::new([500.0, 500.0, 400.0], [0.0, 0.0, 1.0]),
        ];
        let series = Series::new(sps, oris);
        // Vertical fault at x=275 (the screenshot's offset=-225), normal +x.
        let fault = Box::new(OneFault::new(PlanarFault::new([275.0, 0.0, 0.0], [1.0, 0.0, 0.0])));
        let model = Model::fit_faulted(&series, fault, KernelOptions::default());

        let face_nz = |m: &crate::mesh::Mesh, t: usize| -> f64 {
            let i = [m.indices[3 * t], m.indices[3 * t + 1], m.indices[3 * t + 2]];
            let p = |k: u32| {
                let b = (k as usize) * 3;
                [m.positions[b] as f64, m.positions[b + 1] as f64, m.positions[b + 2] as f64]
            };
            let (a, b, c) = (p(i[0]), p(i[1]), p(i[2]));
            let e1 = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
            let e2 = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
            let n = [
                e1[1] * e2[2] - e1[2] * e2[1],
                e1[2] * e2[0] - e1[0] * e2[2],
                e1[0] * e2[1] - e1[1] * e2[0],
            ];
            let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt().max(1e-12);
            n[2].abs() / len // |cos| with +Z; ~0 ⇒ vertical wall face
        };
        for b in 0..model.nblocks() {
            for s in 0..model.block_interface_count(b) {
                let m = model
                    .mesh_block_interface(b, s, [0.0, 0.0, 0.0], 1000.0, mesh_cfg(5))
                    .unwrap();
                let walls = (0..m.triangle_count()).filter(|&t| face_nz(&m, t) < 0.05).count();
                assert_eq!(walls, 0, "block {b} iface {s}: {walls} vertical wall faces at the fault");
            }
        }
    }

    /// For a NON-faulted model, `mesh_block_interface(0, …)` is the ordinary
    /// whole-domain open sheet — the unified path doesn't regress the common case.
    #[test]
    fn nonfaulted_block_mesh_matches_whole_domain() {
        use crate::octree::octree_dual_contour;
        let model = Model::fit(&faulted_offset_series(), KernelOptions::default());
        assert_eq!(model.nblocks(), 1);
        let iso = model.lithology().interfaces[0].isovalue;

        let block = model
            .mesh_block_interface(0, 0, [0.0, 0.0, 0.0], 1000.0, mesh_cfg(5))
            .expect("single block");
        let whole = octree_dual_contour(&model, [0.0, 0.0, 0.0], 1000.0, iso, mesh_cfg(5));
        // Same mesh: no clip planes for a single block, so identical triangle count.
        assert_eq!(
            block.triangle_count(),
            whole.triangle_count(),
            "single-block mesh should equal the whole-domain open sheet"
        );
    }

    /// A field-less block (no contact) yields no mesh, and out-of-range indices None.
    #[test]
    fn mesh_block_interface_handles_missing() {
        // Right-only data → block 0 (negative side) is field-less.
        let mut sps = Vec::new();
        for &(x, y) in &[(600.0, 200.0), (800.0, 200.0), (600.0, 800.0), (800.0, 800.0)] {
            sps.push(SurfacePoint::new([x, y, 600.0], 0));
        }
        let oris = vec![Orientation::new([700.0, 500.0, 600.0], [0.0, 0.0, 1.0])];
        let series = Series::new(sps, oris);
        let fault = Box::new(OneFault::new(PlanarFault::new([500.0, 0.0, 0.0], [1.0, 0.0, 0.0])));
        let model = Model::fit_faulted(&series, fault, KernelOptions::default());

        assert!(model.mesh_block_interface(0, 0, [0.0, 0.0, 0.0], 1000.0, mesh_cfg(4)).is_none());
        assert!(model.mesh_block_interface(1, 9, [0.0, 0.0, 0.0], 1000.0, mesh_cfg(4)).is_none());
        assert!(model.mesh_block_interface(1, 0, [0.0, 0.0, 0.0], 1000.0, mesh_cfg(4)).is_some());
    }
}
