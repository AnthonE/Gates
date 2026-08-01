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
    this.craftJobs = null; // Uint16Array over client_craft_jobs_ptr
    this.recipes = null; // Uint16Array over client_recipes_ptr (14-word rows)
    this.pieceChanges = null; // Uint32Array over client_piece_changes_ptr
    this.pieceDefs = null; // Uint16Array over client_piece_defs_ptr (8-word rows)
    this.deployChanges = null; // Uint32Array over client_deploy_changes_ptr
    this.deployDefs = null; // Uint16Array over client_deploy_defs_ptr (4-word rows)
    this.stock = null; // Uint32Array over client_stock_ptr (item,units pairs)
    this.chat = null; // Uint8Array over client_chat_ptr (id, global, len, text)
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
    const craftJobsPtr = ex.client_craft_jobs_ptr();
    const recipesPtr = ex.client_recipes_ptr();
    const pieceChangesPtr = ex.client_piece_changes_ptr();
    const pieceDefsPtr = ex.client_piece_defs_ptr();
    const deployChangesPtr = ex.client_deploy_changes_ptr();
    const deployDefsPtr = ex.client_deploy_defs_ptr();
    const stockPtr = ex.client_stock_ptr();
    const chatPtr = ex.client_chat_ptr();

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
    this.craftJobs = new Uint16Array(buf, craftJobsPtr, 4 * 2);
    // 64 recipes × (output, count, ticks lo/hi, station, n_inputs, 4×(item,count)).
    this.recipes = new Uint16Array(buf, recipesPtr, 64 * 14);
    // 32 piece records × [cx<<16|cz, level<<16|loc<<8|row].
    this.pieceChanges = new Uint32Array(buf, pieceChangesPtr, 32 * 2);
    // 32 piece defs × (shape, material, hp, n_costs, 2×(item,count)).
    this.pieceDefs = new Uint16Array(buf, pieceDefsPtr, 32 * 8);
    // 24 deploy records × [cx<<16|cz, level<<16|loc<<8|row].
    this.deployChanges = new Uint32Array(buf, deployChangesPtr, 24 * 2);
    // 16 deploy defs × (arch, placement, hp, item).
    this.deployDefs = new Uint16Array(buf, deployDefsPtr, 16 * 4);
    // 4 hearth stock rows × (item, units).
    this.stock = new Uint32Array(buf, stockPtr, 4 * 2);
    // One popped chat line: id (4 LE bytes), global, length, 48 text bytes.
    this.chat = new Uint8Array(buf, chatPtr, 6 + 48);
  }
}
