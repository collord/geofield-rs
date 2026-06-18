// T020/T022/T023 [US4] — the reusable hierarchy panel.
//
// Presents the model as a draggable tree (youngest at top): container nodes (series/…)
// with surface children, the three-state seam control on the boundary BELOW each
// container, a floor marker, validation badges, and the read-only derived-relationship
// summary. The panel EDITS DECLARED STATE ONLY (via the `on*` mutation callbacks) and
// renders relationships/validation SOLELY from `CoreUpdate.derived` — it never recomputes
// geology (FR-017, core-api.md invariant 1).
//
// Reusable: this component takes its data + mutation callbacks as props, so any host (not
// just the web3 testbed) can embed it (FR-019).
//
// Build status: type-checked design; not yet `tsc`-verified here (web3 deps uninstalled).

import { DndContext, type DragEndEvent } from "@dnd-kit/core";
import {
  CoreUpdate,
  RelationDto,
  StackElementDto,
  effectiveRelationOf,
  validationOf,
} from "./dto";
import { resolveDrop, type DropAction, type NodeType } from "./dragResolver";

export interface HierarchyPanelProps {
  update: CoreUpdate;
  /** One callback per editable gesture — each maps to exactly one core mutation (FR-018). */
  onReorderContainers: (from: string, to: string) => void;
  onReorderWithin: (elementId: string, from: number, to: number) => void;
  onMoveSurface: (surfaceElementId: string, surfaceIndex: number, toElementId: string) => void;
  onSetRelation: (elementId: string, rel: RelationDto) => void;
  onSetUnboundedAbove: (elementId: string, v: boolean) => void;
}

const RELATION_CHOICES: RelationDto[] = ["erode", "onlap", "conformable"];

export function HierarchyPanel(props: HierarchyPanelProps) {
  const { update } = props;
  const kindById = (id: string) =>
    update.model.find((e) => e.id === id)?.kind ?? "series";

  function dispatch(action: DropAction) {
    switch (action.kind) {
      case "reorderContainers":
        return props.onReorderContainers(action.from, action.to);
      case "reorderWithin":
        return props.onReorderWithin(action.elementId, action.from, action.to);
      case "moveSurface":
        return props.onMoveSurface(action.surfaceElementId, action.surfaceIndex, action.toElementId);
      case "none":
        return; // explicit no-op (e.g. within-reorder disabled for a vein)
    }
  }

  function onDragEnd(ev: DragEndEvent) {
    if (!ev.over) return;
    const drag = ev.active.data.current as
      | { type: NodeType; elementId: string; surfaceIndex?: number }
      | undefined;
    const drop = ev.over.data.current as
      | { type: NodeType; elementId: string; surfaceIndex?: number }
      | undefined;
    if (!drag || !drop) return;
    dispatch(resolveDrop(drag, drop, kindById));
  }

  return (
    <DndContext onDragEnd={onDragEnd}>
      <div className="hierarchy" role="tree" aria-label="stratigraphic hierarchy">
        {/* youngest at top — model[0] is youngest */}
        {update.model.map((el, i) => (
          <div key={el.id}>
            <ContainerNode el={el} update={update} onSetUnboundedAbove={props.onSetUnboundedAbove} />
            {/* seam control sits on the boundary BELOW this container (between it and the
                next-older one). The floor (last, unbounded_above) shows no seam. */}
            {i < update.model.length - 1 && (
              <SeamControl el={el} update={update} onSetRelation={props.onSetRelation} />
            )}
          </div>
        ))}
      </div>
    </DndContext>
  );
}

function ContainerNode(props: {
  el: StackElementDto;
  update: CoreUpdate;
  onSetUnboundedAbove: (id: string, v: boolean) => void;
}) {
  const { el, update } = props;
  const validation = validationOf(update.derived, el.id);
  const eff = effectiveRelationOf(update.derived, el.id);
  const isFloor = el.kind === "series" && el.unbounded_above && el.relation === null;

  return (
    <div className="container-node" role="treeitem" data-element-id={el.id}>
      <div className="container-header">
        <span className="kind-icon" data-kind={el.kind} />
        <span className="container-name">{el.name}</span>
        {/* validation badge — green when ok, amber otherwise. SOURCED FROM derived. */}
        <span
          className={"validation-badge " + (validation?.ok ? "ok" : "warn")}
          title={validation?.ok ? "well constrained" : validation?.detail ?? ""}
        >
          {validation?.ok ? "✓" : "!"}
        </span>
        {/* floor marker (no seam below it) */}
        {isFloor && <span className="floor-chip" title="fills upward / floor">floor ⤒</span>}
      </div>

      {/* surface children, in ascending-isovalue order */}
      <ul className="surface-list">
        {el.kind === "series" &&
          el.surfaces.map((sid, si) => (
            <li key={sid} className="surface-node" data-element-id={el.id} data-surface-index={si}>
              surface {sid}
            </li>
          ))}
      </ul>

      {/* derived-relationship summary — read-only (design doc §6.6) */}
      <div className="derived-summary">effective: {eff ?? "—"}</div>

      {/* Axis-B toggle: a series may be marked the floor (fills upward). */}
      {el.kind === "series" && (
        <label className="unbounded-toggle">
          <input
            type="checkbox"
            checked={el.unbounded_above}
            onChange={(e) => props.onSetUnboundedAbove(el.id, e.target.checked)}
          />
          fills upward (floor)
        </label>
      )}
    </div>
  );
}

function SeamControl(props: {
  el: StackElementDto;
  update: CoreUpdate;
  onSetRelation: (id: string, rel: RelationDto) => void;
}) {
  const { el, update } = props;
  const eff = effectiveRelationOf(update.derived, el.id);

  // A cross-cutting body (US2) shows a read-only "cross-cuts" chip instead of the control.
  // The tooltip records the syn-depositional limitation (design doc §3.5): a body younger
  // than some local units and older than others is out of scope — model it as multiple
  // bodies, one per chronological position.
  if (eff === "crossCut") {
    return (
      <div
        className="seam cross-cut-chip"
        title="Cross-cuts older elements (derived from chronology + overlap, not declared). Syn-depositional bodies (younger than some units, older than others) are out of scope — model them as multiple bodies."
      >
        ⨯ cross-cuts
      </div>
    );
  }
  // The floor shows no seam control (handled by the caller not rendering it for the last
  // element); a defensively-guarded series with no relation also shows nothing editable.
  if (el.kind !== "series" || el.relation === null) {
    return <div className="seam floor-seam">— fills upward —</div>;
  }

  return (
    <div className="seam seam-control" role="radiogroup" aria-label="relationship to series below">
      {RELATION_CHOICES.map((rel) => (
        <button
          key={rel}
          className={"seam-state " + (el.relation === rel ? "active" : "")}
          aria-pressed={el.relation === rel}
          onClick={() => props.onSetRelation(el.id, rel)}
        >
          {rel}
        </button>
      ))}
    </div>
  );
}
