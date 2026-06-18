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
}

/// The full core→UI update: canonical declared state + the derived view. (Meshes
/// travel on the existing typed-array mesh ABI, not in this JSON payload.)
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CoreUpdate {
    pub model: Vec<StackElementDto>,
    pub derived: DerivedView,
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
                }
            })
            .collect()
    }

    /// Compute the read-only derived view. Pure function of declared state + the
    /// fitted fields — the property core-api.md invariant 5 pins.
    pub fn derived_view(&self) -> DerivedView {
        use crate::stack::StackElement;
        let mut effective_relation = Vec::with_capacity(self.series.len());
        let mut validation = Vec::with_capacity(self.series.len());
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
        }
        DerivedView {
            effective_relation,
            validation,
            cuts_below: Vec::new(), // populated by the ABI layer which has the domain box
        }
    }

    /// The full `CoreUpdate` (declared state + derived view) for a UI round-trip.
    pub fn core_update(&self) -> CoreUpdate {
        CoreUpdate {
            model: self.to_dtos(),
            derived: self.derived_view(),
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

    /// T016 [US4] — the DTO JSON contract round-trips losslessly. The hand-mirrored TS
    /// (`web3/src/HierarchyPanel/dto.ts`) depends on these exact serde shapes; this test
    /// fails if the Rust shape changes incompatibly, bounding drift until ts-rs lands.
    #[test]
    fn core_update_json_round_trips() {
        let stack = erode_over_floor();
        let update = stack.core_update();
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
        let a = stack.derived_view();
        let b = stack.derived_view();
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
}
