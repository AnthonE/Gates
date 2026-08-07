//! Raw C-ABI wasm bridge — no bindgen, the same pattern as the parity
//! probe (`ci/parity.mjs`): plain `extern "C"` exports plus fixed buffers
//! the JS side views zero-copy over wasm memory (DESIGN.md L8: no
//! per-frame objects, no per-packet allocation). One client per instance —
//! a browser tab is one player. Compiled only for wasm32; native callers
//! use `ClientCore` directly.
//!
//! Calling convention: JS writes incoming bytes at `client_in_ptr()`,
//! calls the matching `*_on_*`/`*_parse_*` with the length; outgoing bytes
//! appear at `client_out_ptr()`; render floats at `client_render_ptr()`
//! (layout documented on `render_fill`). u64 seeds cross as BigInt.

use crate::core::ClientCore;
use protocol::{
    decode_refuse, decode_welcome, encode_action_cancel, encode_action_consume,
    encode_action_container, encode_action_craft, encode_action_deploy, encode_action_drink,
    encode_action_feed, encode_action_lock, encode_action_loot, encode_action_move,
    encode_action_place, encode_action_repair, encode_action_respawn, encode_action_throw,
    encode_action_upgrade, encode_action_use, encode_chat, encode_hello, peek_kind, Hello,
    CHAT_MAX_BYTES, DEPLOY_SYNC_BATCH, KIND_REFUSE, KIND_WELCOME, MAX_ITEM_NAME_BYTES,
    PIECE_SYNC_BATCH, PROTO_VER, SLOT_SYNC_BATCH,
};
use sim_core::limits::{
    CRAFT_QUEUE, DATAGRAM_BUDGET_BYTES, HEARTH_STOCK_ROWS, INV_SLOTS, MAX_BACKPACKS,
    MAX_DEPLOY_DEFS, MAX_ITEM_DEFS, MAX_PIECE_COSTS, MAX_PIECE_DEFS, MAX_RECIPES,
    MAX_RECIPE_INPUTS, MAX_SNAPSHOT_ENTITIES,
};
use sim_core::movement::{POS_XZ_Q, POS_Y_Q};
use sim_core::terrain::{self, ScatterTable};
use std::cell::RefCell;

/// Incoming scratch: comfortably above the server's 1100 B send clamp.
const IN_CAP: usize = 2048;
/// Handshake words:
/// [kind, player_id, seed_lo, seed_hi, tick, refuse_code, dev].
const HS_WORDS: usize = 7;
/// Render layout: a 13-float own/status block, a count, then 8 floats per
/// remote (ids ride the parallel u32 buffer — f32 can't hold late-shard
/// generations exactly).
const OWN_FLOATS: usize = 13;
const REMOTE_FLOATS: usize = 8;
const RENDER_FLOATS: usize = OWN_FLOATS + 1 + MAX_SNAPSHOT_ENTITIES * REMOTE_FLOATS;
/// Terrain fill grid bound: the far mesh at 8 m over 2,048 m is 257
/// samples plus a 2-sample normal apron.
const HEIGHTS_MAX_N: usize = 259;
/// Slot fill: one chunk of 8×8 scatter cells, 8 floats per resolved slot
/// (occupant, x, y, z, yaw, scale, cx, cz — cell coords so the renderer
/// can key instances by cell for harvest/respawn).
const SLOTS_MAX_CELLS: usize = 8;
const SLOT_FLOATS: usize = 8;
/// Clutter fill: one 16 m tile of the sub-metre ground population, 6 floats
/// per element (kind, x, y, z, yaw, scale). No cell coords — clutter is not
/// harvested, so nothing downstream needs to key an instance back to a cell.
const CLUTTER_FLOATS: usize = 6;
/// Catalog view row: length byte + name bytes.
const CATALOG_ROW: usize = 1 + MAX_ITEM_NAME_BYTES;
/// Chat view: speaker id (4 LE bytes), global flag, length, then the
/// line's UTF-8 bytes. One popped line at a time, like the toasts.
const CHAT_VIEW_BYTES: usize = 6 + CHAT_MAX_BYTES;
/// Recipe view row, u16 words: output, out_count, ticks lo, ticks hi,
/// station, n_inputs, then (item, count) per input slot.
const RECIPE_ROW_WORDS: usize = 6 + 2 * MAX_RECIPE_INPUTS;
/// Piece-def view row, u16 words: shape, material, hp, n_costs, then
/// (item, count) per cost slot.
const PIECE_DEF_ROW_WORDS: usize = 4 + 2 * MAX_PIECE_COSTS;
/// Deploy-def view row, u16 words: arch, placement, hp, item.
const DEPLOY_DEF_ROW_WORDS: usize = 4;
/// `client_on_stream` error flag (high bit; real flags are low bits).
///
/// Defined in `core.rs` beside the `APPLIED_*` flags it shares the word
/// with, not here: two files each holding half of one bit layout is how
/// `APPLIED_MOVE` came to be assigned this exact value.
const STREAM_ERR: u32 = crate::core::STREAM_ERR;

struct Bridge {
    core: Option<ClientCore>,
    in_buf: [u8; IN_CAP],
    out_buf: [u8; DATAGRAM_BUDGET_BYTES],
    hs_buf: [u32; HS_WORDS],
    render: Box<[f32; RENDER_FLOATS]>,
    remote_ids: [u32; MAX_SNAPSHOT_ENTITIES],
    heights: Vec<f32>,
    slots: [f32; SLOTS_MAX_CELLS * SLOTS_MAX_CELLS * SLOT_FLOATS],
    clutter: [f32; terrain::CLUTTER_TILE_CAP * CLUTTER_FLOATS],
    /// The haven pad, memoized by seed. `terrain::haven` is a pure function
    /// of the seed and a global argmax over the whole road ring, and
    /// `terrain_fill_slots` was paying for the whole search once per
    /// streamed chunk — measured 5,453 height taps mean per call against a
    /// doc claim of ~1,000 (`findings/pass-20260804-205133-02-judge.md`),
    /// and the ring-phase check chain this pass added to the selector makes
    /// each of those searches dearer. Caching cannot change what is drawn:
    /// the key is the only input.
    haven: Option<(u64, terrain::Haven)>,
    /// (cell key, harvested) pairs from the last stream message.
    changes: [u32; SLOT_SYNC_BATCH * 2],
    changes_len: u32,
    /// Own inventory view: item, count per slot.
    inv: [u16; INV_SLOTS * 2],
    /// Open container view: item, count per slot, same layout as `inv`
    /// so a panel can draw either with one reader.
    cont: [u16; INV_SLOTS * 2],
    /// Item names: `CATALOG_ROW` bytes per item index.
    catalog: Box<[u8; MAX_ITEM_DEFS * CATALOG_ROW]>,
    /// Craft queue view: recipe, remaining per job slot.
    craft_jobs: [u16; CRAFT_QUEUE * 2],
    /// Recipe table view: `RECIPE_ROW_WORDS` u16s per recipe index.
    recipes: Box<[u16; MAX_RECIPES * RECIPE_ROW_WORDS]>,
    /// Piece records the last stream message added, packed as u32 pairs:
    /// [cx << 16 | cz, level << 16 | loc << 8 | row].
    piece_changes: [u32; PIECE_SYNC_BATCH * 2],
    piece_changes_len: u32,
    /// Piece-def table view: `PIECE_DEF_ROW_WORDS` u16s per piece row.
    piece_defs: Box<[u16; MAX_PIECE_DEFS * PIECE_DEF_ROW_WORDS]>,
    /// Deployable records the last stream message added, packed like the
    /// piece pairs plus the door's state bits:
    /// [cx << 16 | cz, locked << 25 | open << 24 | level << 16 | loc << 8
    /// | row].
    deploy_changes: [u32; DEPLOY_SYNC_BATCH * 2],
    deploy_changes_len: u32,
    /// The whole standing-bag set, refreshed on `APPLIED_BAGS`: ids in
    /// one buffer, world-metre positions (x, y, z) in another. Two
    /// buffers rather than one interleaved: an id past 2^24 does not
    /// survive an f32, and a shard that runs long enough to mint one
    /// must not start drawing bags on top of each other.
    bag_ids: Box<[u32]>,
    bag_pos: Box<[f32]>,
    bags_len: u32,
    /// Deploy-def table view: `DEPLOY_DEF_ROW_WORDS` u16s per row.
    deploy_defs: Box<[u16; MAX_DEPLOY_DEFS * DEPLOY_DEF_ROW_WORDS]>,
    /// Last stock ack: [item, units] u32 pairs (rows live in the core).
    stock: [u32; HEARTH_STOCK_ROWS * 2],
    /// Last popped chat line, in the `CHAT_VIEW_BYTES` layout.
    chat: [u8; CHAT_VIEW_BYTES],
}

impl Bridge {
    fn new() -> Self {
        Self {
            core: None,
            in_buf: [0; IN_CAP],
            out_buf: [0; DATAGRAM_BUDGET_BYTES],
            hs_buf: [0; HS_WORDS],
            render: Box::new([0.0; RENDER_FLOATS]),
            remote_ids: [0; MAX_SNAPSHOT_ENTITIES],
            heights: vec![0.0; HEIGHTS_MAX_N * HEIGHTS_MAX_N],
            slots: [0.0; SLOTS_MAX_CELLS * SLOTS_MAX_CELLS * SLOT_FLOATS],
            clutter: [0.0; terrain::CLUTTER_TILE_CAP * CLUTTER_FLOATS],
            haven: None,
            changes: [0; SLOT_SYNC_BATCH * 2],
            changes_len: 0,
            inv: [0; INV_SLOTS * 2],
            cont: [0; INV_SLOTS * 2],
            catalog: Box::new([0; MAX_ITEM_DEFS * CATALOG_ROW]),
            craft_jobs: [0; CRAFT_QUEUE * 2],
            recipes: Box::new([0; MAX_RECIPES * RECIPE_ROW_WORDS]),
            piece_changes: [0; PIECE_SYNC_BATCH * 2],
            piece_changes_len: 0,
            piece_defs: Box::new([0; MAX_PIECE_DEFS * PIECE_DEF_ROW_WORDS]),
            deploy_changes: [0; DEPLOY_SYNC_BATCH * 2],
            deploy_changes_len: 0,
            bag_ids: vec![0; MAX_BACKPACKS].into_boxed_slice(),
            bag_pos: vec![0.0; MAX_BACKPACKS * 3].into_boxed_slice(),
            bags_len: 0,
            deploy_defs: Box::new([0; MAX_DEPLOY_DEFS * DEPLOY_DEF_ROW_WORDS]),
            stock: [0; HEARTH_STOCK_ROWS * 2],
            chat: [0; CHAT_VIEW_BYTES],
        }
    }
}

thread_local! {
    static BRIDGE: RefCell<Bridge> = RefCell::new(Bridge::new());
}

fn with<R>(f: impl FnOnce(&mut Bridge) -> R) -> R {
    BRIDGE.with(|b| f(&mut b.borrow_mut()))
}

#[no_mangle]
pub extern "C" fn client_proto_ver() -> u32 {
    PROTO_VER as u32
}

#[no_mangle]
pub extern "C" fn client_in_ptr() -> *mut u8 {
    with(|b| b.in_buf.as_mut_ptr())
}

#[no_mangle]
pub extern "C" fn client_in_cap() -> u32 {
    IN_CAP as u32
}

#[no_mangle]
pub extern "C" fn client_out_ptr() -> *const u8 {
    with(|b| b.out_buf.as_ptr())
}

/// Encode the hello message into the out buffer; returns its length.
#[no_mangle]
pub extern "C" fn client_hello() -> u32 {
    with(|b| {
        encode_hello(
            &Hello {
                proto_ver: PROTO_VER,
            },
            &mut b.out_buf,
        )
        .map(|n| n as u32)
        .unwrap_or(0)
    })
}

/// Parse a handshake reply from the in buffer. Returns 1 (welcome) or
/// 2 (refuse) with fields in the handshake words, 0 on garbage.
#[no_mangle]
pub extern "C" fn client_parse_handshake(len: u32) -> u32 {
    with(|b| {
        let bytes = &b.in_buf[..(len as usize).min(IN_CAP)];
        match peek_kind(bytes) {
            Ok(KIND_WELCOME) => match decode_welcome(bytes) {
                Ok(w) => {
                    b.hs_buf = [
                        1,
                        w.player_id,
                        w.seed as u32,
                        (w.seed >> 32) as u32,
                        w.tick,
                        0,
                        w.dev as u32,
                    ];
                    1
                }
                Err(_) => 0,
            },
            Ok(KIND_REFUSE) => match decode_refuse(bytes) {
                Ok(r) => {
                    b.hs_buf = [2, 0, 0, 0, 0, r.code as u32, 0];
                    2
                }
                Err(_) => 0,
            },
            _ => 0,
        }
    })
}

#[no_mangle]
pub extern "C" fn client_hs_ptr() -> *const u32 {
    with(|b| b.hs_buf.as_ptr())
}

/// Create (or recreate) the client. Everything prior is discarded — a
/// reconnect is a fresh join at v0.
#[no_mangle]
pub extern "C" fn client_new(seed: u64, player_id: u32, server_tick: u32) {
    with(|b| b.core = Some(ClientCore::new(seed, player_id, server_tick)));
}

/// One incoming datagram of `len` bytes from the in buffer. Returns the
/// `Ingest` code (0 error · 1 applied · 2 applied-delta · 3 stale · 4
/// no-baseline), or 5 when no client exists.
#[no_mangle]
pub extern "C" fn client_on_datagram(len: u32) -> u32 {
    with(|b| {
        let n = (len as usize).min(IN_CAP);
        let Bridge {
            core: Some(core),
            in_buf,
            ..
        } = b
        else {
            return 5;
        };
        core.on_datagram(&in_buf[..n]) as u32
    })
}

/// One event-lane stream message of `len` bytes from the in buffer.
/// Returns the `core::APPLIED_*` flags, or the high bit on a decode error
/// / missing client. Refreshes whichever views the flags point at: the
/// slot-change pairs, the inventory words, the catalog rows.
///
/// **This word is word 0 of two, and it cannot announce word 1** — bits
/// 0..30 are flags and bit 31 is the error above, so there is no spare bit
/// to announce with. A caller that wants the `APPLIED2_*` flags reads
/// `client_applied2()` after this returns, unconditionally; see it for why.
#[no_mangle]
pub extern "C" fn client_on_stream(len: u32) -> u32 {
    with(|b| {
        let n = (len as usize).min(IN_CAP);
        let Bridge {
            core: Some(core),
            in_buf,
            changes,
            changes_len,
            inv,
            cont,
            catalog,
            craft_jobs,
            recipes,
            piece_changes,
            piece_changes_len,
            piece_defs,
            deploy_changes,
            deploy_changes_len,
            bag_ids,
            bag_pos,
            bags_len,
            deploy_defs,
            stock,
            ..
        } = b
        else {
            return STREAM_ERR;
        };
        let flags = match core.on_stream(&in_buf[..n]) {
            Ok(f) => f,
            Err(_) => return STREAM_ERR,
        };
        if flags & (crate::core::APPLIED_SLOTS | crate::core::APPLIED_RESET) != 0 {
            let ch = core.slot_changes();
            for (i, &(key, harvested)) in ch.iter().enumerate() {
                changes[i * 2] = key;
                changes[i * 2 + 1] = harvested as u32;
            }
            *changes_len = ch.len() as u32;
        }
        if flags & crate::core::APPLIED_INV != 0 {
            for (i, s) in core.inv.iter().enumerate() {
                inv[i * 2] = s.item;
                inv[i * 2 + 1] = s.count;
            }
        }
        // Word 1, not word 0 — the container flag lives in `applied2`
        // because word 0 is full (`core.rs`). Read unconditionally after
        // the decode: `applied2` is rebuilt per message, so a message that
        // touched no container leaves it clear and this cannot republish a
        // stale view.
        if core.applied2() & crate::core::APPLIED2_CONT != 0 {
            for (i, s) in core.cont.iter().enumerate() {
                cont[i * 2] = s.item;
                cont[i * 2 + 1] = s.count;
            }
        }
        if flags & crate::core::APPLIED_CATALOG != 0 {
            for i in 0..(core.catalog.count as usize).min(MAX_ITEM_DEFS) {
                let name = core.catalog.name(i);
                let row = &mut catalog[i * CATALOG_ROW..(i + 1) * CATALOG_ROW];
                row[0] = name.len() as u8;
                row[1..1 + name.len()].copy_from_slice(name);
            }
        }
        if flags & crate::core::APPLIED_CRAFT_Q != 0 {
            for (i, &(recipe, remaining)) in core.jobs.iter().enumerate() {
                craft_jobs[i * 2] = recipe as u16;
                craft_jobs[i * 2 + 1] = remaining as u16;
            }
        }
        if flags & crate::core::APPLIED_RECIPES != 0 {
            for i in 0..(core.recipes_have as usize).min(MAX_RECIPES) {
                let def = &core.recipes.recipes[i];
                let row = &mut recipes[i * RECIPE_ROW_WORDS..(i + 1) * RECIPE_ROW_WORDS];
                row[0] = def.output;
                row[1] = def.out_count;
                row[2] = def.ticks as u16;
                row[3] = (def.ticks >> 16) as u16;
                row[4] = def.station as u16;
                row[5] = def.n_inputs as u16;
                for (k, &(item, count)) in def.inputs.iter().enumerate() {
                    row[6 + k * 2] = item;
                    row[6 + k * 2 + 1] = count;
                }
            }
        }
        if flags & (crate::core::APPLIED_PIECES | crate::core::APPLIED_PIECE_RESET) != 0 {
            let ch = core.piece_changes();
            for (i, rec) in ch.iter().enumerate() {
                piece_changes[i * 2] = ((rec.cx as u32) << 16) | rec.cz as u32;
                piece_changes[i * 2 + 1] =
                    ((rec.level as u32) << 16) | ((rec.loc as u32) << 8) | rec.row as u32;
            }
            *piece_changes_len = ch.len() as u32;
        }
        if flags & crate::core::APPLIED_PIECE_DEFS != 0 {
            for i in 0..(core.piece_defs_have as usize).min(MAX_PIECE_DEFS) {
                let def = &core.piece_defs.pieces[i];
                let row = &mut piece_defs[i * PIECE_DEF_ROW_WORDS..(i + 1) * PIECE_DEF_ROW_WORDS];
                row[0] = def.shape as u16;
                row[1] = def.material as u16;
                row[2] = def.hp;
                row[3] = def.n_costs as u16;
                for (k, &(item, count)) in def.costs.iter().enumerate() {
                    row[4 + k * 2] = item;
                    row[4 + k * 2 + 1] = count;
                }
            }
        }
        if flags & (crate::core::APPLIED_DEPLOYS | crate::core::APPLIED_DEPLOY_RESET) != 0 {
            let ch = core.deploy_changes();
            for (i, rec) in ch.iter().enumerate() {
                deploy_changes[i * 2] = ((rec.cx as u32) << 16) | rec.cz as u32;
                deploy_changes[i * 2 + 1] = ((rec.locked as u32) << 25)
                    | ((rec.open as u32) << 24)
                    | ((rec.level as u32) << 16)
                    | ((rec.loc as u32) << 8)
                    | rec.row as u32;
            }
            *deploy_changes_len = ch.len() as u32;
        }
        if flags & crate::core::APPLIED_BAGS != 0 {
            // The whole set, every time: bags are few, they never move,
            // and a full re-read cannot drift out of step with the
            // server the way an applied delta can.
            let bags = core.bags.entries();
            for (i, b) in bags.iter().enumerate() {
                bag_ids[i] = b.id;
                bag_pos[i * 3] = b.qx as f32 * POS_XZ_Q;
                bag_pos[i * 3 + 1] = b.qy as f32 * POS_Y_Q;
                bag_pos[i * 3 + 2] = b.qz as f32 * POS_XZ_Q;
            }
            *bags_len = bags.len() as u32;
        }
        if flags & crate::core::APPLIED_DEPLOY_DEFS != 0 {
            for i in 0..(core.deploy_defs_have as usize).min(MAX_DEPLOY_DEFS) {
                let def = &core.deploy_defs.defs[i];
                let row =
                    &mut deploy_defs[i * DEPLOY_DEF_ROW_WORDS..(i + 1) * DEPLOY_DEF_ROW_WORDS];
                row[0] = def.arch as u16;
                row[1] = def.placement as u16;
                row[2] = def.hp;
                row[3] = def.item;
            }
        }
        if flags & crate::core::APPLIED_STOCK != 0 {
            for (i, &(item, units)) in core.stock.iter().enumerate() {
                stock[i * 2] = item as u32;
                stock[i * 2 + 1] = units;
            }
        }
        flags
    })
}

/// Deployable records from the last stream message, packed as u32 pairs
/// ([cx << 16 | cz, open << 24 | level << 16 | loc << 8 | row]).
#[no_mangle]
pub extern "C" fn client_deploy_changes_ptr() -> *const u32 {
    with(|b| b.deploy_changes.as_ptr())
}

#[no_mangle]
pub extern "C" fn client_deploy_changes_len() -> u32 {
    with(|b| b.deploy_changes_len)
}

/// Standing death backpacks: one u32 id per bag, `client_bags_len()`
/// of them, parallel to `client_bags_ptr`. Refreshed by
/// `client_on_stream` whenever `APPLIED_BAGS` is set.
#[no_mangle]
pub extern "C" fn client_bag_ids_ptr() -> *const u32 {
    with(|b| b.bag_ids.as_ptr())
}

/// Standing death backpacks as world metres: three f32 (x, y, z) per bag,
/// in the same order as `client_bag_ids_ptr`.
#[no_mangle]
pub extern "C" fn client_bags_ptr() -> *const f32 {
    with(|b| b.bag_pos.as_ptr())
}

#[no_mangle]
pub extern "C" fn client_bags_len() -> u32 {
    with(|b| b.bags_len)
}

/// Encode an eat request for the inventory slot into the out buffer;
/// returns its length, or 0 if the slot is past the sim's array. Whether
/// the slot holds food, and whether eating it would do anything, are the
/// sim's verdict — both come back as events (survival.rs).
#[no_mangle]
pub extern "C" fn client_action_consume(slot: u32) -> u32 {
    with(|b| {
        u8::try_from(slot)
            .ok()
            .and_then(|s| encode_action_consume(s, &mut b.out_buf).ok())
            .map(|n| n as u32)
            .unwrap_or(0)
    })
}

/// Encode a move request into the out buffer; returns its length, or 0 if
/// the arguments are not a shape the wire will carry (`inventory.rs`).
///
/// Zero on refusal is this bridge's whole posture and it earns its keep
/// here more than anywhere else: a bad action frame ends the reader task
/// server-side (`server/src/net.rs`), so a UI bug that built a nonsense
/// drag would arrive as *the player being disconnected* — which is
/// precisely the reference's own failure on this verb, three times in
/// half an hour. Refusing to encode it keeps the bug local to the panel.
///
/// `bag` is 0 for a move inside your own inventory. Whether the item is
/// there, whether it fits, and whether the bag is still in reach are the
/// sim's verdict — `moved` or `move_refused`, and the refusal carries the
/// address back so the panel rolls back exactly the drag it drew.
#[no_mangle]
pub extern "C" fn client_action_move(
    bag: u32,
    from_kind: u32,
    from_slot: u32,
    to_kind: u32,
    to_slot: u32,
    count: u32,
) -> u32 {
    with(|b| {
        let (Ok(fk), Ok(fs), Ok(tk), Ok(ts), Ok(n)) = (
            u8::try_from(from_kind),
            u8::try_from(from_slot),
            u8::try_from(to_kind),
            u8::try_from(to_slot),
            u16::try_from(count),
        ) else {
            return 0;
        };
        encode_action_move(bag, fk, fs, tk, ts, n, &mut b.out_buf)
            .map(|len| len as u32)
            .unwrap_or(0)
    })
}

/// Encode an open-container request into the out buffer, or a close when
/// `kind` is `CONT_SELF` (0); returns its length, or 0 if the arguments
/// are not a shape the wire will carry.
///
/// Zero on refusal for `client_action_move`'s reason, and it is the same
/// failure: a nonsense open frame ends the reader task server-side, so a
/// panel bug would arrive as the player being disconnected. A close must
/// carry `cont = 0` — the encoder refuses the pair rather than quietly
/// dropping the handle, because a handle on a close means the caller
/// thinks it is opening something.
///
/// What comes back is `EventMsg::ContSync` on the event lane, read through
/// `client_cont_kind` / `client_cont_handle` / `client_cont_ptr` after any
/// `client_applied2() & APPLIED2_CONT`. The server may send a close of its
/// own at any time — the container despawned, or the player walked out of
/// reach — so the panel is never authoritative about its own visibility.
#[no_mangle]
pub extern "C" fn client_action_container(kind: u32, cont: u32) -> u32 {
    with(|b| {
        u8::try_from(kind)
            .ok()
            .and_then(|k| encode_action_container(k, cont, &mut b.out_buf).ok())
            .map(|n| n as u32)
            .unwrap_or(0)
    })
}

/// Encode a drink request into the out buffer; returns its length.
/// Payload-free — the sim reads the heightfield under the sender's own
/// feet, so there is nothing here for the client to aim and nothing for it
/// to get wrong (survival.rs). Whether there is water there, and whether
/// the meter has room, are the sim's verdict; both come back as events.
#[no_mangle]
pub extern "C" fn client_action_drink() -> u32 {
    with(|b| {
        encode_action_drink(&mut b.out_buf)
            .map(|n| n as u32)
            .unwrap_or(0)
    })
}

/// Encode a loot request into the out buffer for the bidi lane; returns
/// its length. Payload-free — the sim picks the nearest bag in reach, so
/// there is nothing here for the client to aim (backpack.rs).
#[no_mangle]
pub extern "C" fn client_action_loot() -> u32 {
    with(|b| {
        encode_action_loot(&mut b.out_buf)
            .map(|n| n as u32)
            .unwrap_or(0)
    })
}

/// Deploy-def table view: `DEPLOY_DEF_ROW_WORDS` u16 words per row
/// (arch, placement, hp, item), refreshed by `client_on_stream`
/// (`APPLIED_DEPLOY_DEFS`).
#[no_mangle]
pub extern "C" fn client_deploy_defs_ptr() -> *const u16 {
    with(|b| b.deploy_defs.as_ptr())
}

/// Deploy-def drip progress: total rows << 16 | rows received so far.
#[no_mangle]
pub extern "C" fn client_deploy_defs_state() -> u32 {
    with(|b| match &b.core {
        Some(core) => ((core.deploy_defs.def_count as u32) << 16) | core.deploy_defs_have as u32,
        None => 0,
    })
}

/// The last removal's address: cx << 16 | cz (pair with
/// `client_removed_info`; valid while the removal flags are set).
#[no_mangle]
pub extern "C" fn client_removed_key() -> u32 {
    with(|b| match &b.core {
        Some(core) => ((core.removed_addr.0 as u32) << 16) | core.removed_addr.1 as u32,
        None => 0,
    })
}

/// The last removal's level << 8 | loc.
#[no_mangle]
pub extern "C" fn client_removed_info() -> u32 {
    with(|b| match &b.core {
        Some(core) => ((core.removed_addr.2 as u32) << 8) | core.removed_addr.3 as u32,
        None => 0,
    })
}

/// The last raid hit's address: cx << 16 | cz (pair with
/// `client_struct_hit_info` and `client_struct_hit_hp`; valid while
/// `APPLIED_STRUCT_HIT` is set).
#[no_mangle]
pub extern "C" fn client_struct_hit_key() -> u32 {
    with(|b| match &b.core {
        Some(core) => ((core.struct_hit.0 as u32) << 16) | core.struct_hit.1 as u32,
        None => 0,
    })
}

/// The last raid hit's level << 8 | loc.
#[no_mangle]
pub extern "C" fn client_struct_hit_info() -> u32 {
    with(|b| match &b.core {
        Some(core) => ((core.struct_hit.2 as u32) << 8) | core.struct_hit.3 as u32,
        None => 0,
    })
}

/// The last raid hit's hp: left << 16 | max. `max == 0` means the def
/// table for that row has not arrived — draw nothing, not a full bar.
#[no_mangle]
pub extern "C" fn client_struct_hit_hp() -> u32 {
    with(|b| match &b.core {
        Some(core) => ((core.struct_hit.4 as u32) << 16) | core.struct_hit.5 as u32,
        None => 0,
    })
}

/// Last stock ack: [item, units] u32 pairs, `client_stock_count` rows.
#[no_mangle]
pub extern "C" fn client_stock_ptr() -> *const u32 {
    with(|b| b.stock.as_ptr())
}

#[no_mangle]
pub extern "C" fn client_stock_count() -> u32 {
    with(|b| match &b.core {
        Some(core) => core.stock_count as u32,
        None => 0,
    })
}

/// Oldest buffered deploy refusal reason; `u32::MAX` when none.
#[no_mangle]
pub extern "C" fn client_deploy_refusal_pop() -> u32 {
    with(
        |b| match b.core.as_mut().and_then(|c| c.pop_deploy_refusal()) {
            Some(reason) => reason as u32,
            None => u32::MAX,
        },
    )
}

/// Encode a deploy-place request into the out buffer; returns its length,
/// or 0 when the arguments are outside the wire's domain.
#[no_mangle]
pub extern "C" fn client_action_deploy(row: u32, cx: u32, cz: u32, level: u32, loc: u32) -> u32 {
    with(|b| {
        encode_action_deploy(
            row as u16,
            cx as u16,
            cz as u16,
            level as u8,
            loc as u8,
            &mut b.out_buf,
        )
        .map(|n| n as u32)
        .unwrap_or(0)
    })
}

/// Encode a feed request into the out buffer; returns its length, or 0
/// when the arguments are outside the wire's domain.
#[no_mangle]
pub extern "C" fn client_action_feed(cx: u32, cz: u32, level: u32) -> u32 {
    with(|b| {
        encode_action_feed(cx as u16, cz as u16, level as u8, &mut b.out_buf)
            .map(|n| n as u32)
            .unwrap_or(0)
    })
}

/// Encode a use request (toggle the door at the address) into the out
/// buffer; returns its length, or 0 when the arguments are outside the
/// wire's domain.
#[no_mangle]
pub extern "C" fn client_action_use(cx: u32, cz: u32, level: u32, loc: u32) -> u32 {
    with(|b| {
        encode_action_use(cx as u16, cz as u16, level as u8, loc as u8, &mut b.out_buf)
            .map(|n| n as u32)
            .unwrap_or(0)
    })
}

/// Encode a lock request (set the lock bit of the door at the address)
/// into the out buffer; returns its length, or 0 when the arguments are
/// outside the wire's domain. No prediction rides with it: whether the
/// door is yours is the sim's verdict, and the announcement is absolute.
#[no_mangle]
pub extern "C" fn client_action_lock(cx: u32, cz: u32, level: u32, loc: u32, locked: u32) -> u32 {
    with(|b| {
        encode_action_lock(
            cx as u16,
            cz as u16,
            level as u8,
            loc as u8,
            locked != 0,
            &mut b.out_buf,
        )
        .map(|n| n as u32)
        .unwrap_or(0)
    })
}

/// Encode an upgrade request (re-row the piece at the address into a
/// higher material) into the out buffer; returns its length, or 0 when
/// the arguments are outside the wire's domain. Nothing is predicted: an
/// upgrade never moves collision, so there is nothing for the predictor
/// to be wrong about, and the announcement re-rows the mirror.
#[no_mangle]
pub extern "C" fn client_action_upgrade(
    cx: u32,
    cz: u32,
    level: u32,
    loc: u32,
    material: u32,
) -> u32 {
    with(|b| {
        encode_action_upgrade(
            cx as u16,
            cz as u16,
            level as u8,
            loc as u8,
            material as u8,
            &mut b.out_buf,
        )
        .map(|n| n as u32)
        .unwrap_or(0)
    })
}

/// Encode a repair request (buy the damaged thing at the address back to
/// its baked hp, in its own materials) into the out buffer; returns its
/// length, or 0 when the arguments are outside the wire's domain.
///
/// **Five arguments, not `client_action_upgrade`'s four-minus-material.**
/// `NOW.md` §0e sized this against upgrade and read the arg list off it;
/// `encode_action_repair` takes a leading `deploy` bit that upgrade has no
/// analogue for, because repair addresses two stores and upgrade addresses
/// one. Pass 0 for a built piece and 1 for a deployable — the same bit
/// `encode_event_struct_hit` writes, in the same leading position, so the
/// two directions agree on which store an address means.
///
/// Nothing is predicted, for the same reason `client_action_upgrade`
/// predicts nothing and a stronger one besides: a repair moves no
/// collision, and whether you can afford it is the sim's verdict over
/// inventory the client only mirrors. The refusal arrives as a
/// `REFUSE_B_*` through `client_build_refusal_pop`.
#[no_mangle]
pub extern "C" fn client_action_repair(deploy: u32, cx: u32, cz: u32, level: u32, loc: u32) -> u32 {
    with(|b| {
        encode_action_repair(
            deploy != 0,
            cx as u16,
            cz as u16,
            level as u8,
            loc as u8,
            &mut b.out_buf,
        )
        .map(|n| n as u32)
        .unwrap_or(0)
    })
}

/// Plant the held throwable against the structure at the address
/// (`charge.rs`). `client_action_repair`'s arguments exactly, including
/// the leading store bit and its meaning: 0 for a built piece, 1 for a
/// deployable.
///
/// Nothing is predicted, and here the reason is sharper than repair's.
/// Whether the hand actually holds a charge, whether the wall is in reach,
/// and whether the store has room are all the sim's verdicts; but the one
/// that matters is that a *predicted* charge would draw a fuse burning on
/// a wall that the server never armed, and the client would then have to
/// un-draw an explosion. The refusal arrives as a `REFUSE_B_*` through
/// `client_build_refusal_pop` — the same five codes repair uses, because
/// they are the same five facts.
#[no_mangle]
pub extern "C" fn client_action_throw(deploy: u32, cx: u32, cz: u32, level: u32, loc: u32) -> u32 {
    with(|b| {
        encode_action_throw(
            deploy != 0,
            cx as u16,
            cz as u16,
            level as u8,
            loc as u8,
            &mut b.out_buf,
        )
        .map(|n| n as u32)
        .unwrap_or(0)
    })
}

/// The last planted charge's cell key (`cx << 16 | cz`); pair with
/// `client_charge_info` and `client_charge_fuse`. Valid while
/// `client_applied2() & APPLIED2_CHARGE` is set.
#[no_mangle]
pub extern "C" fn client_charge_key() -> u32 {
    with(|b| match &b.core {
        Some(core) => ((core.charge_placed.0 as u32) << 16) | core.charge_placed.1 as u32,
        None => 0,
    })
}

/// That charge's `deploy << 16 | level << 8 | loc`, and its row in the
/// high byte pair. Packed as `store << 24 | level << 16 | loc << 8 | row`
/// — `EV_CHARGE_PLACED`'s own `b` field, unchanged, so a caller that
/// already unpacks a struct hit unpacks this with the same shifts.
#[no_mangle]
pub extern "C" fn client_charge_info() -> u32 {
    with(|b| match &b.core {
        Some(core) => {
            (if core.charge_deploy { 1 << 24 } else { 0 })
                | ((core.charge_placed.2 as u32) << 16)
                | ((core.charge_placed.3 as u32) << 8)
                | core.charge_placed.4 as u32
        }
        None => 0,
    })
}

/// Ticks left on that charge's fuse at the moment it was planted. Never
/// zero when the flag is set — the sim refuses a zero fuse and the decoder
/// refuses one on the wire — so a zero here means no charge has landed
/// yet, not a charge about to blow.
#[no_mangle]
pub extern "C" fn client_charge_fuse() -> u32 {
    with(|b| match &b.core {
        Some(core) => core.charge_placed.5 as u32,
        None => 0,
    })
}

/// Toggle the door at the address optimistically, on this client's own
/// input (NETCODE.md §6.1). Returns the predicted open state (0 or 1),
/// or `u32::MAX` when nothing was predicted — no known door there, or a
/// prediction already outstanding. Call it beside `client_action_use`:
/// the renderer redraws on a 0/1, and leaves the door alone otherwise.
#[no_mangle]
pub extern "C" fn client_predict_door(cx: u32, cz: u32, level: u32, loc: u32) -> u32 {
    with(|b| {
        match b
            .core
            .as_mut()
            .and_then(|c| c.predict_door(cx as u16, cz as u16, level as u8, loc as u8))
        {
            Some(open) => open as u32,
            None => u32::MAX,
        }
    })
}

/// Piece records from the last stream message, packed as u32 pairs
/// ([cx << 16 | cz, level << 16 | loc << 8 | row]).
#[no_mangle]
pub extern "C" fn client_piece_changes_ptr() -> *const u32 {
    with(|b| b.piece_changes.as_ptr())
}

#[no_mangle]
pub extern "C" fn client_piece_changes_len() -> u32 {
    with(|b| b.piece_changes_len)
}

/// Piece-def table view: `PIECE_DEF_ROW_WORDS` u16 words per piece row
/// (shape, material, hp, n_costs, then item/count per cost slot),
/// refreshed by `client_on_stream` (`APPLIED_PIECE_DEFS`).
#[no_mangle]
pub extern "C" fn client_piece_defs_ptr() -> *const u16 {
    with(|b| b.piece_defs.as_ptr())
}

/// Piece-def drip progress: total rows << 16 | rows received so far.
#[no_mangle]
pub extern "C" fn client_piece_defs_state() -> u32 {
    with(|b| match &b.core {
        Some(core) => ((core.piece_defs.piece_count as u32) << 16) | core.piece_defs_have as u32,
        None => 0,
    })
}

/// Oldest buffered build refusal reason; `u32::MAX` when none.
#[no_mangle]
pub extern "C" fn client_build_refusal_pop() -> u32 {
    with(
        |b| match b.core.as_mut().and_then(|c| c.pop_build_refusal()) {
            Some(reason) => reason as u32,
            None => u32::MAX,
        },
    )
}

/// Encode a place request into the out buffer for the bidi lane; returns
/// its length, or 0 when the arguments are outside the wire's domain.
#[no_mangle]
pub extern "C" fn client_action_place(row: u32, cx: u32, cz: u32, level: u32, loc: u32) -> u32 {
    with(|b| {
        encode_action_place(
            row as u16,
            cx as u16,
            cz as u16,
            level as u8,
            loc as u8,
            &mut b.out_buf,
        )
        .map(|n| n as u32)
        .unwrap_or(0)
    })
}

/// Craft queue view: `CRAFT_QUEUE` × (recipe, remaining) u16 words,
/// refreshed by `client_on_stream` (`APPLIED_CRAFT_Q`).
#[no_mangle]
pub extern "C" fn client_craft_jobs_ptr() -> *const u16 {
    with(|b| b.craft_jobs.as_ptr())
}

/// Craft queue summary: live job count << 16 | head-unit remaining ticks
/// at the last announce (the UI counts down between messages).
#[no_mangle]
pub extern "C" fn client_craft_q() -> u32 {
    with(|b| match &b.core {
        Some(core) => ((core.jobs_count as u32) << 16) | core.craft_eta_ticks as u32,
        None => 0,
    })
}

/// Recipe table view: `RECIPE_ROW_WORDS` u16 words per recipe index
/// (output, out_count, ticks lo/hi, station, n_inputs, then item/count
/// per input slot), refreshed by `client_on_stream` (`APPLIED_RECIPES`).
#[no_mangle]
pub extern "C" fn client_recipes_ptr() -> *const u16 {
    with(|b| b.recipes.as_ptr())
}

/// Recipe drip progress: total rows << 16 | rows received so far.
#[no_mangle]
pub extern "C" fn client_recipes_state() -> u32 {
    with(|b| match &b.core {
        Some(core) => ((core.recipes.recipe_count as u32) << 16) | core.recipes_have as u32,
        None => 0,
    })
}

/// Oldest buffered craft-done toast as `item << 16 | added`; `u32::MAX`
/// when none.
#[no_mangle]
pub extern "C" fn client_craft_pop() -> u32 {
    with(
        |b| match b.core.as_mut().and_then(|c| c.pop_craft_toast()) {
            Some((item, added)) => ((item as u32) << 16) | added as u32,
            None => u32::MAX,
        },
    )
}

/// Own food and water as `food << 16 | water`, and their ceilings as
/// `max_food << 16 | max_water`. Both zero in the ceilings means no
/// `Vitals` has arrived — a shard whose content has no `[survival]`
/// section never sends one, and the HUD reads that as "no meters to draw".
#[no_mangle]
pub extern "C" fn client_vitals() -> u32 {
    with(|b| {
        b.core
            .as_ref()
            .map(|c| ((c.food as u32) << 16) | c.water as u32)
            .unwrap_or(0)
    })
}

#[no_mangle]
pub extern "C" fn client_vitals_max() -> u32 {
    with(|b| {
        b.core
            .as_ref()
            .map(|c| ((c.max_food as u32) << 16) | c.max_water as u32)
            .unwrap_or(0)
    })
}

/// The last eat: `refused << 24 | slot << 16 | item`, where `refused` is a
/// `sim_core::survival::REFUSE_C_*` code and 0 means the eat landed.
#[no_mangle]
pub extern "C" fn client_consume() -> u32 {
    with(|b| {
        b.core
            .as_ref()
            .map(|c| {
                let item = c.last_eat >> 16;
                let slot = c.last_eat & 0xFFFF;
                ((c.last_eat_refused as u32) << 24) | (slot << 16) | (item & 0xFFFF)
            })
            .unwrap_or(0)
    })
}

/// The last drink: `water restored << 16 | hp it cost`. Zero means none
/// has landed this session. A *refused* drink is not here — it arrives on
/// the eat readout with a `REFUSE_C_*` code, one refusal channel for the
/// whole survival module (survival.rs).
#[no_mangle]
pub extern "C" fn client_drank() -> u32 {
    with(|b| b.core.as_ref().map(|c| c.last_drink).unwrap_or(0))
}

/// Own health as `hp << 16 | max`. `max == 0` means no health reading has
/// arrived — a shard whose content disarms combat never sends one, and the
/// HUD reads that as "no vitals to draw", not as "dead".
#[no_mangle]
pub extern "C" fn client_health() -> u32 {
    with(|b| {
        b.core
            .as_ref()
            .map(|c| ((c.hp as u32) << 16) | c.hp_max as u32)
            .unwrap_or(0)
    })
}

/// Oldest buffered hitmarker (damage this client's swing dealt);
/// `u32::MAX` when none.
#[no_mangle]
pub extern "C" fn client_hit_pop() -> u32 {
    with(|b| match b.core.as_mut().and_then(|c| c.pop_hit()) {
        Some(damage) => damage as u32,
        None => u32::MAX,
    })
}

/// Word 1 of the applied word — the `core::APPLIED2_*` flags for the last
/// `client_on_stream`, valid until the next one.
///
/// Read it after **every** `client_on_stream`, not on a bit of the word 0
/// return: that word has no spare bit to announce this one with, which is
/// the whole reason word 1 exists. It is cheap (a load, no view refresh)
/// and it is zero on any message that set nothing in it, so an
/// unconditional read cannot see a stale verdict.
///
/// Today it carries one flag, `APPLIED2_MOVE` — the move verdict the panel
/// reads through `client_move_readout` and `client_move_payload`.
#[no_mangle]
pub extern "C" fn client_applied2() -> u32 {
    with(|b| b.core.as_ref().map_or(0, |c| c.applied2()))
}

/// The last move's verdict: `refusal reason << 24 | to slot << 16 |
/// from kind << 8 | from slot`, with the *to kind* deducible from the
/// pair the panel sent. Zero reason ⇒ the move landed, and
/// `client_move_payload` then holds what actually moved.
///
/// Packed rather than returned as five calls because the panel reads it
/// on one `APPLIED2_MOVE` flag and must not see half of one verdict beside
/// half of the next — `client_consume`'s shape, for `client_consume`'s
/// reason.
#[no_mangle]
pub extern "C" fn client_move_readout() -> u32 {
    with(|b| {
        b.core
            .as_ref()
            .map(|c| {
                let addr = c.last_move;
                ((c.last_move_refused as u32) << 24)
                    | ((addr & 0xFF) << 16)
                    | (((addr >> 24) & 0xFF) << 8)
                    | ((addr >> 16) & 0xFF)
            })
            .unwrap_or(0)
    })
}

/// What the last accepted move carried: count << 16 | the item that left
/// the source slot. Zero when the last verdict was a refusal. See
/// `ClientCore::last_move_count` for why the item is here and the slot
/// contents are not.
#[no_mangle]
pub extern "C" fn client_move_payload() -> u32 {
    with(|b| b.core.as_ref().map(|c| c.last_move_count).unwrap_or(0))
}

/// Oldest buffered death's victim id; `u32::MAX` when none. The killer of
/// the death this call returned is then in `client_death_killer` — one pop
/// hands the caller a whole feed line.
#[no_mangle]
pub extern "C" fn client_death_pop() -> u32 {
    with(|b| {
        b.core
            .as_mut()
            .and_then(|c| c.pop_death())
            .unwrap_or(u32::MAX)
    })
}

/// Killer of the death `client_death_pop` returned last.
#[no_mangle]
pub extern "C" fn client_death_killer() -> u32 {
    with(|b| b.core.as_ref().map(|c| c.last_death_killer).unwrap_or(0))
}

/// The death screen, packed: `dead << 24 | woke_on_bag << 16 | cause`.
/// Zero means alive and never yet woken — which is also what a client with
/// no core reads, so the overlay is closed by default and only an actual
/// `Death` can open it.
///
/// Packed rather than four calls for the reason every readout here is:
/// this is polled from the RAF loop on a flag, and one `u32` across the
/// wasm boundary is one call instead of four. The killer, the weapon and
/// the range are the rest of the sentence and ride their own two calls
/// below — `client_death_killer` already existed for the feed, and reusing
/// it here would have coupled the screen to the ring's pop cursor.
#[no_mangle]
pub extern "C" fn client_death_screen() -> u32 {
    with(|b| {
        b.core
            .as_ref()
            .map(|c| {
                ((c.dead as u32) << 24) | ((c.woke_on_bag as u32) << 16) | c.own_death_cause as u32
            })
            .unwrap_or(0)
    })
}

/// Who killed the body this client is driving, for the death screen. Not
/// `client_death_killer`: that one moves with the kill feed's pop cursor,
/// and the screen must not change its sentence because a stranger died.
#[no_mangle]
pub extern "C" fn client_death_by() -> u32 {
    with(|b| b.core.as_ref().map(|c| c.own_death_killer).unwrap_or(0))
}

/// The rest of the sentence: `item << 16 | range_cm`. `item` is
/// `NO_ITEM` (0xffff) when the world did it rather than a hand, which is
/// the same sentinel the inventory and catalog already use.
#[no_mangle]
pub extern "C" fn client_death_weapon() -> u32 {
    with(|b| {
        b.core
            .as_ref()
            .map(|c| ((c.own_death_item as u32) << 16) | c.own_death_range_cm as u32)
            .unwrap_or(0)
    })
}

/// Encode an answer to the death screen into the out buffer; returns its
/// length. `on_bag` nonzero asks for the nearest of your own ready bags,
/// zero asks for a beach (ALPHA.md §1, "choose beach or a bag"). Whether a
/// bag of yours is ready is the sim's verdict, and it comes back as the
/// `Respawn` event's own `on_bag` bit — so a client that asked for a bag
/// and got a beach is told, rather than left to guess from a coordinate.
#[no_mangle]
pub extern "C" fn client_action_respawn(on_bag: u32) -> u32 {
    with(|b| {
        encode_action_respawn(on_bag != 0, &mut b.out_buf)
            .map(|n| n as u32)
            .unwrap_or(0)
    })
}

/// Oldest buffered craft refusal reason; `u32::MAX` when none.
#[no_mangle]
pub extern "C" fn client_craft_refusal_pop() -> u32 {
    with(
        |b| match b.core.as_mut().and_then(|c| c.pop_craft_refusal()) {
            Some(reason) => reason as u32,
            None => u32::MAX,
        },
    )
}

/// Encode a craft request into the out buffer for the bidi lane; returns
/// its length, or 0 when the arguments are outside the wire's domain.
#[no_mangle]
pub extern "C" fn client_action_craft(recipe: u32, count: u32) -> u32 {
    with(|b| {
        encode_action_craft(recipe as u16, count as u16, &mut b.out_buf)
            .map(|n| n as u32)
            .unwrap_or(0)
    })
}

/// Encode a cancel of queue job `index` into the out buffer; returns its
/// length, or 0 when the index is outside the queue.
#[no_mangle]
pub extern "C" fn client_action_cancel(index: u32) -> u32 {
    with(|b| {
        encode_action_cancel(index as u16, &mut b.out_buf)
            .map(|n| n as u32)
            .unwrap_or(0)
    })
}

/// Encode a chat line for the bidi lane. The text is read from the in
/// buffer — JS writes it there and calls straight through, the same
/// synchronous write-then-call the incoming path uses, so nothing else
/// can be mid-flight in that buffer. Returns the frame's length, or 0
/// when the line is empty, over-long, not UTF-8, or carries a control
/// character (`ChatText::sanitize`): a refusal here is the client
/// declining to send, not the server declining to relay.
#[no_mangle]
pub extern "C" fn client_action_chat(len: u32, global: u32) -> u32 {
    with(|b| {
        let len = len as usize;
        if len > IN_CAP {
            return 0;
        }
        // Split the borrow: the source is the in buffer, the target the
        // out buffer, and they are distinct fields.
        let Bridge {
            in_buf, out_buf, ..
        } = b;
        encode_chat(&in_buf[..len], global != 0, out_buf)
            .map(|n| n as u32)
            .unwrap_or(0)
    })
}

/// Pop one received chat line into the chat view; 1 if one was there, 0
/// if the ring is empty. Layout at `client_chat_ptr()`: speaker id as 4
/// LE bytes, then the global flag, then the byte length, then the line.
#[no_mangle]
pub extern "C" fn client_chat_pop() -> u32 {
    with(|b| {
        let Some(core) = b.core.as_mut() else {
            return 0;
        };
        let Some((from, global, text)) = core.pop_chat() else {
            return 0;
        };
        b.chat[..4].copy_from_slice(&from.to_le_bytes());
        b.chat[4] = global as u8;
        b.chat[5] = text.len() as u8;
        b.chat[6..6 + text.len()].copy_from_slice(text.as_bytes());
        1
    })
}

#[no_mangle]
pub extern "C" fn client_chat_ptr() -> *const u8 {
    with(|b| b.chat.as_ptr())
}

/// Pairs of (cell key, harvested 0/1) from the last stream message.
#[no_mangle]
pub extern "C" fn client_slot_changes_ptr() -> *const u32 {
    with(|b| b.changes.as_ptr())
}

#[no_mangle]
pub extern "C" fn client_slot_changes_len() -> u32 {
    with(|b| b.changes_len)
}

/// Own inventory view: `INV_SLOTS` × (item, count) u16 words.
#[no_mangle]
pub extern "C" fn client_inv_ptr() -> *const u16 {
    with(|b| b.inv.as_ptr())
}

/// Open container view: `INV_SLOTS` × (item, count) u16 words, the same
/// layout as `client_inv_ptr`. Meaningful only while `client_cont_kind()`
/// is nonzero; a box fills the first `BOX_SLOTS` and the tail stays zero.
#[no_mangle]
pub extern "C" fn client_cont_ptr() -> *const u16 {
    with(|b| b.cont.as_ptr())
}

/// Which container is open: `inventory::CONT_SELF` (0) for none, else
/// `CONT_BAG` or `CONT_BOX`. **The server owns this, not the panel** — it
/// goes to zero on its own when the container despawns or the player walks
/// out of reach, so a panel that draws on a nonzero value can never
/// outlive the reach a move will be judged against.
#[no_mangle]
pub extern "C" fn client_cont_kind() -> u32 {
    with(|b| b.core.as_ref().map(|c| c.cont_kind as u32).unwrap_or(0))
}

/// The open container's handle — a bag id, or a packed `box_key`. Zero
/// when nothing is open. This is the value a move must carry as its `bag`
/// argument to `client_action_move`, and passing anything else is how the
/// two ends come to disagree about which container a drag touched.
#[no_mangle]
pub extern "C" fn client_cont_handle() -> u32 {
    with(|b| b.core.as_ref().map(|c| c.cont_handle).unwrap_or(0))
}

/// Item-name rows: `1 + MAX_ITEM_NAME_BYTES` bytes per index (len, bytes).
#[no_mangle]
pub extern "C" fn client_catalog_ptr() -> *const u8 {
    with(|b| b.catalog.as_ptr())
}

/// Oldest buffered gather toast as `item << 16 | added`; `u32::MAX` when
/// none (a real toast never reaches it: items cap far below u16::MAX).
#[no_mangle]
pub extern "C" fn client_toast_pop() -> u32 {
    with(|b| match b.core.as_mut().and_then(|c| c.pop_toast()) {
        Some((item, added)) => ((item as u32) << 16) | added as u32,
        None => u32::MAX,
    })
}

/// The own weak-spot mark's cell key (cx << 16 | cz), or `u32::MAX` when
/// no mark is up. Refreshed by `client_on_stream` (`APPLIED_MARK` flag).
#[no_mangle]
pub extern "C" fn client_weak_mark_cell() -> u32 {
    with(|b| match &b.core {
        Some(core) => core.mark_cell,
        None => u32::MAX,
    })
}

/// The mark detail: `weak_hit << 8 | mark8` — heading over the 256-entry
/// yaw LUT (0 faces +Z, rotating toward +X) plus whether the announcing
/// hit landed weak. Meaningless while no mark is up.
#[no_mangle]
pub extern "C" fn client_weak_mark_info() -> u32 {
    with(|b| match &b.core {
        Some(core) => ((core.mark_weak_hit as u32) << 8) | core.mark8 as u32,
        None => 0,
    })
}

/// Whether the scatter slot at cell (cx, cz) is currently harvested —
/// the renderer's build-time check for chunks streaming in.
#[no_mangle]
pub extern "C" fn client_cell_harvested(cx: u32, cz: u32) -> u32 {
    with(|b| match &b.core {
        Some(core) => {
            core.harvested
                .contains(sim_core::gather::cell_key(cx as u16, cz as u16)) as u32
        }
        None => 0,
    })
}

#[no_mangle]
pub extern "C" fn client_set_input(
    buttons: u32,
    yaw: u32,
    pitch: u32,
    move_x: i32,
    move_z: i32,
    sel: u32,
) {
    with(|b| {
        if let Some(core) = b.core.as_mut() {
            core.set_input(
                buttons as u8,
                yaw as u16,
                pitch as u8,
                move_x.clamp(-127, 127) as i8,
                move_z.clamp(-127, 127) as i8,
                sel as u8,
            );
        }
    })
}

/// Advance real time; returns the fixed client ticks that ran.
#[no_mangle]
pub extern "C" fn client_advance(dt_ms: f64) -> u32 {
    with(|b| b.core.as_mut().map(|c| c.advance(dt_ms)).unwrap_or(0))
}

/// The due input datagram, encoded into the out buffer; 0 when none.
#[no_mangle]
pub extern "C" fn client_poll_input() -> u32 {
    with(|b| {
        let Bridge {
            core: Some(core),
            out_buf,
            ..
        } = b
        else {
            return 0;
        };
        core.poll_input(out_buf) as u32
    })
}

/// Fill the render buffer and remote-id buffer; returns the remote count.
/// Also decays the correction offset — call exactly once per render frame.
///
/// Float layout: [0] started · [1..4] own render x/y/z (smoothed) ·
/// [4] vy m/s · [5] grounded · [6] nudge isn't wire state client-side, so:
/// [6] correction error m · [7] mispredictions · [8] snapshots applied ·
/// [9] server tick estimate · [10] client tick · [11] hard resyncs ·
/// [12] reserved · [13] remote count · then per remote k at
/// `14 + k*8`: x, y, z, yaw (0..65536), pitch (0..255), live, vy?=0, 0.
#[no_mangle]
pub extern "C" fn client_render() -> u32 {
    with(|b| {
        let Bridge {
            core: Some(core),
            render,
            remote_ids,
            ..
        } = b
        else {
            return 0;
        };
        core.predict.decay_error();
        let p = core.predict.render_position();
        render[0] = if core.predict.started { 1.0 } else { 0.0 };
        render[1] = p[0];
        render[2] = p[1];
        render[3] = p[2];
        render[4] = core.predict.body.qvy as f32 * sim_core::movement::VEL_Q;
        render[5] = if core.predict.body.grounded { 1.0 } else { 0.0 };
        render[6] = core.predict.error_magnitude();
        render[7] = core.predict.mispredictions as f32;
        render[8] = core.snapshots_applied as f32;
        render[9] = core.clock.server_est as f32;
        render[10] = core.clock.client_tick as f32;
        render[11] = core.clock.resyncs as f32;
        render[12] = 0.0;
        let at = core.render_tick();
        let mut n = 0usize;
        let mut rs = crate::interp::RemoteState::default();
        // Collect ids first: sample borrows interp immutably.
        let mut ids = [0u32; MAX_SNAPSHOT_ENTITIES];
        let mut n_ids = 0usize;
        for id in core.interp.ids() {
            ids[n_ids] = id;
            n_ids += 1;
        }
        for &id in &ids[..n_ids] {
            if core.interp.sample(id, at, &mut rs) {
                let base = OWN_FLOATS + 1 + n * REMOTE_FLOATS;
                remote_ids[n] = id;
                render[base] = rs.x;
                render[base + 1] = rs.y;
                render[base + 2] = rs.z;
                render[base + 3] = rs.yaw;
                render[base + 4] = rs.pitch;
                render[base + 5] = if rs.live { 1.0 } else { 0.0 };
                render[base + 6] = 0.0;
                render[base + 7] = 0.0;
                n += 1;
            }
        }
        render[OWN_FLOATS] = n as f32;
        n as u32
    })
}

#[no_mangle]
pub extern "C" fn client_render_ptr() -> *const f32 {
    with(|b| b.render.as_ptr())
}

#[no_mangle]
pub extern "C" fn client_remote_ids_ptr() -> *const u32 {
    with(|b| b.remote_ids.as_ptr())
}

// ---------------------------------------------------------------------------
// Terrain: the shared worldgen, for the render worker (TERRAIN.md §4)
// ---------------------------------------------------------------------------

#[no_mangle]
pub extern "C" fn terrain_height_at(seed: u64, x: f32, z: f32) -> f32 {
    terrain::height(seed, x, z)
}

#[no_mangle]
pub extern "C" fn terrain_moisture_at(seed: u64, x: f32, z: f32) -> f32 {
    terrain::moisture(seed, x, z)
}

/// Fill an `n × n` height grid at `(x0 + i·step, z0 + j·step)`, row-major
/// by j. Returns the float count written, 0 if `n` exceeds the buffer.
#[no_mangle]
pub extern "C" fn terrain_fill_heights(seed: u64, x0: f32, z0: f32, n: u32, step: f32) -> u32 {
    with(|b| {
        let n = n as usize;
        if n == 0 || n > HEIGHTS_MAX_N {
            return 0;
        }
        for j in 0..n {
            let z = z0 + j as f32 * step;
            for i in 0..n {
                b.heights[j * n + i] = terrain::height(seed, x0 + i as f32 * step, z);
            }
        }
        (n * n) as u32
    })
}

#[no_mangle]
pub extern "C" fn terrain_heights_ptr() -> *const f32 {
    with(|b| b.heights.as_ptr())
}

/// Resolve the scatter slots of a `cells × cells` block starting at cell
/// `(cx0, cz0)`. Writes 8 floats per resolved slot — occupant, x, y, z,
/// yaw (0..255), scale, cx, cz — and returns the slot count.
#[no_mangle]
pub extern "C" fn terrain_fill_slots(seed: u64, cx0: i32, cz0: i32, cells: u32) -> u32 {
    with(|b| {
        let cells = (cells as usize).min(SLOTS_MAX_CELLS) as i32;
        let table = ScatterTable::alpha_default();
        // Once per SESSION, not once per batch and certainly not per cell:
        // the argmax sweeps the whole road ring. Keyed on the seed, which is
        // its only input, so a shard change re-resolves and nothing else can.
        let haven = match b.haven {
            Some((s, h)) if s == seed => h,
            _ => {
                let h = terrain::haven(seed);
                b.haven = Some((seed, h));
                h
            }
        };
        let mut n = 0usize;
        for dz in 0..cells {
            for dx in 0..cells {
                let s = terrain::scatter(seed, &table, &haven, cx0 + dx, cz0 + dz);
                if s.occupant == terrain::Occupant::None {
                    continue;
                }
                let base = n * SLOT_FLOATS;
                b.slots[base] = s.occupant as u8 as f32;
                b.slots[base + 1] = s.x;
                b.slots[base + 2] = s.y;
                b.slots[base + 3] = s.z;
                b.slots[base + 4] = s.yaw as f32;
                b.slots[base + 5] = s.scale;
                b.slots[base + 6] = (cx0 + dx) as f32;
                b.slots[base + 7] = (cz0 + dz) as f32;
                n += 1;
            }
        }
        n as u32
    })
}

#[no_mangle]
pub extern "C" fn terrain_slots_ptr() -> *const f32 {
    with(|b| b.slots.as_ptr())
}

/// The ground material's four identity weights, packed one per byte in
/// (sand, grass, litter, rock) order, little end first.
///
/// This exists so the terrain worker has NO splat law of its own. It used to
/// carry a JS copy of the bands and the cliff override, which is the exact
/// arrangement `threejs-procedural-fields` rejects — "geometry and shading
/// claim the same feature but evaluate different functions" — and it is now
/// load-bearing twice over, because the clutter population draws its kind
/// from these same weights. One law, one language, no drift to gate against.
#[no_mangle]
pub extern "C" fn terrain_splat_from(h: f32, moist: f32, slope: f32) -> u32 {
    let w = terrain::splat_from(h, moist, slope);
    u32::from_le_bytes(w)
}

/// Fill one 16 m clutter tile: the uniform grid, then the prop-base skirts.
/// Returns the element count; the floats are at `terrain_clutter_ptr()`,
/// `CLUTTER_FLOATS` apart, valid until the next call. Total coverage means
/// this returns ~`CLUTTER_PER_TILE` on land and 0 at sea, plus up to
/// `SKIRT_PER_TILE` of skirt — so a caller sizing a pool sizes it for
/// `CLUTTER_TILE_CAP`.
///
/// The two populations share one buffer, one call and one element layout on
/// purpose: they are the same four kinds drawn through the same four pools,
/// so the client streams them together and pays no second material, no second
/// program (the prewarm count gate's subject) and no second draw call.
#[no_mangle]
pub extern "C" fn terrain_fill_clutter(seed: u64, tile_x: i32, tile_z: i32) -> u32 {
    with(|b| {
        let mut buf = [terrain::CLUTTER_NONE; terrain::CLUTTER_TILE_CAP];
        let grid =
            terrain::clutter_fill(seed, tile_x, tile_z, &mut buf[..terrain::CLUTTER_PER_TILE]);
        // The skirts need the scatter grid resolved, which needs the same
        // once-per-session haven `terrain_fill_slots` memoizes. Same key, same
        // reason: the argmax sweeps the whole road ring.
        let table = ScatterTable::alpha_default();
        let haven = match b.haven {
            Some((s, h)) if s == seed => h,
            _ => {
                let h = terrain::haven(seed);
                b.haven = Some((seed, h));
                h
            }
        };
        let mut skirt = [terrain::CLUTTER_NONE; terrain::SKIRT_PER_TILE];
        let ns = terrain::skirt_fill(seed, &table, &haven, tile_x, tile_z, &mut skirt);
        buf[grid..grid + ns].copy_from_slice(&skirt[..ns]);
        let n = grid + ns;
        for (i, e) in buf.iter().take(n).enumerate() {
            let base = i * CLUTTER_FLOATS;
            b.clutter[base] = e.kind as u8 as f32;
            b.clutter[base + 1] = e.x;
            b.clutter[base + 2] = e.y;
            b.clutter[base + 3] = e.z;
            b.clutter[base + 4] = e.yaw as f32;
            b.clutter[base + 5] = e.scale;
        }
        n as u32
    })
}

#[no_mangle]
pub extern "C" fn terrain_clutter_ptr() -> *const f32 {
    with(|b| b.clutter.as_ptr())
}
