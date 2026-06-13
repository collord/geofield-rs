//! Adaptive octree dual contouring.
//!
//! Refines the octree where the isosurface has detail (sign-changing cells whose
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
//! Hermite data (zero-crossings + analytic gradients) comes from
//! `Model::scalar_field` / `scalar_field_gradient` — the same source the uniform
//! contourer uses. The vertex/QEF machinery is shared with `mesh.rs`.
//!
//! Corner numbering throughout: corner index `c` has offsets
//! `(c&1, (c>>1)&1, (c>>2)&1)` = (x, y, z) ∈ {0,1}³ (same as `mesh::corner_offset`).

use crate::mesh::{corner_offset, normalize, qef_vertex, Mesh, CELL_EDGES};
use crate::Model;

/// A node in the dual-contouring octree.
enum Node {
    /// Internal node: 8 children indexed by corner number (x + 2y + 4z).
    Internal(Box<[Option<NodeId>; 8]>),
    /// Leaf with a surface crossing: holds its dual vertex (index into the output
    /// mesh, filled during contouring) and the corner sign data.
    Leaf(Leaf),
}

struct Leaf {
    /// Field-minus-isovalue at the 8 cell corners.
    corners: [f64; 8],
    /// Dual vertex world position.
    vertex: [f64; 3],
    /// Output mesh vertex index (assigned lazily during contouring).
    out_index: u32,
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
}

impl Default for OctreeConfig {
    fn default() -> Self {
        Self {
            max_depth: 6,
            min_depth: 2,
            qef_threshold: 1e-4,
        }
    }
}

/// Dual-contour the isosurface `field == isovalue` over the cube `[min, max]`
/// (must be a cube for a clean octree; callers pass the model extent's bounding
/// cube). Adaptive: fine near detail, coarse in flat regions.
pub fn octree_dual_contour(
    model: &Model,
    min: [f64; 3],
    size: f64,
    isovalue: f64,
    cfg: OctreeConfig,
) -> Mesh {
    let mut tree = Octree {
        nodes: Vec::new(),
        bounds: Vec::new(),
        root: 0,
    };
    tree.root = build(&mut tree, model, min, size, 0, isovalue, &cfg);
    balance(&mut tree, model, isovalue, &cfg);

    let mut mesh = Mesh::default();
    // Emit each leaf's vertex into the mesh, recording its out_index.
    assign_vertices(&mut tree, &mut mesh);
    // Crack-free contouring by minimal-edge iteration (2:1 balance guarantees
    // each surface-crossing minimal edge is shared by exactly 4 leaves).
    contour_edges(&tree, &mut mesh);
    // Final orientation pass: the per-edge winding is locally consistent but can
    // flip whole patches at transitions, leaving normals that disagree across the
    // surface (visible as dark bands). The field gradient points toward
    // increasing field EVERYWHERE, so we flip any triangle whose geometric normal
    // disagrees with the gradient at its centroid — giving globally coherent
    // outward normals independent of the edge bookkeeping.
    orient_to_gradient(model, &mut mesh);
    mesh
}

/// Flip each triangle whose geometric normal opposes the field gradient at its
/// centroid, so all faces are oriented toward increasing field.
fn orient_to_gradient(model: &Model, mesh: &mut Mesh) {
    let pos = |i: u32| {
        let b = (i as usize) * 3;
        [
            mesh.positions[b] as f64,
            mesh.positions[b + 1] as f64,
            mesh.positions[b + 2] as f64,
        ]
    };
    let tri_count = mesh.indices.len() / 3;
    for t in 0..tri_count {
        let i0 = mesh.indices[3 * t];
        let i1 = mesh.indices[3 * t + 1];
        let i2 = mesh.indices[3 * t + 2];
        let a = pos(i0);
        let b = pos(i1);
        let c = pos(i2);
        let ab = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
        let ac = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
        let n = [
            ab[1] * ac[2] - ab[2] * ac[1],
            ab[2] * ac[0] - ab[0] * ac[2],
            ab[0] * ac[1] - ab[1] * ac[0],
        ];
        let ctr = [
            (a[0] + b[0] + c[0]) / 3.0,
            (a[1] + b[1] + c[1]) / 3.0,
            (a[2] + b[2] + c[2]) / 3.0,
        ];
        let g = model.scalar_field_gradient(ctr);
        if n[0] * g[0] + n[1] * g[1] + n[2] * g[2] < 0.0 {
            // Reverse winding (swap two vertices).
            mesh.indices[3 * t + 1] = i2;
            mesh.indices[3 * t + 2] = i1;
        }
    }
}

/// Sample the field at a node's 8 corners (field − isovalue).
fn sample_corners(model: &Model, min: [f64; 3], size: f64, isovalue: f64) -> [f64; 8] {
    let mut c = [0.0; 8];
    for i in 0..8 {
        let (dx, dy, dz) = corner_offset(i);
        let p = [
            min[0] + dx as f64 * size,
            min[1] + dy as f64 * size,
            min[2] + dz as f64 * size,
        ];
        c[i] = model.scalar_field(p) - isovalue;
    }
    c
}

/// Does any of the 12 cell edges change sign?
fn has_crossing(corners: &[f64; 8]) -> bool {
    CELL_EDGES
        .iter()
        .any(|&(a, b)| (corners[a] <= 0.0) != (corners[b] <= 0.0))
}

/// Hermite crossings + QEF vertex for a cell. Returns (vertex, error) or None if
/// the cell has no surface crossing.
fn cell_vertex(
    model: &Model,
    min: [f64; 3],
    size: f64,
    corners: &[f64; 8],
) -> Option<([f64; 3], f64)> {
    let mut crossings: Vec<([f64; 3], [f64; 3])> = Vec::new();
    for &(a, b) in CELL_EDGES.iter() {
        let (va, vb) = (corners[a], corners[b]);
        if (va <= 0.0) == (vb <= 0.0) {
            continue;
        }
        let t = va / (va - vb);
        let (ai, aj, ak) = corner_offset(a);
        let (bi, bj, bk) = corner_offset(b);
        let pa = [
            min[0] + ai as f64 * size,
            min[1] + aj as f64 * size,
            min[2] + ak as f64 * size,
        ];
        let pb = [
            min[0] + bi as f64 * size,
            min[1] + bj as f64 * size,
            min[2] + bk as f64 * size,
        ];
        let p = [
            pa[0] + (pb[0] - pa[0]) * t,
            pa[1] + (pb[1] - pa[1]) * t,
            pa[2] + (pb[2] - pa[2]) * t,
        ];
        crossings.push((p, normalize(model.scalar_field_gradient(p))));
    }
    if crossings.is_empty() {
        return None;
    }
    let hi = [min[0] + size, min[1] + size, min[2] + size];
    Some(qef_vertex(&crossings, min, hi))
}

/// Recursively build the octree. A cell becomes a leaf when: it is at max depth,
/// or (past min depth) it has no crossing, or its single-vertex QEF error is
/// under threshold. Empty (no-crossing) cells below min depth still collapse to a
/// leaf with no vertex contribution.
fn build(
    tree: &mut Octree,
    model: &Model,
    min: [f64; 3],
    size: f64,
    depth: u32,
    isovalue: f64,
    cfg: &OctreeConfig,
) -> NodeId {
    let corners = sample_corners(model, min, size, isovalue);
    let crossing = has_crossing(&corners);

    let make_leaf = |tree: &mut Octree| -> NodeId {
        let (vertex, _) = cell_vertex(model, min, size, &corners)
            .unwrap_or(([min[0] + size * 0.5, min[1] + size * 0.5, min[2] + size * 0.5], 0.0));
        let id = tree.nodes.len();
        tree.nodes.push(Node::Leaf(Leaf {
            corners,
            vertex,
            out_index: u32::MAX,
        }));
        tree.bounds.push((min, size));
        id
    };

    if depth >= cfg.max_depth {
        return make_leaf(tree);
    }
    if depth >= cfg.min_depth {
        // Below the forced floor: stop where there's no detail.
        if !crossing {
            return make_leaf(tree);
        }
        if let Some((_, err)) = cell_vertex(model, min, size, &corners) {
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
        children[c] = Some(build(tree, model, cmin, half, depth + 1, isovalue, cfg));
    }
    let id = tree.nodes.len();
    tree.nodes.push(Node::Internal(Box::new(children)));
    tree.bounds.push((min, size));
    id
}

/// 2:1 balance: ensure no leaf is more than one level coarser than a leaf
/// touching it. We enforce it structurally by re-subdividing any leaf that is
/// adjacent to a node more than one level finer. Implemented as a fixpoint over
/// depth: repeatedly subdivide offending leaves until stable.
///
/// For the edgeProc recursion to be crack-free we only need that, along any
/// shared face/edge, refinement levels differ by at most one. We achieve this by
/// tracking each leaf's depth and, for any leaf whose neighbor (found by point
/// sampling at face centers) is ≥2 levels finer, splitting it once; repeat.
fn balance(tree: &mut Octree, model: &Model, isovalue: f64, cfg: &OctreeConfig) {
    // Collect leaves with their depth by walking the tree.
    loop {
        let mut to_split: Vec<NodeId> = Vec::new();
        let mut leaf_depth: std::collections::HashMap<NodeId, u32> =
            std::collections::HashMap::new();
        collect_leaf_depths(tree, tree.root, 0, &mut leaf_depth);

        // Max depth present anywhere (a leaf's neighbors finer than depth+1 force
        // a split). We detect imbalance via the size ratio of edge-adjacent
        // leaves: find, for each leaf, the finest leaf sharing any of its 6 face
        // neighborhoods by sampling just outside each face center.
        for (&leaf_id, &d) in leaf_depth.iter() {
            let (min, size) = tree.bounds[leaf_id];
            // Only surface leaves matter for cracks.
            if let Node::Leaf(l) = &tree.nodes[leaf_id] {
                if !has_crossing(&l.corners) {
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
            split_leaf(tree, model, leaf_id, leaf_depth[&leaf_id], isovalue, cfg);
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
fn split_leaf(
    tree: &mut Octree,
    model: &Model,
    leaf_id: NodeId,
    depth: u32,
    isovalue: f64,
    cfg: &OctreeConfig,
) {
    let (min, size) = tree.bounds[leaf_id];
    let half = size * 0.5;
    let mut children: [Option<NodeId>; 8] = [None; 8];
    // Children are forced to exist one level finer; build() will further refine
    // them by the normal rule if warranted, capped at max_depth.
    for c in 0..8 {
        let (dx, dy, dz) = corner_offset(c);
        let cmin = [
            min[0] + dx as f64 * half,
            min[1] + dy as f64 * half,
            min[2] + dz as f64 * half,
        ];
        children[c] = Some(build(tree, model, cmin, half, depth + 1, isovalue, cfg));
    }
    tree.nodes[leaf_id] = Node::Internal(Box::new(children));
}

/// Emit each surface leaf's QEF vertex into the mesh, recording out_index.
fn assign_vertices(tree: &mut Octree, mesh: &mut Mesh) {
    for id in 0..tree.nodes.len() {
        if let Node::Leaf(l) = &mut tree.nodes[id] {
            if has_crossing(&l.corners) {
                let idx = (mesh.positions.len() / 3) as u32;
                mesh.positions.push(l.vertex[0] as f32);
                mesh.positions.push(l.vertex[1] as f32);
                mesh.positions.push(l.vertex[2] as f32);
                l.out_index = idx;
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Crack-free contouring by minimal-edge iteration.
//
// Rather than the recursive cellProc/faceProc/edgeProc (whose transition tables
// are error-prone to reproduce), we iterate the minimal surface-crossing edges
// directly. With a 2:1-balanced octree, every minimal edge that crosses the
// surface is shared by exactly 4 leaves; we locate them by point sampling the 4
// quadrant centers around the edge and emit one quad. This is simple point-
// location, not fragile recursion — far harder to get subtly wrong.
//
// A "minimal edge" is an edge of a *finest* leaf among those sharing it. We find
// them by walking every leaf and, for each of its 12 edges that change sign,
// keying the edge by (axis, quantized midpoint) so each is processed once, by
// the finest leaf that owns it.
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

fn contour_edges(tree: &Octree, mesh: &mut Mesh) {
    // Finest leaf size present (for quantization + half-step offsets).
    let mut finest = f64::INFINITY;
    for id in 0..tree.nodes.len() {
        if matches!(tree.nodes[id], Node::Leaf(_)) {
            finest = finest.min(tree.bounds[id].1);
        }
    }
    // Domain box = the root node's bounds (used to project boundary vertices onto
    // the domain faces, closing the surface flush to the box).
    let (rmin, rsize) = tree.bounds[tree.root];
    let domain_min = rmin;
    let domain_max = [rmin[0] + rsize, rmin[1] + rsize, rmin[2] + rsize];
    if !finest.is_finite() {
        return;
    }

    let mut seen: std::collections::HashSet<(usize, i64, i64, i64)> =
        std::collections::HashSet::new();
    // Boundary vertices placed ON domain faces, deduped by quantized position so
    // adjacent boundary edges share them (a connected boundary strip).
    let mut boundary_verts: std::collections::HashMap<(i64, i64, i64), u32> =
        std::collections::HashMap::new();

    for id in 0..tree.nodes.len() {
        let (lmin, lsize) = tree.bounds[id];
        let corners = match &tree.nodes[id] {
            Node::Leaf(l) if has_crossing(&l.corners) => l.corners,
            _ => continue,
        };

        for &(ca, cb) in CELL_EDGES.iter() {
            if (corners[ca] <= 0.0) == (corners[cb] <= 0.0) {
                continue;
            }
            let diff = ca ^ cb;
            let axis = match diff {
                1 => 0,
                2 => 1,
                4 => 2,
                _ => continue,
            };
            let (ax, ay, az) = corner_offset(ca);
            let p0 = [
                lmin[0] + ax as f64 * lsize,
                lmin[1] + ay as f64 * lsize,
                lmin[2] + az as f64 * lsize,
            ];
            let mut mid = p0;
            mid[axis] += lsize * 0.5;
            let (u, v) = match axis {
                0 => (1, 2),
                1 => (0, 2),
                _ => (0, 1),
            };

            let key = (
                axis,
                qkey(mid[0], finest),
                qkey(mid[1], finest),
                qkey(mid[2], finest),
            );

            // The surface crossing point on this edge (where the boundary vertex
            // is projected to, if a quadrant is exterior).
            let (va, vb) = (corners[ca], corners[cb]);
            let t = va / (va - vb);
            let cross = {
                let mut c = p0;
                c[axis] += lsize * t;
                c
            };

            let off = finest * 0.5;
            let quad_centers = [
                quad_center(mid, u, v, -off, -off),
                quad_center(mid, u, v, off, -off),
                quad_center(mid, u, v, off, off),
                quad_center(mid, u, v, -off, off),
            ];

            // Resolve each quadrant: interior leaf, or exterior (boundary).
            #[derive(Clone, Copy)]
            enum Q {
                Leaf(NodeId),
                Exterior([f64; 3]), // the quadrant center, used to pick the face
            }
            let mut q = [Q::Exterior([0.0; 3]); 4];
            let mut this_is_finest = true;
            for (qi, qc) in quad_centers.iter().enumerate() {
                match leaf_at(tree, tree.root, *qc) {
                    Some(lf) => {
                        q[qi] = Q::Leaf(lf);
                        if tree.bounds[lf].1 < lsize - 1e-9 {
                            this_is_finest = false;
                        }
                    }
                    None => q[qi] = Q::Exterior(*qc),
                }
            }
            if !this_is_finest {
                continue;
            }
            // All-exterior would be degenerate (shouldn't happen for a real edge).
            if q.iter().all(|x| matches!(x, Q::Exterior(_))) {
                continue;
            }
            if !seen.insert(key) {
                continue;
            }

            // Build the 4 ring vertices. Interior quadrants use the leaf's vertex;
            // exterior quadrants get a boundary vertex placed at `cross` projected
            // onto the domain face nearest that quadrant.
            let mut vtx = [u32::MAX; 4];
            let mut good = true;
            for qi in 0..4 {
                match q[qi] {
                    Q::Leaf(lf) => match &tree.nodes[lf] {
                        Node::Leaf(l) if l.out_index != u32::MAX => vtx[qi] = l.out_index,
                        _ => {
                            good = false;
                            break;
                        }
                    },
                    Q::Exterior(qc) => {
                        // Project the edge crossing onto the domain face that the
                        // exterior quadrant center lies beyond (in u and/or v).
                        let mut bp = cross;
                        for &d in &[u, v] {
                            if qc[d] < domain_min[d] {
                                bp[d] = domain_min[d];
                            } else if qc[d] > domain_max[d] {
                                bp[d] = domain_max[d];
                            }
                        }
                        vtx[qi] = boundary_vertex(mesh, &mut boundary_verts, bp, finest);
                    }
                }
            }
            if !good {
                continue;
            }

            // Triangulate the ring, emitting each triangle only if its 3 vertices
            // are distinct (boundary rings can collapse two corners onto the same
            // deduped face vertex, which would make slivers / non-manifold edges).
            // orient_to_gradient fixes winding later.
            for tri in [[vtx[0], vtx[1], vtx[2]], [vtx[0], vtx[2], vtx[3]]] {
                if tri[0] != tri[1] && tri[1] != tri[2] && tri[0] != tri[2] {
                    mesh.indices.extend_from_slice(&tri);
                }
            }
        }
    }
}

/// Get or create a deduplicated boundary vertex at world position `p` (on a
/// domain face), keyed by quantized position so adjacent boundary edges share it.
fn boundary_vertex(
    mesh: &mut Mesh,
    cache: &mut std::collections::HashMap<(i64, i64, i64), u32>,
    p: [f64; 3],
    finest: f64,
) -> u32 {
    let key = (
        qkey(p[0], finest),
        qkey(p[1], finest),
        qkey(p[2], finest),
    );
    if let Some(&idx) = cache.get(&key) {
        return idx;
    }
    let idx = (mesh.positions.len() / 3) as u32;
    mesh.positions.push(p[0] as f32);
    mesh.positions.push(p[1] as f32);
    mesh.positions.push(p[2] as f32);
    cache.insert(key, idx);
    idx
}

/// Center point offset from edge midpoint `mid` by `du,dv` along in-plane axes.
fn quad_center(mid: [f64; 3], u: usize, v: usize, du: f64, dv: f64) -> [f64; 3] {
    let mut p = mid;
    p[u] += du;
    p[v] += dv;
    p
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assemble::KernelOptions;
    use crate::data::{Orientation, Series, SurfacePoint};

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
        let cfg = OctreeConfig {
            max_depth: 5,
            min_depth: 2,
            qef_threshold: 1e-5,
        };
        let mesh = octree_dual_contour(&model, [-0.5, -0.5, 0.0], 2.0, 0.0, cfg);
        assert!(mesh.vertex_count() > 0, "no vertices");
        assert!(mesh.triangle_count() > 0, "no triangles");

        let vmax = mesh.vertex_count() as u32;
        assert!(mesh.indices.iter().all(|&i| i < vmax), "dangling index");

        // Vertices that lie within the data footprint should sit near z=1.
        let near: Vec<f64> = mesh
            .positions
            .chunks_exact(3)
            .filter(|p| p[0] >= 0.0 && p[0] <= 1.0 && p[1] >= 0.0 && p[1] <= 1.0)
            .map(|p| p[2] as f64)
            .collect();
        assert!(!near.is_empty(), "no vertices in data footprint");
        let mean = near.iter().sum::<f64>() / near.len() as f64;
        assert!((mean - 1.0).abs() < 0.2, "sheet not near z=1: mean={mean}");
    }

    #[test]
    fn empty_when_no_crossing() {
        let model = flat_model();
        // A box entirely above the contact (z in [1.5, 2.0]) has no crossing.
        let mesh = octree_dual_contour(&model, [0.0, 0.0, 1.5], 0.5, 0.0, OctreeConfig::default());
        assert_eq!(mesh.triangle_count(), 0);
    }

    /// Regression for the missing-triangle / winding bugs: on a CURVED surface
    /// that forces refinement (so coarse/fine transitions exist), the mesh must
    /// be (a) crack-free — few boundary edges relative to interior — and (b)
    /// consistently wound. The earlier recursive contourer left ~30% boundary
    /// edges (holes) here; this guards the edge-iteration contourer against
    /// regressing. (Measured with the same edge/winding stats used during the
    /// fix; thresholds are generous — the real numbers are ≪ them.)
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
        let boundary = undirected.values().filter(|&&c| c == 1).count();
        let nonmanifold = undirected.values().filter(|&&c| c > 2).count();
        let total = undirected.len();

        // Crack-free: boundary edges are only the open-sheet perimeter, a small
        // fraction. (Broken contourer hit ~30%+; we require < 10%.)
        assert!(
            (boundary as f64) < 0.10 * (total as f64),
            "too many boundary edges (holes): {boundary}/{total}"
        );
        // Near-manifold (vanilla DC may leave a tiny number at deepest cells).
        assert!(
            (nonmanifold as f64) < 0.02 * (total as f64),
            "too many non-manifold edges: {nonmanifold}/{total}"
        );
        // Consistent winding: every interior edge traversed once each direction.
        let mut bad = 0;
        for (&(a, b), &n) in &dir {
            let opp = *dir.get(&(b, a)).unwrap_or(&0);
            if opp == 0 {
                continue; // boundary edge
            }
            if n != opp {
                bad += 1;
            }
        }
        assert_eq!(bad, 0, "inconsistent winding on {bad} edges");

        // Globally coherent normals: EVERY face normal must agree with the field
        // gradient at its centroid (the orient_to_gradient pass). Edge-paired
        // winding alone does NOT guarantee this — a fully flipped patch passes the
        // winding check but shades dark; this assertion is what actually catches
        // the misoriented-normal bug.
        let posn = |i: u32| {
            let b = (i as usize) * 3;
            [
                mesh.positions[b] as f64,
                mesh.positions[b + 1] as f64,
                mesh.positions[b + 2] as f64,
            ]
        };
        let mut backward = 0;
        for t in mesh.indices.chunks_exact(3) {
            let a = posn(t[0]);
            let b = posn(t[1]);
            let c = posn(t[2]);
            let ab = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
            let ac = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
            let n = [
                ab[1] * ac[2] - ab[2] * ac[1],
                ab[2] * ac[0] - ab[0] * ac[2],
                ab[0] * ac[1] - ab[1] * ac[0],
            ];
            let ctr = [
                (a[0] + b[0] + c[0]) / 3.0,
                (a[1] + b[1] + c[1]) / 3.0,
                (a[2] + b[2] + c[2]) / 3.0,
            ];
            let g = model.scalar_field_gradient(ctr);
            if n[0] * g[0] + n[1] * g[1] + n[2] * g[2] < 0.0 {
                backward += 1;
            }
        }
        assert_eq!(backward, 0, "{backward} triangles oriented against the gradient");
    }
}

