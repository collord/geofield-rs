//! Adaptive octree dual contouring (Ju, Losasso, Schaefer, Warren 2002).
//!
//! Refines the octree where the isosurface has detail (sign-changing cells whose
//! single-QEF-vertex residual exceeds a threshold), 2:1-balances the tree, then
//! contours it with the crack-free `cellProc/faceProc/edgeProc` recursion. The
//! 2:1 balance is the precondition that lets `edgeProc` emit one quad per minimal
//! edge without gaps across refinement levels.
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
    // Crack-free contouring.
    cell_proc(&tree, tree.root, &mut mesh);
    mesh
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
// Crack-free contouring: cellProc / faceProc / edgeProc (Ju et al. 2002).
//
// Tables below follow the canonical implementation. Children are indexed by
// corner number c = x + 2y + 4z. Faces and edges are processed per axis.
// ---------------------------------------------------------------------------

/// cellProc: recurse into all 8 children, then the 12 internal faces and 6
/// internal edges between them.
fn cell_proc(tree: &Octree, id: NodeId, mesh: &mut Mesh) {
    let ch = match &tree.nodes[id] {
        Node::Internal(ch) => ch,
        Node::Leaf(_) => return,
    };
    let c: Vec<NodeId> = ch.iter().map(|x| x.unwrap()).collect();

    for &cid in &c {
        cell_proc(tree, cid, mesh);
    }

    // 12 internal faces: pairs of children sharing a face, grouped by axis.
    // (axis, child_a, child_b)
    const FACES: [(usize, usize, usize); 12] = [
        // x-faces (children differ in x bit)
        (0, 0, 1), (0, 2, 3), (0, 4, 5), (0, 6, 7),
        // y-faces (differ in y bit)
        (1, 0, 2), (1, 1, 3), (1, 4, 6), (1, 5, 7),
        // z-faces (differ in z bit)
        (2, 0, 4), (2, 1, 5), (2, 2, 6), (2, 3, 7),
    ];
    for (axis, a, b) in FACES {
        face_proc(tree, c[a], c[b], axis, mesh);
    }

    // 6 internal edges: groups of 4 children around an edge, per axis.
    // (axis, [4 children around the edge in CCW order])
    const EDGES: [(usize, [usize; 4]); 6] = [
        // edges along x: vary x, the 4 cells around share a common x-edge
        (0, [0, 2, 6, 4]),
        (0, [1, 3, 7, 5]),
        // edges along y
        (1, [0, 1, 5, 4]),
        (1, [2, 3, 7, 6]),
        // edges along z
        (2, [0, 1, 3, 2]),
        (2, [4, 5, 7, 6]),
    ];
    for (axis, quad) in EDGES {
        edge_proc(tree, [c[quad[0]], c[quad[1]], c[quad[2]], c[quad[3]]], axis, mesh);
    }
}

/// faceProc: process the shared face between two nodes adjacent along `axis`.
/// Recurses into the 4 child-face pairs and the 4 perpendicular edges straddling
/// the face. Handles level differences by reusing a coarser node in place of its
/// (absent) children.
fn face_proc(tree: &Octree, a: NodeId, b: NodeId, axis: usize, mesh: &mut Mesh) {
    let a_leaf = matches!(tree.nodes[a], Node::Leaf(_));
    let b_leaf = matches!(tree.nodes[b], Node::Leaf(_));
    if a_leaf && b_leaf {
        return; // a face between two leaves generates no edges itself
    }

    // The two child columns on each side of the face, for this axis.
    // For axis x: a contributes its +x children {1,3,5,7}, b its −x children {0,2,4,6}.
    let (a_face, b_face) = face_children(axis);
    let child = |node: NodeId, want: usize| -> NodeId {
        match &tree.nodes[node] {
            Node::Internal(ch) => ch[want].unwrap(),
            Node::Leaf(_) => node, // coarser side: reuse the leaf
        }
    };

    // 4 child face pairs across the face.
    for i in 0..4 {
        let na = child(a, a_face[i]);
        let nb = child(b, b_face[i]);
        face_proc(tree, na, nb, axis, mesh);
    }

    // 4 edges perpendicular to the face, each shared by 4 of the 8 child cells
    // straddling the face. Tables give the (edge axis, 4 children) per face axis.
    for (eaxis, quad_a, quad_b) in face_edge_groups(axis) {
        let n0 = child(a, quad_a[0]);
        let n1 = child(b, quad_b[0]);
        let n2 = child(b, quad_b[1]);
        let n3 = child(a, quad_a[1]);
        edge_proc(tree, [n0, n1, n2, n3], eaxis, mesh);
    }
}

/// edgeProc: the 4 nodes around a common edge (axis `axis`). If all 4 are leaves,
/// emit the dual quad connecting their vertices. Otherwise recurse into the
/// finer children along the edge, substituting a coarser node for its children.
fn edge_proc(tree: &Octree, nodes: [NodeId; 4], axis: usize, mesh: &mut Mesh) {
    let all_leaves = nodes
        .iter()
        .all(|&n| matches!(tree.nodes[n], Node::Leaf(_)));

    if all_leaves {
        emit_edge_quad(tree, nodes, axis, mesh);
        return;
    }

    // Recurse: along the edge axis there are 2 sub-edges. For each, descend each
    // of the 4 nodes to the child touching that sub-edge (or reuse if it's a
    // leaf). The 4 children around each sub-edge are given by the edge tables.
    let pairs = edge_child_pairs(axis); // [2][4] child indices per sub-edge
    for sub in 0..2 {
        let mut sub_nodes = [0usize; 4];
        for q in 0..4 {
            sub_nodes[q] = match &tree.nodes[nodes[q]] {
                Node::Internal(ch) => ch[pairs[sub][q]].unwrap(),
                Node::Leaf(_) => nodes[q],
            };
        }
        edge_proc(tree, sub_nodes, axis, mesh);
    }
}

/// Emit the quad joining the dual vertices of the 4 leaves around an edge, if the
/// edge actually crosses the surface (sign change on the minimal edge). The
/// minimal edge sign is read from the finest (smallest) of the 4 leaves.
fn emit_edge_quad(tree: &Octree, nodes: [NodeId; 4], axis: usize, mesh: &mut Mesh) {
    // Find the finest leaf (smallest size) to read the minimal-edge sign change.
    let mut min_size = f64::INFINITY;
    let mut finest = nodes[0];
    for &n in &nodes {
        let s = tree.bounds[n].1;
        if s < min_size {
            min_size = s;
            finest = n;
        }
    }
    let lf = match &tree.nodes[finest] {
        Node::Leaf(l) => l,
        _ => return,
    };
    // The minimal edge along `axis` on the finest cell: its two endpoint corners.
    let (c0, c1) = minimal_edge_corners(axis);
    let s0 = lf.corners[c0] <= 0.0;
    let s1 = lf.corners[c1] <= 0.0;
    if s0 == s1 {
        return; // edge does not cross the surface → no face here
    }

    // Gather the 4 vertices; skip if any node is a non-surface leaf.
    let mut v = [0u32; 4];
    for (q, &n) in nodes.iter().enumerate() {
        match &tree.nodes[n] {
            Node::Leaf(l) if l.out_index != u32::MAX => v[q] = l.out_index,
            _ => return,
        }
    }

    // Winding from the sign direction so the surface faces increasing field.
    // s0 = corner at the edge's "low" end is inside (≤0): flip accordingly.
    let (t0, t1) = if s1 {
        ([v[0], v[1], v[2]], [v[0], v[2], v[3]])
    } else {
        ([v[0], v[2], v[1]], [v[0], v[3], v[2]])
    };
    mesh.indices.extend_from_slice(&t0);
    mesh.indices.extend_from_slice(&t1);
}

// --- topology tables -------------------------------------------------------

/// For a face shared along `axis`: the 4 children of side A (the +axis side) and
/// of side B (the −axis side) that touch the face, in matching order.
fn face_children(axis: usize) -> ([usize; 4], [usize; 4]) {
    match axis {
        0 => ([1, 3, 5, 7], [0, 2, 4, 6]), // x: A's +x children, B's −x children
        1 => ([2, 3, 6, 7], [0, 1, 4, 5]), // y
        _ => ([4, 5, 6, 7], [0, 1, 2, 3]), // z
    }
}

/// The 4 perpendicular-edge groups straddling a face along `axis`. Each entry is
/// (edge_axis, two A-children, two B-children) describing the 4-node edge fan.
fn face_edge_groups(axis: usize) -> [(usize, [usize; 2], [usize; 2]); 4] {
    // For a face along `axis`, the perpendicular edges run along the other two
    // axes. We produce 4 edge groups: 2 per perpendicular axis.
    match axis {
        0 => [
            (1, [1, 5], [0, 4]),
            (1, [3, 7], [2, 6]),
            (2, [1, 3], [0, 2]),
            (2, [5, 7], [4, 6]),
        ],
        1 => [
            (0, [2, 6], [0, 4]),
            (0, [3, 7], [1, 5]),
            (2, [2, 3], [0, 1]),
            (2, [6, 7], [4, 5]),
        ],
        _ => [
            (0, [4, 6], [0, 2]),
            (0, [5, 7], [1, 3]),
            (1, [4, 5], [0, 1]),
            (1, [6, 7], [2, 3]),
        ],
    }
}

/// For an edge fan along `axis`, the child indices each of the 4 surrounding
/// nodes descends to, for the two sub-edges. `pairs[sub][q]` is the child of
/// node `q` touching sub-edge `sub`.
fn edge_child_pairs(axis: usize) -> [[usize; 4]; 2] {
    // The 4 nodes around an x-edge are ordered (−y−z, +y−z, +y+z, −y+z) etc.
    // Each descends to the 2 children sharing the edge along the edge axis.
    match axis {
        0 => [[6, 4, 0, 2], [7, 5, 1, 3]], // x: children vary in x for sub-edges
        1 => [[5, 4, 0, 1], [7, 6, 2, 3]], // y
        _ => [[3, 2, 0, 1], [7, 6, 4, 5]], // z
    }
}

/// The two corner indices of the minimal edge along `axis` (the edge at the
/// origin corner of the cell), used to read the sign change.
fn minimal_edge_corners(axis: usize) -> (usize, usize) {
    match axis {
        0 => (0, 1), // x-edge from corner 0 to corner 1
        1 => (0, 2), // y-edge
        _ => (0, 4), // z-edge
    }
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
}

