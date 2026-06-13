#!/usr/bin/env python3
"""dump_kernel.py — snapshot gempy's assembled cokriging K and b.

Monkeypatches gempy_engine's `kernel_reduction(cov, b, ...)` (the function that
receives the fully-assembled covariance matrix and RHS right before the solve)
to capture (cov, b, weights) on a tiny model, then dumps them as .npy alongside
the inputs and live kernel_options. This is the step-5 block-diff input — the
authority on where the geofield-rs assembly diverges from gempy.

Run:  cd validation && uv run python dump_kernel.py --out fixtures/kernel_tiny
"""

from __future__ import annotations
import argparse
import json
import sys
from pathlib import Path

import numpy as np

SCRIPT_DIR = Path(__file__).resolve().parent


def capture(model_builder):
    """Run model_builder() with kernel_reduction patched to snapshot K, b, w."""
    import gempy_engine.modules.solver.solver_interface as si

    snap = {}
    orig = si.kernel_reduction

    def patched(cov, b, kernel_options, x0=None):
        snap["cov"] = np.array(cov, dtype=np.float64, copy=True)
        snap["b"] = np.array(b, dtype=np.float64, copy=True)
        w = orig(cov, b, kernel_options, x0)
        snap["w"] = np.array(w, dtype=np.float64, copy=True)
        return w

    si.kernel_reduction = patched
    # The API layer imports the symbol into its own namespace; patch there too.
    try:
        import gempy_engine.API.interp_single._interp_scalar_field as isf
        if hasattr(isf, "kernel_reduction"):
            isf.kernel_reduction = patched
    except Exception as e:  # noqa: BLE001
        print(f"[warn] could not patch interp_scalar_field namespace: {e}", file=sys.stderr)

    try:
        data = model_builder()
    finally:
        si.kernel_reduction = orig

    if "cov" not in snap:
        raise RuntimeError(
            "kernel_reduction was never called — the patch missed the namespace "
            "gempy actually calls through. Inspect the import in "
            "_interp_scalar_field.py and patch that binding."
        )
    return data, snap


def build_tiny():
    """Smallest model matching geofield-rs's tiny_series / block_assembly.rs:
    2 surface points on 1 interface + 1 orientation, pole straight up."""
    import gempy as gp

    # gempy needs >=1 orientation and an interface; build from in-memory tables
    # via the pandas readers would require CSVs, so write tiny CSVs to a tmp dir.
    return gp  # placeholder; real builder set in main once we choose the model


def build_from_csv(sp_csv: Path, ori_csv: Path, extent, resolution, mapping):
    import gempy as gp

    def _builder():
        data = gp.create_geomodel(
            project_name="kernel_dump",
            extent=extent,
            resolution=resolution,
            refinement=1,
            importer_helper=gp.data.ImporterHelper(
                path_to_orientations=str(ori_csv),
                path_to_surface_points=str(sp_csv),
            ),
        )
        gp.map_stack_to_surfaces(gempy_model=data, mapping_object=mapping)
        gp.compute_model(data)
        return data

    return _builder


def extract_kernel_options(data) -> dict:
    ko = data.interpolation_options.kernel_options
    out = {}
    for attr in ("range", "c_o", "uni_degree", "i_res", "gi_res",
                 "number_dimensions", "n_uni_eq"):
        if hasattr(ko, attr):
            v = getattr(ko, attr)
            out[attr] = float(v) if isinstance(v, (int, float)) else v
    kf = getattr(ko, "kernel_function", None)
    out["kernel_function"] = getattr(kf, "name", str(kf)) if kf else None
    return out


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--out", type=Path, default=SCRIPT_DIR / "fixtures/kernel_tiny")
    args = ap.parse_args()
    outdir = args.out
    outdir.mkdir(parents=True, exist_ok=True)

    # Tiny model: 2 surface points on one interface + 1 orientation. Written as
    # CSVs so gempy's standard reader path is exercised (same as model1).
    sp_csv = outdir / "input_surface_points.csv"
    ori_csv = outdir / "input_orientations.csv"
    sp_csv.write_text("X,Y,Z,formation\n0,0,0,rockA\n1000,0,200,rockA\n")
    ori_csv.write_text("X,Y,Z,azimuth,dip,polarity,formation\n500,0,100,90,0,1,rockA\n")

    builder = build_from_csv(
        sp_csv, ori_csv,
        extent=[0, 1000, -100, 100, 0, 300],
        resolution=[2, 2, 2],
        mapping={"Series": ("rockA",)},
    )

    print("[1/2] computing + capturing kernel ...")
    data, snap = capture(builder)

    np.save(outdir / "gempy_K.npy", snap["cov"])
    np.save(outdir / "gempy_b.npy", snap["b"])
    np.save(outdir / "gempy_w.npy", snap["w"])
    (outdir / "kernel_options.json").write_text(
        json.dumps(extract_kernel_options(data), indent=2, default=str))
    (outdir / "series.json").write_text(json.dumps({"order": ["rockA"]}, indent=2))

    print(f"[ok] K {snap['cov'].shape}  b {snap['b'].shape}  w {snap['w'].shape}")
    print(f"[ok] dumped to {outdir}")
    # Quick echo so we can eyeball structure immediately.
    np.set_printoptions(precision=4, suppress=True, linewidth=140)
    print("K =\n", snap["cov"])
    print("b =", snap["b"].ravel())
    print("w =", snap["w"].ravel())


if __name__ == "__main__":
    main()
