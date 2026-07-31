import { defineConfig } from "vite";

// COOP/COEP from day one (DESIGN.md §9) so SharedArrayBuffer is available
// the moment a slice wants it.
const crossOriginIsolation = {
  "Cross-Origin-Opener-Policy": "same-origin",
  "Cross-Origin-Embedder-Policy": "require-corp",
};

export default defineConfig({
  server: { headers: crossOriginIsolation },
  preview: { headers: crossOriginIsolation },
  build: { target: "es2022" },
});
