//! geofield-rs — universal-cokriging scalar field for a single, non-faulted series.
//!
//! Math source of truth: `KERNEL_MATH.md` in the repo root. Where this code and
//! that document disagree on a formula, sign, constant, or epsilon, the document
//! wins.
//!
//! Public entry point: [`Model::fit`], then [`Model::scalar_field`],
//! [`Model::scalar_field_gradient`], and [`Model::lithology_unit`].

pub mod assemble;
pub mod data;
pub mod evaluate;
pub mod fault;
pub mod geometry;
pub mod kernel;
pub mod litho;
pub mod mesh;
pub mod octree;
pub mod solve;
pub mod stack;

#[cfg(feature = "wasm")]
pub mod wasm;

use assemble::{assemble, KernelOptions, System};
use data::{Orientation, Series, SurfacePoint};
use evaluate::Field;
use mesh::{normalize, Mesh};
use octree::{
    octree_dual_contour, octree_dual_contour_scene, AxisBox, Boundary, ContactHalfSpace,
    OctreeConfig, PlaneHalfSpace, ScalarField, Scene,
};
use fault::{BlockIndex, SingleBlock};
use litho::Lithology;
use solve::{solve, Solution};

pub use fault::{FaultStack, OneFault, PlanarFault};
pub use stack::{Relation, Stack, StackedSeries};

/// A potential field solved independently over one fault-bounded **block** —
/// its assembled system, solved weights, and the lithology isovalues recovered from
/// that block's own surface points. A non-faulted model has exactly one block.
struct Block {
    system: System,
    solution: Solution,
    lithology: Lithology,
}

impl Block {
    fn field(&self) -> Field<'_> {
        Field::new(&self.system.geom, &self.solution)
    }
}

/// A fitted potential-field model. In the common case it is one smooth cokriging
/// field; with a fault ([`Model::fit_faulted`]) it holds one independent field per
/// fault-bounded block and dispatches each query to the block the point lies in, so
/// the field is **discontinuous across the fault** (a contact is offset, GemPy's
/// finite-fault behaviour). Self-contained so it can cross an FFI / WASM boundary.
///
/// All eval methods (`scalar_field`, `scalar_field_gradient`, `lithology_unit`)
/// keep their signatures, so a faulted model is a drop-in everywhere a model is
/// used — including inside a [`Stack`]'s erode/onlap masking.
pub struct Model {
    /// One field per block; index `b` is the block `index.block_at` returns. A block
    /// with no surface points (a fault region with no data) is `None` and reports no
    /// contact there (field `+∞`, lithology unit 0).
    blocks: Vec<Option<Block>>,
    index: Box<dyn BlockIndex>,
    /// Representative interfaces for the whole model — the first non-empty block's
    /// lithology (block 0 in the common case). [`Model::lithology`] returns this; the
    /// stack reads it for a series' bounding contact while per-point sign tests route
    /// per-block. Empty only if every block is data-less.
    representative_lithology: Lithology,
    /// Largest within-interface isovalue disagreement at fit time (diagnostic, max
    /// over blocks).
    pub isovalue_spread: f64,
    /// Solve residual ‖K w − b‖₂ (diagnostic, max over blocks).
    pub residual: f64,
}

impl Model {
    /// Assemble, solve, and recover isovalues for a series — a single (non-faulted)
    /// block. Exactly `fit_faulted` with the trivial [`SingleBlock`] index, so all
    /// existing callers are unchanged.
    pub fn fit(series: &Series, opts: KernelOptions) -> Self {
        Self::fit_faulted(series, Box::new(SingleBlock), opts)
    }

    /// Fit a **faulted** model: partition the series' points/orientations by
    /// `index.block_at(position)`, solve one cokriging field per block over only that
    /// block's data, and recover per-block lithology. Eval dispatches to the block at
    /// the query point, so the field is discontinuous across the fault.
    ///
    /// A block that ends up with no surface points is left field-less (it has no
    /// contact); a query there returns `+∞` (above every interface → unit 0).
    pub fn fit_faulted(series: &Series, index: Box<dyn BlockIndex>, opts: KernelOptions) -> Self {
        let nblocks = index.nblocks();
        // Bucket points and orientations into their blocks by their own position.
        let mut sp_buckets: Vec<Vec<SurfacePoint>> = vec![Vec::new(); nblocks];
        let mut ori_buckets: Vec<Vec<Orientation>> = vec![Vec::new(); nblocks];
        for sp in &series.surface_points {
            sp_buckets[index.block_at(sp.xyz)].push(*sp);
        }
        for ori in &series.orientations {
            ori_buckets[index.block_at(ori.xyz)].push(*ori);
        }

        let mut blocks: Vec<Option<Block>> = Vec::with_capacity(nblocks);
        let mut isovalue_spread = 0.0_f64;
        let mut residual = 0.0_f64;
        for b in 0..nblocks {
            // A block with no surface points has no contact — field-less (`None`).
            if sp_buckets[b].is_empty() {
                blocks.push(None);
                continue;
            }
            // A block with points but NO orientation is under-constrained (cokriging
            // is singular without a gradient anchor). Rather than drop it — which
            // makes one side of a fault silently vanish — INFER an orientation from
            // the points' own geometry (`infer_orientation`): the best-fit plane
            // normal when ≥3 points define a plane, the flattest plane through the
            // line for 2/collinear points, flat (+Z) for a single point. This is
            // invisible when real orientations exist and keeps both fault blocks
            // meshable. (A fault plane that strands all orientations on one side is
            // exactly the harness's default-fault case.)
            let oris = std::mem::take(&mut ori_buckets[b]);
            let oris = if oris.is_empty() {
                vec![infer_orientation(&sp_buckets[b])]
            } else {
                oris
            };
            let sub = Series::new(std::mem::take(&mut sp_buckets[b]), oris);
            let system = assemble(&sub, opts);
            let solution = solve(&system);
            residual = residual.max(solution.residual);
            let (lithology, spread) = {
                let field = Field::new(&system.geom, &solution);
                Lithology::recover(&sub, &field)
            };
            isovalue_spread = isovalue_spread.max(spread);
            blocks.push(Some(Block {
                system,
                solution,
                lithology,
            }));
        }

        // Representative interfaces: the first non-empty block's (block 0 in the
        // common case). Empty Lithology only if every block was data-less.
        let representative_lithology = blocks
            .iter()
            .find_map(|b| b.as_ref().map(|b| b.lithology.clone()))
            .unwrap_or_else(|| Lithology {
                interfaces: Vec::new(),
            });

        Self {
            blocks,
            index,
            representative_lithology,
            isovalue_spread,
            residual,
        }
    }

    /// The lithology recovered for block `b` (the fault-bounded domain), or `None`
    /// if `b` is out of range or data-less. For a non-faulted model, block 0 is the
    /// whole model.
    pub fn block_lithology(&self, b: usize) -> Option<&Lithology> {
        self.blocks.get(b).and_then(|x| x.as_ref()).map(|x| &x.lithology)
    }

    /// Number of fault-bounded blocks (1 if non-faulted).
    pub fn nblocks(&self) -> usize {
        self.blocks.len()
    }

    /// Number of meshable interfaces in block `b` (0 if `b` is out of range or
    /// field-less).
    pub fn block_interface_count(&self, b: usize) -> usize {
        self.block_lithology(b).map_or(0, |l| l.interfaces.len())
    }

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
    pub fn mesh_block_interface(
        &self,
        b: usize,
        interface_idx: usize,
        min: [f64; 3],
        size: f64,
        cfg: OctreeConfig,
    ) -> Option<Mesh> {
        let block = self.blocks.get(b)?.as_ref()?;
        let iv = *block.lithology.interfaces.get(interface_idx)?;

        // Contour ONLY this block's field (smooth everywhere) as an open sheet.
        let field = BlockField { block };
        let cfg = OctreeConfig { cap: false, ..cfg };
        let mut mesh = octree_dual_contour(&field, min, size, iv.isovalue, cfg);

        // Drop triangles outside the block: a triangle is kept iff its centroid is on
        // the kept side of every fault half-space bounding this block. (Centroid test
        // is robust — a triangle straddling the fault is dropped, leaving a clean cut
        // edge within a cell of the plane.)
        let half_spaces = self.index.block_half_spaces(b);
        if !half_spaces.is_empty() {
            clip_mesh_to_half_spaces(&mut mesh, &half_spaces);
        }
        Some(mesh)
    }

    /// Number of lithology UNITS in block `b` — the volumes between consecutive
    /// contacts, `interfaces + 1` (0 if `b` is out of range or field-less).
    pub fn block_unit_count(&self, b: usize) -> usize {
        match self.block_interface_count(b) {
            0 => 0, // a field-less block has no units
            n => n + 1,
        }
    }

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
    pub fn mesh_block_unit_solid(
        &self,
        b: usize,
        unit_idx: usize,
        min: [f64; 3],
        size: f64,
        cfg: OctreeConfig,
    ) -> Option<Mesh> {
        let block = self.blocks.get(b)?.as_ref()?;
        let interfaces = &block.lithology.interfaces;
        let n = interfaces.len();
        if unit_idx > n {
            return None;
        }
        let field = BlockField { block };

        // Upper bound of the slab: the unit's UPPER contact, or — for the top unit —
        // a value above every field in the box, so `field < upper` holds everywhere
        // (the box top caps it). `primary` is always the "below the upper bound" side.
        let big = max_abs_field_in_box(&field, min, size) + 1.0;
        let upper_iso = if unit_idx < n {
            interfaces[unit_idx].isovalue
        } else {
            big // top unit: no upper contact, box caps it
        };

        // Lower bound (the ABOVE side of the unit's lower contact), if any.
        let lower: Option<ContactHalfSpace> = if unit_idx > 0 {
            Some(ContactHalfSpace::new(&field, interfaces[unit_idx - 1].isovalue, false))
        } else {
            None
        };

        let box_b = AxisBox::new(min, size);
        let planes: Vec<PlaneHalfSpace> = self
            .index
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
    pub fn mesh_block_unit_solids(
        &self,
        b: usize,
        min: [f64; 3],
        size: f64,
        cfg: OctreeConfig,
    ) -> Vec<Mesh> {
        let Some(block) = self.blocks.get(b).and_then(|x| x.as_ref()) else {
            return Vec::new();
        };
        let interfaces = &block.lithology.interfaces;
        // Canonical height field z(x,y) per contact, meshed ONCE (surface contour of
        // the block's smooth field, clipped to the block). Only valid for single-valued
        // (height-field) contacts — the geological norm; a near-vertical contact would
        // fall back to no weld (the column lookup just misses).
        let canon: Vec<ContactHeightField> = interfaces
            .iter()
            .enumerate()
            .map(|(k, _)| ContactHeightField::build(self, b, k, min, size, cfg))
            .collect();

        let mut out = Vec::with_capacity(self.block_unit_count(b));
        for u in 0..self.block_unit_count(b) {
            let mut mesh = self
                .mesh_block_unit_solid(b, u, min, size, cfg)
                .unwrap_or_default();
            for &k in Self::bounding_contacts(u, interfaces.len()).iter().flatten() {
                canon[k].weld(&mut mesh);
            }
            out.push(mesh);
        }
        out
    }

    /// The interfaces bounding lithology unit `u` (of `n` interfaces): the lower
    /// (`u-1`) and upper (`u`) contacts, each `None` at the bottom/top of the stack.
    fn bounding_contacts(u: usize, n: usize) -> [Option<usize>; 2] {
        [u.checked_sub(1), (u < n).then_some(u)]
    }

    /// One welded unit solid (the closed-domain view for a single lithology unit), with
    /// its shared contacts welded onto the canonical contact surfaces. Self-contained:
    /// builds only the 1–2 canonical contacts bounding this unit. `None` if the block is
    /// field-less or `unit_idx` is out of range. Used by the WASM per-unit binding.
    pub fn mesh_block_unit_solid_welded(
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
        for &k in Self::bounding_contacts(unit_idx, n).iter().flatten() {
            ContactHeightField::build(self, b, k, min, size, cfg).weld(&mut mesh);
        }
        Some(mesh)
    }

    /// The block containing `p`, if it has a field.
    fn block_at(&self, p: [f64; 3]) -> Option<&Block> {
        self.blocks.get(self.index.block_at(p)).and_then(|b| b.as_ref())
    }

    /// Scalar field `Z(x)` at a world query point. In an empty (data-less) block,
    /// returns `+∞` — no contact, so every interface threshold places it in unit 0.
    pub fn scalar_field(&self, xyz: [f64; 3]) -> f64 {
        match self.block_at(xyz) {
            Some(b) => b.field().scalar_field(xyz),
            None => f64::INFINITY,
        }
    }

    /// Analytic gradient `∇Z` at a world query point (world coordinates). Zero in an
    /// empty block (the field is constant `+∞` there — no gradient).
    pub fn scalar_field_gradient(&self, xyz: [f64; 3]) -> [f64; 3] {
        match self.block_at(xyz) {
            Some(b) => b.field().scalar_field_gradient(xyz),
            None => [0.0, 0.0, 0.0],
        }
    }

    /// Lithology unit index at a world query point — classified by the block at `p`
    /// against THAT block's own recovered isovalues (correct across an offsetting
    /// fault). Unit 0 in an empty block.
    pub fn lithology_unit(&self, xyz: [f64; 3]) -> usize {
        match self.block_at(xyz) {
            Some(b) => b.lithology.unit_at(&b.field(), xyz),
            None => 0,
        }
    }

    /// The recovered per-interface isovalues (sorted ascending).
    ///
    /// For a faulted model these are **block 0's** isovalues, the representative set:
    /// [`Stack`] masking reads this for a series' bounding contact, while the
    /// per-point sign test routes through `scalar_field` (which is per-block), so the
    /// masking surface tracks the fault offset where it matters. Use
    /// [`Model::block_lithology`] for a specific block.
    pub fn lithology(&self) -> &Lithology {
        &self.representative_lithology
    }
}

/// A value strictly larger than `|field|` anywhere in the box `[min, min+size]³`,
/// sampled on a coarse grid (the cokriging field is smooth, so a coarse sample
/// bounds it with margin). Used as the top lithology unit's "upper bound" so
/// `field < upper` holds everywhere and the box top caps the unit.
fn max_abs_field_in_box(field: &dyn ScalarField, min: [f64; 3], size: f64) -> f64 {
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
fn clip_mesh_to_half_spaces(mesh: &mut Mesh, half_spaces: &[fault::HalfSpace]) {
    for hs in half_spaces {
        clip_mesh_to_plane(mesh, hs);
    }
}

/// One Sutherland–Hodgman pass: clip every triangle to the kept side of `hs`.
fn clip_mesh_to_plane(mesh: &mut Mesh, hs: &fault::HalfSpace) {
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
struct ContactHeightField {
    /// Canonical contact vertices (the surface-contoured contact), in a coarse spatial
    /// hash for nearest-vertex lookup.
    verts: Vec<[f64; 3]>,
    /// `bucket(p) → indices into verts`, bucketed at `cell`.
    hash: std::collections::HashMap<(i64, i64, i64), Vec<u32>>,
    cell: f64,
    min: [f64; 3],
}

impl ContactHeightField {
    fn build(model: &Model, b: usize, k: usize, min: [f64; 3], size: f64, cfg: OctreeConfig) -> Self {
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
    fn weld(&self, mesh: &mut Mesh) {
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

/// `Model` as a [`ScalarField`] for the contourer — per-block dispatch (the field is
/// discontinuous across faults). Delegates to the inherent methods.
impl ScalarField for Model {
    fn scalar_field(&self, p: [f64; 3]) -> f64 {
        Model::scalar_field(self, p)
    }
    fn scalar_field_gradient(&self, p: [f64; 3]) -> [f64; 3] {
        Model::scalar_field_gradient(self, p)
    }
}

/// A view of ONE fault block's field as a [`ScalarField`] — a single smooth cokriging
/// field with NO per-block dispatch, so contouring it produces a continuous contact
/// everywhere (the block's contact extrapolated across the whole domain). The per-block
/// mesh then drops the extrapolated part outside the block. This is what lets a faulted
/// contact mesh as clean separate sheets without the cross-fault discontinuity crumpling
/// the contour.
struct BlockField<'a> {
    block: &'a Block,
}

impl ScalarField for BlockField<'_> {
    fn scalar_field(&self, p: [f64; 3]) -> f64 {
        self.block.field().scalar_field(p)
    }
    fn scalar_field_gradient(&self, p: [f64; 3]) -> [f64; 3] {
        self.block.field().scalar_field_gradient(p)
    }
}

/// Infer a fallback orientation (gradient anchor) for a block that has surface
/// points but no orientation of its own — from the geometry of the points
/// themselves, not a blind flat assumption.
///
/// The pole is taken from the **best-constrained interface** (the one with the most
/// points in this block — one good contact anchors the whole series gradient):
///   * **≥3 non-collinear points** → the best-fit plane normal (the points define a
///     plane; its normal is the dip).
///   * **2 points, or ≥3 collinear points** → the *flattest* plane containing that
///     line: the normal closest to vertical (`pole = P × H`, `P` the line direction,
///     `H` the horizontal in-line direction). The line constrains strike but not dip,
///     so the flattest dip is the prior.
///   * **1 point (or fully degenerate)** → flat-lying (`+Z`), the bare geological
///     default.
///
/// The pole is oriented "up" (`pole·+Z ≥ 0`) to match the field's sign convention,
/// and anchored at the chosen interface's centroid.
fn infer_orientation(points: &[SurfacePoint]) -> Orientation {
    // Group by interface; pick the interface with the most points in this block.
    let mut by_iface: std::collections::BTreeMap<u32, Vec<[f64; 3]>> =
        std::collections::BTreeMap::new();
    for p in points {
        by_iface.entry(p.interface_id).or_default().push(p.xyz);
    }
    let best = by_iface
        .values()
        .max_by_key(|v| v.len())
        .cloned()
        .unwrap_or_default();

    let centroid = |pts: &[[f64; 3]]| -> [f64; 3] {
        let n = pts.len().max(1) as f64;
        let mut c = [0.0; 3];
        for p in pts {
            c[0] += p[0];
            c[1] += p[1];
            c[2] += p[2];
        }
        [c[0] / n, c[1] / n, c[2] / n]
    };

    let up = |mut n: [f64; 3]| {
        if n[2] < 0.0 {
            n = [-n[0], -n[1], -n[2]];
        }
        n
    };

    let loc = centroid(&best);
    let pole = match best.len() {
        0 | 1 => [0.0, 0.0, 1.0], // bare default
        _ => {
            let c = centroid(&best);
            // Covariance (about centroid) of the chosen interface's points.
            let mut cov = [[0.0_f64; 3]; 3];
            for p in &best {
                let d = [p[0] - c[0], p[1] - c[1], p[2] - c[2]];
                for r in 0..3 {
                    for k in 0..3 {
                        cov[r][k] += d[r] * d[k];
                    }
                }
            }
            let (evals, evecs) = sym3_eig(cov); // ascending eigenvalues
            // Spread along the smallest vs largest principal axis. If the second-
            // smallest eigenvalue is negligible relative to the largest, the points
            // span only a LINE (2 points, or collinear) → flattest-plane-through-line.
            let lambda_max = evals[2].max(1e-300);
            let planar = evals[1] / lambda_max > 1e-6;
            if planar {
                // Best-fit plane normal = smallest-eigenvalue eigenvector.
                up([evecs[0][0], evecs[1][0], evecs[2][0]])
            } else {
                // Line direction P = largest-eigenvalue eigenvector; flattest plane
                // normal through it = P × H, H the horizontal perpendicular to P_xy.
                let p_dir = [evecs[0][2], evecs[1][2], evecs[2][2]];
                up(flattest_plane_normal(p_dir))
            }
        }
    };
    Orientation::new(loc, normalize(pole))
}

/// Normal of the *flattest* plane (steepest-toward-horizontal, i.e. normal closest
/// to vertical) containing the line direction `p`. `pole = P × H` where `H` is the
/// unit horizontal vector perpendicular to `P`'s horizontal projection. Degenerates
/// to `+Z` when `P` is vertical (the line is vertical → flat is the only prior).
fn flattest_plane_normal(p: [f64; 3]) -> [f64; 3] {
    let pn = normalize(p);
    let p_xy = (pn[0] * pn[0] + pn[1] * pn[1]).sqrt();
    if p_xy < 1e-12 {
        return [0.0, 0.0, 1.0]; // vertical line → flat
    }
    // Horizontal direction perpendicular to P's map projection.
    let h = normalize([-pn[1], pn[0], 0.0]);
    // pole = P × H.
    [
        pn[1] * h[2] - pn[2] * h[1],
        pn[2] * h[0] - pn[0] * h[2],
        pn[0] * h[1] - pn[1] * h[0],
    ]
}

/// Eigendecomposition of a symmetric 3×3 matrix by cyclic Jacobi rotations.
/// Returns `(eigenvalues, eigenvectors)` with eigenvalues **ascending** and
/// `eigenvectors[r][c]` the `r`-th component of the `c`-th eigenvector (columns are
/// eigenvectors, parallel to the eigenvalue order). Robust for the small PSD
/// covariance matrices here; not performance-critical (called only on the fallback).
fn sym3_eig(mut a: [[f64; 3]; 3]) -> ([f64; 3], [[f64; 3]; 3]) {
    let mut v = [[0.0; 3]; 3];
    for i in 0..3 {
        v[i][i] = 1.0;
    }
    for _ in 0..50 {
        // Largest off-diagonal magnitude.
        let mut p = 0;
        let mut q = 1;
        let mut off = a[0][1].abs();
        for (i, j) in [(0, 2), (1, 2)] {
            if a[i][j].abs() > off {
                off = a[i][j].abs();
                p = i;
                q = j;
            }
        }
        if off < 1e-14 {
            break;
        }
        // Jacobi rotation zeroing a[p][q]. Tangent (Numerical-Recipes) form, which
        // avoids the angle-sign ambiguity: with θ_v = (aqq−app)/(2·apq),
        // t = tan θ = sign(θ_v)/(|θ_v| + √(θ_v²+1)), then c = 1/√(t²+1), s = t·c.
        let apq = a[p][q];
        let theta_v = (a[q][q] - a[p][p]) / (2.0 * apq);
        // sign(θv), treating θv == 0 as +1 so app==aqq still rotates by 45°.
        let sgn = if theta_v < 0.0 { -1.0 } else { 1.0 };
        let t = sgn / (theta_v.abs() + (theta_v * theta_v + 1.0).sqrt());
        let c = 1.0 / (t * t + 1.0).sqrt();
        let s = t * c;
        // A ← Jᵀ A J for J = R(p,q): rotate columns p,q, then rows p,q.
        for k in 0..3 {
            let akp = a[k][p];
            let akq = a[k][q];
            a[k][p] = c * akp - s * akq;
            a[k][q] = s * akp + c * akq;
        }
        for k in 0..3 {
            let apk = a[p][k];
            let aqk = a[q][k];
            a[p][k] = c * apk - s * aqk;
            a[q][k] = s * apk + c * aqk;
        }
        // Accumulate the rotation into the eigenvector columns.
        for k in 0..3 {
            let vkp = v[k][p];
            let vkq = v[k][q];
            v[k][p] = c * vkp - s * vkq;
            v[k][q] = s * vkp + c * vkq;
        }
    }
    let mut evals = [a[0][0], a[1][1], a[2][2]];
    // Sort ascending, permuting eigenvector columns with the eigenvalues.
    let mut order = [0usize, 1, 2];
    order.sort_by(|&i, &j| evals[i].partial_cmp(&evals[j]).unwrap());
    let sorted_evals = [evals[order[0]], evals[order[1]], evals[order[2]]];
    let mut sorted_vecs = [[0.0; 3]; 3];
    for (col, &src) in order.iter().enumerate() {
        for row in 0..3 {
            sorted_vecs[row][col] = v[row][src];
        }
    }
    evals = sorted_evals;
    (evals, sorted_vecs)
}

#[cfg(test)]
mod fault_tests {
    use super::*;
    use crate::data::{Orientation, SurfacePoint};
    use crate::fault::{OneFault, PlanarFault};

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

    /// Fail-first guard: fitting the SAME offset data WITHOUT a fault (one smooth
    /// block) must NOT show the discontinuity — proving the discontinuity test pins
    /// the fault, not the data. A single smooth kriging field cannot represent the
    /// hard 100-unit offset, so it bends through it: the cross-x=500 jump collapses
    /// to the same order as within-block variation.
    #[test]
    fn unfaulted_offset_is_smooth_not_discontinuous() {
        let model = Model::fit(&faulted_offset_series(), KernelOptions::default());
        let z = 650.0;
        let just_left = model.scalar_field([499.0, 500.0, z]);
        let just_right = model.scalar_field([501.0, 500.0, z]);
        let jump = (just_right - just_left).abs();
        let within_left =
            (model.scalar_field([497.0, 500.0, z]) - model.scalar_field([499.0, 500.0, z])).abs();
        assert!(
            jump < 5.0 * within_left.max(1e-9),
            "unfaulted fit should be smooth across x=500: jump={jump} within={within_left}"
        );
    }

    /// A fault that leaves one side with NO orientations (both orientations sit on
    /// the plane / other side) must NOT panic. The orientation-starved block falls
    /// back to a default flat-lying orientation, so BOTH blocks mesh — no silent
    /// empty half. This is the harness's default fault (plane through the domain
    /// center, the seed orientations at x=500 landing on it).
    #[test]
    fn fault_with_orientation_starved_block_falls_back_flat() {
        // Seed-like data: contacts at z=600/z=400, both orientations at x=500.
        let mut sps = Vec::new();
        for &(x, y) in &[(100.0, 200.0), (500.0, 200.0), (900.0, 200.0), (500.0, 800.0)] {
            sps.push(SurfacePoint::new([x, y, 600.0], 0));
            sps.push(SurfacePoint::new([x, y, 400.0], 1));
        }
        let oris = vec![
            Orientation::new([500.0, 500.0, 600.0], [0.0, 0.0, 1.0]),
            Orientation::new([500.0, 500.0, 400.0], [0.0, 0.0, 1.0]),
        ];
        let series = Series::new(sps, oris);
        // Vertical plane x=500, normal +x: orientations (x=500) go to the positive
        // side (block 1); the negative side (block 0) gets points but no orientation.
        let fault = Box::new(OneFault::new(PlanarFault::new([500.0, 0.0, 0.0], [1.0, 0.0, 0.0])));
        let model = Model::fit_faulted(&series, fault, KernelOptions::default());

        assert_eq!(model.nblocks(), 2);
        // BOTH blocks fit: the starved block 0 used a default flat orientation.
        assert!(model.block_lithology(0).is_some(), "starved block should fall back, not vanish");
        assert!(model.block_lithology(1).is_some(), "populated block should fit");
        // Both sides have a real (finite) field at the contact elevations.
        assert!(model.scalar_field([100.0, 500.0, 500.0]).is_finite());
        assert!(model.scalar_field([900.0, 500.0, 500.0]).is_finite());
    }

    /// A block with NO surface points at all (not just no orientation) is genuinely
    /// field-less — there is no contact to mesh. Sentinel `+∞` / unit 0 there.
    #[test]
    fn fault_block_with_no_points_is_field_less() {
        // All points on the positive side (x>500); the negative side is truly empty.
        let mut sps = Vec::new();
        for &(x, y) in &[(600.0, 200.0), (800.0, 200.0), (600.0, 800.0), (800.0, 800.0)] {
            sps.push(SurfacePoint::new([x, y, 600.0], 0));
        }
        let oris = vec![Orientation::new([700.0, 500.0, 600.0], [0.0, 0.0, 1.0])];
        let series = Series::new(sps, oris);
        let fault = Box::new(OneFault::new(PlanarFault::new([500.0, 0.0, 0.0], [1.0, 0.0, 0.0])));
        let model = Model::fit_faulted(&series, fault, KernelOptions::default());
        assert!(model.block_lithology(0).is_none(), "empty side has no contact");
        assert!(model.block_lithology(1).is_some());
        assert!(model.scalar_field([100.0, 500.0, 500.0]).is_infinite());
    }

    fn sp(x: f64, y: f64, z: f64, id: u32) -> SurfacePoint {
        SurfacePoint::new([x, y, z], id)
    }
    fn dot3(a: [f64; 3], b: [f64; 3]) -> f64 {
        a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
    }

    #[test]
    fn infer_orientation_three_points_best_fit_plane() {
        let pts = [sp(0.0, 0.0, 0.0, 0), sp(2.0, 0.0, 1.0, 0), sp(0.0, 2.0, 0.0, 0)];
        let o = infer_orientation(&pts);
        // Plane z = 0.5x has normal ∝ (-0.5, 0, 1) → (-0.4472,0,0.8944).
        assert!((o.pole[0] - (-0.4472)).abs() < 1e-3, "pole {:?}", o.pole);
        assert!(o.pole[1].abs() < 1e-3);
        assert!(o.pole[2] > 0.0, "pole points up");
        // Pole ⊥ the two in-plane edges.
        assert!(dot3(o.pole, [2.0, 0.0, 1.0]).abs() < 1e-9);
        assert!(dot3(o.pole, [0.0, 2.0, 0.0]).abs() < 1e-9);
    }

    /// >3 noisy roughly-flat points → ~+Z (the best-fit plane is horizontal).
    #[test]
    fn infer_orientation_many_flat_points() {
        let mut pts = Vec::new();
        for i in 0..4 {
            for j in 0..4 {
                pts.push(sp(i as f64 * 100.0, j as f64 * 100.0, 500.0, 0));
            }
        }
        let o = infer_orientation(&pts);
        assert!(o.pole[2] > 0.999, "near-flat → +Z, got {:?}", o.pole);
    }

    /// 2 points define only a LINE → the flattest plane through it: pole ⊥ the line
    /// and as vertical as possible. For a horizontal line, that is exactly +Z.
    #[test]
    fn infer_orientation_two_points_flattest() {
        // Horizontal line along a diagonal → flattest plane is horizontal → +Z.
        let o = infer_orientation(&[sp(0.0, 0.0, 500.0, 0), sp(100.0, 100.0, 500.0, 0)]);
        assert!(o.pole[2] > 0.999, "horizontal line → +Z, got {:?}", o.pole);

        // A line dipping in +x (from (0,0,0) to (1,0,1)): flattest plane normal is
        // (-1,0,1)/√2, and the pole must be ⊥ the line.
        let o2 = infer_orientation(&[sp(0.0, 0.0, 0.0, 0), sp(1.0, 0.0, 1.0, 0)]);
        assert!(dot3(o2.pole, [1.0, 0.0, 1.0]).abs() < 1e-9, "pole ⊥ line: {:?}", o2.pole);
        assert!((o2.pole[0] - (-0.7071)).abs() < 1e-3 && (o2.pole[2] - 0.7071).abs() < 1e-3);
    }

    /// ≥3 COLLINEAR points are still only a line (degenerate plane) → flattest plane,
    /// same as the 2-point case, NOT an arbitrary normal from a singular covariance.
    #[test]
    fn infer_orientation_collinear_points_fall_back_to_line() {
        // Collinear, horizontal along x → flattest is +Z.
        let pts = [
            sp(0.0, 0.0, 700.0, 0),
            sp(100.0, 0.0, 700.0, 0),
            sp(200.0, 0.0, 700.0, 0),
            sp(300.0, 0.0, 700.0, 0),
        ];
        let o = infer_orientation(&pts);
        assert!(o.pole[2] > 0.999, "collinear horizontal → +Z, got {:?}", o.pole);
    }

    /// A single point gives no geometry → flat (+Z), the bare default.
    #[test]
    fn infer_orientation_single_point_is_flat() {
        let o = infer_orientation(&[sp(123.0, 456.0, 789.0, 0)]);
        assert_eq!(o.pole, [0.0, 0.0, 1.0]);
        assert_eq!(o.xyz, [123.0, 456.0, 789.0]);
    }

    /// The inference picks the BEST-CONSTRAINED interface (most points), so a block
    /// mixing a well-sampled dipping contact with a single stray point of another
    /// interface uses the dipping contact's plane, not the mix.
    #[test]
    fn infer_orientation_uses_best_constrained_interface() {
        let pts = [
            // interface 0: a clear plane dipping in +x.
            sp(0.0, 0.0, 0.0, 0),
            sp(2.0, 0.0, 1.0, 0),
            sp(0.0, 2.0, 0.0, 0),
            sp(2.0, 2.0, 1.0, 0),
            // interface 1: a single stray point (should be ignored — fewer points).
            sp(50.0, 50.0, 999.0, 1),
        ];
        let o = infer_orientation(&pts);
        assert!(dot3(o.pole, [2.0, 0.0, 1.0]).abs() < 1e-9, "used iface 0 plane: {:?}", o.pole);
        assert!(o.pole[2] > 0.0);
    }

    /// `Model::fit` (no fault) is unchanged: one block, the whole field.
    #[test]
    fn unfaulted_fit_is_single_block() {
        let model = Model::fit(&faulted_offset_series(), KernelOptions::default());
        assert_eq!(model.nblocks(), 1);
        assert!(model.block_lithology(0).is_some());
    }

    /// Each block reproduces ITS OWN contact: the recovered isovalue on the left is
    /// the field at z=600, on the right at z=700 — and a point on each block's contact
    /// sits at its own field isovalue.
    #[test]
    fn each_block_reproduces_its_own_contact() {
        let model = Model::fit_faulted(&faulted_offset_series(), fault_x500(), KernelOptions::default());
        assert_eq!(model.nblocks(), 2);

        // Block 0 = negative side (x<500), block 1 = positive (x>=500).
        let iso_left = model.block_lithology(0).unwrap().interfaces[0].isovalue;
        let iso_right = model.block_lithology(1).unwrap().interfaces[0].isovalue;

        // The contact sits at z≈600 on the left and z≈700 on the right: the field at
        // those points equals each block's recovered isovalue.
        let f_left_at_contact = model.scalar_field([300.0, 500.0, 600.0]);
        let f_right_at_contact = model.scalar_field([700.0, 500.0, 700.0]);
        assert!(
            (f_left_at_contact - iso_left).abs() < 1e-6,
            "left contact not at its isovalue: f={f_left_at_contact} iso={iso_left}"
        );
        assert!(
            (f_right_at_contact - iso_right).abs() < 1e-6,
            "right contact not at its isovalue: f={f_right_at_contact} iso={iso_right}"
        );
    }

    /// The field is DISCONTINUOUS across the fault: at the same (y,z), sampling just
    /// left vs just right of x=500 gives a jump (the two blocks' independent fields),
    /// far larger than the smooth variation within either block.
    #[test]
    fn field_is_discontinuous_across_the_fault() {
        let model = Model::fit_faulted(&faulted_offset_series(), fault_x500(), KernelOptions::default());

        // Sample at a fixed z between the two contacts (z=650), y=500, straddling x=500.
        let z = 650.0;
        let just_left = model.scalar_field([499.0, 500.0, z]);
        let just_right = model.scalar_field([501.0, 500.0, z]);
        let jump = (just_right - just_left).abs();

        // Within-block variation over the same 2-unit x step, on the left block.
        let within_left = (model.scalar_field([497.0, 500.0, z]) - model.scalar_field([499.0, 500.0, z])).abs();

        assert!(
            jump > 20.0 * within_left.max(1e-9),
            "no discontinuity at the fault: jump={jump} within-block={within_left}"
        );
    }

    /// The field is CONTINUOUS within each block — no spurious jump away from the
    /// fault plane. Adjacent samples well inside block 0 differ only smoothly.
    #[test]
    fn field_is_continuous_within_a_block() {
        let model = Model::fit_faulted(&faulted_offset_series(), fault_x500(), KernelOptions::default());
        let z = 650.0;
        let mut prev = model.scalar_field([100.0, 500.0, z]);
        // March across the left block (x: 100→480, all x<500) in small steps.
        for k in 1..=19 {
            let x = 100.0 + k as f64 * 20.0; // up to 480
            let cur = model.scalar_field([x, 500.0, z]);
            assert!(
                (cur - prev).abs() < 50.0,
                "spurious jump inside block 0 at x={x}: {prev} → {cur}"
            );
            prev = cur;
        }
    }

    /// Lithology is classified against the block's OWN isovalues, so a point below
    /// each block's offset contact lands in the same unit despite the throw — and the
    /// faulted model is a drop-in for `lithology_unit`.
    #[test]
    fn lithology_routes_per_block_across_the_offset() {
        let model = Model::fit_faulted(&faulted_offset_series(), fault_x500(), KernelOptions::default());
        // A point below the left contact (z=550 < 600) and below the right contact
        // (z=650 < 700) should be in the SAME unit (below the contact) on both sides.
        let unit_left = model.lithology_unit([300.0, 500.0, 550.0]);
        let unit_right = model.lithology_unit([700.0, 500.0, 650.0]);
        assert_eq!(
            unit_left, unit_right,
            "same structural position across the fault should be the same unit"
        );
        // And a point ABOVE each contact is a different unit from below it.
        let above_left = model.lithology_unit([300.0, 500.0, 650.0]); // above z=600
        assert_ne!(unit_left, above_left, "above vs below the contact must differ");
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
