// Feature 002 (enhanced-authoring) T017 [US1] — the dual-table edit-and-apply data modal.
//
// Opening reads the interface's points/orientations into LOCAL state. The modal's Apply
// commits them back to the project (cheap, local — the caller sets dirty) and closes;
// Cancel discards the modal-local edits. The modal NEVER runs the worker — the panel-level
// Apply re-fits the whole stack. (Two Apply levels by design: modal Apply = commit table
// edits; panel Apply = re-fit. Edit several surfaces, then fit once.)

import { useState } from "react";
import type { OrientationRow, PointRow } from "./project";
import { OrientationsTable, PointsTable } from "./tables";

export function SurfaceDataModal(props: {
  surfaceName: string;
  parentName: string;
  points: PointRow[];
  orientations: OrientationRow[];
  onApply: (points: PointRow[], orientations: OrientationRow[]) => void;
  onCancel: () => void;
}) {
  const [points, setPoints] = useState<PointRow[]>(props.points.map((p) => ({ ...p })));
  const [orientations, setOrientations] = useState<OrientationRow[]>(
    props.orientations.map((o) => ({ ...o })),
  );

  const canApply = points.length > 0;
  const summary = canApply
    ? `${points.length} point${points.length === 1 ? "" : "s"}, ${orientations.length} orientation${
        orientations.length === 1 ? "" : "s"
      } — OK`
    : "0 points — add at least one";

  return (
    <div className="modal-backdrop" data-testid="surface-data-modal" onClick={props.onCancel}>
      <div className="modal" role="dialog" aria-label="edit surface data" onClick={(e) => e.stopPropagation()}>
        <div className="modal-head">
          <strong>{props.surfaceName}</strong>
          <span className="modal-parent">{props.parentName}</span>
        </div>

        <p className="modal-hint" title="X=East, Y=North, Z=Up; dip-direction is clockwise from North (+Y)">
          Orientations: dip = degrees from horizontal, dip-direction = degrees clockwise from North
          (+Y). Axes: X=East, Y=North, Z=Up. Paste TSV/CSV into a table to bulk-add rows.
        </p>

        <div className="modal-tables">
          <PointsTable rows={points} onChange={setPoints} />
          <OrientationsTable rows={orientations} onChange={setOrientations} />
        </div>

        <div className="modal-foot">
          <span className={"modal-summary " + (canApply ? "ok" : "warn")} data-testid="modal-summary">
            {summary}
          </span>
          <button
            type="button"
            className="apply-btn"
            data-testid="modal-apply"
            disabled={!canApply}
            onClick={() => props.onApply(points, orientations)}
          >
            Apply
          </button>
          <button type="button" data-testid="modal-cancel" onClick={props.onCancel}>
            Cancel
          </button>
        </div>
      </div>
    </div>
  );
}
