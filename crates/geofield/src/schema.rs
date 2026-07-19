//! Hierarchy UI ↔ core DTOs (feature 001 / `docs/stack_hierarchy_spec.md` §7,
//! contract `specs/001-stratigraphic-hierarchy/contracts/core-api.md`).
//!
//! **This module is the single source of truth for the hierarchy grammar that
//! crosses the boundary** (Constitution Principle III). The TS side is a faithful
//! hand-mirror of these shapes (`web3/src/HierarchyPanel/dto.ts`) until `ts-rs`
//! auto-generation lands (deferred; see research.md R1 update). The
//! [`tests`] round-trip pins the serde contract so the Rust shape cannot drift
//! silently.
//!
//! ## Scope (MVP)
//!
//! Only the element kind the model currently supports — `Series` (the two-axis
//! stratigraphic group) — and the read-only derived view US1/US4 surface. The
//! `Vein` / `Intrusion` element DTOs, fault-network DTOs, and the surface-DAG land
//! WITH their model code in US2/US3/US8 (no dead DTOs for kinds that don't exist —
//! Principle VI/IV).

use crate::stack::{Relation, Stack};
use crate::SeriesModel;
use serde::{Deserialize, Serialize};

/// Axis-A seam, as it crosses the boundary. Mirrors [`crate::stack::Relation`] plus
/// the floor case (no seam). `lowercase` to match the design-doc §7 TS union.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RelationDto {
    Erode,
    Onlap,
    Conformable,
}

impl From<Relation> for RelationDto {
    fn from(r: Relation) -> Self {
        match r {
            Relation::Erode => RelationDto::Erode,
            Relation::Onlap => RelationDto::Onlap,
            Relation::Conformable => RelationDto::Conformable,
        }
    }
}

impl From<RelationDto> for Relation {
    fn from(r: RelationDto) -> Self {
        match r {
            RelationDto::Erode => Relation::Erode,
            RelationDto::Onlap => Relation::Onlap,
            RelationDto::Conformable => Relation::Conformable,
        }
    }
}

/// One stratigraphic container's **declared state** (editable in the UI, owned by
/// the core). MVP: only the `series` kind. `relation` is `None` for the floor
/// (which carries `unbounded_above = true`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum StackElementDto {
    Series {
        /// Stable element id (the UI's drag/selection key; index-as-string in the MVP).
        id: String,
        name: String,
        /// Interface (surface) ids in ascending-isovalue order.
        surfaces: Vec<u32>,
        /// Axis A — `None` for the floor (no seam below).
        relation: Option<RelationDto>,
        /// Axis B — fills upward until a younger element claims the space.
        unbounded_above: bool,
    },
    /// A cross-cutting vein (tabular HW/FW body). Carries no Axis-A relation; its cut-set
    /// is derived (shown read-only in the UI as a "cross-cuts" chip).
    Vein {
        id: String,
        name: String,
        surfaces: Vec<u32>,
    },
    /// A cross-cutting intrusion (closed body). Same UI treatment as a vein.
    Intrusion {
        id: String,
        name: String,
        surfaces: Vec<u32>,
    },
    /// The singleton topography surface (`docs/topography_lithology_solids_spec.md` §2).
    /// One interface (the ground); above it is Air. Carries no relation / `unbounded_above`.
    Topography {
        id: String,
        name: String,
        surfaces: Vec<u32>,
    },
    /// An IMPORTED mesh surface (feature 004). One interface (the mesh). Carries its role and, for a
    /// contact/erosion role, the same Axis-A relation bit a fitted contact would. The DERIVED
    /// field/patches are never in the DTO (Principle III) — only the declared role + relation.
    ImportedSurface {
        id: String,
        name: String,
        surfaces: Vec<u32>,
        /// The surface role: "contact" | "erosion" | "veinWall" | "topography".
        role: String,
        /// Axis A — present for contact/erosion, `None` otherwise.
        relation: Option<RelationDto>,
    },
}

/// The **effective relationship** the core derives for display (design doc §6.6).
/// `crossCut` / future kinds are not produced by the MVP (series-only) but the
/// variant exists so the TS union is stable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum EffectiveRelation {
    Erode,
    Onlap,
    Conformable,
    CrossCut,
    Floor,
}

/// Per-element validation the UI surfaces read-only (FR-017). MVP: a coarse
/// well-/under-constrained signal derived from the fitted model having interfaces.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ContainerValidation {
    /// `true` when the series has at least one meshable interface.
    pub ok: bool,
    /// Human-readable detail for the hover tooltip (empty when `ok`).
    pub detail: String,
}

/// One derived volume's coordinate within an element — the `(elementId, index)` pair the
/// UI keys lithology assignment on (`docs/topography_lithology_solids_spec.md` §5.1). The
/// core supplies the coordinate only; lithology names/colours live UI-side.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UnitInfo {
    /// The mesher's stable unit coordinate within the element: index 0 is the BOTTOM slab
    /// (below the lowest contact), index n the top. The `units` LIST is emitted top-first
    /// (descending index) for the UI ladder, but this VALUE is the bottom-0 mesher index —
    /// the stable `(elementId, index)` key for lithology assignment.
    pub index: usize,
}

/// The read-only derived view the core returns each round-trip. The UI treats this
/// as a cache; it NEVER recomputes any of it (FR-017, core-api.md invariant 1).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DerivedView {
    /// `effectiveRelation` per element id (parallel to `model`).
    pub effective_relation: Vec<(String, EffectiveRelation)>,
    /// `validation` per element id.
    pub validation: Vec<(String, ContainerValidation)>,
    /// Older elements each cross-cutting body cuts. Empty for every element in the
    /// MVP (series have no cut-set); present so the shape is stable for US2.
    pub cuts_below: Vec<(String, Vec<String>)>,
    /// Per-element ordered unit list (`docs/topography_lithology_solids_spec.md` §5.1). The
    /// volume coordinate the UI renders as the unit/contact ladder and keys lithology
    /// assignment on. Derived (post-clip) count; the Air unit above topography is NOT listed.
    pub units: Vec<(String, Vec<UnitInfo>)>,
}

/// The full core→UI update: canonical declared state + the derived view. (Meshes
/// travel on the existing typed-array mesh ABI, not in this JSON payload.)
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CoreUpdate {
    pub model: Vec<StackElementDto>,
    pub derived: DerivedView,
}

/// Sign of the fitted field's variation with z (its "up" sense), sampled across the domain box
/// `ext = [x0,x1,y0,y1,z0,z1]`, for ordering the unit ladder by physical elevation:
///   * `Some(true)`  — field INCREASES with z (higher isovalue = higher z; unit index n is the top).
///   * `Some(false)` — field DECREASES with z (a deposit-above / erode field; unit index 0 is the top).
///   * `None`        — mixed sign (a FOLDED field, no single up-sense) or ~flat in z → caller keeps
///                     the default mesher order. Sampling a 3×3 (x,y) grid at z-mid ± h detects folds.
pub(crate) fn field_up_sense<M: SeriesModel>(model: &M, ext: [f64; 6]) -> Option<bool> {
    let [x0, x1, y0, y1, z0, z1] = ext;
    let zc = 0.5 * (z0 + z1);
    let h = 1e-3 * (z1 - z0).abs().max(1.0); // small step; field |grad| ~1e-3/unit ≫ f64 noise
    let (mut pos, mut neg) = (0u32, 0u32);
    for &fx in &[0.35, 0.5, 0.65] {
        for &fy in &[0.35, 0.5, 0.65] {
            let x = x0 + fx * (x1 - x0);
            let y = y0 + fy * (y1 - y0);
            let d = model.scalar_field([x, y, zc + h]) - model.scalar_field([x, y, zc - h]);
            if d > 1e-12 {
                pos += 1;
            } else if d < -1e-12 {
                neg += 1;
            }
        }
    }
    match (pos, neg) {
        (p, 0) if p > 0 => Some(true),
        (0, n) if n > 0 => Some(false),
        _ => None, // folded (both signs) or flat (neither) → default order
    }
}

impl<M: SeriesModel> Stack<M> {
    /// Project the live stack to its declared-state DTOs. Element ids are the stack
    /// index as a string in the MVP (stable for one session's edits).
    pub fn to_dtos(&self) -> Vec<StackElementDto> {
        use crate::stack::StackElement;
        self.series
            .iter()
            .enumerate()
            .map(|(i, e)| {
                let id = i.to_string();
                match e {
                    StackElement::Series(s) => StackElementDto::Series {
                        id,
                        name: format!("series {i}"),
                        surfaces: s.model.interface_ids(),
                        relation: s.relation.map(Into::into),
                        unbounded_above: s.unbounded_above,
                    },
                    StackElement::Vein(v) => StackElementDto::Vein {
                        id,
                        name: format!("vein {i}"),
                        surfaces: v.model.interface_ids(),
                    },
                    StackElement::Intrusion(it) => StackElementDto::Intrusion {
                        id,
                        name: format!("intrusion {i}"),
                        surfaces: it.model.interface_ids(),
                    },
                    StackElement::Topography(t) => StackElementDto::Topography {
                        id,
                        name: format!("topography {i}"),
                        surfaces: t.model.interface_ids(),
                    },
                    StackElement::ImportedSurface(im) => StackElementDto::ImportedSurface {
                        id,
                        name: format!("imported {i}"),
                        surfaces: im.model.interface_ids(),
                        role: match im.role {
                            crate::stack::ImportRole::Contact => "contact",
                            crate::stack::ImportRole::Erosion => "erosion",
                            crate::stack::ImportRole::VeinWall => "veinWall",
                            crate::stack::ImportRole::Topography => "topography",
                        }
                        .to_string(),
                        relation: im.relation.map(Into::into),
                    },
                }
            })
            .collect()
    }

    /// Compute the read-only derived view. Pure function of declared state + the
    /// fitted fields — the property core-api.md invariant 5 pins.
    ///
    /// `ext = [x0,x1,y0,y1,z0,z1]` is the domain box, used only to order each series' unit ladder
    /// by physical elevation (top-of-stack first): the mesher's unit index runs with ASCENDING
    /// isovalue, which is bottom→top only when the field increases with z. An erode series deposits
    /// ABOVE its unconformity (its field decreases with z), so its isovalue order is inverted — we
    /// detect that from the fitted field and flip the ladder so the physically-upper unit lists on
    /// top. Index VALUES are unchanged (they stay the stable assignment key); only display order moves.
    pub fn derived_view(&self, ext: [f64; 6]) -> DerivedView {
        use crate::stack::StackElement;
        let mut effective_relation = Vec::with_capacity(self.series.len());
        let mut validation = Vec::with_capacity(self.series.len());
        let mut units = Vec::with_capacity(self.series.len());
        for (i, e) in self.series.iter().enumerate() {
            let id = i.to_string();
            let eff = match e {
                // Floor (unbounded_above, no relation) → Floor; else the Axis-A relation.
                StackElement::Series(s) => {
                    if s.unbounded_above && s.relation.is_none() {
                        EffectiveRelation::Floor
                    } else {
                        match s.relation {
                            Some(Relation::Erode) => EffectiveRelation::Erode,
                            Some(Relation::Onlap) => EffectiveRelation::Onlap,
                            Some(Relation::Conformable) => EffectiveRelation::Conformable,
                            // A bounded series with no relation is degenerate; report
                            // Conformable (the inert no-op) rather than inventing a clip.
                            None => EffectiveRelation::Conformable,
                        }
                    }
                }
                // Cross-cutting bodies are derived cross-cuts (read-only chip in the UI).
                StackElement::Vein(_) | StackElement::Intrusion(_) => EffectiveRelation::CrossCut,
                // Topography reuses the erode clip mechanism and reads as Erode in the UI's
                // effective-relation chip (it is the youngest erosional surface, §2.1).
                StackElement::Topography(_) => EffectiveRelation::Erode,
                // An imported surface reads by its declared relation (contact/erosion) or as a
                // cross-cut for a wall role.
                StackElement::ImportedSurface(im) => match im.relation {
                    Some(Relation::Erode) => EffectiveRelation::Erode,
                    Some(Relation::Onlap) => EffectiveRelation::Onlap,
                    Some(Relation::Conformable) | None => EffectiveRelation::Conformable,
                },
            };
            effective_relation.push((id.clone(), eff));

            let has_iface = !e.model().interface_isovalues().is_empty();
            validation.push((
                id.clone(),
                ContainerValidation {
                    ok: has_iface,
                    detail: if has_iface {
                        String::new()
                    } else {
                        "no meshable interface (element lacks surface points or orientation)".into()
                    },
                },
            ));

            // Per-element ordered unit list (§5.1). A series exposes its present units — now
            // ALL of them, since fill-upward is universal (§3): a whole-model series with K
            // interfaces has K+1 units. A cross-cutting body has one interior rock unit;
            // topography NONE (its only unit is Air, which is never listed/meshed). This is
            // the whole-model representative count (parallel to `interface_isovalues`); the
            // per-block `present_unit_count` agrees by construction (it is now keep-all).
            let n_ifaces = e.model().interface_isovalues().len();
            let unit_count = match e {
                StackElement::Series(_) if n_ifaces > 0 => n_ifaces + 1,
                StackElement::Vein(_) | StackElement::Intrusion(_) if has_iface => 1,
                // An imported contact has ONE interface (the mesh) ⇒ 2 units (below / above).
                StackElement::ImportedSurface(_) if has_iface => 2,
                _ => 0, // topography (Air only), or a field-less element
            };
            // Emit TOP-of-stack first: the mesher's unit index runs with ASCENDING isovalue, so
            // index 0 is the BOTTOM slab and index n the top ONLY when the field increases with z.
            // A deposit-above (erode) field decreases with z, inverting that — so order by the
            // fitted field's actual up-sense at this domain. Field-up (or folded/ambiguous, the
            // safe default): list n..0. Field-down: list 0..n so the physically-upper unit is on
            // top. The `index` VALUE stays the stable mesher coordinate (the assignment key).
            let field_down = matches!(e, StackElement::Series(_))
                && field_up_sense(e.model(), ext) == Some(false);
            let order: Vec<usize> = if field_down {
                (0..unit_count).collect()
            } else {
                (0..unit_count).rev().collect()
            };
            units.push((id.clone(), order.into_iter().map(|index| UnitInfo { index }).collect()));
        }
        DerivedView {
            effective_relation,
            validation,
            cuts_below: Vec::new(), // populated by the ABI layer which has the domain box
            units,
        }
    }

    /// The full `CoreUpdate` (declared state + derived view) for a UI round-trip. `ext` is the
    /// domain box, used to order each series' unit ladder by physical elevation (see `derived_view`).
    pub fn core_update(&self, ext: [f64; 6]) -> CoreUpdate {
        CoreUpdate {
            model: self.to_dtos(),
            derived: self.derived_view(ext),
        }
    }
}

// Native DTO tests — build real kernel `Model`s, so native-only (mirrors stack::tests).
#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;
    use crate::assemble::KernelOptions;
    use crate::data::{Orientation, Series, SurfacePoint};
    use crate::mesh::normalize;
    use crate::stack::StackedSeries;
    use crate::Model;

    fn dipping_plane(z0: f64, sx: f64, sy: f64) -> Model {
        let mut sps = Vec::new();
        for i in 0..4 {
            for j in 0..4 {
                let x = i as f64 * 300.0 + 50.0;
                let y = j as f64 * 300.0 + 50.0;
                let z = z0 + sx * (x - 500.0) + sy * (y - 500.0);
                sps.push(SurfacePoint::new([x, y, z], 0));
            }
        }
        let oris = vec![Orientation::new([500.0, 500.0, z0], normalize([-sx, -sy, 1.0]))];
        Model::fit(&Series::new(sps, oris), KernelOptions::default())
    }

    /// A 2-element stack: a younger Erode series over an unbounded-above floor.
    fn erode_over_floor() -> Stack<Model> {
        Stack::new(vec![
            StackedSeries::new(dipping_plane(600.0, 0.4, 0.0), Relation::Erode).into(),
            StackedSeries::floor(dipping_plane(500.0, 0.0, 0.0)).into(),
        ])
    }

    /// The unit ladder is ordered by PHYSICAL elevation, not raw mesher index. A deposit-above
    /// (field DECREASES with z) series — what the UI's `erode ⇒ deposit-above` produces — has its
    /// mesher unit index running OPPOSITE to elevation, so index 0 is the physically-TOP slab and
    /// must lead the ladder (not the default `.rev()`). Pins the fix for the "Unknown over basalt"
    /// inversion. Index VALUES stay the stable mesher coordinate; only display order changes.
    #[test]
    fn deposit_above_series_lists_units_top_first() {
        let down = {
            let mut sps = Vec::new();
            for i in 0..4 {
                for j in 0..4 {
                    let (x, y) = (i as f64 * 300.0 + 50.0, j as f64 * 300.0 + 50.0);
                    sps.push(SurfacePoint::new([x, y, 600.0], 0));
                }
            }
            let oris = vec![Orientation::new([500.0, 500.0, 600.0], [0.0, 0.0, -1.0])]; // pole DOWN
            Model::fit(&Series::new(sps, oris), KernelOptions::default())
        };
        let stack = Stack::new(vec![
            StackedSeries::new(down, Relation::Erode).into(),
            StackedSeries::floor(dipping_plane(400.0, 0.0, 0.0)).into(),
        ]);
        let dv = stack.derived_view([0.0, 1000.0, 0.0, 1000.0, 0.0, 1000.0]);
        let erode: Vec<usize> = dv.units[0].1.iter().map(|u| u.index).collect();
        assert_eq!(erode, vec![0, 1], "deposit-above (field-down) lists index 0 (the top slab) first");
        let floor: Vec<usize> = dv.units[1].1.iter().map(|u| u.index).collect();
        assert_eq!(floor, vec![1, 0], "field-up floor keeps the default top-first-by-rev order");
    }

    /// T016 [US4] — the DTO JSON contract round-trips losslessly. The hand-mirrored TS
    /// (`web3/src/HierarchyPanel/dto.ts`) depends on these exact serde shapes; this test
    /// fails if the Rust shape changes incompatibly, bounding drift until ts-rs lands.
    #[test]
    fn core_update_json_round_trips() {
        let stack = erode_over_floor();
        let update = stack.core_update([0.0, 1000.0, 0.0, 1000.0, 0.0, 1000.0]);
        let json = serde_json::to_string(&update).expect("serialize CoreUpdate");
        let back: CoreUpdate = serde_json::from_str(&json).expect("deserialize CoreUpdate");
        assert_eq!(update, back, "CoreUpdate must round-trip through JSON unchanged");
    }

    /// T016 [US4] — `derived_view` is a PURE function of declared state (core-api.md
    /// invariant 5): deriving twice from the same stack yields identical output, and the
    /// floor reports `EffectiveRelation::Floor` while the younger Erode reports `Erode`.
    #[test]
    fn derived_view_is_pure_and_correct() {
        let stack = erode_over_floor();
        let a = stack.derived_view([0.0, 1000.0, 0.0, 1000.0, 0.0, 1000.0]);
        let b = stack.derived_view([0.0, 1000.0, 0.0, 1000.0, 0.0, 1000.0]);
        assert_eq!(a, b, "derived view must be a pure function of declared state");

        // Element "0" is the younger Erode; "1" is the floor.
        let eff: std::collections::HashMap<_, _> = a.effective_relation.into_iter().collect();
        assert_eq!(eff["0"], EffectiveRelation::Erode, "younger series is Erode");
        assert_eq!(eff["1"], EffectiveRelation::Floor, "the unbounded-above floor reports Floor");

        // Both series fit real planes ⇒ both validate ok.
        let val: std::collections::HashMap<_, _> = a.validation.into_iter().collect();
        assert!(val["0"].ok && val["1"].ok, "both series have meshable interfaces");
    }

    /// T016 [US4] — declared-state DTOs reflect the two axes: the floor carries
    /// `relation: None` + `unbounded_above: true`; a bounded series carries `Some(rel)` +
    /// `false`. This is the shape the UI edits.
    #[test]
    fn dtos_carry_both_axes() {
        let stack = erode_over_floor();
        let dtos = stack.to_dtos();
        let StackElementDto::Series { relation, unbounded_above, .. } = &dtos[0] else {
            panic!("element 0 should be a series");
        };
        assert_eq!(*relation, Some(RelationDto::Erode));
        assert!(!*unbounded_above, "younger series is bounded");

        let StackElementDto::Series { relation, unbounded_above, .. } = &dtos[1] else {
            panic!("element 1 should be a series");
        };
        assert_eq!(*relation, None, "floor has no Axis-A relation");
        assert!(*unbounded_above, "floor is unbounded above");
    }

    /// T025 [US5] — a scenario-shaped DTO set deserializes and the core accepts it. This
    /// is the native half of the testbed's scenario-validity guarantee (FR-022); the TS
    /// fixtures (`web3/src/scenarios.ts`) emit this exact JSON shape.
    #[test]
    fn scenario_shaped_dtos_deserialize() {
        // The conformal-column + erosional-unconformity scenarios reduce to this DTO JSON.
        let json = r#"[
            {"kind":"series","id":"0","name":"basalt","surfaces":[0],"relation":"erode","unbounded_above":false},
            {"kind":"series","id":"1","name":"sediments","surfaces":[0,1,2],"relation":null,"unbounded_above":true}
        ]"#;
        let dtos: Vec<StackElementDto> = serde_json::from_str(json).expect("scenario DTOs parse");
        assert_eq!(dtos.len(), 2);
        let StackElementDto::Series { unbounded_above, relation, surfaces, .. } = &dtos[1] else {
            panic!("the floor scenario element should be a series");
        };
        assert!(*unbounded_above && relation.is_none(), "the floor scenario element");
        assert_eq!(surfaces.len(), 3, "3-contact floor");
    }

    // ===== Feature 003: topography DTO + DerivedView.units =====

    /// A serialized scenario carrying the `unbounded_above` slot parses with either value
    /// without error. The flag is preserved (parsed into the DTO and round-tripped, see the
    /// derived-view / round-trip tests above) — this pins that neither value is ever a parse
    /// failure for an existing serialized project.
    #[test]
    fn legacy_unbounded_above_slot_still_parses() {
        for v in ["true", "false"] {
            let json = format!(
                r#"[{{"kind":"series","id":"0","name":"s","surfaces":[0],"relation":null,"unbounded_above":{v}}}]"#
            );
            let dtos: Vec<StackElementDto> =
                serde_json::from_str(&json).expect("legacy unbounded_above must still parse");
            assert!(matches!(dtos[0], StackElementDto::Series { .. }));
        }
    }

    /// The topography DTO serializes/deserializes by its `kind` tag and the derived view
    /// lists its (empty) unit set (§5.1, T009/T010). Topography carries no relation /
    /// unbounded slot and contributes NO rock unit (its only unit is Air).
    #[test]
    fn topography_dto_and_units_shape() {
        use crate::stack::{Stack, StackElement, StackedSeries, Topography};
        let topo = StackElement::Topography(Topography { model: dipping_plane(900.0, 0.0, 0.0) });
        let floor = StackedSeries::floor(dipping_plane(500.0, 0.0, 0.0));
        let stack = Stack::new(vec![topo, floor.into()]);

        let dtos = stack.to_dtos();
        assert!(matches!(dtos[0], StackElementDto::Topography { .. }), "element 0 is topography");
        // Round-trips by its kind tag.
        let s = serde_json::to_string(&dtos[0]).unwrap();
        assert!(s.contains("\"kind\":\"topography\""), "tagged 'topography': {s}");
        let back: StackElementDto = serde_json::from_str(&s).unwrap();
        assert!(matches!(back, StackElementDto::Topography { .. }));

        // DerivedView.units: topography lists none; the floor lists its units.
        let dv = stack.derived_view([0.0, 1000.0, 0.0, 1000.0, 0.0, 1000.0]);
        assert_eq!(dv.units[0].1.len(), 0, "topography contributes no rock unit (Air only)");
        assert_eq!(dv.units[1].1.len(), 2, "1-contact floor exposes 2 units (universal fill-up)");
    }
}
