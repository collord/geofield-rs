# Imported meshes

Static `.obj` meshes served at `/meshes/<file>`. An imported-surface element in a project
references one via `objPath: "meshes/<file>"`; on **export** the project JSON drops the inline
`objText` and keeps only the path, and the **loader re-fetches** the mesh from here (see
`hydrateMeshPaths` in `src/HierarchyPanel/project.ts`).

To share a project that imports a big mesh: export the `.geoproj.json` (slim) **and** drop the
`.obj` into this folder. A project whose mesh is not here will fail to load with a clear error
naming the missing path.
