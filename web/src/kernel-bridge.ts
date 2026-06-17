// The "kernel" module the engine wasm imports (geofield_kernel.wasm, EUPL).
//
// wasm-bindgen compiles the engine's `#[link(wasm_import_module = "kernel")]` imports
// into ES-module imports `import * as importN from "kernel"`. Vite aliases the bare
// specifier "kernel" to THIS file (see vite.config.ts), so these exports satisfy them.
//
// The engine and the kernel are two SEPARATE wasm instances with separate linear
// memories. Scalar-in/scalar-out functions (gf_eval, counts, fit) forward straight to
// the kernel's exports. But any kernel function that WRITES A BUFFER writes into the
// KERNEL's memory at the pointer it is given — and the engine passes a pointer into the
// ENGINE's memory. So the buffer-returning functions are bridged HERE: call the real
// kernel export into a kernel scratch buffer, then copy the result bytes into the
// engine's memory at the engine-supplied out pointer.
//
// `fit`/`fit_faulted` are the mirror case: the engine passes a pointer to an f64 input
// buffer in ENGINE memory; we copy it into a kernel `gf_alloc` buffer before calling the
// real kernel `gf_fit`.

// ---------------------------------------------------------------------------
// The kernel instance (instantiated by initKernel(), awaited before the engine inits).
// ---------------------------------------------------------------------------

interface KernelExports {
  memory: WebAssembly.Memory;
  gf_alloc(len: number): number;
  gf_free(ptr: number, len: number): void;
  gf_fit(ptr: number, lenF64: number): number;
  gf_fit_faulted(ptr: number, lenF64: number): number;
  gf_eval(handle: number, block: number, x: number, y: number, z: number): number;
  gf_eval_grad(handle: number, block: number, x: number, y: number, z: number, outPtr: number): void;
  gf_lithology_unit(handle: number, x: number, y: number, z: number): number;
  gf_nblocks(handle: number): number;
  gf_block_interface_count(handle: number, b: number): number;
  gf_block_unit_count(handle: number, b: number): number;
  gf_block_interface_isovalues(handle: number, b: number, outPtr: number, cap: number): number;
  gf_block_interface_ids(handle: number, b: number, outPtr: number, cap: number): number;
  gf_block_half_spaces(handle: number, b: number, outPtr: number, cap: number): number;
  gf_interface_count(handle: number): number;
  gf_interface_isovalues(handle: number, outPtr: number, cap: number): number;
  gf_interface_ids(handle: number, outPtr: number, cap: number): number;
  gf_diagnostics(handle: number, outPtr: number): void;
  gf_release(handle: number): void;
  // Optional instrumentation (feature = "wasm-abi-stats"). Present unconditionally in the
  // kernel ABI; gf_eval_count returns 0 when the feature is off.
  gf_eval_count(): bigint;
  gf_eval_count_reset(): void;
}

let kernel: KernelExports | null = null;

/** The engine's memory, set by the host right after the engine wasm is instantiated, so
 *  the buffer-bridge shims can copy bytes INTO the engine's address space. */
let engineMemory: WebAssembly.Memory | null = null;

/** Called by the host once the engine wasm is up, so bridges can reach engine memory. */
export function setEngineMemory(mem: WebAssembly.Memory): void {
  engineMemory = mem;
}

/** Instantiate geofield_kernel.wasm (it imports nothing). Idempotent. */
export async function initKernel(wasmUrl: string | URL): Promise<void> {
  if (kernel) return;
  let instance: WebAssembly.Instance;
  if (typeof WebAssembly.instantiateStreaming === "function") {
    ({ instance } = await WebAssembly.instantiateStreaming(fetch(wasmUrl), {}));
  } else {
    const bytes = await (await fetch(wasmUrl)).arrayBuffer();
    ({ instance } = await WebAssembly.instantiate(bytes, {}));
  }
  kernel = instance.exports as unknown as KernelExports;
}

function k(): KernelExports {
  if (!kernel) throw new Error("kernel wasm not initialized — call initKernel() first");
  return kernel;
}

/** Total field evals (gf_eval + gf_eval_grad) since the last reset — the per-sample
 *  wasm-module-boundary crossings. Returns 0 if the kernel was built without
 *  `wasm-abi-stats`. For the web2 scale harness. */
export function evalCount(): number {
  return Number(k().gf_eval_count());
}
export function evalCountReset(): void {
  k().gf_eval_count_reset();
}

// Scratch buffer in KERNEL memory for bridged reads (grown on demand).
let scratchPtr = 0;
let scratchLen = 0;
function kernelScratch(bytes: number): number {
  if (bytes > scratchLen) {
    if (scratchPtr) k().gf_free(scratchPtr, scratchLen);
    scratchPtr = k().gf_alloc(bytes);
    scratchLen = bytes;
  }
  return scratchPtr;
}

/** Copy `nBytes` from kernel scratch (at `scratchPtr`) into engine memory at `engOut`. */
function copyKernelToEngine(engOut: number, nBytes: number): void {
  if (!engineMemory) throw new Error("engine memory not registered");
  const src = new Uint8Array(k().memory.buffer, scratchPtr, nBytes);
  const dst = new Uint8Array(engineMemory.buffer, engOut, nBytes);
  dst.set(src);
}

/** Copy `nBytes` from engine memory at `engIn` into a fresh kernel buffer; return its ptr. */
function copyEngineToKernel(engIn: number, nBytes: number): number {
  if (!engineMemory) throw new Error("engine memory not registered");
  const ptr = k().gf_alloc(nBytes);
  const src = new Uint8Array(engineMemory.buffer, engIn, nBytes);
  const dst = new Uint8Array(k().memory.buffer, ptr, nBytes);
  dst.set(src);
  return ptr;
}

// ---------------------------------------------------------------------------
// Exports the engine imports as module "kernel". Names MUST match the engine's
// `extern "C"` import names exactly.
// ---------------------------------------------------------------------------

// --- Pure scalar in/out: forward straight through. -------------------------

export function gf_eval(handle: number, block: number, x: number, y: number, z: number): number {
  return k().gf_eval(handle, block, x, y, z);
}
export function gf_lithology_unit(handle: number, x: number, y: number, z: number): number {
  return k().gf_lithology_unit(handle, x, y, z);
}
export function gf_nblocks(handle: number): number {
  return k().gf_nblocks(handle);
}
export function gf_block_interface_count(handle: number, b: number): number {
  return k().gf_block_interface_count(handle, b);
}
export function gf_block_unit_count(handle: number, b: number): number {
  return k().gf_block_unit_count(handle, b);
}
export function gf_interface_count(handle: number): number {
  return k().gf_interface_count(handle);
}
export function gf_release(handle: number): void {
  k().gf_release(handle);
}

// --- Fit: engine f64 input buffer (engine memory) → kernel buffer. ---------

function fitBridge(fn: (ptr: number, len: number) => number, engPtr: number, lenF64: number): number {
  const nBytes = lenF64 * 8;
  const kPtr = copyEngineToKernel(engPtr, nBytes);
  const handle = fn(kPtr, lenF64);
  k().gf_free(kPtr, nBytes);
  return handle;
}
export function gf_fit(ptr: number, lenF64: number): number {
  return fitBridge(k().gf_fit.bind(k()), ptr, lenF64);
}
export function gf_fit_faulted(ptr: number, lenF64: number): number {
  return fitBridge(k().gf_fit_faulted.bind(k()), ptr, lenF64);
}

// --- Buffer-out: kernel writes into kernel scratch → copy into engine memory. ---

export function gf_eval_grad(
  handle: number, block: number, x: number, y: number, z: number, outPtr: number,
): void {
  const sp = kernelScratch(24); // 3 f64
  k().gf_eval_grad(handle, block, x, y, z, sp);
  copyKernelToEngine(outPtr, 24);
}

function bufferOut(
  call: (sp: number, cap: number) => number, outPtr: number, cap: number, f64sPerItem: number,
): number {
  const sp = kernelScratch(cap * f64sPerItem * 8);
  const n = call(sp, cap);
  copyKernelToEngine(outPtr, n * f64sPerItem * 8);
  return n;
}
export function gf_block_interface_isovalues(handle: number, b: number, outPtr: number, cap: number): number {
  return bufferOut((sp, c) => k().gf_block_interface_isovalues(handle, b, sp, c), outPtr, cap, 1);
}
export function gf_block_interface_ids(handle: number, b: number, outPtr: number, cap: number): number {
  return bufferOut((sp, c) => k().gf_block_interface_ids(handle, b, sp, c), outPtr, cap, 1);
}
export function gf_interface_isovalues(handle: number, outPtr: number, cap: number): number {
  return bufferOut((sp, c) => k().gf_interface_isovalues(handle, sp, c), outPtr, cap, 1);
}
export function gf_interface_ids(handle: number, outPtr: number, cap: number): number {
  return bufferOut((sp, c) => k().gf_interface_ids(handle, sp, c), outPtr, cap, 1);
}
export function gf_block_half_spaces(handle: number, b: number, outPtr: number, cap: number): number {
  return bufferOut((sp, c) => k().gf_block_half_spaces(handle, b, sp, c), outPtr, cap, 7);
}
export function gf_diagnostics(handle: number, outPtr: number): void {
  const sp = kernelScratch(16); // 2 f64
  k().gf_diagnostics(handle, sp);
  copyKernelToEngine(outPtr, 16);
}
