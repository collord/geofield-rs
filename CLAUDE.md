<!-- SPECKIT START -->
For technologies, project structure, shell commands, and other context, read the
current plan: `specs/003-topography-lithology-solids/plan.md` (feature: topography,
lithology catalog & solid volumes — promote the surface model to a named-volume model.
A cross-cutting Rust + ABI + worker + UI change: a dedicated `StackElement::Topography`
variant (singleton, youngest, erode-clip, Air above); make `unbounded_above` inert by
deriving a series' top-unit keep/drop from whether a younger element caps it (net-new
core logic, not a flag read); expose per-element unit coordinates on `DerivedView.units`;
a UI-side lithology catalog (Air + Unknown fixed, inferred from bodies, user-added) with
a many-to-one `(element, unit) → lithology` assignment serialized into the project JSON;
reuse the capped-Scene mesher with topography as a universal top masker, skip Air, and
wire the existing `RenderMode` to a Surfaces⇄Solids toggle that re-meshes without
re-fitting, colouring solids by lithology and grouping OBJ by lithology). Its design
source of record is `docs/topography_lithology_solids_spec.md`. The prior feature is
`specs/002-enhanced-authoring/plan.md` (source of record
`docs/enhanced_authoring_spec.md`); before it, `specs/001-stratigraphic-hierarchy/plan.md`
(source of record `docs/stack_hierarchy_spec.md`). The project constitution is
`.specify/memory/constitution.md`.
<!-- SPECKIT END -->
