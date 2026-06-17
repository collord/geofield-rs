//! Inputs (surface points, orientations, series) and the coordinate `Transform`
//! (rescaling) per KERNEL_MATH.md §5.1.
//!
//! All kernel math runs in the normalized box. The `Transform` here must match
//! gempy's `Transform.from_input_points` center/scale exactly or the field shape
//! drifts (§5.1 trap).

/// A surface point: a location belonging to an interface.
///
/// The cokriging constraint is on the **increment** rest − ref within an
/// interface, so each surface point records which interface it belongs to; the
/// per-interface reference point is chosen at the `Series` level.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SurfacePoint {
    pub xyz: [f64; 3],
    pub interface_id: u32,
}

impl SurfacePoint {
    pub fn new(xyz: [f64; 3], interface_id: u32) -> Self {
        Self { xyz, interface_id }
    }
}

/// An orientation: a location carrying a pole (gradient direction) vector.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Orientation {
    pub xyz: [f64; 3],
    pub pole: [f64; 3],
}

impl Orientation {
    pub fn new(xyz: [f64; 3], pole: [f64; 3]) -> Self {
        Self { xyz, pole }
    }
}

/// A single series: its surface points, orientations, and the chosen reference
/// surface point per interface (index into `surface_points`).
#[derive(Debug, Clone)]
pub struct Series {
    pub surface_points: Vec<SurfacePoint>,
    pub orientations: Vec<Orientation>,
    /// For each interface id, the index (into `surface_points`) of its reference
    /// point. Keyed by interface id.
    pub reference_per_interface: std::collections::BTreeMap<u32, usize>,
}

impl Series {
    /// Build a series, choosing the **first** surface point seen for each
    /// interface (in input order) as that interface's reference.
    pub fn new(surface_points: Vec<SurfacePoint>, orientations: Vec<Orientation>) -> Self {
        let mut reference_per_interface = std::collections::BTreeMap::new();
        for (i, sp) in surface_points.iter().enumerate() {
            reference_per_interface.entry(sp.interface_id).or_insert(i);
        }
        Self {
            surface_points,
            orientations,
            reference_per_interface,
        }
    }
}

/// Coordinate rescaling (KERNEL_MATH.md §5.1).
///
/// Maps raw world coordinates into the isotropic normalized box via
/// `x_norm = scale * (x − center)` (SRT order, rotation = 0). With the gempy
/// `main` normalization, the extreme input point maps to ±0.25 per axis.
///
/// `scale` is a single isotropic scalar (smallest per-axis scale, from
/// `anisotropic_limit = [1,1,1]`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Transform {
    pub center: [f64; 3],
    /// Isotropic scale: one scalar applied to all three axes.
    pub scale: f64,
}

impl Transform {
    /// Build the rescaling from the concatenation of all surface-point and
    /// orientation xyz, per §5.1:
    ///
    /// ```text
    /// max_coord   = max over points (per axis)
    /// min_coord   = min over points (per axis)
    /// range_coord = 2 * (max_coord − min_coord)   # per axis; 0 → 1e-10
    /// scale_axis  = 1 / range_coord               # per axis
    /// scale       = min over axes (isotropic cube)
    /// center      = (max_coord + min_coord) / 2
    /// ```
    pub fn from_input_points(surface_points: &[SurfacePoint], orientations: &[Orientation]) -> Self {
        let mut min_coord = [f64::INFINITY; 3];
        let mut max_coord = [f64::NEG_INFINITY; 3];

        let mut consider = |xyz: &[f64; 3]| {
            for k in 0..3 {
                if xyz[k] < min_coord[k] {
                    min_coord[k] = xyz[k];
                }
                if xyz[k] > max_coord[k] {
                    max_coord[k] = xyz[k];
                }
            }
        };
        for sp in surface_points {
            consider(&sp.xyz);
        }
        for o in orientations {
            consider(&o.xyz);
        }

        assert!(
            min_coord[0].is_finite(),
            "Transform::from_input_points: no input points"
        );

        let mut center = [0.0; 3];
        // Smallest per-axis scale → isotropic cube (anisotropic_limit = [1,1,1]).
        let mut scale = f64::INFINITY;
        for k in 0..3 {
            center[k] = (max_coord[k] + min_coord[k]) / 2.0;
            let mut range_coord = 2.0 * (max_coord[k] - min_coord[k]);
            if range_coord == 0.0 {
                range_coord = 1e-10;
            }
            let scale_axis = 1.0 / range_coord;
            if scale_axis < scale {
                scale = scale_axis;
            }
        }

        Self { center, scale }
    }

    /// World → normalized: `x_norm = scale * (x − center)`.
    #[inline]
    pub fn apply(&self, xyz: [f64; 3]) -> [f64; 3] {
        [
            self.scale * (xyz[0] - self.center[0]),
            self.scale * (xyz[1] - self.center[1]),
            self.scale * (xyz[2] - self.center[2]),
        ]
    }

    /// Normalized → world: inverse of `apply`.
    #[inline]
    pub fn unapply(&self, xyz_norm: [f64; 3]) -> [f64; 3] {
        [
            xyz_norm[0] / self.scale + self.center[0],
            xyz_norm[1] / self.scale + self.center[1],
            xyz_norm[2] / self.scale + self.center[2],
        ]
    }

    /// Transform a gradient/pole by the inverse-transpose of the linear (3×3)
    /// part (§5.1). For an isotropic scale `s` with no rotation the linear part
    /// is `s·I`, so its inverse-transpose is `(1/s)·I` — i.e. world gradients
    /// scale by `1/s` into the normalized box. (Magnitude is *not* preserved in
    /// general; it is preserved only up to the uniform `1/s` factor. Pole-unit /
    /// RHS-scaling questions are resolved later against the fixture, §7.)
    #[inline]
    pub fn apply_gradient(&self, g: [f64; 3]) -> [f64; 3] {
        [g[0] / self.scale, g[1] / self.scale, g[2] / self.scale]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rescale_unrescale_roundtrip() {
        let sps = vec![
            SurfacePoint::new([0.0, 0.0, 0.0], 0),
            SurfacePoint::new([100.0, 50.0, 10.0], 0),
            SurfacePoint::new([40.0, 20.0, 5.0], 0),
        ];
        let oris = vec![Orientation::new([30.0, 10.0, 2.0], [0.0, 0.0, 1.0])];
        let t = Transform::from_input_points(&sps, &oris);
        for p in [[0.0, 0.0, 0.0], [100.0, 50.0, 10.0], [13.0, 7.0, 1.0]] {
            let back = t.unapply(t.apply(p));
            for k in 0..3 {
                assert!((back[k] - p[k]).abs() < 1e-9, "roundtrip failed at {p:?}");
            }
        }
    }

    #[test]
    fn scale_is_isotropic_and_extreme_maps_to_quarter() {
        // Largest extent is on x (0..100). The isotropic scale is set by that
        // axis: scale = 1 / (2*100) = 1/200. The extreme x point should map to
        // ±0.25; other axes map to smaller magnitudes (isotropic cube).
        let sps = vec![
            SurfacePoint::new([0.0, 0.0, 0.0], 0),
            SurfacePoint::new([100.0, 50.0, 10.0], 0),
        ];
        let oris = vec![];
        let t = Transform::from_input_points(&sps, &oris);
        assert!((t.scale - 1.0 / 200.0).abs() < 1e-15, "scale={}", t.scale);
        assert_eq!(t.center, [50.0, 25.0, 5.0]);

        let hi = t.apply([100.0, 50.0, 10.0]);
        assert!((hi[0] - 0.25).abs() < 1e-15, "x extreme should map to +0.25");
        // y/z extents are smaller, so they land strictly inside ±0.25.
        assert!(hi[1].abs() < 0.25 && hi[2].abs() < 0.25);
    }

    #[test]
    fn degenerate_axis_does_not_blow_up() {
        // A flat (zero-extent) z axis must not produce inf scale; the floored
        // range_coord = 1e-10 gives a huge per-axis scale, but the *isotropic*
        // scale is the minimum (set by a non-degenerate axis), so it stays sane.
        let sps = vec![
            SurfacePoint::new([0.0, 0.0, 5.0], 0),
            SurfacePoint::new([10.0, 10.0, 5.0], 0),
        ];
        let t = Transform::from_input_points(&sps, &[]);
        assert!(t.scale.is_finite() && t.scale > 0.0);
        assert!((t.scale - 1.0 / 20.0).abs() < 1e-15);
    }

    #[test]
    fn isotropic_gradient_scales_by_inverse_scale() {
        let sps = vec![
            SurfacePoint::new([0.0, 0.0, 0.0], 0),
            SurfacePoint::new([10.0, 10.0, 10.0], 0),
        ];
        let t = Transform::from_input_points(&sps, &[]);
        let g = t.apply_gradient([0.0, 0.0, 1.0]);
        assert!((g[2] - 1.0 / t.scale).abs() < 1e-12);
        assert_eq!(g[0], 0.0);
        assert_eq!(g[1], 0.0);
    }
}
