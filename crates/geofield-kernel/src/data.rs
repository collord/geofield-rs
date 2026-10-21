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
/// Maps raw world coordinates into the normalized box via
/// `x_norm = scale ⊙ (x − center)` (SRT order, rotation = 0) — a **per-axis** scale.
/// With equal data spans on every axis the scale is isotropic and the extreme input
/// point maps to ±0.25 per axis; otherwise gempy's `_adjust_scale_to_limit_ratio` can
/// leave it anisotropic (see [`adjust_scale_to_limit_ratio`], the §5.1 landmine).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Transform {
    pub center: [f64; 3],
    /// Per-axis scale `[sx, sy, sz]`.
    pub scale: [f64; 3],
}

/// gempy's `Transform._adjust_scale_to_limit_ratio(s, anisotropic_limit=[1,1,1])`,
/// transcribed verbatim (KERNEL_MATH.md §5.1).
///
/// ⚠ It is NOT "take the minimum". The six ratios are computed once from the **input**
/// scales, but the assignments mutate `s` in place and later ones read the mutated
/// values, so the result depends on axis order and can stay anisotropic: per-axis
/// scales `[1/2000, 1/700, 1/760]` come out as `[1/2000, 1/760, 1/2000]`. That is the
/// reference behavior; reproduce it, don't "fix" it.
pub fn adjust_scale_to_limit_ratio(mut s: [f64; 3]) -> [f64; 3] {
    const LIMIT: f64 = 1.0;
    let ratios = [s[0] / s[1], s[0] / s[2], s[1] / s[0], s[1] / s[2], s[2] / s[0], s[2] / s[1]];
    if ratios[0] > LIMIT {
        s[0] = s[1] * LIMIT;
    }
    if ratios[1] > LIMIT {
        s[0] = s[2] * LIMIT;
    }
    if ratios[2] > LIMIT {
        s[1] = s[0] * LIMIT;
    }
    if ratios[3] > LIMIT {
        s[1] = s[2] * LIMIT;
    }
    if ratios[4] > LIMIT {
        s[2] = s[0] * LIMIT;
    }
    if ratios[5] > LIMIT {
        s[2] = s[1] * LIMIT;
    }
    s
}

impl Transform {
    /// Build the rescaling from the concatenation of all surface-point and
    /// orientation xyz, per §5.1 (gempy `Transform.from_input_points`):
    ///
    /// ```text
    /// max_coord   = max over points (per axis)
    /// min_coord   = min over points (per axis)
    /// range_coord = 2 * (max_coord − min_coord)   # per axis; 0 → 1e-10
    /// scale       = adjust_scale_to_limit_ratio(1 / range_coord)
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
        let mut scaling_factors = [0.0; 3];
        for k in 0..3 {
            center[k] = (max_coord[k] + min_coord[k]) / 2.0;
            let mut range_coord = 2.0 * (max_coord[k] - min_coord[k]);
            if range_coord == 0.0 {
                range_coord = 1e-10;
            }
            scaling_factors[k] = 1.0 / range_coord;
        }

        Self { center, scale: adjust_scale_to_limit_ratio(scaling_factors) }
    }

    /// Whether all three axes share one scale (the common case: equal data spans).
    pub fn is_isotropic(&self) -> bool {
        self.scale[0] == self.scale[1] && self.scale[1] == self.scale[2]
    }

    /// World → normalized: `x_norm = scale ⊙ (x − center)`.
    #[inline]
    pub fn apply(&self, xyz: [f64; 3]) -> [f64; 3] {
        [
            self.scale[0] * (xyz[0] - self.center[0]),
            self.scale[1] * (xyz[1] - self.center[1]),
            self.scale[2] * (xyz[2] - self.center[2]),
        ]
    }

    /// Normalized → world: inverse of `apply`.
    #[inline]
    pub fn unapply(&self, xyz_norm: [f64; 3]) -> [f64; 3] {
        [
            xyz_norm[0] / self.scale[0] + self.center[0],
            xyz_norm[1] / self.scale[1] + self.center[1],
            xyz_norm[2] / self.scale[2] + self.center[2],
        ]
    }

    /// Transform an orientation pole into the normalized box — gempy
    /// `Transform.transform_gradient(g, preserve_magnitude=True)` (§5.1): multiply by the
    /// inverse-transpose of the linear part (`diag(1/scale)`), then rescale back to the
    /// input magnitude. Only the **direction** changes, and only when the scale is
    /// anisotropic (isotropic → the input pole, up to rounding). This is what gempy puts
    /// in the RHS (`dip_gradients`). A zero pole is returned unchanged (gempy would
    /// produce NaN; geofield's ABI never passes one).
    #[inline]
    pub fn apply_gradient(&self, g: [f64; 3]) -> [f64; 3] {
        let t = [g[0] * (1.0 / self.scale[0]), g[1] * (1.0 / self.scale[1]), g[2] * (1.0 / self.scale[2])];
        let g_mag = (g[0] * g[0] + g[1] * g[1] + g[2] * g[2]).sqrt();
        if g_mag == 0.0 {
            return g;
        }
        let t_mag = (t[0] * t[0] + t[1] * t[1] + t[2] * t[2]).sqrt();
        let ratio = t_mag / g_mag;
        [t[0] / ratio, t[1] / ratio, t[2] / ratio]
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
    fn largest_span_on_x_gives_isotropic_scale_and_quarter_extreme() {
        // x has the largest span (0..100): every other axis is pulled down to x's scale,
        // 1/(2*100), so the box is isotropic and the extreme x point maps to +0.25.
        let sps = vec![
            SurfacePoint::new([0.0, 0.0, 0.0], 0),
            SurfacePoint::new([100.0, 50.0, 10.0], 0),
        ];
        let t = Transform::from_input_points(&sps, &[]);
        assert_eq!(t.scale, [1.0 / 200.0; 3]);
        assert!(t.is_isotropic());
        assert_eq!(t.center, [50.0, 25.0, 5.0]);

        let hi = t.apply([100.0, 50.0, 10.0]);
        assert!((hi[0] - 0.25).abs() < 1e-15, "x extreme should map to +0.25");
        assert!(hi[1].abs() < 0.25 && hi[2].abs() < 0.25);
    }

    /// The gempy order-dependence, pinned with the exact case observed in a live gempy
    /// dump (1000 × 350 × 380 span → scale [0.0005, 0.00132, 0.0005]).
    #[test]
    fn gempy_limit_ratio_is_order_dependent_not_min() {
        let s = adjust_scale_to_limit_ratio([1.0 / 2000.0, 1.0 / 700.0, 1.0 / 760.0]);
        assert_eq!(s, [1.0 / 2000.0, 1.0 / 760.0, 1.0 / 2000.0]);
        // Already-isotropic and min-on-x inputs collapse to the minimum.
        assert_eq!(adjust_scale_to_limit_ratio([0.5, 0.5, 0.5]), [0.5; 3]);
        assert_eq!(adjust_scale_to_limit_ratio([0.1, 0.2, 0.3]), [0.1; 3]);
        assert_eq!(adjust_scale_to_limit_ratio([0.3, 0.2, 0.1]), [0.1, 0.1, 0.1]);
        assert_eq!(adjust_scale_to_limit_ratio([0.2, 0.3, 0.1]), [0.1, 0.1, 0.1]);
        assert_eq!(adjust_scale_to_limit_ratio([0.2, 0.1, 0.3]), [0.1, 0.1, 0.1]);
        // Smallest scale on x with s_y > s_z: y is pulled to z's (unmodified) scale, not x's.
        assert_eq!(adjust_scale_to_limit_ratio([0.1, 0.3, 0.2]), [0.1, 0.2, 0.1]);
    }

    #[test]
    fn degenerate_axis_does_not_blow_up() {
        // A flat (zero-extent) z axis floors range_coord to 1e-10 (a huge per-axis scale);
        // the limit-ratio pass pulls it back to the finite axes' scale.
        let sps = vec![
            SurfacePoint::new([0.0, 0.0, 5.0], 0),
            SurfacePoint::new([10.0, 10.0, 5.0], 0),
        ];
        let t = Transform::from_input_points(&sps, &[]);
        assert_eq!(t.scale, [1.0 / 20.0; 3]);
    }

    #[test]
    fn gradient_keeps_magnitude_and_tilts_only_when_anisotropic() {
        let iso = Transform { center: [0.0; 3], scale: [0.1; 3] };
        let g = [0.3, -0.2, 0.9];
        let gi = iso.apply_gradient(g);
        for k in 0..3 {
            assert!((gi[k] - g[k]).abs() < 1e-15, "isotropic must keep the pole");
        }
        // gempy's observed case: az 80 / dip 8 pole under scale [1/2000, 1/760, 1/2000].
        let an = Transform { center: [0.0; 3], scale: [1.0 / 2000.0, 1.0 / 760.0, 1.0 / 2000.0] };
        let pole = [0.137059, 0.02416716, 0.990268];
        let ga = an.apply_gradient(pole);
        let mag = |v: [f64; 3]| (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
        assert!((mag(ga) - mag(pole)).abs() < 1e-12, "magnitude preserved");
        // y shrinks by 760/2000 relative to x and z (then renormalized).
        assert!((ga[1] / ga[2] - (pole[1] / pole[2]) * (760.0 / 2000.0)).abs() < 1e-12);
        assert!((ga[0] - 0.13709).abs() < 5e-5 && (ga[1] - 0.00919).abs() < 5e-5);
    }
}
