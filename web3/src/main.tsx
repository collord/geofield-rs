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
import {
  SERIES_COLORS,
  Viewer,
  surfaceKey,
  type DigLineView,
  type SectionContent,
  type SectionQuad,
  type SectionViewState,
  type ViewState,
} from "./Viewer";
import { exportObj } from "./exportObj";
import { TEMPLATES, templateById, blankProject } from "./scenarios";
import {
  type DigitizedLine,
  type Project,
  type SectionSet,
  type SurfaceType,
  loadProject,
  saveProject,
  toSeriesJson,
  buildAuthoredOverlay,
  validate,
  exportProjectJson,
  importProjectJson,
  hydrateMeshPaths,
  slimForExport,
  newId,
} from "./HierarchyPanel/project";
import {
  planeK,
  slabTol,
  distToPlane,
  toInPlane,
  fromInPlane,
  extentDiagonal,
} from "./section/frame";
import { classifyLoops, sectionLoops, sectionPolylines } from "./section/slice";
import { SectionSetForm, SectionsPanel } from "./HierarchyPanel/SectionsPanel";
import {
  addSurface,
  applyInterfaceData,
  addDigitizedLine,
  appendDigitizedVertex,
  moveDigitizedVertex,
  deleteDigitizedVertex,
  deleteSliceLines,
  reverseDigitizedLine,
  createDigitizedSurface,
  getInterface,
  renameInterface,
  flipOrientation,
  duplicateInterface,
  deleteInterface,
  addInterface,
  renameElement,
  setImportRole,
  setImportExtend,
  setIntrusionMode,
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
import type { ManifoldMessage, RenderMode, RunCommand, SurfacePayload, WorkerMessage } from "./protocol";

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

/** One overlay-size slider (0.1×–5×) with a numeric readout. Scales a width/100 baseline, so the
 *  same value reads sensibly on a model of any world size. */
function OverlaySlider(props: {
  label: string;
  testid: string;
  value: number;
  onChange: (v: number) => void;
}) {
  return (
    <label className="overlay-size-slider" style={{ display: "inline-flex", alignItems: "center", gap: "0.35em", fontSize: "0.8em" }}>
      <span>{props.label}</span>
      <input
        type="range"
        data-testid={props.testid}
        min={0.1}
        max={5}
        step={0.1}
        value={props.value}
        onChange={(e) => props.onChange(Number(e.target.value))}
      />
      <span style={{ width: "2.5em", textAlign: "right" }}>{props.value.toFixed(1)}×</span>
    </label>
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
  // Authored points/normals overlay (parametric surfaces): a display toggle + three size sliders
  // (each a scale on a width/100 baseline, so scale-invariant). Persisted in Project.view.
  const [showOverlay, setShowOverlay] = useState(false);
  const [overlayPointScale, setOverlayPointScale] = useState(1);
  const [overlayArrowScale, setOverlayArrowScale] = useState(1);
  const [overlayLabelScale, setOverlayLabelScale] = useState(1);
  const [fitNonce, setFitNonce] = useState(0);
  const [drawNonce, setDrawNonce] = useState(0);
  const [pendingBounds, setPendingBounds] = useState<{ x0: number; x1: number; y0: number; y1: number } | null>(null);
  const drawBounds = useMemo(() => (drawNonce ? { nonce: drawNonce } : undefined), [drawNonce]);
  // --- Cross-sections (feature 005). `activeSection` = the open section VIEW (view-state,
  // exported inside Project.view.section); sections themselves live in project.sections.
  // Section mutations go through `commitSections` (save immediately, NEVER dirty, never
  // re-fit — the setRelation pattern, spec FR-009). Digitizing state is the tool: a target
  // interface + an armed flag; `activeLineRef` tracks the line being extended on the
  // current slice (cleared on step/open/redraw, so a new slice starts a new line).
  const [activeSection, setActiveSection] = useState<{ setId: string; index: number } | null>(null);
  const [drawSectionNonce, setDrawSectionNonce] = useState(0);
  const drawSection = useMemo(() => (drawSectionNonce ? { nonce: drawSectionNonce } : undefined), [drawSectionNonce]);
  const [pendingBaseline, setPendingBaseline] = useState<{ a: [number, number]; b: [number, number] } | null>(null);
  const [digitizeTarget, setDigitizeTarget] = useState<{ elementId: string; interfaceId: string } | null>(null);
  const [digitizeArmed, setDigitizeArmed] = useState(false);
  const activeLineRef = useRef<string | null>(null);
  // Authored points/normals overlay payload — rebuilt only when the elements change (pure).
  const authoredData = useMemo(() => buildAuthoredOverlay(project), [project]);
  // Camera round-trip: the Viewer fills `viewApiRef` with a getter (read on Export);
  // `importedView` (a new object per Import) is pushed back to re-apply a saved view.
  const viewApiRef = useRef<{ getView: () => ViewState; debugCounts: () => { spheres: number; arrows: number } } | null>(null);
  const [importedView, setImportedView] = useState<ViewState | undefined>(undefined);
  const [status, setStatus] = useState<string>("idle");
  const [addMenuOpen, setAddMenuOpen] = useState(false);
  // Which interface the data modal is editing: { elementId, interfaceId } | null.
  const [editing, setEditing] = useState<{ elementId: string; interfaceId: string } | null>(null);

  const lastApplied = useRef<Project>(project);
  const workerRef = useRef<Worker | null>(null);

  // The DEDICATED, PERSISTENT Manifold-boolean worker (manifoldWorker.ts). Unlike the solver
  // worker (terminate-per-fit), this one is created lazily ONCE and never terminated — that is
  // exactly why it can host manifold-3d (which deadlocks on re-instantiation). `runTokenRef` is a
  // monotonic run id bumped on every `runStack`; `pendingRef` holds the CURRENT run's continuation
  // so the once-wired worker `onmessage` can route the composed result to the right `finish` and
  // DROP a superseded run's late result (review 2.1/2.2).
  const manifoldWorkerRef = useRef<Worker | null>(null);
  const runTokenRef = useRef(0);
  const pendingRef = useRef<{
    token: number;
    finish: (surfaces: SurfacePayload[], statusText?: string) => void;
    onDone?: () => void;
  } | null>(null);

  // Lazily create the persistent Manifold worker and wire its handlers ONCE. Reused across every
  // fit and every project load; the `if` guard makes React StrictMode's dev double-mount idempotent.
  function getManifoldWorker(): Worker {
    if (!manifoldWorkerRef.current) {
      const w = new Worker(new URL("./manifoldWorker.ts", import.meta.url), { type: "module" });
      w.onmessage = (e: MessageEvent<ManifoldMessage>) => {
        const msg = e.data;
        const pending = pendingRef.current;
        // STALE: a superseded run's result (or one arriving after we cleared pending) — drop it so
        // it can't overwrite the current model's surfaces/status (review 2.2).
        if (!pending || msg.token !== pending.token) return;
        if (msg.type === "progress") {
          // Stream per-unit progress to the same `status:` line + __statusLog. Keep "(Manifold)" so
          // the /Manifold/ e2e assertion still matches.
          const phase = `composing unit ${msg.k}/${msg.n} (Manifold)…`;
          const wnd = window as unknown as { __statusLog?: string[] };
          (wnd.__statusLog ??= []).push(phase);
          setStatus(phase);
        } else if (msg.type === "done") {
          pendingRef.current = null;
          pending.finish(
            msg.surfaces,
            msg.droppedUnits > 0
              ? `⚠ done — ${msg.droppedUnits} rock unit${msg.droppedUnits > 1 ? "s" : ""} could not be composed`
              : "done",
          );
        } else {
          // A compose failure. Set an error status but do NOT run `onDone` — a failed Apply must
          // stay dirty so the user knows it isn't applied — and release the Apply spinner directly.
          pendingRef.current = null;
          setStatus("error: solid composition failed — " + msg.message);
          setApplying(false);
        }
      };
      w.onerror = (ev: ErrorEvent) => {
        pendingRef.current = null;
        setStatus("error: Manifold worker failed — " + (ev.message || "unknown error"));
        setApplying(false);
      };
      manifoldWorkerRef.current = w;
    }
    return manifoldWorkerRef.current;
  }

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
    // Bump the run token FIRST: this supersedes any Manifold compose still in flight from a prior
    // run (its late result will be dropped by the token check), and tags this run's compose.
    const token = ++runTokenRef.current;
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
      // A non-fatal kernel/compose diagnostic — surface it without discarding the run. The most
      // recent warning wins the status line; it is replaced by the next status/done (review 4.2).
      else if (msg.type === "warning") setStatus("⚠ " + msg.message);
      else if (msg.type === "done") {
        const finish = (surfaces: SurfacePayload[], statusText = "done") => {
          setSurfaces(surfaces);
          setState(applyCoreUpdate(msg.update)); // derived = read-only cache (FR-018)
          setStatus(statusText);
          onDone?.();
        };
        // Unified solid mesher: the worker shipped a boolean RECIPE over single-level-set primitives.
        // Relay it to the DEDICATED, PERSISTENT Manifold worker (it deadlocks if re-instantiated, so
        // it lives in its own never-terminated worker — docs/14, review 2.1). The geology is already
        // baked into the recipe (Rust's clip_maskers); the Manifold worker just runs the booleans and
        // streams per-unit progress. Its result returns via the once-wired handler in
        // `getManifoldWorker`, correlated to this run by `token` (stale results are dropped).
        if (msg.recipeOps && msg.recipeMeta) {
          const { extent, assignment } = msg.recipeMeta;
          const ops = msg.recipeOps;
          pendingRef.current = { token, finish, onDone };
          const mw = getManifoldWorker();
          // Transfer the recipe buffers into the worker (zero-copy). After this `msg.recipeOps` is
          // detached on the main thread — we do not read it again.
          const transfer: Transferable[] = [];
          for (const o of ops) transfer.push(o.positions.buffer, o.indices.buffer);
          mw.postMessage({ type: "compose", token, ops, extent, assignment }, transfer);
        } else {
          finish(msg.surfaces);
        }
      }
    };
    // A worker-level failure (module load, wasm instantiation, uncaught async rejection) fires
    // `onerror`, not `onmessage` — without this the view stays blank/stale with no status and
    // `applying` never clears (review 4.7).
    worker.onerror = (e: ErrorEvent) => {
      setStatus("error: worker failed — " + (e.message || "unknown error"));
      setApplying(false);
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
    return () => {
      workerRef.current?.terminate();
      // Release the persistent Manifold worker on true unmount (page teardown). A remount lazily
      // recreates it via `getManifoldWorker`; nulling the ref keeps that idempotent.
      manifoldWorkerRef.current?.terminate();
      manifoldWorkerRef.current = null;
    };
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

  // FUNCTIONAL commit (feature 005): rapid successive geometry edits — pointer-event
  // digitizing and back-to-back `__digitizeLine` hook calls — can outrun React's render, so
  // a plain `commit(op(project))` builds on a STALE closure and last-write-wins silently
  // drops the earlier edit. Digitizing paths therefore commit through `prev`-based updaters.
  // (The autosave inside the updater is a debounced idempotent write; the app does not run
  // under StrictMode, so updaters execute once.)
  function commitFn(fn: (prev: Project) => Project) {
    setProject((prev) => {
      const next = fn(prev);
      if (next === prev) return prev;
      const reconciled = syncInferredLithologies(next);
      saveProject(reconciled);
      return reconciled;
    });
    setDirty(true);
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

  // =====================================================================================
  // Cross-sections (feature 005).
  // =====================================================================================

  // Section-set mutations are VIEW-state (FR-009): save immediately, never `commit`, never
  // re-fit, `dirty` untouched — the same immediate path as setRelation/commitLithology.
  function commitSections(next: Project) {
    setProject(next);
    saveProject(next);
  }

  const activeSet = activeSection ? (project.sections ?? []).find((s) => s.id === activeSection.setId) : undefined;

  const onSectionBaseline = (a: [number, number], b: [number, number]) => setPendingBaseline({ a, b });
  function onCreateSections(name: string, spacing: number, count: number, offset: number) {
    if (!pendingBaseline) return;
    const set: SectionSet = { id: newId("sect"), name, a: pendingBaseline.a, b: pendingBaseline.b, spacing, count, offset };
    commitSections({ ...project, sections: [...(project.sections ?? []), set] });
    setPendingBaseline(null);
    activeLineRef.current = null;
    setActiveSection({ setId: set.id, index: 0 });
  }
  function onDeleteSectionSet(setId: string) {
    // Deleting a section never deletes geometry: digitized lines carry their own plane
    // snapshot and stay valid surface data (spec edge case / data-model.md).
    commitSections({ ...project, sections: (project.sections ?? []).filter((s) => s.id !== setId) });
    if (activeSection?.setId === setId) setActiveSection(null);
  }
  function onToggleShowSection(setId: string, on: boolean) {
    commitSections({
      ...project,
      sections: (project.sections ?? []).map((s) => (s.id === setId ? { ...s, showInScene: on } : s)),
    });
  }
  function onOpenSection(setId: string) {
    activeLineRef.current = null;
    setActiveSection({ setId, index: 0 });
  }
  function onCloseSection() {
    setActiveSection(null);
    setDigitizeArmed(false);
    activeLineRef.current = null;
  }
  // Step next/previous through a serial set, clamped at the ends; the digitizing tool and
  // its target survive the step (US-SECT-4), only the per-slice active line resets.
  function onStepSection(dir: -1 | 1) {
    if (!activeSection || !activeSet) return;
    const index = Math.min(Math.max(activeSection.index + dir, 0), activeSet.count - 1);
    if (index !== activeSection.index) {
      activeLineRef.current = null;
      setActiveSection({ setId: activeSection.setId, index });
    }
  }

  // --- digitizing (GEOMETRY: everything below commits → dirty → deferred re-fit) --------
  function onPickDigitizeTarget(t: { elementId: string; interfaceId: string } | "new" | null) {
    activeLineRef.current = null;
    if (t === null) {
      setDigitizeTarget(null);
      setDigitizeArmed(false);
      return;
    }
    if (t === "new") {
      const ids = { elementId: newId("el"), interfaceId: newId("iface") };
      commitFn((prev) => createDigitizedSurface(prev, ids).project);
      setDigitizeTarget(ids);
      return;
    }
    setDigitizeTarget(t);
  }
  // All digitizing edits commit FUNCTIONALLY (commitFn) — pointer events and e2e hook calls
  // arrive faster than renders, and a stale-closure commit would silently drop edits.
  function onDigitizeClick(world: [number, number, number]) {
    if (!activeSection || !activeSet || !digitizeTarget) return;
    const frame = planeK(activeSet, activeSection.index);
    const { elementId, interfaceId } = digitizeTarget;
    const sect = activeSection;
    commitFn((prev) => {
      const iface = getInterface(prev, elementId, interfaceId);
      if (!iface) return prev;
      const sliceLines = (iface.digitized ?? []).filter(
        (l) => l.sectionRef?.setId === sect.setId && l.sectionRef.index === sect.index,
      );
      let lineId = activeLineRef.current;
      if (lineId && !sliceLines.some((l) => l.id === lineId)) lineId = null; // stale after redraw/undo
      if (!lineId) {
        // clicking near an existing slice line's TAIL resumes it (click-to-extend, US-DIG-1)
        const pick = Math.max(frame.uLen, 1) * 0.02;
        for (const l of sliceLines) {
          const tail = l.vertices[l.vertices.length - 1];
          if (tail && Math.hypot(tail[0] - world[0], tail[1] - world[1], tail[2] - world[2]) < pick) {
            activeLineRef.current = l.id;
            return prev;
          }
        }
        const id = newId("line");
        activeLineRef.current = id;
        const line: DigitizedLine = {
          id,
          frame: { origin: frame.origin, s: frame.s }, // plane SNAPSHOT — the line outlives the section
          vertices: [world],
          sectionRef: { setId: sect.setId, index: sect.index },
        };
        return addDigitizedLine(prev, elementId, interfaceId, line);
      }
      return appendDigitizedVertex(prev, elementId, interfaceId, lineId, world);
    });
  }
  const onDigitizeMoveVertex = (lineId: string, vi: number, world: [number, number, number]) => {
    if (!digitizeTarget) return;
    const { elementId, interfaceId } = digitizeTarget;
    commitFn((prev) => moveDigitizedVertex(prev, elementId, interfaceId, lineId, vi, world));
  };
  const onDigitizeDeleteVertex = (lineId: string, vi: number) => {
    if (!digitizeTarget) return;
    const { elementId, interfaceId } = digitizeTarget;
    commitFn((prev) => deleteDigitizedVertex(prev, elementId, interfaceId, lineId, vi));
  };
  function onRedigitizeSlice() {
    if (!activeSection || !digitizeTarget) return;
    activeLineRef.current = null;
    const { elementId, interfaceId } = digitizeTarget;
    const sect = activeSection;
    commitFn((prev) => deleteSliceLines(prev, elementId, interfaceId, sect.setId, sect.index));
    setDigitizeArmed(true);
  }
  const onFlipLineToMatch = (elementId: string, interfaceId: string, lineId: string) =>
    commitFn((prev) => reverseDigitizedLine(prev, elementId, interfaceId, lineId));

  // The SECTION SCENE: everything the section view draws, computed UI-side from the
  // already-returned mesh payloads (research.md R1/R2 — no worker round-trip; stepping
  // re-slices instantly). Traces in contact mode; filled unit intervals in solid mode
  // (open chains = the Manifold-off watertightness fallback, drawn unfilled).
  const sectionScene: SectionViewState | undefined = useMemo(() => {
    if (!activeSection || !activeSet) return undefined;
    const frame = planeK(activeSet, activeSection.index);
    const [, , , , z0, z1] = project.extent;
    const quantum = Math.max(frame.uLen, z1 - z0, 1) * 1e-5;
    const hexToNum = (hex: string) => parseInt(hex.replace("#", ""), 16) || 0x888888;
    const lithoColors = Object.fromEntries(catalogOf(project).lithologies.map((l) => [l.id, l.color]));
    const traces: SectionContent["traces"] = [];
    const fills: SectionContent["fills"] = [];
    for (const s of surfaces) {
      if (hiddenKeys.has(surfaceKey(s))) continue;
      const color =
        renderMode === "solid"
          ? s.lithologyId && lithoColors[s.lithologyId] !== undefined
            ? hexToNum(lithoColors[s.lithologyId])
            : 0xc8c8cd
          : SERIES_COLORS[s.series % SERIES_COLORS.length];
      if (renderMode === "solid") {
        const { closed, open } = sectionLoops(s.positions, s.indices, frame, quantum);
        if (closed.length > 0) fills.push({ color, shapes: classifyLoops(closed) });
        if (open.length > 0) traces.push({ color, polys: open });
      } else {
        const polys = sectionPolylines(s.positions, s.indices, frame, quantum);
        if (polys.length > 0) traces.push({ color, polys });
      }
    }
    const tol = slabTol(activeSet, project.extent);
    const backdrop = authoredData.points
      .filter((p) => Math.abs(distToPlane(frame, p.pos)) <= tol)
      .map((p) => ({ p: toInPlane(frame, p.pos), color: SERIES_COLORS[p.colorIndex % SERIES_COLORS.length] }));
    // digitized lines lying on THIS plane (matched by geometry, not sectionRef — a line
    // whose section was deleted still shows wherever a coincident plane is opened)
    const planeEps = Math.max(1e-6 * extentDiagonal(project.extent), quantum * 10);
    const lines: DigLineView[] = [];
    project.elements.forEach((el, colorIndex) => {
      const defaultFlip = el.kind === "series" && el.relation === "erode";
      const ifaces =
        el.kind === "series" ? el.interfaces
        : el.kind === "vein" ? [el.footWall, el.hangingWall]
        : el.kind === "intrusion" ? ((el.mode ?? "shell") === "solid" ? [el.envelopeOuter] : [el.envelopeInner, el.envelopeOuter])
        : el.kind === "topography" ? [el.ground]
        : [];
      for (const iface of ifaces) {
        const sign = ((!!iface.flipped !== defaultFlip) ? -1 : 1) as 1 | -1;
        for (const line of iface.digitized ?? []) {
          const parallel = Math.abs(line.frame.s[0] * frame.s[0] + line.frame.s[1] * frame.s[1]) > 0.9999;
          if (!parallel || Math.abs(distToPlane(frame, line.frame.origin)) > planeEps) continue;
          lines.push({
            lineId: line.id,
            color: SERIES_COLORS[colorIndex % SERIES_COLORS.length],
            isTarget: digitizeTarget?.interfaceId === iface.id,
            sign,
            s: line.frame.s,
            vertices: line.vertices,
          });
        }
      }
    });
    return {
      key: `${activeSection.setId}:${activeSection.index}`,
      label: `${activeSet.name}${activeSet.count > 1 ? ` · ${activeSection.index + 1}/${activeSet.count}` : ""}`,
      frame,
      zRange: [z0, z1],
      content: { traces, fills, backdrop, lines },
    };
  }, [activeSection, activeSet, project, surfaces, renderMode, hiddenKeys, digitizeTarget, authoredData]);

  // 3D plane quads for sets with showInScene on (US-SECT-5).
  const sectionQuads: SectionQuad[] | undefined = useMemo(() => {
    const sets = (project.sections ?? []).filter((s) => s.showInScene);
    if (sets.length === 0) return undefined;
    const [, , , , z0, z1] = project.extent;
    const quads: SectionQuad[] = [];
    for (const set of sets) {
      for (let k = 0; k < set.count; k++) {
        quads.push({
          frame: planeK(set, k),
          zRange: [z0, z1],
          active: activeSection?.setId === set.id && activeSection.index === k,
        });
      }
    }
    return quads;
  }, [project.sections, project.extent, activeSection]);

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
  const onSetIntrusionMode = (elementId: string, mode: "solid" | "shell") =>
    commit(setIntrusionMode(project, elementId, mode));

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
      __overlayCounts?: () => { spheres: number; arrows: number } | undefined;
      __exactSpikeDigest?: () => Promise<string | undefined>;
      __slimForExport?: (p: Project) => Project;
      __importProjectJson?: (jsonText: string) => Promise<Project>;
      __project?: () => Project;
      __sections?: () => SectionSet[];
      __activeSection?: () => { setId: string; index: number } | null;
      __addSectionSet?: (set: Partial<SectionSet> & { a: [number, number]; b: [number, number] }) => string;
      __openSection?: (setId: string, index: number) => void;
      __sectionTraces?: (setId: string, index: number) => { series: number; block: number; iface: number; polylines: number[][][] }[];
      __digitizeLine?: (
        elementId: string | null,
        interfaceId: string | null,
        setId: string,
        index: number,
        verts2d: [number, number][],
      ) => { elementId: string; interfaceId: string; lineId: string } | null;
      __exportState?: () => Project;
    };
    w.__reorderContainers = onReorderContainers;
    w.__moveSurface = onMoveSurface;
    // Points/normals overlay geometry counts, for the overlay e2e (e2e/points-normals.spec.ts).
    w.__overlayCounts = () => viewApiRef.current?.debugCounts();
    // Test hooks (harmless in the demo): inject a Project directly (skips the file picker)
    // and expose the rendered surfaces for geometric assertions. Used by the flipped-erosion
    // QEF-spike regression spec (e2e/qef-spike.spec.ts). `__getView` reads the current camera
    // (so a viewpoint can be grabbed/asserted, and shared for repro).
    w.__loadProject = loadFreshProject;
    // Externalized-mesh round-trip hooks (e2e/mesh-path-roundtrip.spec.ts): slim a project for export
    // and re-load a slim JSON (re-fetching path-only meshes). Harmless in the demo.
    w.__slimForExport = slimForExport;
    w.__importProjectJson = (jsonText: string) => importProjectJson(new File([jsonText], "rt.geoproj.json"));
    w.__surfaces = surfaces;
    w.__getView = () => viewApiRef.current?.getView();
    // --- feature 005 hooks (e2e; harmless in the demo — contracts/project-json.md) -------
    w.__project = () => project;
    w.__sections = () => project.sections ?? [];
    w.__activeSection = () => activeSection;
    // Programmatic set creation (the gesture path is separately covered by canvas clicks).
    // FUNCTIONAL update: a hook call right after another hook's commit must not build on the
    // stale closure `project` (React's update queue preserves order between updaters).
    w.__addSectionSet = (partial) => {
      const set: SectionSet = {
        id: newId("sect"),
        name: partial.name ?? "Sections",
        a: partial.a,
        b: partial.b,
        spacing: partial.spacing ?? 0,
        count: partial.count ?? 1,
        offset: partial.offset ?? 0,
        showInScene: partial.showInScene ?? false,
      };
      setProject((prev) => {
        const next = { ...prev, sections: [...(prev.sections ?? []), set] };
        saveProject(next); // non-dirty view-state, like commitSections
        return next;
      });
      return set.id;
    };
    w.__openSection = (setId, index) => {
      activeLineRef.current = null;
      setActiveSection({ setId, index });
    };
    // World-space traces of every payload on plane (setId, index) — the loft "trace lands
    // on the drawn line" assertion reads these (research.md R9).
    w.__sectionTraces = (setId, index) => {
      const set = (project.sections ?? []).find((s) => s.id === setId);
      if (!set) return [];
      const frame = planeK(set, index);
      const [, , , , z0, z1] = project.extent;
      const quantum = Math.max(frame.uLen, z1 - z0, 1) * 1e-5;
      return surfaces.map((s) => ({
        series: s.series,
        block: s.block,
        iface: s.interface,
        polylines: sectionPolylines(s.positions, s.indices, frame, quantum).map((poly) =>
          poly.map((p) => fromInPlane(frame, p[0], p[1]) as number[]),
        ),
      }));
    };
    // Programmatic digitizing (bypasses pointer math; same commit path as the tool).
    // elementId/interfaceId null ⇒ creates a new digitized surface and targets it. Commits
    // FUNCTIONALLY (commitFn): back-to-back hook calls outrun renders, and a stale-closure
    // commit would drop the prior line (the race the section-validation specs exposed).
    w.__digitizeLine = (elementId, interfaceId, setId, index, verts2d) => {
      const create = !elementId || !interfaceId ? { elementId: newId("el"), interfaceId: newId("iface") } : null;
      const el = create?.elementId ?? elementId!;
      const ifid = create?.interfaceId ?? interfaceId!;
      const lineId = newId("line");
      // The SET is resolved inside the updater too (it may have been added by an
      // immediately-preceding __addSectionSet whose updater hasn't applied yet). Ids are
      // returned optimistically; an unknown setId makes the commit a no-op.
      commitFn((prev) => {
        const set = (prev.sections ?? []).find((s) => s.id === setId);
        if (!set || index < 0 || index >= set.count) return prev;
        const frame = planeK(set, index);
        const line: DigitizedLine = {
          id: lineId,
          frame: { origin: frame.origin, s: frame.s },
          vertices: verts2d.map(([du, dw]) => fromInPlane(frame, du, dw)),
          sectionRef: { setId, index },
        };
        const p = create ? createDigitizedSurface(prev, create).project : prev;
        return addDigitizedLine(p, el, ifid, line);
      });
      setDigitizeTarget({ elementId: el, interfaceId: ifid });
      return { elementId: el, interfaceId: ifid, lineId };
    };
    // The exact object Export serializes (incl. the view block with the active section) —
    // lets the persistence e2e round-trip without driving a real file download.
    w.__exportState = () => buildExportProject();
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
      if (p.view.overlay !== undefined) setShowOverlay(p.view.overlay);
      if (p.view.overlayPointScale !== undefined) setOverlayPointScale(p.view.overlayPointScale);
      if (p.view.overlayArrowScale !== undefined) setOverlayArrowScale(p.view.overlayArrowScale);
      if (p.view.overlayLabelScale !== undefined) setOverlayLabelScale(p.view.overlayLabelScale);
      setImportedView({ camera: p.view.camera, target: p.view.target, up: p.view.up, fov: p.view.fov });
    }
    // Restore the open section (feature 005, FR-010). Guarded: a dangling/clamped ref was
    // already sanitized on import; a seed/template without one lands in the 3D view.
    const sect = p.view?.section;
    activeLineRef.current = null;
    setDigitizeTarget(null);
    setDigitizeArmed(false);
    setActiveSection(sect && (p.sections ?? []).some((s) => s.id === sect.setId) ? sect : null);
    if (p.elements.length > 0 && validate(p).canApply) runStack(p, undefined, mode, clip);
    else {
      setSurfaces([]);
      setState(emptyState());
      setStatus("idle");
    }
  }
  const onNew = () => confirmIfDirty(() => loadFreshProject(blankProject()));
  const onSeed = (templateId: string) =>
    confirmIfDirty(async () => {
      const t = templateById(templateId);
      if (!t) return;
      try {
        // Hydrate any imported-surface `objPath` refs (e.g. the imported-mesh template) by
        // fetching the .obj from public/meshes/ — the same step importProjectJson runs. A
        // template with no mesh refs is returned unchanged.
        loadFreshProject(await hydrateMeshPaths(t.build()));
      } catch (err) {
        setStatus("seed failed: " + (err instanceof Error ? err.message : String(err)));
      }
    });
  // The exported project = the live project + the captured view block (camera, render
  // toggles, and — feature 005 — the open section, so a reopened export lands on the same
  // slice, FR-010). Shared by onExport and the __exportState e2e hook.
  function buildExportProject(): Project {
    const cam = viewApiRef.current?.getView();
    const view = cam && {
      ...cam,
      renderMode,
      seamClip,
      edges: wireframe,
      overlay: showOverlay,
      overlayPointScale,
      overlayArrowScale,
      overlayLabelScale,
      section: activeSection ?? undefined,
    };
    return view ? { ...project, view } : project;
  }
  const onExport = () => {
    exportProjectJson(buildExportProject());
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
        // Record an external reference so EXPORT can drop the (often multi-MB) inline `objText` and
        // the loader re-fetches from `public/meshes/<file>`. The user places the .obj there; the live
        // session still uses the `objText` just read, so import works with no server round-trip.
        objPath: `meshes/${file.name}`,
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
            <label
              className="debug-toggle"
              title="Overlay the authored contact points (spheres) and surface normals (arrows — the geometric up-normal ⊥ each surface, derived from the reading's dip/dip-direction) that define each parametric surface, labeled by row number. Imported meshes have none."
            >
              <input
                type="checkbox"
                data-testid="points-normals-toggle"
                checked={showOverlay}
                onChange={(e) => setShowOverlay(e.target.checked)}
              />
              points/normals
            </label>
            <button
              type="button"
              data-testid="fit-view"
              title="Frame the camera on the data (above-south-east, fit to extent)."
              onClick={() => setFitNonce((n) => n + 1)}
            >
              Fit view
            </button>
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
          {/* Overlay size sliders — shown only when the points/normals overlay is on. Each is a
              scale multiplier on the width/100 baseline (default 1.0×), so scale-invariant. */}
          {showOverlay && (
            <div className="overlay-size-controls" data-testid="overlay-size-controls">
              <OverlaySlider
                label="point size"
                testid="overlay-point-size"
                value={overlayPointScale}
                onChange={setOverlayPointScale}
              />
              <OverlaySlider
                label="arrow length"
                testid="overlay-arrow-size"
                value={overlayArrowScale}
                onChange={setOverlayArrowScale}
              />
              <OverlaySlider
                label="label size"
                testid="overlay-label-size"
                value={overlayLabelScale}
                onChange={setOverlayLabelScale}
              />
            </div>
          )}
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
            {validation.softWarnings.length > 0 && (
              <div className="soft-warnings" data-testid="soft-warnings" style={{ fontSize: "0.75em", color: "#96690a", display: "grid", gap: "0.2rem", width: "100%" }}>
                {validation.softWarnings.slice(0, 4).map((wrn, i) => (
                  <div key={i}>
                    ⚠ {wrn.reason}
                    {wrn.fix === "flipLine" && wrn.lineId && wrn.interfaceId && (
                      <button
                        type="button"
                        data-testid="flip-line-to-match"
                        style={{ marginLeft: "0.4em" }}
                        onClick={() => onFlipLineToMatch(wrn.elementId, wrn.interfaceId!, wrn.lineId!)}
                      >
                        flip this line to match
                      </button>
                    )}
                  </div>
                ))}
              </div>
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
          onSetIntrusionMode={onSetIntrusionMode}
          unitName={(elementId, unitIndex, displayPos) => unitNameOf(project, elementId, unitIndex, displayPos)}
          onRenameUnit={(elementId, unitIndex, name) => commitLithology(renameUnit(project, elementId, unitIndex, name))}
          catalog={catalogOf(project).lithologies}
          lithologyOf={(elementId, unitIndex) => lithologyOf(project, elementId, unitIndex)}
          onAssignLithology={(elementId, unitIndex, id) => commitLithology(assignLithology(project, elementId, unitIndex, id))}
        />

        <SectionsPanel
          project={project}
          activeSection={activeSection}
          onDrawBaseline={() => setDrawSectionNonce((n) => n + 1)}
          onOpen={onOpenSection}
          onClose={onCloseSection}
          onDelete={onDeleteSectionSet}
          onToggleShow={onToggleShowSection}
          digitizeTarget={digitizeTarget}
          onPickTarget={onPickDigitizeTarget}
          digitizeArmed={digitizeArmed}
          onArmDigitize={setDigitizeArmed}
          onRedigitizeSlice={onRedigitizeSlice}
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
          authoredData={authoredData}
          showAuthored={showOverlay}
          overlaySizes={{ point: overlayPointScale, arrow: overlayArrowScale, label: overlayLabelScale }}
          drawBounds={drawBounds}
          onBoundsXY={onBoundsXY}
          fitNonce={fitNonce}
          drawSection={drawSection}
          onSectionBaseline={onSectionBaseline}
          sectionView={sectionScene}
          onSectionStep={onStepSection}
          sectionQuads={sectionQuads}
          digitize={{ armed: digitizeArmed }}
          onDigitizeClick={onDigitizeClick}
          onDigitizeMove={onDigitizeMoveVertex}
          onDigitizeDelete={onDigitizeDeleteVertex}
        />
        {pendingBounds && (
          <ZRangeForm
            extent={project.extent}
            rect={pendingBounds}
            onConfirm={onConfirmBounds}
            onCancel={() => setPendingBounds(null)}
          />
        )}
        {pendingBaseline && (
          <SectionSetForm
            baseline={pendingBaseline}
            defaultSpacing={extentDiagonal(project.extent) / 10}
            nextName={`Section ${(project.sections?.length ?? 0) + 1}`}
            onConfirm={onCreateSections}
            onCancel={() => setPendingBaseline(null)}
          />
        )}
        {sectionScene && activeSet && activeSection && (
          <div
            data-testid="section-active"
            style={{
              position: "absolute", top: 12, right: 12, zIndex: 10, background: "#fff",
              border: "1px solid #ccd", borderRadius: 6, padding: "0.35rem 0.6rem",
              display: "flex", gap: "0.4rem", alignItems: "center", fontSize: "0.85em",
              boxShadow: "0 2px 8px rgba(0,0,0,0.15)",
            }}
          >
            <span data-testid="section-label">{sectionScene.label}</span>
            <button
              type="button"
              data-testid="section-step-prev"
              title="previous parallel section (PageUp)"
              disabled={activeSection.index <= 0}
              onClick={() => onStepSection(-1)}
            >
              ◀
            </button>
            <button
              type="button"
              data-testid="section-step-next"
              title="next parallel section (PageDown)"
              disabled={activeSection.index >= activeSet.count - 1}
              onClick={() => onStepSection(1)}
            >
              ▶
            </button>
            <button type="button" data-testid="section-close" title="back to the 3D view" onClick={onCloseSection}>
              ✕ 3D
            </button>
          </div>
        )}
      </main>

      {editing && editingIface && (
        <SurfaceDataModal
          surfaceName={editingIface.name}
          parentName={editingIface.parentName}
          points={editingIface.points}
          orientations={editingIface.orientations}
          digitized={editingIface.digitized}
          onApply={(points, orientations, digitized) => {
            commit(applyInterfaceData(project, editing.elementId, editing.interfaceId, points, orientations, digitized));
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
