// Feature 002 (enhanced-authoring) T022 [US2] — the per-surface and per-series "…" menus.
//
// SurfaceMenu: Edit data… / Rename… / Duplicate / Delete for one surface.
// SeriesMenu: Rename… / Delete series / + interface, on a multi-interface series header.
// Both use prompt() for rename (the testbed is a demo; a real product would use an inline
// editor) and window.confirm for delete.

import { useState } from "react";
import type { ProjectElement } from "./project";

export function SurfaceMenu(props: {
  onEdit: () => void;
  onRename: (name: string) => void;
  onFlip: () => void;
  flipped?: boolean;
  onDuplicate?: () => void;
  onDelete: () => void;
}) {
  const [open, setOpen] = useState(false);
  const close = () => setOpen(false);
  return (
    <span className="row-menu">
      <button
        type="button"
        className="row-menu-trigger"
        aria-label="surface actions"
        data-testid="surface-menu"
        onClick={() => setOpen((o) => !o)}
      >
        …
      </button>
      {open && (
        <div className="row-menu-popover" role="menu">
          <button
            type="button"
            role="menuitem"
            data-testid="surface-edit"
            onClick={() => {
              close();
              props.onEdit();
            }}
          >
            Edit data…
          </button>
          <button
            type="button"
            role="menuitem"
            onClick={() => {
              close();
              const name = window.prompt("Rename surface");
              if (name) props.onRename(name);
            }}
          >
            Rename…
          </button>
          <button
            type="button"
            role="menuitem"
            data-testid="surface-flip"
            onClick={() => {
              close();
              props.onFlip();
            }}
          >
            Flip orientation{props.flipped ? " ✓" : ""}
          </button>
          {props.onDuplicate && (
            <button
              type="button"
              role="menuitem"
              onClick={() => {
                close();
                props.onDuplicate!();
              }}
            >
              Duplicate
            </button>
          )}
          <button
            type="button"
            role="menuitem"
            className="danger"
            onClick={() => {
              close();
              if (window.confirm("Delete this surface?")) props.onDelete();
            }}
          >
            Delete
          </button>
        </div>
      )}
    </span>
  );
}

export function SeriesMenu(props: {
  el: ProjectElement;
  onRename: (name: string) => void;
  onDelete: () => void;
  onAddInterface: () => void;
}) {
  const [open, setOpen] = useState(false);
  const close = () => setOpen(false);
  const isSeries = props.el.kind === "series";
  return (
    <span className="row-menu">
      <button
        type="button"
        className="row-menu-trigger"
        aria-label="series actions"
        data-testid="series-menu"
        onClick={() => setOpen((o) => !o)}
      >
        …
      </button>
      {open && (
        <div className="row-menu-popover" role="menu">
          <button
            type="button"
            role="menuitem"
            onClick={() => {
              close();
              const name = window.prompt("Rename", props.el.name);
              if (name) props.onRename(name);
            }}
          >
            Rename…
          </button>
          {isSeries && (
            <button
              type="button"
              role="menuitem"
              data-testid="series-add-interface"
              onClick={() => {
                close();
                props.onAddInterface();
              }}
            >
              + interface
            </button>
          )}
          <button
            type="button"
            role="menuitem"
            className="danger"
            onClick={() => {
              close();
              if (window.confirm(`Delete "${props.el.name}"?`)) props.onDelete();
            }}
          >
            Delete
          </button>
        </div>
      )}
    </span>
  );
}
