import { SurfacePointRow, OrientationRow, SeriesDef } from "./data";

const NUM_COLS_SP: (keyof SurfacePointRow)[] = ["x", "y", "z"];
const NUM_COLS_ORI: (keyof OrientationRow)[] = [
  "x",
  "y",
  "z",
  "azimuth",
  "dip",
  "polarity",
];

/** The series `<select>` shared by both tables: pick which series a row belongs to. */
function SeriesSelect({
  series,
  value,
  onChange,
}: {
  series: SeriesDef[];
  value: number;
  onChange: (v: number) => void;
}) {
  return (
    <select value={value} onChange={(e) => onChange(Number(e.target.value))}>
      {series.map((s, i) => (
        <option value={i} key={i}>
          {s.name}
        </option>
      ))}
    </select>
  );
}

interface SPProps {
  rows: SurfacePointRow[];
  series: SeriesDef[];
  activeSeries: number;
  onChange: (rows: SurfacePointRow[]) => void;
}

export function SurfacePointTable({ rows, series, activeSeries, onChange }: SPProps) {
  const setCell = (i: number, key: keyof SurfacePointRow, raw: string) => {
    const next = rows.slice();
    if (key === "formation") next[i] = { ...next[i], formation: raw };
    else next[i] = { ...next[i], [key]: Number(raw) };
    onChange(next);
  };
  const addRow = () =>
    onChange([
      ...rows,
      { x: 0, y: 0, z: 0, formation: "rock1", series: activeSeries },
    ]);
  const delRow = (i: number) => onChange(rows.filter((_, j) => j !== i));

  return (
    <div className="table-wrap">
      <div className="table-head">
        <h3>Surface points ({rows.length})</h3>
        <button onClick={addRow}>+ row</button>
      </div>
      <table>
        <thead>
          <tr>
            <th>X</th>
            <th>Y</th>
            <th>Z</th>
            <th>formation</th>
            <th>series</th>
            <th></th>
          </tr>
        </thead>
        <tbody>
          {rows.map((r, i) => (
            <tr key={i}>
              {NUM_COLS_SP.map((c) => (
                <td key={c}>
                  <input
                    type="number"
                    value={r[c]}
                    onChange={(e) => setCell(i, c, e.target.value)}
                  />
                </td>
              ))}
              <td>
                <input
                  value={r.formation}
                  onChange={(e) => setCell(i, "formation", e.target.value)}
                />
              </td>
              <td>
                <SeriesSelect
                  series={series}
                  value={r.series}
                  onChange={(v) => setCell(i, "series", String(v))}
                />
              </td>
              <td>
                <button className="del" onClick={() => delRow(i)}>
                  ×
                </button>
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

interface OriProps {
  rows: OrientationRow[];
  series: SeriesDef[];
  activeSeries: number;
  onChange: (rows: OrientationRow[]) => void;
}

export function OrientationTable({ rows, series, activeSeries, onChange }: OriProps) {
  const setCell = (i: number, key: keyof OrientationRow, raw: string) => {
    const next = rows.slice();
    if (key === "formation") next[i] = { ...next[i], formation: raw };
    else next[i] = { ...next[i], [key]: Number(raw) };
    onChange(next);
  };
  const addRow = () =>
    onChange([
      ...rows,
      {
        x: 0,
        y: 0,
        z: 0,
        azimuth: 90,
        dip: 0,
        polarity: 1,
        formation: "rock1",
        series: activeSeries,
      },
    ]);
  const delRow = (i: number) => onChange(rows.filter((_, j) => j !== i));

  return (
    <div className="table-wrap">
      <div className="table-head">
        <h3>Orientations ({rows.length})</h3>
        <button onClick={addRow}>+ row</button>
      </div>
      <table>
        <thead>
          <tr>
            <th>X</th>
            <th>Y</th>
            <th>Z</th>
            <th>azimuth</th>
            <th>dip</th>
            <th>polarity</th>
            <th>formation</th>
            <th>series</th>
            <th></th>
          </tr>
        </thead>
        <tbody>
          {rows.map((r, i) => (
            <tr key={i}>
              {NUM_COLS_ORI.map((c) => (
                <td key={c}>
                  <input
                    type="number"
                    value={r[c]}
                    onChange={(e) => setCell(i, c, e.target.value)}
                  />
                </td>
              ))}
              <td>
                <input
                  value={r.formation}
                  onChange={(e) => setCell(i, "formation", e.target.value)}
                />
              </td>
              <td>
                <SeriesSelect
                  series={series}
                  value={r.series}
                  onChange={(v) => setCell(i, "series", String(v))}
                />
              </td>
              <td>
                <button className="del" onClick={() => delRow(i)}>
                  ×
                </button>
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}
