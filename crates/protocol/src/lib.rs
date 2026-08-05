//! Packet schemas, bit-level codec, quantization tables, golden tests.
//! Shared native/wasm. Zero game logic (DESIGN.md §4) — the sim's types
//! (`InputFrame`, the quantized body fields) come from `sim-core`; this
//! crate only says how they cross the wire.
//!
//! Two datagram schemas, v0 (DESIGN.md §5.4/§5.5, NETCODE.md §3):
//!
//! **Input, C→S** — `kind:3 · snapshot_ack:16 · ack_bits:32 ·
//! first_client_tick:32 · frame_count:4`, then if any frames:
//! `first_seq:16` and per frame `buttons:8 · yaw:16 · pitch:8 · move_x:8 ·
//! move_z:8 · sel:3` (hotbar selector 0–5; 6–7 refuse as malformed).
//! Frames are the client's unacked tail, oldest first, seq-consecutive by
//! construction (seq rides the wire once).
//!
//! **Snapshot, S→C** — `kind:3 · tick:32 · baseline_age:8 ·
//! last_executed_seq:16 · nudge:2 · removed_count:7 · entity_count:7`,
//! then removed ids (`u32` each), then entity records. `baseline_age == 0`
//! is the canonical zero-state (NETCODE.md §3, the Quake-3 move): every
//! record absolute, no baseline needed. Otherwise records may delta
//! against the client's state at `tick − baseline_age`.
//!
//! Encode and decode both live on hot paths, so both are allocation-free
//! and total: every failure is a `WireError`, never a panic — decode eats
//! arbitrary bytes (client-driven on the server side), and the golden
//! suite flips bits to prove it.

pub mod bits;
pub mod chat;
pub mod event;
pub mod goldens;

pub use bits::WireError;
use bits::{BitReader, BitWriter};
pub use chat::{decode_chat, encode_chat, ChatMsg, ChatText, CHAT_MAX_BYTES};
pub use event::{
    decode_event, encode_event_bag_dropped, encode_event_bag_removed, encode_event_bag_sync,
    encode_event_build_refused, encode_event_catalog, encode_event_chat,
    encode_event_consume_refused, encode_event_consumed, encode_event_cont_sync,
    encode_event_craft_done, encode_event_craft_q, encode_event_craft_refused, encode_event_death,
    encode_event_deploy_defs, encode_event_deploy_placed, encode_event_deploy_refused,
    encode_event_deploy_sync, encode_event_door, encode_event_drank, encode_event_gather,
    encode_event_health, encode_event_hit, encode_event_inv, encode_event_move_refused,
    encode_event_moved, encode_event_piece_defs, encode_event_piece_placed,
    encode_event_piece_sync, encode_event_recipes, encode_event_removed, encode_event_respawn,
    encode_event_slot_change, encode_event_slot_sync, encode_event_stock, encode_event_struct_hit,
    encode_event_vitals, encode_event_weak_mark, EventMsg, InvSlot, ItemCatalog, WireBag,
    BAG_SYNC_BATCH, CATALOG_BATCH, CONT_SYNC_BATCH, DEPLOY_DEFS_BATCH, DEPLOY_SYNC_BATCH,
    MAX_EVENT_MSG_BYTES, MAX_ITEM_NAME_BYTES, PIECE_DEFS_BATCH, PIECE_SYNC_BATCH, RECIPE_BATCH,
    SLOT_SYNC_BATCH,
};
use sim_core::input::InputFrame;
use sim_core::limits::{HOTBAR_SLOTS, MAX_INPUT_FRAMES, MAX_SNAPSHOT_ENTITIES};

/// Wire protocol version. Bumps only with a packet-layout change and
/// regenerated goldens in the same commit (CLAUDE.md wall 6). v1 added
/// the reliable event lane (`KIND_EVENT`, `event.rs`). v2 added the
/// hotbar selector to every input frame and the weak-mark event subtype.
/// v3 added the C→S action lane (`KIND_ACTION`: craft request / cancel on
/// the bidi stream) and the craft event subtypes (queue, done, refused,
/// recipe catalog). v4 added the build lane: the place action and the
/// piece event subtypes (placed, join sync, refused, piece-def catalog).
/// v5 added the deployable lane — deploy-place and feed actions (the
/// action subtype field widened 2 → 3 bits), the deploy event subtypes
/// (placed, join sync, refused, def catalog, stock ack) and the
/// piece/deploy removal broadcasts (the event subtype field widened
/// 4 → 5 bits). v6 added the door lane: the use action (address only)
/// and the door event subtype, and every placed-deployable record on the
/// wire grew its open bit — so a v5 deploy record parses off-by-one from
/// here on, the hello gate refuses the pair. v7 added the welcome's `dev`
/// bit: the shard states whether it is a dev shard, which is what gates
/// the client's dev affordances. v8 added the lock lane: the lock action
/// (address + the absolute bit), the door event grew its locked bit, and
/// every placed-deployable record on the wire grew one too — so a v7
/// deploy record parses off-by-one from here on, exactly like v6's open
/// bit, and the hello gate refuses the pair. v9 added the upgrade lane:
/// the upgrade action (address + the target material), which fills the
/// last code the 3-bit action subtype field had left. No S→C layout
/// moved — an upgrade re-rows an address, which is what the piece-placed
/// broadcast already says. v10 added the chat lane: `KIND_CHAT` C→S (the
/// last code the 3-bit kind field had left) and the chat event subtype
/// S→C — chat is player-authored text, not a sim command, so it gets a
/// kind of its own rather than an action subtype it could not have had
/// anyway. v11 added the combat lane: three event subtypes (hit, health,
/// death) for melee v0 — no datagram layout moved, because a player's hp
/// is not on the snapshot. That is deliberate and it is the cheap half of
/// the choice: hp is an own-fact for the player it belongs to and a
/// one-shot broadcast when it reaches zero, so it rides the reliable lane
/// at the moment it changes instead of costing every entity record a
/// field on every tick. When remote health bars or a downed state need it
/// continuously, that is a snapshot-widening bump of its own, not a
/// retrofit of this one. v12 added the death-backpack lane: the loot
/// action — the ninth, which widened the action subtype field 3 → 4 bits,
/// so **every C→S action message moved by one bit** — and three event
/// subtypes (bag dropped, join sync, removed). No datagram layout moved:
/// a bag is class-S furniture on the reliable lane, not a snapshot
/// entity, for the same reason a placed deployable is. v13 added the raid
/// lane: the struct-hit event subtype, the 31st — which is why the same
/// bump **widened the event subtype field 5 → 6 bits, moving every S→C
/// event message by one bit.** Taking the widening here rather than at 32
/// is deliberate: it costs one bit in a commit already regenerating every
/// golden, where taking it later costs the same bit in a commit that also
/// has to be about something else, and stopping at 31 would leave the
/// unknown-subtype probe with nothing unused to probe. No datagram layout
/// moved — a wall's hp is not on the snapshot, for melee v0's reason: it
/// changes on a swing, not on a tick. v14 added the survival clock
/// (survival.rs): the eat action — the tenth, which fits the field v12
/// widened, so **no action message moved** — and three event subtypes,
/// vitals / consumed / consume-refused, the 32nd through 34th, which fit
/// the field v13 widened, so **no event message moved either.** That is
/// the whole return on taking both widenings early. No datagram layout
/// moved: the meter pair is an own-fact on the reliable lane like hp, not
/// a snapshot field, and for the same reason — it changes on a drain
/// step, and a client that misses one hears the whole truth from the
/// next. v15 gave thirst its real answer (survival.rs `drink`): the drink
/// action — the eleventh, which fits the field v12 widened, so **no
/// action message moved** — and one event subtype, `drank`, the 35th,
/// which fits the field v13 widened, so **no event message moved
/// either.** The drink action is payload-free for `Loot`'s reason: the
/// sim reads the heightfield under the sender's own feet, so there is no
/// target here to forge and no way to drink from an ocean you are not
/// standing at. `Drank` is its own subtype rather than a `Consumed` with
/// an empty item, because the sea is salt and costs hp: `Health` is
/// absolute and states the number without the cause, so a client that
/// only heard the health drop could not tell a drink from a knife.
/// v16 gave death a screen and a choice (ALPHA.md §1's respawn flow). The
/// body no longer wakes by itself, so three things moved and all three fit
/// fields earlier versions widened: the respawn action — the **twelfth**,
/// inside v12's 4-bit field, so no action message moved — carrying one bit
/// for the choice (beach or your own bag); the `respawn` event, the
/// **36th** subtype, inside v13's 6-bit field, so no other event message
/// moved either; and `Death`, which widened from (victim, killer) to carry
/// the cause, the weapon and the range. That last one is the only layout
/// change in the version, and it is the whole point of it: ALPHA.md §1
/// specifies "who/what killed you — **range and weapon**, no map
/// position", and a kill feed that can only say *who* cannot say it. The
/// three extra fields are read off the victim's own record at encode, the
/// way a bag's position is — the corpse is still in its slot until the
/// player answers, which is what makes that safe. Fixtures are keyed
/// `v16_*`.
/// v17 added the move verb — the first way a player can say *which* item
/// (`inventory.rs`). Three messages, all inside fields earlier versions
/// widened, so nothing existing moved a bit: the move action, the
/// **thirteenth**, inside v12's 4-bit subtype; and the `moved` and
/// `move_refused` events, the **37th** and **38th** subtypes, inside v13's
/// 6-bit field. The action is the widest on its lane at nine bytes,
/// because it carries a 32-bit container id unconditionally rather than
/// conditionally on the kinds — a total decoder is worth five bytes on a
/// reliable stream that sends one of these per drag. Fixtures are keyed
/// `v17_*`.
/// v18 made the deployed box a container (`CONT_BOX`, `inventory.rs`).
/// **Not one bit moved and not one fixture's bytes changed but the
/// hello's** — the kind field has been two bits since v17 and its third
/// value was reserved for exactly this. The version moves anyway, and
/// that is the point of wall 6 rather than an exception to it: the same
/// nine bytes that meant `Malformed` on a v17 server now mean "move it
/// into the box at this address", so a v18 client against a v17 server
/// would have its drags declined by the frame reader — the disconnect the
/// move verb exists to never cause. A widened *meaning* is a wire change
/// even when the layout is byte-identical, and `PROTO_VER` is the only
/// thing that catches a mismatched build. Fixtures are keyed `v18_*`.
/// v19 opened a container to the client that opened it (`event.rs`
/// `SUB_CONT_SYNC`, `server/core.rs`'s container view). Two messages, both
/// inside fields earlier versions widened: the container action, the
/// **fourteenth** of v12's sixteen, and the contents sync, the **39th** of
/// v13's sixty-four. Nothing existing moved a bit.
///
/// It is the first S→C message whose *audience* is one client rather than
/// everyone in AOI, and that is the design, not an optimization: a
/// container's contents fanned out to the neighbourhood is an ESP feed —
/// a raider reading a base's boxes from outside its walls — so the sync
/// goes only to the client holding the container open, and only while the
/// server can still prove that client is in reach of it. Fixtures are
/// keyed `v19_*`.
pub const PROTO_VER: u16 = 19;

/// Datagram kind field width — room for the class-S lanes to grow into.
pub const KIND_BITS: u32 = 3;
pub const KIND_INPUT: u32 = 0;
pub const KIND_SNAPSHOT: u32 = 1;
/// Stream-lane message kinds (the bidi handshake, DESIGN.md §5.9). Same
/// 3-bit kind space; stream messages ride length-prefixed (u16 LE) frames
/// on the reliable lane, never datagrams.
pub const KIND_HELLO: u32 = 2;
pub const KIND_WELCOME: u32 = 3;
pub const KIND_REFUSE: u32 = 4;
/// S→C reliable event-lane messages (`event.rs`): subtyped, so the whole
/// lane spends one kind.
pub const KIND_EVENT: u32 = 5;
/// C→S reliable action messages (craft request / cancel): subtyped like
/// the event lane, riding the same bidi stream in length-prefixed frames
/// (the server's 64 B `read_frame` acceptance).
pub const KIND_ACTION: u32 = 6;
/// C→S chat lines (`chat.rs`), on the same bidi stream in the same
/// length-prefixed frames. Chat is player-authored text, never a sim
/// command, so it rides its own kind instead of the action lane's
/// subtype space — which had no code left for it regardless. This is the
/// last code the 3-bit kind field holds: an eighth lane costs a width
/// bump, and that bump would widen every input datagram and every
/// snapshot, so the next lane should subtype an existing kind.
pub const KIND_CHAT: u32 = 7;

/// Longest stream-lane message payload the handshake accepts. Overflow
/// policy: refuse (`Malformed`) — a hello has no business being big.
pub const MAX_STREAM_MSG_BYTES: usize = 64;

const FRAME_COUNT_BITS: u32 = 4;
const COUNT_BITS: u32 = 7;
/// Hotbar selector width: 3 bits hold 0–5; 6–7 are refused at decode.
const SEL_BITS: u32 = 3;

// Position on the wire, absolute records (DESIGN.md §5.5 quanta: 3 cm x/z,
// 1 cm y; widths + biases registered in DECISIONS.md §open, pinned by
// `test_protocol_golden`):
// x/z: 17 bits of 3 cm quanta = 0..3932 m, covers ISLAND_SIZE 2048 m.
// y: 14 bits of 1 cm quanta biased −20.48 m = −20.48..+143.35 m, covers
//    sea floor −12 m through ridge ~60 m with fall headroom.
// vy: 14 bits of 1 cm/s quanta biased −81.92 m/s = ±81.9 m/s, covers
//     TERMINAL_VELOCITY 50 (NETCODE §3's ±16 figure predates the spoken
//     terminal; the at-rest bit elides it entirely when still).
pub const POS_XZ_BITS: u32 = 17;
pub const POS_Y_BITS: u32 = 14;
pub const POS_Y_BIAS: i32 = 2048;
pub const VEL_BITS: u32 = 14;
pub const VEL_BIAS: i32 = 8192;

// Delta records: per-axis position deltas vs baseline, ±3.81 m x/z and
// ±5.11 m y — a sprint covers 0.37 m and terminal fall 3.33 m between
// 15 Hz snapshots, so real motion always fits; anything bigger falls back
// to an absolute record.
const DPOS_XZ_BITS: u32 = 8;
const DPOS_XZ_BIAS: i32 = 128;
const DPOS_Y_BITS: u32 = 10;
const DPOS_Y_BIAS: i32 = 512;

/// Peek a datagram's kind without decoding it — the dispatch read.
pub fn peek_kind(buf: &[u8]) -> Result<u32, WireError> {
    BitReader::new(buf).read(KIND_BITS)
}

// ---------------------------------------------------------------------------
// Stream-lane handshake messages (bidi, DESIGN.md §5.9)
// ---------------------------------------------------------------------------

/// C→S on the bidi stream: `hello{proto_ver}`. The version gate happens
/// before anything else exists for this client.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Hello {
    pub proto_ver: u16,
}

/// S→C: the join bundle v0 — player id, world seed, current server tick.
/// Grows (spawn ring, catalog hash) with the slices that add those.
///
/// `dev` is the shard stating what it is. A dev shard is one running dev
/// overrides (today: `shard.toml dev_spawn`); the client installs its
/// dev-only affordances — the `setView` camera hook a capture harness aims
/// with — only when this bit is set, so a public shard's client has no
/// such surface at all. It is a statement about the SHARD, never a grant
/// of authority: nothing behind it changes what the sim accepts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Welcome {
    pub player_id: u32,
    pub seed: u64,
    pub tick: u32,
    pub dev: bool,
}

/// S→C: refusal with a posted reason — a shard at cap refuses at hello,
/// never hangs (DESIGN.md §5.9).
pub const REFUSE_VERSION: u8 = 0;
pub const REFUSE_FULL: u8 = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Refuse {
    pub code: u8,
}

pub fn encode_hello(msg: &Hello, buf: &mut [u8]) -> Result<usize, WireError> {
    let mut w = BitWriter::new(buf);
    w.write(KIND_HELLO, KIND_BITS)?;
    w.write(msg.proto_ver as u32, 16)?;
    Ok(w.finish())
}

pub fn decode_hello(buf: &[u8]) -> Result<Hello, WireError> {
    let mut r = BitReader::new(buf);
    if r.read(KIND_BITS)? != KIND_HELLO {
        return Err(WireError::Malformed);
    }
    let proto_ver = r.read(16)? as u16;
    expect_zero_padding(&mut r)?;
    Ok(Hello { proto_ver })
}

pub fn encode_welcome(msg: &Welcome, buf: &mut [u8]) -> Result<usize, WireError> {
    let mut w = BitWriter::new(buf);
    w.write(KIND_WELCOME, KIND_BITS)?;
    w.write(msg.player_id, 32)?;
    w.write(msg.seed as u32, 32)?;
    w.write((msg.seed >> 32) as u32, 32)?;
    w.write(msg.tick, 32)?;
    w.write(msg.dev as u32, 1)?;
    Ok(w.finish())
}

pub fn decode_welcome(buf: &[u8]) -> Result<Welcome, WireError> {
    let mut r = BitReader::new(buf);
    if r.read(KIND_BITS)? != KIND_WELCOME {
        return Err(WireError::Malformed);
    }
    let player_id = r.read(32)?;
    let lo = r.read(32)? as u64;
    let hi = r.read(32)? as u64;
    let tick = r.read(32)?;
    let dev = r.read(1)? != 0;
    expect_zero_padding(&mut r)?;
    Ok(Welcome {
        player_id,
        seed: lo | (hi << 32),
        tick,
        dev,
    })
}

pub fn encode_refuse(msg: &Refuse, buf: &mut [u8]) -> Result<usize, WireError> {
    let mut w = BitWriter::new(buf);
    w.write(KIND_REFUSE, KIND_BITS)?;
    w.write(msg.code as u32, 8)?;
    Ok(w.finish())
}

pub fn decode_refuse(buf: &[u8]) -> Result<Refuse, WireError> {
    let mut r = BitReader::new(buf);
    if r.read(KIND_BITS)? != KIND_REFUSE {
        return Err(WireError::Malformed);
    }
    let code = r.read(8)? as u8;
    expect_zero_padding(&mut r)?;
    Ok(Refuse { code })
}

// ---------------------------------------------------------------------------
// Action messages (C→S, the reliable bidi lane)
// ---------------------------------------------------------------------------

/// Widened 3 → 4 in wire v12: `ACT_LOOT` is the ninth action and eight
/// was the ceiling. Every action message therefore moved by one bit —
/// the goldens are regenerated in the same commit (CLAUDE.md wall 6),
/// and the C→S action lane is the only lane affected (no datagram
/// layout, no S→C event, moved).
const ACTION_SUB_BITS: u32 = 4;
const ACT_CRAFT: u32 = 0;
const ACT_CANCEL: u32 = 1;
const ACT_PLACE: u32 = 2;
const ACT_DEPLOY: u32 = 3;
const ACT_FEED: u32 = 4;
const ACT_USE: u32 = 5;
const ACT_LOCK: u32 = 6;
const ACT_UPGRADE: u32 = 7;
const ACT_LOOT: u32 = 8;
/// The eat verb (wire v14, survival.rs). Tenth of the sixteen a 4-bit
/// field holds, so no width moved for it.
const ACT_CONSUME: u32 = 9;
/// The drink verb (wire v15, survival.rs). Eleventh of the sixteen, so
/// no width moved for it either — five action codes remain.
const ACT_DRINK: u32 = 10;
/// The respawn verb (wire v16, world.rs). Twelfth of the sixteen, so no
/// width moved for it either — four action codes remain.
const ACT_RESPAWN: u32 = 11;
/// The move verb (wire v17, `inventory.rs`). Thirteenth of the sixteen,
/// so no width moved for it either — three action codes remain.
const ACT_MOVE: u32 = 12;
/// Open or close a ground container (wire v19, `server/core.rs`).
/// Fourteenth of the sixteen, so no width moved for it — two action codes
/// remain.
const ACT_CONTAINER: u32 = 13;
/// The highest live action code, named rather than counted — the event
/// lane's `SUB_MAX` discipline, which this lane did not have.
///
/// It is worth more here than there, because this lane's field is four
/// bits and two codes from full while the event lane's is six bits and
/// twenty-five from full. A fifteenth and a sixteenth action are already
/// named in `NOW.md` (a repair, a throw); the seventeenth truncates into a
/// live code, and both ends would then agree on bytes that mean two
/// different things — the worst shape of wire drift there is. The assert
/// below is what turns that from a comment someone must remember into a
/// build failure.
const ACT_MAX: u32 = ACT_CONTAINER;
const _: () = assert!(
    ACT_MAX < (1 << ACTION_SUB_BITS),
    "an action subtype past the field width would truncate into a live code"
);
/// Container-kind width, on the move action and now the container verb.
/// Two bits for **three** live kinds (`inventory::CONT_SELF`, `CONT_BAG`,
/// `CONT_BOX`), so only 3 is forgeable and it refuses at decode — the
/// `BUILD_MAT_BITS` posture. Spending the pair at v17 rather than one bit
/// is what let the box land at v18 and the container verb at v19 without
/// moving a single existing message; the width is now full, and a fourth
/// kind widens this field and every fixture with it. Every check is
/// against `inventory::CONT_MAX`, never against this width — the two are
/// deliberately not the same number.
const CONT_KIND_BITS: u32 = 2;
/// Move-count width. Full `u16`, matching `ItemStack::count` and the
/// stack ladder's own type, so no count a container can legally hold is
/// unsendable. Zero refuses at decode: a move of nothing is not a move,
/// and letting it cross would spend a refusal event on a message the
/// wire could have declined itself.
const MOVE_COUNT_BITS: u32 = 16;
/// Cancel index width mirrors the queue (`CRAFT_QUEUE` = 4 fits 3 bits);
/// values past the queue refuse at decode like a forged hotbar selector.
const CANCEL_INDEX_BITS: u32 = 3;
/// Build-grid field widths (limits.rs: `MAX_BUILD_COORD` 1024 cells,
/// `MAX_BUILD_LEVELS` 8, four locs, `MAX_PIECE_DEFS` 32 rows). Coord,
/// level, and loc widths are exact; piece rows past the cap refuse at
/// decode. Shared with the event lane's piece records (`event.rs`).
pub(crate) const BUILD_CELL_BITS: u32 = 10;
pub(crate) const BUILD_LEVEL_BITS: u32 = 3;
pub(crate) const BUILD_LOC_BITS: u32 = 2;
pub(crate) const PIECE_ROW_BITS: u32 = 8;
/// Deployable rows cross in 4 bits — exactly `MAX_DEPLOY_DEFS`, so the
/// width itself is the range check.
pub(crate) const DEPLOY_ROW_BITS: u32 = 4;
/// The upgrade action's target material (build.rs `MAT_*`: wood, stone,
/// metal). Three values in two bits, so the fourth is forgeable and the
/// decoder refuses it — the same posture as the hotbar selector.
const BUILD_MAT_BITS: u32 = 2;
/// Inventory-slot width on the action lane — `INV_SLOTS` is 30, so five
/// bits hold it and the two forgeable values above it refuse at decode,
/// the same posture as the hotbar selector.
const ACTION_SLOT_BITS: u32 = 5;

/// One decoded C→S action. The wire enforces shape (recipe inside the
/// sim's table, a live index, a nonzero count); meaning — does the recipe
/// exist, are the inputs there — is the sim's verdict, delivered as a
/// craft-refused event.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActionMsg {
    /// Enqueue `count` crafts of recipe row `recipe`.
    Craft { recipe: u16, count: u16 },
    /// Cancel the queue job at `index`, refunding remaining inputs.
    CraftCancel { index: u16 },
    /// Place baked piece row `row` at build-grid address (cx, cz, level,
    /// loc). Shape here too: address inside the grid, row inside the
    /// table; support/terrain/cost are the sim's verdict, delivered as a
    /// build-refused event.
    Place {
        row: u16,
        cx: u16,
        cz: u16,
        level: u8,
        loc: u8,
    },
    /// Place baked deployable row `row` at the grid address. Same
    /// contract: the wire enforces shape, the sim delivers meaning as a
    /// deploy-refused event.
    Deploy {
        row: u16,
        cx: u16,
        cz: u16,
        level: u8,
        loc: u8,
    },
    /// Feed the hearth at the address from the sender's inventory.
    Feed { cx: u16, cz: u16, level: u8 },
    /// Use the deployable at the address — today that means toggling a
    /// door open/closed. Address only: the wire never carries the state
    /// the client wants, because a toggle raced against another player's
    /// toggle would then fight; the sim flips what it has and announces.
    Use {
        cx: u16,
        cz: u16,
        level: u8,
        loc: u8,
    },
    /// Set the lock bit of the door at the address (lock v0). This one
    /// *does* carry state, and for the same reason `Use` doesn't: a lock
    /// press is a deliberate setting, so two racing presses must agree on
    /// the result rather than swap it. Owner-only is the sim's verdict.
    Lock {
        cx: u16,
        cz: u16,
        level: u8,
        loc: u8,
        locked: bool,
    },
    /// Upgrade the piece at the address into `material` — the rung, not a
    /// step, for the same reason `Lock` carries state: two presses racing
    /// must agree on the result. Whether that rung is above the piece's
    /// own, and whether the sender can pay, are the sim's verdict.
    Upgrade {
        cx: u16,
        cz: u16,
        level: u8,
        loc: u8,
        material: u8,
    },
    /// Open the nearest death backpack in reach and take what fits
    /// (backpack.rs). **Payload-free, and deliberately**: every other
    /// action names a grid address because the thing it acts on has one,
    /// and a bag does not — it lies wherever a body fell. The sim picks
    /// the nearest bag inside the same reach a feed and a door use, so
    /// there is no id here to forge, no address to aim past a wall, and
    /// no way to loot something the sender is not standing on.
    Loot,
    /// Eat what is in inventory slot `slot` (survival.rs). The slot is
    /// shape-checked here — past the sim's array it does not decode — and
    /// everything else is the sim's verdict: an empty hand, a stack of
    /// wood, and a full pair of meters all come back as a consume-refused
    /// event rather than as a wire error.
    Consume { slot: u8 },
    /// Drink from the water at your feet (survival.rs). **Payload-free
    /// for `Loot`'s reason, and a stronger one**: the only thing a drink
    /// acts on is the heightfield, which is a pure function of the seed
    /// both sides already hold, so the sim asks it directly under the
    /// sender's own body. There is no position here to forge, no reach to
    /// stretch, and no way to drink an ocean from a hilltop.
    Drink,
    /// Answer the death screen: `on_bag` asks to wake on the nearest of
    /// your own ready sleeping bags, false asks for a beach (ALPHA.md §1,
    /// "choose beach or a bag").
    ///
    /// One bit, and no bag id — the same posture `Loot` takes, for the same
    /// reason and one more. A bag has no grid address to name, the sim
    /// already picks the nearest ready one from where the body fell, and a
    /// forgeable id here would let a client wake on somebody else's bag.
    /// What the bit *does* buy is the only half of the choice a client can
    /// hold an opinion about: whether to go back to the fight you just lost
    /// or leave it. Sent by a live body it does nothing (world.rs).
    Respawn { on_bag: bool },
    /// Move `count` items from one slot to another (`inventory.rs`).
    ///
    /// **The one action that carries a target id**, and the exception is
    /// the point of the verb. `Loot` and `Respawn` refuse an id because
    /// the sim can pick the nearest thing itself and a forgeable id would
    /// let a client reach a bag that is not theirs; here the whole verb
    /// *is* the choice, so the id crosses and the sim spends a reach check
    /// on it instead — which is the same defence, moved from "you cannot
    /// name it" to "you cannot name it from over there".
    ///
    /// `cont` is the **one** ground container this move touches, and what
    /// it holds depends on the kind naming it: a bag id for `CONT_BAG`, a
    /// packed `box_key(cx, cz, level)` address for `CONT_BOX`. Zero and
    /// ignored for a move inside your own inventory. One handle for both
    /// sides is why a bag→box move is refused rather than encoded — see
    /// `REFUSE_M_NO_CONTAINER`.
    ///
    /// Shape is the wire's business (a kind inside the live ones, a slot
    /// inside `INV_SLOTS`, a nonzero count); whether the item is there,
    /// whether it fits and whether the container is in reach is the sim's
    /// verdict, delivered as a move-refused event. The slot bound here is
    /// the widest container's — a box has fewer slots and the sim refuses
    /// the difference, because the wire must not need a content table to
    /// decode a frame.
    Move {
        cont: u32,
        from_kind: u8,
        from_slot: u8,
        to_kind: u8,
        to_slot: u8,
        count: u16,
    },
    /// Open a ground container, or close whatever is open (wire v19).
    ///
    /// **One field, three meanings, and no combination to validate.** The
    /// obvious shape is an `open: bool` beside a kind, and it is the wrong
    /// one: two fields that must agree is two fields a forged frame can
    /// disagree, and then the decoder owns a rule instead of a range.
    /// Here `kind` alone says it — `CONT_SELF` is "nothing is open", which
    /// is not a pun on the enum but the literal statement the server
    /// stores, and `CONT_BAG` / `CONT_BOX` name the thing. The only cross-
    /// field rule left is that a close carries no handle, and that is a
    /// zero check rather than a state machine.
    ///
    /// `cont` is the same handle `Move` carries and means the same thing:
    /// a bag id for `CONT_BAG`, a packed `box_key(cx, cz, level)` for
    /// `CONT_BOX`. Deliberately the same field in the same order, because
    /// a client that can name a container to open must name it *identically*
    /// to open it and to move inside it — a second addressing scheme is
    /// how the reference's move verb bled (`inventory.rs`).
    ///
    /// **This grants nothing.** It is a subscription, not a permission:
    /// `inventory::CONT_BAG` states that reach is checked when the move
    /// resolves and never when a panel opened, and this message does not
    /// bend that. The server re-proves reach every tick before it sends a
    /// single slot, so a forged open of a box across the map yields no
    /// contents, and a real open of a box you then walk away from stops
    /// yielding them. Nothing here reaches the sim, nothing here enters
    /// the WAL, and `World::state_hash` never sees it.
    Container { kind: u8, cont: u32 },
}

/// The container verb — see `ActionMsg::Container`. A kind past
/// `CONT_MAX` and a close carrying a handle are both refused here, so the
/// decoder's checks and the encoder's are the same two.
pub fn encode_action_container(kind: u8, cont: u32, buf: &mut [u8]) -> Result<usize, WireError> {
    if kind > sim_core::inventory::CONT_MAX || (kind == sim_core::inventory::CONT_SELF && cont != 0)
    {
        return Err(WireError::Range);
    }
    let mut w = BitWriter::new(buf);
    w.write(KIND_ACTION, KIND_BITS)?;
    w.write(ACT_CONTAINER, ACTION_SUB_BITS)?;
    w.write(kind as u32, CONT_KIND_BITS)?;
    w.write(cont, 32)?;
    Ok(w.finish())
}

/// The move verb — see `ActionMsg::Move`. Range-checks every field it can
/// (kinds, slots, a nonzero count) so a client bug is a local `Range`
/// rather than a frame the server declines by ending the session: a bad
/// action frame kills the reader task (`server/src/net.rs`), which is
/// exactly the disconnect this verb is written to never cause.
pub fn encode_action_move(
    cont: u32,
    from_kind: u8,
    from_slot: u8,
    to_kind: u8,
    to_slot: u8,
    count: u16,
    buf: &mut [u8],
) -> Result<usize, WireError> {
    if from_kind > sim_core::inventory::CONT_MAX
        || to_kind > sim_core::inventory::CONT_MAX
        || from_slot as usize >= sim_core::limits::INV_SLOTS
        || to_slot as usize >= sim_core::limits::INV_SLOTS
        || count == 0
    {
        return Err(WireError::Range);
    }
    let mut w = BitWriter::new(buf);
    w.write(KIND_ACTION, KIND_BITS)?;
    w.write(ACT_MOVE, ACTION_SUB_BITS)?;
    w.write(cont, 32)?;
    w.write(from_kind as u32, CONT_KIND_BITS)?;
    w.write(from_slot as u32, ACTION_SLOT_BITS)?;
    w.write(to_kind as u32, CONT_KIND_BITS)?;
    w.write(to_slot as u32, ACTION_SLOT_BITS)?;
    w.write(count as u32, MOVE_COUNT_BITS)?;
    Ok(w.finish())
}

/// Answer the death screen — see `ActionMsg::Respawn`.
pub fn encode_action_respawn(on_bag: bool, buf: &mut [u8]) -> Result<usize, WireError> {
    let mut w = BitWriter::new(buf);
    w.write(KIND_ACTION, KIND_BITS)?;
    w.write(ACT_RESPAWN, ACTION_SUB_BITS)?;
    w.write_bit(on_bag)?;
    Ok(w.finish())
}

/// The drink verb. No payload — see `ActionMsg::Drink`.
pub fn encode_action_drink(buf: &mut [u8]) -> Result<usize, WireError> {
    let mut w = BitWriter::new(buf);
    w.write(KIND_ACTION, KIND_BITS)?;
    w.write(ACT_DRINK, ACTION_SUB_BITS)?;
    Ok(w.finish())
}

/// The eat verb. `slot` rides the inventory-slot width the event lane
/// already uses, so the width itself is the range check.
pub fn encode_action_consume(slot: u8, buf: &mut [u8]) -> Result<usize, WireError> {
    if slot as usize >= sim_core::limits::INV_SLOTS {
        return Err(WireError::Range);
    }
    let mut w = BitWriter::new(buf);
    w.write(KIND_ACTION, KIND_BITS)?;
    w.write(ACT_CONSUME, ACTION_SUB_BITS)?;
    w.write(slot as u32, ACTION_SLOT_BITS)?;
    Ok(w.finish())
}

pub fn encode_action_loot(buf: &mut [u8]) -> Result<usize, WireError> {
    let mut w = BitWriter::new(buf);
    w.write(KIND_ACTION, KIND_BITS)?;
    w.write(ACT_LOOT, ACTION_SUB_BITS)?;
    Ok(w.finish())
}

pub fn encode_action_craft(recipe: u16, count: u16, buf: &mut [u8]) -> Result<usize, WireError> {
    if recipe as usize >= sim_core::limits::MAX_RECIPES
        || count == 0
        || count > sim_core::limits::CRAFT_COUNT_MAX
    {
        return Err(WireError::Range);
    }
    let mut w = BitWriter::new(buf);
    w.write(KIND_ACTION, KIND_BITS)?;
    w.write(ACT_CRAFT, ACTION_SUB_BITS)?;
    w.write(recipe as u32, 8)?;
    w.write(count as u32, 8)?;
    Ok(w.finish())
}

pub fn encode_action_cancel(index: u16, buf: &mut [u8]) -> Result<usize, WireError> {
    if index as usize >= sim_core::limits::CRAFT_QUEUE {
        return Err(WireError::Range);
    }
    let mut w = BitWriter::new(buf);
    w.write(KIND_ACTION, KIND_BITS)?;
    w.write(ACT_CANCEL, ACTION_SUB_BITS)?;
    w.write(index as u32, CANCEL_INDEX_BITS)?;
    Ok(w.finish())
}

pub fn encode_action_place(
    row: u16,
    cx: u16,
    cz: u16,
    level: u8,
    loc: u8,
    buf: &mut [u8],
) -> Result<usize, WireError> {
    if row as usize >= sim_core::limits::MAX_PIECE_DEFS
        || cx as usize >= sim_core::limits::MAX_BUILD_COORD
        || cz as usize >= sim_core::limits::MAX_BUILD_COORD
        || level as usize >= sim_core::limits::MAX_BUILD_LEVELS
        || loc > sim_core::build::LOC_EDGE_N
    {
        return Err(WireError::Range);
    }
    let mut w = BitWriter::new(buf);
    w.write(KIND_ACTION, KIND_BITS)?;
    w.write(ACT_PLACE, ACTION_SUB_BITS)?;
    w.write(row as u32, PIECE_ROW_BITS)?;
    w.write(cx as u32, BUILD_CELL_BITS)?;
    w.write(cz as u32, BUILD_CELL_BITS)?;
    w.write(level as u32, BUILD_LEVEL_BITS)?;
    w.write(loc as u32, BUILD_LOC_BITS)?;
    Ok(w.finish())
}

pub fn encode_action_deploy(
    row: u16,
    cx: u16,
    cz: u16,
    level: u8,
    loc: u8,
    buf: &mut [u8],
) -> Result<usize, WireError> {
    if row as usize >= sim_core::limits::MAX_DEPLOY_DEFS
        || cx as usize >= sim_core::limits::MAX_BUILD_COORD
        || cz as usize >= sim_core::limits::MAX_BUILD_COORD
        || level as usize >= sim_core::limits::MAX_BUILD_LEVELS
        || loc > sim_core::build::LOC_EDGE_N
    {
        return Err(WireError::Range);
    }
    let mut w = BitWriter::new(buf);
    w.write(KIND_ACTION, KIND_BITS)?;
    w.write(ACT_DEPLOY, ACTION_SUB_BITS)?;
    w.write(row as u32, DEPLOY_ROW_BITS)?;
    w.write(cx as u32, BUILD_CELL_BITS)?;
    w.write(cz as u32, BUILD_CELL_BITS)?;
    w.write(level as u32, BUILD_LEVEL_BITS)?;
    w.write(loc as u32, BUILD_LOC_BITS)?;
    Ok(w.finish())
}

pub fn encode_action_feed(cx: u16, cz: u16, level: u8, buf: &mut [u8]) -> Result<usize, WireError> {
    if cx as usize >= sim_core::limits::MAX_BUILD_COORD
        || cz as usize >= sim_core::limits::MAX_BUILD_COORD
        || level as usize >= sim_core::limits::MAX_BUILD_LEVELS
    {
        return Err(WireError::Range);
    }
    let mut w = BitWriter::new(buf);
    w.write(KIND_ACTION, KIND_BITS)?;
    w.write(ACT_FEED, ACTION_SUB_BITS)?;
    w.write(cx as u32, BUILD_CELL_BITS)?;
    w.write(cz as u32, BUILD_CELL_BITS)?;
    w.write(level as u32, BUILD_LEVEL_BITS)?;
    Ok(w.finish())
}

pub fn encode_action_use(
    cx: u16,
    cz: u16,
    level: u8,
    loc: u8,
    buf: &mut [u8],
) -> Result<usize, WireError> {
    if cx as usize >= sim_core::limits::MAX_BUILD_COORD
        || cz as usize >= sim_core::limits::MAX_BUILD_COORD
        || level as usize >= sim_core::limits::MAX_BUILD_LEVELS
        || loc > sim_core::build::LOC_EDGE_N
    {
        return Err(WireError::Range);
    }
    let mut w = BitWriter::new(buf);
    w.write(KIND_ACTION, KIND_BITS)?;
    w.write(ACT_USE, ACTION_SUB_BITS)?;
    w.write(cx as u32, BUILD_CELL_BITS)?;
    w.write(cz as u32, BUILD_CELL_BITS)?;
    w.write(level as u32, BUILD_LEVEL_BITS)?;
    w.write(loc as u32, BUILD_LOC_BITS)?;
    Ok(w.finish())
}

pub fn encode_action_lock(
    cx: u16,
    cz: u16,
    level: u8,
    loc: u8,
    locked: bool,
    buf: &mut [u8],
) -> Result<usize, WireError> {
    if cx as usize >= sim_core::limits::MAX_BUILD_COORD
        || cz as usize >= sim_core::limits::MAX_BUILD_COORD
        || level as usize >= sim_core::limits::MAX_BUILD_LEVELS
        || loc > sim_core::build::LOC_EDGE_N
    {
        return Err(WireError::Range);
    }
    let mut w = BitWriter::new(buf);
    w.write(KIND_ACTION, KIND_BITS)?;
    w.write(ACT_LOCK, ACTION_SUB_BITS)?;
    w.write(cx as u32, BUILD_CELL_BITS)?;
    w.write(cz as u32, BUILD_CELL_BITS)?;
    w.write(level as u32, BUILD_LEVEL_BITS)?;
    w.write(loc as u32, BUILD_LOC_BITS)?;
    w.write_bit(locked)?;
    Ok(w.finish())
}

pub fn encode_action_upgrade(
    cx: u16,
    cz: u16,
    level: u8,
    loc: u8,
    material: u8,
    buf: &mut [u8],
) -> Result<usize, WireError> {
    if cx as usize >= sim_core::limits::MAX_BUILD_COORD
        || cz as usize >= sim_core::limits::MAX_BUILD_COORD
        || level as usize >= sim_core::limits::MAX_BUILD_LEVELS
        || loc > sim_core::build::LOC_EDGE_N
        || material > sim_core::build::MAT_METAL
    {
        return Err(WireError::Range);
    }
    let mut w = BitWriter::new(buf);
    w.write(KIND_ACTION, KIND_BITS)?;
    w.write(ACT_UPGRADE, ACTION_SUB_BITS)?;
    w.write(cx as u32, BUILD_CELL_BITS)?;
    w.write(cz as u32, BUILD_CELL_BITS)?;
    w.write(level as u32, BUILD_LEVEL_BITS)?;
    w.write(loc as u32, BUILD_LOC_BITS)?;
    w.write(material as u32, BUILD_MAT_BITS)?;
    Ok(w.finish())
}

/// Total decode of one C→S action frame — client-driven bytes, so the
/// same never-panic contract as the input datagrams.
pub fn decode_action(buf: &[u8]) -> Result<ActionMsg, WireError> {
    let mut r = BitReader::new(buf);
    if r.read(KIND_BITS)? != KIND_ACTION {
        return Err(WireError::Malformed);
    }
    let msg = match r.read(ACTION_SUB_BITS)? {
        ACT_CRAFT => {
            let recipe = r.read(8)? as u16;
            let count = r.read(8)? as u16;
            if recipe as usize >= sim_core::limits::MAX_RECIPES
                || count == 0
                || count > sim_core::limits::CRAFT_COUNT_MAX
            {
                return Err(WireError::Malformed);
            }
            ActionMsg::Craft { recipe, count }
        }
        ACT_CANCEL => {
            let index = r.read(CANCEL_INDEX_BITS)? as u16;
            if index as usize >= sim_core::limits::CRAFT_QUEUE {
                return Err(WireError::Malformed);
            }
            ActionMsg::CraftCancel { index }
        }
        ACT_PLACE => {
            let row = r.read(PIECE_ROW_BITS)? as u16;
            let cx = r.read(BUILD_CELL_BITS)? as u16;
            let cz = r.read(BUILD_CELL_BITS)? as u16;
            let level = r.read(BUILD_LEVEL_BITS)? as u8;
            let loc = r.read(BUILD_LOC_BITS)? as u8;
            // Coord/level/loc widths are exact; only the row can be forged.
            if row as usize >= sim_core::limits::MAX_PIECE_DEFS {
                return Err(WireError::Malformed);
            }
            ActionMsg::Place {
                row,
                cx,
                cz,
                level,
                loc,
            }
        }
        ACT_DEPLOY => {
            // Every field width is exact (deploy rows are 4 bits =
            // MAX_DEPLOY_DEFS): nothing here can be forged out of range.
            ActionMsg::Deploy {
                row: r.read(DEPLOY_ROW_BITS)? as u16,
                cx: r.read(BUILD_CELL_BITS)? as u16,
                cz: r.read(BUILD_CELL_BITS)? as u16,
                level: r.read(BUILD_LEVEL_BITS)? as u8,
                loc: r.read(BUILD_LOC_BITS)? as u8,
            }
        }
        ACT_FEED => ActionMsg::Feed {
            cx: r.read(BUILD_CELL_BITS)? as u16,
            cz: r.read(BUILD_CELL_BITS)? as u16,
            level: r.read(BUILD_LEVEL_BITS)? as u8,
        },
        // Address only, every width exact: nothing here can be forged
        // out of range, and the sim refuses an address holding no door.
        ACT_USE => ActionMsg::Use {
            cx: r.read(BUILD_CELL_BITS)? as u16,
            cz: r.read(BUILD_CELL_BITS)? as u16,
            level: r.read(BUILD_LEVEL_BITS)? as u8,
            loc: r.read(BUILD_LOC_BITS)? as u8,
        },
        // Same: address + one bit, every width exact. Whether the sender
        // owns the door is the sim's verdict, not the wire's.
        ACT_LOCK => ActionMsg::Lock {
            cx: r.read(BUILD_CELL_BITS)? as u16,
            cz: r.read(BUILD_CELL_BITS)? as u16,
            level: r.read(BUILD_LEVEL_BITS)? as u8,
            loc: r.read(BUILD_LOC_BITS)? as u8,
            locked: r.read_bit()?,
        },
        ACT_UPGRADE => {
            let cx = r.read(BUILD_CELL_BITS)? as u16;
            let cz = r.read(BUILD_CELL_BITS)? as u16;
            let level = r.read(BUILD_LEVEL_BITS)? as u8;
            let loc = r.read(BUILD_LOC_BITS)? as u8;
            let material = r.read(BUILD_MAT_BITS)? as u8;
            // Two bits hold three materials, so the fourth is forgeable.
            if material > sim_core::build::MAT_METAL {
                return Err(WireError::Malformed);
            }
            ActionMsg::Upgrade {
                cx,
                cz,
                level,
                loc,
                material,
            }
        }
        ACT_LOOT => ActionMsg::Loot,
        ACT_CONSUME => {
            let slot = r.read(ACTION_SLOT_BITS)? as u8;
            if slot as usize >= sim_core::limits::INV_SLOTS {
                return Err(WireError::Malformed);
            }
            ActionMsg::Consume { slot }
        }
        ACT_DRINK => ActionMsg::Drink,
        ACT_RESPAWN => ActionMsg::Respawn {
            on_bag: r.read_bit()?,
        },
        ACT_MOVE => {
            let cont = r.read(32)?;
            let from_kind = r.read(CONT_KIND_BITS)? as u8;
            let from_slot = r.read(ACTION_SLOT_BITS)? as u8;
            let to_kind = r.read(CONT_KIND_BITS)? as u8;
            let to_slot = r.read(ACTION_SLOT_BITS)? as u8;
            let count = r.read(MOVE_COUNT_BITS)? as u16;
            // Two bits hold three kinds and five hold thirty slots, so both
            // have forgeable values above the live range; a zero count is
            // forgeable too. The same-slot ask is *not* refused here — it
            // is a legal shape and an illegal move, so the sim owns it and
            // answers with a reason the player can be shown. A box slot
            // past `BOX_SLOTS` takes the same route for the same reason:
            // the shape is legal, the address is not, and the sim says so.
            if from_kind > sim_core::inventory::CONT_MAX
                || to_kind > sim_core::inventory::CONT_MAX
                || from_slot as usize >= sim_core::limits::INV_SLOTS
                || to_slot as usize >= sim_core::limits::INV_SLOTS
                || count == 0
            {
                return Err(WireError::Malformed);
            }
            ActionMsg::Move {
                cont,
                from_kind,
                from_slot,
                to_kind,
                to_slot,
                count,
            }
        }
        ACT_CONTAINER => {
            let kind = r.read(CONT_KIND_BITS)? as u8;
            let cont = r.read(32)?;
            // Two bits hold three kinds, so value 3 is forgeable. The
            // handle is *not* range-checked and deliberately: a bag id and
            // a packed box address have no shape a decoder could tell apart
            // from a wrong one, and "that container does not exist" is a
            // fact only the sim's stores hold. The server answers it by
            // sending no contents, never by refusing the frame — refusing
            // it here would end the session, which is the disconnect the
            // container verbs exist to never cause (`inventory.rs`).
            //
            // What *is* checked is the one cross-field rule: a close names
            // no container. A handle riding a `CONT_SELF` close is a client
            // that thinks it is opening something, and letting it decode
            // would leave the two ends disagreeing about what is open.
            if kind > sim_core::inventory::CONT_MAX
                || (kind == sim_core::inventory::CONT_SELF && cont != 0)
            {
                return Err(WireError::Malformed);
            }
            ActionMsg::Container { kind, cont }
        }
        _ => return Err(WireError::Malformed),
    };
    expect_zero_padding(&mut r)?;
    Ok(msg)
}

// ---------------------------------------------------------------------------
// Input datagram (C→S)
// ---------------------------------------------------------------------------

/// One input datagram: the Gaffer ack header plus the client's unacked
/// input tail (NETCODE.md §3). Frames are seq-consecutive, oldest first;
/// `push` enforces it so the wire can carry one seq for all of them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InputDatagram {
    /// Newest snapshot tick **applied** (low 16 bits, wrap-compare). An
    /// ack means applied, not merely received: a stale snapshot the client
    /// discarded is never acked, so the server's mirror of what the client
    /// knows (its delta baseline) folds exactly the acked set.
    pub snapshot_ack: u16,
    /// Bit n set ⇒ snapshot tick `snapshot_ack − n − 1` also applied.
    pub ack_bits: u32,
    /// Client tick of `frames[0]`; with no frames, the client's current
    /// tick (the datagram still feeds the server's clock estimate).
    pub first_client_tick: u32,
    frames: [InputFrame; MAX_INPUT_FRAMES],
    frame_count: u8,
}

impl InputDatagram {
    pub fn new(snapshot_ack: u16, ack_bits: u32, first_client_tick: u32) -> Self {
        Self {
            snapshot_ack,
            ack_bits,
            first_client_tick,
            frames: [InputFrame::default(); MAX_INPUT_FRAMES],
            frame_count: 0,
        }
    }

    /// Append the next unacked frame. Refuses past `MAX_INPUT_FRAMES`
    /// (policy: the *client* drops its oldest before pushing — the cap is
    /// on the datagram, not the intent) and refuses a seq gap (`Malformed`)
    /// because the wire reconstructs seqs from the first one.
    pub fn push(&mut self, frame: InputFrame) -> Result<(), WireError> {
        let n = self.frame_count as usize;
        if n >= MAX_INPUT_FRAMES {
            return Err(WireError::Cap);
        }
        if n > 0 && frame.seq != self.frames[n - 1].seq.wrapping_add(1) {
            return Err(WireError::Malformed);
        }
        self.frames[n] = frame;
        self.frame_count += 1;
        Ok(())
    }

    pub fn frames(&self) -> &[InputFrame] {
        &self.frames[..self.frame_count as usize]
    }

    /// Client tick of frame `i` (ticks advance in lockstep with seqs).
    pub fn client_tick_of(&self, i: usize) -> u32 {
        self.first_client_tick.wrapping_add(i as u32)
    }
}

pub fn encode_input(dg: &InputDatagram, buf: &mut [u8]) -> Result<usize, WireError> {
    let mut w = BitWriter::new(buf);
    w.write(KIND_INPUT, KIND_BITS)?;
    w.write(dg.snapshot_ack as u32, 16)?;
    w.write(dg.ack_bits, 32)?;
    w.write(dg.first_client_tick, 32)?;
    let frames = dg.frames();
    w.write(frames.len() as u32, FRAME_COUNT_BITS)?;
    if let Some(first) = frames.first() {
        w.write(first.seq as u32, 16)?;
        for f in frames {
            if f.sel as usize >= HOTBAR_SLOTS {
                return Err(WireError::Range);
            }
            w.write(f.buttons as u32, 8)?;
            w.write(f.yaw as u32, 16)?;
            w.write(f.pitch as u32, 8)?;
            w.write(f.move_x as u8 as u32, 8)?;
            w.write(f.move_z as u8 as u32, 8)?;
            w.write(f.sel as u32, SEL_BITS)?;
        }
    }
    Ok(w.finish())
}

pub fn decode_input(buf: &[u8]) -> Result<InputDatagram, WireError> {
    let mut r = BitReader::new(buf);
    if r.read(KIND_BITS)? != KIND_INPUT {
        return Err(WireError::Malformed);
    }
    let snapshot_ack = r.read(16)? as u16;
    let ack_bits = r.read(32)?;
    let first_client_tick = r.read(32)?;
    let count = r.read(FRAME_COUNT_BITS)? as usize;
    if count > MAX_INPUT_FRAMES {
        return Err(WireError::Malformed);
    }
    let mut dg = InputDatagram::new(snapshot_ack, ack_bits, first_client_tick);
    if count > 0 {
        let first_seq = r.read(16)? as u16;
        for i in 0..count {
            let frame = InputFrame {
                seq: first_seq.wrapping_add(i as u16),
                buttons: r.read(8)? as u8,
                yaw: r.read(16)? as u16,
                pitch: r.read(8)? as u8,
                move_x: r.read(8)? as u8 as i8,
                move_z: r.read(8)? as u8 as i8,
                sel: r.read(SEL_BITS)? as u8,
            };
            if frame.sel as usize >= HOTBAR_SLOTS {
                return Err(WireError::Malformed);
            }
            // Cannot fail: count ≤ cap, seqs consecutive by construction.
            dg.push(frame)?;
        }
    }
    expect_zero_padding(&mut r)?;
    Ok(dg)
}

// ---------------------------------------------------------------------------
// Snapshot datagram (S→C)
// ---------------------------------------------------------------------------

/// The time-dilation nudge, 2 bits in every snapshot header (NETCODE.md §4,
/// the Overwatch scheme).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Nudge {
    #[default]
    Ok = 0,
    Faster = 1,
    Slower = 2,
    HardResync = 3,
}

impl Nudge {
    fn from_bits(v: u32) -> Self {
        match v {
            1 => Nudge::Faster,
            2 => Nudge::Slower,
            3 => Nudge::HardResync,
            _ => Nudge::Ok,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SnapshotHeader {
    /// Server tick, low 32 bits (4.5 years at 30 Hz before wrap).
    pub tick: u32,
    /// 0 ⇒ zero-state baseline (all records absolute); else the baseline
    /// is the client's acked state at `tick − baseline_age`. The sent-state
    /// ring is 32 snapshots ≈ 64 ticks (NETCODE.md §3), well inside u8.
    pub baseline_age: u8,
    /// Newest input seq the sim executed for this client — drives
    /// prediction reconciliation (DESIGN.md §5.6).
    pub last_executed_seq: u16,
    pub nudge: Nudge,
}

/// One class-D entity as the wire sees it: the quantized body (the sim
/// runs on exactly these values — NETCODE.md §3) plus view yaw/pitch.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct EntityState {
    pub id: u32,
    /// Position in world-absolute quanta (3 cm x/z, 1 cm y — `movement.rs`).
    pub qx: i32,
    pub qy: i32,
    pub qz: i32,
    /// Vertical velocity in 1 cm/s quanta; 0 rides the at-rest bit.
    pub qvy: i32,
    pub grounded: bool,
    pub yaw: u16,
    pub pitch: u8,
}

/// Incremental snapshot encoder — the priority-fill loop's tool
/// (DESIGN.md §5.5): removals first, then entities best-first; an
/// `Overflow` cleanly rolls back the record that didn't fit and the
/// caller sheds it (it stays accumulated and wins soon). The budget IS
/// the buffer: hand it `DATAGRAM_BUDGET_BYTES` and overflow is the gate.
pub struct SnapshotEncoder<'a, 'b> {
    w: BitWriter<'a>,
    baseline: &'b [EntityState],
    has_baseline: bool,
    removed_count_at: usize,
    entity_count_at: usize,
    removed: u32,
    entities: u32,
    entity_started: bool,
}

impl<'a, 'b> SnapshotEncoder<'a, 'b> {
    /// `baseline` is the client's acked entity set at `tick −
    /// baseline_age`. Zero-state (`baseline_age == 0`) has no entities by
    /// definition, so a non-empty baseline with it is refused — that
    /// mismatch is a server bug surfacing, not a case to paper over.
    pub fn begin(
        buf: &'a mut [u8],
        header: &SnapshotHeader,
        baseline: &'b [EntityState],
    ) -> Result<Self, WireError> {
        if header.baseline_age == 0 && !baseline.is_empty() {
            return Err(WireError::Malformed);
        }
        let mut w = BitWriter::new(buf);
        w.write(KIND_SNAPSHOT, KIND_BITS)?;
        w.write(header.tick, 32)?;
        w.write(header.baseline_age as u32, 8)?;
        w.write(header.last_executed_seq as u32, 16)?;
        w.write(header.nudge as u32, 2)?;
        let removed_count_at = w.bit_pos();
        w.write(0, COUNT_BITS)?;
        let entity_count_at = w.bit_pos();
        w.write(0, COUNT_BITS)?;
        Ok(Self {
            w,
            baseline,
            has_baseline: header.baseline_age != 0,
            removed_count_at,
            entity_count_at,
            removed: 0,
            entities: 0,
            entity_started: false,
        })
    }

    /// An entity that left the interest set. Must precede every
    /// `add_entity` (wire order); capped like the set itself.
    pub fn add_removed(&mut self, id: u32) -> Result<(), WireError> {
        if self.entity_started {
            return Err(WireError::Order);
        }
        if self.removed as usize >= MAX_SNAPSHOT_ENTITIES {
            return Err(WireError::Cap);
        }
        let mark = self.w.bit_pos();
        if let Err(e) = self.w.write(id, 32) {
            self.w.rewind_to(mark);
            return Err(e);
        }
        self.removed += 1;
        Ok(())
    }

    /// Add one entity, delta-encoded against the baseline when it's there
    /// and the motion fits the delta widths, absolute otherwise. On
    /// `Overflow` the record is rolled back whole; the encoder stays
    /// usable (a smaller record may still fit).
    pub fn add_entity(&mut self, e: &EntityState) -> Result<(), WireError> {
        self.entity_started = true;
        if self.entities as usize >= MAX_SNAPSHOT_ENTITIES {
            return Err(WireError::Cap);
        }
        check_ranges(e)?;
        let mark = self.w.bit_pos();
        if let Err(err) = self.encode_entity(e) {
            self.w.rewind_to(mark);
            return Err(err);
        }
        self.entities += 1;
        Ok(())
    }

    fn encode_entity(&mut self, e: &EntityState) -> Result<(), WireError> {
        self.w.write(e.id, 32)?;
        if self.has_baseline {
            if let Some(b) = self.baseline.iter().find(|b| b.id == e.id) {
                let dx = e.qx as i64 - b.qx as i64;
                let dy = e.qy as i64 - b.qy as i64;
                let dz = e.qz as i64 - b.qz as i64;
                let fits = |d: i64, bias: i32, bits: u32| -> bool {
                    d + bias as i64 >= 0 && d + (bias as i64) < (1i64 << bits)
                };
                if fits(dx, DPOS_XZ_BIAS, DPOS_XZ_BITS)
                    && fits(dy, DPOS_Y_BIAS, DPOS_Y_BITS)
                    && fits(dz, DPOS_XZ_BIAS, DPOS_XZ_BITS)
                {
                    return self.encode_delta(e, b, dx, dy, dz);
                }
            }
        }
        self.encode_absolute(e)
    }

    fn encode_delta(
        &mut self,
        e: &EntityState,
        b: &EntityState,
        dx: i64,
        dy: i64,
        dz: i64,
    ) -> Result<(), WireError> {
        self.w.write_bit(true)?; // is_delta
        let pos_changed = dx != 0 || dy != 0 || dz != 0;
        let vel_changed = e.qvy != b.qvy;
        let look_changed = e.yaw != b.yaw || e.pitch != b.pitch;
        self.w.write_bit(pos_changed)?;
        self.w.write_bit(vel_changed)?;
        self.w.write_bit(look_changed)?;
        self.w.write_bit(e.grounded)?;
        if pos_changed {
            self.w
                .write((dx + DPOS_XZ_BIAS as i64) as u32, DPOS_XZ_BITS)?;
            self.w
                .write((dy + DPOS_Y_BIAS as i64) as u32, DPOS_Y_BITS)?;
            self.w
                .write((dz + DPOS_XZ_BIAS as i64) as u32, DPOS_XZ_BITS)?;
        }
        if vel_changed {
            self.write_vel(e.qvy)?;
        }
        if look_changed {
            self.w.write(e.yaw as u32, 16)?;
            self.w.write(e.pitch as u32, 8)?;
        }
        Ok(())
    }

    fn encode_absolute(&mut self, e: &EntityState) -> Result<(), WireError> {
        self.w.write_bit(false)?; // is_delta
        self.w.write(e.qx as u32, POS_XZ_BITS)?;
        self.w.write((e.qy + POS_Y_BIAS) as u32, POS_Y_BITS)?;
        self.w.write(e.qz as u32, POS_XZ_BITS)?;
        self.w.write_bit(e.grounded)?;
        self.write_vel(e.qvy)?;
        self.w.write(e.yaw as u32, 16)?;
        self.w.write(e.pitch as u32, 8)?;
        Ok(())
    }

    fn write_vel(&mut self, qvy: i32) -> Result<(), WireError> {
        let at_rest = qvy == 0;
        self.w.write_bit(at_rest)?;
        if !at_rest {
            self.w.write((qvy + VEL_BIAS) as u32, VEL_BITS)?;
        }
        Ok(())
    }

    /// Patch the counts, return the encoded byte length.
    pub fn finish(mut self) -> Result<usize, WireError> {
        self.w
            .patch(self.removed_count_at, self.removed, COUNT_BITS)?;
        self.w
            .patch(self.entity_count_at, self.entities, COUNT_BITS)?;
        Ok(self.w.finish())
    }
}

/// Absolute-record range check, up front so a `Range` refusal never needs
/// a rollback and delta eligibility never hides an unencodable state.
fn check_ranges(e: &EntityState) -> Result<(), WireError> {
    let in_window = |v: i64, bias: i32, bits: u32| -> bool {
        v + bias as i64 >= 0 && v + (bias as i64) < (1i64 << bits)
    };
    if !in_window(e.qx as i64, 0, POS_XZ_BITS)
        || !in_window(e.qy as i64, POS_Y_BIAS, POS_Y_BITS)
        || !in_window(e.qz as i64, 0, POS_XZ_BITS)
        || !in_window(e.qvy as i64, VEL_BIAS, VEL_BITS)
    {
        return Err(WireError::Range);
    }
    Ok(())
}

/// Whole-snapshot convenience over the incremental encoder — the shape
/// tests and goldens use. `Overflow` here is an error, not a shed: callers
/// that need the fill loop use `SnapshotEncoder` directly.
pub fn encode_snapshot(
    header: &SnapshotHeader,
    removed: &[u32],
    entities: &[EntityState],
    baseline: &[EntityState],
    buf: &mut [u8],
) -> Result<usize, WireError> {
    let mut enc = SnapshotEncoder::begin(buf, header, baseline)?;
    for &id in removed {
        enc.add_removed(id)?;
    }
    for e in entities {
        enc.add_entity(e)?;
    }
    enc.finish()
}

/// A decoded snapshot: reconstructed absolute states (deltas already
/// applied onto the baseline) plus the removed-id list. Fixed storage;
/// unused tail slots stay `Default` so equality is well-defined.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Snapshot {
    pub header: SnapshotHeader,
    removed: [u32; MAX_SNAPSHOT_ENTITIES],
    removed_count: u8,
    entities: [EntityState; MAX_SNAPSHOT_ENTITIES],
    entity_count: u8,
}

impl Snapshot {
    pub fn removed(&self) -> &[u32] {
        &self.removed[..self.removed_count as usize]
    }

    pub fn entities(&self) -> &[EntityState] {
        &self.entities[..self.entity_count as usize]
    }
}

/// Header-only read, so a receiver can pick the baseline snapshot
/// (`tick − baseline_age`) out of its applied ring before the full
/// `decode_snapshot` call. Reads exactly the header bits; the body goes
/// untouched.
pub fn peek_snapshot_header(buf: &[u8]) -> Result<SnapshotHeader, WireError> {
    let mut r = BitReader::new(buf);
    if r.read(KIND_BITS)? != KIND_SNAPSHOT {
        return Err(WireError::Malformed);
    }
    read_snapshot_header(&mut r)
}

fn read_snapshot_header(r: &mut BitReader) -> Result<SnapshotHeader, WireError> {
    Ok(SnapshotHeader {
        tick: r.read(32)?,
        baseline_age: r.read(8)? as u8,
        last_executed_seq: r.read(16)? as u16,
        nudge: Nudge::from_bits(r.read(2)?),
    })
}

/// Decode against the baseline the header names — the **sent content** of
/// the snapshot at `tick − baseline_age`, which the client holds because
/// it applied (and therefore acked) that snapshot. Entities absent from
/// that snapshot arrive absolute; nothing is ever folded across snapshots,
/// so both sides hold byte-identical baselines by construction. Total:
/// arbitrary bytes in, `Ok` or a `WireError` out, never a panic.
pub fn decode_snapshot(buf: &[u8], baseline: &[EntityState]) -> Result<Snapshot, WireError> {
    let mut r = BitReader::new(buf);
    if r.read(KIND_BITS)? != KIND_SNAPSHOT {
        return Err(WireError::Malformed);
    }
    let header = read_snapshot_header(&mut r)?;
    let removed_count = r.read(COUNT_BITS)? as usize;
    let entity_count = r.read(COUNT_BITS)? as usize;
    if removed_count > MAX_SNAPSHOT_ENTITIES || entity_count > MAX_SNAPSHOT_ENTITIES {
        return Err(WireError::Malformed);
    }
    let mut snap = Snapshot {
        header,
        removed: [0; MAX_SNAPSHOT_ENTITIES],
        removed_count: removed_count as u8,
        entities: [EntityState::default(); MAX_SNAPSHOT_ENTITIES],
        entity_count: entity_count as u8,
    };
    for slot in snap.removed.iter_mut().take(removed_count) {
        *slot = r.read(32)?;
    }
    for slot in snap.entities.iter_mut().take(entity_count) {
        *slot = decode_entity(&mut r, header.baseline_age, baseline)?;
    }
    expect_zero_padding(&mut r)?;
    Ok(snap)
}

fn decode_entity(
    r: &mut BitReader,
    baseline_age: u8,
    baseline: &[EntityState],
) -> Result<EntityState, WireError> {
    let id = r.read(32)?;
    let is_delta = r.read_bit()?;
    if !is_delta {
        let qx = r.read(POS_XZ_BITS)? as i32;
        let qy = r.read(POS_Y_BITS)? as i32 - POS_Y_BIAS;
        let qz = r.read(POS_XZ_BITS)? as i32;
        let grounded = r.read_bit()?;
        let qvy = read_vel(r)?;
        return Ok(EntityState {
            id,
            qx,
            qy,
            qz,
            qvy,
            grounded,
            yaw: r.read(16)? as u16,
            pitch: r.read(8)? as u8,
        });
    }
    if baseline_age == 0 {
        return Err(WireError::Malformed);
    }
    let b = baseline
        .iter()
        .find(|b| b.id == id)
        .ok_or(WireError::Malformed)?;
    let mut e = *b;
    let pos_changed = r.read_bit()?;
    let vel_changed = r.read_bit()?;
    let look_changed = r.read_bit()?;
    e.grounded = r.read_bit()?;
    if pos_changed {
        // wrapping: baseline values are the decoder's own prior state, but
        // totality on arbitrary bytes must hold regardless.
        e.qx =
            b.qx.wrapping_add(r.read(DPOS_XZ_BITS)? as i32 - DPOS_XZ_BIAS);
        e.qy = b.qy.wrapping_add(r.read(DPOS_Y_BITS)? as i32 - DPOS_Y_BIAS);
        e.qz =
            b.qz.wrapping_add(r.read(DPOS_XZ_BITS)? as i32 - DPOS_XZ_BIAS);
    }
    if vel_changed {
        e.qvy = read_vel(r)?;
    }
    if look_changed {
        e.yaw = r.read(16)? as u16;
        e.pitch = r.read(8)? as u8;
    }
    Ok(e)
}

fn read_vel(r: &mut BitReader) -> Result<i32, WireError> {
    if r.read_bit()? {
        Ok(0)
    } else {
        Ok(r.read(VEL_BITS)? as i32 - VEL_BIAS)
    }
}

/// Strict tail: only byte padding may remain, and it must be zero — a
/// packet with trailing garbage never passes as valid.
pub(crate) fn expect_zero_padding(r: &mut BitReader) -> Result<(), WireError> {
    let rem = r.remaining_bits();
    if rem >= 8 {
        return Err(WireError::Malformed);
    }
    if rem > 0 && r.read(rem as u32)? != 0 {
        return Err(WireError::Malformed);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use sim_core::limits::DATAGRAM_BUDGET_BYTES;

    fn ent(id: u32) -> EntityState {
        EntityState {
            id,
            qx: 34_000,
            qy: 900,
            qz: 34_000,
            qvy: 0,
            grounded: true,
            yaw: 0x1234,
            pitch: 7,
        }
    }

    #[test]
    fn wire_widths_cover_the_sim() {
        use sim_core::movement::{POS_XZ_Q, POS_Y_Q, TERMINAL_VELOCITY, VEL_Q};
        use sim_core::terrain::{AMPLITUDE, ISLAND_SIZE};
        // x/z: the whole island fits the absolute width.
        assert!(((ISLAND_SIZE / POS_XZ_Q) as i64) < (1i64 << POS_XZ_BITS));
        // y: bias reaches below any sea floor; ceiling clears the relief.
        assert!((POS_Y_BIAS as f32) * POS_Y_Q >= 12.0);
        assert!(((1 << POS_Y_BITS) - POS_Y_BIAS) as f32 * POS_Y_Q > AMPLITUDE);
        // vy: terminal velocity fits with headroom.
        assert!((VEL_BIAS as f32) * VEL_Q > TERMINAL_VELOCITY);
    }

    #[test]
    fn zero_state_refuses_a_baseline() {
        let hdr = SnapshotHeader {
            tick: 1,
            baseline_age: 0,
            last_executed_seq: 0,
            nudge: Nudge::Ok,
        };
        let baseline = [ent(1)];
        let mut buf = [0u8; 64];
        assert!(matches!(
            SnapshotEncoder::begin(&mut buf, &hdr, &baseline),
            Err(WireError::Malformed)
        ));
    }

    #[test]
    fn removals_precede_entities() {
        let hdr = SnapshotHeader {
            tick: 1,
            baseline_age: 0,
            last_executed_seq: 0,
            nudge: Nudge::Ok,
        };
        let mut buf = [0u8; 128];
        let mut enc = SnapshotEncoder::begin(&mut buf, &hdr, &[]).unwrap();
        enc.add_entity(&ent(1)).unwrap();
        assert_eq!(enc.add_removed(9), Err(WireError::Order));
    }

    #[test]
    fn out_of_range_state_is_refused_not_clamped() {
        let hdr = SnapshotHeader {
            tick: 1,
            baseline_age: 0,
            last_executed_seq: 0,
            nudge: Nudge::Ok,
        };
        let mut buf = [0u8; 128];
        let mut enc = SnapshotEncoder::begin(&mut buf, &hdr, &[]).unwrap();
        let mut bad = ent(1);
        bad.qx = -1;
        assert_eq!(enc.add_entity(&bad), Err(WireError::Range));
        bad = ent(2);
        bad.qy = (1 << POS_Y_BITS) - POS_Y_BIAS;
        assert_eq!(enc.add_entity(&bad), Err(WireError::Range));
    }

    #[test]
    fn overflow_sheds_the_record_and_the_rest_still_decodes() {
        let hdr = SnapshotHeader {
            tick: 42,
            baseline_age: 0,
            last_executed_seq: 5,
            nudge: Nudge::Ok,
        };
        // Room for the header and one absolute record, not two.
        let mut buf = [0u8; 28];
        let mut enc = SnapshotEncoder::begin(&mut buf, &hdr, &[]).unwrap();
        enc.add_entity(&ent(1)).unwrap();
        assert_eq!(enc.add_entity(&ent(2)), Err(WireError::Overflow));
        let len = enc.finish().unwrap();
        let snap = decode_snapshot(&buf[..len], &[]).unwrap();
        assert_eq!(snap.entities(), &[ent(1)]);
        assert_eq!(snap.removed(), &[] as &[u32]);
    }

    #[test]
    fn input_push_enforces_the_cap_and_seq_run() {
        let mut dg = InputDatagram::new(7, 0, 100);
        let mut f = InputFrame {
            seq: 40,
            ..InputFrame::default()
        };
        dg.push(f).unwrap();
        f.seq = 42; // gap
        assert_eq!(dg.push(f), Err(WireError::Malformed));
        f.seq = 41;
        dg.push(f).unwrap();
        for i in 0..MAX_INPUT_FRAMES as u16 - 2 {
            f.seq = 42 + i;
            dg.push(f).unwrap();
        }
        f.seq = 40 + MAX_INPUT_FRAMES as u16;
        assert_eq!(dg.push(f), Err(WireError::Cap));
    }

    #[test]
    fn input_seq_wraps_across_u16() {
        let mut dg = InputDatagram::new(0, 0, 9);
        for i in 0..4u16 {
            let f = InputFrame {
                seq: 0xFFFE_u16.wrapping_add(i),
                ..InputFrame::default()
            };
            dg.push(f).unwrap();
        }
        let mut buf = [0u8; DATAGRAM_BUDGET_BYTES];
        let len = encode_input(&dg, &mut buf).unwrap();
        let back = decode_input(&buf[..len]).unwrap();
        assert_eq!(back, dg);
        assert_eq!(back.frames()[3].seq, 1);
    }

    #[test]
    fn trailing_garbage_is_malformed() {
        let dg = InputDatagram::new(1, 2, 3);
        let mut buf = [0u8; DATAGRAM_BUDGET_BYTES];
        let len = encode_input(&dg, &mut buf).unwrap();
        // One spare byte after a valid packet must fail the strict tail.
        assert_eq!(decode_input(&buf[..len + 1]), Err(WireError::Malformed));
    }

    /// The container action's shape, and the two things it refuses.
    ///
    /// The handle is deliberately *not* refused for any value, including
    /// `u32::MAX`, and that is asserted rather than left to inference: it
    /// is the difference between "the server sends you nothing" and "the
    /// server ends your session", and the second is the disconnect the
    /// reference's own move verb kept causing (`inventory.rs`).
    #[test]
    fn container_action_round_trips_and_refuses_only_its_two_shapes() {
        use sim_core::inventory::{CONT_BAG, CONT_BOX, CONT_MAX, CONT_SELF};
        let mut buf = [0u8; 64];

        for (kind, cont) in [
            (CONT_BAG, 1u32),
            (CONT_BOX, 0x0123_2E85),
            (CONT_BAG, u32::MAX),
            (CONT_SELF, 0),
        ] {
            let len = encode_action_container(kind, cont, &mut buf).unwrap();
            assert_eq!(
                decode_action(&buf[..len]).unwrap(),
                ActionMsg::Container { kind, cont },
                "kind {kind} handle {cont}"
            );
            assert_eq!(peek_kind(&buf[..len]).unwrap(), KIND_ACTION);
        }

        // A kind past the live set.
        assert_eq!(
            encode_action_container(CONT_MAX + 1, 0, &mut buf),
            Err(WireError::Range)
        );
        // A close that names a container.
        assert_eq!(
            encode_action_container(CONT_SELF, 1, &mut buf),
            Err(WireError::Range)
        );
        // And both again off the wire, forged past what the encoder emits.
        let mut w = BitWriter::new(&mut buf);
        w.write(KIND_ACTION, KIND_BITS).unwrap();
        w.write(ACT_CONTAINER, ACTION_SUB_BITS).unwrap();
        w.write(CONT_MAX as u32 + 1, CONT_KIND_BITS).unwrap();
        w.write(0, 32).unwrap();
        let len = w.finish();
        assert_eq!(decode_action(&buf[..len]), Err(WireError::Malformed));

        let mut w = BitWriter::new(&mut buf);
        w.write(KIND_ACTION, KIND_BITS).unwrap();
        w.write(ACT_CONTAINER, ACTION_SUB_BITS).unwrap();
        w.write(CONT_SELF as u32, CONT_KIND_BITS).unwrap();
        w.write(1, 32).unwrap();
        let len = w.finish();
        assert_eq!(decode_action(&buf[..len]), Err(WireError::Malformed));
    }

    /// The action subtype field's spare room, stated as a number.
    ///
    /// `ACT_MAX`'s compile-time assert proves nothing *truncates*; this
    /// says how close the lane is to needing a width bump, so the pass
    /// that spends the last code has to change a line that says so rather
    /// than discover it. Two left after the container verb, and `NOW.md`
    /// already names two more C→S verbs (a repair, a throw).
    #[test]
    fn the_action_lane_has_the_room_it_claims() {
        assert_eq!(ACT_MAX, ACT_CONTAINER);
        assert_eq!(
            (1 << ACTION_SUB_BITS) - 1 - ACT_MAX,
            2,
            "the spare action codes moved — say so where the count is written"
        );
    }
}
