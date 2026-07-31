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
    this.inv = null; // Uint16Array over client_inv_ptr (item,count ×30)
    this.catalog = null; // Uint8Array over client_catalog_ptr (25 B rows)
    this.slotChanges = null; // Uint32Array over client_slot_changes_ptr
    this.inCap = ex.client_in_cap();
    this.refresh();
  }

  refresh() {
    const ex = this.ex;
    // Every ptr getter FIRST, then read the buffer. A getter can allocate on
    // its first call, which grows the memory and detaches any buffer already
    // in hand — so `const buf = ex.memory.buffer` before these would capture a
    // reference that `new Uint8Array(buf, …)` then rejects as detached. That
    // was a hard boot failure in the browser ("Cannot perform Construct on a
    // detached ArrayBuffer") that no native or node gate could see.
    const inPtr = ex.client_in_ptr();
    const outPtr = ex.client_out_ptr();
    const renderPtr = ex.client_render_ptr();
    const remoteIdsPtr = ex.client_remote_ids_ptr();
    const invPtr = ex.client_inv_ptr();
    const catalogPtr = ex.client_catalog_ptr();
    const slotChangesPtr = ex.client_slot_changes_ptr();

    const buf = ex.memory.buffer;
    if (buf === this.buffer) return;
    this.buffer = buf;
    this.input = new Uint8Array(buf, inPtr, this.inCap);
    this.output = new Uint8Array(buf, outPtr, 1100);
    // 13 own/status floats + count + 64 remotes × 8 floats.
    this.render = new Float32Array(buf, renderPtr, 14 + 64 * 8);
    this.remoteIds = new Uint32Array(buf, remoteIdsPtr, 64);
    this.inv = new Uint16Array(buf, invPtr, 30 * 2);
    this.catalog = new Uint8Array(buf, catalogPtr, 64 * 25);
    this.slotChanges = new Uint32Array(buf, slotChangesPtr, 64 * 2);
  }
}
