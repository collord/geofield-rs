// Barrel for the reusable hierarchy panel (FR-023). A host embeds the authoring UI via:
//   import { HierarchyPanel } from "./HierarchyPanel";
//   import { toSeriesJson, validate, type Project } from "./HierarchyPanel/project";
// and "./HierarchyPanel/hierarchy.css" for styling. The package depends only on React +
// @dnd-kit, the DTO contract (dto.ts, mirrored from crates/geofield/src/schema.rs), and the
// SeriesJson protocol shape it compiles to (protocol.ts). The authoring model is UI-only and
// never crosses the wasm boundary.

export { HierarchyPanel } from "./HierarchyPanel";
export type { HierarchyPanelProps } from "./HierarchyPanel";
export { applyCoreUpdate, emptyState } from "./store";
export type { HierarchyState } from "./store";
export { resolveDrop, allowsWithinReorder } from "./dragResolver";
export type { DropAction, DragNode, DropZone, NodeType } from "./dragResolver";
export * from "./dto";

// authoring model + operations
export {
  toSeriesJson,
  validate,
  dipDirToPole,
  saveProject,
  loadProject,
  exportProjectJson,
  importProjectJson,
  newId,
} from "./project";
export type {
  Project,
  ProjectElement,
  InterfaceData,
  PointRow,
  OrientationRow,
  SurfaceType,
  ProjectValidation,
} from "./project";
export * from "./projectOps";

// authoring components
export { AddSurfaceMenu } from "./AddSurfaceMenu";
export { LithologyCatalog } from "./LithologyCatalog";
export { VolumeVisibility } from "./VolumeVisibility";
export { SurfaceDataModal } from "./SurfaceDataModal";
export { SurfaceMenu, SeriesMenu } from "./SurfaceMenu";
export { PointsTable, OrientationsTable } from "./tables";
