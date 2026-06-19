// The editable STACK panel: an ordered list of series (youngest at the top, oldest at the
// bottom — stratigraphic reading order). Each series has a name, a RELATION select
// (Erode/Onlap/Conformable/Basement), and an optional planar fault (strike + offset). This
// is the editor's analogue of web2's read-only relations schematic — the structure that
// makes the model "an erosional surface cutting a conformable stack" rather than a bag of
// surfaces. Adding/removing a series renumbers the row tables' `series` indices in App.

import { SeriesDef, FaultConfig, SPAN, faultFromControls, faultToControls } from "./data";
import { RELATION, RELATION_CHOICES, relationInfo, cssHex } from "./relations";

interface Props {
  series: SeriesDef[];
  /** The series whose new rows are added to (highlighted). */
  activeSeries: number;
  onSelect: (index: number) => void;
  onChange: (series: SeriesDef[]) => void;
  /** Add a new (empty) youngest series; oldest stays the basement. */
  onAdd: () => void;
  /** Remove a series (and its rows, handled in App). Disabled when only one remains. */
  onRemove: (index: number) => void;
}

export function SeriesPanel({ series, activeSeries, onSelect, onChange, onAdd, onRemove }: Props) {
  const patch = (i: number, p: Partial<SeriesDef>) => {
    const next = series.slice();
    next[i] = { ...next[i], ...p };
    onChange(next);
  };

  const setFaultEnabled = (i: number, enabled: boolean) =>
    patch(i, { fault: enabled ? faultFromControls(0, 0) : null });

  const setFaultControl = (i: number, azimuth: number, offset: number) =>
    patch(i, { fault: faultFromControls(azimuth, offset) });

  return (
    <aside className="series-panel">
      <div className="series-panel-head">
        <span className="tree-title">stack (youngest ↑)</span>
        <button onClick={onAdd}>+ series</button>
      </div>
      {series.map((s, i) => {
        const info = relationInfo(s.relation);
        const ctl = s.fault ? faultToControls(s.fault) : { azimuth: 0, offset: 0 };
        const active = i === activeSeries;
        return (
          <div
            className={"series-card" + (active ? " active" : "")}
            key={i}
            onClick={() => onSelect(i)}
          >
            <div className="series-card-head">
              <span className="tree-swatch" style={{ background: cssHex(info.color) }} />
              <input
                className="series-name"
                value={s.name}
                onChange={(e) => patch(i, { name: e.target.value })}
                onClick={(e) => e.stopPropagation()}
              />
              <button
                className="del"
                title="remove series"
                disabled={series.length <= 1}
                onClick={(e) => {
                  e.stopPropagation();
                  onRemove(i);
                }}
              >
                ×
              </button>
            </div>

            <label className="series-rel" onClick={(e) => e.stopPropagation()}>
              relation
              <select
                value={s.relation}
                onChange={(e) => patch(i, { relation: Number(e.target.value) })}
              >
                {RELATION_CHOICES.map((c) => (
                  <option value={c.code} key={c.code}>
                    {c.name}
                  </option>
                ))}
              </select>
            </label>
            <div className="series-gloss">{info.gloss}</div>

            <label className="series-fault" onClick={(e) => e.stopPropagation()}>
              <input
                type="checkbox"
                checked={s.fault != null}
                onChange={(e) => setFaultEnabled(i, e.target.checked)}
              />
              fault
            </label>
            {s.fault && (
              <div className="series-fault-controls" onClick={(e) => e.stopPropagation()}>
                <label className="res">
                  strike
                  <input
                    type="range"
                    min={0}
                    max={180}
                    step={5}
                    value={ctl.azimuth}
                    onChange={(e) => setFaultControl(i, Number(e.target.value), ctl.offset)}
                  />
                  {ctl.azimuth.toFixed(0)}°
                </label>
                <label className="res">
                  offset
                  <input
                    type="range"
                    min={-SPAN / 2}
                    max={SPAN / 2}
                    step={SPAN / 40}
                    value={ctl.offset}
                    onChange={(e) => setFaultControl(i, ctl.azimuth, Number(e.target.value))}
                  />
                  {ctl.offset.toFixed(0)}
                </label>
              </div>
            )}
          </div>
        );
      })}
      <div className="series-reading">
        ↑ youngest · oldest ↓ — an <em>Erode</em> series truncates everything older; the
        oldest is conventionally the <em>Basement</em>.
      </div>
    </aside>
  );
}

/** Re-export so App can build a default new series without importing data twice. */
export function newSeries(name: string): SeriesDef {
  return { name, relation: RELATION.ERODE, fault: null };
}

/** A fault is on a series iff it has a non-null FaultConfig. */
export function seriesHasFault(s: SeriesDef): s is SeriesDef & { fault: FaultConfig } {
  return s.fault != null;
}
