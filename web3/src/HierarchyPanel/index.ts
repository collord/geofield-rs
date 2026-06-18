// Barrel for the reusable hierarchy panel (FR-019). A host embeds it via:
//   import { HierarchyPanel, applyCoreUpdate } from "./HierarchyPanel";
// and "./HierarchyPanel/hierarchy.css" for styling. The package depends only on React +
// @dnd-kit and the DTO contract (dto.ts, mirrored from crates/geofield/src/schema.rs).

export { HierarchyPanel } from "./HierarchyPanel";
export type { HierarchyPanelProps } from "./HierarchyPanel";
export { applyCoreUpdate, emptyState } from "./store";
export type { HierarchyState } from "./store";
export { resolveDrop, allowsWithinReorder } from "./dragResolver";
export type { DropAction, DragNode, DropZone, NodeType } from "./dragResolver";
export * from "./dto";
