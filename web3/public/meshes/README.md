# Imported meshes

Static `.obj` meshes served at `/meshes/<file>`. An imported-surface element in a project
references one via `objPath: "meshes/<file>"`; on **export** the project JSON drops the inline
`objText` and keeps only the path, and the **loader re-fetches** the mesh from here (see
`hydrateMeshPaths` in `src/HierarchyPanel/project.ts`).

To share a project that imports a big mesh: export the `.geoproj.json` (slim) **and** drop the
`.obj` into this folder. A project whose mesh is not here will fail to load with a clear error
naming the missing path.

## "Erosion over stack (imported meshes)" template fixtures

The `erosion-stack-*.obj` files (five ~100k-vertex surfaces: `bed1`, `bed2`, `bed3`,
`erosion`, `cover`) back the **"Erosion over stack (imported meshes)"** seed template — the
same geology as the parametric "Erosion over conformal stack" repro, but every surface is an
imported, value-noise-perturbed mesh. They are gitignored like any other mesh; regenerate them
deterministically with:

```sh
cargo test -p geofield --test gen_erosion_meshes -- --ignored --nocapture
```

(the generator lives at `crates/geofield/tests/gen_erosion_meshes.rs`). Without them, seeding
that template reports a "mesh not found" error naming the missing `meshes/erosion-stack-*.obj`.
