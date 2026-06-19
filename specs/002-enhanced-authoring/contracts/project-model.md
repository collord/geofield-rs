# Contract: Authoring ↔ Fit Seam

**Feature**: 002-enhanced-authoring | **Date**: 2026-06-19

This is the **single seam** between the new authoring layer and the unchanged fit path. The worker, `protocol.ts`, `kernel-bridge.ts`, and the Rust ABI are not changed by this feature; the contract here is satisfied entirely in `web3/src/HierarchyPanel/project.ts`.

---

## `toSeriesJson(project: Project): SeriesJson[]`

**Purity**: pure, deterministic, no I/O. Same project ⇒ identical `SeriesJson[]`.

**Output**: the youngest-first `SeriesJson[]` that `RunCommand.series` already expects (see `web3/src/protocol.ts`, unchanged). One `SeriesJson` per `ProjectElement`, in `elements` order.

**Per-element rules**:

| Project element | Emitted `SeriesJson` |
|-----------------|----------------------|
| `series` | `relation` = code(relation); `unboundedAbove` = element.unboundedAbove; `sp_xyz`/`sp_if` flattened across interfaces (interface array index → `sp_if` value); `ori_xyz`/`ori_pole` concatenated across interfaces (poles via `dipDirToPole`); `fault_point=[0,0,0]`, `fault_normal=[0,0,0]`; **no** `crossCut`. |
| `vein` | points/orientations from `footWall` (interface id 0) then `hangingWall` (id 1); `crossCut={kind:"vein",footWall:0,hangingWall:1}`; `relation`/`unboundedAbove` emitted as valid defaults (ignored by core for cross-cutting bodies). |
| `intrusion` | points/orientations from `envelopeInner` (id 0) then `envelopeOuter` (id 1); `crossCut={kind:"intrusion",envelopeInner:0,envelopeOuter:1}`; equal inner/outer ⇒ a single envelope. |

**Relation code** (mirrors `relation_from_code` / scenarios.ts): `"erode"→0`, `"onlap"→1`, `"conformable"→2`.

**Invariants the contract must preserve**:
- `sp_xyz.length === 3 * sp_if.length` (one interface id per point).
- `ori_xyz.length === ori_pole.length` and both `=== 3 * nOrientations`.
- Element order is preserved (chronology is youngest-first in both `elements` and the output array).
- Emits exactly what `scenarios.ts` emits today for the equivalent geology — a seeded-template project must compile to a `SeriesJson[]` that fits identically to the corresponding former scenario (this is the round-trip anchor; see quickstart).

**Precondition**: `validate(project).canApply === true`. `toSeriesJson` is only ever called from Apply, which is disabled when `canApply` is false. (A surface with zero points cannot produce a meaningful `SeriesJson`; the precondition check lives **before** the fit — see [research.md R10](../research.md).)

---

## `dipDirToPole(dipDeg: number, dipDirDeg: number): [number, number, number]`

The one bit of geology in the UI; the one place a sign error silently yields an upside-down field. Transcribed sign-for-sign from the design brief §5.3 and documented at the call site with its axis convention.

**Axis convention** (stated in the orientation-entry tooltip too): **X=East, Y=North, Z=Up; dip-direction is the azimuth of steepest descent, measured clockwise from North (+Y). The returned pole points UP-dip, out of the footwall.**

```ts
// geology convention: dip-direction is azimuth of steepest descent; pole points UP-dip.
// X=East, Y=North, Z=Up. dipDir clockwise from +Y. A wrong sign here silently flips the field.
function dipDirToPole(dipDeg: number, dipDirDeg: number): [number, number, number] {
  const d = (dipDeg * Math.PI) / 180;
  const a = (dipDirDeg * Math.PI) / 180;
  const nx = Math.sin(d) * Math.sin(a);
  const ny = Math.sin(d) * Math.cos(a);
  const nz = Math.cos(d);
  return [nx, ny, nz]; // already unit; points up out of the footwall
}
```

**Round-trip requirement**: the raw `dip`/`dipDir` are kept in `OrientationRow` and are **not** re-derived from the pole. Reopening the data modal shows exactly the values the user entered; conversion happens only inside `toSeriesJson`.

**Result shape**: a unit vector (`|pole| ≈ 1`), suitable for `SeriesJson.ori_pole` (3 floats per orientation).

---

## `validate(project: Project): ProjectValidation`

Pure, structural-only. See [data-model.md](../data-model.md) "Validation rules". Returns hard errors (block Apply, each with a `reason`), soft warnings (advisory), and `canApply`. **Does not** re-encode any core geological rule — it only checks preconditions the fit needs to even run (Constitution III; [research.md R10](../research.md)).

---

## Persistence helpers (in `project.ts`)

| Function | Contract |
|----------|----------|
| `saveProject(p: Project): void` | debounced (~500 ms) localStorage write, keyed by `p.name`. |
| `loadProject(): Project \| null` | read the autosaved project on startup; `null` ⇒ start from a blank/seed template. |
| `exportProjectJson(p: Project): void` | trigger a browser download of `<p.name>.geoproj.json` (the serialized `Project`). |
| `importProjectJson(file: File): Promise<Project>` | parse a `.geoproj.json` back into a `Project`; caller confirms-if-dirty before replacing. |

These touch only the `Project` shape and platform APIs (localStorage, Blob, file input). No core/wasm involvement.
