
"""
run_model1_validation.py
========================
End-to-end validation of `geofield-rs` against gempy_engine on the canonical
"Model 1 — horizontal stratigraphy" (jan_models/model1), a single-series,
non-faulted, two-formation model. This is the reference_model.rs target.

What it does
------------
1. Builds the gempy v3 model exactly as the tutorial does (downloads the two
   CSVs from the gempy_data repo).
2. Runs `gp.compute_model(..., compute_scalar_gradient=True)`.
3. Dumps a self-describing fixture directory:
     - input_surface_points.csv / input_orientations.csv (the raw inputs)
     - input_hashes.json         (SHA-256 of each input file — provenance)
     - kernel_options.json       (the LIVE kernel_options gempy actually used)
     - transform.json            (the rescaling center/scale gempy computed)
     - grid.json                 (regular-grid spec: extent + resolution)
     - gempy_scalar_field.npy     (scalar field on the regular grid)
     - gempy_scalar_gradient.npy  (gradient, if exposed)  [optional]
     - gempy_litho.npy            (lithology id block)
     - provenance.json           (gempy version/commit, timestamp)
4. If a geofield-rs binary is available, invokes it on the same grid and diffs
   the scalar fields (and gradient/litho if present).

The Rust tests can also consume this fixture directly (see BUILD_geofield_rs.md
step 9). This script is the Python half of validation/make_fixtures.py,
specialized to model1.

Usage
-----
    python run_model1_validation.py --out validation/fixtures/model1
    # then, once the Rust binary exists:
    python run_model1_validation.py --out validation/fixtures/model1 \
        --rust-bin ../target/release/geofield-cli

The Rust binary contract (CLI mode) is documented in `rust_cli_contract()`
below: it reads the fixture dir and writes rust_scalar_field.npy etc.
"""

from __future__ import annotations
import argparse
import hashlib
import json
import subprocess
import sys
from datetime import datetime, timezone
from pathlib import Path

import numpy as np


DATA_PATH = "https://raw.githubusercontent.com/cgre-aachen/gempy_data/master/"
ORI_URL = DATA_PATH + "data/input_data/jan_models/model1_orientations.csv"
SP_URL = DATA_PATH + "data/input_data/jan_models/model1_surface_points.csv"

EXTENT = [0, 1000, 0, 1000, 0, 1000]
# Regular grid we evaluate on for the diff. gempy v3 defaults to octree
# refinement; we force a dense regular grid so both sides evaluate the SAME
# points. 50^3 matches the legacy tutorial resolution and is cheap.
RESOLUTION = [50, 50, 50]


# ----------------------------------------------------------------------------
# gempy side
# ----------------------------------------------------------------------------
def build_and_compute(kernel: str = "cubic"):
    import gempy as gp
    from gempy_engine.core.data.kernel_classes.kernel_functions import AvailableKernelFunctions

    data = gp.create_geomodel(
        project_name="horizontal",
        extent=EXTENT,
        resolution=RESOLUTION,           # force a regular grid for the diff
        refinement=1,                    # we diff on the regular grid, not octree
                                         # meshes — keep refinement cheap
        importer_helper=gp.data.ImporterHelper(
            path_to_orientations=ORI_URL,
            path_to_surface_points=SP_URL,
        ),
    )

    gp.map_stack_to_surfaces(
        gempy_model=data,
        mapping_object={"Strat_Series": ("rock2", "rock1")},
    )

    # We want the scalar field gradient too, for the gradient_consistency check.
    try:
        data.interpolation_options.compute_scalar_gradient = True
    except Exception as e:
        print(f"[warn] could not set compute_scalar_gradient: {e}", file=sys.stderr)

    # Kernel choice (gempy AvailableKernelFunctions member). Recorded in the dumped
    # kernel_options.json, which geofield-cli reads — so both sides use the same kernel.
    data.interpolation_options.kernel_options.kernel_function = AvailableKernelFunctions[kernel]

    gp.compute_model(data)
    return gp, data


def extract_kernel_options(data) -> dict:
    """Pull the LIVE kernel_options. Never hardcode these — the Rust side reads
    whatever gempy actually used (range, c_o, i_res, gi_res, uni_degree, dims).

    Verified against gempy 2025.2.0: kernel_options exposes range, c_o, i_res,
    gi_res, uni_degree, n_uni_eq, number_dimensions, kernel_function."""
    ko = data.interpolation_options.kernel_options
    out = {}
    for attr in (
        "range", "c_o", "uni_degree", "i_res", "gi_res",
        "number_dimensions", "n_uni_eq",
    ):
        if hasattr(ko, attr):
            v = getattr(ko, attr)
            out[attr] = float(v) if isinstance(v, (int, float)) else v
    kf = getattr(ko, "kernel_function", None)
    out["kernel_function"] = getattr(kf, "name", str(kf)) if kf is not None else None

    # Nuggets are per-input-point in v3 (default 0.01 for orientations on these
    # tutorial models, not the kernel_options). Pull the per-element values and
    # record both the common value and the full arrays. The Rust CLI reads
    # grad_nugget / sp_nugget; emit those keys when uniform.
    sp_nuggets, ori_nuggets = [], []
    for el in data.structural_frame.structural_elements:
        sp = getattr(el, "surface_points", None)
        ori = getattr(el, "orientations", None)
        if sp is not None and getattr(sp, "nugget", None) is not None:
            sp_nuggets += list(np.atleast_1d(sp.nugget))
        if ori is not None and getattr(ori, "nugget", None) is not None:
            ori_nuggets += list(np.atleast_1d(ori.nugget))

    def _uniform(vals):
        vals = [float(v) for v in vals]
        return vals[0] if vals and all(abs(v - vals[0]) < 1e-15 for v in vals) else None

    sp_u, ori_u = _uniform(sp_nuggets), _uniform(ori_nuggets)
    if sp_u is not None:
        out["sp_nugget"] = sp_u
    if ori_u is not None:
        out["grad_nugget"] = ori_u
    out["_sp_nuggets"] = [float(v) for v in sp_nuggets]
    out["_ori_nuggets"] = [float(v) for v in ori_nuggets]
    if sp_u is None or ori_u is None:
        out["_nugget_note"] = (
            "non-uniform nuggets detected; geofield-rs uses a single grad/sp "
            "nugget — parity will be approximate until per-point nuggets are "
            "supported. Inspect _sp_nuggets/_ori_nuggets."
        )
    return out


def extract_transform(data) -> dict:
    """The coordinate rescaling gempy applied (KERNEL_MATH.md §5.1). geofield-rs
    derives its own Transform from the inputs and should reproduce this exactly;
    we dump gempy's for an independent cross-check (center = −position, isotropic
    scale). Verified against gempy 2025.2.0: data.input_transform exposes
    position, scale, rotation, isometric_scale."""
    out = {}
    t = getattr(data, "input_transform", None)
    if t is not None:
        for attr in ("position", "scale", "rotation", "isometric_scale"):
            if hasattr(t, attr):
                v = getattr(t, attr)
                out[attr] = np.asarray(v).tolist() if hasattr(v, "__len__") else float(v)
        # Derived: center = −position (gempy translates by −center).
        if "position" in out:
            out["center"] = [-p for p in out["position"]]
    if not out:
        out["_note"] = "transform not located on data.input_transform; inspect manually"
    return out


def extract_grid_points(data) -> np.ndarray:
    """The EXACT world-coordinate grid points gempy evaluated, in gempy's order.
    Dumping these (rather than re-deriving a grid in Rust) removes grid-ordering
    convention as a source of spurious diff. Verified: data.grid.regular_grid
    .values is (N,3), iterating z fastest then y then x (cell centers)."""
    rg = data.grid.regular_grid
    vals = np.asarray(rg.values, dtype=np.float64)
    assert vals.ndim == 2 and vals.shape[1] == 3, f"unexpected grid shape {vals.shape}"
    return vals


def extract_grid_meta(data) -> dict:
    rg = data.grid.regular_grid
    return {
        "extent": np.asarray(rg.extent).tolist(),
        "resolution": np.asarray(rg.resolution).tolist(),
        "n_points": int(np.prod(rg.resolution)),
        "ordering": "rows of grid.npy; z fastest, then y, then x (gempy cell centers)",
        "note": "Rust evaluates at grid.npy points directly — order is authoritative.",
    }


def extract_scalar_field(data) -> np.ndarray:
    """Scalar field on the regular grid, flattened to (N,). Verified:
    raw_arrays.scalar_field_matrix is (n_series, N); single series -> row 0."""
    raw = data.solutions.raw_arrays
    sf = np.asarray(raw.scalar_field_matrix, dtype=np.float64)
    if sf.ndim == 2:
        if sf.shape[0] != 1:
            print(f"[warn] scalar_field_matrix has {sf.shape[0]} series rows; "
                  f"using row 0 (single-series scope)", file=sys.stderr)
        sf = sf[0]
    return np.ascontiguousarray(sf)


def extract_litho(data) -> np.ndarray:
    """Lithology id block, (N,). Verified: raw_arrays.lith_block."""
    return np.asarray(data.solutions.raw_arrays.lith_block, dtype=np.float64)


def extract_gradient(data):
    """Optional. Verified: raw_arrays.gradient exists but is often EMPTY ((0,))
    on these models even with compute_scalar_gradient=True. Return None for
    empty so the diff is skipped rather than comparing against nothing."""
    g = getattr(data.solutions.raw_arrays, "gradient", None)
    if g is None:
        return None
    g = np.asarray(g)
    return g if g.size > 0 else None


def extract_isovalues(data):
    """Recovered scalar value at each interface's surface points — the isovalue
    magnitudes the Rust side must match (KERNEL_MATH.md §7.4). Verified:
    raw_arrays.scalar_field_at_surface_points, shape (n_series, n_interfaces)."""
    v = getattr(data.solutions.raw_arrays, "scalar_field_at_surface_points", None)
    return None if v is None else np.asarray(v, dtype=np.float64).tolist()


def series_order(data) -> list:
    """Ordered formation names (bottom->top within the series), so the Rust CLI
    maps formation -> interface id identically. Mirrors the map_stack mapping."""
    names = []
    for el in data.structural_frame.structural_elements:
        nm = getattr(el, "name", None)
        if nm is not None:
            names.append(str(nm))
    return names


# ----------------------------------------------------------------------------
# fixtures
# ----------------------------------------------------------------------------
def sha256_of_url_text(text: str) -> str:
    return hashlib.sha256(text.encode("utf-8")).hexdigest()


def fetch_text(url: str) -> str:
    import urllib.request
    with urllib.request.urlopen(url) as r:
        return r.read().decode("utf-8")


def write_fixtures(outdir: Path, gp, data):
    outdir.mkdir(parents=True, exist_ok=True)

    # raw inputs + hashes (provenance — see hash_orientations discussion)
    sp_text = fetch_text(SP_URL)
    ori_text = fetch_text(ORI_URL)
    (outdir / "input_surface_points.csv").write_text(sp_text)
    (outdir / "input_orientations.csv").write_text(ori_text)
    hashes = {
        "surface_points_sha256": sha256_of_url_text(sp_text),
        "orientations_sha256": sha256_of_url_text(ori_text),
        "surface_points_url": SP_URL,
        "orientations_url": ORI_URL,
    }
    (outdir / "input_hashes.json").write_text(json.dumps(hashes, indent=2))

    (outdir / "kernel_options.json").write_text(
        json.dumps(extract_kernel_options(data), indent=2, default=str)
    )
    (outdir / "transform.json").write_text(
        json.dumps(extract_transform(data), indent=2, default=str)
    )
    (outdir / "grid.json").write_text(json.dumps(extract_grid_meta(data), indent=2))

    # The exact grid points the Rust CLI evaluates at (order-authoritative).
    grid_pts = extract_grid_points(data)
    np.save(outdir / "grid.npy", grid_pts)
    print(f"[ok] grid points {grid_pts.shape}")

    # Ordered formation list so the Rust CLI assigns interface ids identically.
    order = series_order(data)
    (outdir / "series.json").write_text(json.dumps({"order": order}, indent=2))
    print(f"[ok] series order {order}")

    sf = extract_scalar_field(data)
    np.save(outdir / "gempy_scalar_field.npy", sf)
    print(f"[ok] scalar field {sf.shape} dtype={sf.dtype}")

    try:
        lith = extract_litho(data)
        np.save(outdir / "gempy_litho.npy", lith)
        print(f"[ok] litho block {lith.shape}")
    except Exception as e:
        print(f"[warn] litho: {e}", file=sys.stderr)

    grad = extract_gradient(data)
    if grad is not None:
        np.save(outdir / "gempy_scalar_gradient.npy", grad)
        print(f"[ok] gradient {grad.shape}")
    else:
        print("[warn] gradient empty/not exposed; gradient diff will be skipped")

    isovals = extract_isovalues(data)
    prov = {
        "gempy_version": getattr(gp, "__version__", "unknown"),
        "generated_utc": datetime.now(timezone.utc).isoformat(),
        "model": "jan_models/model1 horizontal stratigraphy",
        "series": {"Strat_Series": series_order(data)},
        "gempy_isovalues_at_surface_points": isovals,
        "note": "interface points mark the BOTTOM of a layer (gempy convention)",
    }
    (outdir / "provenance.json").write_text(json.dumps(prov, indent=2))
    print(f"[ok] fixtures written to {outdir}")


# ----------------------------------------------------------------------------
# rust side
# ----------------------------------------------------------------------------
def rust_cli_contract() -> str:
    return (
        "geofield-cli contract  (build: cargo build --release --features cli):\n"
        "  geofield-cli --fixture <DIR>\n"
        "  reads:  <DIR>/input_surface_points.csv  (X,Y,Z,formation)\n"
        "          <DIR>/input_orientations.csv     (X,Y,Z,azimuth,dip,polarity,formation)\n"
        "          <DIR>/kernel_options.json        (range,c_o,i_res,gi_res,uni_degree,nuggets)\n"
        "          <DIR>/series.json                ({\"order\": [formation, ...]})\n"
        "          <DIR>/grid.npy                   ((N,3) world points; ORDER AUTHORITATIVE)\n"
        "  writes: <DIR>/rust_scalar_field.npy   ((N,) same order as gempy_scalar_field.npy)\n"
        "          <DIR>/rust_litho.npy           ((N,) unit ids)\n"
        "          <DIR>/rust_isovalues.json      (recovered per-interface isovalues)\n"
        "  Reads LIVE kernel options + the exact grid points — never hardcodes,\n"
        "  never re-derives the grid (removes ordering as a diff source).\n"
    )


def run_rust(rust_bin: Path, outdir: Path):
    print(f"[run] {rust_bin} --fixture {outdir}")
    subprocess.run([str(rust_bin), "--fixture", str(outdir)], check=True)


def diff_fields(outdir: Path):
    gempy_sf = np.load(outdir / "gempy_scalar_field.npy")
    rust_path = outdir / "rust_scalar_field.npy"
    if not rust_path.exists():
        print(f"[skip] {rust_path} not found — run with --rust-bin once built")
        return False

    rust_sf = np.load(rust_path)
    if rust_sf.shape != gempy_sf.shape:
        print(f"[FAIL] shape mismatch: gempy {gempy_sf.shape} vs rust {rust_sf.shape}")
        return False

    # The scalar field is a potential — defined up to an affine (scale+offset)
    # transform per series. A raw elementwise diff can spuriously fail if the
    # two implementations level/scale the field differently even when the
    # GEOMETRY (isosurfaces, hence litho) is identical. Report BOTH:
    #   (a) raw relative diff (strict — expect pass if conventions match)
    #   (b) affine-invariant diff (fit rust = m*gempy + b, then residual)
    abs_err = np.abs(rust_sf - gempy_sf)
    denom = np.maximum(np.abs(gempy_sf), 1e-12)
    rel = abs_err / denom
    print(f"[raw]    max|Δ|={abs_err.max():.3e}  max rel={rel.max():.3e}  "
          f"mean rel={rel.mean():.3e}")

    # affine-invariant
    A = np.vstack([gempy_sf.ravel(), np.ones(gempy_sf.size)]).T
    (m, b), *_ = np.linalg.lstsq(A, rust_sf.ravel(), rcond=None)
    resid = rust_sf.ravel() - (m * gempy_sf.ravel() + b)
    rms = np.sqrt(np.mean(resid**2)) / (gempy_sf.std() + 1e-12)
    print(f"[affine] rust ≈ {m:.6f}*gempy + {b:.6e}   normalized RMS resid={rms:.3e}")
    print("         (m≈1,b≈0 → conventions match; m≠1/b≠0 but tiny RMS →")
    print("          field-invariant scale/level diff, geometry still identical)")

    raw_pass = rel.max() < 1e-6
    affine_pass = rms < 1e-6
    print(f"[raw]    {'PASS' if raw_pass else 'FAIL'} @ 1e-6 rel")
    print(f"[affine] {'PASS' if affine_pass else 'FAIL'} @ 1e-6 normRMS")

    # litho diff if both present — this is the geometry-level truth
    g_lith_p = outdir / "gempy_litho.npy"
    r_lith_p = outdir / "rust_litho.npy"
    if g_lith_p.exists() and r_lith_p.exists():
        gl, rl = np.load(g_lith_p), np.load(r_lith_p)
        if gl.shape == rl.shape:
            agree = np.mean(gl.round() == rl.round())
            print(f"[litho]  voxel agreement: {agree*100:.4f}%  "
                  f"({'PASS' if agree > 0.9999 else 'FAIL'} @ 99.99%)")

    return raw_pass or affine_pass


# Repo root = parent of validation/ (this file's dir), so paths work whether the
# script is invoked from the repo root or from validation/ (uv run).
SCRIPT_DIR = Path(__file__).resolve().parent
REPO_ROOT = SCRIPT_DIR.parent


def default_rust_bin() -> Path | None:
    """Auto-locate the built geofield-cli (release preferred, then debug)."""
    for profile in ("release", "debug"):
        cand = REPO_ROOT / "target" / profile / "geofield-cli"
        if cand.exists():
            return cand
    return None


def diff_isovalues(outdir: Path):
    """Compare recovered isovalue magnitudes (the §7.4 check) when both sides
    expose them. gempy: provenance.json gempy_isovalues_at_surface_points;
    rust: rust_isovalues.json."""
    rust_p = outdir / "rust_isovalues.json"
    prov_p = outdir / "provenance.json"
    if not (rust_p.exists() and prov_p.exists()):
        return
    rust = json.loads(rust_p.read_text())
    prov = json.loads(prov_p.read_text())
    gempy_iso = prov.get("gempy_isovalues_at_surface_points")
    rust_iso = [iv["isovalue"] for iv in rust.get("isovalues", [])]
    print(f"[isoval] gempy={gempy_iso}  rust={rust_iso}")
    print("         (these are the magnitudes the §7.4 RHS-scaling open item is "
          "about; equal => RHS scaling matches, not just field shape)")


# ----------------------------------------------------------------------------
def main():
    ap = argparse.ArgumentParser(description=__doc__,
                                 formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--out", type=Path, default=REPO_ROOT / "validation/fixtures/model1")
    ap.add_argument("--rust-bin", type=Path, default=None,
                    help="path to geofield-cli (default: auto-locate target/{release,debug}); "
                         "pass to run it and diff")
    ap.add_argument("--no-rust", action="store_true",
                    help="only write gempy fixtures; skip the Rust diff")
    ap.add_argument("--kernel", default="cubic",
                    choices=["cubic", "matern_5_2", "exponential"],
                    help="gempy kernel_function (default cubic, the original gate); "
                         "pair a non-cubic kernel with its own --out dir, e.g. "
                         "fixtures/model1_matern_5_2")
    ap.add_argument("--show-contract", action="store_true",
                    help="print the Rust CLI contract and exit")
    args = ap.parse_args()

    if args.show_contract:
        print(rust_cli_contract())
        return

    print("[1/3] building gempy model + computing ...")
    gp, data = build_and_compute(args.kernel)

    print("[2/3] writing fixtures ...")
    write_fixtures(args.out, gp, data)

    print("[3/3] rust diff ...")
    rust_bin = args.rust_bin or (None if args.no_rust else default_rust_bin())
    if rust_bin and rust_bin.exists():
        run_rust(rust_bin, args.out)
        ok = diff_fields(args.out)
        diff_isovalues(args.out)
        sys.exit(0 if ok else 1)
    else:
        if not args.no_rust:
            print("    geofield-cli not found — build it with:")
            print("      cargo build --release --features cli")
        print("    fixtures are ready for the Rust tests / a later --rust-bin run.")
        print(rust_cli_contract())


if __name__ == "__main__":
    main()
