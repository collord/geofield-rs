// Feature 002 (enhanced-authoring) T008 — the reusable points/orientations table.
//
// Pure presentational: takes typed rows + change callbacks. Supports per-cell numeric
// entry with Tab/Enter navigation (Enter on the last row adds a row), per-row delete, a
// row-count/validity badge, and clipboard paste of TSV/CSV mapped to columns IN ORDER.
// Wrong-width rows are FLAGGED, not silently dropped or truncated (Constitution IV).

import { useState } from "react";
import type { PointRow, OrientationRow } from "./project";

/** A column descriptor: the field key and its header label. */
interface ColumnSpec<Row> {
  key: keyof Row;
  label: string;
}

const POINT_COLUMNS: ColumnSpec<PointRow>[] = [
  { key: "x", label: "X" },
  { key: "y", label: "Y" },
  { key: "z", label: "Z" },
];

const ORIENTATION_COLUMNS: ColumnSpec<OrientationRow>[] = [
  { key: "x", label: "X" },
  { key: "y", label: "Y" },
  { key: "z", label: "Z" },
  { key: "dip", label: "Dip" },
  { key: "dipDir", label: "Dip Dir" },
];

/** Parse a pasted TSV/CSV blob into numeric rows of exactly `width` columns. Rows whose
 *  width differs are returned in `flagged` (not dropped) so the UI can report them. */
function parsePaste(
  text: string,
  width: number,
): { rows: number[][]; flagged: { line: number; raw: string; got: number }[] } {
  const rows: number[][] = [];
  const flagged: { line: number; raw: string; got: number }[] = [];
  const lines = text.split(/\r?\n/).filter((l) => l.trim().length > 0);
  lines.forEach((line, i) => {
    const cells = line.split(/[,\t]+|\s{2,}|\s+/).filter((c) => c.length > 0);
    if (cells.length !== width) {
      flagged.push({ line: i + 1, raw: line, got: cells.length });
      return;
    }
    const nums = cells.map((c) => Number(c));
    if (nums.some((n) => Number.isNaN(n))) {
      flagged.push({ line: i + 1, raw: line, got: cells.length });
      return;
    }
    rows.push(nums);
  });
  return { rows, flagged };
}

interface DataTableProps<Row> {
  title: string;
  columns: ColumnSpec<Row>[];
  rows: Row[];
  onChange: (rows: Row[]) => void;
  emptyRow: () => Row;
  /** test id stem (e.g. "points" / "orientations"). */
  testid: string;
}

// Read/write a numeric field by key without requiring an index signature on Row.
function getNum<Row>(row: Row, key: keyof Row): number {
  const v = row[key] as unknown as number;
  return Number.isFinite(v) ? v : 0;
}

function DataTable<Row extends object>(props: DataTableProps<Row>) {
  const { title, columns, rows, onChange, emptyRow, testid } = props;
  const [pasteNote, setPasteNote] = useState<string>("");

  const setCell = (ri: number, key: keyof Row, value: number) => {
    const next = rows.map((r, i) => (i === ri ? { ...r, [key]: value } : r));
    onChange(next);
  };
  const addRow = () => onChange([...rows, emptyRow()]);
  const deleteRow = (ri: number) => onChange(rows.filter((_, i) => i !== ri));

  const onPaste = (e: React.ClipboardEvent) => {
    const text = e.clipboardData.getData("text");
    if (!text.trim()) return;
    e.preventDefault();
    const { rows: parsed, flagged } = parsePaste(text, columns.length);
    if (parsed.length > 0) {
      const mapped = parsed.map((nums) => {
        const r = emptyRow();
        columns.forEach((c, ci) => {
          (r as Record<keyof Row, number>)[c.key] = nums[ci];
        });
        return r;
      });
      onChange([...rows, ...mapped]);
    }
    setPasteNote(
      flagged.length > 0
        ? `pasted ${parsed.length} row(s); flagged ${flagged.length} (expected ${columns.length} columns): line(s) ${flagged
            .map((f) => f.line)
            .join(", ")}`
        : `pasted ${parsed.length} row(s)`,
    );
  };

  const onKeyDown = (e: React.KeyboardEvent, ri: number) => {
    if (e.key === "Enter") {
      e.preventDefault();
      if (ri === rows.length - 1) addRow();
    }
  };

  return (
    <div className="data-table" data-testid={`table-${testid}`} onPaste={onPaste}>
      <div className="data-table-head">
        <strong>{title}</strong>
        <span className="row-count" data-testid={`rowcount-${testid}`}>
          {rows.length} row{rows.length === 1 ? "" : "s"}
        </span>
      </div>
      <table>
        <thead>
          <tr>
            {columns.map((c) => (
              <th key={String(c.key)}>{c.label}</th>
            ))}
            <th aria-label="delete" />
          </tr>
        </thead>
        <tbody>
          {rows.map((row, ri) => (
            <tr key={ri}>
              {columns.map((c) => (
                <td key={String(c.key)}>
                  <input
                    type="number"
                    value={getNum(row, c.key)}
                    onChange={(e) => setCell(ri, c.key, Number(e.target.value))}
                    onKeyDown={(e) => onKeyDown(e, ri)}
                  />
                </td>
              ))}
              <td>
                <button type="button" className="row-del" title="delete row" onClick={() => deleteRow(ri)}>
                  ⌫
                </button>
              </td>
            </tr>
          ))}
        </tbody>
      </table>
      <div className="data-table-foot">
        <button type="button" onClick={addRow} data-testid={`addrow-${testid}`}>
          + row
        </button>
        {pasteNote && <span className="paste-note">{pasteNote}</span>}
      </div>
    </div>
  );
}

export function emptyPoint(): PointRow {
  return { x: 0, y: 0, z: 0 };
}
export function emptyOrientation(): OrientationRow {
  return { x: 0, y: 0, z: 0, dip: 0, dipDir: 0 };
}

export function PointsTable(props: { rows: PointRow[]; onChange: (rows: PointRow[]) => void }) {
  return (
    <DataTable
      title="Points"
      columns={POINT_COLUMNS}
      rows={props.rows}
      onChange={props.onChange}
      emptyRow={emptyPoint}
      testid="points"
    />
  );
}

export function OrientationsTable(props: {
  rows: OrientationRow[];
  onChange: (rows: OrientationRow[]) => void;
}) {
  return (
    <DataTable
      title="Orientations (optional)"
      columns={ORIENTATION_COLUMNS}
      rows={props.rows}
      onChange={props.onChange}
      emptyRow={emptyOrientation}
      testid="orientations"
    />
  );
}
