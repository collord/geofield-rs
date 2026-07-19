# Contract: UI ↔ Worker DTOs + Project Element

The main-thread ↔ worker protocol and the project model, extended for imported surfaces. Mirrors how `crossCut?` / `topography?` discriminate `SeriesJson` today (`web3/src/protocol.ts`).

## `SeriesJson.importedMesh?` (worker input)

A new optional discriminator on `SeriesJson` (sibling to `crossCut?`, `topography?`). When present, the worker routes the element to `stack.add_imported_surface(...)` and ignores the point/orientation fields.

```
importedMesh?: {
  role: "contact" | "erosion" | "veinWall" | "topography";
  // geometry: raw bytes (preferred — parsed in the engine) OR pre-flattened arrays
  bytes?: Uint8Array;            // .obj file bytes
  vertices?: number[];          // 3*N  (if pre-parsed)
  faces?: number[];             // 3*T  (triangle vertex indices)
  normals?: number[];           // 3*N  (optional)
}
```

- `relation` (existing field) carries the erode/onlap/conformable bit when `role ∈ {contact, erosion}`.
- Exactly one of `bytes` / (`vertices`+`faces`) is set. Bytes preferred so the parser lives once in the engine (D4).

## `ProjectElement { kind: "importedSurface" }` (project model)

A new variant of the `ProjectElement` union (`web3/src/HierarchyPanel/project.ts`):

```
{ kind: "importedSurface"; id: string; name: string;
  role: "contact" | "erosion" | "veinWall" | "topography";
  relation?: "erode" | "onlap" | "conformable";
  mesh: { vertices: number[]; faces: number[]; normals?: number[] } }
```

- `toSeriesJson` compiles it to a `SeriesJson` with `importedMesh` set (and `relation` where applicable).
- Round-trips through `.geoproj` save/load intact (FR-012); the derived field/representation is NOT stored.
- The mesh is embedded in the project (or referenced) so a reopened project reconstructs the surface.

## UI affordance contract (FR-013)

- A **distinct** "Import mesh (.obj)" control, separate from the existing project-JSON "Import" button (which loads `.geoproj`). Accepts `.obj`. On selection: read bytes → add an `importedSurface` element → re-fit/re-mesh → the element appears in the hierarchy and renders.
- The element exposes the same per-element affordances as a fitted surface where they apply: role/relation selection, lithology assignment (via its derived units), rename, delete.

## Behavioral (Playwright) contract — SC-009

- Import an `.obj` → an imported-surface element appears in the hierarchy and its geometry renders in the viewer.
- Assign it a role (e.g. erosion) → the model re-meshes and the role's effect is visible/asserted (older units truncated).
- Save then reload the project → the imported surface and its role persist (FR-012).
