//! T-junction welding: insert a set of crease points into a target triangle mesh so each point
//! becomes a SHARED vertex, splitting whatever triangle (or shared edge) it lands on.
//!
//! ## Why
//!
//! When two surfaces are dual-contoured independently — e.g. a flat stratigraphic contact and the
//! cross-cutting vein WALL it terminates against — the contact's clipped free-boundary edge lands
//! *on* the wall's surface but at points that are NOT wall vertices: they fall in the middle of a
//! wall triangle (a T-junction). Any sub-cell mismatch between the two independent tessellations
//! of that shared crease curve then shows as see-through triangular slivers. (This is the seam the
//! octree module flags as an unsolved follow-up — two passes discretize the seam into different
//! edges.) Welding the contact's crease vertices INTO the wall mesh removes the T-junctions: the
//! shared crease curve is then a chain of shared vertices, watertight on both sides.
//!
//! ## Mesh-agnostic
//!
//! The target is just a [`Mesh`] of triangles — it does not matter whether it came from our
//! implicit field or was IMPORTED. For implicit↔implicit the crease points already lie on the
//! target surface (projection is a near no-op); for implicit↔imported they generally do NOT, so we
//! PROJECT each point onto the target's nearest triangle first, then split there. The split is the
//! same in both cases.

use crate::mesh::Mesh;

/// Where a projected point sits on its triangle, so the split inserts it without leaving a new
/// T-junction. Barycentric coords `(u, v, w)` for vertices `(a, b, c)` accompany each.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Hit {
    /// Strictly inside the face: split the one triangle into three.
    Interior,
    /// On edge `e` (0 = bc, 1 = ca, 2 = ab) within tolerance: split BOTH triangles sharing that
    /// edge (this one and its neighbour) into two each, so the neighbour stays watertight.
    OnEdge(u8),
    /// Within tolerance of an existing vertex: nothing to do (already shared).
    OnVertex,
}

/// Result of projecting a query point onto the whole mesh: the closest triangle, the closest
/// point on it, the squared distance, the barycentric coords, and where it landed.
struct Projection {
    tri: usize,
    point: [f64; 3],
    dist2: f64,
    bary: [f64; 3],
    hit: Hit,
}

#[inline]
fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
#[inline]
fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
#[inline]
fn pos(mesh: &Mesh, vi: u32) -> [f64; 3] {
    let o = vi as usize * 3;
    [
        mesh.positions[o] as f64,
        mesh.positions[o + 1] as f64,
        mesh.positions[o + 2] as f64,
    ]
}

/// Closest point on triangle `(a,b,c)` to `p`, returned with its barycentric coords `(wa,wb,wc)`
/// w.r.t. `(a,b,c)`. Standard Ericson region test (Real-Time Collision Detection §5.1.5).
/// Public position-only wrapper around [`closest_on_triangle`], for tests/diagnostics that need
/// the closest point on a triangle (e.g. "is this open edge backed by a wall triangle?").
pub fn closest_point_on_triangle_pub(p: [f64; 3], a: [f64; 3], b: [f64; 3], c: [f64; 3]) -> [f64; 3] {
    closest_on_triangle(p, a, b, c).0
}

fn closest_on_triangle(p: [f64; 3], a: [f64; 3], b: [f64; 3], c: [f64; 3]) -> ([f64; 3], [f64; 3]) {
    let ab = sub(b, a);
    let ac = sub(c, a);
    let ap = sub(p, a);
    let d1 = dot(ab, ap);
    let d2 = dot(ac, ap);
    if d1 <= 0.0 && d2 <= 0.0 {
        return (a, [1.0, 0.0, 0.0]);
    }
    let bp = sub(p, b);
    let d3 = dot(ab, bp);
    let d4 = dot(ac, bp);
    if d3 >= 0.0 && d4 <= d3 {
        return (b, [0.0, 1.0, 0.0]);
    }
    let vc = d1 * d4 - d3 * d2;
    if vc <= 0.0 && d1 >= 0.0 && d3 <= 0.0 {
        let v = d1 / (d1 - d3);
        let q = [a[0] + v * ab[0], a[1] + v * ab[1], a[2] + v * ab[2]];
        return (q, [1.0 - v, v, 0.0]);
    }
    let cp = sub(p, c);
    let d5 = dot(ab, cp);
    let d6 = dot(ac, cp);
    if d6 >= 0.0 && d5 <= d6 {
        return (c, [0.0, 0.0, 1.0]);
    }
    let vb = d5 * d2 - d1 * d6;
    if vb <= 0.0 && d2 >= 0.0 && d6 <= 0.0 {
        let w = d2 / (d2 - d6);
        let q = [a[0] + w * ac[0], a[1] + w * ac[1], a[2] + w * ac[2]];
        return (q, [1.0 - w, 0.0, w]);
    }
    let va = d3 * d6 - d5 * d4;
    if va <= 0.0 && (d4 - d3) >= 0.0 && (d5 - d6) >= 0.0 {
        let w = (d4 - d3) / ((d4 - d3) + (d5 - d6));
        let q = [
            b[0] + w * (c[0] - b[0]),
            b[1] + w * (c[1] - b[1]),
            b[2] + w * (c[2] - b[2]),
        ];
        return (q, [0.0, 1.0 - w, w]);
    }
    // Interior.
    let denom = 1.0 / (va + vb + vc);
    let v = vb * denom;
    let w = vc * denom;
    let q = [
        a[0] + ab[0] * v + ac[0] * w,
        a[1] + ab[1] * v + ac[1] * w,
        a[2] + ab[2] * v + ac[2] * w,
    ];
    (q, [1.0 - v - w, v, w])
}

/// Classify a barycentric hit into interior / on-edge / on-vertex, using `edge_eps` as the
/// fraction-of-bary tolerance for "on" an edge or vertex. Edge index convention: edge 0 is
/// opposite vertex a (i.e. bc), edge 1 opposite b (ca), edge 2 opposite c (ab) — so "on edge e"
/// means bary[e] ≈ 0.
fn classify(bary: [f64; 3], edge_eps: f64) -> Hit {
    // On a vertex if two of the three bary coords are ~0.
    let near0 = [bary[0] < edge_eps, bary[1] < edge_eps, bary[2] < edge_eps];
    let zeros = near0.iter().filter(|&&z| z).count();
    if zeros >= 2 {
        return Hit::OnVertex;
    }
    if zeros == 1 {
        let e = near0.iter().position(|&z| z).unwrap() as u8;
        return Hit::OnEdge(e);
    }
    Hit::Interior
}

/// Project `p` onto the whole `mesh` by scanning all triangles (callers restrict the candidate set
/// via a spatial grid before calling — see [`weld_points_into_mesh`]). Returns `None` if the mesh
/// has no triangles. `edge_eps` classifies the landing.
fn project(mesh: &Mesh, p: [f64; 3], cand: &[usize], edge_eps: f64) -> Option<Projection> {
    let mut best: Option<Projection> = None;
    for &tri in cand {
        let i = tri * 3;
        let (a, b, c) = (
            pos(mesh, mesh.indices[i]),
            pos(mesh, mesh.indices[i + 1]),
            pos(mesh, mesh.indices[i + 2]),
        );
        let (q, bary) = closest_on_triangle(p, a, b, c);
        let d2 = {
            let d = sub(q, p);
            dot(d, d)
        };
        if best.as_ref().map_or(true, |bp| d2 < bp.dist2) {
            best = Some(Projection {
                tri,
                point: q,
                dist2: d2,
                bary,
                hit: classify(bary, edge_eps),
            });
        }
    }
    best
}

/// Append a vertex at `p` with normal barycentrically interpolated across triangle `tri`'s
/// vertices (so smooth shading is preserved). Falls back to the face normal if the mesh carries no
/// per-vertex normals. Returns the new vertex index.
fn push_vertex(mesh: &mut Mesh, tri: usize, bary: [f64; 3], p: [f64; 3]) -> u32 {
    let idx = (mesh.positions.len() / 3) as u32;
    mesh.positions.push(p[0] as f32);
    mesh.positions.push(p[1] as f32);
    mesh.positions.push(p[2] as f32);
    if !mesh.normals.is_empty() {
        let i = tri * 3;
        let n = |k: usize| {
            let v = mesh.indices[i + k] as usize * 3;
            [
                mesh.normals[v] as f64,
                mesh.normals[v + 1] as f64,
                mesh.normals[v + 2] as f64,
            ]
        };
        let (na, nb, nc) = (n(0), n(1), n(2));
        let mut nm = [
            na[0] * bary[0] + nb[0] * bary[1] + nc[0] * bary[2],
            na[1] * bary[0] + nb[1] * bary[1] + nc[1] * bary[2],
            na[2] * bary[0] + nb[2] * bary[1] + nc[2] * bary[2],
        ];
        let len = (nm[0] * nm[0] + nm[1] * nm[1] + nm[2] * nm[2]).sqrt();
        if len > 1e-12 {
            nm = [nm[0] / len, nm[1] / len, nm[2] / len];
        }
        mesh.normals.push(nm[0] as f32);
        mesh.normals.push(nm[1] as f32);
        mesh.normals.push(nm[2] as f32);
    }
    let _ = p;
    idx
}

/// The 3 corner indices of triangle `tri`.
#[inline]
fn corners(mesh: &Mesh, tri: usize) -> [u32; 3] {
    let i = tri * 3;
    [mesh.indices[i], mesh.indices[i + 1], mesh.indices[i + 2]]
}

/// Overwrite triangle `tri` in place with `(x, y, z)` (used so a split reuses the original slot
/// for its first sub-triangle and appends the rest, preserving winding).
#[inline]
fn set_tri(mesh: &mut Mesh, tri: usize, x: u32, y: u32, z: u32) {
    let i = tri * 3;
    mesh.indices[i] = x;
    mesh.indices[i + 1] = y;
    mesh.indices[i + 2] = z;
}

#[inline]
fn add_tri(mesh: &mut Mesh, x: u32, y: u32, z: u32) {
    mesh.indices.push(x);
    mesh.indices.push(y);
    mesh.indices.push(z);
}

/// Find the OTHER triangle sharing the undirected edge `(p, q)`, if any, scanning candidates.
fn neighbour_across(mesh: &Mesh, cand: &[usize], exclude: usize, p: u32, q: u32) -> Option<usize> {
    for &tri in cand {
        if tri == exclude {
            continue;
        }
        let c = corners(mesh, tri);
        let has_p = c.contains(&p);
        let has_q = c.contains(&q);
        if has_p && has_q {
            return Some(tri);
        }
    }
    None
}

/// Split triangle `tri` at an INTERIOR point with new vertex `nv`: 1 triangle → 3, fanned from
/// `nv`, preserving the original winding.
fn split_interior(mesh: &mut Mesh, tri: usize, nv: u32) {
    let [a, b, c] = corners(mesh, tri);
    set_tri(mesh, tri, a, b, nv);
    add_tri(mesh, b, c, nv);
    add_tri(mesh, c, a, nv);
}

/// Split triangle `tri` along the edge OPPOSITE its local corner `e` (0→bc, 1→ca, 2→ab) at new
/// vertex `nv`: 1 triangle → 2, preserving winding. The split edge's two endpoints are the two
/// corners other than `e`.
fn split_on_edge(mesh: &mut Mesh, tri: usize, e: u8, nv: u32) {
    let [a, b, c] = corners(mesh, tri);
    match e {
        // Edge bc (opposite a): triangles (a,b,nv) + (a,nv,c).
        0 => {
            set_tri(mesh, tri, a, b, nv);
            add_tri(mesh, a, nv, c);
        }
        // Edge ca (opposite b): triangles (a,b,nv) + (nv,b,c).
        1 => {
            set_tri(mesh, tri, a, b, nv);
            add_tri(mesh, nv, b, c);
        }
        // Edge ab (opposite c): triangles (a,nv,c) + (nv,b,c).
        _ => {
            set_tri(mesh, tri, a, nv, c);
            add_tri(mesh, nv, b, c);
        }
    }
}

/// The two endpoints of triangle `tri`'s edge opposite local corner `e`.
fn edge_endpoints(mesh: &Mesh, tri: usize, e: u8) -> (u32, u32) {
    let [a, b, c] = corners(mesh, tri);
    match e {
        0 => (b, c),
        1 => (c, a),
        _ => (a, b),
    }
}

/// Which local corner of `tri` is opposite the edge `(p, q)` (i.e. the corner that is NEITHER `p`
/// nor `q`)? Returns its edge index 0/1/2. `None` if `tri` doesn't contain both `p` and `q`.
fn opposite_corner(mesh: &Mesh, tri: usize, p: u32, q: u32) -> Option<u8> {
    let c = corners(mesh, tri);
    for (e, &corner) in c.iter().enumerate() {
        if corner != p && corner != q {
            // edge index == local index of the opposite corner only if our edge convention holds:
            // edge e is opposite corner e. The corner that's neither p nor q IS the opposite one.
            return Some(e as u8);
        }
    }
    None
}

/// A simple triangle spatial hash over the mesh's current triangles, by their min-corner cell.
/// Rebuilt once before a weld batch; we look up a query point's neighbourhood to bound the
/// projection scan. Triangles appended DURING the batch are added so later queries see them.
struct TriGrid {
    cell: f64,
    min: [f64; 3],
    map: std::collections::HashMap<(i64, i64, i64), Vec<usize>>,
}

impl TriGrid {
    fn key(&self, p: [f64; 3]) -> (i64, i64, i64) {
        (
            ((p[0] - self.min[0]) / self.cell).floor() as i64,
            ((p[1] - self.min[1]) / self.cell).floor() as i64,
            ((p[2] - self.min[2]) / self.cell).floor() as i64,
        )
    }
    /// Candidate triangles in the 3×3×3 neighbourhood of `p`.
    fn candidates(&self, p: [f64; 3]) -> Vec<usize> {
        let (kx, ky, kz) = self.key(p);
        let mut out = Vec::new();
        for dx in -1..=1 {
            for dy in -1..=1 {
                for dz in -1..=1 {
                    if let Some(v) = self.map.get(&(kx + dx, ky + dy, kz + dz)) {
                        out.extend_from_slice(v);
                    }
                }
            }
        }
        out
    }
    fn build(mesh: &Mesh, cell: f64) -> Self {
        let mut min = [f64::INFINITY; 3];
        for v in mesh.positions.chunks_exact(3) {
            for k in 0..3 {
                min[k] = min[k].min(v[k] as f64);
            }
        }
        if !min[0].is_finite() {
            min = [0.0, 0.0, 0.0];
        }
        let mut grid = TriGrid {
            cell,
            min,
            map: std::collections::HashMap::new(),
        };
        for tri in 0..mesh.triangle_count() {
            grid.insert(mesh, tri);
        }
        grid
    }
    /// Insert a triangle under every grid cell its bounding box overlaps — NOT just its 3 corner
    /// cells. A coarse octree triangle can span many finest-grid cells; corner-only registration
    /// would leave an interior query point (e.g. a crease vertex landing mid-face of a big wall
    /// triangle) finding nothing in its 3×3×3 neighbourhood. Bbox registration guarantees the
    /// containing triangle is always a candidate.
    fn insert(&mut self, mesh: &Mesh, tri: usize) {
        let c = corners(mesh, tri);
        let mut lo = [f64::INFINITY; 3];
        let mut hi = [f64::NEG_INFINITY; 3];
        for &vi in &c {
            let p = pos(mesh, vi);
            for k in 0..3 {
                lo[k] = lo[k].min(p[k]);
                hi[k] = hi[k].max(p[k]);
            }
        }
        let (kx0, ky0, kz0) = self.key(lo);
        let (kx1, ky1, kz1) = self.key(hi);
        for kx in kx0..=kx1 {
            for ky in ky0..=ky1 {
                for kz in kz0..=kz1 {
                    self.map.entry((kx, ky, kz)).or_default().push(tri);
                }
            }
        }
    }
}

/// Insert each of `points` into `target`, splitting the triangle (or shared edge) it projects to
/// so the point becomes a shared vertex — removing T-junctions. `weld_eps` is the max distance a
/// point may sit from the target surface to be welded (points farther than this are skipped: they
/// are not actually on this surface). `cell` sizes the spatial hash (≈ the mesh's typical edge
/// length). Returns the number of points actually welded in.
///
/// Mesh-agnostic: `target` may be implicit-generated or imported. For implicit↔implicit the points
/// already lie on the surface (dist ≈ 0); for imported targets the projection moves the point onto
/// the nearest triangle first.
pub fn weld_points_into_mesh(
    target: &mut Mesh,
    points: &[[f64; 3]],
    weld_eps: f64,
    cell: f64,
) -> usize {
    if target.triangle_count() == 0 || points.is_empty() {
        return 0;
    }
    let mut grid = TriGrid::build(target, cell);
    // Bary tolerance for "on edge/vertex": a small fraction so a point genuinely on an edge takes
    // the watertight two-triangle split, and a point on a corner is a no-op.
    let edge_eps = 1e-4;
    let mut welded = 0;

    for &p in points {
        let cand = grid.candidates(p);
        if cand.is_empty() {
            continue;
        }
        let proj = match project(target, p, &cand, edge_eps) {
            Some(pr) if pr.dist2 <= weld_eps * weld_eps => pr,
            _ => continue, // not on this surface (or no candidate) — skip
        };
        match proj.hit {
            Hit::OnVertex => {
                // Already a shared vertex; nothing to insert.
                welded += 1;
            }
            Hit::Interior => {
                let nv = push_vertex(target, proj.tri, proj.bary, proj.point);
                split_interior(target, proj.tri, nv);
                // The original slot now holds a sub-triangle; index the 2 appended ones.
                grid.insert(target, proj.tri);
                let t = target.triangle_count();
                grid.insert(target, t - 2);
                grid.insert(target, t - 1);
                welded += 1;
            }
            Hit::OnEdge(e) => {
                let (p0, p1) = edge_endpoints(target, proj.tri, e);
                // Find the neighbour across this edge BEFORE we mutate, from the candidate set.
                let neigh = neighbour_across(target, &cand, proj.tri, p0, p1);
                let nv = push_vertex(target, proj.tri, proj.bary, proj.point);
                split_on_edge(target, proj.tri, e, nv);
                grid.insert(target, proj.tri);
                grid.insert(target, target.triangle_count() - 1);
                if let Some(nt) = neigh {
                    if let Some(ne) = opposite_corner(target, nt, p0, p1) {
                        split_on_edge(target, nt, ne, nv);
                        grid.insert(target, nt);
                        grid.insert(target, target.triangle_count() - 1);
                    }
                }
                welded += 1;
            }
        }
    }
    welded
}

/// SNAP `sheet`'s free-boundary vertices ONTO `target`'s surface: move each free-boundary vertex
/// within `weld_eps` of a `target` triangle to the closest point on that triangle. This is the
/// MESH-AGNOSTIC seam closer — `target` is just triangles, so it works whether the wall came from
/// our implicit field OR was IMPORTED (no field). Unlike [`weld_points_into_mesh`] (which splits
/// the TARGET to admit the sheet's vertices), this moves the SHEET's own crease vertices onto the
/// target, closing the gap where a clipped contact's edge lands sub-cell SHORT of the wall (the
/// see-through dashes the shared tree shrinks but doesn't eliminate). Returns vertices moved.
///
/// Only free-boundary vertices are touched (interior sheet vertices stay put), and only those
/// within `weld_eps` of the target (so the box-rim boundary, far from the wall, is left alone).
/// `cell` sizes the spatial hash (≈ the target's typical edge length).
pub fn snap_free_boundary_to_mesh(
    sheet: &mut Mesh,
    target: &Mesh,
    weld_eps: f64,
    cell: f64,
) -> usize {
    if sheet.triangle_count() == 0 || target.triangle_count() == 0 {
        return 0;
    }
    // Which sheet vertices are on its free boundary (incidence-1 edges).
    let mut inc: std::collections::HashMap<(u32, u32), u32> = std::collections::HashMap::new();
    for t in sheet.indices.chunks_exact(3) {
        for &(a, b) in &[(t[0], t[1]), (t[1], t[2]), (t[2], t[0])] {
            let e = if a < b { (a, b) } else { (b, a) };
            *inc.entry(e).or_insert(0) += 1;
        }
    }
    let mut free: std::collections::HashSet<u32> = std::collections::HashSet::new();
    for (&(a, b), &c) in &inc {
        if c == 1 {
            free.insert(a);
            free.insert(b);
        }
    }

    let grid = TriGrid::build(target, cell);
    let mut moved = 0;
    for vi in free {
        let p = pos(sheet, vi);
        let cand = grid.candidates(p);
        // Closest point across candidate target triangles.
        let mut best = (weld_eps * weld_eps, None::<[f64; 3]>);
        for &tri in &cand {
            let i = tri * 3;
            let (a, b, c) = (
                pos(target, target.indices[i]),
                pos(target, target.indices[i + 1]),
                pos(target, target.indices[i + 2]),
            );
            let (q, _) = closest_on_triangle(p, a, b, c);
            let d = sub(q, p);
            let d2 = dot(d, d);
            if d2 < best.0 {
                best = (d2, Some(q));
            }
        }
        if let Some(q) = best.1 {
            let o = vi as usize * 3;
            sheet.positions[o] = q[0] as f32;
            sheet.positions[o + 1] = q[1] as f32;
            sheet.positions[o + 2] = q[2] as f32;
            moved += 1;
        }
    }
    moved
}

/// The positions of `mesh`'s free-boundary vertices (those on an undirected edge with incidence
/// 1). These are a clipped open sheet's rim — its outer box rim AND its inner clip crease where it
/// terminates against another surface. Callers filter to the ones actually on the target surface
/// via `weld_eps` inside [`weld_points_into_mesh`]. De-duplicated by vertex index.
pub fn free_boundary_points(mesh: &Mesh) -> Vec<[f64; 3]> {
    let mut inc: std::collections::HashMap<(u32, u32), u32> = std::collections::HashMap::new();
    for t in mesh.indices.chunks_exact(3) {
        for &(a, b) in &[(t[0], t[1]), (t[1], t[2]), (t[2], t[0])] {
            let e = if a < b { (a, b) } else { (b, a) };
            *inc.entry(e).or_insert(0) += 1;
        }
    }
    let mut verts: std::collections::HashSet<u32> = std::collections::HashSet::new();
    for (&(a, b), &c) in &inc {
        if c == 1 {
            verts.insert(a);
            verts.insert(b);
        }
    }
    verts.into_iter().map(|vi| pos(mesh, vi)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One quad (two triangles) in the z=0 plane, spanning [0,1]², with upward normals. The shared
    /// diagonal edge is (0,0)→(1,1).
    fn unit_quad() -> Mesh {
        let mut m = Mesh::default();
        // verts: 0=(0,0) 1=(1,0) 2=(1,1) 3=(0,1)
        for p in [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [1.0, 1.0, 0.0], [0.0, 1.0, 0.0]] {
            m.positions.extend([p[0] as f32, p[1] as f32, p[2] as f32]);
            m.normals.extend([0.0, 0.0, 1.0]);
        }
        // tri 0 = (0,1,2), tri 1 = (0,2,3) — shared edge 0–2.
        m.indices.extend([0, 1, 2, 0, 2, 3]);
        m
    }

    /// Count undirected edges by incidence; `(nonmanifold, open)` like the stack tests.
    fn edge_incidence(mesh: &Mesh) -> (usize, usize) {
        let mut inc: std::collections::HashMap<(u32, u32), u32> = std::collections::HashMap::new();
        for t in mesh.indices.chunks_exact(3) {
            for &(a, b) in &[(t[0], t[1]), (t[1], t[2]), (t[2], t[0])] {
                let e = if a < b { (a, b) } else { (b, a) };
                *inc.entry(e).or_insert(0) += 1;
            }
        }
        (
            inc.values().filter(|&&c| c > 2).count(),
            inc.values().filter(|&&c| c == 1).count(),
        )
    }

    #[test]
    fn interior_point_splits_one_triangle_into_three() {
        let mut m = unit_quad();
        let before = m.triangle_count();
        // A point well inside triangle 0 (0,1,2): centroid-ish (0.7, 0.2).
        let n = weld_points_into_mesh(&mut m, &[[0.7, 0.2, 0.0]], 1e-6, 1.0);
        assert_eq!(n, 1, "the interior point should weld");
        assert_eq!(m.triangle_count(), before + 2, "interior split adds 2 triangles");
        // The mesh stays a closed disk: boundary edges unchanged (the quad's 4 rim edges), no
        // non-manifold edges introduced.
        let (nm, _open) = edge_incidence(&m);
        assert_eq!(nm, 0, "interior split must not create a non-manifold edge");
    }

    #[test]
    fn on_edge_point_splits_both_shared_triangles() {
        let mut m = unit_quad();
        let before = m.triangle_count();
        // Midpoint of the shared diagonal (0,0)→(1,1): exactly on edge 0–2.
        let n = weld_points_into_mesh(&mut m, &[[0.5, 0.5, 0.0]], 1e-6, 1.0);
        assert_eq!(n, 1);
        assert_eq!(
            m.triangle_count(),
            before + 2,
            "an on-edge weld splits BOTH triangles (2→4)"
        );
        // Crucially watertight: the new vertex is shared, so no T-junction (no open interior edge).
        let (nm, open) = edge_incidence(&m);
        assert_eq!(nm, 0, "on-edge split must stay manifold");
        // The only open edges are the 4 outer rim edges of the unit quad.
        assert_eq!(open, 4, "only the quad rim is open; the diagonal stays shared");
    }

    #[test]
    fn point_on_existing_vertex_is_a_noop_split() {
        let mut m = unit_quad();
        let before = m.triangle_count();
        let n = weld_points_into_mesh(&mut m, &[[1.0, 1.0, 0.0]], 1e-6, 1.0);
        assert_eq!(n, 1, "counts as welded (already shared)");
        assert_eq!(m.triangle_count(), before, "a corner hit splits nothing");
    }

    #[test]
    fn far_point_is_skipped() {
        let mut m = unit_quad();
        let before = m.triangle_count();
        // 0.5 above the plane, weld_eps only 1e-3 → not on the surface.
        let n = weld_points_into_mesh(&mut m, &[[0.5, 0.5, 0.5]], 1e-3, 1.0);
        assert_eq!(n, 0, "a point off the surface must not weld");
        assert_eq!(m.triangle_count(), before);
    }

    #[test]
    fn imported_offset_point_projects_then_welds() {
        // Simulate an IMPORTED target: the crease point sits slightly off the surface (as it would
        // when our implicit contact meets a foreign mesh). With weld_eps large enough it projects
        // onto the face and splits.
        let mut m = unit_quad();
        let before = m.triangle_count();
        let n = weld_points_into_mesh(&mut m, &[[0.7, 0.2, 0.05]], 0.1, 1.0);
        assert_eq!(n, 1, "an off-surface point within weld_eps projects and welds");
        assert_eq!(m.triangle_count(), before + 2);
        let (nm, _) = edge_incidence(&m);
        assert_eq!(nm, 0);
    }

    /// A sheet whose free-boundary vertex sits slightly OFF a target wall is snapped onto the
    /// wall's surface (the mesh-agnostic seam closer — works for an imported wall, no field). A
    /// vertex too far (> weld_eps) is left alone.
    #[test]
    fn snap_moves_only_near_free_boundary_onto_target() {
        // Target: a vertical wall quad in the x=1 plane, spanning y,z ∈ [0,1].
        let mut wall = Mesh::default();
        for p in [[1.0, 0.0, 0.0], [1.0, 1.0, 0.0], [1.0, 1.0, 1.0], [1.0, 0.0, 1.0]] {
            wall.positions.extend([p[0] as f32, p[1] as f32, p[2] as f32]);
            wall.normals.extend([1.0, 0.0, 0.0]);
        }
        wall.indices.extend([0, 1, 2, 0, 2, 3]);

        // Sheet: a horizontal quad whose RIGHT edge (x≈0.92) is a free boundary just short of the
        // wall at x=1 — the contact-crease-short-of-the-wall case. Its left edge is far away.
        let mut sheet = Mesh::default();
        for p in [[0.0, 0.0, 0.5], [0.92, 0.0, 0.5], [0.92, 1.0, 0.5], [0.0, 1.0, 0.5]] {
            sheet.positions.extend([p[0] as f32, p[1] as f32, p[2] as f32]);
            sheet.normals.extend([0.0, 0.0, 1.0]);
        }
        sheet.indices.extend([0, 1, 2, 0, 2, 3]);

        let moved = snap_free_boundary_to_mesh(&mut sheet, &wall, 0.2, 1.0);
        assert_eq!(moved, 2, "the two right-edge vertices (within eps of the wall) snap");
        // Those two vertices now lie ON the wall (x == 1); the far-left two are untouched (x == 0).
        let xs: Vec<f32> = sheet.positions.chunks_exact(3).map(|p| p[0]).collect();
        assert_eq!(xs.iter().filter(|&&x| (x - 1.0).abs() < 1e-5).count(), 2, "right edge on wall");
        assert_eq!(xs.iter().filter(|&&x| x.abs() < 1e-5).count(), 2, "left edge untouched");
    }
}
