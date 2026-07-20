// Drillhole import modal (prototype): paste a POST-DESURVEY lithology interval table, preview
// the parse (holes / intervals / inferred unit order / per-surface contact counts / warnings)
// live, and Apply to auto-build the conformable stack. Mirrors SurfaceDataModal's two-Apply
// design: THIS Apply only commits the derived elements (the caller sets dirty); the panel-level
// Apply runs the fit.

import { useMemo, useState } from "react";
import type { DrillholeInterval } from "./project";
import { extractContacts, inferUnitOrder, parseDrillholeTable } from "./drillholes";

const HINT =
  "Columns (header row, any order; tab/comma-separated): holeid, start_x, start_y, start_z, " +
  "end_x, end_y, end_z, unitname. Intervals are post-desurvey (3D endpoints); extra columns like " +
  "geolfrom/geolto are ignored. Contacts are extracted at unit transitions and grouped as " +
  "“top of <unit>” surfaces; re-importing replaces surfaces with those names.";

export function DrillholeImportModal(props: {
  onApply: (intervals: DrillholeInterval[]) => void;
  onCancel: () => void;
}) {
  const [text, setText] = useState("");

  const preview = useMemo(() => {
    if (text.trim() === "") return null;
    const parsed = parseDrillholeTable(text);
    if (parsed.errors.length > 0) return { parsed, lines: [`✗ ${parsed.errors.join("; ")}`], canApply: false };
    const { order, hadCycle } = inferUnitOrder(parsed.intervals);
    const { byUnit, warnings } = extractContacts(parsed.intervals);
    const holes = new Set(parsed.intervals.map((i) => i.holeId)).size;
    const contacts = [...byUnit.values()].reduce((n, pts) => n + pts.length, 0);
    const lines = [
      `${holes} hole${holes === 1 ? "" : "s"}, ${parsed.intervals.length} intervals, ${contacts} contacts` +
        (parsed.flagged.length > 0 ? ` (${parsed.flagged.length} rows flagged)` : ""),
      `youngest → oldest: ${order.join(", ")}`,
      ...order.filter((u) => byUnit.has(u)).map((u) => `  top of ${u}: ${byUnit.get(u)!.length} point${byUnit.get(u)!.length === 1 ? "" : "s"}`),
      ...(hadCycle ? ["⚠ contradictory unit order across holes — broke the tie by mean elevation"] : []),
      ...warnings.map((w) => `⚠ ${w}`),
      ...order.filter((u) => byUnit.has(u) && byUnit.get(u)!.length < 2).map((u) => `⚠ top of ${u} has <2 points — fit may collapse`),
      ...parsed.flagged.slice(0, 5).map((f) => `⚠ line ${f.line} skipped: ${f.reason}`),
    ];
    return { parsed, lines, canApply: contacts > 0 };
  }, [text]);

  return (
    <div className="modal-backdrop" data-testid="drillhole-import-modal" onClick={props.onCancel}>
      <div className="modal" role="dialog" aria-label="import drillholes" onClick={(e) => e.stopPropagation()}>
        <div className="modal-head">
          <strong>Import drillholes</strong>
          <span className="modal-parent">lithology intervals → contact surfaces</span>
        </div>

        <p className="modal-hint">{HINT}</p>

        <textarea
          data-testid="drillhole-text"
          value={text}
          onChange={(e) => setText(e.target.value)}
          placeholder={"holeid\tstart_x\tstart_y\tstart_z\tend_x\tend_y\tend_z\tunitname"}
          spellCheck={false}
          style={{ width: "100%", minHeight: "10rem", fontFamily: "monospace", boxSizing: "border-box" }}
        />

        <pre className="modal-hint" data-testid="drillhole-preview" style={{ whiteSpace: "pre-wrap", margin: "0.5rem 0" }}>
          {preview ? preview.lines.join("\n") : "paste a table to preview"}
        </pre>

        <div className="modal-foot">
          <button
            type="button"
            className="apply-btn"
            data-testid="drillhole-apply"
            disabled={!preview?.canApply}
            onClick={() => preview?.canApply && props.onApply(preview.parsed.intervals)}
          >
            Apply
          </button>
          <button type="button" data-testid="drillhole-cancel" onClick={props.onCancel}>
            Cancel
          </button>
        </div>
      </div>
    </div>
  );
}
