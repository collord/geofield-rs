//! A minimal, byte-oriented Wavefront `.obj` parser for IMPORTED mesh surfaces (feature 004).
//!
//! Scope is deliberately tiny (Principle VI): only `v` (vertices), `vn` (vertex normals), and `f`
//! (faces) are read — enough to build a [`crate::octree::TriMesh`] and fit a field through its
//! vertices+normals. It consumes **bytes** (`&[u8]`), never a filesystem path: the browser reads the
//! file and passes the buffer across the wasm ABI, so this stays wasm-safe with no `std::fs` and no
//! native-only dependency (Constitution V). BSD, in the `geofield` engine — no new crate.
//!
//! Robustness rules (feature 004 FR-008/FR-011, data-model validation):
//! - Faces may be `f a b c`, `f a/t b/t c/t`, `f a/t/n …`, or `f a//n …`; only the vertex index is
//!   used. Negative (relative) indices are supported. Polygons with >3 vertices are fan-triangulated.
//! - Duplicate vertices are welded (bit-tight, by quantized key) so a split-vertex export still forms
//!   connected components for winding normalization; face indices are remapped to the welded set.
//! - Zero-area (degenerate) triangles are dropped.
//! - A parse that yields no non-degenerate triangle is an [`ObjError`] (caller rejects the import),
//!   never a silent empty mesh.
//!
//! Winding normalization, hole/non-manifold DETECTION, and the `TriMesh`/`ClipMesh` construction live
//! in the caller (`stack.rs` ingress); this module only turns bytes into deduplicated geometry.

/// Deduplicated triangle geometry parsed from an `.obj` byte buffer.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ObjMesh {
    /// Welded vertex positions.
    pub vertices: Vec<[f64; 3]>,
    /// Triangle vertex indices into `vertices` (already fan-triangulated, degenerate faces dropped).
    pub triangles: Vec<[usize; 3]>,
    /// Per-vertex normals, parallel to `vertices`, IFF the file supplied usable `vn` for every kept
    /// vertex; `None` otherwise (the caller then derives orientation from face winding).
    pub normals: Option<Vec<[f64; 3]>>,
}

/// Why an `.obj` buffer could not become a usable mesh (FR-011 — reject, don't emit garbage).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ObjError {
    /// The buffer is not valid UTF-8 (obj is a text format).
    NotUtf8,
    /// No vertices were found.
    NoVertices,
    /// A face referenced a vertex index outside the vertex list (0, or out of range).
    BadFaceIndex(String),
    /// Parsing succeeded but left no non-degenerate triangle to mesh.
    NoTriangles,
}

impl std::fmt::Display for ObjError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ObjError::NotUtf8 => write!(f, "imported mesh is not valid UTF-8 text (.obj expected)"),
            ObjError::NoVertices => write!(f, "imported mesh has no vertices"),
            ObjError::BadFaceIndex(s) => write!(f, "imported mesh face references an invalid vertex index: {s}"),
            ObjError::NoTriangles => write!(f, "imported mesh has no non-degenerate triangles to mesh"),
        }
    }
}

/// Quantize a coordinate to a bit-tight integer key for duplicate-vertex welding. `1e7` matches the
/// engine's `combinatorial_key` position quantum; welding here only merges genuinely-coincident
/// split vertices, so exactness is not required — only determinism.
fn key(p: [f64; 3]) -> (i64, i64, i64) {
    let q = |v: f64| (v * 1e7).round() as i64;
    (q(p[0]), q(p[1]), q(p[2]))
}

fn is_degenerate(a: [f64; 3], b: [f64; 3], c: [f64; 3]) -> bool {
    // Zero cross-product magnitude ⇒ collinear ⇒ zero area. Squared length vs a relative epsilon of
    // the edge lengths so scale doesn't matter.
    let ab = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
    let ac = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
    let cx = [
        ab[1] * ac[2] - ab[2] * ac[1],
        ab[2] * ac[0] - ab[0] * ac[2],
        ab[0] * ac[1] - ab[1] * ac[0],
    ];
    let area2 = cx[0] * cx[0] + cx[1] * cx[1] + cx[2] * cx[2];
    let s = (ab[0] * ab[0] + ab[1] * ab[1] + ab[2] * ab[2])
        * (ac[0] * ac[0] + ac[1] * ac[1] + ac[2] * ac[2]);
    area2 <= s * 1e-20 || area2 == 0.0
}

/// Resolve an obj index token (1-based, or negative-relative) against the current count. Returns a
/// 0-based index, or `None` if out of range / zero.
fn resolve_index(tok: &str, count: usize) -> Option<usize> {
    // A face vertex token is `v`, `v/t`, `v/t/n`, or `v//n`; only the leading `v` matters here.
    let first = tok.split('/').next()?;
    let raw: i64 = first.trim().parse().ok()?;
    if raw > 0 {
        let i = (raw - 1) as usize;
        (i < count).then_some(i)
    } else if raw < 0 {
        let i = count as i64 + raw; // -1 = last
        (i >= 0).then_some(i as usize)
    } else {
        None // 0 is invalid in obj
    }
}

/// Extract the normal index from a face token `v/t/n` or `v//n`, if present (1-based / negative).
fn normal_index(tok: &str, count: usize) -> Option<usize> {
    let n = tok.split('/').nth(2)?;
    let raw: i64 = n.trim().parse().ok()?;
    if raw > 0 {
        let i = (raw - 1) as usize;
        (i < count).then_some(i)
    } else if raw < 0 {
        let i = count as i64 + raw;
        (i >= 0).then_some(i as usize)
    } else {
        None
    }
}

/// Parse a Wavefront `.obj` byte buffer into deduplicated triangle geometry.
///
/// Welds duplicate vertices, fan-triangulates polygons, drops degenerate triangles, and carries
/// per-vertex normals only if the file provided them for every kept vertex. See module docs.
pub fn parse_obj(bytes: &[u8]) -> Result<ObjMesh, ObjError> {
    let text = std::str::from_utf8(bytes).map_err(|_| ObjError::NotUtf8)?;

    let mut raw_v: Vec<[f64; 3]> = Vec::new();
    let mut raw_vn: Vec<[f64; 3]> = Vec::new();
    // Face records BEFORE welding: (vertex_idx, optional normal_idx) per corner.
    let mut faces: Vec<Vec<(usize, Option<usize>)>> = Vec::new();

    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut it = line.split_whitespace();
        match it.next() {
            Some("v") => {
                let c: Vec<f64> = it.filter_map(|t| t.parse().ok()).collect();
                if c.len() >= 3 {
                    raw_v.push([c[0], c[1], c[2]]);
                }
            }
            Some("vn") => {
                let c: Vec<f64> = it.filter_map(|t| t.parse().ok()).collect();
                if c.len() >= 3 {
                    raw_vn.push([c[0], c[1], c[2]]);
                }
            }
            Some("f") => {
                let toks: Vec<&str> = it.collect();
                let mut corners: Vec<(usize, Option<usize>)> = Vec::with_capacity(toks.len());
                for tok in &toks {
                    let vi = resolve_index(tok, raw_v.len())
                        .ok_or_else(|| ObjError::BadFaceIndex((*tok).to_string()))?;
                    let ni = normal_index(tok, raw_vn.len());
                    corners.push((vi, ni));
                }
                if corners.len() >= 3 {
                    faces.push(corners);
                }
            }
            _ => {} // ignore vt, mtllib, usemtl, o, g, s, …
        }
    }

    if raw_v.is_empty() {
        return Err(ObjError::NoVertices);
    }

    // Pass 1: weld vertices by quantized key (remember one raw normal per welded vertex, first seen),
    // and rewrite each face's corners to welded indices. Keep this phase separate from the geometry
    // reads below so `welded` is not borrowed mutably while we index it.
    let mut welded: Vec<[f64; 3]> = Vec::new();
    let mut welded_normal: Vec<Option<[f64; 3]>> = Vec::new();
    let mut map: std::collections::HashMap<(i64, i64, i64), usize> = std::collections::HashMap::new();
    let mut welded_faces: Vec<Vec<usize>> = Vec::with_capacity(faces.len());
    for face in &faces {
        let mut ws = Vec::with_capacity(face.len());
        for &(vi, ni) in face {
            let p = raw_v[vi];
            let w = match map.get(&key(p)) {
                Some(&w) => {
                    if welded_normal[w].is_none() {
                        welded_normal[w] = ni.map(|n| raw_vn[n]);
                    }
                    w
                }
                None => {
                    let w = welded.len();
                    welded.push(p);
                    welded_normal.push(ni.map(|n| raw_vn[n]));
                    map.insert(key(p), w);
                    w
                }
            };
            ws.push(w);
        }
        welded_faces.push(ws);
    }

    // Pass 2: fan-triangulate, dropping repeated-corner and zero-area triangles.
    let mut triangles: Vec<[usize; 3]> = Vec::new();
    for ws in &welded_faces {
        for i in 1..ws.len() - 1 {
            let (a, b, c) = (ws[0], ws[i], ws[i + 1]);
            if a == b || b == c || a == c {
                continue;
            }
            if is_degenerate(welded[a], welded[b], welded[c]) {
                continue;
            }
            triangles.push([a, b, c]);
        }
    }

    if triangles.is_empty() {
        return Err(ObjError::NoTriangles);
    }

    // Normals only if every kept (referenced) vertex got one. Unreferenced welded vertices are
    // dropped implicitly by the mesher, so we only require normals on referenced ones.
    let mut referenced = vec![false; welded.len()];
    for t in &triangles {
        for &i in t {
            referenced[i] = true;
        }
    }
    let all_have_normals = referenced
        .iter()
        .enumerate()
        .all(|(i, &r)| !r || welded_normal[i].is_some());
    let normals = all_have_normals.then(|| {
        welded_normal
            .iter()
            .map(|n| n.unwrap_or([0.0, 0.0, 1.0]))
            .collect()
    });

    Ok(ObjMesh { vertices: welded, triangles, normals })
}

#[cfg(test)]
mod tests {
    use super::*;

    const SPAN: &[u8] = include_bytes!("../tests/fixtures/mesh-span-domain.obj");
    const UNDER: &[u8] = include_bytes!("../tests/fixtures/mesh-under-covering.obj");
    const BAD_WIND: &[u8] = include_bytes!("../tests/fixtures/mesh-bad-winding.obj");
    const DEGENERATE: &[u8] = include_bytes!("../tests/fixtures/mesh-degenerate.obj");

    #[test]
    fn parses_span_domain_quad_with_normals() {
        let m = parse_obj(SPAN).expect("valid");
        assert_eq!(m.vertices.len(), 4, "4 corners, none welded away");
        assert_eq!(m.triangles.len(), 2, "two triangles");
        // vn 0 0 1 supplied via `f a//n` → normals present for every referenced vertex.
        let n = m.normals.expect("normals carried from vn");
        assert_eq!(n.len(), 4);
        assert!(n.iter().all(|v| (v[2] - 1.0).abs() < 1e-12), "all +z");
    }

    #[test]
    fn parses_under_covering_and_bounds_are_inside_domain() {
        let m = parse_obj(UNDER).expect("valid");
        assert_eq!(m.triangles.len(), 2);
        let (mut lo, mut hi) = ([f64::MAX; 3], [f64::MIN; 3]);
        for v in &m.vertices {
            for k in 0..3 {
                lo[k] = lo[k].min(v[k]);
                hi[k] = hi[k].max(v[k]);
            }
        }
        assert_eq!(lo, [200.0, 200.0, 500.0]);
        assert_eq!(hi, [800.0, 800.0, 500.0]);
    }

    #[test]
    fn parses_bad_winding_without_normals() {
        // No `vn` in this fixture → normals None (caller derives from winding).
        let m = parse_obj(BAD_WIND).expect("valid geometry despite inconsistent winding");
        assert_eq!(m.triangles.len(), 2);
        assert!(m.normals.is_none(), "no vn lines ⇒ no per-vertex normals");
    }

    #[test]
    fn degenerate_mesh_is_rejected() {
        // Collinear triangle (zero area) + a duplicate vertex → nothing meshable.
        assert_eq!(parse_obj(DEGENERATE), Err(ObjError::NoTriangles));
    }

    #[test]
    fn welds_duplicate_vertices() {
        // Two triangles sharing an edge, written with DUPLICATE vertex rows (split export). Welding
        // must collapse them so the two triangles share indices (a connected component).
        let src = b"v 0 0 0\nv 1 0 0\nv 0 1 0\nv 1 0 0\nv 1 1 0\nv 0 1 0\nf 1 2 3\nf 4 5 6\n";
        let m = parse_obj(src).expect("valid");
        assert_eq!(m.vertices.len(), 4, "6 rows → 4 welded corners");
        assert_eq!(m.triangles.len(), 2);
    }

    #[test]
    fn rejects_empty_and_non_utf8() {
        assert_eq!(parse_obj(b""), Err(ObjError::NoVertices));
        assert_eq!(parse_obj(&[0xff, 0xfe, 0x00]), Err(ObjError::NotUtf8));
    }

    #[test]
    fn supports_slash_face_tokens_and_negative_indices() {
        // `f v/t/n` and negative (relative) indices both resolve to the vertex.
        let src = b"v 0 0 0\nv 1 0 0\nv 0 1 0\nf -3/1/1 -2/1/1 -1/1/1\n";
        let m = parse_obj(src).expect("valid");
        assert_eq!(m.triangles, vec![[0, 1, 2]]);
    }
}
