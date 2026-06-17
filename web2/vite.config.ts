import { defineConfig } from "vite";
import { fileURLToPath } from "node:url";

// Same two-wasm wiring as web/: the engine's `import * from "kernel"` resolves to the
// kernel bridge (which we reuse from ../web/src). The kernel wasm is served from
// web2/public/ (built with the wasm-abi-stats feature so the harness can report
// boundary-crossing eval counts).
export default defineConfig({
  resolve: {
    alias: {
      kernel: fileURLToPath(new URL("../web/src/kernel-bridge.ts", import.meta.url)),
    },
  },
  server: { fs: { allow: ["..", "../.."] } },
});
