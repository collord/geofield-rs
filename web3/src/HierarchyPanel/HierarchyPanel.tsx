// Feature 002 (enhanced-authoring) — the reusable hierarchy panel, now PROJECT-DRIVEN.
//
// Renders the editable Project tree (youngest at top) with Add controls and per-surface /
// per-series "…" menus. A series with exactly ONE interface renders FLAT (as a plain
// surface row, no header) — the "container is internal" simplification; a multi-interface
// series nests under a header; vein/intrusion always show their named sub-surfaces.
//
// The panel still renders RELATIONSHIPS and the validation badge SOLELY from
// `CoreUpdate.derived` (it never recomputes geology — FR-018, Constitution III). The core
// assigns element ids as the stack index ("0","1",…) youngest-first (schema.rs::core_update),
// parallel to `project.elements`, so derived state is looked up BY POSITION. Author-time
// (structural) validation comes from the `validation` prop (project.ts::validate) and is
// distinct from the core's fitted-result validation.

import { DndContext, type DragEndEvent } from "@dnd-kit/core";
import { CoreUpdate, RelationDto, effectiveRelationOf, validationOf } from "./dto";
import { resolveDrop, type DropAction, type NodeType } from "./dragResolver";
import type { InterfaceData, Project, ProjectElement, ProjectValidation } from "./project";
import { SurfaceMenu, SeriesMenu } from "./SurfaceMenu";

export interface HierarchyPanelProps {
  project: Project;
  update: CoreUpdate;
  validation: ProjectValidation;
  onReorderContainers: (from: string, to: string) => void;
  onReorderWithin: (elementId: string, from: number, to: number) => void;
  /** Move a top-level element one slot earlier (-1, toward youngest/top) or later (+1)
   *  in the chronology — the click/keyboard reorder affordance (drag is unbuilt). */
  onMoveElement: (elementId: string, dir: -1 | 1) => void;
  onMoveSurface: (surfaceElementId: string, surfaceIndex: number, toElementId: string) => void;
  onSetRelation: (elementId: string, rel: RelationDto) => void;
  onSetUnboundedAbove: (elementId: string, v: boolean) => void;
  // authoring entry points (US1/US2)
  onAddSurfaceClick: () => void;
  onEditSurface: (elementId: string, interfaceId: string) => void;
  onRenameSurface: (elementId: string, interfaceId: string, name: string) => void;
  onFlipSurface: (elementId: string, interfaceId: string) => void;
  onDuplicateSurface: (elementId: string, interfaceId: string) => void;
  onDeleteSurface: (elementId: string, interfaceId: string) => void;
  onAddInterface: (elementId: string) => void;
  onRenameElement: (elementId: string, name: string) => void;
  onDeleteElement: (elementId: string) => void;
}

const RELATION_CHOICES: RelationDto[] = ["erode", "onlap", "conformable"];

export function HierarchyPanel(props: HierarchyPanelProps) {
  const { project } = props;
  // Drag node ids are project element ids; resolve their kind for the drop resolver.
  const kindById = (id: string) => project.elements.find((e) => e.id === id)?.kind ?? "series";

  function dispatch(action: DropAction) {
    switch (action.kind) {
      case "reorderContainers":
        return props.onReorderContainers(action.from, action.to);
      case "reorderWithin":
        return props.onReorderWithin(action.elementId, action.from, action.to);
      case "moveSurface":
        return props.onMoveSurface(action.surfaceElementId, action.surfaceIndex, action.toElementId);
      case "none":
        return;
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
        {project.elements.map((el, i) => (
          <div key={el.id}>
            <ElementNode
              el={el}
              index={i}
              count={project.elements.length}
              isFloorBoundary={i < project.elements.length - 1}
              props={props}
            />
          </div>
        ))}

        {project.elements.length === 0 && (
          <div className="empty-hint">No surfaces yet — add one to start.</div>
        )}

        <button type="button" className="add-surface-btn" data-testid="add-surface" onClick={props.onAddSurfaceClick}>
          + Add Surface
        </button>
      </div>
    </DndContext>
  );
}

/** One container. Singleton series render flat; multi-interface series + cross-cutting
 *  bodies render with named sub-surfaces. Derived state is looked up by stack index. */
function ElementNode(props: {
  el: ProjectElement;
  index: number;
  count: number;
  isFloorBoundary: boolean;
  props: HierarchyPanelProps;
}) {
  const { el, index, count, isFloorBoundary } = props;
  const p = props.props;
  const coreId = String(index); // core ids are stack-index strings, youngest-first
  const validation = validationOf(p.update.derived, coreId);
  const eff = effectiveRelationOf(p.update.derived, coreId);

  const singletonSeries = el.kind === "series" && el.interfaces.length === 1;

  const subSurfaces: { iface: InterfaceData; surfaceIndex: number }[] =
    el.kind === "series"
      ? el.interfaces.map((iface, si) => ({ iface, surfaceIndex: si }))
      : el.kind === "vein"
        ? [
            { iface: el.footWall, surfaceIndex: 0 },
            { iface: el.hangingWall, surfaceIndex: 1 },
          ]
        : [
            { iface: el.envelopeInner, surfaceIndex: 0 },
            { iface: el.envelopeOuter, surfaceIndex: 1 },
          ];

  const reorder = (
    <ReorderControls
      name={el.name}
      isFirst={index === 0}
      isLast={index >= count - 1}
      onUp={() => p.onMoveElement(el.id, -1)}
      onDown={() => p.onMoveElement(el.id, 1)}
    />
  );

  return (
    <div className="container-node" role="treeitem" data-element-id={el.id} data-kind={el.kind}>
      {singletonSeries ? (
        // FLAT: a one-interface series is just "a surface". The reorder control leads the row.
        <div className="surface-row-flat">
          {reorder}
          <SurfaceRow
            el={el}
            iface={subSurfaces[0].iface}
            surfaceIndex={0}
            validation={validation?.ok}
            validationDetail={validation?.detail}
            p={p}
          />
        </div>
      ) : (
        <>
          <div className="container-header">
            {reorder}
            <span className="kind-icon" data-kind={el.kind} />
            <span className="container-name">{el.name}</span>
            <span
              className={"validation-badge " + (validation?.ok ? "ok" : "warn")}
              title={validation?.ok ? "well constrained" : validation?.detail ?? ""}
            >
              {validation?.ok ? "✓" : "!"}
            </span>
            <SeriesMenu
              el={el}
              onRename={(name) => p.onRenameElement(el.id, name)}
              onDelete={() => p.onDeleteElement(el.id)}
              onAddInterface={() => p.onAddInterface(el.id)}
            />
          </div>
          <ul className="surface-list">
            {subSurfaces.map(({ iface, surfaceIndex }) => (
              <li
                key={iface.id}
                className="surface-node"
                data-element-id={el.id}
                data-surface-index={surfaceIndex}
              >
                <SurfaceRow el={el} iface={iface} surfaceIndex={surfaceIndex} p={p} inline />
              </li>
            ))}
          </ul>
        </>
      )}

      <div className="derived-summary">effective: {eff ?? "—"}</div>

      {el.kind === "series" && (
        <label className="unbounded-toggle">
          <input
            type="checkbox"
            checked={el.unboundedAbove}
            onChange={(e) => p.onSetUnboundedAbove(el.id, e.target.checked)}
          />
          fills upward (floor)
        </label>
      )}

      {/* A cross-cutting body (vein/intrusion) shows the read-only "cross-cuts" chip the
          CORE derives (effectiveRelation = crossCut), not the 3-state seam control. The
          tooltip records the syn-depositional punt. (Preserved from feature 001.) */}
      {isFloorBoundary && eff === "crossCut" && (
        <div
          className="seam cross-cut-chip"
          title="Cross-cuts older elements (derived from chronology + overlap, not declared). Syn-depositional bodies (younger than some units, older than others) are out of scope — model them as multiple bodies."
        >
          ⨯ cross-cuts
        </div>
      )}

      {/* seam control on the boundary BELOW this series (the floor shows none). */}
      {isFloorBoundary && el.kind === "series" && !el.unboundedAbove && (
        <div className="seam seam-control" role="radiogroup" aria-label="relationship to series below">
          {RELATION_CHOICES.map((rel) => (
            <button
              key={rel}
              className={"seam-state " + (el.relation === rel ? "active" : "")}
              aria-pressed={el.relation === rel}
              onClick={() => p.onSetRelation(el.id, rel)}
            >
              {rel}
            </button>
          ))}
        </div>
      )}
    </div>
  );
}

/** A single surface row — its name + the per-surface "…" menu. Used both flat (singleton
 *  series) and inline (inside a multi-interface series / cross-cutting body). */
function SurfaceRow(props: {
  el: ProjectElement;
  iface: InterfaceData;
  surfaceIndex: number;
  validation?: boolean;
  validationDetail?: string;
  p: HierarchyPanelProps;
  inline?: boolean;
}) {
  const { el, iface, p, inline } = props;
  return (
    <div className={"surface-row" + (inline ? " inline" : "")}>
      {!inline && <span className="kind-icon" data-kind={el.kind} />}
      <span className="surface-name">{iface.name}</span>
      <span className="point-count" title="contact points">{iface.points.length} pts</span>
      {props.validation !== undefined && (
        <span
          className={"validation-badge " + (props.validation ? "ok" : "warn")}
          title={props.validation ? "well constrained" : props.validationDetail ?? ""}
        >
          {props.validation ? "✓" : "!"}
        </span>
      )}
      <SurfaceMenu
        onEdit={() => p.onEditSurface(el.id, iface.id)}
        onRename={(name) => p.onRenameSurface(el.id, iface.id, name)}
        onFlip={() => p.onFlipSurface(el.id, iface.id)}
        flipped={iface.flipped}
        onDuplicate={el.kind === "series" ? () => p.onDuplicateSurface(el.id, iface.id) : undefined}
        onDelete={() => p.onDeleteSurface(el.id, iface.id)}
      />
    </div>
  );
}

/** Up/down chronology reorder for a top-level element — the click/keyboard affordance for
 *  changing stack order (drag-and-drop is unbuilt; the dnd resolver runs the same op). ▲ moves
 *  toward the top (younger); ▼ toward the bottom (older). Disabled at the respective end. */
function ReorderControls(props: {
  name: string;
  isFirst: boolean;
  isLast: boolean;
  onUp: () => void;
  onDown: () => void;
}) {
  return (
    <span className="reorder-controls" aria-label={`reorder ${props.name}`}>
      <button
        type="button"
        className="reorder-btn"
        data-testid="reorder-up"
        title="Move earlier (toward top / younger)"
        aria-label={`move ${props.name} up`}
        disabled={props.isFirst}
        onClick={props.onUp}
      >
        ▲
      </button>
      <button
        type="button"
        className="reorder-btn"
        data-testid="reorder-down"
        title="Move later (toward bottom / older)"
        aria-label={`move ${props.name} down`}
        disabled={props.isLast}
        onClick={props.onDown}
      >
        ▼
      </button>
    </span>
  );
}
