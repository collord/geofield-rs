// Object tree: lists the mesh surfaces grouped by fault block, with checkboxes to
// hide/show each surface (and each block as a whole). Visibility lives in App as a
// Set of hidden surface keys; the Viewer reads the same set to toggle THREE objects.

import { SurfaceMesh, surfaceKey, surfaceLabel, pickColorCss } from "./model";

interface Props {
  surfaces: SurfaceMesh[];
  hidden: Set<string>;
  onToggle: (key: string, hide: boolean) => void;
  onToggleMany: (keys: string[], hide: boolean) => void;
}

/** A small colored swatch matching the surface's render color is overkill here; the
 *  tree is about structure + visibility, so it stays text + checkboxes. */
export function ObjectTree({ surfaces, hidden, onToggle, onToggleMany }: Props) {
  const drawn = surfaces.filter((s) => s.triangleCount > 0);
  if (drawn.length === 0) {
    return (
      <aside className="tree">
        <div className="tree-title">objects</div>
        <div className="tree-empty">no surfaces</div>
      </aside>
    );
  }

  // Group by block (undefined block → a single "Model" group).
  const groups = new Map<string, SurfaceMesh[]>();
  for (const s of drawn) {
    const g = s.block === undefined ? "Model" : `Block ${blockName(s.block)}`;
    if (!groups.has(g)) groups.set(g, []);
    groups.get(g)!.push(s);
  }

  return (
    <aside className="tree">
      <div className="tree-title">objects</div>
      {[...groups.entries()].map(([groupName, items]) => {
        const keys = items.map(surfaceKey);
        const allHidden = keys.every((k) => hidden.has(k));
        const anyHidden = keys.some((k) => hidden.has(k));
        return (
          <div className="tree-group" key={groupName}>
            <label className="tree-row tree-grouphead">
              <input
                type="checkbox"
                checked={!allHidden}
                ref={(el) => {
                  if (el) el.indeterminate = anyHidden && !allHidden;
                }}
                onChange={(e) => onToggleMany(keys, !e.target.checked)}
              />
              <span className="tree-name">{groupName}</span>
              <span className="tree-count">{items.length}</span>
            </label>
            {items.map((s) => {
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
        );
      })}
    </aside>
  );
}

/** Block 0 → "A", 1 → "B", … (matches how the blocks read in the viewer). */
function blockName(b: number): string {
  return String.fromCharCode(65 + b);
}
