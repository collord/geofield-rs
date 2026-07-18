# Data Model — Cross-Sections & Digitized Surfaces (005)

Additive extensions to the web3 authoring model (`web3/src/HierarchyPanel/project.ts`).
Everything here is **declared-state minimal** (Constitution III): planes, frames, tangents, poles,
traces, and fills are derived and never stored. Serialization contract:
[contracts/project-json.md](contracts/project-json.md). Derivation math:
[contracts/younging-derivation.md](contracts/younging-derivation.md).

## Entities

### SectionSet *(new — `Project.sections?: SectionSet[]`)*

A family of parallel vertical section planes from one baseline. A single section is a set with
`count: 1`. **View/authoring construct, not geology**: no mutation of a SectionSet sets `dirty` or
triggers a fit.

| Field | Type | Meaning |
|---|---|---|
| `id` | string | stable client id |
| `name` | string | display name (default "Section N" / "Sections N") |
| `a`, `b` | `[x, y]` | baseline endpoints, picked in plan view; `A → B` is the in-plane u axis |
| `spacing` | number | offset between adjacent planes along `ŝ` (world units); irrelevant when `count = 1` |
| `count` | integer ≥ 1 | number of parallel planes |
| `offset` | number (default 0) | shift of plane 0 from the baseline along `ŝ` (the fragment's front/back offsets collapse to this start-offset + count) |
| `showInScene` | boolean (default false) | 3D translucent-quad toggle (US-SECT-5) |

**Derived (never stored)** — see contracts/younging-derivation.md §1:
`u = normalize(b − a)` (horizontal), `w = ẑ`, `s = u × w` (horizontal, unit);
plane *k* (0-based, k < count): passes through `a + ŝ·(offset + k·spacing)`, normal `ŝ`, spanning
the full extent Z-range; in-plane coordinates `(du, dw)` measured from the offset baseline origin.

**Invariants**: `|b − a| > εbaseline` (degenerate two-click rejected at the gesture, spec edge case);
`count ≥ 1`; `spacing > 0` when `count > 1`.

### ActiveSection *(new — `Project.view.section?`)*

View-state, persisted with the saved view (US-PROJ-4 extension, FR-010).

| Field | Type | Meaning |
|---|---|---|
| `setId` | string | the open SectionSet |
| `index` | integer | which plane of the set is active (clamped step, spec assumption) |

Section view open/closed is implied by presence; stepping mutates `index` only (non-dirty).

### DigitizedLine *(new — `InterfaceData.digitized?: DigitizedLine[]`)*

An ordered polyline drawn on one section plane for one surface. **Geometry**: every mutation is a
deferred `commit` edit (dirty, re-fit on Apply). Self-sufficient: carries a snapshot of its plane so
deleting its section never orphans it.

| Field | Type | Meaning |
|---|---|---|
| `id` | string | stable client id |
| `frame` | `{ origin: [x,y,z], s: [x,y,z] }` | plane snapshot at digitize time: a point on the plane and the unit out-of-plane normal `ŝ` |
| `vertices` | `[x, y, z][]`, length ≥ 2 to be fittable | ordered world-space vertices, all on the plane; **order is the drawn direction** and is load-bearing (it encodes younging) |
| `sectionRef` | `{ setId, index }?` | soft reference for UI grouping/step-editing; may dangle after section deletion — never dereferenced for geometry |

**Derived (never stored)**: per-segment tangents `t̂ᵢ`, per-segment poles `mᵢ = ŝ × t̂ᵢ` (contract §2),
orientation sample positions (segment midpoints), the line's mean pole (for the consistency warning),
and the younging-affordance geometry.

**Invariants**: all vertices satisfy `|dot(v − origin, ŝ)| ≤ εplane` — guaranteed by the plane-locked
tool, **hard-guarded on project import** (FR-019); points-table edits to a digitized vertex are
projected back onto the plane before storing (the documented resolution of the table/plane tension).

### InterfaceData *(extended)*

`digitized?: DigitizedLine[]` added. `points`, `orientations`, `flipped` unchanged. A surface may
freely mix typed rows and digitized lines. **The points table and points/normals overlay render the
union** of `points` and all digitized vertices (tagged by provenance); digitized rows edit through to
their line — no copy exists (FR-012).

`flipped` (US-DATA-2) applies to digitized poles through the **same XOR at the same single site**
(`flattenInterfaces`): `sign = (flipped !== defaultFlip) ? −1 : +1` multiplies `mᵢ`. Flip never
touches `vertices`.

### Project / view *(extended)*

- `sections?: SectionSet[]` (as above).
- `view.section?: ActiveSection` — active slice restored on import (SC-005).
- Everything else unchanged; `.geoproj.json` stays **version 1** (fields are optional/additive;
  old files load unchanged, old apps ignore new fields — acceptable for the prototype).

## State transitions

| Action | Path | dirty? | re-fit? |
|---|---|---|---|
| create / rename / re-baseline / re-space / delete SectionSet | non-dirty updater (setRelation pattern) | no | no |
| open section view / step / toggle plane quad | view-state updater | no | no |
| toggle render mode while in section view | existing `setRenderModeAndRerun` | no | yes (existing documented tradeoff, `main.tsx:325`) |
| add/extend line, move vertex, delete vertex, re-digitize slice | `commit` | yes | on Apply |
| flip surface (per-surface `flipped`) | `commit` (existing `flipOrientation`) | yes | on Apply |
| "flip this line to match" (reverses `vertices` order) | `commit` | yes | on Apply |
| Apply | existing `onApply` → `toSeriesJson` → worker | clears | yes |

## Compile mapping (→ existing ABI, no change)

Per surface, `flattenInterfaces` emits **after** the typed rows:

- each digitized vertex → one `sp_xyz` entry (same `sp_if` as the interface);
- each digitized segment → one `ori_xyz` (midpoint) + one `ori_pole = sign · (ŝ × t̂)`.

The wasm `add_series`/`add_topography`/… calls and `SeriesJson` shape are untouched.

## Validation rules (extends `validate()`, `project.ts:438` — structural only)

**Hard (push to `hardErrors`, blocks Apply — FR-019)**

1. A `DigitizedLine` with `vertices.length < 2`.
2. A surface whose **entire** data set (no typed points, no typed orientations) lies on a single
   plane: all digitized lines share one `frame` plane (co-planar within `εplane`).
3. Import-time: any digitized vertex off its line's plane beyond `εplane` (tool makes this
   unreachable; the guard is for hand-edited/foreign files).

**Soft (push to `softWarnings` — FR-020)**

4. Under-constrained: all of a surface's lines on one plane (but other data exists), or on parallel
   planes with no across-section variation (mean in-plane profiles equal within tolerance) — warn
   "≥2 (ideally 3) sections determine the surface".
5. Inconsistent younging: two sibling lines with `dot(meanPole_i, meanPole_j) < 0` — warn + offer
   per-line flip-to-match (R7).
6. A line that back-tracks or self-crosses in-plane (segment-pair intersection in `(du, dw)`) —
   warn "ambiguous direction".

These stay **structural** (can the fit even run / is the input self-consistent); no core geological
rule is re-encoded (the note at `project.ts:432-437` still holds).
