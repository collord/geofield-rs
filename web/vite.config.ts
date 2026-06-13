import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// wasm-pack output (web/src/wasm) is plain ESM + a .wasm asset; Vite serves it
// directly. No special wasm plugin needed for the --target web build.
export default defineConfig({
  plugins: [react()],
  server: { fs: { allow: [".."] } },
});
