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

import { Fragment } from "react";
import { DndContext, type DragEndEvent } from "@dnd-kit/core";
import { CoreUpdate, RelationDto, effectiveRelationOf, validationOf, unitsOf } from "./dto";
import { resolveDrop, type DropAction, type NodeType } from "./dragResolver";
import type { InterfaceData, Lithology, Project, ProjectElement, ProjectValidation } from "./project";
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
  /** Set an imported surface's role (feature 004). A geometry edit → re-fits on Apply. */
  onSetImportRole?: (elementId: string, role: "contact" | "erosion" | "veinWall" | "topography") => void;
  /** Toggle projecting an imported surface to the domain bounds (feature 004 US2). */
  onSetImportExtend?: (elementId: string, extend: boolean) => void;
  /** Switch an intrusion between solid (one envelope) and shell (inner+outer). Geometry edit. */
  onSetIntrusionMode?: (elementId: string, mode: "solid" | "shell") => void;
  // unit ladder (US2/US3)
  /** Display name for a derived unit: the user's name if set, else "Unit {displayPos}" (the
   *  visual ladder position, 1 = top). The stored name is keyed by the stable `unitIndex`. */
  unitName: (elementId: string, unitIndex: number, displayPos: number) => string;
  onRenameUnit: (elementId: string, unitIndex: number, name: string) => void;
  /** Lithology assigned to a unit + the catalog to choose from (US3). When absent, the unit
   *  row shows just its name (US2 standalone). */
  lithologyOf?: (elementId: string, unitIndex: number) => Lithology | undefined;
  catalog?: Lithology[];
  onAssignLithology?: (elementId: string, unitIndex: number, lithologyId: string) => void;
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
  // Derived unit coordinates for THIS element (youngest→oldest), from the core (FR-009).
  const units = unitsOf(p.update.derived, coreId);

  // Topography renders FLAT (its single ground contact; no rock units — Air above).
  const flatSurface = el.kind === "topography";

  // Contacts (interfaces) of the element, in render order. A series' interfaces are stored
  // ascending-isovalue (oldest contact first); the ladder is youngest-at-top, so reverse.
  const contacts: { iface: InterfaceData; surfaceIndex: number }[] =
    el.kind === "series"
      ? el.interfaces.map((iface, si) => ({ iface, surfaceIndex: si })).reverse()
      : el.kind === "vein"
        ? [
            { iface: el.hangingWall, surfaceIndex: 1 },
            { iface: el.footWall, surfaceIndex: 0 },
          ]
        : el.kind === "topography"
          ? [{ iface: el.ground, surfaceIndex: 0 }]
          : el.kind === "intrusion"
            ? (el.mode ?? "shell") === "solid"
              ? [{ iface: el.envelopeOuter, surfaceIndex: 0 }]
              : [
                  { iface: el.envelopeOuter, surfaceIndex: 1 },
                  { iface: el.envelopeInner, surfaceIndex: 0 },
                ]
            : // importedSurface: the geometry IS the mesh (no editable InterfaceData contacts).
              // Its units still render from the derived view; the mesh row is shown separately.
              [];

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
      {flatSurface ? (
        // FLAT: topography is just "the ground surface". The reorder control leads the row.
        <div className="surface-row-flat">
          {reorder}
          <SurfaceRow
            el={el}
            iface={contacts[0].iface}
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
            {el.kind === "importedSurface" && (
              // Role selector for an imported surface (feature 004): contact / erosion / topography.
              // Changing it re-clips older elements; a geometry edit that re-fits on Apply.
              <select
                data-testid="import-role"
                value={el.role}
                onChange={(e) =>
                  p.onSetImportRole?.(
                    el.id,
                    e.target.value as "contact" | "erosion" | "veinWall" | "topography",
                  )
                }
                aria-label="imported surface role"
              >
                <option value="contact">contact</option>
                <option value="erosion">erosion</option>
                <option value="topography">topography</option>
              </select>
            )}
            {el.kind === "importedSurface" && (
              // Extend-to-bounds toggle (feature 004 US2): OFF by default (footprint-only, no
              // extrapolation). Turning it on projects the surface to the domain box via co-kriging —
              // warn, since extrapolating real (e.g. lidar) data beyond its footprint is often wrong.
              <label
                data-testid="import-extend"
                title="Project this surface beyond its data footprint out to the domain box (co-kriged extrapolation). Off = use the mesh only where it has data."
                style={{ fontSize: "0.8em", display: "inline-flex", alignItems: "center", gap: "0.25em" }}
              >
                <input
                  type="checkbox"
                  checked={el.extendToBounds}
                  onChange={(e) => {
                    if (
                      e.target.checked &&
                      !window.confirm(
                        "Extend this imported surface beyond its data footprint, out to the domain bounds?\n\n" +
                          "This EXTRAPOLATES the surface (co-kriging) into areas it has no data — often not physically meaningful for scanned/lidar data. Leave off to use the mesh only where it actually covers.",
                      )
                    ) {
                      return; // cancelled — leave unchecked
                    }
                    p.onSetImportExtend?.(el.id, e.target.checked);
                  }}
                />
                extend
              </label>
            )}
            {el.kind === "intrusion" && (
              // Solid vs shell (feature: imported mesh surfaces). Solid = one envelope (a filled
              // stock/pluton). Shell = a hollow-cored body between an inner and outer wall (ring
              // dike / cored pluton). A geometry edit that re-fits on Apply. Switching to solid
              // hides the inner envelope but preserves its authored points.
              <select
                data-testid="intrusion-mode"
                value={el.mode ?? "shell"}
                onChange={(e) => p.onSetIntrusionMode?.(el.id, e.target.value as "solid" | "shell")}
                aria-label="intrusion mode"
                title="Solid = a filled body bounded by one envelope. Shell = a hollow-cored body between an inner and outer envelope (ring dike / cored pluton)."
              >
                <option value="solid">solid</option>
                <option value="shell">shell</option>
              </select>
            )}
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
          {/* The UNIT/CONTACT LADDER (feature 003 §1), youngest at top: a series with N
              contacts shows N+1 unit rows interleaved with N contact rows. Units come from
              the core's derived coordinates (never authored); contacts are the existing
              authoring surfaces. A cross-cutting body shows its interior unit + its walls. */}
          <ul className="ladder">
            {units.map((u, ui) => (
              <Fragment key={`u-${u.index}`}>
                <li className="unit-node" data-element-id={el.id} data-unit-index={u.index}>
                  <UnitRow el={el} unitIndex={u.index} displayPos={ui + 1} p={p} />
                </li>
                {contacts[ui] && (
                  <li
                    className="surface-node"
                    data-element-id={el.id}
                    data-surface-index={contacts[ui].surfaceIndex}
                  >
                    <SurfaceRow el={el} iface={contacts[ui].iface} surfaceIndex={contacts[ui].surfaceIndex} p={p} inline />
                  </li>
                )}
              </Fragment>
            ))}
            {/* Any contacts without a paired unit above (e.g. a field-less element with no
                derived units) still render so authoring is never lost. */}
            {contacts.slice(units.length).map(({ iface, surfaceIndex }) => (
              <li key={iface.id} className="surface-node" data-element-id={el.id} data-surface-index={surfaceIndex}>
                <SurfaceRow el={el} iface={iface} surfaceIndex={surfaceIndex} p={p} inline />
              </li>
            ))}
          </ul>
        </>
      )}

      <div className="derived-summary">effective: {eff ?? "—"}</div>

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

/** A derived UNIT row (feature 003 §1) — its name (inline-editable, unlike a contact) plus,
 *  when a catalog is wired (US3), a lithology dropdown + colour swatch. The unit itself is
 *  derived by the core; only its NAME and lithology ASSIGNMENT are UI metadata. */
function UnitRow(props: { el: ProjectElement; unitIndex: number; displayPos: number; p: HierarchyPanelProps }) {
  const { el, unitIndex, displayPos, p } = props;
  // The stored name (if any) is keyed by the stable mesher index; the DEFAULT label uses the
  // visual ladder position (1 = top) so "Unit 1" is the top slab, matching the 3D view.
  const name = p.unitName(el.id, unitIndex, displayPos);
  const litho = p.lithologyOf?.(el.id, unitIndex);
  return (
    <div className="unit-row" data-testid="unit-row">
      <span className="unit-marker" aria-hidden style={litho ? { background: litho.color } : undefined} />
      <input
        className="unit-name"
        data-testid="unit-name"
        aria-label={`unit ${unitIndex + 1} name`}
        value={name}
        onChange={(e) => p.onRenameUnit(el.id, unitIndex, e.target.value)}
      />
      {p.catalog && p.onAssignLithology && (
        <select
          className="unit-litho"
          data-testid="unit-litho"
          aria-label={`lithology for ${name}`}
          value={litho?.id ?? "unknown"}
          onChange={(e) => p.onAssignLithology!(el.id, unitIndex, e.target.value)}
        >
          {p.catalog.map((l) => (
            <option key={l.id} value={l.id}>
              {l.name}
            </option>
          ))}
        </select>
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
