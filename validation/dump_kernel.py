#!/usr/bin/env python3
"""dump_kernel.py — snapshot gempy's assembled cokriging K and b.

Monkeypatches gempy_engine's `kernel_reduction(cov, b, ...)` (the function that
receives the fully-assembled covariance matrix and RHS right before the solve)
to capture (cov, b, weights) on a tiny model, then dumps them as .npy alongside
the inputs and live kernel_options. This is the step-5 block-diff input — the
authority on where the geofield-rs assembly diverges from gempy.

Run:  cd validation && uv run python dump_kernel.py --out fixtures/kernel_tiny

Kernel choice: `--kernel {cubic,matern_5_2,exponential}` selects gempy's
`AvailableKernelFunctions` member (default cubic, the original gate). `--model multi`
uses a 3-orientation / 2-interface model so the CG_CG off-diagonal (orientation pairs
at different locations — where kernels differ most, incl. gempy's exponential r⁴
divisor, KERNEL_MATH.md §1c.4) is actually exercised; the tiny model's single
orientation never reaches it.
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


def build_from_csv(sp_csv: Path, ori_csv: Path, extent, resolution, mapping, kernel="cubic"):
    import gempy as gp
    from gempy_engine.core.data.kernel_classes.kernel_functions import AvailableKernelFunctions

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
        data.interpolation_options.kernel_options.kernel_function = AvailableKernelFunctions[kernel]
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

    # Live per-element nuggets (the CLI reads these — never hardcode). On these
    # models orientation nugget defaults to 0.01, surface points to ~2e-5.
    sp_n, ori_n = [], []
    for el in data.structural_frame.structural_elements:
        sp = getattr(el, "surface_points", None)
        ori = getattr(el, "orientations", None)
        if sp is not None and getattr(sp, "nugget", None) is not None:
            sp_n += list(np.atleast_1d(sp.nugget))
        if ori is not None and getattr(ori, "nugget", None) is not None:
            ori_n += list(np.atleast_1d(ori.nugget))

    def _uniform(vals):
        vals = [float(v) for v in vals]
        return vals[0] if vals and all(abs(v - vals[0]) < 1e-15 for v in vals) else None

    sp_u, ori_u = _uniform(sp_n), _uniform(ori_n)
    if sp_u is not None:
        out["sp_nugget"] = sp_u
    if ori_u is not None:
        out["grad_nugget"] = ori_u
    return out


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--out", type=Path, default=SCRIPT_DIR / "fixtures/kernel_tiny")
    ap.add_argument("--kernel", default="cubic",
                    choices=["cubic", "matern_5_2", "exponential"],
                    help="gempy AvailableKernelFunctions member (default cubic)")
    ap.add_argument("--model", default="tiny", choices=["tiny", "multi"],
                    help="tiny: 2 points + 1 orientation; multi: 5 points on 2 "
                         "interfaces + 3 orientations (exercises CG_CG off-diagonal)")
    args = ap.parse_args()
    outdir = args.out
    outdir.mkdir(parents=True, exist_ok=True)

    # Tiny model: 2 surface points on one interface + 1 orientation. Written as
    # CSVs so gempy's standard reader path is exercised (same as model1).
    sp_csv = outdir / "input_surface_points.csv"
    ori_csv = outdir / "input_orientations.csv"
    if args.model == "tiny":
        sp_csv.write_text("X,Y,Z,formation\n0,0,0,rockA\n1000,0,200,rockA\n")
        ori_csv.write_text("X,Y,Z,azimuth,dip,polarity,formation\n500,0,100,90,0,1,rockA\n")
        extent, formations = [0, 1000, -100, 100, 0, 300], ("rockA",)
    else:
        # Rows are written in gempy's stack order (rockB above rockA, so rockB first):
        # gempy groups its rows by surface in stack order, while geofield-cli keeps file
        # order — writing them pre-sorted keeps the two row layouts identical.
        # The data span is 600 on EVERY axis: current gempy rescales anisotropically
        # (and transforms the poles) when the spans differ, which geofield's isotropic
        # Transform (KERNEL_MATH.md §5.1) does not reproduce — a known, separate
        # divergence this kernel gate must not be confounded by.
        sp_csv.write_text(
            "X,Y,Z,formation\n"
            "100,-200,500,rockB\n550,250,600,rockB\n"
            "0,-300,100,rockA\n400,300,0,rockA\n600,0,150,rockA\n")
        ori_csv.write_text(
            "X,Y,Z,azimuth,dip,polarity,formation\n"
            "300,0,550,80,8,1,rockB\n"
            "100,-100,90,90,10,1,rockA\n"
            "500,200,80,100,15,1,rockA\n")
        extent, formations = [-100, 700, -400, 400, -100, 700], ("rockB", "rockA")
    n_interfaces = len(formations)

    builder = build_from_csv(
        sp_csv, ori_csv,
        extent=extent,
        resolution=[2, 2, 2],
        mapping={"Series": formations},
        kernel=args.kernel,
    )

    print("[1/2] computing + capturing kernel ...")
    data, snap = capture(builder)

    np.save(outdir / "gempy_K.npy", snap["cov"])
    np.save(outdir / "gempy_b.npy", snap["b"])
    np.save(outdir / "gempy_w.npy", snap["w"])
    (outdir / "kernel_options.json").write_text(
        json.dumps(extract_kernel_options(data), indent=2, default=str))
    (outdir / "series.json").write_text(json.dumps({"order": list(formations)}, indent=2))

    print(f"[ok] K {snap['cov'].shape}  b {snap['b'].shape}  w {snap['w'].shape}")
    print(f"[ok] dumped to {outdir}")
    np.set_printoptions(precision=4, suppress=True, linewidth=140)

    # Block layout (single series, degree-1): n_grad = 3*n_ori gradient rows
    # (component-major gx..,gy..,gz..), then n_sp increment rows, then 3 drift.
    n = snap["cov"].shape[0]
    n_ori = _count_csv_rows(ori_csv)
    n_sp_total = _count_csv_rows(sp_csv)
    n_grad = 3 * n_ori
    n_sp = n_sp_total - n_interfaces
    layout = {"n": n, "n_grad": n_grad, "n_sp": n_sp, "n_uni": 3,
              "n_ori": n_ori, "n_sp_total": n_sp_total}
    print(f"[layout] {layout}")
    assert n_grad + n_sp + 3 == n, f"layout {layout} != dim {n}"

    # Build the geofield-rs K/b on the SAME inputs via the CLI.
    rust_bin = default_rust_bin()
    if rust_bin is None:
        print("[warn] geofield-cli not built; skipping block diff. "
              "Build: cargo build --release --features cli")
        return
    import subprocess
    subprocess.run([str(rust_bin), "--fixture", str(outdir), "--dump-kernel"], check=True)
    rust_K = np.load(outdir / "rust_K.npy")
    rust_b = np.load(outdir / "rust_b.npy").ravel()
    gK, gb = snap["cov"], snap["b"].ravel()

    block_ratio_table(gK, gb, rust_K, rust_b, n_grad, n_sp)


def _count_csv_rows(path: Path) -> int:
    lines = [ln for ln in path.read_text().splitlines() if ln.strip()]
    return max(0, len(lines) - 1)  # minus header


def default_rust_bin():
    for profile in ("release", "debug"):
        cand = REPO_ROOT / "target" / profile / "geofield-cli"
        if cand.exists():
            return cand
    return None


REPO_ROOT = SCRIPT_DIR.parent


def _block_ratio(g, m, label):
    """Report the ratio mine/gempy over a block, only where gempy is nonzero."""
    mask = np.abs(g) > 1e-12
    if not mask.any():
        same = np.allclose(m, 0, atol=1e-10)
        return f"  {label:<14} gempy all~0; mine {'~0 OK' if same else 'NONZERO!'}"
    r = m[mask] / g[mask]
    # also the raw max abs diff (the true gate)
    d = np.abs(m - g).max()
    return (f"  {label:<14} ratio[min,max]=[{r.min():.4f},{r.max():.4f}]  "
            f"max|Δ|={d:.3e}")


def block_ratio_table(gK, gb, mK, mb, n_grad, n_sp):
    g, s = n_grad, n_grad + n_sp  # block boundaries
    print("\n=== per-block ratio (mine/gempy) + max|Δ| — GATE: ratio→1.0, Δ→~1e-4 ===")
    print(_block_ratio(gK[:g, :g],   mK[:g, :g],   "CG_CG"))
    print(_block_ratio(gK[:g, g:s],  mK[:g, g:s],  "CG_SP"))
    print(_block_ratio(gK[g:s, g:s], mK[g:s, g:s], "SP_SP"))
    print(_block_ratio(gK[:g, s:],   mK[:g, s:],   "U_G"))
    print(_block_ratio(gK[g:s, s:],  mK[g:s, s:],  "U_SP"))
    print(_block_ratio(gb,           mb,           "b (RHS)"))
    full = np.abs(mK - gK).max()
    print(f"  {'FULL K':<14} max|Δ|={full:.3e}   "
          f"{'PASS' if full < 1e-4 else 'FAIL'} @ 1e-4")
    bd = np.abs(mb - gb).max()
    print(f"  {'FULL b':<14} max|Δ|={bd:.3e}   "
          f"{'PASS' if bd < 1e-4 else 'FAIL'} @ 1e-4")


if __name__ == "__main__":
    main()
