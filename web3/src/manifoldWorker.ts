// The DEDICATED, PERSISTENT Manifold-boolean worker.
//
// WHY A SECOND WORKER: manifold-3d deadlocks in `subtract` on the 2nd instantiation in one JS realm,
// and the solver worker (worker.ts) is terminate-per-fit — it re-instantiates every run, so it can't
// host Manifold. A Web Worker is a SEPARATE realm, so THIS worker instantiates Manifold exactly ONCE
// (the lazy `initManifold` singleton) and is NEVER terminated: no 2nd instantiation, no deadlock, and
// the heavy #bodies×#triangles compose runs off the UI thread (review 2.1). The main thread relays a
// recipe here, we compose it, stream per-unit progress, and ship the composed surfaces back.
//
// The main thread guards against STALE results with a monotonic `token`: we echo it on every response,
// and main drops any whose token is no longer current (review 2.2). This worker always finishes the
// current compose (it is synchronously busy for one message); a superseded request simply queues.

import { initManifold, executeRecipe } from "./manifoldVolumes";
import type { ManifoldCommand, ManifoldMessage } from "./protocol";

function emit(msg: ManifoldMessage, transfer: Transferable[] = []): void {
  (self as unknown as Worker).postMessage(msg, transfer);
}

self.onmessage = async (e: MessageEvent<ManifoldCommand>): Promise<void> => {
  const cmd = e.data;
  if (cmd.type !== "compose") return;
  const { token, ops, extent, assignment } = cmd;
  try {
    // ONCE per worker lifetime — the idempotent singleton returns the cached instance after the
    // first call, so this realm never re-instantiates (the whole point).
    const wasm = await initManifold();
    const { surfaces, droppedUnits } = executeRecipe(wasm, ops, extent, assignment, (k, n) =>
      emit({ type: "progress", token, k, n }),
    );
    // Transfer the composed geometry back (zero-copy). `normals` is a zero-length buffer today
    // (matches the solver worker's transfer list) — harmless to include.
    const transfer: Transferable[] = [];
    for (const s of surfaces) transfer.push(s.positions.buffer, s.normals.buffer, s.indices.buffer);
    emit({ type: "done", token, surfaces, droppedUnits }, transfer);
  } catch (err) {
    // An actual compose failure (distinct from droppedUnits, which flows through `done`).
    emit({ type: "error", token, message: err instanceof Error ? err.message : String(err) });
  }
};
