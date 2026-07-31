// Raw C-ABI wasm loader (no bindgen — the same pattern as ci/parity.mjs
// and ci/client_smoke.mjs). Views over wasm memory are cached and rebuilt
// only when the memory's ArrayBuffer identity changes (a grow detaches
// views); after warmup the bridge never grows, so refresh() is one
// identity compare per frame — zero-copy, zero-GC (DESIGN.md L8).

export async function loadWasm(url) {
  const res = await fetch(url);
  if (!res.ok) throw new Error(`wasm fetch failed: ${res.status} ${url}`);
  const { instance } = await WebAssembly.instantiate(
    await res.arrayBuffer(),
    {},
  );
  return instance.exports;
}

export class WasmViews {
  constructor(ex) {
    this.ex = ex;
    this.buffer = null;
    this.input = null; // Uint8Array over client_in_ptr
    this.output = null; // Uint8Array over client_out_ptr
    this.render = null; // Float32Array over client_render_ptr
    this.remoteIds = null; // Uint32Array over client_remote_ids_ptr
    this.inCap = ex.client_in_cap();
    this.refresh();
  }

  refresh() {
    const buf = this.ex.memory.buffer;
    if (buf === this.buffer) return;
    this.buffer = buf;
    const ex = this.ex;
    this.input = new Uint8Array(buf, ex.client_in_ptr(), this.inCap);
    this.output = new Uint8Array(buf, ex.client_out_ptr(), 1100);
    // 13 own/status floats + count + 64 remotes × 8 floats.
    this.render = new Float32Array(buf, ex.client_render_ptr(), 14 + 64 * 8);
    this.remoteIds = new Uint32Array(buf, ex.client_remote_ids_ptr(), 64);
  }
}
