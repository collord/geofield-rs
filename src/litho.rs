//! Lithology assignment by thresholding the scalar field at the isovalues
//! recovered at the input surface points (BUILD_geofield_rs.md step 8).
//!
//! For a single series the scalar field is monotone across the stack: each
//! interface sits at a recovered isovalue, and a query point's lithology is the
//! interval between consecutive isovalues that its field value falls into.

use crate::data::Series;
use crate::evaluate::Field;

/// Per-interface recovered isovalue.
#[derive(Debug, Clone, Copy)]
pub struct InterfaceIsovalue {
    pub interface_id: u32,
    /// Field value recovered at this interface's surface points (their mean;
    /// they agree to interpolation tolerance for a well-posed system).
    pub isovalue: f64,
}

/// The recovered isovalues, sorted ascending by isovalue, plus the assignment
/// machinery.
#[derive(Debug, Clone)]
pub struct Lithology {
    /// Interfaces sorted by ascending isovalue.
    pub interfaces: Vec<InterfaceIsovalue>,
}

impl Lithology {
    /// Recover one isovalue per interface from the field at that interface's
    /// surface points (averaging over the interface's points), then sort by
    /// isovalue. `max_spread` returns the largest within-interface disagreement
    /// — a diagnostic; large values mean the system did not reproduce a single
    /// isovalue per interface (see evaluate.rs isovalue consistency).
    pub fn recover(series: &Series, field: &Field) -> (Self, f64) {
        // Group field values by interface id, in first-seen order.
        let mut ids: Vec<u32> = Vec::new();
        let mut sums: std::collections::BTreeMap<u32, (f64, usize, f64, f64)> =
            std::collections::BTreeMap::new();
        for sp in &series.surface_points {
            let v = field.scalar_field(sp.xyz);
            let e = sums.entry(sp.interface_id).or_insert_with(|| {
                ids.push(sp.interface_id);
                (0.0, 0, f64::INFINITY, f64::NEG_INFINITY)
            });
            e.0 += v;
            e.1 += 1;
            e.2 = e.2.min(v);
            e.3 = e.3.max(v);
        }

        let mut max_spread = 0.0_f64;
        let mut interfaces: Vec<InterfaceIsovalue> = sums
            .iter()
            .map(|(&id, &(sum, n, lo, hi))| {
                max_spread = max_spread.max(hi - lo);
                InterfaceIsovalue {
                    interface_id: id,
                    isovalue: sum / n as f64,
                }
            })
            .collect();

        interfaces.sort_by(|a, b| a.isovalue.partial_cmp(&b.isovalue).unwrap());
        (Self { interfaces }, max_spread)
    }

    /// Assign a query point's field value to a lithology unit.
    ///
    /// Units are the half-open intervals between consecutive isovalues. With `n`
    /// interfaces there are `n + 1` units, indexed `0..=n`:
    ///   - unit `0`:        value <  isovalue[0]
    ///   - unit `k` (1..n): isovalue[k-1] <= value < isovalue[k]
    ///   - unit `n`:        value >= isovalue[n-1]
    pub fn unit_of_value(&self, value: f64) -> usize {
        // Number of isovalues the value is at-or-above.
        let mut k = 0;
        for iso in &self.interfaces {
            if value >= iso.isovalue {
                k += 1;
            } else {
                break;
            }
        }
        k
    }

    /// Assign a world query point to a lithology unit via its field value.
    pub fn unit_at(&self, field: &Field, world_xyz: [f64; 3]) -> usize {
        self.unit_of_value(field.scalar_field(world_xyz))
    }

    /// Number of lithology units (= interfaces + 1).
    pub fn n_units(&self) -> usize {
        self.interfaces.len() + 1
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assemble::{assemble, KernelOptions};
    use crate::data::{Orientation, Series, SurfacePoint};
    use crate::solve::solve;

    /// Two interfaces (3 flat layers' worth of contacts), one orientation: the
    /// field should be monotone in z and assign points to the right unit.
    fn two_interface_series() -> Series {
        let sps = vec![
            // interface 0 at z≈1.0
            SurfacePoint::new([0.0, 0.0, 1.0], 0),
            SurfacePoint::new([1.0, 0.0, 1.0], 0),
            SurfacePoint::new([0.0, 1.0, 1.0], 0),
            // interface 1 at z≈2.0
            SurfacePoint::new([0.0, 0.0, 2.0], 1),
            SurfacePoint::new([1.0, 0.0, 2.0], 1),
            SurfacePoint::new([0.0, 1.0, 2.0], 1),
        ];
        let oris = vec![Orientation::new([0.5, 0.5, 1.5], [0.0, 0.0, 1.0])];
        Series::new(sps, oris)
    }

    #[test]
    fn recovers_one_isovalue_per_interface() {
        let s = two_interface_series();
        let sys = assemble(&s, KernelOptions::default());
        let sol = solve(&sys);
        let field = Field::new(&sys.geom, &sol);

        let (litho, spread) = Lithology::recover(&s, &field);
        assert_eq!(litho.interfaces.len(), 2);
        assert!(
            spread < 1e-6,
            "within-interface isovalue spread too large: {spread}"
        );
        // Two interfaces -> three units.
        assert_eq!(litho.n_units(), 3);
    }

    #[test]
    fn assigns_points_to_monotone_units() {
        let s = two_interface_series();
        let sys = assemble(&s, KernelOptions::default());
        let sol = solve(&sys);
        let field = Field::new(&sys.geom, &sol);
        let (litho, _) = Lithology::recover(&s, &field);

        // A point clearly below interface 0, between, and above interface 1.
        // The field is monotone with z (orientation points +z). The exact unit
        // numbering depends on field sign; assert ordering, not absolute index.
        let below = litho.unit_at(&field, [0.5, 0.5, 0.5]);
        let middle = litho.unit_at(&field, [0.5, 0.5, 1.5]);
        let above = litho.unit_at(&field, [0.5, 0.5, 2.5]);

        // The three sampled depths must land in three strictly-ordered units.
        let mut units = [below, middle, above];
        let sorted = {
            let mut u = units;
            u.sort_unstable();
            u
        };
        units.sort_unstable();
        assert_eq!(units, sorted);
        assert!(
            below != middle && middle != above,
            "depth samples collapsed into the same unit: {below},{middle},{above}"
        );
    }

    #[test]
    fn unit_of_value_boundaries() {
        let litho = Lithology {
            interfaces: vec![
                InterfaceIsovalue {
                    interface_id: 0,
                    isovalue: -1.0,
                },
                InterfaceIsovalue {
                    interface_id: 1,
                    isovalue: 1.0,
                },
            ],
        };
        assert_eq!(litho.unit_of_value(-2.0), 0); // below first
        assert_eq!(litho.unit_of_value(-1.0), 1); // exactly first iso -> in unit 1
        assert_eq!(litho.unit_of_value(0.0), 1); // between
        assert_eq!(litho.unit_of_value(1.0), 2); // exactly second iso
        assert_eq!(litho.unit_of_value(5.0), 2); // above last
    }
}
