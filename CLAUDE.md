<!-- SPECKIT START -->
For technologies, project structure, shell commands, and other context, read the
current plan: `specs/005-cross-section-digitizing/plan.md` (feature: cross-sections &
digitized surfaces — vertical section planes from a two-click plan-view baseline,
opened face-on (orthographic, u right / Z up) showing traces of the applied model, in
serial parallel sets with stepping; and polyline digitizing on those planes where each
vertex becomes a contact point and the DRAWN DIRECTION yields orientation poles
`m = ŝ × t̂` ("youngs to the LEFT" — the source fragment's `t × s` is a sign slip;
see the feature's contracts/younging-derivation.md). Resolved approach: 100% UI-side
TypeScript in `web3/` — traces & solid-interval fills sliced on the main thread from
already-returned mesh payloads (no worker round-trip; Manifold NOT used for fills);
digitized lines stored as `InterfaceData.digitized` (world vertices + plane snapshot,
poles derived at the single `flattenInterfaces` flip-XOR site); sections are
non-dirty view-state (`Project.sections` + `view.section`, setRelation-style updater);
loft = the existing co-kriging fit — no new engine/ABI/wasm code. Source fragment
`web3/SPEC_cross_sections.md` merges into `web3/SPEC.md` §10/§11 as part of the work;
the gate is five new Playwright specs. Prior features:
`specs/004-imported-mesh-surfaces/plan.md` (imported `.obj` meshes as any surface
role, exact inside their bounds, smoothly extended to the domain box via the
`mesh_clip::clip` corefiner patch-stitch);
`specs/003-topography-lithology-solids/plan.md` (source of record
`docs/topography_lithology_solids_spec.md`); before them,
`specs/002-enhanced-authoring/plan.md` and `specs/001-stratigraphic-hierarchy/plan.md`.
The project constitution is `.specify/memory/constitution.md`.
<!-- SPECKIT END -->
