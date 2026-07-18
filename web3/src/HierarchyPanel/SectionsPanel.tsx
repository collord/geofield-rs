// Feature 005 (cross-sections) — the SECTIONS side-panel + the serial-set creation form.
//
// Section-SET operations (create / open / step / delete / show-in-3D) are VIEW-state: the
// callbacks route to main.tsx's NON-dirty updater (the setRelation pattern) — they never
// re-fit (spec FR-009). The DIGITIZING controls (target picker, arm toggle, redraw-slice)
// drive GEOMETRY edits that commit + defer to Apply.
//
// Coverage bound (Principle IV): per-MEMBER ops on a serial set (delete one plane of a
// set) are out of scope — edit or delete the whole set.

import { useState } from "react";
import type { Project, SectionSet } from "./project";
import { parametricInterfaces } from "./projectOps";

/** The spacing/count/offset form shown after the two-click baseline gesture (mirrors the
 *  bounds ZRangeForm). count = 1 ⇒ a single section (spacing ignored). */
export function SectionSetForm(props: {
  baseline: { a: [number, number]; b: [number, number] };
  defaultSpacing: number;
  nextName: string;
  onConfirm: (name: string, spacing: number, count: number, offset: number) => void;
  onCancel: () => void;
}) {
  const [name, setName] = useState(props.nextName);
  const [spacing, setSpacing] = useState(String(Math.round(props.defaultSpacing)));
  const [count, setCount] = useState("1");
  const [offset, setOffset] = useState("0");
  const num = (s: string) => Number.parseFloat(s);
  const nCount = Number.parseInt(count, 10);
  const valid =
    Number.isInteger(nCount) && nCount >= 1 && Number.isFinite(num(offset)) &&
    (nCount === 1 || (Number.isFinite(num(spacing)) && num(spacing) > 0));
  const len = Math.hypot(props.baseline.b[0] - props.baseline.a[0], props.baseline.b[1] - props.baseline.a[1]);
  return (
    <div
      data-testid="section-set-form"
      style={{
        position: "absolute", top: 12, left: 12, zIndex: 10, background: "#fff",
        border: "1px solid #ccd", borderRadius: 6, padding: "0.75rem", boxShadow: "0 2px 8px rgba(0,0,0,0.15)",
        display: "grid", gap: "0.4rem", minWidth: 240,
      }}
    >
      <strong style={{ fontSize: "0.9em" }}>New cross-section(s)</strong>
      <div style={{ fontSize: "0.75em", color: "#556" }}>baseline length {len.toFixed(0)}</div>
      <label style={{ fontSize: "0.8em" }}>
        name{" "}
        <input data-testid="section-name" value={name} onChange={(e) => setName(e.target.value)} style={{ width: 130 }} />
      </label>
      <label style={{ fontSize: "0.8em" }}>
        count{" "}
        <input data-testid="section-serial-count" type="number" min={1} step={1} value={count} onChange={(e) => setCount(e.target.value)} style={{ width: 70 }} />
      </label>
      <label style={{ fontSize: "0.8em" }} title="offset between adjacent parallel planes (world units)">
        spacing{" "}
        <input data-testid="section-serial-spacing" type="number" value={spacing} onChange={(e) => setSpacing(e.target.value)} style={{ width: 90 }} disabled={nCount === 1} />
      </label>
      <label style={{ fontSize: "0.8em" }} title="start offset of the first plane from the drawn baseline">
        offset{" "}
        <input data-testid="section-serial-offset" type="number" value={offset} onChange={(e) => setOffset(e.target.value)} style={{ width: 90 }} />
      </label>
      <div style={{ display: "flex", gap: "0.4rem" }}>
        <button
          type="button"
          data-testid="section-create"
          disabled={!valid}
          onClick={() => props.onConfirm(name.trim() || props.nextName, nCount === 1 ? 0 : num(spacing), nCount, num(offset))}
        >
          Create
        </button>
        <button type="button" onClick={props.onCancel}>Cancel</button>
      </div>
    </div>
  );
}

export function SectionsPanel(props: {
  project: Project;
  activeSection: { setId: string; index: number } | null;
  onDrawBaseline: () => void; // arms the two-click gesture in the Viewer
  onOpen: (setId: string) => void;
  onClose: () => void;
  onDelete: (setId: string) => void;
  onToggleShow: (setId: string, on: boolean) => void;
  // --- digitizing (geometry; commits) ---
  digitizeTarget: { elementId: string; interfaceId: string } | null;
  onPickTarget: (t: { elementId: string; interfaceId: string } | "new" | null) => void;
  digitizeArmed: boolean;
  onArmDigitize: (on: boolean) => void;
  onRedigitizeSlice: () => void;
}) {
  const sections = props.project.sections ?? [];
  const targets = parametricInterfaces(props.project);
  const targetKey = props.digitizeTarget
    ? `${props.digitizeTarget.elementId}:${props.digitizeTarget.interfaceId}`
    : "";
  const inSection = props.activeSection !== null;
  return (
    <div className="sections-panel" style={{ marginTop: "0.75rem" }}>
      <div style={{ display: "flex", alignItems: "baseline", gap: "0.5rem" }}>
        <strong style={{ fontSize: "0.9em" }}>Cross-sections</strong>
        <button
          type="button"
          data-testid="section-new"
          title="Draw a section baseline: top-down view, click two points A→B; then set spacing × count for a serial set."
          onClick={props.onDrawBaseline}
        >
          New section…
        </button>
      </div>
      {sections.length === 0 && (
        <div style={{ fontSize: "0.75em", color: "#789" }}>none — draw a baseline in plan view</div>
      )}
      <ul style={{ listStyle: "none", margin: "0.25rem 0", padding: 0, display: "grid", gap: "0.2rem" }}>
        {sections.map((set: SectionSet) => {
          const isOpen = props.activeSection?.setId === set.id;
          return (
            <li key={set.id} style={{ display: "flex", gap: "0.35rem", alignItems: "center", fontSize: "0.8em" }}>
              <span style={{ flex: 1, fontWeight: isOpen ? 600 : 400 }}>
                {set.name}
                {set.count > 1 ? ` (${set.count} × ${set.spacing})` : ""}
              </span>
              <label className="debug-toggle" title="Show this set's planes in the 3D view">
                <input
                  type="checkbox"
                  data-testid={`section-show-${set.id}`}
                  checked={!!set.showInScene}
                  onChange={(e) => props.onToggleShow(set.id, e.target.checked)}
                />
                3D
              </label>
              {isOpen ? (
                <button type="button" data-testid={`section-close-${set.id}`} onClick={props.onClose}>close</button>
              ) : (
                <button type="button" data-testid={`section-open-${set.id}`} onClick={() => props.onOpen(set.id)}>open</button>
              )}
              <button type="button" data-testid={`section-delete-${set.id}`} onClick={() => props.onDelete(set.id)}>✕</button>
            </li>
          );
        })}
      </ul>

      {inSection && (
        <div className="digitize-controls" style={{ display: "grid", gap: "0.3rem", fontSize: "0.8em", borderTop: "1px dashed #ccd", paddingTop: "0.4rem" }}>
          <label>
            digitize into{" "}
            <select
              data-testid="digitize-target"
              value={targetKey}
              onChange={(e) => {
                const v = e.target.value;
                if (v === "") props.onPickTarget(null);
                else if (v === "__new__") props.onPickTarget("new");
                else {
                  const [elementId, interfaceId] = v.split(":");
                  props.onPickTarget({ elementId, interfaceId });
                }
              }}
            >
              <option value="">— pick a target surface —</option>
              <option value="__new__">+ new surface…</option>
              {targets.map((t) => (
                <option key={`${t.elementId}:${t.interfaceId}`} value={`${t.elementId}:${t.interfaceId}`}>
                  {t.label}
                </option>
              ))}
            </select>
          </label>
          <div style={{ display: "flex", gap: "0.4rem" }}>
            <button
              type="button"
              data-testid="digitize-toggle"
              disabled={!props.digitizeTarget}
              title="Click on the section to add ordered vertices; drag a vertex to move it; alt-click a vertex to delete it. The DRAWN DIRECTION sets the younger side (arrow + hachures)."
              onClick={() => props.onArmDigitize(!props.digitizeArmed)}
            >
              {props.digitizeArmed ? "■ stop digitizing" : "✏ digitize line"}
            </button>
            <button
              type="button"
              data-testid="redigitize-slice"
              disabled={!props.digitizeTarget}
              title="Delete the target surface's line(s) on THIS slice and re-arm the tool to redraw them"
              onClick={props.onRedigitizeSlice}
            >
              redraw this slice
            </button>
          </div>
        </div>
      )}
    </div>
  );
}
