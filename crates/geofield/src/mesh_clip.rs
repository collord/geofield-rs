//! Mesh–mesh seam corefinement: terminate a clipped surface **C** exactly against a clipping
//! surface **W**, sharing the seam vertices by index so the join is watertight by construction.
//!
//! This is the *keep-one-side* half of a mesh boolean — corefine C against W along their
//! intersection polyline, then discard the side of C we don't keep — NOT a full CSG. It is the
//! principled replacement for the staircased CSG-field-union and the scalloping
//! [`crate::weld::snap_free_boundary_to_mesh`] on the contact-meets-wall case, and it is the path
//! that *imported, field-less* surfaces must use.
//!
//! ## Field-agnostic by construction
//!
//! Inputs and output are plain `(Vec<[f64; 3]>, Vec<[usize; 3]>)` ([`ClipMesh`]). NOTHING here
//! calls [`ScalarField`](geofield_types::ScalarField), `Model`, or any field method — so it works
//! identically for generated and imported meshes. The one optional field touch point is the
//! `refine` hook on [`clip`], which (when `Some`) may nudge a *newly inserted seam vertex* along
//! the already-fixed seam onto a field's zero set for sub-triangle accuracy; it is `None` by
//! default and changes no topology either way.
//!
//! ## Algorithm (mirrors established corefinement, see `docs/mesh_clip_design.md`)
//!
//!   * **A** — for every crossing (C-triangle, W-triangle) pair, the triangle–triangle
//!     intersection **segment** (Möller `compute_intervals_isectline` form).
//!   * **B** — assemble the seam polyline, keying each endpoint by its combinatorial incidence
//!     (which W/C edge or vertex it lies on) so two triangles meeting at a seam point agree on one
//!     shared index — not a spatial-hash weld.
//!   * **C** — re-triangulate each cut face with a 2D constrained Delaunay triangulation in the
//!     face plane (dominant-axis projection + the `cdt` crate, whose exact Shewchuk predicates make
//!     the result native/wasm-identical), with the seam segments as constraints. Interior corners
//!     keep their *bit-exact* input 3D coords (no scalloping). Discard C's sub-triangles on the
//!     non-kept side of W; keep all of W's.
//!   * **D** — emit C and W as two buffers plus a `shared` map pairing the seam vertices that are
//!     the same point in each.

use std::collections::{HashMap, HashSet};

use geofield_types::ScalarField;

/// A triangle mesh as this module sees it: world-space vertices and CCW triangles. Both inputs and
/// the output use this; no field type ever appears.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ClipMesh {
    pub vertices: Vec<[f64; 3]>,
    pub triangles: Vec<[usize; 3]>,
}

impl ClipMesh {
    pub fn new(vertices: Vec<[f64; 3]>, triangles: Vec<[usize; 3]>) -> Self {
        Self { vertices, triangles }
    }

    /// Convert from the crate's GPU-ready flat [`crate::mesh::Mesh`] (positions `f32`, indices
    /// `u32`). Drops normals — `clip` recomputes geometry from positions and the output carries
    /// none. Purely structural; no field involved.
    pub fn from_mesh(m: &crate::mesh::Mesh) -> Self {
        let vertices = (0..m.vertex_count())
            .map(|i| {
                [
                    m.positions[3 * i] as f64,
                    m.positions[3 * i + 1] as f64,
                    m.positions[3 * i + 2] as f64,
                ]
            })
            .collect();
        let triangles = m
            .indices
            .chunks_exact(3)
            .map(|t| [t[0] as usize, t[1] as usize, t[2] as usize])
            .collect();
        Self { vertices, triangles }
    }

    /// Convert to the crate's flat [`crate::mesh::Mesh`] (no normals — a downstream pass fills them
    /// if needed, e.g. face-averaged, since corefinement does not preserve per-vertex normals).
    pub fn to_mesh(&self) -> crate::mesh::Mesh {
        let mut positions = Vec::with_capacity(self.vertices.len() * 3);
        for v in &self.vertices {
            positions.push(v[0] as f32);
            positions.push(v[1] as f32);
            positions.push(v[2] as f32);
        }
        let indices = self
            .triangles
            .iter()
            .flat_map(|t| [t[0] as u32, t[1] as u32, t[2] as u32])
            .collect();
        crate::mesh::Mesh { positions, normals: Vec::new(), indices }
    }

    fn tri(&self, t: usize) -> ([f64; 3], [f64; 3], [f64; 3]) {
        let [i, j, k] = self.triangles[t];
        (self.vertices[i], self.vertices[j], self.vertices[k])
    }

    /// Unnormalized outward (CCW) normal of triangle `t` — `(b−a) × (c−a)`.
    fn tri_normal_raw(&self, t: usize) -> [f64; 3] {
        let (a, b, c) = self.tri(t);
        cross(sub(b, a), sub(c, a))
    }

    /// Winding-normalize an IMPORTED mesh so it satisfies [`clip`]'s `W` precondition: each connected
    /// component is made consistently wound, CLOSED components are oriented outward by volume, and an
    /// OPEN component is left internally consistent (its absolute sense is the caller's
    /// [`KeepSide`]/[`Keep::Predicate`] choice — `clip` only needs consistency). BIT-tight duplicate
    /// vertices are welded and degenerate triangles dropped, so a split-vertex import still
    /// flood-fills. Field-agnostic; delegates to [`crate::mesh::normalize_winding_per_component`].
    /// Our octree output is already consistently wound and does not need this.
    pub fn winding_normalized(self) -> Self {
        let tris: Vec<[u32; 3]> = self
            .triangles
            .iter()
            .map(|t| [t[0] as u32, t[1] as u32, t[2] as u32])
            .collect();
        let (vertices, tris) = crate::mesh::normalize_winding_per_component(&self.vertices, &tris);
        Self {
            vertices,
            triangles: tris.into_iter().map(|t| [t[0] as usize, t[1] as usize, t[2] as usize]).collect(),
        }
    }
}

/// Which side of W to keep for C, signed by W's per-triangle outward (CCW) normal: `Above` keeps
/// the half-space the normal points into (signed distance ≥ 0), `Below` the other.
///
/// Precondition: W is a coherently oriented sheet (all triangles wound consistently). Our octree
/// contourer satisfies this; imported meshes must be winding-normalized BEFORE `clip` (per
/// connected component — a multi-part import may mix normal senses) via
/// [`ClipMesh::winding_normalized`] / [`crate::mesh::normalize_winding_per_component`]. `clip`
/// asserts this in debug builds; it does not repair winding.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeepSide {
    Above,
    Below,
}

/// How [`clip`] decides which side of W to keep for C.
///
/// `Side` is the original single-wall behavior: keep the chosen side of every cutting W plane.
/// `Predicate` lets the caller carry an arbitrary keep rule over a sub-triangle's centroid — used
/// for a vein *band*, whose kept region is the UNION `f < lo || f > hi` (the band exterior), which a
/// single `KeepSide` cannot express (it is a union of two half-spaces, not one). The caller supplies
/// `|p| body_field(p) < lo || body_field(p) > hi`: robust because an off-seam centroid is well clear
/// of the band edges, and `clip` stays field-agnostic (it only calls the closure). An imported,
/// field-less wall band would supply a geometric predicate instead. This is the mesh analog of
/// [`crate::octree::SlabComplement`] — express the band as ONE complement, not two ANDed half-spaces.
#[derive(Clone, Copy)]
pub enum Keep<'a> {
    Side(KeepSide),
    Predicate(&'a dyn Fn([f64; 3]) -> bool),
}

/// Result of [`clip`]: C and W as separate buffers (matching how callers emit them), plus the seam
/// vertices that are the *same point* in each, paired `(c_index, w_index)`.
#[derive(Clone, Debug, Default)]
pub struct ClipResult {
    pub c: ClipMesh,
    pub w: ClipMesh,
    /// One pair per seam vertex: its index in `c.vertices` and in `w.vertices`. Empty when no seam
    /// was created (the empty-seam branches, or a true no-op).
    pub shared: Vec<(usize, usize)>,
    /// How many (C-triangle, W-triangle) pairs were COPLANAR and handled by the 2D overlap-boundary
    /// path (Part A) rather than the transversal segment path. Zero for the common transverse case;
    /// nonzero flags that imported/locally-parallel geometry exercised the coplanar branch — kept
    /// observable so a future regression there is visible, not silent (the old code dropped these).
    pub coplanar_pairs: usize,
}

/// Corefine `c` against `w`, keeping the `keep` side of `w` for `c`. See module docs.
///
/// `refine`: optional field hook. `None` (default, and the only mode imported meshes use) is
/// purely the C∩W polyline; `Some(f)` may nudge a newly inserted seam vertex along the seam onto
/// `f`'s zero set. Off by default; never moves an interior vertex, never changes topology.
pub fn clip(
    c: &ClipMesh,
    w: &ClipMesh,
    keep: Keep,
    refine: Option<&dyn ScalarField>,
) -> ClipResult {
    debug_assert!(
        w_is_consistently_wound(w),
        "W must be winding-normalized before clip — see ClipMesh::winding_normalized"
    );
    let _ = refine; // Milestone 1: hook is accepted but the no-op default is the only path wired.

    // --- Step A: intersection segments per (C-tri, W-tri) crossing pair. ---------------------
    // A transversal crossing yields one segment; a COPLANAR overlap yields the overlap polygon's
    // boundary edges (Part A). For large inputs a uniform AABB grid over W replaces the O(|C|·|W|)
    // sweep (Part B) with BYTE-IDENTICAL output. `coplanar_pairs` is the set of (tc, tw) pairs that
    // were coplanar — the drop-C/keep-W fill rule needs it to tell a coincident C sub-triangle (drop)
    // from one merely side-clipped (keep test).
    let (raw_segments, coplanar_pairs) = intersection_segments(c, w);

    // No crossings: empty seam → branch on which side of W all of C lies (NOT a blanket no-op).
    if raw_segments.is_empty() {
        return empty_seam_result(c, w, keep);
    }

    // --- Step B: canonical seam vertices (shared by combinatorial key). ----------------------
    // Each seam endpoint is an edge–face crossing: one mesh's edge pierces the other mesh's face, so
    // it lies on a definite COMBINATORIAL feature of each side (a vertex, an edge, or — for the side
    // it passes through — that face). Keying on the pair `(C-feature, W-feature)` of GLOBAL indices
    // makes two triangles that compute the *same* crossing agree on one id EXACTLY — no tolerance —
    // so it is immune to the import-extent fragility a position quantum has: a large bounding box
    // makes `extent · 1e-7` coarse relative to a fine local seam feature, merging genuinely distinct
    // crossings (the `combinatorial_key_watertight_under_coarse_extent` test pins this — and note it
    // is EXTENT, not a translation, that coarsens the quantum). The features come from classifying
    // each endpoint within both its C-triangle and its W-triangle (`classify_in_triangle`) —
    // barycentric, hence scale- and translation-invariant, unlike the quantum. The quantum survives
    // ONLY as a guarded fallback for an endpoint interior to BOTH faces (never produced by a real
    // tri–tri segment endpoint, which lies on the piercing edge of at least one side — guard + log).
    let mut seam = SeamBuilder::new(seam_quantum(c, w));
    let segments: Vec<SeamSeg> = raw_segments
        .iter()
        .map(|r| SeamSeg {
            tc: r.tc,
            tw: r.tw,
            a: seam.intern(r.a, classify_in_triangle(c, r.tc, r.a), classify_in_triangle(w, r.tw, r.a)),
            b: seam.intern(r.b, classify_in_triangle(c, r.tc, r.b), classify_in_triangle(w, r.tw, r.b)),
        })
        .collect();
    seam.warn_if_fallback_fired();

    // --- Step C+D: re-triangulate the cut faces and assemble the two output buffers. ---------
    let mut out_c = SideBuilder::from_mesh(c);
    let mut out_w = SideBuilder::from_mesh(w);

    // Group seam segments by the C-triangle / W-triangle they cut.
    let mut by_c: HashMap<usize, Vec<usize>> = HashMap::new();
    let mut by_w: HashMap<usize, Vec<usize>> = HashMap::new();
    for (si, s) in segments.iter().enumerate() {
        by_c.entry(s.tc).or_default().push(si);
        by_w.entry(s.tw).or_default().push(si);
    }

    // C side: cut faces re-triangulated, sub-triangles on the discard side dropped. UNCUT faces are
    // classified whole by their centroid's side of W — kept only on the `keep` side, dropped on the
    // discard side (a face with no seam segment lies entirely on one side). Surviving faces keep
    // their original vertex indices (interior vertices bit-identical to input).
    let mut c_seam_index: HashMap<usize, usize> = HashMap::new(); // seam-id -> index in out_c
    for tc in 0..c.triangles.len() {
        match by_c.get(&tc) {
            None => {
                let (a, b, cc) = c.tri(tc);
                let centroid = [
                    (a[0] + b[0] + cc[0]) / 3.0,
                    (a[1] + b[1] + cc[1]) / 3.0,
                    (a[2] + b[2] + cc[2]) / 3.0,
                ];
                let kept = match keep {
                    Keep::Side(ks) => side_kept(nearest_w_signed(w, centroid), ks),
                    Keep::Predicate(p) => p(centroid),
                };
                if kept {
                    out_c.keep_original(tc);
                }
            }
            Some(segs) => {
                let segs: Vec<&SeamSeg> = segs.iter().map(|&i| &segments[i]).collect();
                retriangulate_c_face(
                    c, w, tc, &segs, &seam, keep, &coplanar_pairs, &mut out_c, &mut c_seam_index,
                );
            }
        }
    }

    // W side: cut faces re-triangulated, ALL sub-triangles kept; uncut faces pass through.
    let mut w_seam_index: HashMap<usize, usize> = HashMap::new(); // seam-id -> index in out_w
    for tw in 0..w.triangles.len() {
        match by_w.get(&tw) {
            None => out_w.keep_original(tw),
            Some(segs) => {
                let segs: Vec<&SeamSeg> = segs.iter().map(|&i| &segments[i]).collect();
                retriangulate_w_face(w, tw, &segs, &seam, &mut out_w, &mut w_seam_index);
            }
        }
    }

    // Pair each seam vertex's index in C with its index in W → the `shared` map.
    let mut shared = Vec::new();
    for (sid, &ci) in &c_seam_index {
        if let Some(&wi) = w_seam_index.get(sid) {
            shared.push((ci, wi));
        }
    }
    shared.sort_unstable();

    // Drop vertices no triangle references (e.g. C corners on the discarded side, left in the buffer
    // after their triangles were removed) and remap indices + the `shared` pairs accordingly, so a
    // consumer never sees a dead vertex.
    let (c_out, c_remap) = compact(out_c.finish());
    let (w_out, w_remap) = compact(out_w.finish());
    let shared = shared
        .into_iter()
        .filter_map(|(ci, wi)| Some((*c_remap.get(&ci)?, *w_remap.get(&wi)?)))
        .collect();

    ClipResult { c: c_out, w: w_out, shared, coplanar_pairs: coplanar_pairs.len() }
}

/// A geometric keep predicate for clipping C against an IMPORTED, field-less SINGLE wall `w`: keep
/// the side selected by `side`, classified by the CLAMPED closest point on `w` (signed by that
/// triangle's CCW normal). This is the field-less analog of the generated path's field predicate
/// `|p| field(p) - iso >= 0`, and the RECOMMENDED way to clip against an import.
///
/// Pass it as `clip(c, w, Keep::Predicate(&keep_side_of_mesh(w, side)), None)`. Equivalent in
/// robustness to `Keep::Side(side)` (both now use the clamped nearest-cutter test — see
/// [`nearest_w_signed_among`]); offered as a `Predicate` so an imported caller can carry the same
/// closure-shaped keep rule the field callers use, and so single-wall and [`keep_outside_band`]
/// imports read alike.
///
/// Assumes `w` is winding-normalized so its CCW normal sense is meaningful — run
/// [`ClipMesh::winding_normalized`] first. Debug-asserts it.
pub fn keep_side_of_mesh(w: &ClipMesh, side: KeepSide) -> impl Fn([f64; 3]) -> bool + '_ {
    debug_assert!(
        w_is_consistently_wound(w),
        "keep_side_of_mesh: W must be winding-normalized — see ClipMesh::winding_normalized"
    );
    move |p| side_kept(nearest_w_signed(w, p), side)
}

/// A geometric keep predicate for an IMPORTED, field-less vein/shell BAND bounded by two walls: keep
/// the band EXTERIOR — `p` below `lo_wall` OR above `hi_wall`, where "below"/"above" are each wall's
/// CCW-normal sense (signed `< 0` / `> 0`). A single [`KeepSide`] cannot express this (it is a UNION
/// of two half-spaces, not one); this is the field-less analog of the generated band predicate
/// `|p| field(p) < lo || field(p) > hi` and the mesh form of [`crate::octree::SlabComplement`].
///
/// Feed `clip` the two walls combined into ONE `w` — append `hi_wall`'s vertices/triangles after
/// `lo_wall`'s with a base-index offset, so the disjoint ranges keep it consistently wound and both
/// seams share one vertex space, exactly as the field band path does (`stack.rs` `clip_walls`):
/// `clip(c, &combined, Keep::Predicate(&keep_outside_band(&lo, &hi)), None)`.
///
/// Precondition: each wall is winding-normalized AND the two are CO-ORIENTED — both CCW normals
/// point the SAME way across the band (toward increasing band coordinate), so `lo_wall` signed `< 0`
/// means "below the band" and `hi_wall` signed `> 0` means "above". Debug-asserts each is wound.
pub fn keep_outside_band<'a>(
    lo_wall: &'a ClipMesh,
    hi_wall: &'a ClipMesh,
) -> impl Fn([f64; 3]) -> bool + 'a {
    debug_assert!(
        w_is_consistently_wound(lo_wall),
        "keep_outside_band: lo_wall must be winding-normalized"
    );
    debug_assert!(
        w_is_consistently_wound(hi_wall),
        "keep_outside_band: hi_wall must be winding-normalized"
    );
    move |p| nearest_w_signed(lo_wall, p) < 0.0 || nearest_w_signed(hi_wall, p) > 0.0
}

/// Remove vertices not referenced by any triangle, remapping the triangle indices. Returns the
/// compacted mesh and `old_index -> new_index` for the survivors (so callers can fix side tables).
fn compact(m: ClipMesh) -> (ClipMesh, HashMap<usize, usize>) {
    let mut remap: HashMap<usize, usize> = HashMap::new();
    let mut vertices = Vec::new();
    let mut intern = |old: usize, src: &[[f64; 3]], verts: &mut Vec<[f64; 3]>| -> usize {
        *remap.entry(old).or_insert_with(|| {
            let n = verts.len();
            verts.push(src[old]);
            n
        })
    };
    let triangles = m
        .triangles
        .iter()
        .map(|t| {
            [
                intern(t[0], &m.vertices, &mut vertices),
                intern(t[1], &m.vertices, &mut vertices),
                intern(t[2], &m.vertices, &mut vertices),
            ]
        })
        .collect();
    (ClipMesh { vertices, triangles }, remap)
}

// =============================================================================================
// Step A — triangle–triangle intersection (transversal segment + coplanar overlap), with an
// optional uniform AABB grid over W so the sweep is not unconditionally O(|C|·|W|).
// =============================================================================================

/// Pair count above which Step A builds a uniform AABB grid over W instead of the brute double loop.
/// Below it, the grid's build cost isn't worth it and the simple sweep runs in microseconds. Output
/// is byte-identical either way, so this only trades build overhead against query savings.
const BRUTE_PAIR_THRESHOLD: usize = 1 << 18;

/// Step A driver: every (C-tri, W-tri) intersection as [`RawSegment`]s — transversal crossings and
/// coplanar overlap-boundary edges — plus the set of (tc, tw) pairs that were COPLANAR (the
/// drop-C/keep-W fill rule needs it). For large inputs a uniform AABB grid over W's triangles
/// supplies per-C candidates instead of scanning all of W; the result is BYTE-IDENTICAL to the brute
/// sweep because (a) AABB overlap is a necessary condition for any intersection, so candidates are a
/// superset of the pairs that actually produce segments, and (b) candidates are visited per-tc in
/// ASCENDING tw order — exactly the brute loop's push order and positions.
fn intersection_segments(c: &ClipMesh, w: &ClipMesh) -> (Vec<RawSegment>, HashSet<(usize, usize)>) {
    let use_grid = c.triangles.len().saturating_mul(w.triangles.len()) > BRUTE_PAIR_THRESHOLD;
    intersection_segments_inner(c, w, use_grid)
}

/// The body of [`intersection_segments`] with the grid choice forced — so a test can run both paths
/// on identical input and assert byte-identical `RawSegment`s (the grid is a candidate filter only).
fn intersection_segments_inner(
    c: &ClipMesh,
    w: &ClipMesh,
    use_grid: bool,
) -> (Vec<RawSegment>, HashSet<(usize, usize)>) {
    let grid = use_grid.then(|| WGrid::build(w));
    let mut raw = Vec::new();
    let mut coplanar = HashSet::new();
    let mut candidates = Vec::new();
    for tc in 0..c.triangles.len() {
        let ct = c.tri(tc);
        let nc = c.tri_normal_raw(tc);
        candidates.clear();
        match &grid {
            Some(g) => g.candidates(tri_aabb(ct), &mut candidates), // deduped + ascending
            None => candidates.extend(0..w.triangles.len()),
        }
        for &tw in &candidates {
            let wt = w.tri(tw);
            let nw = w.tri_normal_raw(tw);
            match tri_tri_intersection(ct, nc, wt, nw) {
                TriTri::None => {}
                TriTri::Crossing(a, b) => raw.push(RawSegment { tc, tw, a, b }),
                TriTri::Coplanar(edges) => {
                    coplanar.insert((tc, tw));
                    for (a, b) in edges {
                        raw.push(RawSegment { tc, tw, a, b });
                    }
                }
            }
        }
    }
    (raw, coplanar)
}

/// Axis-aligned bounding box of one triangle, as `(lo, hi)`.
fn tri_aabb(t: ([f64; 3], [f64; 3], [f64; 3])) -> ([f64; 3], [f64; 3]) {
    let mut lo = t.0;
    let mut hi = t.0;
    for p in [t.1, t.2] {
        for d in 0..3 {
            lo[d] = lo[d].min(p[d]);
            hi[d] = hi[d].max(p[d]);
        }
    }
    (lo, hi)
}

/// A uniform spatial hash over W's triangle AABBs: each triangle is bucketed into every integer cell
/// its AABB overlaps, so a C triangle's AABB queries only the W triangles in nearby cells. Cell size
/// is W's mean triangle extent, so a typical triangle spans ~1 cell and a query touches few buckets.
/// Pure candidate filtering — never changes which pairs actually intersect, only which are tested.
struct WGrid {
    origin: [f64; 3],
    inv_cell: f64,
    cells: HashMap<[i64; 3], Vec<usize>>,
}

impl WGrid {
    fn build(w: &ClipMesh) -> Self {
        // Cell size = mean triangle extent (floored to the overall extent / a cap so a flat sheet
        // with tiny extent on one axis can't make cells absurdly small). Guards against an all-zero
        // mesh by falling back to 1.0.
        let mut sum_ext = 0.0;
        let mut origin = [f64::INFINITY; 3];
        for t in 0..w.triangles.len() {
            let (lo, _) = tri_aabb(w.tri(t));
            for d in 0..3 {
                origin[d] = origin[d].min(lo[d]);
            }
            sum_ext += bbox_extent(&[w.tri(t).0, w.tri(t).1, w.tri(t).2]);
        }
        if !origin[0].is_finite() {
            origin = [0.0; 3];
        }
        let n = w.triangles.len().max(1) as f64;
        let mean = sum_ext / n;
        let cell = if mean > 0.0 { mean } else { 1.0 };
        let mut cells: HashMap<[i64; 3], Vec<usize>> = HashMap::new();
        let inv_cell = 1.0 / cell;
        for t in 0..w.triangles.len() {
            let (lo, hi) = tri_aabb(w.tri(t));
            let (cl, ch) = (Self::cell_of(origin, inv_cell, lo), Self::cell_of(origin, inv_cell, hi));
            for x in cl[0]..=ch[0] {
                for y in cl[1]..=ch[1] {
                    for z in cl[2]..=ch[2] {
                        cells.entry([x, y, z]).or_default().push(t);
                    }
                }
            }
        }
        Self { origin, inv_cell, cells }
    }

    #[inline]
    fn cell_of(origin: [f64; 3], inv_cell: f64, p: [f64; 3]) -> [i64; 3] {
        [
            ((p[0] - origin[0]) * inv_cell).floor() as i64,
            ((p[1] - origin[1]) * inv_cell).floor() as i64,
            ((p[2] - origin[2]) * inv_cell).floor() as i64,
        ]
    }

    /// Gather (into `out`) the W triangles whose buckets the query AABB touches, deduped and sorted
    /// ASCENDING — so the caller tests them in the same order the brute loop would.
    fn candidates(&self, aabb: ([f64; 3], [f64; 3]), out: &mut Vec<usize>) {
        let (lo, hi) = aabb;
        let (cl, ch) = (
            Self::cell_of(self.origin, self.inv_cell, lo),
            Self::cell_of(self.origin, self.inv_cell, hi),
        );
        for x in cl[0]..=ch[0] {
            for y in cl[1]..=ch[1] {
                for z in cl[2]..=ch[2] {
                    if let Some(ts) = self.cells.get(&[x, y, z]) {
                        out.extend_from_slice(ts);
                    }
                }
            }
        }
        out.sort_unstable();
        out.dedup();
    }
}

/// What two triangles do where they meet. `n_c`/`n_w` are the UNnormalized face normals.
///
///   * `None` — disjoint, parallel-but-separated, or touching at a point/edge (no area to seam).
///   * `Crossing(a, b)` — the transversal intersection segment (Möller interval-overlap form).
///   * `Coplanar(edges)` — the pair lies in a shared plane and overlaps; the boundary edges of their
///     2D overlap polygon (Part A). Each edge becomes a seam constraint on BOTH faces, so a contact
///     coincident with a wall is split along the overlap rim instead of running silently through it.
enum TriTri {
    None,
    Crossing([f64; 3], [f64; 3]),
    Coplanar(Vec<([f64; 3], [f64; 3])>),
}

fn tri_tri_intersection(
    ct: ([f64; 3], [f64; 3], [f64; 3]),
    n_c: [f64; 3],
    wt: ([f64; 3], [f64; 3], [f64; 3]),
    n_w: [f64; 3],
) -> TriTri {
    let (c0, c1, c2) = ct;
    let (w0, w1, w2) = wt;

    // A degenerate (zero-area) triangle has no well-defined plane — can't classify it; bail.
    let (nc2, nw2) = (dot(n_c, n_c), dot(n_w, n_w));
    if nc2 == 0.0 || nw2 == 0.0 {
        return TriTri::None;
    }

    // Plane of C: n_c·X + d_c = 0. Signed distances (× |n_c|) of W's verts to it.
    let d_c = -dot(n_c, c0);
    let (dw0, dw1, dw2) = (
        snap0(dot(n_c, w0) + d_c),
        snap0(dot(n_c, w1) + d_c),
        snap0(dot(n_c, w2) + d_c),
    );
    // All of W strictly on one side of C's plane → no intersection.
    if dw0 * dw1 > 0.0 && dw0 * dw2 > 0.0 {
        return TriTri::None;
    }

    // Plane of W. Signed distances (× |n_w|) of C's verts to it.
    let d_w = -dot(n_w, w0);
    let (dc0, dc1, dc2) = (
        snap0(dot(n_w, c0) + d_w),
        snap0(dot(n_w, c1) + d_w),
        snap0(dot(n_w, c2) + d_w),
    );
    if dc0 * dc1 > 0.0 && dc0 * dc2 > 0.0 {
        return TriTri::None;
    }

    // Parallel test, SCALE-INVARIANT: |n_c × n_w|² ≤ sin²ε · |n_c|²·|n_w|². The old absolute
    // `dot(dir,dir) ≤ COPLANAR_EPS2` scaled with edge length² (the normals are unnormalized), so it
    // called large genuinely-crossing pairs coplanar and tiny ones never; this ratio is dimensionless.
    let dir = cross(n_c, n_w);
    if dot(dir, dir) <= COPLANAR_SIN2 * nc2 * nw2 {
        // Parallel planes. COINCIDENT (→ coplanar overlap) iff W's verts lie in C's plane, tested
        // scale-relative: |dw_i| (= |n_c| · perp-distance) ≤ |n_c| · ext · eps. Otherwise the planes
        // are parallel but separated → genuinely no intersection.
        let ext = bbox_extent(&[c0, c1, c2, w0, w1, w2]).max(f64::MIN_POSITIVE);
        let tol2 = nc2 * (ext * COPLANAR_DIST_EPS).powi(2);
        let coincident = dw0 * dw0 <= tol2 && dw1 * dw1 <= tol2 && dw2 * dw2 <= tol2;
        if coincident {
            let edges = coplanar_overlap_edges(ct, n_c, wt);
            if !edges.is_empty() {
                return TriTri::Coplanar(edges);
            }
        }
        return TriTri::None;
    }

    // --- transversal crossing: the overlap of the two triangles' intervals along the cross line. ---
    let axis = dominant_axis(dir);
    let (ic, iw) = match (
        tri_interval(axis, (c0, c1, c2), (dc0, dc1, dc2)),
        tri_interval(axis, (w0, w1, w2), (dw0, dw1, dw2)),
    ) {
        (Some(ic), Some(iw)) => (ic, iw),
        _ => return TriTri::None,
    };
    let lo = if ic.lo_s >= iw.lo_s { ic.lo_p } else { iw.lo_p };
    let lo_s = ic.lo_s.max(iw.lo_s);
    let hi = if ic.hi_s <= iw.hi_s { ic.hi_p } else { iw.hi_p };
    let hi_s = ic.hi_s.min(iw.hi_s);
    if hi_s < lo_s {
        return TriTri::None; // intervals don't overlap: planes cross but triangles don't
    }
    if (hi_s - lo_s) <= seg_len_eps(&[c0, c1, c2, w0, w1, w2]) {
        return TriTri::None; // a touch/graze, not a segment — no zero-length constraint
    }
    TriTri::Crossing(lo, hi)
}

/// The boundary edges of the 2D overlap polygon of two COINCIDENT (coplanar) triangles, as 3D
/// segments lying in the shared plane (Part A). Project both triangles to 2D by dropping the shared
/// normal's dominant axis, clip C (subject) against W (convex clip) with a 2D Sutherland–Hodgman
/// pass, and lift each resulting polygon vertex back onto C's plane. Returns the consecutive vertex
/// pairs (a closed loop); empty when the overlap is empty or degenerate (a touch/sliver, no area).
///
/// Each output vertex is a retained C vertex, a contained W vertex, or a C-edge×W-edge crossing — so
/// it lands on an edge/vertex feature of at least one side and the combinatorial seam key
/// ([`classify_in_triangle`]) interns it exactly, never hitting the `Face`/`Face` quantum fallback.
fn coplanar_overlap_edges(
    ct: ([f64; 3], [f64; 3], [f64; 3]),
    n: [f64; 3],
    wt: ([f64; 3], [f64; 3], [f64; 3]),
) -> Vec<([f64; 3], [f64; 3])> {
    let drop = dominant_axis(n);
    let (u, v) = other_axes(drop);
    let to2 = |p: [f64; 3]| (p[u], p[v]);
    let subject = vec![to2(ct.0), to2(ct.1), to2(ct.2)];
    let mut clip = vec![to2(wt.0), to2(wt.1), to2(wt.2)];
    // The half-plane "inside" test needs the clip triangle wound CCW in this projection; dropping an
    // axis can mirror it, so orient by signed area.
    if signed_area2(clip[0], clip[1], clip[2]) < 0.0 {
        clip.reverse();
    }
    let poly = clip_convex_2d(&subject, &clip);
    if poly.len() < 3 {
        return Vec::new();
    }
    // Reject a degenerate (zero-area) overlap — a touch along an edge, not a real coincident region.
    let mut area2 = 0.0;
    for i in 1..poly.len() - 1 {
        area2 += signed_area2(poly[0], poly[i], poly[i + 1]);
    }
    let scale = bbox_extent(&[ct.0, ct.1, ct.2, wt.0, wt.1, wt.2]).max(f64::MIN_POSITIVE);
    if area2.abs() <= (scale * COPLANAR_DIST_EPS).powi(2) {
        return Vec::new();
    }
    // Lift a 2D point back onto C's plane n·X = n·c0 (the dropped axis solved from the plane eqn;
    // n[drop] is the dominant component, hence nonzero).
    let plane_d = dot(n, ct.0);
    let lift = |(uu, vv): (f64, f64)| -> [f64; 3] {
        let mut p = [0.0; 3];
        p[u] = uu;
        p[v] = vv;
        p[drop] = (plane_d - n[u] * uu - n[v] * vv) / n[drop];
        p
    };
    let pts3: Vec<[f64; 3]> = poly.iter().map(|&q| lift(q)).collect();
    let mut edges = Vec::with_capacity(pts3.len());
    for i in 0..pts3.len() {
        let a = pts3[i];
        let b = pts3[(i + 1) % pts3.len()];
        if sub(b, a) != [0.0, 0.0, 0.0] {
            edges.push((a, b));
        }
    }
    edges
}

/// Clip convex polygon `subject` against convex polygon `clip` (both 2D, `clip` wound CCW) — the
/// Sutherland–Hodgman algorithm, the 2D analog of the per-triangle half-space clip in `lib.rs`.
/// Returns the convex intersection polygon's vertices in order (empty if they don't overlap).
fn clip_convex_2d(subject: &[(f64, f64)], clip: &[(f64, f64)]) -> Vec<(f64, f64)> {
    let mut output = subject.to_vec();
    for i in 0..clip.len() {
        if output.is_empty() {
            break;
        }
        let (ax, ay) = clip[i];
        let (bx, by) = clip[(i + 1) % clip.len()];
        // Inside = left of the directed clip edge a→b (CCW clip ⇒ interior is on the left).
        let inside = |p: (f64, f64)| (bx - ax) * (p.1 - ay) - (by - ay) * (p.0 - ax) >= 0.0;
        let intersect = |p: (f64, f64), q: (f64, f64)| -> (f64, f64) {
            let dp = (q.0 - p.0, q.1 - p.1);
            let num = (bx - ax) * (p.1 - ay) - (by - ay) * (p.0 - ax);
            let den = (by - ay) * dp.0 - (bx - ax) * dp.1;
            let t = if den != 0.0 { num / den } else { 0.0 };
            (p.0 + dp.0 * t, p.1 + dp.1 * t)
        };
        let input = std::mem::take(&mut output);
        for j in 0..input.len() {
            let cur = input[j];
            let prev = input[(j + input.len() - 1) % input.len()];
            let (cur_in, prev_in) = (inside(cur), inside(prev));
            if cur_in {
                if !prev_in {
                    output.push(intersect(prev, cur));
                }
                output.push(cur);
            } else if prev_in {
                output.push(intersect(prev, cur));
            }
        }
    }
    output
}

/// A triangle's intersection interval with the other plane: the parametric extent along the line's
/// dominant axis, plus the 3D points at the lo/hi ends. `d*` are the signed distances of the
/// triangle's verts to the other plane (already `snap0`'d).
struct Interval {
    lo_s: f64,
    hi_s: f64,
    lo_p: [f64; 3],
    hi_p: [f64; 3],
}

fn tri_interval(
    axis: usize,
    v: ([f64; 3], [f64; 3], [f64; 3]),
    d: (f64, f64, f64),
) -> Option<Interval> {
    let verts = [v.0, v.1, v.2];
    let dist = [d.0, d.1, d.2];
    // The lone vertex on its own side of the plane; the two edges from it straddle.
    let lone = lone_vertex(dist)?;
    let (i, j, k) = (lone, (lone + 1) % 3, (lone + 2) % 3);

    // Crossing point on edge i→j and i→k (signed distances have opposite sign across each).
    let pj = edge_cross(verts[i], verts[j], dist[i], dist[j]);
    let pk = edge_cross(verts[i], verts[k], dist[i], dist[k]);
    let (sj, sk) = (pj[axis], pk[axis]);

    let (lo_s, lo_p, hi_s, hi_p) = if sj <= sk { (sj, pj, sk, pk) } else { (sk, pk, sj, pj) };
    Some(Interval { lo_s, hi_s, lo_p, hi_p })
}

/// Index of the vertex whose signed distance is alone on one side of the plane (the other two are
/// on the opposite side or on it). Returns `None` only for the all-zero (coplanar) case, which the
/// caller has already excluded.
fn lone_vertex(d: [f64; 3]) -> Option<usize> {
    // Count strict signs; the minority (or the single nonzero straddler) is the apex.
    let pos = d.iter().filter(|&&x| x > 0.0).count();
    let neg = d.iter().filter(|&&x| x < 0.0).count();
    if pos == 0 && neg == 0 {
        return None;
    }
    // The lone vertex is the one whose sign differs from the other two. With a zero present
    // (vertex on the plane) the two nonzero ones share a sign; the zero vertex pairs with either.
    for i in 0..3 {
        let (j, k) = ((i + 1) % 3, (i + 2) % 3);
        let si = d[i].signum() as i32 * (d[i] != 0.0) as i32;
        let sj = d[j].signum() as i32 * (d[j] != 0.0) as i32;
        let sk = d[k].signum() as i32 * (d[k] != 0.0) as i32;
        // i is lone if both others share a (nonzero) sign opposite to i, treating 0 as "either".
        if sj == sk && sj != 0 && si != sj {
            return Some(i);
        }
    }
    // Fallback (a vertex exactly on the plane): pick any nonzero vertex as apex.
    (0..3).find(|&i| d[i] != 0.0)
}

/// Point where segment a→b crosses the plane, given the verts' signed distances (opposite signs).
fn edge_cross(a: [f64; 3], b: [f64; 3], da: f64, db: f64) -> [f64; 3] {
    let t = da / (da - db);
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t]
}

// =============================================================================================
// Step B — canonical seam vertices.
// =============================================================================================

#[derive(Clone, Copy, Debug, PartialEq)]
struct RawSegment {
    tc: usize,
    tw: usize,
    a: [f64; 3],
    b: [f64; 3],
}

/// A seam segment after interning its endpoints to canonical seam-vertex ids.
struct SeamSeg {
    tc: usize,
    tw: usize,
    a: usize,
    b: usize,
}

/// The lowest-dimensional feature of one mesh that a seam endpoint lies on, in that mesh's GLOBAL
/// indices: a corner `Vertex`, an `Edge` (canonical unordered vertex-index pair), or the `Face`
/// interior. A tri–tri segment endpoint is an edge–face crossing, so it is `Edge`/`Vertex` on the
/// piercing side and (generically) `Face` on the side it passes through. The combinatorial seam key
/// is the pair `(C-feature, W-feature)`: two computations of the SAME crossing — e.g. the two
/// triangles sharing the piercing edge — derive the same pair and thus one shared id, exactly.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Feature {
    Vertex(usize),
    Edge(usize, usize),
    Face(usize),
}

impl Feature {
    /// An edge feature with its endpoints ordered canonically, so two triangles sharing the edge key
    /// it identically regardless of their local winding.
    fn edge(a: usize, b: usize) -> Self {
        Feature::Edge(a.min(b), a.max(b))
    }
}

/// Classify seam endpoint `p` against triangle `t` of mesh `m`: the [`Feature`] of `t` it lies on,
/// in `m`'s global indices. `p` is a seam endpoint, so it lies in `t`'s plane within `t`. Uses
/// NORMALIZED barycentric coordinates — dimensionless ratios of coordinate *differences*, hence
/// invariant to a large scene offset or scale (the property the position quantum lacked, and the
/// whole point of combinatorial keying). The `Edge`/`Vertex` collapse near a boundary is what makes
/// a point on a shared edge or at a shared vertex key identically from either incident triangle; an
/// interior point is `Face(t)`. The canonicalization lives here and is reused on the C and W sides.
fn classify_in_triangle(m: &ClipMesh, t: usize, p: [f64; 3]) -> Feature {
    let [ia, ib, ic] = m.triangles[t];
    let (a, b, c) = (m.vertices[ia], m.vertices[ib], m.vertices[ic]);
    // Barycentric weights of `p` w.r.t. (a, b, c) — Ericson's projected form, robust in the
    // triangle's own plane. A near-degenerate triangle (≈0 denominator) can't be classified → treat
    // `p` as interior (`Face`), letting the OTHER side's edge/vertex feature carry the key.
    let (e0, e1, e2) = (sub(b, a), sub(c, a), sub(p, a));
    let (d00, d01, d11) = (dot(e0, e0), dot(e0, e1), dot(e1, e1));
    let (d20, d21) = (dot(e2, e0), dot(e2, e1));
    let den = d00 * d11 - d01 * d01;
    if den.abs() <= COPLANAR_EPS2 {
        return Feature::Face(t);
    }
    let lb = (d11 * d20 - d01 * d21) / den;
    let lc = (d00 * d21 - d01 * d20) / den;
    let la = 1.0 - lb - lc;
    let near = |x: f64| x.abs() < SEAM_BARY_EPS;
    // Two weights ≈ 0 → at the third vertex; one weight ≈ 0 → on the edge opposite that vertex.
    match (near(la), near(lb), near(lc)) {
        (false, true, true) => Feature::Vertex(ia),
        (true, false, true) => Feature::Vertex(ib),
        (true, true, false) => Feature::Vertex(ic),
        (true, _, _) => Feature::edge(ib, ic),
        (_, true, _) => Feature::edge(ia, ic),
        (_, _, true) => Feature::edge(ia, ib),
        _ => Feature::Face(t),
    }
}

/// Interns seam endpoints to a stable id keyed by their COMBINATORIAL incidence — the `(C-feature,
/// W-feature)` pair of GLOBAL indices, exact and tolerance-free. The STORED position is the first
/// one seen for that key; two computations of the same crossing share its id and may differ in the
/// last ulp without consequence (the key, not the position, decides identity). The quantized weld is
/// retained ONLY as a guarded fallback for the degenerate endpoint interior to BOTH faces — which a
/// real tri–tri segment endpoint never is, since it lies on a piercing edge — and `fallback_hits`
/// records whether it ever fired, so the combinatorial key's completeness stays observable.
struct SeamBuilder {
    quantum: f64,
    map: HashMap<(Feature, Feature), usize>,
    fallback: HashMap<[i64; 3], usize>,
    fallback_hits: usize,
    points: Vec<[f64; 3]>,
}

impl SeamBuilder {
    fn new(quantum: f64) -> Self {
        Self {
            quantum,
            map: HashMap::new(),
            fallback: HashMap::new(),
            fallback_hits: 0,
            points: Vec::new(),
        }
    }
    fn intern(&mut self, p: [f64; 3], c_feat: Feature, w_feat: Feature) -> usize {
        // Interior to BOTH faces: no edge/vertex to key on, so fall back to the quantized weld. A
        // real tri–tri segment endpoint is always on the piercing edge of one side, so this is a
        // guard, not an expected path; `warn_if_fallback_fired` surfaces it if that ever breaks.
        if let (Feature::Face(_), Feature::Face(_)) = (c_feat, w_feat) {
            self.fallback_hits += 1;
            let k = [
                (p[0] / self.quantum).round() as i64,
                (p[1] / self.quantum).round() as i64,
                (p[2] / self.quantum).round() as i64,
            ];
            if let Some(&id) = self.fallback.get(&k) {
                return id;
            }
            let id = self.points.len();
            self.points.push(p);
            self.fallback.insert(k, id);
            return id;
        }
        let key = (c_feat, w_feat);
        if let Some(&id) = self.map.get(&key) {
            return id;
        }
        let id = self.points.len();
        self.points.push(p);
        self.map.insert(key, id);
        id
    }
    fn pos(&self, id: usize) -> [f64; 3] {
        self.points[id]
    }
    /// Log once if any endpoint fell back to the quantized weld — a signal that the combinatorial key
    /// missed a case (a real seam endpoint should always be on a piercing edge). Quiet otherwise.
    fn warn_if_fallback_fired(&self) {
        if self.fallback_hits > 0 {
            eprintln!(
                "[mesh_clip] seam keying fell back to the quantized weld for {} endpoint(s) \
                 interior to both faces — combinatorial key incomplete for this input",
                self.fallback_hits
            );
        }
    }
}

// =============================================================================================
// Step C — per-cut-face re-triangulation, and Step D buffer assembly.
// =============================================================================================

/// Accumulates one output side (C or W): starts as a copy of the input vertices, appends seam
/// vertices, and collects the kept triangles. Original vertex indices are preserved so untouched
/// interior triangles stay bit-identical.
struct SideBuilder {
    vertices: Vec<[f64; 3]>,
    triangles: Vec<[usize; 3]>,
    orig: ClipMesh,
}

impl SideBuilder {
    fn from_mesh(m: &ClipMesh) -> Self {
        Self { vertices: m.vertices.clone(), triangles: Vec::new(), orig: m.clone() }
    }
    fn keep_original(&mut self, t: usize) {
        self.triangles.push(self.orig.triangles[t]);
    }
    /// Append a vertex, returning its index. Used for seam vertices (interior corners reuse their
    /// original index via `orig`).
    fn push_vertex(&mut self, p: [f64; 3]) -> usize {
        let i = self.vertices.len();
        self.vertices.push(p);
        i
    }
    fn push_tri(&mut self, a: usize, b: usize, c: usize) {
        self.triangles.push([a, b, c]);
    }
    fn finish(self) -> ClipMesh {
        ClipMesh { vertices: self.vertices, triangles: self.triangles }
    }
}

/// Re-triangulate C's cut triangle `tc`, keeping only the sub-triangles on the `keep` side of W.
/// Original corners reuse their input indices (bit-exact); seam endpoints are looked up / appended
/// once per seam-id into `c_seam_index`. `coplanar_pairs` are the (tc, tw) pairs found COPLANAR — a
/// sub-triangle COINCIDENT with such a cutter is dropped (W is authoritative there; see `clip` docs).
fn retriangulate_c_face(
    c: &ClipMesh,
    w: &ClipMesh,
    tc: usize,
    segs: &[&SeamSeg],
    seam: &SeamBuilder,
    keep: Keep,
    coplanar_pairs: &HashSet<(usize, usize)>,
    out: &mut SideBuilder,
    c_seam_index: &mut HashMap<usize, usize>,
) {
    let (pts3, local_kind, constraints) = build_face_cdt_input(c, tc, segs, seam);
    let normal = c.tri_normal_raw(tc);
    let sub = cdt_face(&pts3, &constraints, normal);

    // The W triangles that cut THIS face, split by HOW they cut it. TRANSVERSAL cutters sign the
    // keep side (a sub-triangle is signed by the NEAREST of these — clamped closest point, NOT
    // `.all()` of their infinite planes, which carved a wedge against a curved W; see
    // `nearest_w_signed_among`). COPLANAR cutters instead mark a coincident region: a sub-triangle
    // whose centroid projects inside one is dropped, because that part of C lies exactly on W and W
    // is kept whole — keeping C too would double the surface (the "drop C, keep W" rule).
    // `Keep::Predicate` ignores the transversal set — it classifies each centroid directly — but the
    // coincident drop still applies (W owns the shared plane regardless of the keep rule).
    let mut transversal_cut: Vec<usize> = Vec::new();
    let mut coplanar_cut: Vec<usize> = Vec::new();
    let mut seen = HashSet::new();
    for s in segs {
        if !seen.insert(s.tw) {
            continue;
        }
        if coplanar_pairs.contains(&(tc, s.tw)) {
            coplanar_cut.push(s.tw);
        } else {
            transversal_cut.push(s.tw);
        }
    }

    // Emit only kept sub-triangles; allocate a seam vertex's out-index LAZILY, the first time a
    // KEPT sub-triangle references it — so a seam vertex whose sub-triangles all fell on the discard
    // side is never appended (no orphaned, unreferenced seam vertex).
    for (i, j, k) in sub {
        let centroid = [
            (pts3[i][0] + pts3[j][0] + pts3[k][0]) / 3.0,
            (pts3[i][1] + pts3[j][1] + pts3[k][1]) / 3.0,
            (pts3[i][2] + pts3[j][2] + pts3[k][2]) / 3.0,
        ];
        // Coincident with a coplanar cutter (centroid projects inside it) → drop (W authoritative).
        if coplanar_cut.iter().any(|&tw| point_in_tri_2d(centroid, w.tri(tw), normal)) {
            continue;
        }
        let kept = match keep {
            // Side: nearest TRANSVERSAL cutter; if this face has only coplanar cutters (a fully
            // coincident face), the surviving ring lies past W's extent — fall back to global nearest
            // W, where an in-plane outer-ring centroid signs ≈0 and the inclusive ≥/≤ test keeps it.
            Keep::Side(ks) => {
                let signed = if transversal_cut.is_empty() {
                    nearest_w_signed(w, centroid)
                } else {
                    nearest_w_signed_among(w, transversal_cut.iter().copied(), centroid)
                };
                side_kept(signed, ks)
            }
            Keep::Predicate(p) => p(centroid),
        };
        if !kept {
            continue;
        }
        let mut map = |li: usize, out: &mut SideBuilder| -> usize {
            match local_kind[li] {
                LocalVtx::Corner(orig) => orig,
                LocalVtx::Seam(sid) => *c_seam_index
                    .entry(sid)
                    .or_insert_with(|| out.push_vertex(seam.pos(sid))),
            }
        };
        let (oi, oj, ok) = (map(i, out), map(j, out), map(k, out));
        out.push_tri(oi, oj, ok);
    }
}

/// What a local CDT-input vertex maps back to: an original mesh vertex (reuse its index) or a seam
/// vertex (allocate an out-index lazily, shared by id across faces).
#[derive(Clone, Copy)]
enum LocalVtx {
    Corner(usize),
    Seam(usize),
}

/// Assemble the CDT input for one cut face: the 3 corner positions followed by the seam endpoints
/// (deduped by seam id), the per-local-vertex back-mapping, and the constraint edges (as local
/// index pairs). Shared by the C and W sides.
fn build_face_cdt_input(
    m: &ClipMesh,
    t: usize,
    segs: &[&SeamSeg],
    seam: &SeamBuilder,
) -> (Vec<[f64; 3]>, Vec<LocalVtx>, Vec<(usize, usize)>) {
    let corners = m.triangles[t];
    let cp = [m.vertices[corners[0]], m.vertices[corners[1]], m.vertices[corners[2]]];
    let mut pts3 = vec![cp[0], cp[1], cp[2]];
    let mut kind = vec![
        LocalVtx::Corner(corners[0]),
        LocalVtx::Corner(corners[1]),
        LocalVtx::Corner(corners[2]),
    ];
    let mut seam_local: HashMap<usize, usize> = HashMap::new();
    // Interior seam constraints (the segments crossing this face). `spade` honours a constraint
    // endpoint that lands on the face boundary, so we do NOT add the boundary edges as constraints
    // (doing so makes the triangulator drop sub-regions) — the convex hull of the points IS the
    // face, and the seam segments alone partition it.
    let mut constraints = Vec::new();
    for s in segs {
        for &sid in &[s.a, s.b] {
            seam_local.entry(sid).or_insert_with(|| {
                let li = pts3.len();
                pts3.push(seam.pos(sid));
                kind.push(LocalVtx::Seam(sid));
                li
            });
        }
        constraints.push((seam_local[&s.a], seam_local[&s.b]));
    }

    // Insert (as plain vertices, no constraint) every GLOBAL seam point that lies on one of this
    // face's edges, even if no local segment referenced it. A seam point on an edge shared with a
    // neighbour face is produced by only ONE of the two faces' segments; both faces must carry it as
    // a vertex or the unsplit side leaves a T-junction (the cross-face tear). Sharing the seam id
    // makes it the SAME output vertex on both faces — watertight across the shared edge.
    for sid in 0..seam.points.len() {
        if seam_local.contains_key(&sid) {
            continue;
        }
        let p = seam.points[sid];
        let on_an_edge = (0..3).any(|e| {
            let (pa, pb) = (cp[e], cp[(e + 1) % 3]);
            let edge = sub(pb, pa);
            let len2 = dot(edge, edge);
            let tparam = dot(sub(p, pa), edge) / len2;
            if !(tparam > 1e-9 && tparam < 1.0 - 1e-9) {
                return false;
            }
            let foot = [pa[0] + edge[0] * tparam, pa[1] + edge[1] * tparam, pa[2] + edge[2] * tparam];
            let d = sub(p, foot);
            dot(d, d) <= len2 * 1e-18
        });
        if on_an_edge {
            let li = pts3.len();
            pts3.push(p);
            kind.push(LocalVtx::Seam(sid));
            seam_local.insert(sid, li);
        }
    }
    (pts3, kind, constraints)
}

/// Re-triangulate W's cut triangle `tw`, keeping ALL sub-triangles (W is never discarded). Seam
/// endpoints are looked up / appended once per seam-id into `w_seam_index`.
fn retriangulate_w_face(
    w: &ClipMesh,
    tw: usize,
    segs: &[&SeamSeg],
    seam: &SeamBuilder,
    out: &mut SideBuilder,
    w_seam_index: &mut HashMap<usize, usize>,
) {
    let (pts3, local_kind, constraints) = build_face_cdt_input(w, tw, segs, seam);
    let normal = w.tri_normal_raw(tw);
    let faces = cdt_face(&pts3, &constraints, normal);
    for (i, j, k) in faces {
        // W keeps every sub-triangle, so a referenced seam vertex is always allocated here.
        let mut map = |li: usize, out: &mut SideBuilder| -> usize {
            match local_kind[li] {
                LocalVtx::Corner(orig) => orig,
                LocalVtx::Seam(sid) => *w_seam_index
                    .entry(sid)
                    .or_insert_with(|| out.push_vertex(seam.pos(sid))),
            }
        };
        let (oi, oj, ok) = (map(i, out), map(j, out), map(k, out));
        out.push_tri(oi, oj, ok);
    }
}

/// Signed distance of `p` to the W mesh: from the triangle whose CLAMPED closest point to `p` is
/// nearest, signed by that triangle's normal — `dot(n, p − closest)`.
///
/// A nearest-CENTROID + infinite-PLANE test mis-signs a bent/oblique W: for a point sitting wholly
/// off the sheet (e.g. a contact face under an erosion ridge's apex, near the domain rim), the
/// nearest triangle by centroid can be a FLANK triangle whose extended plane puts the point on the
/// wrong side → that uncut face is wrongly kept, leaving an isolated 1-triangle sliver in the eroded
/// zone. The clamped closest point is an actual point ON the sheet, so its normal gives the correct
/// side for a point off a non-planar sheet. Used to classify an UNCUT C face (one not crossing W).
fn nearest_w_signed(w: &ClipMesh, p: [f64; 3]) -> f64 {
    nearest_w_signed_among(w, 0..w.triangles.len(), p)
}

/// As [`nearest_w_signed`], but restricted to a SUBSET of W's triangles (`tw_iter`): the signed
/// distance from the subset triangle whose clamped closest point to `p` is nearest, by that
/// triangle's normal. Returns `0.0` for an empty subset (no triangle ⇒ `p` is treated as on-seam).
/// Only the winning triangle's normal is needed, so it is computed once at the end.
///
/// This is the cut-face keep test for `Keep::Side`: a sub-triangle is signed by the NEAREST of the
/// W triangles that actually cut its face — NOT by `.all()` of their infinite planes. Against a
/// curved/bent W those cutters splay, and the `.all()` form discards a sub-triangle that sits on the
/// keep side of its OWN nearest cutter merely because it falls behind a splayed co-cutter's extended
/// plane — carving a wedge out of the kept surface (the bent-wall wedge-gap bug). The clamped
/// nearest-cutter test is the same robust classification the uncut path uses, so the imported
/// `Keep::Side` path is now as bent-wall-safe as the generated `Keep::Predicate` (field) path.
fn nearest_w_signed_among(
    w: &ClipMesh,
    tw_iter: impl IntoIterator<Item = usize>,
    p: [f64; 3],
) -> f64 {
    let mut best_d2 = f64::INFINITY;
    let mut best: Option<usize> = None;
    let mut best_closest = p;
    for t in tw_iter {
        let (a, b, c) = w.tri(t);
        let cl = crate::weld::closest_point_on_triangle_pub(p, a, b, c);
        let d = sub(p, cl);
        let d2 = dot(d, d);
        if d2 < best_d2 {
            best_d2 = d2;
            best = Some(t);
            best_closest = cl;
        }
    }
    match best {
        Some(t) => dot(w.tri_normal_raw(t), sub(p, best_closest)),
        None => 0.0,
    }
}

/// 2D constrained Delaunay re-triangulation of one face: project 3D points to the face plane by
/// dropping the dominant axis, run `cdt` with the constraint edges, and return triangles as index
/// triples into `pts3` — wound to match `face_normal`. The 3D points are NOT moved.
fn cdt_face(
    pts3: &[[f64; 3]],
    constraints: &[(usize, usize)],
    face_normal: [f64; 3],
) -> Vec<(usize, usize, usize)> {
    use spade::{ConstrainedDelaunayTriangulation, Point2, Triangulation};

    let drop = dominant_axis(face_normal);
    let (u, v) = other_axes(drop);
    let pts2: Vec<(f64, f64)> = pts3.iter().map(|p| (p[u], p[v])).collect();

    let mut t: ConstrainedDelaunayTriangulation<Point2<f64>> = ConstrainedDelaunayTriangulation::new();
    let mut handles = Vec::with_capacity(pts2.len());
    for &(x, y) in &pts2 {
        match t.insert(Point2::new(x, y)) {
            Ok(h) => handles.push(h),
            Err(_) => return Vec::new(), // degenerate input (NaN/inf) — caller's assert catches the tear
        }
    }
    // `spade` resolves a constraint endpoint that lands on an existing vertex or edge (the
    // seam-enters-a-face-edge case `cdt` silently dropped), splitting as needed. We forbid the
    // constraint from being SPLIT into new vertices here (the seam chain doesn't self-cross within a
    // single face for the corefinement we feed it); if that ever fires, it's a real new case to
    // handle, not to paper over.
    let n_before = t.num_vertices();
    for &(a, b) in constraints {
        if a < handles.len() && b < handles.len() && a != b {
            let _ = t.add_constraint_and_split(handles[a], handles[b], |p| p);
        }
    }
    debug_assert_eq!(
        t.num_vertices(),
        n_before,
        "spade split a seam constraint into new vertices — unhandled self-crossing in a face"
    );

    // `inner_faces()` yields only the triangulated interior (the face). Map each spade vertex back
    // to our input index via its fixed handle; for our inputs that equals insertion order.
    let area_eps = face_area2(&pts2) * 1e-9;
    let mut out = Vec::new();
    for f in t.inner_faces() {
        let vs = f.vertices();
        let (i, j, k) = (vs[0].fix().index(), vs[1].fix().index(), vs[2].fix().index());
        if i >= pts3.len() || j >= pts3.len() || k >= pts3.len() {
            continue; // a split-added vertex (guarded against above) — skip defensively
        }
        // Drop degenerate (collinear) slivers — e.g. the triangle along an edge through an on-edge
        // seam point. Its centroid would mislabel the keep/discard test.
        if signed_area2(pts2[i], pts2[j], pts2[k]).abs() <= area_eps {
            continue;
        }
        // Dropping an axis can mirror orientation; re-wind to the original face normal.
        let n = cross(sub(pts3[j], pts3[i]), sub(pts3[k], pts3[i]));
        if dot(n, face_normal) < 0.0 {
            out.push((i, k, j));
        } else {
            out.push((i, j, k));
        }
    }
    out
}

/// Twice the signed area of a 2D triangle (the z of the cross product) — zero iff collinear.
#[inline]
fn signed_area2(a: (f64, f64), b: (f64, f64), c: (f64, f64)) -> f64 {
    (b.0 - a.0) * (c.1 - a.1) - (b.1 - a.1) * (c.0 - a.0)
}
/// Is `p` inside (or on the boundary of) triangle `tri`, projected to 2D by dropping `n`'s dominant
/// axis? Sign-consistency of the three edge cross products — robust and tolerance-free. Used by the
/// coplanar drop rule to test whether a C sub-triangle's centroid lies on a coincident W cutter.
fn point_in_tri_2d(p: [f64; 3], tri: ([f64; 3], [f64; 3], [f64; 3]), n: [f64; 3]) -> bool {
    let drop = dominant_axis(n);
    let (u, v) = other_axes(drop);
    let to2 = |q: [f64; 3]| (q[u], q[v]);
    let (q, a, b, c) = (to2(p), to2(tri.0), to2(tri.1), to2(tri.2));
    let (d1, d2, d3) = (signed_area2(a, b, q), signed_area2(b, c, q), signed_area2(c, a, q));
    let has_neg = d1 < 0.0 || d2 < 0.0 || d3 < 0.0;
    let has_pos = d1 > 0.0 || d2 > 0.0 || d3 > 0.0;
    !(has_neg && has_pos)
}
/// Twice the area of the face's first triangle (its 3 corners are pts2[0..3]) — the area scale used
/// to make the degenerate-sliver threshold dimension-independent.
#[inline]
fn face_area2(pts2: &[(f64, f64)]) -> f64 {
    if pts2.len() < 3 {
        return 1.0;
    }
    signed_area2(pts2[0], pts2[1], pts2[2]).abs().max(f64::MIN_POSITIVE)
}

// =============================================================================================
// Empty-seam handling and winding precondition.
// =============================================================================================

/// No C∩W crossings: all of C is on one side of W. Decide by one representative C vertex.
fn empty_seam_result(c: &ClipMesh, w: &ClipMesh, keep: Keep) -> ClipResult {
    let w_unchanged = w.clone();
    if c.vertices.is_empty() || w.triangles.is_empty() {
        return ClipResult { c: c.clone(), w: w_unchanged, shared: Vec::new(), coplanar_pairs: 0 };
    }
    // Representative point: a C triangle centroid (interior, robust to a vertex sitting on W).
    let rep = if let Some(&[i, j, k]) = c.triangles.first() {
        [
            (c.vertices[i][0] + c.vertices[j][0] + c.vertices[k][0]) / 3.0,
            (c.vertices[i][1] + c.vertices[j][1] + c.vertices[k][1]) / 3.0,
            (c.vertices[i][2] + c.vertices[j][2] + c.vertices[k][2]) / 3.0,
        ]
    } else {
        c.vertices[0]
    };
    // Side of the nearest W triangle (clamped closest point) — enough when C lies wholly on one
    // side and W spans the region.
    let kept = match keep {
        Keep::Side(ks) => side_kept(nearest_w_signed(w, rep), ks),
        Keep::Predicate(p) => p(rep),
    };
    if kept {
        ClipResult { c: c.clone(), w: w_unchanged, shared: Vec::new(), coplanar_pairs: 0 } // C on keep side → unchanged
    } else {
        ClipResult { c: ClipMesh::default(), w: w_unchanged, shared: Vec::new(), coplanar_pairs: 0 } // discard side → emptied
    }
}

/// Is the `keep` side selected by `signed` (signed distance to W along its CCW normal)?
fn side_kept(signed: f64, keep: KeepSide) -> bool {
    match keep {
        KeepSide::Above => signed >= 0.0,
        KeepSide::Below => signed <= 0.0,
    }
}

/// Debug precondition: every pair of triangles sharing an edge agrees on outward normal sense (no
/// face flipped relative to its neighbour). Cheap sampled check — full multi-component winding
/// normalization is an import-side concern.
fn w_is_consistently_wound(w: &ClipMesh) -> bool {
    // Map each directed edge to its triangle; a consistently wound manifold never sees the SAME
    // directed edge from two triangles (it sees each edge once forward, once reversed).
    let mut seen: HashMap<(usize, usize), usize> = HashMap::new();
    for [a, b, c] in &w.triangles {
        for &(p, q) in &[(*a, *b), (*b, *c), (*c, *a)] {
            if seen.insert((p, q), 1).is_some() {
                return false; // same directed edge twice → a flipped neighbour
            }
        }
    }
    true
}

// =============================================================================================
// Small local vector helpers (kept local, matching octree.rs / weld.rs convention).
// =============================================================================================

/// Absolute squared-magnitude floor for a DEGENERATE Gram determinant in [`classify_in_triangle`]
/// (a near-zero-area triangle the barycentric solve can't invert). NOT the parallel test anymore —
/// that is now the scale-invariant [`COPLANAR_SIN2`].
const COPLANAR_EPS2: f64 = 1e-18;

/// sin²ε for the SCALE-INVARIANT parallel/coplanar test in [`tri_tri_intersection`]: two faces are
/// parallel iff `|n_c × n_w|² ≤ COPLANAR_SIN2 · |n_c|² · |n_w|²`. A dimensionless ratio (ε ≈ 1e-6 rad),
/// so unlike an absolute threshold on the unnormalized cross product it is immune to edge length and
/// scene scale — a large genuinely-crossing pair is no longer mislabeled coplanar.
const COPLANAR_SIN2: f64 = 1e-12;

/// Relative perpendicular-distance tolerance for "lies IN the plane" (coincident faces) and for the
/// degenerate-overlap area floor — scaled by the triangle pair's bounding extent, so it tracks the
/// scene scale rather than an absolute epsilon.
const COPLANAR_DIST_EPS: f64 = 1e-9;

/// Barycentric tolerance for snapping a seam endpoint onto a triangle edge/vertex feature in
/// [`classify_in_triangle`]. Dimensionless (a ratio), so — unlike the position quantum — it is
/// immune to the scene's absolute scale and offset. Loose enough to catch a point that is on an edge
/// by construction (whose off-edge weight is ulp-scale, even at million-scale coordinates) yet far
/// tighter than any real seam feature's barycentric separation, so distinct seam points never merge.
const SEAM_BARY_EPS: f64 = 1e-9;

#[inline]
fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
#[inline]
fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
#[inline]
fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
/// Snap a signed distance whose magnitude is below an absolute epsilon to exactly 0, so a vertex
/// grazing the other plane is classified consistently (the Möller `d=0` robustness snap).
#[inline]
fn snap0(d: f64) -> f64 {
    if d.abs() < 1e-12 {
        0.0
    } else {
        d
    }
}
/// Index of the largest-magnitude component of `v`.
#[inline]
fn dominant_axis(v: [f64; 3]) -> usize {
    let a = [v[0].abs(), v[1].abs(), v[2].abs()];
    if a[0] >= a[1] && a[0] >= a[2] {
        0
    } else if a[1] >= a[2] {
        1
    } else {
        2
    }
}
/// The two axes that are not `drop`, in ascending order.
#[inline]
fn other_axes(drop: usize) -> (usize, usize) {
    match drop {
        0 => (1, 2),
        1 => (0, 2),
        _ => (0, 1),
    }
}
/// Largest axis-aligned extent of a point set (max over axes of `hi − lo`); the length scale the
/// scale-relative epsilons key off. Zero for a single point or fully-degenerate set.
fn bbox_extent(pts: &[[f64; 3]]) -> f64 {
    let mut lo = [f64::INFINITY; 3];
    let mut hi = [f64::NEG_INFINITY; 3];
    for p in pts {
        for d in 0..3 {
            lo[d] = lo[d].min(p[d]);
            hi[d] = hi[d].max(p[d]);
        }
    }
    (hi[0] - lo[0]).max(hi[1] - lo[1]).max(hi[2] - lo[2])
}
/// A length scale for "a segment too short to be a real seam piece" — a tiny fraction of the
/// triangles' extent, so a graze becomes a touch, not a degenerate constraint.
fn seg_len_eps(pts: &[[f64; 3]]) -> f64 {
    bbox_extent(pts).max(1.0) * 1e-9
}
/// The quantization grid for the seam-weld FALLBACK only (Step B keys combinatorially now — see
/// [`SeamBuilder`]). Small relative to the meshes' extent, but far below any real feature size;
/// retained solely to weld the degenerate endpoint interior to both faces. Its scale-relative
/// coarseness is exactly why it is no longer the primary path for imported meshes.
fn seam_quantum(c: &ClipMesh, w: &ClipMesh) -> f64 {
    let mut lo = [f64::INFINITY; 3];
    let mut hi = [f64::NEG_INFINITY; 3];
    for p in c.vertices.iter().chain(w.vertices.iter()) {
        for d in 0..3 {
            lo[d] = lo[d].min(p[d]);
            hi[d] = hi[d].max(p[d]);
        }
    }
    let ext = ((hi[0] - lo[0]).max(hi[1] - lo[1]).max(hi[2] - lo[2])).max(1.0);
    ext * 1e-7
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mesh_test_util::{assert_seam_watertight, incidence, merge};

    /// A planar grid mesh: `nx × ny` quads over parameters (s, t) ∈ [0,1]², each corner placed by
    /// `f(s, t) -> [x,y,z]`. Triangulated with a consistent diagonal and CCW winding as seen from
    /// the side `f`'s natural normal points. Returns the mesh; no field involved.
    fn grid<F: Fn(f64, f64) -> [f64; 3]>(nx: usize, ny: usize, f: F) -> ClipMesh {
        let mut verts = Vec::new();
        let idx = |i: usize, j: usize| i * (ny + 1) + j;
        for i in 0..=nx {
            for j in 0..=ny {
                verts.push(f(i as f64 / nx as f64, j as f64 / ny as f64));
            }
        }
        let mut tris = Vec::new();
        for i in 0..nx {
            for j in 0..ny {
                let (a, b, c, d) = (idx(i, j), idx(i + 1, j), idx(i + 1, j + 1), idx(i, j + 1));
                tris.push([a, b, c]);
                tris.push([a, c, d]);
            }
        }
        ClipMesh::new(verts, tris)
    }

    /// Best-fit plane of a point set: returns `(point_on_plane, unit_normal)` from the centroid and
    /// the smallest-eigenvector of the covariance, via power iteration on the inverse (we find the
    /// normal as the direction of least variance). Field-agnostic — used to check that no C vertex
    /// strayed off C's OWN plane (the anti-scallop check for a planar test contact, no field needed).
    fn least_squares_plane(pts: &[[f64; 3]]) -> ([f64; 3], [f64; 3]) {
        let n = pts.len() as f64;
        let mut c = [0.0; 3];
        for p in pts {
            for d in 0..3 {
                c[d] += p[d] / n;
            }
        }
        // Covariance matrix (symmetric 3×3).
        let mut cov = [[0.0; 3]; 3];
        for p in pts {
            let q = sub(*p, c);
            for i in 0..3 {
                for j in 0..3 {
                    cov[i][j] += q[i] * q[j];
                }
            }
        }
        // Smallest-eigenvector = normal. Find it as the largest eigenvector of (tr·I − cov), which
        // shares eigenvectors with cov but inverts the order, via power iteration.
        let tr = cov[0][0] + cov[1][1] + cov[2][2];
        let mut m = cov;
        for i in 0..3 {
            for j in 0..3 {
                m[i][j] = if i == j { tr } else { 0.0 } - m[i][j];
            }
        }
        let mut v = [1.0, 0.37, 0.91]; // arbitrary non-axis start
        for _ in 0..128 {
            let mut nv = [
                m[0][0] * v[0] + m[0][1] * v[1] + m[0][2] * v[2],
                m[1][0] * v[0] + m[1][1] * v[1] + m[1][2] * v[2],
                m[2][0] * v[0] + m[2][1] * v[1] + m[2][2] * v[2],
            ];
            let len = (nv[0] * nv[0] + nv[1] * nv[1] + nv[2] * nv[2]).sqrt().max(1e-300);
            for d in 0..3 {
                nv[d] /= len;
            }
            v = nv;
        }
        (c, v)
    }

    /// A jittered planar grid: like `grid` but each interior vertex is nudged WITHIN the plane by a
    /// deterministic pseudo-random offset, so the tessellation is irregular and won't align with
    /// another mesh's — reproducing the "two independent tessellations" condition without a field.
    /// `seed` varies the jitter; `tangent_u`/`tangent_v` are two in-plane directions to jitter along
    /// (so jittered points stay exactly on the plane). Boundary vertices are left unjittered so the
    /// rim stays a clean straight edge for the watertightness frame test.
    fn jittered_grid<F: Fn(f64, f64) -> [f64; 3]>(
        nx: usize,
        ny: usize,
        seed: u64,
        amp: f64,
        tangent_u: [f64; 3],
        tangent_v: [f64; 3],
        f: F,
    ) -> ClipMesh {
        // Deterministic hash → [-0.5, 0.5]; no Math.random (banned, and we want reproducibility).
        let rnd = |i: usize, j: usize, k: u64| -> f64 {
            let mut h = seed
                .wrapping_mul(0x9E3779B97F4A7C15)
                .wrapping_add((i as u64) << 32)
                .wrapping_add((j as u64) << 8)
                .wrapping_add(k);
            h ^= h >> 33;
            h = h.wrapping_mul(0xFF51AFD7ED558CCD);
            h ^= h >> 33;
            ((h % 100_000) as f64) / 100_000.0 - 0.5
        };
        let mut verts = Vec::new();
        let idx = |i: usize, j: usize| i * (ny + 1) + j;
        for i in 0..=nx {
            for j in 0..=ny {
                let mut p = f(i as f64 / nx as f64, j as f64 / ny as f64);
                let interior = i > 0 && i < nx && j > 0 && j < ny;
                if interior {
                    let du = rnd(i, j, 1) * amp;
                    let dv = rnd(i, j, 2) * amp;
                    for d in 0..3 {
                        p[d] += du * tangent_u[d] + dv * tangent_v[d];
                    }
                }
                verts.push(p);
            }
        }
        let mut tris = Vec::new();
        for i in 0..nx {
            for j in 0..ny {
                let (a, b, cc, d) = (idx(i, j), idx(i + 1, j), idx(i + 1, j + 1), idx(i, j + 1));
                tris.push([a, b, cc]);
                tris.push([a, cc, d]);
            }
        }
        ClipMesh::new(verts, tris)
    }

    /// The canonical staircase fixture: a low-angle contact C overshooting a steep, slightly TILTED
    /// wall W — tilted on purpose so the seam crosses both meshes' triangle INTERIORS (the designed,
    /// geologic case of two independently tessellated surfaces), NOT along grid edges.
    ///   * W = the steep plane x = 0.2·z (normal ≈ (+1,0,−0.2)), over y ∈ [0,100], z ∈ [−60,60].
    ///   * C = the gently dipping plane z = 5 + 0.1·x, over x ∈ [−100,100], y ∈ [0,100] — spans BOTH
    ///     sides of W. The seam solves to x ≈ 1.02 (off every grid node), so it cuts through C's and
    ///     W's faces rather than running along their edges. Clipping to `Above` keeps x ≳ 1.02.
    ///
    /// (The grid-ALIGNED case — a whole C edge lying on W — is a known degeneracy deferred past
    /// Milestone 1; see `docs/mesh_clip_design.md` §5 / the deferred-work memory.)
    fn contact_and_wall() -> (ClipMesh, ClipMesh) {
        let c = grid(20, 10, |s, t| {
            let x = -100.0 + s * 200.0;
            let y = t * 100.0;
            [x, y, 5.0 + 0.1 * x]
        });
        let w = grid(10, 12, |s, t| {
            let y = s * 100.0;
            let z = -60.0 + t * 120.0;
            // x = 0.2·z plane; (s,t)→(y,z) ordering makes the CCW normal point in +x (−x·1 dominant).
            [0.2 * z, y, z]
        });
        (c, w)
    }

    /// Milestone 1, assertion (a): after clipping C to W, the seam is watertight in the MERGED
    /// surface — every edge has incidence 2 except the true open rim (C's outer boundary and W's
    /// outer boundary), which is the only place incidence-1 edges may live.
    #[test]
    fn seam_is_watertight_except_true_rim() {
        let (c, w) = contact_and_wall();
        let res = clip(&c, &w, Keep::Side(KeepSide::Above), None);
        // True rim — the only place incidence-1 edges may live: C's kept half (y = 0/100, x = +100;
        // the x = −100 edge is on the discarded side) and W (y = 0/100, z = ±60). The seam (x ≈ 1.02,
        // interior in y) is not on the frame, so an off-frame incidence-1 edge is a torn seam.
        let eps = 0.5;
        let on_frame = move |p: [f64; 3]| {
            p[1].abs() < eps || (p[1] - 100.0).abs() < eps
                || (p[0] - 100.0).abs() < eps
                || (p[2] + 60.0).abs() < eps || (p[2] - 60.0).abs() < eps
        };
        assert_seam_watertight(&res, &on_frame);
    }

    /// Milestone 1, assertion (b): no interior C vertex moved — every ORIGINAL C vertex that
    /// survives on the kept side is bit-identical to its input position (the anti-scallop check, in
    /// its strongest form: exact equality, not a tolerance). Seam vertices are new appended
    /// vertices, never displacements of originals.
    #[test]
    fn c_interior_vertices_are_bit_identical() {
        let (c, w) = contact_and_wall();
        let res = clip(&c, &w, Keep::Side(KeepSide::Above), None);
        // Output is compacted (orphan verts dropped, indices remapped), so identify originals by
        // position. Every output C vertex is EITHER bit-identical to an input C vertex (an untouched
        // original — never displaced) OR a seam vertex on the C∩W line (x ≈ 0.2·z, on W's plane).
        let bits = |p: [f64; 3]| [p[0].to_bits(), p[1].to_bits(), p[2].to_bits()];
        let inputs: std::collections::HashSet<[u64; 3]> = c.vertices.iter().map(|&p| bits(p)).collect();
        let mut kept_interior = 0;
        for &v in &res.c.vertices {
            let is_original = inputs.contains(&bits(v)); // EXACT — no tolerance, the anti-scallop core
            let is_seam = (v[0] - 0.2 * v[2]).abs() < 1e-6; // on the wall = a new seam vertex
            assert!(
                is_original || is_seam,
                "C vertex {v:?} is neither a bit-exact original nor on the seam — it was displaced"
            );
            if is_original && v[0] > 10.0 {
                kept_interior += 1; // an interior original well clear of the seam, kept unchanged
            }
        }
        assert!(kept_interior > 0, "no interior C vertex survived on the kept side");
    }

    /// Milestone 1, assertion (c): seam vertices are SHARED BY INDEX between the C-side and W-side
    /// triangle sets — the `shared` map pairs C-index ↔ W-index, both are actually referenced by
    /// their side's triangles, and the two positions coincide.
    #[test]
    fn seam_vertices_are_shared_by_index() {
        let (c, w) = contact_and_wall();
        let res = clip(&c, &w, Keep::Side(KeepSide::Above), None);
        assert!(!res.shared.is_empty(), "no shared seam vertices");

        let c_refs: std::collections::HashSet<usize> =
            res.c.triangles.iter().flatten().copied().collect();
        let w_refs: std::collections::HashSet<usize> =
            res.w.triangles.iter().flatten().copied().collect();

        for &(ci, wi) in &res.shared {
            // Same point in both buffers.
            let (pc, pw) = (res.c.vertices[ci], res.w.vertices[wi]);
            let d = sub(pc, pw);
            assert!(dot(d, d).sqrt() < 1e-9, "shared seam vertex positions differ: {pc:?} vs {pw:?}");
            // Actually used by both sides' triangles (a shared vertex nobody references is a lie).
            assert!(c_refs.contains(&ci), "C seam vertex {ci} not referenced by any C triangle");
            assert!(w_refs.contains(&wi), "W seam vertex {wi} not referenced by any W triangle");
            // The seam lies on BOTH planes: on W (x = 0.2·z) and on C (z = 5 + 0.1·x). Check the
            // point satisfies W's plane equation — that is what "shared, on the wall" means.
            assert!((pc[0] - 0.2 * pc[2]).abs() < 1e-6, "seam vertex off the wall x=0.2z: {pc:?}");
        }
    }

    /// Empty-seam branch: a C wholly on the discard side of W is REMOVED, not passed through.
    #[test]
    fn empty_seam_discard_side_removes_c() {
        // C entirely at x < 0 (below the wall normal +x); keep = Above → C must vanish.
        let c = grid(4, 4, |s, t| [-100.0 + s * 50.0, t * 100.0, 0.0]); // x ∈ [-100,-50]
        let w = grid(4, 4, |s, t| [0.0, s * 100.0, -60.0 + t * 120.0]);
        let res = clip(&c, &w, Keep::Side(KeepSide::Above), None);
        assert!(res.c.triangles.is_empty(), "C on the discard side should be removed");
        assert_eq!(res.w.triangles.len(), w.triangles.len(), "W must be unchanged");
    }

    /// Empty-seam branch: a C wholly on the KEEP side is passed through unchanged.
    #[test]
    fn empty_seam_keep_side_passes_through() {
        let c = grid(4, 4, |s, t| [50.0 + s * 50.0, t * 100.0, 0.0]); // x ∈ [50,100]
        let w = grid(4, 4, |s, t| [0.0, s * 100.0, -60.0 + t * 120.0]);
        let res = clip(&c, &w, Keep::Side(KeepSide::Above), None);
        assert_eq!(res.c.vertices, c.vertices, "keep-side C must be unchanged");
        assert_eq!(res.c.triangles, c.triangles, "keep-side C must be unchanged");
    }

    // ── Milestone 2 ──────────────────────────────────────────────────────────────────────────
    // The staircase REGRESSION: a contact meeting a steep wall, the two meshes INDEPENDENTLY
    // tessellated (so their seams don't align — the condition that produced the staircase). Assert
    // the seam is watertight AND no C vertex was displaced off C's own plane (the anti-scallop
    // check). Run in two flavours: field-agnostic (no kernel) and on real contoured meshes.

    /// Milestone 2 (field-agnostic): two INDEPENDENTLY tessellated planes — coprime grid resolutions
    /// plus in-plane jitter, so neither the seam nor any vertex aligns between C and W. This is the
    /// staircase condition reproduced WITHOUT a field (proving the module's imported-mesh contract).
    ///
    /// Asserts (a) watertight seam (the shared helper) and (b) ANTI-SCALLOP: every kept C vertex
    /// sits within a tight tolerance of C's OWN least-squares plane — i.e. the clip did not pull any
    /// C vertex off C to meet W. Seam vertices included: they must lie on C's plane too (they are the
    /// true C∩W line, which is on C by construction).
    #[test]
    fn m2_staircase_watertight_and_no_scallop_field_agnostic() {
        // C: gently dipping plane z = 5 + 0.1x over x∈[−100,100], y∈[0,100]. In-plane tangents:
        // u along (1,0,0.1) (the dip direction), v along (0,1,0).
        let cu = normalize_t([1.0, 0.0, 0.1]);
        let cv = [0.0, 1.0, 0.0];
        let c = jittered_grid(23, 11, 0xC0FFEE, 3.0, cu, cv, |s, t| {
            let x = -100.0 + s * 200.0;
            [x, t * 100.0, 5.0 + 0.1 * x]
        });
        // W: steep plane x = 0.2z over y∈[0,100], z∈[−60,60]. Tangents: u along (0,1,0), v along
        // (0.2,0,1) (the in-plane dip). Coprime resolution (13×17 vs C's 23×11).
        let wu = [0.0, 1.0, 0.0];
        let wv = normalize_t([0.2, 0.0, 1.0]);
        let w = jittered_grid(13, 17, 0xBEEF, 3.0, wu, wv, |s, t| {
            let z = -60.0 + t * 120.0;
            [0.2 * z, s * 100.0, z]
        });

        let res = clip(&c, &w, Keep::Side(KeepSide::Above), None);

        // Watertight. Frame: C's kept rim (y=0/100, x=+100) and W's rim (y=0/100, z=±60). Jitter is
        // boundary-free so the frame stays straight; eps is generous vs the seam's distance from it.
        let eps = 1.0;
        let on_frame = move |p: [f64; 3]| {
            p[1].abs() < eps || (p[1] - 100.0).abs() < eps
                || (p[0] - 100.0).abs() < eps
                || (p[2] + 60.0).abs() < eps || (p[2] - 60.0).abs() < eps
        };
        assert_seam_watertight(&res, &on_frame);

        // Anti-scallop: no kept C vertex (interior OR seam) is off C's own least-squares plane.
        let (p0, n) = least_squares_plane(&c.vertices);
        let mut off_max = 0.0_f64;
        for &v in &res.c.vertices {
            off_max = off_max.max(dot(n, sub(v, p0)).abs());
        }
        // C is an exact plane; the only deviation is float noise. A tight bound (1e-6 of the 200-unit
        // extent) catches any real scalloping (which would be a fraction of a cell ≈ several units).
        assert!(off_max < 1e-3, "C vertices strayed off C's plane: {off_max:.4} (scalloping)");
    }

    /// Task 04 acceptance: the SAME independently-tessellated seam as
    /// `m2_staircase_watertight_and_no_scallop_field_agnostic` (feature spacing ≈ 8 units near the
    /// origin), but a CLOSED far-away tetrahedron is welded onto W to balloon the scene EXTENT to
    /// ≈ 1e9 without adding any crossing. The position quantum is `extent · 1e-7` ≈ 100 units — now
    /// COARSER than the seam's crossing spacing — so the old quantized weld merges genuinely distinct
    /// crossings into a handful of ids and the seam tears apart (it collapses to ~2 shared vertices,
    /// 26 torn edges). This is the "large bounding box, fine local features" import case from the
    /// module docs. Note it is EXTENT, not pure translation, that coarsens the quantum: a translation
    /// leaves `extent = hi − lo` unchanged, and every crossing is computed bit-identically by the two
    /// triangles sharing its piercing edge, so the weld survives any offset — the discriminating
    /// stress is a wide box with a fine seam. Combinatorial keying is by integer index pairs,
    /// independent of extent, so the seam keeps all of its vertices and stays watertight: FAILS on the
    /// quantized weld, PASSES here — and (integer keys) is strictly more native==wasm-deterministic.
    #[test]
    fn combinatorial_key_watertight_under_coarse_extent() {
        // Seam scene near the origin, identical to the m2 field-agnostic test.
        let cu = normalize_t([1.0, 0.0, 0.1]);
        let cv = [0.0, 1.0, 0.0];
        let c = jittered_grid(23, 11, 0xC0FFEE, 3.0, cu, cv, |s, t| {
            let x = -100.0 + s * 200.0;
            [x, t * 100.0, 5.0 + 0.1 * x]
        });
        let wu = [0.0, 1.0, 0.0];
        let wv = normalize_t([0.2, 0.0, 1.0]);
        let mut w = jittered_grid(13, 17, 0xBEEF, 3.0, wu, wv, |s, t| {
            let z = -60.0 + t * 120.0;
            [0.2 * z, s * 100.0, z]
        });
        // A CLOSED, outward-wound tetrahedron 1e9 away: it intersects nothing (no extra seam) and has
        // no open edges of its own (so it adds no spurious torn edges to the watertight check), but it
        // stretches the combined bounding box to ≈ 1e9, coarsening `seam_quantum` to ≈ 100 units. It
        // is a disconnected component, so W stays consistently wound (no shared directed edge).
        let f = 1.0e9;
        let base = w.vertices.len();
        w.vertices.extend_from_slice(&[[f, f, f], [f + 1.0, f, f], [f, f + 1.0, f], [f, f, f + 1.0]]);
        for tri in [[0, 2, 1], [0, 1, 3], [0, 3, 2], [1, 2, 3]] {
            w.triangles.push([base + tri[0], base + tri[1], base + tri[2]]);
        }
        debug_assert!(w_is_consistently_wound(&w), "far tetra broke W winding");

        let res = clip(&c, &w, Keep::Side(KeepSide::Above), None);

        // Same rim frame as the unballooned test: C's kept rim (y=0/100, x=+100) and W's rim
        // (y=0/100, z=±60). The far tetra is closed, so it contributes no incidence-1 edges.
        let eps = 1.0;
        let on_frame = move |p: [f64; 3]| {
            p[1].abs() < eps
                || (p[1] - 100.0).abs() < eps
                || (p[0] - 100.0).abs() < eps
                || (p[2] + 60.0).abs() < eps
                || (p[2] - 60.0).abs() < eps
        };
        assert_seam_watertight(&res, &on_frame);
    }

    /// Milestone 2 (real contoured meshes): contour two dipping-plane fields with the OCTREE (the
    /// staircase tessellations the engine actually produces), then clip C against W. Native-only —
    /// it links the kernel; the field-agnostic variant covers wasm. Anti-scallop here uses the field
    /// directly (`|field−iso|/|grad|`, the octree tests' idiom) since C is a fitted contact.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn m2_staircase_on_real_contoured_meshes() {
        use crate::assemble::KernelOptions;
        use crate::data::{Orientation, Series, SurfacePoint};
        use crate::mesh::Mesh;
        use crate::octree::{octree_dual_contour, OctreeConfig};
        use crate::Model;

        // A planar contact z = z0 + sx·(x−500) over the [0,1000]³ box, as a fitted Model.
        let plane = |z0: f64, sx: f64| -> (Model, f64) {
            let mut sps = Vec::new();
            for i in 0..4 {
                for j in 0..4 {
                    let x = i as f64 * 300.0 + 50.0;
                    let y = j as f64 * 300.0 + 50.0;
                    sps.push(SurfacePoint::new([x, y, z0 + sx * (x - 500.0)], 0));
                }
            }
            let oris = vec![Orientation::new([500.0, 500.0, z0], normalize_t([-sx, 0.0, 1.0]))];
            let m = Model::fit(&Series::new(sps, oris), KernelOptions::default());
            let iso = m.lithology().interfaces[0].isovalue;
            (m, iso)
        };
        let cfg = OctreeConfig { max_depth: 5, min_depth: 3, qef_threshold: 1e-6, cap: false };
        let to_clip = |m: &Mesh| -> ClipMesh {
            let verts = (0..m.vertex_count())
                .map(|i| [m.positions[3 * i] as f64, m.positions[3 * i + 1] as f64, m.positions[3 * i + 2] as f64])
                .collect();
            let tris = m.indices.chunks_exact(3).map(|t| [t[0] as usize, t[1] as usize, t[2] as usize]).collect();
            ClipMesh::new(verts, tris)
        };

        // C: gently dipping (low angle). W: steeply dipping the other way (high angle) — they cross
        // near box centre, the staircase geometry from `csg_truncation_is_watertight`.
        let (mc, isoc) = plane(600.0, 0.08);
        let (mw, isow) = plane(600.0, -0.9);
        let cmesh = to_clip(&octree_dual_contour(&mc, [0.0, 0.0, 0.0], 1000.0, isoc, cfg));
        let wmesh = to_clip(&octree_dual_contour(&mw, [0.0, 0.0, 0.0], 1000.0, isow, cfg));
        assert!(cmesh.triangles.len() > 50 && wmesh.triangles.len() > 50, "trivial contour meshes");

        // Winding may be inconsistent straight from the contourer for the cheap precondition; the
        // clip only needs W consistently wound, which the single-field octree output is.
        let res = clip(&cmesh, &wmesh, Keep::Side(KeepSide::Above), None);
        assert!(!res.shared.is_empty(), "real contoured meshes produced no seam");

        // Watertight: the only incidence-1 edges are on the BOX rim (any coordinate at 0 or 1000),
        // the contoured meshes' true open boundary. The seam is interior.
        let on_box = move |p: [f64; 3]| (0..3).any(|d| p[d].abs() < 2.0 || (p[d] - 1000.0).abs() < 2.0);
        assert_seam_watertight(&res, &on_box);

        // Anti-scallop against the FIELD: every (compacted, so orphan-free) kept C vertex stays near
        // C's contact (|f−iso|/|grad| small). A clip that pulled C vertices onto W to close the seam
        // would push this up by ~a cell — the staircase/scallop signature.
        let cell = 1000.0 / 2f64.powi(cfg.max_depth as i32);
        let mut off_max = 0.0_f64;
        for &v in &res.c.vertices {
            let f = mc.scalar_field(v) - isoc;
            let g = mc.scalar_field_gradient(v);
            let gm = (g[0] * g[0] + g[1] * g[1] + g[2] * g[2]).sqrt().max(1e-9);
            off_max = off_max.max((f / gm).abs());
        }
        assert!(
            off_max < cell,
            "kept C vertices strayed off the contact: {off_max:.1} ≥ cell {cell:.0} (scalloping)"
        );
    }

    /// Regression for the wedge-gap: a BENT wall (out-of-plane V, so adjacent W-triangle normals
    /// SPLAY) cutting a contact that overruns it. The previous keep test required a sub-triangle's
    /// centroid to be on the keep side of EVERY co-wall cutter (`.all()`); against splayed planes a
    /// kept triangle bordering one W-face sits behind its splayed neighbour and was discarded —
    /// carving a wedge of contact away from the seam (the "giant gap on one orientation" bug). The
    /// fix classifies each sub-triangle against the NEAREST cutter. Both orientations of the bend
    /// must stay watertight; a flat-wall fixture cannot catch this (all cut planes coincide).
    ///
    /// Unit-length a 3-vector (test-local; the module's `normalize` lives in `mesh.rs` and isn't in
    /// scope here, and we want no dependency on it for the field-agnostic path).
    fn normalize_t(v: [f64; 3]) -> [f64; 3] {
        let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt().max(1e-300);
        [v[0] / l, v[1] / l, v[2] / l]
    }

    // ---- Imported-mesh winding normalization (docs/02_imported_mesh_winding_normalization.md) ----

    /// A mis-wound multi-component fixture: a CLOSED tetrahedron (verts 0..4) + a disjoint OPEN
    /// quad sheet (verts 4..), with a deterministic subset of triangles flipped so the mesh is NOT
    /// consistently wound. Returns it plus the count of tetra verts (4) for component identification.
    fn miswound_two_components() -> ClipMesh {
        let mut verts: Vec<[f64; 3]> =
            vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
        // Outward-wound tetra (CCW seen from outside).
        let mut tris: Vec<[usize; 3]> = vec![[0, 2, 1], [0, 1, 3], [0, 3, 2], [1, 2, 3]];
        // A disjoint 2×2 open sheet at z = 5, well clear of the tetra.
        let base = verts.len();
        for i in 0..3 {
            for j in 0..3 {
                verts.push([10.0 + i as f64, 20.0 + j as f64, 5.0]);
            }
        }
        let sidx = |i: usize, j: usize| base + i * 3 + j;
        for i in 0..2 {
            for j in 0..2 {
                let (a, b, c, d) = (sidx(i, j), sidx(i + 1, j), sidx(i + 1, j + 1), sidx(i, j + 1));
                tris.push([a, b, c]);
                tris.push([a, c, d]);
            }
        }
        // Mis-wind: flip every 3rd triangle (local inconsistency in BOTH components).
        for (k, t) in tris.iter_mut().enumerate() {
            if k % 3 == 0 {
                t.swap(0, 1);
            }
        }
        ClipMesh::new(verts, tris)
    }

    /// Acceptance (1): a mis-wound multi-component mesh, after `winding_normalized`, is consistently
    /// wound, and its CLOSED component (the tetra) is oriented OUTWARD by signed volume. Exercises
    /// the per-component path (the open sheet stays open-consistent; only the tetra gets oriented).
    #[test]
    fn winding_normalize_is_per_component_consistent_and_outward() {
        let m = miswound_two_components();
        assert!(!w_is_consistently_wound(&m), "fixture should be mis-wound");

        let n = m.winding_normalized();
        assert!(w_is_consistently_wound(&n), "normalized mesh is not consistently wound");

        // No duplicate verts in the fixture, so the weld is identity → tetra triangles still
        // reference verts 0..4. Its signed volume must be POSITIVE (outward).
        let vol6: f64 = n
            .triangles
            .iter()
            .filter(|t| t.iter().all(|&v| v < 4))
            .map(|t| {
                let (a, b, c) = (n.vertices[t[0]], n.vertices[t[1]], n.vertices[t[2]]);
                a[0] * (b[1] * c[2] - b[2] * c[1]) - a[1] * (b[0] * c[2] - b[2] * c[0])
                    + a[2] * (b[0] * c[1] - b[1] * c[0])
            })
            .sum();
        assert!(vol6 > 0.0, "closed tetra was not oriented outward (vol6 = {vol6})");
    }

    /// Acceptance (1b): welding lets a SPLIT-VERTEX import (each face authored with its own vertex
    /// copies, no shared indices) still flood-fill — without the weld every triangle is its own
    /// component and nothing can be made mutually consistent.
    #[test]
    fn winding_normalize_welds_split_vertex_import() {
        // Two adjacent triangles of a quad, authored as 6 independent vertices (STL style), with the
        // second triangle wound the opposite way.
        let verts = vec![
            [0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [1.0, 1.0, 0.0], // tri 0 (CCW)
            [0.0, 0.0, 0.0], [1.0, 1.0, 0.0], [0.0, 1.0, 0.0], // tri 1, duplicated shared edge verts
        ];
        let tris = vec![[0usize, 1, 2], [3, 5, 4]]; // tri 1 reversed → inconsistent across the shared edge
        let m = ClipMesh::new(verts, tris);
        let n = m.winding_normalized();
        assert_eq!(n.vertices.len(), 4, "shared-edge duplicates should weld to 4 unique verts");
        assert!(w_is_consistently_wound(&n), "welded sheet should normalize to one consistent component");
    }

    /// Acceptance (2): an imported (mis-wound) wall `W`, after `winding_normalized`, clips cleanly —
    /// the seam is watertight and the debug precondition holds. Keep is a winding-INDEPENDENT
    /// predicate on the wall plane, so the kept side does not depend on the normalizer's arbitrary
    /// open-sheet sense.
    #[test]
    fn imported_wall_normalized_then_clip_is_watertight() {
        let (c, w) = contact_and_wall();
        // Simulate an arbitrary import: flip a subset of W's triangles.
        let mut wt = w.triangles.clone();
        for (k, t) in wt.iter_mut().enumerate() {
            if k % 2 == 0 {
                t.swap(0, 1);
            }
        }
        let w_bad = ClipMesh::new(w.vertices.clone(), wt);
        assert!(!w_is_consistently_wound(&w_bad), "fixture should be mis-wound");

        let w_fixed = w_bad.winding_normalized();
        assert!(w_is_consistently_wound(&w_fixed), "normalize failed to make W consistent");

        // x = 0.2·z is the wall plane; keep the +x side regardless of W's winding sense.
        let keep_plus_x = |p: [f64; 3]| p[0] > 0.2 * p[2];
        let res = clip(&c, &w_fixed, Keep::Predicate(&keep_plus_x), None);
        let eps = 0.5;
        let on_frame = move |p: [f64; 3]| {
            p[1].abs() < eps
                || (p[1] - 100.0).abs() < eps
                || (p[0] - 100.0).abs() < eps
                || (p[2] + 60.0).abs() < eps
                || (p[2] - 60.0).abs() < eps
        };
        assert_seam_watertight(&res, &on_frame);
    }

    /// The precondition is actually enforced: an un-normalized mis-wound `W` trips `clip`'s debug
    /// assert (so the normalize step above is doing real work). Only meaningful in debug builds.
    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "winding-normalized")]
    fn clip_rejects_unnormalized_wall_in_debug() {
        let (c, w) = contact_and_wall();
        let mut wt = w.triangles.clone();
        wt[0].swap(0, 1); // one flipped triangle is enough to break consistency
        let w_bad = ClipMesh::new(w.vertices.clone(), wt);
        let _ = clip(&c, &w_bad, Keep::Side(KeepSide::Above), None);
    }

    // ── Task 03: robust Keep::Side / geometric predicate against a CURVED imported W ──────────
    // (docs/03_clip_keepside_curved_wall_robustness.md)

    /// A field-agnostic CURVED wall (a parabolic curtain): x = `bend`·0.01·(y−40)² over y∈[0,100],
    /// z∈[−60,60], FINELY tessellated in y (24 segments) so adjacent triangles' normals SPLAY along
    /// the curve, coarse (4) in z. The apex (y=40) sits off the coarse contact's grid lines. `bend`
    /// = ±1 bows the curtain toward ±x. The parametric grid is consistently wound; every triangle's
    /// CCW normal has `n_x = Δy·Δz > 0`, so the sheet is coherently +x-oriented.
    fn curved_wall(bend: f64) -> ClipMesh {
        grid(24, 4, |s, t| {
            let y = s * 100.0;
            let z = -60.0 + t * 120.0;
            [bend * 0.01 * (y - 40.0).powi(2), y, z]
        })
    }

    /// A COARSE flat contact at z=15 — mid-cell vs the curtain's z rows at 0/30, so the seam cuts
    /// wall-triangle INTERIORS not edges; x∈[−65,55] (x=0 off any grid line), y∈[0,100]. Overshoots
    /// the curved wall on BOTH sides; coarse in y so a single contact face straddles the curve and
    /// is cut by SEVERAL splayed wall triangles — the condition the `.all()`-over-planes keep test
    /// mis-handled.
    fn coarse_contact() -> ClipMesh {
        grid(4, 4, |s, t| [-65.0 + s * 120.0, t * 100.0, 15.0])
    }

    /// Rim whitelist shared by the curved-wall tests: the contact's outer boundary (x = −65/55,
    /// y = 0/100) and the curtain's (y = 0/100, z = ±60). The seam is interior (z = 15, x ∈ [0,36]),
    /// so any incidence-1 edge OFF this frame is a torn (wedge-carved) seam.
    fn curved_on_frame(p: [f64; 3]) -> bool {
        let eps = 0.5;
        p[1].abs() < eps
            || (p[1] - 100.0).abs() < eps
            || (p[0] + 65.0).abs() < eps
            || (p[0] - 55.0).abs() < eps
            || (p[2] + 60.0).abs() < eps
            || (p[2] - 60.0).abs() < eps
    }

    /// Acceptance (a): the curved-wall wedge-carve driven through `Keep::Side` — the imported,
    /// field-LESS path, NOT the field `Keep::Predicate` the generated wedge regression exercises. A
    /// coarse contact overruns a curved wall; each straddling contact face is cut by several splayed
    /// wall triangles. The old keep test kept a sub-triangle only if its centroid lay on the keep
    /// side of EVERY cutter's infinite plane (`.all()`), carving a wedge out of the kept surface next
    /// to the seam (a torn boundary). The clamped nearest-cutter test keeps it watertight — in BOTH
    /// bend orientations (a flat wall can't catch this: a face's cutters are then coplanar).
    #[test]
    fn keep_side_curved_wall_no_wedge_carve_either_bend() {
        for bend in [1.0_f64, -1.0] {
            let c = coarse_contact();
            let w = curved_wall(bend).winding_normalized();
            let res = clip(&c, &w, Keep::Side(KeepSide::Above), None);
            assert_seam_watertight(&res, &curved_on_frame);
        }
    }

    /// Acceptance (b): clip an imported, field-LESS contact against a curved imported wall via the
    /// geometric [`keep_side_of_mesh`] predicate (the recommended import path). The seam is watertight
    /// AND no contact vertex is pulled off the contact's own plane (anti-scallop) — the curved wall
    /// does not scallop the kept surface, the field-less analog of
    /// `m2_staircase_watertight_and_no_scallop_field_agnostic`.
    #[test]
    fn keep_side_of_mesh_predicate_curved_wall_watertight_no_scallop() {
        let c = coarse_contact(); // planar at z = 15
        let w = curved_wall(1.0).winding_normalized();
        let pred = keep_side_of_mesh(&w, KeepSide::Above);
        let res = clip(&c, &w, Keep::Predicate(&pred), None);

        assert_seam_watertight(&res, &curved_on_frame);

        // Anti-scallop: every kept contact vertex stays on the plane z = 15 (the contact is exactly
        // planar; only float noise deviates). Seam vertices are the true C∩W line, which lies on C.
        let mut off_max = 0.0_f64;
        for v in &res.c.vertices {
            off_max = off_max.max((v[2] - 15.0).abs());
        }
        assert!(off_max < 1e-6, "kept C vertices strayed off z=15: {off_max} (scalloping)");
    }

    /// Flip a consistently-wound sheet whole if its CCW normal points −x, so KeepSide/predicate
    /// "above" is the +x half. Flipping every triangle preserves consistency. (A curtain's normals
    /// are uniformly ±x, so triangle 0's sign speaks for the sheet — see [`curved_wall`].)
    fn orient_plus_x(mut m: ClipMesh) -> ClipMesh {
        if m.tri_normal_raw(0)[0] < 0.0 {
            for t in &mut m.triangles {
                t.swap(0, 1);
            }
        }
        m
    }

    /// Task 03: [`keep_outside_band`] classifies the band EXTERIOR — below `lo_wall` OR above
    /// `hi_wall` (each wall's CCW-normal sense) — which a single `KeepSide` cannot express. Two
    /// parallel curved curtains offset in x; assert the predicate is true outside / false inside,
    /// then drive `clip` with both walls combined into one W and confirm the kept contact is the band
    /// exterior (no vertex strictly between the walls).
    #[test]
    fn keep_outside_band_keeps_imported_band_exterior() {
        let curve = |y: f64| 0.01 * (y - 40.0).powi(2);
        let curtain = |x0: f64| {
            orient_plus_x(
                grid(24, 4, |s, t| {
                    let (y, z) = (s * 100.0, -60.0 + t * 120.0);
                    [x0 + curve(y), y, z]
                })
                .winding_normalized(),
            )
        };
        let lo = curtain(-20.0); // band low wall at x = −20 + curve(y)
        let hi = curtain(20.0); //  band high wall at x = +20 + curve(y)

        let pred = keep_outside_band(&lo, &hi);
        // At y = 40 the curtains sit at x = −20 and x = +20.
        assert!(pred([-50.0, 40.0, 15.0]), "x=−50 is below lo → kept (exterior)");
        assert!(!pred([0.0, 40.0, 15.0]), "x=0 is inside the band → dropped");
        assert!(pred([50.0, 40.0, 15.0]), "x=+50 is above hi → kept (exterior)");

        // Drive it through clip: combine both curtains into ONE W (offset indices keep it
        // consistently wound), clip a contact spanning the whole x-range.
        let mut combined = lo.clone();
        let base = combined.vertices.len();
        combined.vertices.extend_from_slice(&hi.vertices);
        combined
            .triangles
            .extend(hi.triangles.iter().map(|t| [t[0] + base, t[1] + base, t[2] + base]));

        let c = coarse_contact(); // z = 15, x ∈ [−65,55]
        let res = clip(&c, &combined, Keep::Predicate(&pred), None);
        assert!(!res.c.triangles.is_empty(), "band exterior should keep some contact");
        // No kept vertex is strictly inside the band, measured by the SAME mesh signed distance the
        // predicate uses (above lo AND below hi). A seam vertex sits on a wall (signed ≈ 0), so it is
        // not "inside"; comparing against the smooth curve would false-positive on the walls' chord
        // error (~0.04 at 24 segments).
        for &v in &res.c.vertices {
            let strictly_inside = nearest_w_signed(&lo, v) > 1e-6 && nearest_w_signed(&hi, v) < -1e-6;
            assert!(!strictly_inside, "kept C vertex {v:?} is strictly inside the band");
        }
    }

    // ------------------------- Task 05: coplanar handling + sweep acceleration -------------------

    /// Area of a triangle given as a corner tuple.
    fn tri_area(t: ([f64; 3], [f64; 3], [f64; 3])) -> f64 {
        let n = cross(sub(t.1, t.0), sub(t.2, t.0));
        0.5 * dot(n, n).sqrt()
    }
    /// Centroid of triangle `t` of mesh `m`.
    fn centroid3(m: &ClipMesh, t: [usize; 3]) -> [f64; 3] {
        let (a, b, c) = (m.vertices[t[0]], m.vertices[t[1]], m.vertices[t[2]]);
        [(a[0] + b[0] + c[0]) / 3.0, (a[1] + b[1] + c[1]) / 3.0, (a[2] + b[2] + c[2]) / 3.0]
    }

    /// Part A acceptance: a COPLANAR overlap is detected, reported, and produces a watertight join —
    /// C's coincident patch dropped, W kept whole, the overlap boundary a shared seam. Star-of-David:
    /// two coplanar (z=0) triangles, C apex-down and W apex-up, overlapping in a hexagon. Every
    /// overlap-boundary vertex is a clean C-edge × W-edge crossing (no shared corner), so the
    /// combinatorial key interns each as (Edge, Edge) and shares it across both faces — no quantum
    /// fallback. The old code dropped coplanar pairs entirely, leaving C running silently through W.
    #[test]
    fn coplanar_overlap_reported_and_watertight() {
        let c = ClipMesh::new(
            vec![[-50.0, 30.0, 0.0], [50.0, 30.0, 0.0], [0.0, -60.0, 0.0]],
            vec![[0, 1, 2]],
        );
        let w = ClipMesh::new(
            vec![[-50.0, -30.0, 0.0], [50.0, -30.0, 0.0], [0.0, 60.0, 0.0]],
            vec![[0, 1, 2]],
        );
        let res = clip(&c, &w, Keep::Side(KeepSide::Above), None);

        // (a) Reported and (b) a real shared seam exists (C meets W at shared vertices).
        assert_eq!(res.coplanar_pairs, 1, "the coplanar pair must be detected and reported");
        assert!(!res.shared.is_empty(), "coplanar overlap produced no shared seam");

        // (c) Drop C, keep W: no surviving C sub-triangle's centroid lies inside the overlap (= inside
        // W's footprint, since every C sub-triangle is within C). W owns the coincident region.
        let (wtri, wn) = (w.tri(0), w.tri_normal_raw(0));
        for t in &res.c.triangles {
            let ctr = centroid3(&res.c, *t);
            assert!(!point_in_tri_2d(ctr, wtri, wn), "a coincident C sub-triangle survived: {ctr:?}");
        }
        // (d) W kept whole: its total area is preserved (subdivided, never clipped).
        let w_area_out: f64 = (0..res.w.triangles.len()).map(|i| tri_area(res.w.tri(i))).sum();
        assert!((w_area_out - tri_area(w.tri(0))).abs() < 1e-6 * 4500.0, "W was clipped: {w_area_out}");

        // (e) Watertight flat join: no non-manifold/doubled edge (drop-C means no coincident double
        // surface), and every seam edge is shared at incidence 2 — C terminated ON the seam, no tear.
        // A coplanar join is a flat 2-manifold, so (unlike a transversal seam) there is no incidence-3.
        let (_, tris) = merge(&res);
        let inc = incidence(&tris);
        assert_eq!(inc.values().copied().max().unwrap_or(0), 2, "non-manifold/doubled edge in join");
        let seam: std::collections::HashSet<usize> = res.shared.iter().map(|&(ci, _)| ci).collect();
        let torn = inc
            .iter()
            .filter(|((a, b), _)| seam.contains(a) && seam.contains(b))
            .filter(|(_, &cnt)| cnt != 2)
            .count();
        assert_eq!(torn, 0, "a seam edge is not a shared incidence-2 join (torn coplanar seam)");
    }

    /// Part A acceptance (the `docs/05` watch-out): the coplanarity test must be SCALE-INVARIANT. Two
    /// genuinely PERPENDICULAR (90°) triangles at small scale (edge ≈ 4e-3) have an UNnormalized
    /// `|n_c × n_w|² ≈ 6.6e-20` that falls below the old absolute `COPLANAR_EPS2 = 1e-18`, so the
    /// pre-fix code mislabeled this hard crossing as coplanar and DROPPED it. The dimensionless ratio
    /// test sees the right angle and returns the crossing. (The absolute threshold's effective angle
    /// scales with edge length² — wrong at both extremes; a dropped 90° crossing is the clean pin.)
    #[test]
    fn small_crossing_triangles_not_called_coplanar() {
        let s = 1.0e-3;
        let cm = ClipMesh::new(
            vec![[-s, -s, 0.0], [3.0 * s, -s, 0.0], [-s, 3.0 * s, 0.0]],
            vec![[0, 1, 2]],
        );
        let wm = ClipMesh::new(
            vec![[s, -s, -2.0 * s], [s, 3.0 * s, -2.0 * s], [s, -s, 2.0 * s]],
            vec![[0, 1, 2]],
        );
        let (ct, wt) = (cm.tri(0), wm.tri(0));
        let (nc, nw) = (cm.tri_normal_raw(0), wm.tri_normal_raw(0));

        // The fixture must actually trip the OLD absolute threshold, else it proves nothing.
        let dir = cross(nc, nw);
        assert!(dot(dir, dir) <= COPLANAR_EPS2, "fixture no longer trips the old absolute threshold");
        // The fix: the scale-invariant test classifies it as a transversal crossing, not coplanar.
        assert!(
            matches!(tri_tri_intersection(ct, nc, wt, nw), TriTri::Crossing(..)),
            "scale-invariant test wrongly dropped a 90° crossing of small triangles",
        );
    }

    /// Part B acceptance: the uniform AABB grid is a pure candidate FILTER, so forcing it on must
    /// produce BYTE-IDENTICAL `RawSegment`s to the brute O(|C|·|W|) sweep — same pairs, same order,
    /// same positions — not merely the same count. Two independently-tessellated crossing grids (the
    /// import-like case). Asserts equality, not "faster"; a wrongly-filtered candidate would diverge.
    #[test]
    fn accelerated_sweep_byte_identical_to_brute() {
        let cu = normalize_t([1.0, 0.0, 0.1]);
        let cv = [0.0, 1.0, 0.0];
        let c = jittered_grid(31, 19, 0xC0FFEE, 3.0, cu, cv, |s, t| {
            let x = -100.0 + s * 200.0;
            [x, t * 100.0, 5.0 + 0.1 * x]
        });
        let wu = [0.0, 1.0, 0.0];
        let wv = normalize_t([0.2, 0.0, 1.0]);
        let w = jittered_grid(23, 29, 0xBEEF, 3.0, wu, wv, |s, t| {
            let z = -60.0 + t * 120.0;
            [0.2 * z, s * 100.0, z]
        });

        let (brute, brute_cop) = intersection_segments_inner(&c, &w, false);
        let (grid, grid_cop) = intersection_segments_inner(&c, &w, true);
        assert!(!brute.is_empty(), "fixture should produce a seam");
        assert_eq!(brute, grid, "grid sweep diverged from brute — a candidate was wrongly filtered");
        assert_eq!(brute_cop, grid_cop, "coplanar-pair set diverged between the two paths");
    }

    /// Part B perf smoke at import scale (200×200 grids ≈ 80k triangles each). Ignored by default —
    /// the brute baseline at this size is infeasible in CI; the grid path is what we exercise. Run
    /// with `--ignored --nocapture` to eyeball the segment count / wall time.
    #[test]
    #[ignore = "perf smoke: import-scale grids; brute baseline at this size is infeasible in CI"]
    fn accelerated_sweep_large_grid() {
        let cu = normalize_t([1.0, 0.0, 0.1]);
        let cv = [0.0, 1.0, 0.0];
        let c = jittered_grid(200, 200, 0xC0FFEE, 1.0, cu, cv, |s, t| {
            let x = -100.0 + s * 200.0;
            [x, t * 100.0, 5.0 + 0.1 * x]
        });
        let wu = [0.0, 1.0, 0.0];
        let wv = normalize_t([0.2, 0.0, 1.0]);
        let w = jittered_grid(200, 200, 0xBEEF, 1.0, wu, wv, |s, t| {
            let z = -60.0 + t * 120.0;
            [0.2 * z, s * 100.0, z]
        });
        // Auto-selects the grid (pairs ≫ threshold); just needs to complete and produce a seam.
        let (segs, _) = intersection_segments(&c, &w);
        assert!(!segs.is_empty(), "expected a seam on the large crossing grids");
        eprintln!(
            "[perf] {} C tris × {} W tris → {} raw segments",
            c.triangles.len(),
            w.triangles.len(),
            segs.len()
        );
    }

    // ───────────────────────── Task 08: property/fuzz + determinism ─────────────────────────

    /// A deterministic `splitmix64` stream — reproducible, no `Math.random` (banned). Each `next`
    /// advances the state and returns a well-mixed `u64`; `unit` maps it to `[0,1)`.
    struct Rng(u64);
    impl Rng {
        fn new(seed: u64) -> Self {
            Rng(seed.wrapping_mul(0x9E3779B97F4A7C15).wrapping_add(0xD1B54A32D192ED03))
        }
        fn next(&mut self) -> u64 {
            self.0 = self.0.wrapping_add(0x9E3779B97F4A7C15);
            let mut z = self.0;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
            z ^ (z >> 31)
        }
        /// Uniform in `[0,1)`.
        fn unit(&mut self) -> f64 {
            (self.next() >> 11) as f64 / (1u64 << 53) as f64
        }
        /// Uniform in `[lo,hi)`.
        fn range(&mut self, lo: f64, hi: f64) -> f64 {
            lo + (hi - lo) * self.unit()
        }
        /// Uniform integer in `[lo,hi]`.
        fn int(&mut self, lo: usize, hi: usize) -> usize {
            lo + (self.next() as usize) % (hi - lo + 1)
        }
    }

    /// A rigid transform `p ↦ R·p + t` and its exact inverse `q ↦ Rᵀ·(q − t)`. `R` is a proper
    /// rotation (det +1, so it PRESERVES winding — no need to re-normalize the transformed W) built
    /// from yaw/pitch/roll; `t` is a large translation. The inverse subtracts `t` FIRST, which
    /// cancels the large common offset to ~1 ULP of `|t|`, so frame/scallop checks stay sharp at any
    /// translation magnitude (this is why we measure invariants in the LOCAL frame, not world).
    struct Rigid {
        r: [[f64; 3]; 3],
        t: [f64; 3],
    }
    impl Rigid {
        fn from_rng(rng: &mut Rng) -> Self {
            let (a, b, g) = (
                rng.range(0.0, std::f64::consts::TAU),
                rng.range(0.0, std::f64::consts::TAU),
                rng.range(0.0, std::f64::consts::TAU),
            );
            let (ca, sa) = (a.cos(), a.sin());
            let (cb, sb) = (b.cos(), b.sin());
            let (cg, sg) = (g.cos(), g.sin());
            // Rz(a)·Ry(b)·Rx(g).
            let r = [
                [ca * cb, ca * sb * sg - sa * cg, ca * sb * cg + sa * sg],
                [sa * cb, sa * sb * sg + ca * cg, sa * sb * cg - ca * sg],
                [-sb, cb * sg, cb * cg],
            ];
            // Translation magnitude spread over ~[1, 1e6] (log-uniform): exercises both modest and
            // genuinely large placements without pushing float noise past the invariants' tolerances.
            let mag = 10f64.powf(rng.range(0.0, 6.0));
            let t = [
                rng.range(-mag, mag),
                rng.range(-mag, mag),
                rng.range(-mag, mag),
            ];
            Rigid { r, t }
        }
        fn fwd(&self, p: [f64; 3]) -> [f64; 3] {
            [
                self.r[0][0] * p[0] + self.r[0][1] * p[1] + self.r[0][2] * p[2] + self.t[0],
                self.r[1][0] * p[0] + self.r[1][1] * p[1] + self.r[1][2] * p[2] + self.t[1],
                self.r[2][0] * p[0] + self.r[2][1] * p[1] + self.r[2][2] * p[2] + self.t[2],
            ]
        }
        fn inv(&self, q: [f64; 3]) -> [f64; 3] {
            let d = [q[0] - self.t[0], q[1] - self.t[1], q[2] - self.t[2]];
            // Rᵀ·d (R orthonormal).
            [
                self.r[0][0] * d[0] + self.r[1][0] * d[1] + self.r[2][0] * d[2],
                self.r[0][1] * d[0] + self.r[1][1] * d[1] + self.r[2][1] * d[2],
                self.r[0][2] * d[0] + self.r[1][2] * d[1] + self.r[2][2] * d[2],
            ]
        }
        fn apply(&self, m: &mut ClipMesh) {
            for v in &mut m.vertices {
                *v = self.fwd(*v);
            }
        }
    }

    /// Every vertex index referenced by `tris` is in `0..n_verts` AND every index in that range is
    /// referenced — i.e. no orphan vertices and no dangling index (the `compact` pass guarantees
    /// this; we assert it as a `clip` post-condition).
    fn assert_no_orphans(n_verts: usize, tris: &[[usize; 3]], side: &str) {
        let mut used = vec![false; n_verts];
        for t in tris {
            for &i in t {
                assert!(i < n_verts, "{side}: triangle index {i} out of range {n_verts}");
                used[i] = true;
            }
        }
        let orphans = used.iter().filter(|&&u| !u).count();
        assert_eq!(orphans, 0, "{side}: {orphans} orphan vertices left in the buffer");
    }

    /// Task 08 (B) — seeded property/fuzz test. Each seed builds two INDEPENDENTLY-tessellated
    /// jittered planes (a gently-dipping contact C overrunning a steeper wall W), applies ONE rigid
    /// transform (rotation + large translation) to the WHOLE scene, and clips. The transform leaves
    /// the topology identical (same `Keep::Above` side per centroid, normals rotate with W) but
    /// exercises arbitrary placement and large coordinate magnitudes — the import condition. Every
    /// seed must satisfy the `clip` post-conditions; the seed is in each assert message for repro.
    ///
    /// Both branches are exercised: a crossing seam (the 4 post-conditions below) and the empty-seam
    /// pass-through/removal (W placed clear of C). The generator is GATED to the validated domain
    /// (jitter < ¼ cell ⇒ no inverted/zero-area triangles; clearly transversal dips ⇒ no coplanar);
    /// degenerate inputs are covered by the explicit Task-05 tests, not folded in here (the watch-out).
    #[test]
    fn clip_fuzz_postconditions_hold() {
        const X: f64 = 100.0; // C spans x∈[−X,X]
        const Y: f64 = 100.0; // both span y∈[0,Y]
        const ZH: f64 = 60.0; // W spans z∈[−ZH,ZH]
        const ZC: f64 = 5.0; //  C height at x=0

        for seed in 0u64..40 {
            let mut rng = Rng::new(seed);

            // Independently-tessellated resolutions (kept coprime-ish and ≥8 so the seam has teeth).
            let (cnx, cny) = (rng.int(12, 28) | 1, rng.int(9, 21) | 1);
            let (wny, wnz) = (rng.int(11, 27) | 1, rng.int(9, 23) | 1);
            // Clearly transversal dips: C nearly flat, W nearly vertical.
            let sx = rng.range(0.04, 0.16); // C dip
            let tw = rng.range(0.10, 0.35); // W tilt (x = x0 + tw·z)

            // Crossing location. 1 in 4 seeds places W clear of C (empty-seam branch), alternating
            // side; the rest cross C's interior (x0 well inside [−X,X], away from the rim).
            let empty = seed % 4 == 0;
            let x0 = if empty {
                let side = if seed % 8 == 0 { 1.0 } else { -1.0 };
                side * (X + tw * ZH + 20.0) // nearest W edge ≥ X+20 ⇒ no crossing
            } else {
                rng.range(-0.5 * X, 0.5 * X)
            };

            // Jitter amplitude gated to ¼ of the smaller cell (no triangle inversion/zero-area).
            let cell = (2.0 * X / cnx as f64).min(Y / cny as f64).min(2.0 * ZH / wnz as f64);
            let amp = cell * 0.25 * rng.unit();

            let cu = normalize_t([1.0, 0.0, sx]); // C in-plane dip direction
            let cv = [0.0, 1.0, 0.0];
            let mut c = jittered_grid(cnx, cny, seed ^ 0xC0FFEE, amp, cu, cv, |s, t| {
                let x = -X + s * 2.0 * X;
                [x, t * Y, ZC + sx * x]
            });
            let wu = [0.0, 1.0, 0.0];
            let wv = normalize_t([tw, 0.0, 1.0]); // W in-plane dip
            let mut w = jittered_grid(wny, wnz, seed ^ 0xBEEF, amp, wu, wv, |s, t| {
                let z = -ZH + t * 2.0 * ZH;
                [x0 + tw * z, s * Y, z]
            });

            // Half the seeds balloon the scene EXTENT with a far, CLOSED tetrahedron welded onto W
            // (the `combinatorial_key_watertight_under_coarse_extent` device, randomized): it crosses
            // nothing, has no open edges, and stays a disjoint consistently-wound component, but it
            // stretches the bounding box to ~1e6–1e9 — coarsening any extent-derived position quantum
            // past the fine local seam spacing. Combinatorial keying is extent-independent, so the
            // seam survives; this is the stress that catches the pre-Task-04 quantized weld (the
            // acceptance: point this fuzzer at pre-Task-04 code and the wide-extent seeds tear).
            if seed % 2 == 1 {
                let f = 10f64.powf(rng.range(6.0, 9.0));
                let base = w.vertices.len();
                w.vertices
                    .extend_from_slice(&[[f, f, f], [f + 1.0, f, f], [f, f + 1.0, f], [f, f, f + 1.0]]);
                for tri in [[0, 2, 1], [0, 1, 3], [0, 3, 2], [1, 2, 3]] {
                    w.triangles.push([base + tri[0], base + tri[1], base + tri[2]]);
                }
            }
            debug_assert!(w_is_consistently_wound(&w), "seed {seed}: W mis-wound before transform");

            // One rigid transform on the WHOLE scene (rotation preserves winding; large translation).
            let xf = Rigid::from_rng(&mut rng);
            xf.apply(&mut c);
            xf.apply(&mut w);
            debug_assert!(w_is_consistently_wound(&w), "seed {seed}: rotation broke W winding");

            let res = clip(&c, &w, Keep::Side(KeepSide::Above), None);

            // Post-condition (4) — no orphan / dangling vertices, ALWAYS (both branches).
            assert_no_orphans(res.c.vertices.len(), &res.c.triangles, &format!("seed {seed} C"));
            assert_no_orphans(res.w.vertices.len(), &res.w.triangles, &format!("seed {seed} W"));

            if res.shared.is_empty() {
                // Empty-seam branch: W unchanged; C either passed through whole or removed entirely.
                assert!(empty, "seed {seed}: interior crossing produced no seam");
                assert_eq!(res.w.vertices, w.vertices, "seed {seed}: W changed with no seam");
                assert_eq!(res.w.triangles, w.triangles, "seed {seed}: W changed with no seam");
                let c_passed = res.c.vertices == c.vertices && res.c.triangles == c.triangles;
                let c_removed = res.c.triangles.is_empty();
                assert!(c_passed || c_removed, "seed {seed}: empty-seam C neither passed nor removed");
                continue;
            }
            assert!(!empty, "seed {seed}: W placed clear of C but a seam appeared");

            // Inverse-transform to the LOCAL frame so the rim/scallop checks are translation-proof.
            let on_frame = |v: [f64; 3]| {
                let q = xf.inv(v);
                let e = 1.0;
                q[1].abs() < e
                    || (q[1] - Y).abs() < e
                    || (q[0] - X).abs() < e // C's kept +x rim
                    || (q[2] - ZH).abs() < e
                    || (q[2] + ZH).abs() < e // W's z rims
            };

            // (1) Watertight seam.
            assert_seam_watertight(&res, &on_frame);

            // (3) shared pairs are consistent: each referenced seam vertex on C coincides (within
            // float noise) with its partner on W, and both are actually used by their side.
            let c_refs: std::collections::HashSet<usize> =
                res.c.triangles.iter().flatten().copied().collect();
            let w_refs: std::collections::HashSet<usize> =
                res.w.triangles.iter().flatten().copied().collect();
            for &(ci, wi) in &res.shared {
                let d = sub(res.c.vertices[ci], res.w.vertices[wi]);
                assert!(
                    dot(d, d).sqrt() < 1e-6,
                    "seed {seed}: shared seam pair positions differ by {}",
                    dot(d, d).sqrt()
                );
                assert!(c_refs.contains(&ci), "seed {seed}: shared C vertex {ci} unreferenced");
                assert!(w_refs.contains(&wi), "seed {seed}: shared W vertex {wi} unreferenced");
            }

            // (2) Anti-scallop: no kept C vertex strays off C's OWN plane (z = ZC + sx·x in the
            // local frame). C is exactly planar pre-jitter (jitter is in-plane), so only float noise
            // deviates; a clip that pulled C onto W to close the seam would show a fraction-of-a-cell
            // (≫ 1e-3) deflection.
            let mut off_max = 0.0_f64;
            for &v in &res.c.vertices {
                let q = xf.inv(v);
                off_max = off_max.max((q[2] - ZC - sx * q[0]).abs());
            }
            assert!(off_max < 1e-3, "seed {seed}: C strayed off its plane by {off_max:.2e} (scallop)");
        }
    }

    /// Canonical, order-stable byte encoding of a `ClipResult` (f64 via `to_bits`, indices as u64 LE,
    /// length-prefixed) — for the determinism check. NOT a public serialization format.
    fn encode_clip(res: &ClipResult) -> Vec<u8> {
        let mut out = Vec::new();
        let push_u64 = |o: &mut Vec<u8>, x: u64| o.extend_from_slice(&x.to_le_bytes());
        let push_mesh = |o: &mut Vec<u8>, m: &ClipMesh| {
            push_u64(o, m.vertices.len() as u64);
            for v in &m.vertices {
                for c in v {
                    push_u64(o, c.to_bits());
                }
            }
            push_u64(o, m.triangles.len() as u64);
            for t in &m.triangles {
                for &i in t {
                    push_u64(o, i as u64);
                }
            }
        };
        push_mesh(&mut out, &res.c);
        push_mesh(&mut out, &res.w);
        push_u64(&mut out, res.shared.len() as u64);
        for &(a, b) in &res.shared {
            push_u64(&mut out, a as u64);
            push_u64(&mut out, b as u64);
        }
        push_u64(&mut out, res.coplanar_pairs as u64);
        out
    }

    /// FNV-1a 64-bit digest — a tiny dependency-free checksum to pin the golden output value.
    fn fnv1a(bytes: &[u8]) -> u64 {
        let mut h = 0xCBF29CE484222325u64;
        for &b in bytes {
            h ^= b as u64;
            h = h.wrapping_mul(0x100000001B3);
        }
        h
    }

    /// Task 08 (D) — `clip` output is DETERMINISTIC across runs, and its byte-exact value is pinned.
    ///
    /// The PRIMARY lock is run-twice byte-identity: Rust seeds each `HashMap` with an advancing
    /// `RandomState`, so two `clip` calls build their internal maps under DIFFERENT seeds — any
    /// output ordering that leaked `HashMap` iteration order would diverge between the two runs (the
    /// audit in `docs/08` found none; this catches a regression that reintroduces one).
    ///
    /// The golden FNV-1a digest pins the exact bytes. `clip` is DESIGNED to be native↔wasm-identical
    /// (`cdt`/`spade` exact Shewchuk predicates + strict IEEE-754 f64), so the same fixture should
    /// produce this same digest on `wasm32`. NOTE the honest gap: this crate has no
    /// `wasm-bindgen-test` harness, so `#[cfg(test)]` only compiles natively — the cross-target half
    /// is an *asserted design invariant*, not an executed check here. Verifying it on wasm would
    /// require adding that harness (a logged, un-closed gap per the constitution, not a silent
    /// "verified"). On native, the digest still guards every future change to `clip`'s output.
    #[test]
    fn clip_output_is_deterministic() {
        // Golden FNV-1a digest of the canonical encoding (computed natively). `clip` is designed so
        // the wasm32 build produces the same value; see the gap note above.
        const GOLDEN: u64 = 0xE8B1_5AB9_14FD_A1AB;

        let (c, w) = contact_and_wall();
        let r1 = clip(&c, &w, Keep::Side(KeepSide::Above), None);
        let r2 = clip(&c, &w, Keep::Side(KeepSide::Above), None);
        let (b1, b2) = (encode_clip(&r1), encode_clip(&r2));
        assert_eq!(b1, b2, "clip output differs between two runs — HashMap order leaked into output");

        let digest = fnv1a(&b1);
        eprintln!("[determinism] clip golden FNV-1a = 0x{digest:016X} ({} bytes)", b1.len());
        assert_eq!(
            digest, GOLDEN,
            "clip output digest changed: 0x{digest:016X} != golden 0x{GOLDEN:016X} \
             (a native change, OR a native↔wasm divergence if seen only on wasm)"
        );
    }
}
