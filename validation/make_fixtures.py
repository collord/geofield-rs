#!/usr/bin/env python3
"""Build gempy reference models and dump fixtures for the Rust validation tests.

NOT part of the crate build. Requires a working `gempy` + `gempy_engine` install
(branch `main`). Run from the repo root:

    python validation/make_fixtures.py

For each reference model it writes a directory under
`validation/fixtures/<model>/` containing:

    kernel_options.json   live KernelOptions (range, c_o, i_res, gi_res,
                          uni_degree, nugget, number_dimensions) — the Rust
                          tests read THESE, never hardcoded values (KERNEL_MATH §7)
    input.json            the exact surface points / orientations / interface ids
                          fed to gempy, in input order, with the chosen reference
                          index per interface — so the Rust side assembles from
                          identical inputs
    transform.json        gempy's resolved center + isotropic scale (§5.1 check)
    K.npy                 the assembled cokriging matrix (gempy index order)
    b.npy                 the assembled RHS vector (gempy index order)
    weights.npy           the solved dual weights w
    index_layout.json     gempy's row meaning per index: a list of
                          {"kind": "grad"|"sp"|"uni", "dim": int|null,
                           "ori": int|null, "sp_inc": int|null, "uni": int|null}
                          so the Rust test can map gempy's order <-> ours and diff
                          block-by-block regardless of interleaving convention
    field.npy             scalar field on the committed grid (model >= dipping)
    grad.npy              scalar-field gradient on the grid (Nx3)
    grid.npy              the committed regular grid points (Nx3)
    litho.npy             lithology block on the grid
    provenance.json       gempy + gempy_engine versions / commit

IMPORTANT — the matrix/RHS/weights dump is the gate for KERNEL_MATH.md §6's
"diff the blocks before the field" insistence. gempy_engine does not expose the
raw assembled K/b on the public API, so this script installs a debug hook (see
`_capture_kernel`). If gempy's internals have moved, that hook is the one place
to fix; the fixture contract above stays stable for the Rust side.

The fixture format is intentionally explicit so the Rust test never has to guess
gempy's interleaving — `index_layout.json` carries the meaning of every row.
"""

from __future__ import annotations

import json
import os
import sys
from pathlib import Path

import numpy as np

FIXTURES = Path(__file__).resolve().parent / "fixtures"


def _provenance() -> dict:
    prov = {}
    try:
        import gempy

        prov["gempy_version"] = getattr(gempy, "__version__", "unknown")
    except Exception as e:  # noqa: BLE001
        prov["gempy_version"] = f"import-failed: {e}"
    try:
        import gempy_engine

        prov["gempy_engine_version"] = getattr(gempy_engine, "__version__", "unknown")
    except Exception as e:  # noqa: BLE001
        prov["gempy_engine_version"] = f"import-failed: {e}"
    return prov


def _capture_kernel():
    """Return a callable that, given a computed gempy solution / interpolation
    input, extracts (K, b, weights, index_layout, kernel_options).

    This is the brittle, version-coupled part. It is isolated here so that when
    gempy internals move, only this function changes. The expected shapes are
    documented in the module docstring.

    Implementation note: in gempy_engine `main` the dual-kriging system is
    assembled inside `gempy_engine.core.data.kernel_classes` / the solver in
    `gempy_engine.modules.solver`. The most robust capture is to monkeypatch the
    function that solves `K w = b` and snapshot its arguments. The exact symbol
    is intentionally left for the operator to confirm against their installed
    version — raise with a clear message rather than dump a wrong matrix.
    """
    raise NotImplementedError(
        "Wire _capture_kernel to your installed gempy_engine: monkeypatch the "
        "K w = b solver to snapshot K, b, w, and build index_layout from the "
        "interpolation input (n_orientations, n_surface_points per series, "
        "uni_degree). See the module docstring for the required fixture shapes. "
        "This is deliberately not guessed — a wrong matrix dump would silently "
        "invalidate the step-5 gate (KERNEL_MATH.md §6)."
    )


def _dump_common(model_dir: Path, kernel_options: dict, inp: dict, transform: dict):
    model_dir.mkdir(parents=True, exist_ok=True)
    (model_dir / "kernel_options.json").write_text(json.dumps(kernel_options, indent=2))
    (model_dir / "input.json").write_text(json.dumps(inp, indent=2))
    (model_dir / "transform.json").write_text(json.dumps(transform, indent=2))
    (model_dir / "provenance.json").write_text(json.dumps(_provenance(), indent=2))


# ---------------------------------------------------------------------------
# Reference models (progression per BUILD_geofield_rs.md):
#   tiny     : 1 interface (2 surface points) + 1 orientation  -> the §6 gate
#   flat3    : 3 flat layers
#   dipping  : dipping single series                            -> the §9 end-to-end
# ---------------------------------------------------------------------------

TINY_INPUT = {
    "surface_points": [
        {"xyz": [0.0, 0.0, 0.0], "interface_id": 0},
        {"xyz": [1.0, 0.0, 0.2], "interface_id": 0},
    ],
    "orientations": [
        {"xyz": [0.5, 0.0, 0.1], "pole": [0.0, 0.0, 1.0]},
    ],
    # reference point index per interface (first point seen, input order)
    "reference_per_interface": {"0": 0},
}


def build_tiny():
    """Smallest model — the block-by-block validation gate.

    Keep the input identical to `tests/block_assembly.rs::tiny_series` and the
    `assemble.rs` unit test. The Rust side reads `input.json` so the two cannot
    drift, but the values are mirrored here for a reviewer's convenience.
    """
    import gempy as gp  # noqa: F401  (import guarded; real build TBD by operator)

    model_dir = FIXTURES / "tiny"

    # TODO(operator): construct the gempy v3 model from TINY_INPUT, set
    # kernel_options (range, c_o, i_res, gi_res, uni_degree, nugget), call
    # gp.compute_model(..., compute_scalar_gradient=True), and use
    # _capture_kernel() to snapshot K/b/w/index_layout. Then:
    #
    #   np.save(model_dir / "K.npy", K)
    #   np.save(model_dir / "b.npy", b)
    #   np.save(model_dir / "weights.npy", w)
    #   (model_dir / "index_layout.json").write_text(json.dumps(index_layout))
    #   _dump_common(model_dir, kernel_options, TINY_INPUT, transform)
    #
    # Field/grid/litho are NOT needed for the tiny model (the gate is the matrix
    # and RHS only). They come with `dipping`.
    raise NotImplementedError("build_tiny: wire to installed gempy (see TODOs)")


def build_flat3():
    raise NotImplementedError("build_flat3: 3 flat layers — implement after tiny passes")


def build_dipping():
    raise NotImplementedError(
        "build_dipping: dipping single series + field/grad/grid/litho — step 9"
    )


MODELS = {
    "tiny": build_tiny,
    "flat3": build_flat3,
    "dipping": build_dipping,
}


def main(argv: list[str]) -> int:
    which = argv[1:] if len(argv) > 1 else ["tiny"]
    FIXTURES.mkdir(parents=True, exist_ok=True)
    failed = []
    for name in which:
        if name not in MODELS:
            print(f"unknown model {name!r}; known: {list(MODELS)}", file=sys.stderr)
            return 2
        try:
            print(f"building fixture: {name}")
            MODELS[name]()
            print(f"  -> {FIXTURES / name}")
        except NotImplementedError as e:
            print(f"  SKIP {name}: {e}", file=sys.stderr)
            failed.append(name)
    if failed:
        print(
            f"\n{len(failed)} model(s) not yet wired: {failed}\n"
            "Fill in the gempy build + _capture_kernel hook, then re-run.",
            file=sys.stderr,
        )
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))
