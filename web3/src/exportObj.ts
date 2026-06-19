// Feature 002 (enhanced-authoring) T034 [US6] — OBJ serialization from the rendered meshes.
//
// Pure: SurfacePayload[] -> one OBJ string. `v`/`vn` from positions/normals, `f` with `//`
// normal refs (OBJ is 1-indexed; the vertex-index offset accumulates across surfaces). Each
// surface becomes a group `g series{si}_block{b}_iface{k}`. No worker round-trip; the caller
// triggers the download.
//
// COVERAGE BOUND (Constitution IV): v1 emits GEOMETRY + GROUPS only. Materials (a companion
// .mtl with the viewer's SERIES_COLORS) are deliberately OUT OF SCOPE for v1 — logged here,
// not silently omitted. The seam is the per-surface group name (carries the series index),
// so a later pass can add `usemtl series{si}` + an .mtl without restructuring this output.

import type { SurfacePayload } from "./protocol";

export function exportObj(surfaces: SurfacePayload[]): string {
  const lines: string[] = ["# geofield enhanced-authoring OBJ export", "# geometry + groups (materials deferred — v1)"];
  let vertexOffset = 0; // OBJ indices are 1-based and accumulate across surfaces

  for (const s of surfaces) {
    const nVerts = s.positions.length / 3;
    lines.push(`g series${s.series}_block${s.block}_iface${s.interface}`);
    for (let i = 0; i < nVerts; i++) {
      lines.push(`v ${s.positions[i * 3]} ${s.positions[i * 3 + 1]} ${s.positions[i * 3 + 2]}`);
    }
    for (let i = 0; i < nVerts; i++) {
      lines.push(`vn ${s.normals[i * 3]} ${s.normals[i * 3 + 1]} ${s.normals[i * 3 + 2]}`);
    }
    for (let i = 0; i < s.indices.length; i += 3) {
      const a = s.indices[i] + 1 + vertexOffset;
      const b = s.indices[i + 1] + 1 + vertexOffset;
      const c = s.indices[i + 2] + 1 + vertexOffset;
      lines.push(`f ${a}//${a} ${b}//${b} ${c}//${c}`);
    }
    vertexOffset += nVerts;
  }

  return lines.join("\n") + "\n";
}
