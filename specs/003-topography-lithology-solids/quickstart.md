# Quickstart — Topography, Lithology Catalog & Solid Volumes

**Feature**: `003-topography-lithology-solids` · run + validation guide

This guide builds the artifacts and runs **both constitutional gates** (Principle I): the **engine**
invariant/parity tests and the **UI** Playwright behaviour specs. It does not duplicate the design —
see [data-model.md](./data-model.md) and the [contracts/](./contracts/).

> Unlike feature 002, this feature **changes Rust** (`crates/geofield/`). You must rebuild the engine
> wasm after any Rust change — `web3` consumes a built `geofield_bg.wasm`. The EUPL kernel
> (`geofield-kernel`) is **not** changed by this feature, but the build steps below rebuild both
> artifacts for completeness.

---

## Prerequisites

- Rust toolchain with the `wasm32-unknown-unknown` target and `wasm-pack` (the engine's existing
  setup — this feature adds no new toolchain requirement).
- Node + npm for the `web3/` harness; Playwright browsers installed (`npx playwright install` once).

---

## 1. Engine gate (Rust) — run first; it's the cheaper, faster signal

The engine changes (topography variant, clip-derived unit keep/drop, `DerivedView.units`, Air-skip
solids) are validated **natively** before touching the browser.

```bash
# from repo root
cargo test -p geofield            # unit derivation, inert-flag oracle, watertightness invariants
```

**Expected** (the new tests this feature adds — see [contracts/engine-abi.md](./contracts/engine-abi.md)):
- **Inert-flag oracle**: a series with `unbounded_above = false` and **no younger cap** still fills
  upward (proves the flag is dead — SC-009).
- **Preserved behaviour**: all pre-existing `present_unit_count` tests still pass (floor keeps its top
  unit; capped series drop theirs — now clip-driven, C2).
- **Reject-2nd-topography**: a second `add_topography` is refused (C1).
- **Watertightness invariant**: the multi-clip volume (topography-capped above + floor-bounded below
  + intrusion-punched) is crack-free, manifold, consistently wound (SC-005, C5) — the named test case.

### Native↔wasm parity (Principle V gate)

The new evaluation (topography clip) and enumeration (`DerivedView.units`) are wasm-reachable, so they
carry the parity obligation. Run the engine's existing parity harness/path on a scenario **with
topography** and confirm native and wasm agree to `f64` round-off (no new mechanism — reuse the
engine's established native↔wasm comparison; if a topography scenario is not yet covered, adding it is
part of this feature, SC-010).

---

## 2. Build the wasm artifacts for the harness

```bash
cd web3
npm install            # first time only
npm run wasm           # builds geofield_kernel.wasm (unchanged) + geofield_bg.wasm (this feature's engine)
```

`npm run wasm` runs both `wasm:kernel` and `wasm:engine`. **Re-run `npm run wasm:engine` after every
Rust change** to `crates/geofield/` or the harness will run stale engine code.

---

## 3. Compile gate (UI)

```bash
# in web3/
npm run build          # tsc -b + vite build — the compile gate (necessary, not sufficient)
```

This catches the `dto.ts` hand-mirror drift (the new `units` field must match `schema.rs`) and the new
catalog/render types. A green `build` is **not** "done" — the behaviour gate below is.

---

## 4. Behaviour gate (UI) — Playwright, the gate of record

```bash
# in web3/
npm run test:e2e       # playwright test — drives the real app via `npm run preview` @ :4173
```

**Expected — the four new specs** (see [contracts/catalog-and-render.md](./contracts/catalog-and-render.md)):

| Spec | Validates |
|------|-----------|
| `topography.spec.ts` | Add a Topography surface → the above-ground region reads as **Air** and older elements are clipped to the ground; the add-surface picker **disables Topography** when one exists. |
| `unit-ladder.spec.ts` | A conformable series with N contacts renders **N+1 unit rows** interleaved with N contact rows; a **unit** row renames, a **contact** row does not; the **"fills upward (floor)" control is absent**. |
| `lithology-catalog.spec.ts` | Air + Unknown are **fixed**; a vein contributes an **inferred** entry that renames with the body and is removed on delete; **user add/rename/recolour/delete** works; **two units → one lithology** (many-to-one); **export → import round-trips** names/colours/assignments. |
| `solids-toggle.spec.ts` | Toggle to **Solids** → closed solids coloured **by lithology**, **Air omitted**; toggle does **not trigger a refit**; OBJ export **grouped by lithology**, no Air group. |

The existing specs ([web3/e2e/](../../web3/e2e): smoke, round-trip, cross-cut, modal-validation,
persistence, qef-spike, flip-orientation) must stay green — the unit-ladder presentation change must
not regress them.

> **Drag/flaky substitution**: where a Playwright assertion would depend on a real drag (e.g. asserting
> the reorder guard keeps topography youngest), drive the wired resolver callback / assert on
> `dragResolver.ts` output instead, following the existing `drag-affordance` / `reorder` pattern — and
> say so in the test, not silently (Principle I).

---

## 5. End-to-end validation scenario (the whole feature in one pass)

A manual smoke (not the gate — the gates above are) to see it work end to end:

1. Start a project with a 3-contact conformable series + one intrusion (a seed template).
2. **Add a Topography** surface (type picker → Topography); author a few ground points + apply.
   → above-ground becomes Air; the series is clipped to the ground; the domain-box ceiling no longer
   bounds it.
3. Open the panel: see the **unit/contact ladder** (4 unit rows for the series, interleaved with 3
   contacts), youngest at top. The intrusion shows its inferred lithology.
4. Open the **Lithology Catalog**: Air + Unknown fixed; the intrusion's inferred entry present. Add a
   "Sandstone" user lithology. Assign two separate series units to "Sandstone" (many-to-one).
5. **Toggle to Solids**: watertight solids coloured by lithology, Air omitted — and **no refit runs**
   (the fit is unchanged).
6. **Export OBJ**: groups by lithology. **Export the project JSON, re-import it**: names, colours, and
   assignments round-trip.

---

## Gate summary (what "done" means here)

| Layer | Compile gate | Behaviour/correctness gate |
|-------|--------------|----------------------------|
| Engine (Rust) | `cargo build` | `cargo test -p geofield` (unit derivation, inert-flag oracle, **watertightness invariant**) + **native↔wasm parity** on the topography scenario |
| UI (web3) | `npm run build` (`tsc -b` + `vite build`) | `npm run test:e2e` (the four new Playwright specs + existing specs green) |

A green compile is necessary, never sufficient. If the environment cannot run a gate this pass, that is
a **logged, un-closed gap** (Principle IV), never a silent "verified."
