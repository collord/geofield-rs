// Feature 002 (enhanced-authoring) T016 [US1] — the five-type picker popover.
//
// The user picks a SURFACE TYPE; container membership is then DERIVED (project.ts/projectOps
// ::addSurface, research.md R4). The user never says "container". One-line glosses explain
// each type's effect in plain language.

import type { SurfaceType } from "./project";

const CHOICES: { type: SurfaceType; label: string; gloss: string }[] = [
  { type: "conformable", label: "Conformable", gloss: "stacks conformably; shares a field with its neighbours" },
  { type: "erosional", label: "Erosional", gloss: "truncates older surfaces below it" },
  { type: "onlap", label: "Onlap", gloss: "laps onto and pinches out against older topography" },
  { type: "vein", label: "Vein", gloss: "tabular body that cross-cuts what it passes through" },
  { type: "intrusion", label: "Intrusion", gloss: "volumetric body that cross-cuts; fills its interior" },
];

export function AddSurfaceMenu(props: { onPick: (type: SurfaceType) => void; onClose: () => void }) {
  return (
    <div className="add-surface-menu" data-testid="add-surface-menu" role="menu">
      <div className="add-surface-menu-head">
        <strong>Add surface</strong>
        <button type="button" className="popover-close" aria-label="close" onClick={props.onClose}>
          ✕
        </button>
      </div>
      {CHOICES.map((c) => (
        <button
          key={c.type}
          type="button"
          className="add-surface-choice"
          data-type={c.type}
          role="menuitem"
          onClick={() => props.onPick(c.type)}
        >
          <span className="choice-label">{c.label}</span>
          <span className="choice-gloss">{c.gloss}</span>
        </button>
      ))}
    </div>
  );
}
