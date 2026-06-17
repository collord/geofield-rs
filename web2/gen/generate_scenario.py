#!/usr/bin/env python3
"""Generate the scale-test scenario for the web2/ harness.

Geology (oldest -> newest):
  * 3 intact sedimentary contacts, ~400 ft apart, gently undulating (sum-of-sines).
    Modeled as ONE conformable series with 3 interface ids (4 lithology units).
  * an erosional surface dipping ~10 deg, with basalt filling ABOVE it.
    Modeled as a younger ERODE series with a single contact (the erosion plane).
  * a late fault (strike 20 deg, dip 80 deg) offsetting the whole column 100 ft
    down-to-the-east. Applied to BOTH series (it post-dates everything).

Sampling: 3000 vertical boreholes on a 300 ft grid jittered by N(0, 40 ft). Each hole
logs a surface point at every geological contact it intersects, tagged by interface id.
A sparse subset also yields orientation (dip) data from the local analytic gradient.

Output: a JSON the harness loads, with per-series {sp_xyz, sp_if, ori_xyz, ori_pole},
the relation, the fault plane, and the domain extent. Run:

    python3 web2/gen/generate_scenario.py --out web2/public/scenario.json
"""

import argparse
import json
import math
import random

# --- Domain ---------------------------------------------------------------
# Z increases UPWARD (elevation). The horizontal extent is DERIVED from the
# borehole count + grid spacing so a 300 ft grid actually yields ~3000 holes:
# side = ceil(sqrt(n)) * grid. For n=3000, grid=300 -> ~55x55 grid, ~16,500 ft sq.
# (3000 holes on a literal 3000 ft square at 300 ft spacing is only 100 holes;
# honoring the 3000-hole target means a proportionally larger map.) Set by configure().
EXTENT = {"x": (0.0, 3000.0), "y": (0.0, 3000.0), "z": (0.0, 2000.0)}


# Map side length (ft), set by configure_extent. Geology below is map-relative so
# features stay in-domain regardless of how many boreholes were requested.
SIDE = 3000.0

# --- Sediment contacts (pre-fault, "geological" elevation) ----------------
# Three undulating contacts ~400 ft apart, centered in the column. Interface ids
# 0 (deepest) .. 2 (shallowest); the conformable series therefore has 4 units.
SED_THICKNESS = 400.0  # vertical separation between adjacent contacts (ft)
SED_BASES = [600.0, 1000.0, 1400.0]  # mean elevation of each contact (reset in configure)
SED_IDS = [0, 1, 2]

# --- Erosional surface ----------------------------------------------------
ERO_DIP_DEG = 10.0
ERO_Z0 = 1300.0  # elevation of the erosion plane at x=0 (reset in configure)
ERO_ID = 10  # interface id for the erosion/basalt contact


def configure_extent(n, grid):
    """Size the map so a `grid`-spaced pattern yields ~n holes, and rescale the
    geology to that map so all features stay inside the z-range."""
    global SIDE, ERO_Z0
    side = math.ceil(math.sqrt(n)) * grid
    SIDE = side
    EXTENT["x"] = (0.0, side)
    EXTENT["y"] = (0.0, side)
    # Deepen the column proportionally so the dipping erosion surface + the stack +
    # the fault offset all fit. The 10-deg erosion plane drops tan(10)*side across
    # the map; size z to contain that plus the stack and headroom.
    ero_drop = math.tan(math.radians(ERO_DIP_DEG)) * side
    z_top = max(2000.0, ero_drop + 3 * SED_THICKNESS + 600.0)
    EXTENT["z"] = (0.0, z_top)
    # Sediment stack centered in the lower-middle of the column.
    base0 = 0.30 * z_top
    SED_BASES[0], SED_BASES[1], SED_BASES[2] = base0, base0 + SED_THICKNESS, base0 + 2 * SED_THICKNESS
    # Erosion plane anchored high at x=0 so, dipping east, it cuts down THROUGH the
    # sediment stack within the map (oblique unconformity).
    ERO_Z0 = base0 + 2.5 * SED_THICKNESS + 0.5 * ero_drop
    # Center the fault on the map.
    FAULT_POINT[0] = side / 2.0
    FAULT_POINT[1] = side / 2.0
    FAULT_POINT[2] = z_top / 2.0


def _undulation(x, y):
    """A gentle sum-of-sines z-perturbation (ft), shared by the 3 conformable
    contacts so they stay parallel (intact stack). Wavelengths scale with the map."""
    return (
        0.05 * SED_THICKNESS * math.sin(2 * math.pi * x / (0.7 * SIDE))
        + 0.04 * SED_THICKNESS * math.sin(2 * math.pi * y / (0.55 * SIDE) + 1.0)
        + 0.02 * SED_THICKNESS * math.sin(2 * math.pi * (x + y) / (0.4 * SIDE))
    )


def sed_contact_z(k, x, y):
    """Pre-fault elevation of sediment contact k at (x, y)."""
    return SED_BASES[k] + _undulation(x, y)


def ero_surface_z(x, y):
    """Pre-fault elevation of the erosion surface at (x, y). Dips east at ~10 deg."""
    return ERO_Z0 - math.tan(math.radians(ERO_DIP_DEG)) * x


# --- Late fault -----------------------------------------------------------
# Strike 020, dip 80, offset 100 ft DOWN-TO-THE-EAST. The fault plane passes
# through the scene center. "Down to the east" = the east (hanging-wall, +x) side
# drops by 100 ft relative to the west side.
FAULT_STRIKE_DEG = 20.0
FAULT_DIP_DEG = 80.0
FAULT_OFFSET = 100.0
FAULT_POINT = [1500.0, 1500.0, 1000.0]  # a point on the fault plane (scene center)


def _fault_normal():
    """Unit normal to the fault plane from strike/dip.

    Strike is the azimuth of the horizontal line in the plane (measured CW from
    north = +y). Dip is measured down from horizontal, toward dip-direction =
    strike + 90 deg. The normal points toward the up-dip/hanging side."""
    strike = math.radians(FAULT_STRIKE_DEG)
    dip = math.radians(FAULT_DIP_DEG)
    # Dip direction azimuth (CW from north): strike + 90.
    dip_az = strike + math.pi / 2.0
    # Horizontal component of the normal points in the dip direction; vertical
    # component from the dip angle. Azimuth -> (x=east, y=north) components.
    nx = math.sin(dip_az) * math.sin(dip)
    ny = math.cos(dip_az) * math.sin(dip)
    nz = math.cos(dip)
    n = math.sqrt(nx * nx + ny * ny + nz * nz)
    return [nx / n, ny / n, nz / n]


FAULT_NORMAL = _fault_normal()


def _east_side(x, y, z):
    """True if (x,y,z) is on the EAST (down-thrown, hanging-wall) side of the fault,
    i.e. the +normal side projected to the east. We use the sign of the in-plane
    east offset: points with (p - point) . normal > 0 are the hanging side."""
    px, py, pz = x - FAULT_POINT[0], y - FAULT_POINT[1], z - FAULT_POINT[2]
    d = px * FAULT_NORMAL[0] + py * FAULT_NORMAL[1] + pz * FAULT_NORMAL[2]
    return d > 0.0


def apply_fault_offset(x, y, z_geol):
    """Given a pre-fault contact elevation z_geol at (x,y), return the OBSERVED
    elevation after the late fault. The east (hanging) side is dropped by
    FAULT_OFFSET. We classify the point by its (x,y) at the contact elevation."""
    if _east_side(x, y, z_geol):
        return z_geol - FAULT_OFFSET
    return z_geol


# --- Borehole sampling ----------------------------------------------------

def gen_boreholes(n_target, grid_spacing, jitter_sigma, seed):
    """Vertical borehole collars on a jittered grid covering the extent, ~n_target
    holes. Returns a list of (x, y)."""
    rng = random.Random(seed)
    x0, x1 = EXTENT["x"]
    y0, y1 = EXTENT["y"]
    nx = int(round((x1 - x0) / grid_spacing))
    ny = int(round((y1 - y0) / grid_spacing))
    collars = []
    for i in range(nx):
        for j in range(ny):
            x = x0 + (i + 0.5) * grid_spacing + rng.gauss(0, jitter_sigma)
            y = y0 + (j + 0.5) * grid_spacing + rng.gauss(0, jitter_sigma)
            x = min(max(x, x0), x1)
            y = min(max(y, y0), y1)
            collars.append((x, y))
    rng.shuffle(collars)
    return collars[:n_target]


def grad_pole(zfunc, x, y, h=1.0):
    """Upward-pointing unit normal (gradient pole) of the surface z = zfunc(x,y),
    from finite differences. Pole = normalize([-dz/dx, -dz/dy, 1])."""
    dzdx = (zfunc(x + h, y) - zfunc(x - h, y)) / (2 * h)
    dzdy = (zfunc(x, y + h) - zfunc(x, y - h)) / (2 * h)
    v = [-dzdx, -dzdy, 1.0]
    n = math.sqrt(sum(c * c for c in v))
    return [c / n for c in v]


def build_series(collars, ori_every, seed):
    """Sample the boreholes into two series' point sets (post-fault elevations).

    Returns (sediments, basalt) dicts each with sp_xyz / sp_if / ori_xyz / ori_pole.
    """
    rng = random.Random(seed + 1)
    z0, z1 = EXTENT["z"]

    sed = {"sp_xyz": [], "sp_if": [], "ori_xyz": [], "ori_pole": []}
    bas = {"sp_xyz": [], "sp_if": [], "ori_xyz": [], "ori_pole": []}

    for idx, (x, y) in enumerate(collars):
        take_ori = (idx % ori_every) == 0

        # Sediment contacts: observed (faulted) elevation, logged if in-range.
        for k, iid in zip(range(3), SED_IDS):
            zg = sed_contact_z(k, x, y)
            z = apply_fault_offset(x, y, zg)
            if z0 <= z <= z1:
                sed["sp_xyz"].extend([x, y, z])
                sed["sp_if"].append(iid)
                if take_ori:
                    # Orientation from the faulted surface: same dip as the analytic
                    # contact (the fault is a vertical-ish offset, leaving local dip ~unchanged).
                    pole = grad_pole(lambda xx, yy: apply_fault_offset(xx, yy, sed_contact_z(k, xx, yy)), x, y)
                    sed["ori_xyz"].extend([x, y, z])
                    sed["ori_pole"].extend(pole)

        # Erosion/basalt contact.
        zg = ero_surface_z(x, y)
        z = apply_fault_offset(x, y, zg)
        if z0 <= z <= z1:
            bas["sp_xyz"].extend([x, y, z])
            bas["sp_if"].append(ERO_ID)
            if take_ori:
                pole = grad_pole(lambda xx, yy: apply_fault_offset(xx, yy, ero_surface_z(xx, yy)), x, y)
                bas["ori_xyz"].extend([x, y, z])
                bas["ori_pole"].extend(pole)

    return sed, bas


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--out", default="web2/public/scenario.json")
    ap.add_argument("--n", type=int, default=3000, help="target borehole count")
    ap.add_argument("--grid", type=float, default=300.0, help="grid spacing (ft)")
    ap.add_argument("--jitter", type=float, default=40.0, help="collar jitter sigma (ft)")
    ap.add_argument("--ori-every", type=int, default=15, help="1 in N holes yields orientations")
    ap.add_argument("--seed", type=int, default=42)
    args = ap.parse_args()

    configure_extent(args.n, args.grid)
    collars = gen_boreholes(args.n, args.grid, args.jitter, args.seed)
    sed, bas = build_series(collars, args.ori_every, args.seed)

    # Stack order: youngest first. Basalt (Erode) over sediments (Basement). The late
    # fault is applied to BOTH series' fits (same plane), so the whole column offsets.
    # Relation codes: 0=Erode 1=Onlap 2=Conformable 3=Basement.
    fault = {"point": FAULT_POINT, "normal": FAULT_NORMAL,
             "strike_deg": FAULT_STRIKE_DEG, "dip_deg": FAULT_DIP_DEG, "offset_ft": FAULT_OFFSET}
    scenario = {
        "extent": [EXTENT["x"][0], EXTENT["x"][1], EXTENT["y"][0], EXTENT["y"][1],
                   EXTENT["z"][0], EXTENT["z"][1]],
        "fault": fault,
        "interface_names": {str(SED_IDS[0]): "sed_lower", str(SED_IDS[1]): "sed_mid",
                            str(SED_IDS[2]): "sed_upper", str(ERO_ID): "erosion_basalt"},
        "series": [
            {"name": "basalt_fill", "relation": 0, "faulted": True, **bas},
            {"name": "sediments", "relation": 3, "faulted": True, **sed},
        ],
        "stats": {
            "boreholes": len(collars),
            "sed_points": len(sed["sp_if"]),
            "basalt_points": len(bas["sp_if"]),
            "sed_orientations": len(sed["ori_pole"]) // 3,
            "basalt_orientations": len(bas["ori_pole"]) // 3,
        },
    }

    import os
    os.makedirs(os.path.dirname(args.out), exist_ok=True)
    with open(args.out, "w") as f:
        json.dump(scenario, f)
    print(f"wrote {args.out}")
    print(json.dumps(scenario["stats"], indent=2))
    print("fault normal:", [round(c, 4) for c in FAULT_NORMAL])


if __name__ == "__main__":
    main()
