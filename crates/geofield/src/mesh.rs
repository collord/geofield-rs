//! Uniform-grid dual contouring of the scalar field (an isosurface extractor).
//!
//! This is the simplest correct dual-contouring variant: a regular grid, one
//! vertex per sign-changing cell, placed by minimizing the quadratic error
//! function (QEF) of the Hermite data (zero-crossings + analytic gradients along
//! cell edges). Faces are quads emitted across each grid edge that changes sign,
//! split into two triangles for rendering.
//!
//! It reuses `Model::scalar_field` (for crossings) and `Model::scalar_field_gradient`
//! (for the Hermite normals) — exactly the data dual contouring is designed to
//! consume. No octree/adaptivity yet (a clean seam for a later pass): the grid is
//! uniform, so there are no hanging nodes and no crack problem.
//!
//! Output is a triangle mesh: a flat `Vec<f32>` of vertex positions (`3*V`) and a
//! flat `Vec<u32>` of triangle indices (`3*T`), ready for a GPU buffer.

use geofield_kernel::ScalarField;

/// An axis-aligned sampling box and resolution for dual contouring.
#[derive(Debug, Clone, Copy)]
pub struct MeshGrid {
    pub min: [f64; 3],
    pub max: [f64; 3],
    /// Number of cells per axis (corners are `res + 1` per axis).
    pub res: usize,
}

impl MeshGrid {
    pub fn new(min: [f64; 3], max: [f64; 3], res: usize) -> Self {
        Self { min, max, res }
    }

    /// World coordinate of corner (i, j, k), i,j,k in `0..=res`.
    #[inline]
    fn corner(&self, i: usize, j: usize, k: usize) -> [f64; 3] {
        let t = |a: f64, b: f64, n: usize, idx: usize| a + (b - a) * (idx as f64 / n as f64);
        [
            t(self.min[0], self.max[0], self.res, i),
            t(self.min[1], self.max[1], self.res, j),
            t(self.min[2], self.max[2], self.res, k),
        ]
    }
}

/// A triangle mesh as flat GPU-ready buffers.
#[derive(Debug, Default, Clone)]
pub struct Mesh {
    /// `3 * vertex_count` positions (x,y,z, …) in world coordinates.
    pub positions: Vec<f32>,
    /// `3 * vertex_count` per-vertex unit normals, parallel to `positions`. Empty
    /// unless the producer fills them (the octree contourer does, from the analytic
    /// field gradient — exact and smooth, so renderers should prefer these over
    /// face-averaged normals). The uniform `dual_contour` leaves this empty.
    pub normals: Vec<f32>,
    /// `3 * triangle_count` indices into `positions`.
    pub indices: Vec<u32>,
}

impl Mesh {
    pub fn vertex_count(&self) -> usize {
        self.positions.len() / 3
    }
    pub fn triangle_count(&self) -> usize {
        self.indices.len() / 3
    }
}

/// The 12 edges of a cell, as pairs of corner-offset indices (0..8).
/// Corner index = i + 2*j + 4*k (local bit layout: bit0=x, bit1=y, bit2=z).
pub(crate) const CELL_EDGES: [(usize, usize); 12] = [
    (0, 1), (2, 3), (4, 5), (6, 7), // x-direction edges
    (0, 2), (1, 3), (4, 6), (5, 7), // y-direction edges
    (0, 4), (1, 5), (2, 6), (3, 7), // z-direction edges
];

/// Local corner offsets for index = i + 2j + 4k.
#[inline]
pub(crate) fn corner_offset(c: usize) -> (usize, usize, usize) {
    (c & 1, (c >> 1) & 1, (c >> 2) & 1)
}

/// Dual-contour the isosurface `field == isovalue` over `grid`.
///
/// Algorithm:
///   1. sample the field at every corner ((res+1)^3 evaluations);
///   2. for each cell with at least one sign-changing edge, solve a small QEF
///      from the per-edge Hermite data to place one vertex inside the cell;
///   3. for each interior grid edge that changes sign, emit the quad formed by
///      the vertices of the 4 cells sharing that edge (two triangles), wound so
///      the normal points along increasing field (toward higher values).
pub fn dual_contour(model: &dyn ScalarField, grid: &MeshGrid, isovalue: f64) -> Mesh {
    let n = grid.res;
    let nc = n + 1; // corners per axis
    let corner_idx = |i: usize, j: usize, k: usize| (i * nc + j) * nc + k;

    // 1. Sample field at all corners.
    let mut vals = vec![0.0_f64; nc * nc * nc];
    for i in 0..nc {
        for j in 0..nc {
            for k in 0..nc {
                let p = grid.corner(i, j, k);
                vals[corner_idx(i, j, k)] = model.scalar_field(p) - isovalue;
            }
        }
    }

    // 2. One vertex per sign-changing cell. cell_vertex[cell] = Some(vertex idx).
    let cell_lin = |i: usize, j: usize, k: usize| (i * n + j) * n + k;
    let mut cell_vertex = vec![u32::MAX; n * n * n];
    let mut mesh = Mesh::default();

    for ci in 0..n {
        for cj in 0..n {
            for ck in 0..n {
                // Gather the 8 corner values of this cell.
                let mut cval = [0.0_f64; 8];
                for c in 0..8 {
                    let (di, dj, dk) = corner_offset(c);
                    cval[c] = vals[corner_idx(ci + di, cj + dj, ck + dk)];
                }

                // Collect Hermite data on sign-changing edges.
                let mut crossings: Vec<([f64; 3], [f64; 3])> = Vec::new(); // (point, normal)
                for &(a, b) in CELL_EDGES.iter() {
                    let (va, vb) = (cval[a], cval[b]);
                    if (va <= 0.0) == (vb <= 0.0) {
                        continue; // no sign change
                    }
                    // Linear interpolation parameter of the zero crossing.
                    let t = va / (va - vb);
                    let (ai, aj, ak) = corner_offset(a);
                    let (bi, bj, bk) = corner_offset(b);
                    let pa = grid.corner(ci + ai, cj + aj, ck + ak);
                    let pb = grid.corner(ci + bi, cj + bj, ck + bk);
                    let p = [
                        pa[0] + (pb[0] - pa[0]) * t,
                        pa[1] + (pb[1] - pa[1]) * t,
                        pa[2] + (pb[2] - pa[2]) * t,
                    ];
                    let g = model.scalar_field_gradient(p);
                    crossings.push((p, normalize(g)));
                }

                if crossings.is_empty() {
                    continue;
                }

                let v = solve_qef(&crossings, grid, ci, cj, ck);
                let idx = mesh.vertex_count() as u32;
                mesh.positions.push(v[0] as f32);
                mesh.positions.push(v[1] as f32);
                mesh.positions.push(v[2] as f32);
                cell_vertex[cell_lin(ci, cj, ck)] = idx;
            }
        }
    }

    // 3. Quads across sign-changing interior grid edges. For each edge direction,
    // the 4 cells sharing the edge supply the quad's 4 vertices.
    emit_quads(&vals, &cell_vertex, n, nc, &mut mesh);

    mesh
}

/// Solve the QEF for the cell vertex: minimize Σ (nᵢ·(x − pᵢ))². We use the
/// normal-equations form with a small Tikhonov regularization toward the mass
/// point (the average crossing), which keeps the vertex inside the cell and
/// stable when the normals are near-degenerate. This is the standard robust
/// fallback; for a uniform grid it is plenty.
fn solve_qef(
    crossings: &[([f64; 3], [f64; 3])],
    grid: &MeshGrid,
    ci: usize,
    cj: usize,
    ck: usize,
) -> [f64; 3] {
    let lo = grid.corner(ci, cj, ck);
    let hi = grid.corner(ci + 1, cj + 1, ck + 1);
    qef_vertex(crossings, lo, hi).0
}

/// Solve the QEF for a cell vertex from its Hermite crossings, clamped into the
/// cell AABB `[lo, hi]`. Returns `(vertex, residual_error)`, where the residual
/// is `Σ (nᵢ·(x − pᵢ))²` at the solved vertex — a measure of how badly one
/// vertex fits the cell's surface detail (used by the octree refinement rule).
///
/// Shared by the uniform and octree dual contourers. Uses the normal-equations
/// form with a small Tikhonov regularization toward the mass point, which keeps
/// the vertex inside the cell and stable when normals are near-degenerate.
pub(crate) fn qef_vertex(
    crossings: &[([f64; 3], [f64; 3])],
    lo: [f64; 3],
    hi: [f64; 3],
) -> ([f64; 3], f64) {
    // Mass point (average of crossings) — also the regularization target.
    let mut mass = [0.0_f64; 3];
    for (p, _) in crossings {
        mass[0] += p[0];
        mass[1] += p[1];
        mass[2] += p[2];
    }
    let inv = 1.0 / crossings.len() as f64;
    mass = [mass[0] * inv, mass[1] * inv, mass[2] * inv];

    // Normal equations AᵀA x = Aᵀb, where each row is nᵢ and bᵢ = nᵢ·pᵢ.
    let mut ata = [[0.0_f64; 3]; 3];
    let mut atb = [0.0_f64; 3];
    for (p, nrm) in crossings {
        let d = nrm[0] * p[0] + nrm[1] * p[1] + nrm[2] * p[2];
        for r in 0..3 {
            for c in 0..3 {
                ata[r][c] += nrm[r] * nrm[c];
            }
            atb[r] += nrm[r] * d;
        }
    }
    // Tikhonov: add λ·I and λ·mass so the system is always solvable and biased to
    // the mass point in degenerate directions.
    let lambda = 1e-3;
    for r in 0..3 {
        ata[r][r] += lambda;
        atb[r] += lambda * mass[r];
    }

    let mut x = solve3(ata, atb).unwrap_or(mass);

    // Clamp the vertex into the cell AABB (DC vertices should stay in their cell;
    // clamp guards against an ill-conditioned QEF escaping it).
    x = [
        x[0].clamp(lo[0], hi[0]),
        x[1].clamp(lo[1], hi[1]),
        x[2].clamp(lo[2], hi[2]),
    ];

    // Residual error Σ (nᵢ·(x − pᵢ))² at the (clamped) vertex.
    let mut err = 0.0;
    for (p, nrm) in crossings {
        let d = nrm[0] * (x[0] - p[0]) + nrm[1] * (x[1] - p[1]) + nrm[2] * (x[2] - p[2]);
        err += d * d;
    }
    (x, err)
}

/// Emit a quad (two triangles) for each interior grid edge that changes sign,
/// using the vertices of the four cells incident to that edge. Winding is chosen
/// from the edge's sign direction so triangles face increasing field value.
fn emit_quads(vals: &[f64], cell_vertex: &[u32], n: usize, nc: usize, mesh: &mut Mesh) {
    let corner_idx = |i: usize, j: usize, k: usize| (i * nc + j) * nc + k;
    let cell_lin = |i: usize, j: usize, k: usize| (i * n + j) * n + k;

    // For an edge along axis `d` at corner (i,j,k)→(i,j,k)+e_d, the 4 incident
    // cells are obtained by stepping −1/0 in the two axes perpendicular to `d`.
    // We only consider edges whose 4 incident cells all exist (interior edges).
    for i in 0..nc {
        for j in 0..nc {
            for k in 0..nc {
                // x-direction edge: (i,j,k) -> (i+1,j,k). The 4 incident cells
                // step −1/0 in j and k, so the edge must be interior in j,k:
                // 1 <= j <= n-1 and 1 <= k <= n-1 (cell indices stay in 0..n).
                if i < n && j >= 1 && j < n && k >= 1 && k < n {
                    let a = vals[corner_idx(i, j, k)];
                    let b = vals[corner_idx(i + 1, j, k)];
                    if (a <= 0.0) != (b <= 0.0) {
                        let cells = [
                            cell_lin(i, j - 1, k - 1),
                            cell_lin(i, j, k - 1),
                            cell_lin(i, j, k),
                            cell_lin(i, j - 1, k),
                        ];
                        push_quad(cell_vertex, &cells, a > b, mesh);
                    }
                }
                // y-direction edge: (i,j,k) -> (i,j+1,k)
                if j < n && i >= 1 && i < n && k >= 1 && k < n {
                    let a = vals[corner_idx(i, j, k)];
                    let b = vals[corner_idx(i, j + 1, k)];
                    if (a <= 0.0) != (b <= 0.0) {
                        let cells = [
                            cell_lin(i - 1, j, k - 1),
                            cell_lin(i, j, k - 1),
                            cell_lin(i, j, k),
                            cell_lin(i - 1, j, k),
                        ];
                        push_quad(cell_vertex, &cells, a <= b, mesh);
                    }
                }
                // z-direction edge: (i,j,k) -> (i,j,k+1)
                if k < n && i >= 1 && i < n && j >= 1 && j < n {
                    let a = vals[corner_idx(i, j, k)];
                    let b = vals[corner_idx(i, j, k + 1)];
                    if (a <= 0.0) != (b <= 0.0) {
                        let cells = [
                            cell_lin(i - 1, j - 1, k),
                            cell_lin(i, j - 1, k),
                            cell_lin(i, j, k),
                            cell_lin(i - 1, j, k),
                        ];
                        push_quad(cell_vertex, &cells, a > b, mesh);
                    }
                }
            }
        }
    }
}

/// Push a quad from 4 cell vertices as two triangles, skipping if any cell has no
/// vertex (shouldn't happen for a valid sign-changing edge, but guards anyway).
/// `flip` reverses winding to keep the surface oriented toward increasing field.
fn push_quad(cell_vertex: &[u32], cells: &[usize; 4], flip: bool, mesh: &mut Mesh) {
    let v: [u32; 4] = [
        cell_vertex[cells[0]],
        cell_vertex[cells[1]],
        cell_vertex[cells[2]],
        cell_vertex[cells[3]],
    ];
    if v.iter().any(|&x| x == u32::MAX) {
        return;
    }
    let (t0, t1) = if flip {
        ([v[0], v[2], v[1]], [v[0], v[3], v[2]])
    } else {
        ([v[0], v[1], v[2]], [v[0], v[2], v[3]])
    };
    mesh.indices.extend_from_slice(&t0);
    mesh.indices.extend_from_slice(&t1);
}

pub(crate) fn normalize(v: [f64; 3]) -> [f64; 3] {
    let n = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    if n < 1e-30 {
        [0.0, 0.0, 0.0]
    } else {
        [v[0] / n, v[1] / n, v[2] / n]
    }
}

/// Solve a 3x3 system by Cramer's rule. Returns None if near-singular.
fn solve3(a: [[f64; 3]; 3], b: [f64; 3]) -> Option<[f64; 3]> {
    let det = a[0][0] * (a[1][1] * a[2][2] - a[1][2] * a[2][1])
        - a[0][1] * (a[1][0] * a[2][2] - a[1][2] * a[2][0])
        + a[0][2] * (a[1][0] * a[2][1] - a[1][1] * a[2][0]);
    if det.abs() < 1e-18 {
        return None;
    }
    let col = |a: [[f64; 3]; 3], c: usize, b: [f64; 3]| {
        let mut m = a;
        for r in 0..3 {
            m[r][c] = b[r];
        }
        m
    };
    let d = |m: [[f64; 3]; 3]| {
        m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1])
            - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0])
            + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0])
    };
    Some([
        d(col(a, 0, b)) / det,
        d(col(a, 1, b)) / det,
        d(col(a, 2, b)) / det,
    ])
}

#[cfg(test)]
mod tests {
    use super::*;
    use geofield_kernel::assemble::KernelOptions;
    use geofield_kernel::data::{Orientation, Series, SurfacePoint};
    use geofield_kernel::Model;

    /// A flat-lying interface at z≈1 should dual-contour to a roughly planar
    /// sheet near z=1 when meshed at isovalue 0.
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
    fn produces_a_nonempty_oriented_sheet() {
        let model = flat_model();
        let grid = MeshGrid::new([-0.2, -0.2, 0.4], [1.2, 1.2, 1.6], 16);
        let mesh = dual_contour(&model, &grid, 0.0);

        assert!(mesh.vertex_count() > 0, "no vertices produced");
        assert!(mesh.triangle_count() > 0, "no triangles produced");
        // Every index must reference a real vertex.
        let vmax = mesh.vertex_count() as u32;
        assert!(mesh.indices.iter().all(|&i| i < vmax), "dangling index");

        // The interface is at the field's zero set; recover its isovalue at a
        // surface point and confirm the meshed vertices cluster near that level.
        // The flat contact is ~z=1 in world; vertices' mean z should be near 1.
        let mean_z: f64 = mesh
            .positions
            .chunks_exact(3)
            .map(|p| p[2] as f64)
            .sum::<f64>()
            / mesh.vertex_count() as f64;
        assert!(
            (mean_z - 1.0).abs() < 0.15,
            "sheet not near z=1: mean_z={mean_z}"
        );
    }

    #[test]
    fn empty_when_isovalue_outside_field_range() {
        let model = flat_model();
        // Far above any plausible field value over this box → no crossings.
        let grid = MeshGrid::new([0.0, 0.0, 0.9], [1.0, 1.0, 1.1], 8);
        let mesh = dual_contour(&model, &grid, 1e6);
        assert_eq!(mesh.triangle_count(), 0);
    }
}
