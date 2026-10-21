import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import { fileURLToPath } from "node:url";

// Two-wasm split: the engine (geofield.wasm, BSD) imports its cokriging field oracle
// from the wasm-import-module "kernel". wasm-bindgen lowers that to an ES import
// `import * as importN from "kernel"`, so we alias the bare specifier "kernel" to the
// bridge module that owns the kernel instance (geofield_kernel.wasm, EUPL) and the
// memory-bridging shims. This is what keeps the EUPL code in its own artifact: the
// engine never links it, it imports it.
export default defineConfig({
  plugins: [react()],
  resolve: {
    alias: {
      kernel: fileURLToPath(new URL("./src/kernel-bridge.ts", import.meta.url)),
    },
  },
  server: { fs: { allow: [".."] } },
});
