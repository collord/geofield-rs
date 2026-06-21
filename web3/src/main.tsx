// web3 — enhanced-authoring testbed (feature 002). A generic stack AUTHORING tool: an
// editable in-memory Project (containers + per-surface point/orientation tables) that
// compiles to SeriesJson[] (project.ts::toSeriesJson) and re-fits on an explicit Apply.
//
// State model (data-model.md): geometry-affecting edits set `dirty` and defer the fit to
// Apply (Cancel reverts to lastApplied); cheap declared-state edits (seam relation / floor)
// keep their immediate round-trip and never set dirty. A grey scrim signals the stale view
// while dirty. The two-wasm core, worker, and ABI are UNCHANGED — this is a TS/React layer.

import { useEffect, useRef, useState } from "react";
import { createRoot } from "react-dom/client";
import { HierarchyPanel, LithologyCatalog, VolumeVisibility, applyCoreUpdate, emptyState, type HierarchyState } from "./HierarchyPanel";
import type { RelationDto } from "./HierarchyPanel/dto";
import "./HierarchyPanel/hierarchy.css";
import { Viewer } from "./Viewer";
import { exportObj } from "./exportObj";
import { TEMPLATES, templateById, blankProject } from "./scenarios";
import {
  type Project,
  type SurfaceType,
  loadProject,
  saveProject,
  toSeriesJson,
  validate,
  exportProjectJson,
  importProjectJson,
} from "./HierarchyPanel/project";
import {
  addSurface,
  applyInterfaceData,
  renameInterface,
  flipOrientation,
  duplicateInterface,
  deleteInterface,
  addInterface,
  renameElement,
  deleteElement,
  reorderContainers,
  reorderWithin,
  findInterface,
  hasTopography,
  unitNameOf,
  renameUnit,
  catalogOf,
  lithologyOf,
  assignLithology,
  addLithology,
  updateLithology,
  deleteLithology,
  syncInferredLithologies,
  indexedAssignment,
} from "./HierarchyPanel/projectOps";
import { AddSurfaceMenu } from "./HierarchyPanel/AddSurfaceMenu";
import { SurfaceDataModal } from "./HierarchyPanel/SurfaceDataModal";
import type { RenderMode, RunCommand, SurfacePayload, WorkerMessage } from "./protocol";

function App() {
  const [project, setProject] = useState<Project>(() => loadProject() ?? blankProject());
  const [dirty, setDirty] = useState(false);
  const [applying, setApplying] = useState(false);
  const [state, setState] = useState<HierarchyState>(emptyState());
  const [surfaces, setSurfaces] = useState<SurfacePayload[]>([]);
  const [renderMode, setRenderMode] = useState<RenderMode>("contact");
  // Debug view controls (visual diagnosis).
  const [wireframe, setWireframe] = useState(false);
  const [hiddenKeys, setHiddenKeys] = useState<Set<string>>(new Set());
  const [status, setStatus] = useState<string>("idle");
  const [addMenuOpen, setAddMenuOpen] = useState(false);
  // Which interface the data modal is editing: { elementId, interfaceId } | null.
  const [editing, setEditing] = useState<{ elementId: string; interfaceId: string } | null>(null);

  const lastApplied = useRef<Project>(project);
  const workerRef = useRef<Worker | null>(null);

  const validation = validate(project);

  // (Re)spawn the worker and post a run for a given compiled stack. Hard-terminate
  // supersedes any in-flight fit (unchanged from feature 001).
  function runStack(p: Project, onDone?: () => void, mode: RenderMode = renderMode) {
    workerRef.current?.terminate();
    const worker = new Worker(new URL("./worker.ts", import.meta.url), { type: "module" });
    workerRef.current = worker;
    worker.onmessage = (e: MessageEvent<WorkerMessage>) => {
      const msg = e.data;
      if (msg.type === "status") {
        const w = window as unknown as { __statusLog?: string[] };
        (w.__statusLog ??= []).push(msg.phase);
        setStatus(msg.phase);
      }
      else if (msg.type === "error") setStatus("error: " + msg.message);
      else if (msg.type === "done") {
        setSurfaces(msg.surfaces);
        setState(applyCoreUpdate(msg.update)); // derived = read-only cache (FR-018)
        setStatus("done");
        onDone?.();
      }
    };
    const cmd: RunCommand = {
      type: "run",
      series: toSeriesJson(p),
      extent: p.extent,
      maxDepth: 6,
      renderMode: mode,
      // Solid-mode lithology tags, keyed by (seriesIndex, unitIndex). The compile seam emits
      // topography youngest, so series index == position in toSeriesJson output.
      assignment: mode === "solid" ? indexedAssignment(p) : undefined,
      // T-junction weld is on by default; `window.__WELD_T_JUNCTIONS = false` (then re-Apply) A/Bs
      // the old independent-mesh behavior — a dev escape hatch, not a user-facing control.
      weldTJunctions: (window as { __WELD_T_JUNCTIONS?: boolean }).__WELD_T_JUNCTIONS ?? true,
    };
    worker.postMessage(cmd);
  }

  // The Surfaces⇄Solids toggle. NOTE (Principle IV coverage bound): the worker is
  // terminate-and-refit per run (the existing hard-cancel design), so toggling re-FITS rather
  // than re-meshing the held stack. The spec prefers re-mesh-without-refit; keeping a fitted
  // WasmStack alive across messages is a larger worker change deferred here. The fit is
  // deterministic so the result is identical — only the (one-time) cost differs.
  function setRenderModeAndRerun(mode: RenderMode) {
    setRenderMode(mode);
    if (project.elements.length > 0 && validate(project).canApply) runStack(project, undefined, mode);
  }

  // Initial fit (if the loaded/seeded project has anything to fit).
  useEffect(() => {
    if (project.elements.length > 0 && validate(project).canApply) {
      runStack(project, () => {
        lastApplied.current = project;
      });
    }
    return () => workerRef.current?.terminate();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // --- committed-edit helper: mutate the project, mark dirty, autosave ------------------
  function commit(next: Project) {
    // Reconcile inferred lithologies with the current bodies on every edit (idempotent):
    // a vein/intrusion add/rename/delete syncs its inferred catalog entry (feature 003 §4.3).
    const reconciled = syncInferredLithologies(next);
    setProject(reconciled);
    setDirty(true);
    saveProject(reconciled);
  }

  // A lithology/metadata edit is NOT a geometry change — it never needs a re-fit and must not
  // gate behind Apply (no `dirty`). It saves immediately; in SOLID mode it re-meshes so the
  // new colour/assignment shows (the tags are applied at mesh-collect time). (feature 003 §6.)
  function commitLithology(next: Project) {
    setProject(next);
    saveProject(next);
    if (renderMode === "solid" && next.elements.length > 0 && validate(next).canApply) {
      runStack(next, undefined, "solid");
    }
  }

  // --- panel-level Apply / Cancel (the re-fit gate) -------------------------------------
  function onApply() {
    if (!validation.canApply) return;
    setApplying(true);
    const snapshot = project;
    runStack(snapshot, () => {
      lastApplied.current = snapshot;
      setDirty(false);
      setApplying(false);
    });
  }
  function onCancel() {
    setProject(lastApplied.current);
    setDirty(false);
  }

  // --- cheap declared-state edits: immediate re-fit, dirty stays false ------------------
  function setRelation(elementId: string, rel: RelationDto) {
    const updated: Project = {
      ...project,
      elements: project.elements.map((el) =>
        el.id === elementId && el.kind === "series" ? { ...el, relation: rel } : el,
      ),
    };
    setProject(updated);
    saveProject(updated);
    runStack(updated); // immediate — never stale (dirty stays false)
  }

  // --- Add surface (US1) ----------------------------------------------------------------
  function onPickType(type: SurfaceType) {
    setAddMenuOpen(false);
    const { project: next, openInterface } = addSurface(project, type);
    commit(next);
    // Open the data modal for the new surface (unless the add was a no-op, e.g. a guarded
    // second topography returns an empty target).
    if (openInterface.interfaceId) setEditing(openInterface);
  }

  // --- surface/series menu actions (US2) ------------------------------------------------
  const onEditSurface = (elementId: string, interfaceId: string) => setEditing({ elementId, interfaceId });
  const onRenameSurface = (elementId: string, interfaceId: string, name: string) =>
    commit(renameInterface(project, elementId, interfaceId, name));
  const onFlipSurface = (elementId: string, interfaceId: string) =>
    commit(flipOrientation(project, elementId, interfaceId));
  const onDuplicateSurface = (elementId: string, interfaceId: string) =>
    commit(duplicateInterface(project, elementId, interfaceId));
  const onDeleteSurface = (elementId: string, interfaceId: string) =>
    commit(deleteInterface(project, elementId, interfaceId));
  const onAddInterface = (elementId: string) => commit(addInterface(project, elementId));
  const onRenameElement = (elementId: string, name: string) => commit(renameElement(project, elementId, name));
  const onDeleteElement = (elementId: string) => commit(deleteElement(project, elementId));

  // --- reorder (US4) — wire the existing resolver output --------------------------------
  const onReorderContainers = (from: string, to: string) => commit(reorderContainers(project, from, to));
  const onReorderWithin = (elementId: string, from: number, to: number) =>
    commit(reorderWithin(project, elementId, from, to));
  // Move a top-level element one slot earlier (dir -1, toward youngest/top) or later
  // (dir +1, toward oldest/bottom) in the chronology — the keyboard/click affordance for
  // reorder (the drag affordance is unbuilt; see HierarchyPanel). Swaps with its neighbour
  // via the SAME reorderContainers resolver. No-op at the ends.
  const onMoveElement = (elementId: string, dir: -1 | 1) => {
    const i = project.elements.findIndex((e) => e.id === elementId);
    const j = i + dir;
    if (i < 0 || j < 0 || j >= project.elements.length) return;
    commit(reorderContainers(project, elementId, project.elements[j].id));
  };
  const onMoveSurface = () =>
    setStatus("moving a surface between containers isn't supported — delete and re-add");

  // Test hook: real @dnd-kit drag is flaky to simulate (see drag-affordance.spec). The
  // reorder.spec drives the SAME wired callback through this hook, proving the round-trip
  // without a brittle synthetic drag. Harmless in the demo; never used by the UI itself.
  useEffect(() => {
    const w = window as unknown as {
      __reorderContainers?: (f: string, t: string) => void;
      __moveSurface?: () => void;
      __loadProject?: (p: Project) => void;
      __surfaces?: SurfacePayload[];
    };
    w.__reorderContainers = onReorderContainers;
    w.__moveSurface = onMoveSurface;
    // Test hooks (harmless in the demo): inject a Project directly (skips the file picker)
    // and expose the rendered surfaces for geometric assertions. Used by the flipped-erosion
    // QEF-spike regression spec (e2e/qef-spike.spec.ts).
    w.__loadProject = loadFreshProject;
    w.__surfaces = surfaces;
  });

  // --- project bar actions (US5) --------------------------------------------------------
  function confirmIfDirty(action: () => void) {
    if (dirty && !window.confirm("Discard unapplied changes?")) return;
    action();
  }
  function loadFreshProject(raw: Project) {
    // Reconcile inferred lithologies with the loaded bodies (seed/import path): a vein/
    // intrusion contributes its inferred catalog entry even on first load (feature 003 §4.3).
    const p = syncInferredLithologies(raw);
    setProject(p);
    setDirty(false);
    saveProject(p);
    lastApplied.current = p;
    if (p.elements.length > 0 && validate(p).canApply) runStack(p);
    else {
      setSurfaces([]);
      setState(emptyState());
      setStatus("idle");
    }
  }
  const onNew = () => confirmIfDirty(() => loadFreshProject(blankProject()));
  const onSeed = (templateId: string) =>
    confirmIfDirty(() => {
      const t = templateById(templateId);
      if (t) loadFreshProject(t.build());
    });
  const onExport = () => exportProjectJson(project);
  const onImportFile = async (file: File) => {
    confirmIfDirty(async () => {
      try {
        loadFreshProject(await importProjectJson(file));
      } catch (err) {
        setStatus("import failed: " + (err instanceof Error ? err.message : String(err)));
      }
    });
  };
  const onRenameProject = (name: string) => commit({ ...project, name });

  // --- OBJ export (US6) -----------------------------------------------------------------
  const canExportObj = surfaces.length > 0 && !dirty;
  const onExportObj = () => {
    if (!canExportObj) return;
    // In solid mode, group by lithology (Air is never in `surfaces`, so no Air group).
    const lithoNames =
      renderMode === "solid"
        ? Object.fromEntries(catalogOf(project).lithologies.map((l) => [l.id, l.name]))
        : undefined;
    const obj = exportObj(surfaces, lithoNames);
    const blob = new Blob([obj], { type: "text/plain" });
    const url = URL.createObjectURL(blob);
    const a = document.createElement("a");
    a.href = url;
    a.download = `${project.name}.obj`;
    document.body.appendChild(a);
    a.click();
    a.remove();
    URL.revokeObjectURL(url);
  };

  const editingIface = editing ? findInterface(project, editing.elementId, editing.interfaceId) : null;
  const firstHardError = validation.hardErrors[0]?.reason;

  return (
    <div style={{ display: "grid", gridTemplateColumns: "380px 1fr", height: "100vh" }}>
      <aside style={{ padding: "1rem", overflow: "auto", borderRight: "1px solid #ccd" }}>
        <h2 style={{ margin: "0 0 0.5rem" }}>Stack authoring</h2>

        {/* Project bar (US5) */}
        <div className="project-bar">
          <input
            className="project-name"
            value={project.name}
            onChange={(e) => onRenameProject(e.target.value)}
            aria-label="project name"
          />
          <div className="project-bar-actions">
            <button type="button" onClick={onNew}>New</button>
            <label className="import-btn">
              Import
              <input
                type="file"
                accept=".json,.geoproj.json,application/json"
                style={{ display: "none" }}
                onChange={(e) => {
                  const f = e.target.files?.[0];
                  if (f) void onImportFile(f);
                  e.target.value = "";
                }}
              />
            </label>
            <button type="button" onClick={onExport}>Export</button>
            <select
              data-testid="seed-template"
              defaultValue=""
              onChange={(e) => {
                if (e.target.value) onSeed(e.target.value);
                e.target.value = "";
              }}
            >
              <option value="" disabled>Seed from template ▾</option>
              {TEMPLATES.map((t) => (
                <option key={t.id} value={t.id}>{t.name}</option>
              ))}
            </select>
            <button type="button" onClick={onExportObj} disabled={!canExportObj} data-testid="export-obj">
              Export OBJ
            </button>
            <button
              type="button"
              data-testid="render-toggle"
              title="Toggle between open contact surfaces and closed lithology solids"
              onClick={() => setRenderModeAndRerun(renderMode === "contact" ? "solid" : "contact")}
            >
              {renderMode === "contact" ? "Surfaces ⇄ Solids" : "Solids ⇄ Surfaces"}
            </button>
            <label className="debug-toggle" title="Show triangle edges (mesh wireframe)">
              <input
                type="checkbox"
                data-testid="wireframe-toggle"
                checked={wireframe}
                onChange={(e) => setWireframe(e.target.checked)}
              />
              edges
            </label>
          </div>
        </div>

        {/* Apply / Cancel bar (shown only when dirty) */}
        {dirty && (
          <div className="apply-bar" data-testid="apply-bar">
            <span className="dirty-dot" /> unapplied changes
            <button
              type="button"
              className="apply-btn"
              data-testid="apply"
              disabled={!validation.canApply || applying}
              title={validation.canApply ? "re-fit the stack" : firstHardError}
              onClick={onApply}
            >
              {applying ? "Applying…" : "Apply"}
            </button>
            <button type="button" data-testid="cancel" onClick={onCancel}>Cancel</button>
            {!validation.canApply && firstHardError && (
              <div className="apply-reason" data-testid="apply-reason">{firstHardError}</div>
            )}
          </div>
        )}

        <div style={{ fontSize: "0.75em", color: "#789" }}>status: {status}</div>
        <hr />

        <HierarchyPanel
          project={project}
          update={{ model: state.model, derived: state.derived }}
          validation={validation}
          onReorderContainers={onReorderContainers}
          onReorderWithin={onReorderWithin}
          onMoveElement={onMoveElement}
          onMoveSurface={onMoveSurface}
          onSetRelation={setRelation}
          onAddSurfaceClick={() => setAddMenuOpen(true)}
          onEditSurface={onEditSurface}
          onRenameSurface={onRenameSurface}
          onFlipSurface={onFlipSurface}
          onDuplicateSurface={onDuplicateSurface}
          onDeleteSurface={onDeleteSurface}
          onAddInterface={onAddInterface}
          onRenameElement={onRenameElement}
          onDeleteElement={onDeleteElement}
          unitName={(elementId, unitIndex, displayPos) => unitNameOf(project, elementId, unitIndex, displayPos)}
          onRenameUnit={(elementId, unitIndex, name) => commitLithology(renameUnit(project, elementId, unitIndex, name))}
          catalog={catalogOf(project).lithologies}
          lithologyOf={(elementId, unitIndex) => lithologyOf(project, elementId, unitIndex)}
          onAssignLithology={(elementId, unitIndex, id) => commitLithology(assignLithology(project, elementId, unitIndex, id))}
        />

        {addMenuOpen && (
          <AddSurfaceMenu
            onPick={onPickType}
            onClose={() => setAddMenuOpen(false)}
            hasTopography={hasTopography(project)}
          />
        )}

        <LithologyCatalog
          lithologies={catalogOf(project).lithologies}
          onAdd={(name, color) => commitLithology(addLithology(project, name, color))}
          onUpdate={(id, patch) => commitLithology(updateLithology(project, id, patch))}
          onDelete={(id) => commitLithology(deleteLithology(project, id))}
        />

        <VolumeVisibility
          surfaces={surfaces}
          hiddenKeys={hiddenKeys}
          onToggle={(key, visible) =>
            setHiddenKeys((prev) => {
              const next = new Set(prev);
              if (visible) next.delete(key);
              else next.add(key);
              return next;
            })
          }
          onShowAll={() => setHiddenKeys(new Set())}
          catalog={Object.fromEntries(catalogOf(project).lithologies.map((l) => [l.id, l]))}
          label={(s) =>
            renderMode === "solid"
              ? `${s.series}·${catalogOf(project).lithologies.find((l) => l.id === s.lithologyId)?.name ?? "Unknown"}` +
                (s.unit !== undefined ? ` (unit ${s.unit})` : "")
              : `series ${s.series} · iface ${s.interface}${s.block ? ` · blk ${s.block}` : ""}`
          }
        />
      </aside>

      <main>
        <Viewer
          surfaces={surfaces}
          extent={project.extent}
          stale={dirty}
          colorBy={renderMode === "solid" ? "lithology" : "series"}
          lithoColors={Object.fromEntries(catalogOf(project).lithologies.map((l) => [l.id, l.color]))}
          wireframe={wireframe}
          hiddenKeys={hiddenKeys}
        />
      </main>

      {editing && editingIface && (
        <SurfaceDataModal
          surfaceName={editingIface.name}
          parentName={editingIface.parentName}
          points={editingIface.points}
          orientations={editingIface.orientations}
          onApply={(points, orientations) => {
            commit(applyInterfaceData(project, editing.elementId, editing.interfaceId, points, orientations));
            setEditing(null);
          }}
          onCancel={() => setEditing(null)}
        />
      )}
    </div>
  );
}

const root = document.getElementById("root");
if (root) createRoot(root).render(<App />);
