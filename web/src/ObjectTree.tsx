// Object tree: lists the mesh surfaces grouped by SERIES → fault block, with checkboxes to
// hide/show each surface (and each series / block as a whole). Visibility lives in App as a
// Set of hidden surface keys; the Viewer reads the same set to toggle THREE objects.

import { SurfaceMesh, surfaceKey, surfaceLabel, pickColorCss } from "./model";
import { SeriesDef } from "./data";
import { relationInfo, cssHex } from "./relations";

interface Props {
  surfaces: SurfaceMesh[];
  series: SeriesDef[];
  hidden: Set<string>;
  onToggle: (key: string, hide: boolean) => void;
  onToggleMany: (keys: string[], hide: boolean) => void;
}

export function ObjectTree({ surfaces, series, hidden, onToggle, onToggleMany }: Props) {
  const drawn = surfaces.filter((s) => s.triangleCount > 0);
  if (drawn.length === 0) {
    return (
      <aside className="tree">
        <div className="tree-title">objects</div>
        <div className="tree-empty">no surfaces</div>
      </aside>
    );
  }

  // Bucket by series, then by block within a series.
  const bySeries = new Map<number, SurfaceMesh[]>();
  for (const s of drawn) {
    if (!bySeries.has(s.series)) bySeries.set(s.series, []);
    bySeries.get(s.series)!.push(s);
  }
  // Series youngest (0) → oldest, matching the stack panel's reading order.
  const seriesIdx = [...bySeries.keys()].sort((a, b) => a - b);

  const groupCheckbox = (items: SurfaceMesh[], onChange: (hide: boolean) => void) => {
    const keys = items.map(surfaceKey);
    const allHidden = keys.every((k) => hidden.has(k));
    const anyHidden = keys.some((k) => hidden.has(k));
    return (
      <input
        type="checkbox"
        checked={!allHidden}
        ref={(el) => {
          if (el) el.indeterminate = anyHidden && !allHidden;
        }}
        onChange={(e) => onChange(!e.target.checked)}
      />
    );
  };

  return (
    <aside className="tree">
      <div className="tree-title">objects</div>
      {seriesIdx.map((si) => {
        const items = bySeries.get(si)!;
        const def = series[si];
        const info = relationInfo(def?.relation ?? 0);
        const seriesKeys = items.map(surfaceKey);
        // Within a series, sub-group by block when faulted (>1 block present).
        const blocks = new Map<number, SurfaceMesh[]>();
        for (const s of items) {
          if (!blocks.has(s.block)) blocks.set(s.block, []);
          blocks.get(s.block)!.push(s);
        }
        const multiBlock = blocks.size > 1;

        return (
          <div className="tree-group" key={si}>
            <label className="tree-row tree-grouphead">
              {groupCheckbox(items, (hide) => onToggleMany(seriesKeys, hide))}
              <span className="tree-swatch" style={{ background: cssHex(info.color) }} />
              <span className="tree-name">
                {def?.name ?? `series ${si}`}
                <span className="tree-rel"> · {info.name}</span>
              </span>
              <span className="tree-count">{items.length}</span>
            </label>

            {[...blocks.entries()]
              .sort((a, b) => a[0] - b[0])
              .map(([blk, blkItems]) => (
                <div key={blk}>
                  {multiBlock && (
                    <label className="tree-row tree-blockhead">
                      {groupCheckbox(blkItems, (hide) =>
                        onToggleMany(blkItems.map(surfaceKey), hide),
                      )}
                      <span className="tree-name">Block {blockName(blk)}</span>
                      <span className="tree-count">{blkItems.length}</span>
                    </label>
                  )}
                  {blkItems.map((s) => {
                    const k = surfaceKey(s);
                    return (
                      <label className="tree-row tree-leaf" key={k}>
                        <input
                          type="checkbox"
                          checked={!hidden.has(k)}
                          onChange={(e) => onToggle(k, !e.target.checked)}
                        />
                        <span
                          className="tree-swatch"
                          style={{ background: pickColorCss(s) }}
                        />
                        <span className="tree-name">{surfaceLabel(s)}</span>
                        <span className="tree-count">
                          {s.triangleCount.toLocaleString()}
                        </span>
                      </label>
                    );
                  })}
                </div>
              ))}
          </div>
        );
      })}
    </aside>
  );
}

/** Block 0 → "A", 1 → "B", … (matches how the blocks read in the viewer). */
function blockName(b: number): string {
  return String.fromCharCode(65 + b);
}
