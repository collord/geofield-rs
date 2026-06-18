// T021 [US4] — the SINGLE explicit drop-semantics resolver (design doc §6.3; research R4).
//
// One dnd-kit context, one resolver that branches on (dragNodeType, dropZoneType). The two
// drag behaviours are deliberately distinct:
//   - container reorder  → CHRONOLOGY change (recompute + remesh) — the consequential drag;
//   - within-group surface reorder → isovalue-sequence change of ONE shared field (local);
//   - cross-container surface move → allowed; re-validates both fields.
//
// Within-group reorder is DISABLED for vein/intrusion (fixed payloads) — in the MVP only
// `series` exists, so that guard is a no-op now but the resolver encodes it for US2.
//
// Build status: type-checked design; not yet `tsc`-verified here (web3 deps uninstalled).

export type NodeType = "container" | "surface";

export interface DragNode {
  type: NodeType;
  /** element id (container) or owning element id (surface). */
  elementId: string;
  /** surface index within its container, when type === "surface". */
  surfaceIndex?: number;
}

export interface DropZone {
  type: NodeType;
  elementId: string;
  surfaceIndex?: number;
}

/** The resolved mutation a drop should trigger. Maps 1:1 to a core mutation (FR-018). */
export type DropAction =
  | { kind: "reorderContainers"; from: string; to: string }
  | { kind: "reorderWithin"; elementId: string; from: number; to: number }
  | { kind: "moveSurface"; surfaceElementId: string; surfaceIndex: number; toElementId: string }
  | { kind: "none"; reason: string };

/** Whether this element kind permits within-group surface reordering. Series: yes;
 *  vein/intrusion (US2): no (fixed HW/FW pair / envelope). */
export function allowsWithinReorder(elementKind: string): boolean {
  return elementKind === "series";
}

/** The one resolver. Pure function of the drag/drop descriptors → exactly one action. */
export function resolveDrop(
  drag: DragNode,
  drop: DropZone,
  elementKindById: (id: string) => string,
): DropAction {
  // Dragging a CONTAINER onto a container zone → chronology reorder.
  if (drag.type === "container" && drop.type === "container") {
    if (drag.elementId === drop.elementId) return { kind: "none", reason: "same container" };
    return { kind: "reorderContainers", from: drag.elementId, to: drop.elementId };
  }

  // Dragging a SURFACE.
  if (drag.type === "surface" && drag.surfaceIndex !== undefined) {
    // …within the SAME container → isovalue reorder (only if the kind allows it).
    if (drop.type === "surface" && drop.elementId === drag.elementId && drop.surfaceIndex !== undefined) {
      if (!allowsWithinReorder(elementKindById(drag.elementId))) {
        return { kind: "none", reason: "within-group reorder disabled for this element kind" };
      }
      return {
        kind: "reorderWithin",
        elementId: drag.elementId,
        from: drag.surfaceIndex,
        to: drop.surfaceIndex,
      };
    }
    // …onto a DIFFERENT container → cross-container move.
    if (drop.elementId !== drag.elementId) {
      return {
        kind: "moveSurface",
        surfaceElementId: drag.elementId,
        surfaceIndex: drag.surfaceIndex,
        toElementId: drop.elementId,
      };
    }
  }

  return { kind: "none", reason: "unhandled drag/drop combination" };
}
