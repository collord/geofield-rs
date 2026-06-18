/// <reference types="vite/client" />

// `?url` imports (e.g. the kernel wasm) resolve to an asset URL string at build time.
declare module "*.wasm?url" {
  const url: string;
  export default url;
}
