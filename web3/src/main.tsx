// web3 — enhanced-authoring testbed (feature 002). A generic stack AUTHORING tool: an
// editable in-memory Project (containers + per-surface point/orientation tables) that
// compiles to SeriesJson[] (project.ts::toSeriesJson) and re-fits on an explicit Apply.
//
// State model (data-model.md): geometry-affecting edits set `dirty` and defer the fit to
// Apply (Cancel reverts to lastApplied); cheap declared-state edits (seam relation / floor)
// keep their immediate round-trip and never set dirty. A grey scrim signals the stale view
// while dirty. The two-wasm core, worker, and ABI are UNCHANGED — this is a TS/React layer.

import { useEffect, useMemo, useRef, useState } from "react";
import { createRoot } from "react-dom/client";
import { HierarchyPanel, LithologyCatalog, VolumeVisibility, applyCoreUpdate, emptyState, type HierarchyState } from "./HierarchyPanel";
import type { RelationDto } from "./HierarchyPanel/dto";
import "./HierarchyPanel/hierarchy.css";
import { Viewer, type ViewState } from "./Viewer";
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
  newId,
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
  setImportRole,
  setImportExtend,
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
import { initManifold, executeRecipe } from "./manifoldVolumes";

/** The minZ/maxZ form shown after the 2-point XY bounds digitization, to complete the rectangular
 *  prism (feature: draw model bounds). Prefilled from the current extent's Z range. */
function ZRangeForm(props: {
  extent: [number, number, number, number, number, number];
  rect: { x0: number; x1: number; y0: number; y1: number };
  onConfirm: (minZ: number, maxZ: number) => void;
  onCancel: () => void;
}) {
  const [minZ, setMinZ] = useState(String(props.extent[4]));
  const [maxZ, setMaxZ] = useState(String(props.extent[5]));
  const { x0, x1, y0, y1 } = props.rect;
  const num = (s: string) => Number.parseFloat(s);
  const valid = Number.isFinite(num(minZ)) && Number.isFinite(num(maxZ)) && num(minZ) !== num(maxZ);
  return (
    <div
      data-testid="z-range-form"
      style={{
        position: "absolute", top: 12, left: 12, zIndex: 10, background: "#fff",
        border: "1px solid #ccd", borderRadius: 6, padding: "0.75rem", boxShadow: "0 2px 8px rgba(0,0,0,0.15)",
        display: "grid", gap: "0.4rem", minWidth: 220,
      }}
    >
      <strong style={{ fontSize: "0.9em" }}>Model bounds — Z range</strong>
      <div style={{ fontSize: "0.75em", color: "#556" }}>
        X [{x0.toFixed(0)}, {x1.toFixed(0)}] · Y [{y0.toFixed(0)}, {y1.toFixed(0)}]
      </div>
      <label style={{ fontSize: "0.8em" }}>
        min Z{" "}
        <input data-testid="min-z" type="number" value={minZ} onChange={(e) => setMinZ(e.target.value)} style={{ width: 90 }} />
      </label>
      <label style={{ fontSize: "0.8em" }}>
        max Z{" "}
        <input data-testid="max-z" type="number" value={maxZ} onChange={(e) => setMaxZ(e.target.value)} style={{ width: 90 }} />
      </label>
      <div style={{ display: "flex", gap: "0.4rem" }}>
        <button type="button" data-testid="confirm-bounds" disabled={!valid} onClick={() => props.onConfirm(num(minZ), num(maxZ))}>
          Set bounds
        </button>
        <button type="button" onClick={props.onCancel}>Cancel</button>
      </div>
    </div>
  );
}

function App() {
  const [project, setProject] = useState<Project>(() => loadProject() ?? blankProject());
  const [dirty, setDirty] = useState(false);
  const [applying, setApplying] = useState(false);
  const [state, setState] = useState<HierarchyState>(emptyState());
  const [surfaces, setSurfaces] = useState<SurfacePayload[]>([]);
  const [renderMode, setRenderMode] = useState<RenderMode>("contact");
  // Debug view controls (visual diagnosis).
  const [wireframe, setWireframe] = useState(false);
  // The cross-cut seam closer. DEFAULT (on) = mesh–mesh CLIP corefine: shared seam by index, no
  // see-through dashes, no scalloping, no rim slivers. Off = the legacy snap, kept as an A/B escape
  // hatch. Toggling re-meshes (it changes geometry), so it re-runs the stack.
  const [seamClip, setSeamClip] = useState(true);
  // Cross-cut volume mesher (docs/13 §5, docs/14): the DEFAULT volume mesher is the Manifold
  // boolean engine (overhanging band + slabs, `(slab−band) ∩ domainBox` — wall shared by
  // construction, junction sealed, borders clean). Off = the legacy field-domain shared-tree path
  // (the wall∩bed peek-a-boo), kept as an A/B escape hatch. Solid mode + a cross-cut vein only;
  // toggling re-meshes. "clip seam" is inert while this is on (Manifold corefines the wall itself).
  const [manifold, setManifold] = useState(true);
  const [hiddenKeys, setHiddenKeys] = useState<Set<string>>(new Set());
  // Model-bounds authoring (feature: draw model bounds). `showBounds` toggles the wireframe box;
  // `drawNonce` bumps to (re)enter top-down 2-point digitization; `pendingBounds` holds the drawn
  // XY rectangle while the user completes the prism with minZ/maxZ.
  const [showBounds, setShowBounds] = useState(false);
  const [drawNonce, setDrawNonce] = useState(0);
  const [pendingBounds, setPendingBounds] = useState<{ x0: number; x1: number; y0: number; y1: number } | null>(null);
  const drawBounds = useMemo(() => (drawNonce ? { nonce: drawNonce } : undefined), [drawNonce]);
  // Camera round-trip: the Viewer fills `viewApiRef` with a getter (read on Export);
  // `importedView` (a new object per Import) is pushed back to re-apply a saved view.
  const viewApiRef = useRef<{ getView: () => ViewState } | null>(null);
  const [importedView, setImportedView] = useState<ViewState | undefined>(undefined);
  const [status, setStatus] = useState<string>("idle");
  const [addMenuOpen, setAddMenuOpen] = useState(false);
  // Which interface the data modal is editing: { elementId, interfaceId } | null.
  const [editing, setEditing] = useState<{ elementId: string; interfaceId: string } | null>(null);

  const lastApplied = useRef<Project>(project);
  const workerRef = useRef<Worker | null>(null);

  const validation = validate(project);

  // (Re)spawn the worker and post a run for a given compiled stack. Hard-terminate
  // supersedes any in-flight fit (unchanged from feature 001).
  function runStack(
    p: Project,
    onDone?: () => void,
    mode: RenderMode = renderMode,
    seamClipOverride?: boolean,
    manifoldOverride?: boolean,
  ) {
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
        const finish = (surfaces: SurfacePayload[]) => {
          setSurfaces(surfaces);
          setState(applyCoreUpdate(msg.update)); // derived = read-only cache (FR-018)
          setStatus("done");
          onDone?.();
        };
        // Unified solid mesher: the worker shipped a boolean RECIPE over single-level-set primitives;
        // compose the rock units on the PERSISTENT main-thread Manifold instance (it hangs if
        // re-instantiated per worker — docs/14). The geology is already baked into the recipe (Rust's
        // clip_maskers); the main thread just runs the booleans. On any failure, fall back to whatever
        // `surfaces` the worker sent (the legacy field path when the toggle is off, else empty).
        if (msg.recipeOps && msg.recipeMeta) {
          const { extent, assignment } = msg.recipeMeta;
          const ops = msg.recipeOps;
          initManifold()
            .then((wasm) => finish(executeRecipe(wasm, ops, extent, assignment)))
            .catch(() => finish(msg.surfaces));
        } else {
          finish(msg.surfaces);
        }
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
      // The cross-cut seam closer A/B (visible checkbox): snap (false) vs mesh–mesh clip (true).
      // An explicit override avoids the stale-closure race when toggled from the checkbox (React
      // state updates are async, so the freshly-set value is passed in directly).
      seamClip: seamClipOverride ?? seamClip,
      // The Manifold boolean A/B (solid mode + a cross-cut vein). Explicit override avoids the
      // stale-closure race when toggled from the checkbox (mirrors seamClip).
      manifold: manifoldOverride ?? manifold,
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
  const onSetImportRole = (elementId: string, role: "contact" | "erosion" | "veinWall" | "topography") =>
    commit(setImportRole(project, elementId, role));
  const onSetImportExtend = (elementId: string, extend: boolean) =>
    commit(setImportExtend(project, elementId, extend));

  // Draw model bounds (feature): the Viewer reports the drawn XY rectangle; show the wireframe and
  // open the minZ/maxZ form to complete the prism.
  const onBoundsXY = (x0: number, x1: number, y0: number, y1: number) => {
    setPendingBounds({ x0, x1, y0, y1 });
    setShowBounds(true);
  };
  // Confirm the prism: commit `project.extent` and re-mesh (extent bounds the mesher's domain box).
  const onConfirmBounds = (minZ: number, maxZ: number) => {
    if (!pendingBounds) return;
    const { x0, x1, y0, y1 } = pendingBounds;
    const lo = Math.min(minZ, maxZ);
    const hi = Math.max(minZ, maxZ);
    const next: Project = { ...project, extent: [x0, x1, y0, y1, lo, hi] };
    setProject(next);
    saveProject(next);
    setPendingBounds(null);
    if (next.elements.length > 0 && validate(next).canApply) runStack(next);
  };
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
      __getView?: () => ViewState | undefined;
      __exactSpikeDigest?: () => Promise<string | undefined>;
    };
    w.__reorderContainers = onReorderContainers;
    w.__moveSurface = onMoveSurface;
    // Test hooks (harmless in the demo): inject a Project directly (skips the file picker)
    // and expose the rendered surfaces for geometric assertions. Used by the flipped-erosion
    // QEF-spike regression spec (e2e/qef-spike.spec.ts). `__getView` reads the current camera
    // (so a viewpoint can be grabbed/asserted, and shared for repro).
    w.__loadProject = loadFreshProject;
    w.__surfaces = surfaces;
    w.__getView = () => viewApiRef.current?.getView();
    // Stage 0 exact-corefiner determinism gate (e2e/exact-determinism.spec.ts, docs/12 §2.3). Runs
    // the exact tri-tri spike in the browser's wasm and returns its digest hex. `spike_digest` is
    // pure compute (no field/kernel), so we init the engine module standalone. The export only
    // exists when the engine is built `--features exact_arr` (see `npm run wasm:engine:exact`);
    // returns undefined otherwise, and the spec skips. Default bundle is unchanged.
    w.__exactSpikeDigest = async () => {
      const mod = (await import("./wasm/geofield.js")) as unknown as {
        default: () => Promise<unknown>;
        exact_spike_digest_hex?: () => string;
      };
      await mod.default();
      return mod.exact_spike_digest_hex?.();
    };
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
    // Restore a saved view (camera + render toggles) if present, so an imported scenario
    // reproduces "what I was looking at" — Solids vs Surfaces, clip seam, edges, and angle.
    // Pass mode/clip EXPLICITLY to runStack (state setters are async, would be stale).
    const mode = p.view?.renderMode ?? renderMode;
    const clip = p.view?.seamClip ?? seamClip;
    if (p.view) {
      setRenderMode(mode);
      setSeamClip(clip);
      if (p.view.edges !== undefined) setWireframe(p.view.edges);
      setImportedView({ camera: p.view.camera, target: p.view.target, up: p.view.up, fov: p.view.fov });
    }
    if (p.elements.length > 0 && validate(p).canApply) runStack(p, undefined, mode, clip);
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
  const onExport = () => {
    const cam = viewApiRef.current?.getView();
    const view = cam && { ...cam, renderMode, seamClip, edges: wireframe };
    exportProjectJson(view ? { ...project, view } : project);
  };
  const onImportFile = async (file: File) => {
    confirmIfDirty(async () => {
      try {
        loadFreshProject(await importProjectJson(file)); // restores the view (camera + toggles)
      } catch (err) {
        setStatus("import failed: " + (err instanceof Error ? err.message : String(err)));
      }
    });
  };
  const onRenameProject = (name: string) => commit({ ...project, name });

  // --- Import a MESH surface (feature 004): read a `.obj` file's text, add an importedSurface
  //     element (a Contact by default), and commit (a geometry edit → re-fits on Apply). Distinct
  //     from the project-JSON "Import" above (FR-013). ---
  const onImportMesh = async (file: File) => {
    try {
      const objText = await file.text();
      const el = {
        kind: "importedSurface" as const,
        id: newId("el"),
        name: file.name.replace(/\.obj$/i, ""),
        role: "contact" as const,
        relation: "conformable" as const,
        objText,
        extendToBounds: false, // footprint-only by default (no extrapolation)
      };
      commit({ ...project, elements: [el, ...project.elements] });
    } catch (err) {
      setStatus("mesh import failed: " + (err instanceof Error ? err.message : String(err)));
    }
  };

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
            <label className="import-btn" data-testid="import-mesh-btn">
              Import mesh
              <input
                type="file"
                accept=".obj"
                data-testid="import-mesh-input"
                style={{ display: "none" }}
                onChange={(e) => {
                  const f = e.target.files?.[0];
                  if (f) void onImportMesh(f);
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
            <button
              type="button"
              data-testid="draw-bounds"
              title="Draw the model bounds: top-down view, click two corners of the rectangle, then set the Z range."
              onClick={() => setDrawNonce((n) => n + 1)}
            >
              Draw bounds
            </button>
            <label className="debug-toggle" title="Show the model-bounds wireframe box">
              <input
                type="checkbox"
                data-testid="bounds-toggle"
                checked={showBounds}
                onChange={(e) => setShowBounds(e.target.checked)}
              />
              bounds
            </label>
            <label
              className="debug-toggle"
              title="Cross-cut seam closer: off = legacy snap, on = mesh–mesh clip corefine (shared seam, no see-through dashes). Re-meshes on toggle."
            >
              <input
                type="checkbox"
                data-testid="seam-clip-toggle"
                checked={seamClip}
                onChange={(e) => {
                  const on = e.target.checked;
                  setSeamClip(on);
                  // Re-mesh immediately so the change is visible (seam closer changes geometry).
                  // Pass `on` explicitly — the state set above hasn't applied yet this tick.
                  if (project.elements.length > 0 && validate(project).canApply) {
                    runStack(project, undefined, renderMode, on);
                  }
                }}
              />
              clip seam
            </label>
            <label
              className="debug-toggle"
              title="Cross-cut volume mesher (Solids only, DEFAULT on): on = the Manifold boolean engine (wall shared by construction, junction sealed, borders clean), off = the legacy field-domain shared tree (the wall∩bed peek-a-boo). Re-meshes on toggle."
            >
              <input
                type="checkbox"
                data-testid="manifold-toggle"
                checked={manifold}
                onChange={(e) => {
                  const on = e.target.checked;
                  setManifold(on);
                  // Re-mesh immediately (the boolean engine changes geometry). Pass `on`
                  // explicitly — the state set above hasn't applied yet this tick.
                  if (project.elements.length > 0 && validate(project).canApply) {
                    runStack(project, undefined, renderMode, undefined, on);
                  }
                }}
              />
              manifold
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
          onSetImportRole={onSetImportRole}
          onSetImportExtend={onSetImportExtend}
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
          viewApiRef={viewApiRef}
          importedView={importedView}
          showBounds={showBounds}
          drawBounds={drawBounds}
          onBoundsXY={onBoundsXY}
        />
        {pendingBounds && (
          <ZRangeForm
            extent={project.extent}
            rect={pendingBounds}
            onConfirm={onConfirmBounds}
            onCancel={() => setPendingBounds(null)}
          />
        )}
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
