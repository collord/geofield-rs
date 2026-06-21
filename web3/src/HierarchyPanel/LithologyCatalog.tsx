// Feature 003 (topography, lithology, solids) T033 [US3] — the lithology catalog panel.
//
// Lists the catalog: Air + Unknown (fixed), inferred (from vein/intrusion bodies, editable
// colour only), and user entries (full CRUD). Adding a user entry makes it available in every
// unit row's lithology dropdown. This is UI-side identity metadata — it never crosses the
// wasm boundary (Constitution III). (Catalog model: project.ts; ops: projectOps.ts.)

import { useState } from "react";
import type { Lithology } from "./project";

export interface LithologyCatalogProps {
  lithologies: Lithology[];
  onAdd: (name: string, color: string) => void;
  onUpdate: (id: string, patch: { name?: string; color?: string }) => void;
  onDelete: (id: string) => void;
}

const DEFAULT_NEW_COLOR = "#7aa64f";

export function LithologyCatalog(props: LithologyCatalogProps) {
  const [newName, setNewName] = useState("");
  const [newColor, setNewColor] = useState(DEFAULT_NEW_COLOR);

  function add() {
    const name = newName.trim();
    if (!name) return;
    props.onAdd(name, newColor);
    setNewName("");
    setNewColor(DEFAULT_NEW_COLOR);
  }

  return (
    <section className="litho-catalog" data-testid="litho-catalog" aria-label="lithology catalog">
      <div className="litho-catalog-head">
        <strong>Lithologies</strong>
      </div>
      <ul className="litho-list">
        {props.lithologies.map((l) => {
          const fixed = l.kind === "air" || l.kind === "unknown";
          const nameEditable = l.kind === "user"; // inferred names sync from the body
          const deletable = l.kind === "user";
          return (
            <li key={l.id} className="litho-item" data-testid="litho-item" data-kind={l.kind}>
              <input
                type="color"
                className="litho-swatch"
                aria-label={`${l.name} colour`}
                value={l.kind === "air" ? "#ffffff" : l.color}
                disabled={fixed}
                onChange={(e) => props.onUpdate(l.id, { color: e.target.value })}
              />
              {nameEditable ? (
                <input
                  className="litho-name"
                  data-testid="litho-name"
                  aria-label={`${l.name} name`}
                  value={l.name}
                  onChange={(e) => props.onUpdate(l.id, { name: e.target.value })}
                />
              ) : (
                <span className="litho-name fixed" title={fixed ? "fixed member" : "inferred from a body"}>
                  {l.name}
                  {l.kind === "inferred" && <span className="litho-tag"> (inferred)</span>}
                </span>
              )}
              {deletable && (
                <button
                  type="button"
                  className="litho-delete danger"
                  aria-label={`delete ${l.name}`}
                  title="delete"
                  onClick={() => props.onDelete(l.id)}
                >
                  ✕
                </button>
              )}
            </li>
          );
        })}
      </ul>
      <div className="litho-add">
        <input
          type="color"
          className="litho-swatch"
          aria-label="new lithology colour"
          value={newColor}
          onChange={(e) => setNewColor(e.target.value)}
        />
        <input
          className="litho-name"
          data-testid="litho-new-name"
          placeholder="add lithology…"
          aria-label="new lithology name"
          value={newName}
          onChange={(e) => setNewName(e.target.value)}
          onKeyDown={(e) => e.key === "Enter" && add()}
        />
        <button type="button" data-testid="litho-add" onClick={add} disabled={!newName.trim()}>
          + Add
        </button>
      </div>
    </section>
  );
}
