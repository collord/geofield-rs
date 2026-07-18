// Feature 002 (enhanced-authoring) T017 [US1] — the dual-table edit-and-apply data modal.
//
// Opening reads the interface's points/orientations into LOCAL state. The modal's Apply
// commits them back to the project (cheap, local — the caller sets dirty) and closes;
// Cancel discards the modal-local edits. The modal NEVER runs the worker — the panel-level
// Apply re-fits the whole stack. (Two Apply levels by design: modal Apply = commit table
// edits; panel Apply = re-fit. Edit several surfaces, then fit once.)
//
// Feature 005: a surface's DIGITIZED vertices appear here too — the table and the section
// drawing are two views of ONE dataset (spec FR-012). Edits write back through the line
// (plane-projected on Apply, applyInterfaceData); vertex ORDER is the drawn direction and
// is not editable here (redraw/flip on the section instead).

import { useState } from "react";
import type { DigitizedLine, OrientationRow, PointRow } from "./project";
import { OrientationsTable, PointsTable } from "./tables";

export function SurfaceDataModal(props: {
  surfaceName: string;
  parentName: string;
  points: PointRow[];
  orientations: OrientationRow[];
  /** The surface's digitized section lines (feature 005); absent/empty for a typed-only surface. */
  digitized?: DigitizedLine[];
  onApply: (points: PointRow[], orientations: OrientationRow[], digitized: DigitizedLine[]) => void;
  onCancel: () => void;
}) {
  const [points, setPoints] = useState<PointRow[]>(props.points.map((p) => ({ ...p })));
  const [orientations, setOrientations] = useState<OrientationRow[]>(
    props.orientations.map((o) => ({ ...o })),
  );
  const [digitized, setDigitized] = useState<DigitizedLine[]>(
    (props.digitized ?? []).map((l) => ({ ...l, vertices: l.vertices.map((v) => [...v] as [number, number, number]) })),
  );

  const digitizedCount = digitized.reduce((n, l) => n + l.vertices.length, 0);
  const canApply = points.length + digitizedCount > 0;
  const summary = canApply
    ? `${points.length} point${points.length === 1 ? "" : "s"}` +
      (digitizedCount > 0 ? ` + ${digitizedCount} digitized` : "") +
      `, ${orientations.length} orientation${orientations.length === 1 ? "" : "s"} — OK`
    : "0 points — add at least one";

  const setVertex = (li: number, vi: number, axis: 0 | 1 | 2, value: number) => {
    setDigitized((prev) =>
      prev.map((l, i) =>
        i === li
          ? { ...l, vertices: l.vertices.map((v, j) => (j === vi ? (v.map((c, k) => (k === axis ? value : c)) as [number, number, number]) : v)) }
          : l,
      ),
    );
  };
  const deleteVertex = (li: number, vi: number) => {
    setDigitized((prev) =>
      prev
        .map((l, i) => (i === li ? { ...l, vertices: l.vertices.filter((_, j) => j !== vi) } : l))
        .filter((l) => l.vertices.length > 0),
    );
  };

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

        {digitized.length > 0 && (
          <div className="digitized-table" data-testid="digitized-table" style={{ marginTop: "0.5rem" }}>
            <strong style={{ fontSize: "0.85em" }}>Digitized vertices</strong>
            <span style={{ fontSize: "0.7em", color: "#789", marginLeft: "0.5em" }}>
              (edits stay on each line's section plane; order = drawn direction — redraw or flip on the section)
            </span>
            <table style={{ fontSize: "0.8em", borderCollapse: "collapse" }}>
              <thead>
                <tr><th>line</th><th>#</th><th>x</th><th>y</th><th>z</th><th /></tr>
              </thead>
              <tbody>
                {digitized.flatMap((l, li) =>
                  l.vertices.map((v, vi) => (
                    <tr key={`${l.id}:${vi}`}>
                      <td style={{ color: "#789" }}>{li + 1}</td>
                      <td style={{ color: "#789" }}>{vi + 1}</td>
                      {[0, 1, 2].map((axis) => (
                        <td key={axis}>
                          <input
                            type="number"
                            style={{ width: 80 }}
                            value={v[axis]}
                            data-testid={`dig-${li}-${vi}-${"xyz"[axis]}`}
                            onChange={(e) => {
                              const n = Number.parseFloat(e.target.value);
                              if (Number.isFinite(n)) setVertex(li, vi, axis as 0 | 1 | 2, n);
                            }}
                          />
                        </td>
                      ))}
                      <td>
                        <button type="button" title="delete vertex" onClick={() => deleteVertex(li, vi)}>✕</button>
                      </td>
                    </tr>
                  )),
                )}
              </tbody>
            </table>
          </div>
        )}

        <div className="modal-foot">
          <span className={"modal-summary " + (canApply ? "ok" : "warn")} data-testid="modal-summary">
            {summary}
          </span>
          <button
            type="button"
            className="apply-btn"
            data-testid="modal-apply"
            disabled={!canApply}
            onClick={() => props.onApply(points, orientations, digitized)}
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
