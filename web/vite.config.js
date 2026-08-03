import { defineConfig } from "vite";

// COOP/COEP from day one (DESIGN.md §9) so SharedArrayBuffer is available
// the moment a slice wants it.
const crossOriginIsolation = {
  "Cross-Origin-Opener-Policy": "same-origin",
  "Cross-Origin-Embedder-Policy": "require-corp",
};

export default defineConfig({
  // `../assets` is the CC0 base-map set (ART.md §7). It lives outside `web/`
  // because it is the repo's art, not the bundle's — the client imports each
  // file by URL, so `vite build` hashes them into `dist/assets/` (which is
  // what the browser gate serves) and the dev server needs to be told it may
  // read a directory above its root.
  server: { headers: crossOriginIsolation, fs: { allow: ["..", "../assets"] } },
  preview: { headers: crossOriginIsolation },
  // Relative asset URLs so the bundle works under ANY path — it is served
  // from the site root in dev and from /games/gates/alpha/ on the public
  // page, and an absolute `/assets/...` 404s under a subpath.
  base: "./",
  build: { target: "es2022" },
});
