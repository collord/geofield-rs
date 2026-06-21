// Feature 003 debug aid — per-volume visibility toggles for the 3D view.
//
// Lists each rendered payload (surface or unit solid) with a checkbox so volumes can be
// hidden one at a time for visual debugging. Keyed by `surfaceKey` (the same key the Viewer
// filters on). Purely a diagnostic panel — not part of the authoring model.

import type { SurfacePayload } from "../protocol";
import { surfaceKey } from "../Viewer";
import type { Lithology } from "./project";

export interface VolumeVisibilityProps {
  surfaces: SurfacePayload[];
  hiddenKeys: Set<string>;
  onToggle: (key: string, visible: boolean) => void;
  onShowAll: () => void;
  /** Solid mode: lithology id → entry, for label + swatch. */
  catalog?: Record<string, Lithology>;
  /** Per-payload display label (e.g. the unit/lithology name). */
  label: (s: SurfacePayload) => string;
}

const SERIES_SWATCH = ["#4f83cc", "#cc7a4f", "#6aa84f", "#9b59b6", "#c0392b"];

export function VolumeVisibility(props: VolumeVisibilityProps) {
  if (props.surfaces.length === 0) return null;
  // De-duplicate by key (contact mode can emit several payloads per logical volume).
  const seen = new Set<string>();
  const rows = props.surfaces.filter((s) => {
    const k = surfaceKey(s);
    if (seen.has(k)) return false;
    seen.add(k);
    return true;
  });

  return (
    <section className="volume-visibility" data-testid="volume-visibility" aria-label="volume visibility">
      <div className="volume-visibility-head">
        <strong>Volumes (debug)</strong>
        <button type="button" className="show-all" data-testid="show-all-volumes" onClick={props.onShowAll}>
          show all
        </button>
      </div>
      <ul className="volume-list">
        {rows.map((s) => {
          const key = surfaceKey(s);
          const visible = !props.hiddenKeys.has(key);
          const swatch =
            (s.lithologyId && props.catalog?.[s.lithologyId]?.color) ||
            SERIES_SWATCH[s.series % SERIES_SWATCH.length];
          return (
            <li key={key} className="volume-item" data-key={key}>
              <label>
                <input
                  type="checkbox"
                  data-testid="volume-toggle"
                  checked={visible}
                  onChange={(e) => props.onToggle(key, e.target.checked)}
                />
                <span className="volume-swatch" aria-hidden style={{ background: swatch }} />
                <span className="volume-label">{props.label(s)}</span>
              </label>
            </li>
          );
        })}
      </ul>
    </section>
  );
}
