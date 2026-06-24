//! Shared `#[cfg(test)]` mesh-invariant helpers used by BOTH [`crate::mesh_clip`] and
//! [`crate::stack`] tests, so the watertightness criteria are defined ONCE.
//!
//! Two criteria live here, deliberately distinct:
//!   * [`assert_volume_watertight`] — a CLOSED solid: every edge incidence is exactly 2, except a
//!     legitimately-open boundary (the `on_rim` predicate) where incidence 1 is allowed; nothing
//!     may ever exceed 2 (a non-manifold crack). This is the closed-solid mesher's invariant.
//!   * [`assert_seam_watertight`] — a contact surface C terminating against a through-wall W: the
//!     seam is a 3-way junction (≥1 incidence-3 edge), incidence-1 edges live only on the true rim,
//!     and there is no non-manifold edge OFF the seam. This is the surface-corefinement invariant,
//!     NOT "incidence 2 everywhere".
//!
//! The single primitive underneath both is [`incidence`] — the undirected-edge counter that every
//! ad-hoc loop in the two modules used to re-implement.

use std::collections::{HashMap, HashSet};

use crate::mesh::Mesh;
use crate::mesh_clip::ClipResult;

/// Undirected-edge incidence over an indexed triangle soup: `(min(a,b), max(a,b)) -> count`. The
/// one canonical edge counter — every watertightness check in the crate's tests is built on this.
pub(crate) fn incidence(tris: &[[usize; 3]]) -> HashMap<(usize, usize), u32> {
    let mut inc: HashMap<(usize, usize), u32> = HashMap::new();
    for t in tris {
        for &(a, b) in &[(t[0], t[1]), (t[1], t[2]), (t[2], t[0])] {
            let e = if a < b { (a, b) } else { (b, a) };
            *inc.entry(e).or_insert(0) += 1;
        }
    }
    inc
}

/// Cast the crate's flat GPU-ready [`Mesh`] (f32 positions, u32 indices) into the `[f64;3]` /
/// `[usize;3]` soup the incidence helpers take. No dedup — a single contoured `Mesh` already
/// shares its vertices by index, so its edges are real as-is.
pub(crate) fn mesh_soup(m: &Mesh) -> (Vec<[f64; 3]>, Vec<[usize; 3]>) {
    let pos = (0..m.vertex_count())
        .map(|i| [m.positions[3 * i] as f64, m.positions[3 * i + 1] as f64, m.positions[3 * i + 2] as f64])
        .collect();
    let tris = m
        .indices
        .chunks_exact(3)
        .map(|t| [t[0] as usize, t[1] as usize, t[2] as usize])
        .collect();
    (pos, tris)
}

/// CLOSED-solid watertightness: every edge incidence is exactly 2, EXCEPT edges whose BOTH
/// endpoints satisfy `on_rim` (a legitimately-open boundary — e.g. the domain box face) may be
/// incidence 1; nothing may ever exceed 2 (a non-manifold crack). Use `&|_| false` for a fully
/// closed solid (cap=true, no open boundary).
pub(crate) fn assert_volume_watertight(
    pos: &[[f64; 3]],
    tris: &[[usize; 3]],
    on_rim: &dyn Fn([f64; 3]) -> bool,
) {
    let inc = incidence(tris);
    let nm = inc
        .iter()
        .filter(|(_, &c)| c > 2)
        .map(|(&(a, b), &c)| (a, b, c))
        .next();
    assert!(nm.is_none(), "non-manifold edge (incidence > 2): {nm:?}");

    let open = inc
        .iter()
        .filter(|(_, &c)| c == 1)
        .filter(|((a, b), _)| !(on_rim(pos[*a]) && on_rim(pos[*b])))
        .map(|(&(a, b), _)| (a, b, pos[a], pos[b]))
        .next();
    assert!(open.is_none(), "open (incidence-1) edge off the rim — a hole: {open:?}");
}

/// Convenience: [`assert_volume_watertight`] for a single flat [`Mesh`].
pub(crate) fn assert_mesh_watertight(m: &Mesh, on_rim: &dyn Fn([f64; 3]) -> bool) {
    let (pos, tris) = mesh_soup(m);
    assert_volume_watertight(&pos, &tris, on_rim);
}

/// Merge a [`ClipResult`]'s C and W into ONE index space, collapsing each `shared` seam pair to a
/// single vertex, so cross-surface watertightness can be checked by edge incidence. Returns
/// `(positions, triangles)`. Uses only the public fields of [`ClipResult`].
pub(crate) fn merge(res: &ClipResult) -> (Vec<[f64; 3]>, Vec<[usize; 3]>) {
    let cn = res.c.vertices.len();
    // W vertices are offset by cn; then each shared (ci, wi) is unified by remapping wi→ci.
    let mut remap: HashMap<usize, usize> = HashMap::new();
    for &(ci, wi) in &res.shared {
        remap.insert(cn + wi, ci);
    }
    let mut positions = res.c.vertices.clone();
    positions.extend(res.w.vertices.iter().copied());
    let resolve = |v: usize| *remap.get(&v).unwrap_or(&v);
    let mut tris = Vec::new();
    for t in &res.c.triangles {
        tris.push([t[0], t[1], t[2]]);
    }
    for t in &res.w.triangles {
        tris.push([resolve(cn + t[0]), resolve(cn + t[1]), resolve(cn + t[2])]);
    }
    (positions, tris)
}

/// SURFACE-seam watertightness for a clipped C∪W, parameterized by the fixture's true outer rim
/// (`on_frame`). Asserts: (1) no torn seam — incidence-1 edges live only on the rim; (2) no
/// non-manifold edge OFF the seam; (3) the seam is a real 3-way junction (≥1 incidence-3 edge),
/// and no seam edge exceeds incidence 3. This is the correct criterion for a contact terminating
/// against a through-wall (NOT "incidence 2 everywhere", which is [`assert_volume_watertight`]).
pub(crate) fn assert_seam_watertight(res: &ClipResult, on_frame: &dyn Fn([f64; 3]) -> bool) {
    assert!(!res.shared.is_empty(), "expected a non-empty seam");
    assert!(!res.c.triangles.is_empty() && !res.w.triangles.is_empty(), "empty output");
    let (pos, tris) = merge(res);
    let inc = incidence(&tris);

    let torn = inc
        .iter()
        .filter(|(_, &c)| c == 1)
        .filter(|((a, b), _)| !(on_frame(pos[*a]) && on_frame(pos[*b])))
        .count();
    assert_eq!(torn, 0, "torn seam edges off the rim: {torn}");

    let seam_verts: HashSet<usize> = res.shared.iter().map(|&(ci, _)| ci).collect();
    let is_seam_edge = |a: usize, b: usize| seam_verts.contains(&a) && seam_verts.contains(&b);

    let bad_nm = inc
        .iter()
        .filter(|(_, &c)| c > 2)
        .filter(|((a, b), _)| !is_seam_edge(*a, *b))
        .count();
    assert_eq!(bad_nm, 0, "non-manifold edges off the seam: {bad_nm}");

    let over = inc
        .iter()
        .filter(|((a, b), _)| is_seam_edge(*a, *b))
        .filter(|(_, &c)| c > 3)
        .count();
    assert_eq!(over, 0, "a seam edge has incidence > 3 (self-overlap): {over}");
    let three = inc
        .iter()
        .filter(|((a, b), _)| is_seam_edge(*a, *b))
        .filter(|(_, &c)| c == 3)
        .count();
    assert!(three > 0, "no 3-way seam junction edges — C did not terminate on the seam");
}
