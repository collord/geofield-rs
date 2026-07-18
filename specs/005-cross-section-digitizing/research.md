# Research — Cross-Sections & Digitized Surfaces (005)

All unknowns from Technical Context resolved below. Sources: `web3/SPEC.md` (governing doc),
`web3/SPEC_cross_sections.md` (input fragment), and a code audit of `web3/src`
(main.tsx, Viewer.tsx, HierarchyPanel/project.ts, worker.ts, protocol.ts, manifoldVolumes.ts).

---

## R1 — Contact-mode traces: UI-side plane∩mesh, from already-returned payloads

**Decision**: Compute surface traces on the main thread as a per-triangle plane-crossing edge walk
over each `SurfacePayload`'s `positions`/`indices` typed arrays (`protocol.ts:96-98`), yielding
world-space segments chained into polylines. Recompute on section open/step and whenever
`surfaces` changes (i.e. after Apply).

**Rationale**: The solver worker is **terminate-per-fit** (`main.tsx:243` orchestration): there is no
held stack to ask for a cheap re-slice, so an engine-side slice op would force either a full re-fit
per section step or a new persistent-worker protocol — both violate "sections never re-fit"
(spec FR-009) and Principle VI. The payloads are plain typed arrays already on the main thread;
an O(#triangles) scan at ~1e5 tris is well inside the <50 ms budget.

**Alternatives considered**: (a) engine-side `mesh_clip`-based slice via the wasm ABI — rejected:
ABI growth, worker round-trip per step, and the no-engine-change premise of the fragment (§11.5);
(b) three.js `clippingPlanes` — rejected: a render effect, not data; can't back e2e assertions or
the backdrop's 2D reading, and doesn't produce traces (it hides geometry).

## R2 — Solid-mode filled intervals: loop assembly + 2D fill; Manifold NOT used

**Decision**: Slice each unit solid's mesh with the section plane (same segment walk as R1), assemble
segments into closed loops by endpoint hashing, classify holes by even-odd containment in the plane's
2D frame, and triangulate with `THREE.ShapeGeometry`. Color by lithology as in 3D solid mode.

**Rationale**: Unit solids from the default Manifold compose path are watertight, so plane∩solid
yields closed loops by construction. This stays entirely UI-side and synchronous — stepping through a
serial set re-fills instantly with zero worker traffic.

**Rejected — thin-slab Manifold intersect**: manifold-3d is confined to the single persistent worker
(re-instantiation deadlocks per realm, `manifoldVolumes.ts:6-10`); slab-cutting would add a
round-trip + protocol surface per section step, for output we'd still have to flatten to 2D.

**Logged bound (Principle IV)**: with the diagnostic **Manifold toggle off** (legacy field-domain
solids), watertightness is not guaranteed; loop assembly may fail. Fallback: draw the unfilled
boundary segments and skip fill for loops that don't close. Noted in SPEC.md §10 on merge and in a
code comment at the fill site. (The toggle is an engine diagnostic, not authoring — SPEC.md §6.4.)

## R3 — Younging convention: prose is normative; the fragment's formula gets a sign fix

**Decision**: The user-facing rule (spec FR-013, fragment §11.1 prose) is normative: *face-on
(section normal `s` toward the viewer, Z up), the surface youngs to the **left** of the drawn
direction*. With the fragment's own frame (`u` = baseline direction = screen right, `w` = +Z =
screen up, `s = u × w` = toward the viewer), the pole to the younger side is

&nbsp;&nbsp;&nbsp;&nbsp;`m = normalize(ŝ × t̂)`   (t̂ = local drawn-direction tangent)

**not** the fragment §11.2's `m = normalize(t × s)`, which points to the *right* of the drawn
direction (worked check: u=+X, s=−Y after `u×w`; face-on camera looks −s=+Y; draw t=+X ⇒ screen-left
is +Z; `ŝ×t̂ = (−Ŷ)×X̂ = +Ẑ` ✓, `t̂×ŝ = −Ẑ` ✗). Full derivation + worked example in
[contracts/younging-derivation.md](contracts/younging-derivation.md).

**Rationale**: Principle II — the document wins, but amendments are deliberate: when the fragment is
merged into `SPEC.md`, §11.2's formula is corrected to match its own §11.1 prose, with a note. The
prose is what users see (the affordance) and what the spec's FR-013 encodes, so it wins over the
formula. The affordance + reversible flip make any residual convention surprise user-correctable.

## R4 — Feeding poles to the fit: additive direct-pole path in `flattenInterfaces`

**Decision**: Extend the single compile site (`flattenInterfaces`, `project.ts:235-261`) to emit,
after the dip/dipDir-derived orientations, the digitized orientations: per line **segment**, one
`ori_xyz` at the segment midpoint and one `ori_pole = flipSign * (ŝ × t̂)` — the same
`s = (flipped !== defaultFlip) ? -1 : 1` XOR already applied to typed orientations, at the same
single site.

**Rejected — back-convert `m` to dip/dipDir rows**: dip∈[0,90] + a *per-surface* flip bit spans only
one hemisphere per surface. A digitized line with steep/overturned segments produces per-segment
poles in *both* hemispheres; forcing them through dip/dipDir would silently negate some — corrupting
exactly the younging information digitizing exists to capture. (It would also inject synthetic rows
into the user's typed table, violating "the table round-trips what was entered", `project.ts:60-62`.)

**Per-segment (not per-vertex)**: a segment has an unambiguous tangent; a vertex needs neighbor
averaging with degenerate cases at endpoints/kinks. Spec assumption explicitly left this open;
per-segment-midpoint is the smallest unambiguous choice. A 2-vertex line yields 1 orientation.

## R5 — Declared state: section sets and self-sufficient digitized lines

**Decision**:
- **Sections**: `Project.sections?: SectionSet[]` where a set = `{id, name, a:[x,y], b:[x,y],
  spacing, count, offset?}` (count=1 ⇒ a single section; per-plane geometry is *derived*, never
  stored). Mutations go through a **non-dirty** updater cloned from the `setRelation`/
  `commitLithology` pattern (`main.tsx:365,390`): save immediately, never `commit`, never re-fit.
  Active section + per-set 3D-plane visibility live in the `Project.view` block (view-state, like
  camera), threaded through the same export/import path as US-PROJ-4.
- **Digitized lines**: `InterfaceData.digitized?: DigitizedLine[]` where a line =
  `{id, sectionRef?, frame: {origin, s}, vertices: [x,y,z][]}` — world-space ordered vertices plus a
  **snapshot of its plane** (origin + unit normal). The plane snapshot makes the line self-sufficient:
  deleting the section (a view construct) never orphans the geometry (spec edge case), and the pole
  derivation always has its `s`. `sectionRef` is a soft reference for UI grouping only.

**Rationale (Principle III)**: baseline+spacing+count and vertex-order+position are the irreducible
declarations; frames, planes, tangents, poles, traces are all derived. Digitized vertices are *not*
copied into `points[]` — the points table renders the union at view time and writes edits back to the
line (FR-012's "one dataset, two views" without a second copy that can drift).

**Alternatives**: storing vertices as in-plane 2D (u,w) would make off-plane states unrepresentable,
but breaks the points-table contract (world XYZ rows, two-way edit) and complicates every consumer;
instead world XYZ + an import-time on-plane guard (hard error, FR-019) + modal edits projected back
onto the plane (documented in data-model.md).

## R6 — Section view: OrthographicCamera mode inside the existing Viewer

**Decision**: Add a section mode to `Viewer.tsx`: a `THREE.OrthographicCamera` (three already ships
it; zero hits in src today) placed on the −s side at the active plane, looking +ŝ·(−1) (i.e. along
−s toward the plane... concretely: positioned so `s` points toward the camera), `up = (0,0,1)`,
frustum fit to baseline-length × Z-range; OrbitControls restricted to pan/zoom (`enableRotate =
false`). Baseline picking clones the proven two-click **drawBounds effect** (`Viewer.tsx:432-539`):
top-down snap, ray∩`THREE.Plane` at z-mid, rubber-band preview, Escape cancels, triggered by a nonce
prop. Digitizing clicks reuse the same `raycaster.ray.intersectPlane` against the section plane.
Section-scene content (traces, fills, backdrop points, younging affordance, plane quads) are
dedicated groups on `stateRef`, mirroring the existing overlay/rubber conventions.

**Rationale**: One Viewer, one scene graph, one set of conventions — smallest intervention; the
bounds gesture is the spec's own stated mirror (US-SECT-1 ↔ US-BOUNDS-1).

## R7 — Younging affordance & consistency check share one derivation module

**Decision**: `src/section/digitizeDerive.ts` owns tangents, poles, and the per-line mean pole. The
Viewer draws the affordance from it (direction arrow cones along the line + short hachure ticks on
the younger side, in-plane offset by `ŝ × t̂`); `flattenInterfaces` compiles from it; `validate()`
warns from it (two sibling lines with `dot(meanPole_i, meanPole_j) < 0` ⇒ "inconsistent younging" +
per-line flip-to-match offer, which reverses that line's *vertex order* — the only per-line flip
that exists; the per-surface `flipped` bit stays the US-DATA-2 toggle).

**Rationale**: one home for the rule (Principle III). Note the flip-to-match action edits geometry
(vertex order) so it **is** a dirty/commit edit, unlike the per-surface flip which is also a commit
edit today (flipping marks dirty — it changes poles).

## R8 — Backdrop slab tolerance

**Decision**: authored points within `|dot(p − origin, ŝ)| ≤ tol` of the plane are shown;
`tol = spacing/2` for serial sets (each point appears on exactly one nearest slice), and for
standalone sections (count=1) `tol = 2.5%` of the extent diagonal. Constant lives in
`section/frame.ts` with this rationale.

## R9 — Test gates (Constitution I) and the unit-test bound

**Decision**: Behavior gate = five new Playwright specs (see quickstart.md): section-view,
section-digitize, section-loft, section-persist, section-validation. They drive the real app via the
established patterns: canvas clicks (as `e2e/model-bounds.spec.ts` does for the bounds gesture) plus
new `window.__` hooks mirroring `__loadProject`/`__getView` (`main.tsx:471-511`): `__sections()`,
`__sectionTraces(setId, index)` (world-space polylines for land-on-line assertions), and
`__digitizeLine(surfaceId, setId, index, verts)` (programmatic injection so loft tests aren't
hostage to pointer-event pixel math). The loft assertion: for each digitized vertex, the re-fit
surface's trace on that section passes within `ε = spacing × 0.1` (tunable; asserted in-plane).

**Logged bound**: web3 has no unit-test framework and none is added (Principle VI); the pure helpers
in `src/section/` are pinned only through these e2e hooks. If slicer edge-cases start biting,
adding vitest is the named seam — deferred, not forgotten.

## R10 — Governing-doc merge

**Decision**: As part of implementation (not follow-up), append the fragment to `web3/SPEC.md` as
§10/§11 (renumbering its acceptance §9 cross-refs), apply the R3 sign fix with an amendment note,
and reduce `web3/SPEC_cross_sections.md` to a pointer (or delete it) so the rule lives once.
`docs/` needs no new engine doc — no engine subsystem is touched.
