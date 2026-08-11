//! Packet schemas, bit-level codec, quantization tables, golden tests.
//! Shared native/wasm. Zero game logic (DESIGN.md §4) — the sim's types
//! (`InputFrame`, the quantized body fields) come from `sim-core`; this
//! crate only says how they cross the wire.
//!
//! Two datagram schemas. The widths below are the wire as it stands at
//! [`PROTO_VER`] — not the v0 sketch DESIGN.md §5.4/§5.5 and NETCODE.md §3
//! describe, which several bumps have since moved. The kind field is
//! [`KIND_BITS`] wide, widened 3 → 4 at v27. This block is the only
//! end-to-end statement of either layout, so it is what anyone costing a
//! worst-case packet reads; `test_module_header_states_the_real_kind_width`
//! checks it against the constant, because a byte-golden compares bytes the
//! same encoder produced and can never notice the prose drifting.
//!
//! **Input, C→S** — `kind:4 · snapshot_ack:16 · ack_bits:32 ·
//! first_client_tick:32 · frame_count:4`, then if any frames:
//! `first_seq:16` and per frame `buttons:8 · yaw:16 · pitch:8 · move_x:8 ·
//! move_z:8 · sel:3` (hotbar selector 0–5; 6–7 refuse as malformed).
//! Frames are the client's unacked tail, oldest first, seq-consecutive by
//! construction (seq rides the wire once).
//!
//! **Snapshot, S→C** — `kind:4 · tick:32 · baseline_age:8 ·
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

pub mod auth;
pub mod bits;
pub mod chat;
pub mod event;
pub mod goldens;

pub use auth::{
    siwe_message, Address, Auth, Challenge, Signature, ADDRESS_BYTES, DOMAIN_MAX, NONCE_BYTES,
    SIGNATURE_BYTES, SIWE_MESSAGE_MAX,
};
pub use bits::WireError;
use bits::{BitReader, BitWriter};
pub use chat::{decode_chat, encode_chat, ChatMsg, ChatText, CHAT_MAX_BYTES};
pub use event::{
    decode_event, encode_event_auth, encode_event_bag_dropped, encode_event_bag_removed,
    encode_event_bag_sync, encode_event_build_refused, encode_event_catalog,
    encode_event_charge_placed, encode_event_chat, encode_event_consume_refused,
    encode_event_consumed, encode_event_cont_sync, encode_event_craft_done, encode_event_craft_q,
    encode_event_craft_refused, encode_event_death, encode_event_deploy_defs,
    encode_event_deploy_placed, encode_event_deploy_refused, encode_event_deploy_sync,
    encode_event_door, encode_event_drank, encode_event_gather, encode_event_health,
    encode_event_hit, encode_event_inv, encode_event_knock, encode_event_known,
    encode_event_move_refused, encode_event_moved, encode_event_oven, encode_event_piece_defs,
    encode_event_piece_placed, encode_event_piece_repaired, encode_event_piece_sync,
    encode_event_recipes, encode_event_removed, encode_event_research,
    encode_event_research_refused, encode_event_respawn, encode_event_shot,
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
///
/// v20 gave a base a second direction to move in: the repair verb
/// (`build.rs`). Two messages, both inside fields earlier versions
/// widened — the repair action, the **fifteenth** of v12's sixteen and the
/// last free one, and `SUB_PIECE_REPAIRED`, the **40th** of v13's
/// sixty-four. Nothing existing moved a bit; the version turns because a
/// subtype that used to be `Malformed` now decodes, and both ends must
/// agree on which.
///
/// The event is deliberately `EV_STRUCT_HIT`'s mirror image and packs its
/// numbers the same way, so a client's hp mirror gains a second writer
/// rather than a second rule. Fixtures are keyed `v20_*`.
///
/// v21 let the repair verb reach a deployable — the door, which is what a
/// raid actually comes through. Both of v20's repair messages grew the one
/// bit that says which store the address names, because a door stands *in*
/// its doorway and the two share an address exactly. This is not a new
/// idea on this wire: `SUB_STRUCT_HIT` has carried the same leading bit,
/// for the same reason, since damage first had to say what it hit — repair
/// now reaches precisely what damage reaches and says so identically.
///
/// No field moved and no subtype was added. The version turns because two
/// payloads grew a bit, which is the case wall 6 exists for: a v20 client
/// reading a v21 repair would take the bit as the top of `cx` and mend an
/// address a kilometre away. Fixtures are keyed `v21_*`.
///
/// v22 gave the player the vertical: jump (`sim-core/input.rs` `BTN_JUMP`,
/// `movement.rs`). **This is v18's case in its purest form yet — not one bit
/// of any payload moved, and of the seventy fixtures exactly one changed a
/// byte: the hello, which carries the version itself.** `buttons` has been a
/// full unmasked octet since v0 (`encode_input` writes 8, `decode_input`
/// reads 8), so bit 3 was already crossing the wire intact and was simply
/// ignored on arrival; nothing about the layout is different.
///
/// The version turns because the *meaning* did, and here the mismatch is
/// worse than v18's declined drag. `movement::step` is shared verbatim by the
/// server and `client-core`'s predictor — that sharing IS the
/// quantize-both-sides law (`NETCODE.md` §3) — so a v22 client against a v21
/// server would predict an arc the server never simulates and be hard-snapped
/// back to the ground on every press. Not a refused action but a permanent,
/// silent, per-press misprediction, which is the exact failure `PROTO_VER`
/// exists to convert into a handshake refusal.
///
/// v23 — **the raid verb** (`charge.rs`). Two additions, in the two
/// directions: `ACT_THROW` on the action lane (plant the held throwable at
/// an address, `encode_action_repair`'s layout to the bit) and
/// `SUB_CHARGE_PLACED` on the event lane (a fuse is burning at an
/// address). No existing message moved a bit — this is the v14/v15/v19
/// shape of turn, a new subtype in each direction rather than a widened
/// field.
///
/// It still turns the version, and not as a formality. A v22 client
/// against a v23 server would receive `SUB_CHARGE_PLACED` and decode it as
/// an unknown subtype — malformed, so the whole batch is dropped, taking
/// every event that shared the datagram with it. The failure would present
/// as walls losing hp with no cause drawn and bags vanishing unannounced,
/// which is exactly the class of silent divergence the handshake refusal
/// exists to convert into a clean "wrong version".
///
/// It also spends the last action code: `ACT_MAX` is now full at 15, so
/// v24's action, if it has one, is the width bump.
///
/// v26 — **sleepers.** One bit on `EntityState`, written unconditionally in
/// both the absolute and the delta encoder beside `grounded`: this body is
/// in the world and nobody is driving it (`sim-core/world.rs`
/// `Player::sleeping`, `reference/SAVES.md` §9.1).
///
/// The cheapest turn in this list by bits and one of the least optional by
/// consequence. A v25 client against a v26 server reads every entity record
/// one bit short from `grounded` onward — the velocity flag lands on the
/// sleeping bit, the look angle on the velocity, and a body's *position*
/// survives while its motion and facing become garbage. That is worse than
/// a refused message: it decodes, so nothing reports an error, and the
/// symptom is bodies twitching at wrong angles rather than a version
/// mismatch anyone can read. Exactly the drift wall 6 exists to convert
/// into a handshake refusal.
///
/// Fixtures are keyed `v26_*` — all 74 renamed, four regenerated: the three
/// snapshot cases, which are the only fixtures whose bytes carry an entity
/// record, and the hello, which is the one message that puts the version
/// number itself on the wire.
///
/// v27 — **identity, proved.** The session token is gone from `Hello` and
/// two stream messages replace it: `Challenge` (a 32-byte server nonce and a
/// timestamp) and `Auth` (a 20-byte address and a 65-byte signature). The
/// shard recovers the signer of a SIWE message it rebuilt itself, so the
/// player key is now an Ethereum address — stable across sessions, which the
/// token never was, and verified rather than asserted, which the token never
/// was either. `crates/server/src/auth.rs` has the argument.
///
/// It cost the two structural changes this list has been avoiding.
/// [`KIND_BITS`] widened 3 → 4, moving **every** message by one bit, because
/// all eight codes were spent and a handshake step cannot subtype a message
/// the peer does not yet trust. And [`MAX_STREAM_MSG_BYTES`] went 64 → 128,
/// because an address plus a signature is 85 bytes.
///
/// A v26 client against a v27 server is refused at the version gate, which
/// is the good case and the whole reason that gate reads the version before
/// anything else: every byte after the kind field moved, so there is no
/// interpretation of a v26 frame that a v27 server could take.
///
/// Fixtures are keyed `v27_*` — all 74 renamed and regenerated (the kind
/// width touches every one), plus two new: `v27_challenge` and `v27_auth`.
///
/// v28 — **the oven** (`sim-core/src/oven.rs`): the `SUB_OVEN` event
/// subtype, the 42nd of v13's 64, and `REFUSE_M_BITS` 3 → 4 for the eighth
/// move refusal, `REFUSE_M_OVEN`. Fixtures keyed `v28_*`.
///
/// v29 put animals on the snapshot lane (`sim-core/src/mob.rs`), and it is
/// the cheapest kind of version turn and the most easily got wrong. **Not
/// one bit of layout moved**: an animal is a class-D body exactly as a
/// player is, so it rides the record that already exists, and what says
/// which is which is the high bit of the id (`limits::MOB_ID_TAG`) — a bit
/// the player-id allocator was quietly capable of setting and is now
/// masked out of (`server/net.rs`).
///
/// The version turns anyway, on **v18's precedent stated one screen up**:
/// *a widened meaning is a wire change even when the layout is
/// byte-identical.* The consequence here is worse than v18's declined drag
/// and about as bad as v22's mispredicted jump — a v28 client against a v29
/// server parses every animal as a player and draws a human being standing
/// in a field, walking at half speed and facing wherever the pig is going.
/// It would look like a bug in the renderer forever. The handshake refusing
/// the pairing is the correct outcome, and it is the only mechanism that
/// can produce it.
///
/// The alternative — a `kind` field on `EntityState` — was rejected on
/// cost: it spends bits on every record of every snapshot, including the
/// overwhelming majority that are players, to carry a value that never
/// changes for the life of an entity and that the id already distinguishes.
/// Fixtures are keyed `v29_*`: all renamed, and exactly **one** differs in
/// bytes from its v28 self — `v29_hello`, which carries the version number.
/// That single changed file is what a bump with no layout change looks
/// like, and it is worth looking at: a diff here with more than one file in
/// it would mean the layout moved after all.
///
/// v31 — **the recycler** (recycler v0). One layout change and it is the
/// expensive kind: `ARCH_BITS` widened 3 → 4, because `ARCH_RECYCLER` = 8
/// is the ninth archetype and three bits held exactly eight. Every
/// `SUB_DEPLOY_DEFS` row moved by a bit, both range checks moved to the new
/// ceiling, and all 82 goldens regenerated. A v30 client against a v31
/// server would read every deployable definition one bit short from the
/// archetype onward — placement landing on hp, hp on the item id — so the
/// handshake refusal is doing real work here rather than being a
/// formality.
///
/// v32 — **research** (research v0). Three changes, none of them a width:
/// `ACT_RESEARCH` = 17 is the eighteenth action and the first to spend one
/// of the fifteen codes `ACT_DEMOLISH`'s bump bought; `SUB_RESEARCH`,
/// `SUB_RESEARCH_REFUSED` and `SUB_KNOWN` are three new event subtypes
/// (44–46 of sixty-four); and `SUB_RECIPES` grew **one bit** per row for
/// `RecipeDef::blueprint`, which is the layout move that makes this a
/// turn rather than an additive version. A v31 client would read a recipe
/// row a bit short from `n_inputs` onward and offer a craft ladder whose
/// ingredient lists are garbage.
///
/// v30 — **the code lock, the crew, and demolish**, landed in one turn
/// because they merged from one branch (lock v1 + hearth crew v1 +
/// demolish v1; `reference/DOORS.md` and `reference/BUILDING.md`). Five
/// layout changes:
///
/// - **`ACTION_SUB_BITS` widened 4 → 5**, so every C→S action message
///   moved by one bit and every C→S golden regenerated. This is the price
///   `ACT_THROW`'s comment quoted at v23 and it finally came due:
///   `ACT_MAX` was full at 15, and `ACT_DEMOLISH` addresses two stores
///   with a leading bit rather than asking an access question, so unlike
///   the two verbs before it it could not fold into an op field.
/// - `ACT_LOCK` is now **`ACT_ACCESS`** and carries a 4-bit op plus a
///   14-bit code where its one `locked` bit was. Nine verbs on one action
///   code — the lock's six and the hearth's three crew ops — because they
///   are one question, *who may do this here*, and which store an op
///   addresses is the sim's branch and not the wire's.
/// - `SUB_DOOR` and the placed-deployable record each grew a `has_lock`
///   bit (the record is 32 bits now). "Bare", "bolted" and "shut" are
///   three prompts, and two bits could only say two of them.
/// - Two new S→C subtypes: `SUB_KNOCK` (42, broadcast) and `SUB_AUTH`
///   (43, own-fact). Both inside what v13's widening to `SUB_BITS` = 6
///   already bought, so no field moved for them.
/// - `PLACEMENT_BITS` widened 2 → 3 for `PLACE_DOOR`, the fifth placement
///   class. It rides only the deploy-defs batch.
///
/// Also, and it is a wire change for v18's stated precedent even though
/// no field moved for it: `REFUSE_B_*` grew two reasons (window spent,
/// nothing there) and `REFUSE_D_*` grew six (no lock, has lock, wrong
/// code, locked out, list full, not empty).
///
/// Fixtures are keyed `v30_*` — all renamed and every C→S action
/// regenerated, plus the hello and the three door/deploy-carrying cases,
/// plus three new: `v30_action_demolish`, `v30_knock` and `v30_auth`.
///
/// **v33 — the arrow becomes visible.** One new event subtype,
/// `SUB_SHOT = 47`, carrying the shooter, the two aim angles and the
/// round's speed and drop (`sim-core`'s `EV_SHOT`). Nothing widened and
/// nothing moved: it is a subtype added at the top of a 6-bit field with
/// sixteen values still free, so every other fixture's bytes are
/// unchanged and only the version they are keyed under moves.
///
/// It is a version bump anyway, and that is the rule working rather than
/// an inconvenience. A v32 client handed a `SUB_SHOT` datagram reads an
/// unknown subtype and treats the whole message as malformed; the bump is
/// what makes that a refused handshake instead of a client that silently
/// drops every shot on the shard.
///
/// **It was authored as v31 and landed as v33**, because research v0 took
/// 32 and `SUB_RESEARCH`/`_REFUSED`/`SUB_KNOWN` took 44–46 while this
/// branch was open — a head-on collision on both numbering axes, and the
/// exact case `CLAUDE.md`'s "protocol and limits.rs never land from two
/// branches in one merge window" names. Renumbering above them at the
/// merge is the resolution; nothing about either feature moved.
///
/// Fixtures are keyed `v33_*`, plus one new: `v33_event_shot`.
///
/// v34 put **twig** under wood on the material ladder (`build.rs`
/// `MAT_TWIG = 0`), which renumbered wood, stone and metal to 1, 2 and 3.
/// **`MATERIAL_BITS` did not move** — it has been 2 since v4 and 2 bits
/// hold four rungs exactly, so not one field widened and not one message
/// changed shape. What changed is what the *values* mean: a `1` in that
/// field was wood on a v33 shard and is twig-plus-one on a v34 one, so a
/// v33 client would read every piece in the world one rung stronger than
/// it is and price its own upgrades against the wrong row. That is v18's
/// case exactly — a widened meaning inside an unchanged layout — and it
/// is the case `PROTO_VER` exists for, because no byte-golden can see it.
///
/// **Three** fixtures move their bytes, and the third is the one that
/// proves the rest: `action_upgrade` climbs to metal, which is now 3;
/// `event_piece_defs`'s max-everything row was re-pointed at 3 so the
/// **top** of the field is pinned by a fixture that actually writes it;
/// and `hello`, which carries `PROTO_VER` itself and would move on any
/// bump. The other 80 are byte-identical under a new name — which is the
/// point, because a renumber that left them alone is exactly the shape of
/// wire change a byte-golden cannot see.
///
/// It is also the first bump whose *cause* was caught by a test rather
/// than by a reviewer: `wire_domains::every_domain_fits_its_wire_field`
/// pins each domain's live maximum and failed on `MAT_*` topping out at 3
/// where it was pinned at 2, with the bump, the regeneration and the pin
/// move all named in its message. The pin is now 3.
///
/// Fixtures are keyed `v34_*`. None added, none removed — 83, as v33.
pub const PROTO_VER: u16 = 34;

/// Datagram kind field width.
///
/// **Widened 3 → 4 at v27**, which moved every message on the wire by one
/// bit. Not a decision taken lightly and not one with an alternative: all
/// eight 3-bit codes were spent (the `KIND_CHAT` comment below said the next
/// lane would have to subtype an existing kind), and SIWE needs *two* new
/// stream messages that cannot subtype anything — a challenge the server
/// sends before it knows who the client is, and an auth the client sends
/// before it is anybody. Subtyping either would mean smuggling a handshake
/// step inside a message the peer has to already trust.
///
/// The cost is one bit per datagram, paid on the snapshot lane where it
/// matters most; `test_snapshot_cap_within_budget` still passes, which is
/// the check that says it fits.
pub const KIND_BITS: u32 = 4;
pub const KIND_INPUT: u32 = 0;
pub const KIND_SNAPSHOT: u32 = 1;
/// Stream-lane message kinds (the bidi handshake, DESIGN.md §5.9). Same
/// kind space as the datagrams above — [`KIND_BITS`] wide, so the codes
/// below run past the eight a kind field once held; stream messages ride
/// length-prefixed (u16 LE) frames on the reliable lane, never datagrams.
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
/// last code the 3-bit kind field held. The bump it warned about is what
/// v27 paid for SIWE — see [`KIND_BITS`] for why the two handshake
/// messages below could not subtype an existing kind instead.
pub const KIND_CHAT: u32 = 7;
/// S→C: the SIWE nonce, sent once per connection between the hello and the
/// welcome (`auth.rs`).
pub const KIND_CHALLENGE: u32 = 8;
/// C→S: the address and the signature over the challenge (`auth.rs`).
pub const KIND_AUTH: u32 = 9;

/// Longest stream-lane message payload the handshake accepts. Overflow
/// policy: refuse (`Malformed`) — a hello has no business being big.
///
/// **64 → 128 at v27.** `Auth` carries a 20-byte address and a 65-byte
/// signature, which is 85 bytes before framing and does not fit in 64. The
/// new number is the next round one above what the largest handshake
/// message needs, and it is still small enough that a client cannot make
/// the server allocate anything interesting by lying about a length.
pub const MAX_STREAM_MSG_BYTES: usize = 128;

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
/// The shard requires a session and the token was absent or not good.
///
/// One code for both, deliberately: telling a caller *which* of "you sent
/// nothing" and "your token was rejected" applies is a probing oracle, and
/// the player-facing sentence is the same either way — sign in through the
/// launcher. `server` distinguishes them in its own counters.
pub const REFUSE_AUTH: u8 = 2;
/// This shard checks copies and the chain says this wallet holds none.
///
/// **A separate code from [`REFUSE_AUTH`] on purpose, and it is the opposite
/// call from the one that merged the two auth cases.** There, telling a
/// stranger which way their signature was wrong is a probing oracle and the
/// player-facing sentence is identical either way. Here it is neither: the
/// address was already *proved*, so nothing is leaked by naming the reason
/// that a `balanceOf` anybody can call does not already say — and the two
/// sentences point at different doors. "Sign in through the launcher" is
/// useless advice to somebody who signed in fine and has not bought the
/// game.
///
/// **Only a definite on-chain zero may send this.** A failed read admits;
/// `server/src/entitle.rs` carries the rule and the type that keeps it.
///
/// No layout moved for this: `Refuse.code` has always been a full `u8` and
/// this is the fourth of 256 values, so `PROTO_VER` does not bump and no
/// golden changes. An older client gets `None` from [`refuse_text`] and
/// says so in its own words rather than guessing at a reason.
pub const REFUSE_TICKET: u8 = 3;

/// What to actually say to the player. One implementation, because the two
/// call sites that print a refusal (the client's connect path and the bot
/// helper in `server/net.rs`) each formatted `refused: code {n}` from their
/// own table — a number is not a sentence, and "code 3" is the least useful
/// thing to tell somebody whose fix is to buy a copy. Two tables is two
/// chances to disagree, which is the argument [`siwe_message`] makes about
/// a message and this makes about a sentence.
///
/// **`&'static str`, and `None` rather than a formatted fallback**, because
/// this crate is on the sim side of wall 3: no `String`, no `format!`. That
/// turns out to be the better shape anyway — a shard newer than this build
/// may refuse for something with no word here, and `None` lets each caller
/// say so in its own voice instead of baking one phrasing into the crate
/// that can least afford to allocate.
pub fn refuse_text(code: u8) -> Option<&'static str> {
    Some(match code {
        REFUSE_VERSION => "this shard runs a different build — update the game",
        REFUSE_FULL => "this shard is full",
        REFUSE_AUTH => "this shard needs a signed identity — sign in through the scry launcher",
        REFUSE_TICKET => {
            "this shard checks copies and this wallet holds none — buy a copy on scry, \
             or play a community shard"
        }
        _ => return None,
    })
}

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

/// S→C: sign this nonce. Sent after the version gate and before anything
/// about this client exists — the server has nothing to lose by sending it
/// to a stranger, which is what lets identity be proved in one extra round
/// trip instead of an issuer lookup.
pub fn encode_challenge(msg: &Challenge, buf: &mut [u8]) -> Result<usize, WireError> {
    let mut w = BitWriter::new(buf);
    w.write(KIND_CHALLENGE, KIND_BITS)?;
    auth::write_challenge(&mut w, msg)?;
    Ok(w.finish())
}

pub fn decode_challenge(buf: &[u8]) -> Result<Challenge, WireError> {
    let mut r = BitReader::new(buf);
    if r.read(KIND_BITS)? != KIND_CHALLENGE {
        return Err(WireError::Malformed);
    }
    let msg = auth::read_challenge(&mut r)?;
    expect_zero_padding(&mut r)?;
    Ok(msg)
}

/// C→S: the address, and the signature that proves it.
pub fn encode_auth(msg: &Auth, buf: &mut [u8]) -> Result<usize, WireError> {
    let mut w = BitWriter::new(buf);
    w.write(KIND_AUTH, KIND_BITS)?;
    auth::write_auth(&mut w, msg)?;
    Ok(w.finish())
}

pub fn decode_auth(buf: &[u8]) -> Result<Auth, WireError> {
    let mut r = BitReader::new(buf);
    if r.read(KIND_BITS)? != KIND_AUTH {
        return Err(WireError::Malformed);
    }
    let msg = auth::read_auth(&mut r)?;
    expect_zero_padding(&mut r)?;
    Ok(msg)
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
const ACTION_SUB_BITS: u32 = 5;
const ACT_CRAFT: u32 = 0;
const ACT_CANCEL: u32 = 1;
const ACT_PLACE: u32 = 2;
const ACT_DEPLOY: u32 = 3;
const ACT_FEED: u32 = 4;
const ACT_USE: u32 = 5;
const ACT_ACCESS: u32 = 6;
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
/// Buy a damaged piece back to full, in its own materials (wire v20,
/// `build.rs`). Fifteenth of the sixteen — the last code the four-bit
/// field has to give away for free. A sixteenth costs a width bump, which
/// is a `PROTO_VER` bump and every golden regenerated.
const ACT_REPAIR: u32 = 14;
/// Plant the held throwable against the structure at an address, fuse
/// burning (wire v23, `charge.rs`). **The sixteenth of sixteen** — the
/// code the comment above `ACT_REPAIR` said would cost a width bump, spent
/// on the verb `NOW.md` had already named as its claimant.
///
/// The lane is now full. A seventeenth action costs `ACTION_SUB_BITS`
/// widened to 5, which moves every action message by one bit, which is a
/// `PROTO_VER` turn and all 72 goldens regenerated — the v12 turn, again.
/// That is not a reason to avoid the next verb; it is a reason to know the
/// price before proposing one, which is what `the_action_lane_has_the_room_it_claims`
/// now asserts as **zero**.
const ACT_THROW: u32 = 15;
/// Take the thing at the address back down and get it back (wire v30,
/// demolish v1). **The seventeenth action, and the one the comment above
/// priced**: `ACTION_SUB_BITS` widened 4 → 5 to carry it, which moved
/// every action message by one bit and regenerated every C→S golden.
///
/// It was worth the turn rather than being folded into an existing verb's
/// op field the way the crew ops were folded into `ACT_ACCESS`, and the
/// reason is that demolish is not an access question: it addresses **two
/// stores** with a leading bit like `Repair` and `Throw`, and hanging it
/// off the access verb would have made one action mean "who may" and
/// "take it down" at once. The lane now has fifteen codes spare, which is
/// where the price bought something: the next verb is free — and
/// `ACT_RESEARCH` below is that verb, landed one version later at the cost
/// of a subtype and no layout at all.
const ACT_DEMOLISH: u32 = 16;
/// Learn the blueprint for what is in inventory `slot`, at a research
/// table in reach (wire v32, research v0). **The eighteenth action, and
/// the first one that was free** — `ACTION_SUB_BITS` widened to 5 for
/// `ACT_DEMOLISH` one version earlier and holds thirty-two, so this cost
/// a subtype and not a layout.
///
/// Payload is the slot alone. The table is found by proximity the way a
/// workbench is (`research.rs`), so there is no address to aim and nothing
/// for the client to guess about which table it meant.
const ACT_RESEARCH: u32 = 17;
/// The highest live action code, named rather than counted — the event
/// lane's `SUB_MAX` discipline, which this lane did not have.
///
/// It was worth more here than there while this lane's field was four
/// bits and **full**; at v30 it is five bits with fifteen codes spare, so
/// the pressure is off — but the assert stays, because the failure it
/// prevents is the worst shape of wire drift there is: an action past the
/// field width truncates into a *live* code, and both ends then agree on
/// bytes that mean two different things.
const ACT_MAX: u32 = ACT_RESEARCH;
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
    /// Repair the structure at the address back to its baked hp (repair
    /// v0). Address plus one bit, for `Use`'s reason turned around: the
    /// amount is the sim's arithmetic over its own store, so nothing about
    /// *how much* crosses and nothing about how much can be forged.
    ///
    /// `deploy` picks the store the address names — `StructHit`'s bit, for
    /// `StructHit`'s reason: the door in a doorway and the doorway itself
    /// share one address, so a verb that guessed would mend the wrong one.
    Repair {
        deploy: bool,
        cx: u16,
        cz: u16,
        level: u8,
        loc: u8,
    },
    /// Plant the held throwable against the structure at the address
    /// (charge v0, `charge.rs`). `Repair`'s fields exactly, including the
    /// leading store bit and for its reason.
    ///
    /// Nothing about the charge crosses — not what it takes off a wall,
    /// not how long it burns, not which item is spent. All three are the
    /// held throwable's content row, read server-side, so a client can
    /// choose *where* to plant and nothing else. It is the same posture
    /// `Repair` takes toward the heal amount, applied to a verb where
    /// getting it wrong would let a forged frame pick its own blast.
    Throw {
        deploy: bool,
        cx: u16,
        cz: u16,
        level: u8,
        loc: u8,
    },
    /// Take the thing at the address back down and get it back (demolish
    /// v1). `Repair`'s shape exactly, leading store bit included and for
    /// its reason: a door and its doorway share one address, so a verb
    /// that guessed would take the wrong one.
    ///
    /// Nothing about *what comes back* crosses — the refund is the
    /// content row's own cost, read server-side — which is `Repair`'s
    /// posture toward the heal amount applied to a verb where getting it
    /// wrong would let a forged frame pay itself.
    Demolish {
        deploy: bool,
        cx: u16,
        cz: u16,
        level: u8,
        loc: u8,
    },
    /// Run one access op at the address — **who may do this here**. This
    /// one *does* carry state, and for the same reason `Use` doesn't: an
    /// access press is a deliberate setting, so two racing presses must
    /// agree on the result rather than swap it. Who is remembered is the
    /// sim's verdict.
    ///
    /// **One action with an op field, not nine actions**, and that is a
    /// wire economy decision rather than a taste one: `ACT_MAX` was full
    /// at 15 in `ACTION_SUB_BITS` = 4, so a second access verb would have
    /// cost a width bump on *every* C→S message. Four bits of op inside
    /// this payload cost four bits on this message alone.
    ///
    /// Ops 0..=5 address the code lock on a door and 6..=8 the hearth's
    /// crew (`sim_core::deploy::ACCESS_OP_*`). Which store an op means is
    /// the **sim's** branch, not the wire's — the decoder bounds the field
    /// and carries the address, and nothing here knows what stands there.
    ///
    /// `code` is range-checked here (0..=9999, or `lock::CODE_NONE` for
    /// "clear the guest code") because it is the one field with a
    /// forgeable value that the sim compares for equality — a code past
    /// the digit space could never be entered by a keypad and so could
    /// never be unset by one either. The op is range-checked for the
    /// ordinary reason: four bits hold sixteen values and nine are spent.
    Access {
        cx: u16,
        cz: u16,
        level: u8,
        loc: u8,
        op: u8,
        code: u16,
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
    /// Learn the blueprint for what is in inventory `slot` (research.rs).
    /// `Consume`'s shape exactly, and for the same reason: the slot is the
    /// sender's claim and the sim is the verdict, so a forged index is a
    /// refusal rather than a disconnect.
    Research { slot: u8 },
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
pub fn encode_action_research(slot: u8, buf: &mut [u8]) -> Result<usize, WireError> {
    if slot as usize >= sim_core::limits::INV_SLOTS {
        return Err(WireError::Range);
    }
    let mut w = BitWriter::new(buf);
    w.write(KIND_ACTION, KIND_BITS)?;
    w.write(ACT_RESEARCH, ACTION_SUB_BITS)?;
    w.write(slot as u32, ACTION_SLOT_BITS)?;
    Ok(w.finish())
}

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

pub fn encode_action_repair(
    deploy: bool,
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
    w.write(ACT_REPAIR, ACTION_SUB_BITS)?;
    // Leading, like `encode_event_struct_hit`'s: the bit that says which
    // store an address means is written before the address, both ways.
    w.write_bit(deploy)?;
    w.write(cx as u32, BUILD_CELL_BITS)?;
    w.write(cz as u32, BUILD_CELL_BITS)?;
    w.write(level as u32, BUILD_LEVEL_BITS)?;
    w.write(loc as u32, BUILD_LOC_BITS)?;
    Ok(w.finish())
}

/// Plant the held throwable at the address. `encode_action_repair`'s
/// layout to the bit — same leading store bit, same four address fields,
/// same absence of any payload — because it is the same address said the
/// same way. The two are deliberately not one encoder with a mode flag:
/// the subtype *is* the mode, and a shared body would put the two verbs
/// one transposed argument apart at every call site.
pub fn encode_action_throw(
    deploy: bool,
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
    w.write(ACT_THROW, ACTION_SUB_BITS)?;
    w.write_bit(deploy)?;
    w.write(cx as u32, BUILD_CELL_BITS)?;
    w.write(cz as u32, BUILD_CELL_BITS)?;
    w.write(level as u32, BUILD_LEVEL_BITS)?;
    w.write(loc as u32, BUILD_LOC_BITS)?;
    Ok(w.finish())
}

use event::{ACCESS_OP_BITS, LOCK_CODE_BITS};

/// How *no code* travels. `lock::CODE_NONE` is `u16::MAX` and does not fit
/// [`LOCK_CODE_BITS`], so it rides as the all-ones pattern of that width;
/// the two spellings meet here and in [`wire_code`] alone, and nowhere in
/// the sim.
const LOCK_CODE_WIRE_NONE: u32 = (1 << LOCK_CODE_BITS) - 1;

/// Sim code → wire code. `None` for a value the keypad could never
/// produce, which the encoder turns into `WireError::Range`.
fn wire_code(code: u16) -> Option<u32> {
    if code == sim_core::lock::CODE_NONE {
        return Some(LOCK_CODE_WIRE_NONE);
    }
    (code <= sim_core::lock::CODE_MAX).then_some(code as u32)
}

/// Wire code → sim code. `None` for the forgeable band above 9999.
fn sim_code(raw: u32) -> Option<u16> {
    if raw == LOCK_CODE_WIRE_NONE {
        return Some(sim_core::lock::CODE_NONE);
    }
    (raw <= sim_core::lock::CODE_MAX as u32).then_some(raw as u16)
}

pub fn encode_action_access(
    cx: u16,
    cz: u16,
    level: u8,
    loc: u8,
    op: u8,
    code: u16,
    buf: &mut [u8],
) -> Result<usize, WireError> {
    let Some(raw) = wire_code(code) else {
        return Err(WireError::Range);
    };
    if cx as usize >= sim_core::limits::MAX_BUILD_COORD
        || cz as usize >= sim_core::limits::MAX_BUILD_COORD
        || level as usize >= sim_core::limits::MAX_BUILD_LEVELS
        || loc > sim_core::build::LOC_EDGE_N
        || op > sim_core::deploy::ACCESS_OP_MAX
    {
        return Err(WireError::Range);
    }
    let mut w = BitWriter::new(buf);
    w.write(KIND_ACTION, KIND_BITS)?;
    w.write(ACT_ACCESS, ACTION_SUB_BITS)?;
    w.write(cx as u32, BUILD_CELL_BITS)?;
    w.write(cz as u32, BUILD_CELL_BITS)?;
    w.write(level as u32, BUILD_LEVEL_BITS)?;
    w.write(loc as u32, BUILD_LOC_BITS)?;
    w.write(op as u32, ACCESS_OP_BITS)?;
    w.write(raw, LOCK_CODE_BITS)?;
    Ok(w.finish())
}

/// Take the thing at the address back down (demolish v1).
pub fn encode_action_demolish(
    deploy: bool,
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
    w.write(ACT_DEMOLISH, ACTION_SUB_BITS)?;
    w.write_bit(deploy)?;
    w.write(cx as u32, BUILD_CELL_BITS)?;
    w.write(cz as u32, BUILD_CELL_BITS)?;
    w.write(level as u32, BUILD_LEVEL_BITS)?;
    w.write(loc as u32, BUILD_LOC_BITS)?;
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
        // Address + one bit, every width exact — and deliberately no
        // amount: how much hp is missing and what that costs are the
        // server's facts, so there is no number here for a client to
        // choose. The bit picks the store, and both its values are live,
        // so there is nothing here to bound either.
        ACT_REPAIR => ActionMsg::Repair {
            deploy: r.read_bit()?,
            cx: r.read(BUILD_CELL_BITS)? as u16,
            cz: r.read(BUILD_CELL_BITS)? as u16,
            level: r.read(BUILD_LEVEL_BITS)? as u8,
            loc: r.read(BUILD_LOC_BITS)? as u8,
        },
        // The repair arm's twin, bound for bound. Every field is read at
        // exactly its domain's width, so — as there — there is nothing
        // here left to range-check: `BUILD_LOC_BITS` holds four values and
        // all four are live, and both values of the store bit are live.
        // The verb's refusals are all facts about the world (is there a
        // wall there, is it in reach, is a charge in your hand), and those
        // are the sim's to state, not the decoder's to guess.
        ACT_THROW => ActionMsg::Throw {
            deploy: r.read_bit()?,
            cx: r.read(BUILD_CELL_BITS)? as u16,
            cz: r.read(BUILD_CELL_BITS)? as u16,
            level: r.read(BUILD_LEVEL_BITS)? as u8,
            loc: r.read(BUILD_LOC_BITS)? as u8,
        },
        // Address + op + code. The two payload fields are the only ones
        // on the C→S lane whose bit width holds values the sim has no
        // meaning for, so both are refused here rather than clamped —
        // whether the sender is *allowed* the op stays the sim's verdict.
        ACT_ACCESS => {
            let cx = r.read(BUILD_CELL_BITS)? as u16;
            let cz = r.read(BUILD_CELL_BITS)? as u16;
            let level = r.read(BUILD_LEVEL_BITS)? as u8;
            let loc = r.read(BUILD_LOC_BITS)? as u8;
            let op = r.read(ACCESS_OP_BITS)? as u8;
            let raw = r.read(LOCK_CODE_BITS)?;
            let (Some(code), true) = (sim_code(raw), op <= sim_core::deploy::ACCESS_OP_MAX) else {
                return Err(WireError::Malformed);
            };
            ActionMsg::Access {
                cx,
                cz,
                level,
                loc,
                op,
                code,
            }
        }
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
        ACT_DEMOLISH => ActionMsg::Demolish {
            deploy: r.read_bit()?,
            cx: r.read(BUILD_CELL_BITS)? as u16,
            cz: r.read(BUILD_CELL_BITS)? as u16,
            level: r.read(BUILD_LEVEL_BITS)? as u8,
            loc: r.read(BUILD_LOC_BITS)? as u8,
        },
        ACT_LOOT => ActionMsg::Loot,
        ACT_CONSUME => {
            let slot = r.read(ACTION_SLOT_BITS)? as u8;
            if slot as usize >= sim_core::limits::INV_SLOTS {
                return Err(WireError::Malformed);
            }
            ActionMsg::Consume { slot }
        }
        ACT_RESEARCH => {
            let slot = r.read(ACTION_SLOT_BITS)? as u8;
            if slot as usize >= sim_core::limits::INV_SLOTS {
                return Err(WireError::Malformed);
            }
            ActionMsg::Research { slot }
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
    /// tick. **Nothing on the server reads it.** This line used to say the
    /// field "feeds the server's clock estimate", which described an
    /// estimate that was never built: `Core::push_input` folds the acks and
    /// the frame tail and drops the rest, and the accessor below has no
    /// callers. It stays on the wire because taking it off is a layout
    /// change (wall 6) and because it is what a clock estimate would be
    /// built from — the field is not evidence that one exists.
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
    /// **Nobody is driving this body.** One bit, sent on every record —
    /// absolute and delta alike, beside `grounded` — because it is the
    /// difference between a player and a target and a client that guessed
    /// wrong about it would draw the wrong thing at the one moment it
    /// matters.
    ///
    /// Deliberately not delta-gated behind a `changed` flag like the look
    /// angle. It flips exactly twice in a body's life (a disconnect, a
    /// return), so a change flag would spend a bit to save a bit and add a
    /// state the decoder could get out of step on; unconditional is one
    /// bit, always right, and reads the same in both encoders.
    pub sleeping: bool,
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
    /// Open-addressed id → baseline record, built once in `begin`. Slot
    /// holds `index + 1`; 0 is empty, which is why the table can be bytes
    /// (`MAX_SNAPSHOT_ENTITIES` is 64, so an index+1 never reaches 255).
    ///
    /// **This replaces a linear scan of the baseline per entity, which was
    /// quadratic in the one number the whole snapshot design caps.** Every
    /// `add_entity` used to walk the baseline looking for the same id;
    /// a full snapshot against a full baseline is 64 × 64 comparisons, per
    /// client, at the snapshot cadence — measured 2026-08-11 as ~10 % of
    /// every instruction the shard executed at `MAX_PLAYERS`
    /// (`server/src/bin/profile.rs`).
    ///
    /// **No byte on the wire moves**, which is the only reason it may
    /// land: this changes how the baseline record is *found*, never what is
    /// written once it is found, so `test_protocol_golden` is the proof.
    /// Duplicate ids — which a snapshot cannot hold, but the type does not
    /// forbid — resolve to the same record `iter().find` returned: inserts
    /// walk the probe chain in order, so the earliest occupies the earlier
    /// slot and a lookup meets it first.
    bl_index: [u8; BASELINE_INDEX_SLOTS],
    removed_count_at: usize,
    entity_count_at: usize,
    removed: u32,
    entities: u32,
    entity_started: bool,
}

/// Slots in `SnapshotEncoder::bl_index`: a power of two at twice
/// `MAX_SNAPSHOT_ENTITIES`, so the table never passes half load and the
/// probe stays short. Derived, not a knob.
const BASELINE_INDEX_SLOTS: usize = MAX_SNAPSHOT_ENTITIES * 2;
const _: () = assert!(
    BASELINE_INDEX_SLOTS.is_power_of_two() && MAX_SNAPSHOT_ENTITIES < 255,
    "the baseline index masks with SLOTS-1 and stores index+1 in a byte"
);

/// Home slot for an entity id. The Fibonacci mix `collide::ColIndex` and
/// `occupy::SlotCache` already use, for the reason they use it: player ids
/// are minted `(generation << 8) | slot` and mob ids share a high tag, so
/// the low bits alone would pile a whole class onto one line.
#[inline]
fn bl_home(id: u32) -> usize {
    (id.wrapping_mul(2_654_435_761) >> 16) as usize & (BASELINE_INDEX_SLOTS - 1)
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
        // A baseline is a snapshot that was **sent**, and `add_entity`
        // refuses past `MAX_SNAPSHOT_ENTITIES`, so a longer one cannot
        // exist and is a caller bug. Refused rather than truncated, for
        // the reason above it: silently indexing the first 64 and letting
        // the tail encode absolute would still produce a legal datagram,
        // which is exactly how a caller with a broken baseline would never
        // find out. It is also what makes `bl_index`'s half-load argument a
        // checked precondition instead of a comment.
        if baseline.len() > MAX_SNAPSHOT_ENTITIES {
            return Err(WireError::Cap);
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
        // The baseline index, filled only when there is a baseline to index
        // — a zero-state snapshot has none by definition and would pay a
        // table for nothing. The length check above is what bounds the fill
        // at half the table's slots, so an empty slot always exists and
        // both probe loops terminate.
        let has_baseline = header.baseline_age != 0;
        let mut bl_index = [0u8; BASELINE_INDEX_SLOTS];
        if has_baseline {
            for (i, e) in baseline.iter().enumerate() {
                let mut ix = bl_home(e.id);
                while bl_index[ix] != 0 {
                    ix = (ix + 1) & (BASELINE_INDEX_SLOTS - 1);
                }
                bl_index[ix] = (i + 1) as u8;
            }
        }
        Ok(Self {
            w,
            baseline,
            has_baseline,
            bl_index,
            removed_count_at,
            entity_count_at,
            removed: 0,
            entities: 0,
            entity_started: false,
        })
    }

    /// Which baseline record carries `id`, or `None`. What
    /// `baseline.iter().find(|b| b.id == id)` answered, in a probe.
    ///
    /// An index rather than the record, so the caller can copy the record
    /// out and stop borrowing `self` — `encode_delta` needs `&mut self`,
    /// and `EntityState` is `Copy` precisely so that costs nothing. The
    /// probe terminates because `begin` never inserts more than
    /// `MAX_SNAPSHOT_ENTITIES` entries into twice as many slots, so an
    /// empty slot always exists to stop on.
    #[inline]
    fn baseline_ix(&self, id: u32) -> Option<usize> {
        let mut ix = bl_home(id);
        loop {
            let slot = self.bl_index[ix];
            if slot == 0 {
                return None;
            }
            let i = slot as usize - 1;
            if self.baseline[i].id == id {
                return Some(i);
            }
            ix = (ix + 1) & (BASELINE_INDEX_SLOTS - 1);
        }
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
            if let Some(b) = self.baseline_ix(e.id).map(|i| self.baseline[i]) {
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
                    return self.encode_delta(e, &b, dx, dy, dz);
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
        self.w.write_bit(e.sleeping)?;
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
        self.w.write_bit(e.sleeping)?;
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
        let sleeping = r.read_bit()?;
        let qvy = read_vel(r)?;
        return Ok(EntityState {
            id,
            qx,
            qy,
            qz,
            qvy,
            grounded,
            sleeping,
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
    e.sleeping = r.read_bit()?;
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
            sleeping: false,
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

    /// A baseline longer than a snapshot can carry is a caller bug, and the
    /// encoder says so rather than indexing what fits. The cap is also what
    /// keeps `bl_index` under half load, so this is the gate on the probe
    /// terminating as well as on the refusal.
    ///
    /// Both sides, because a refusal that also refuses the legal case is a
    /// worse bug than the one it prevents: exactly `MAX_SNAPSHOT_ENTITIES`
    /// is the largest baseline the fill can produce and it must be accepted.
    #[test]
    fn a_baseline_longer_than_a_snapshot_is_refused() {
        let hdr = SnapshotHeader {
            tick: 1,
            baseline_age: 1,
            last_executed_seq: 0,
            nudge: Nudge::Ok,
        };
        let mut buf = [0u8; DATAGRAM_BUDGET_BYTES];
        let full: Vec<EntityState> = (0..MAX_SNAPSHOT_ENTITIES as u32).map(ent).collect();
        assert!(
            SnapshotEncoder::begin(&mut buf, &hdr, &full).is_ok(),
            "a full baseline is what the fill loop produces every snapshot"
        );
        let over: Vec<EntityState> = (0..MAX_SNAPSHOT_ENTITIES as u32 + 1).map(ent).collect();
        assert!(matches!(
            SnapshotEncoder::begin(&mut buf, &hdr, &over),
            Err(WireError::Cap)
        ));
    }

    /// The baseline lookup is an index now, not a scan, and the delta it
    /// picks has to be the record `iter().find` would have picked. Every id
    /// in a full baseline, looked up through the real encode path: a probe
    /// that lost one would encode absolute, which is legal on the wire and
    /// therefore invisible to a golden — so the byte length is what tells.
    #[test]
    fn every_baseline_id_is_found_by_the_index() {
        let hdr = SnapshotHeader {
            tick: 2,
            baseline_age: 1,
            last_executed_seq: 0,
            nudge: Nudge::Ok,
        };
        // Ids shaped like the shard's: `(generation << 8) | slot` for
        // players and the mob tag above them, so the probe meets the same
        // clustering the real table does.
        let baseline: Vec<EntityState> = (0..MAX_SNAPSHOT_ENTITIES as u32)
            .map(|i| {
                ent(if i < 40 {
                    (1 << 8) | i
                } else {
                    0x4000_0000 | i
                })
            })
            .collect();
        let mut buf = [0u8; DATAGRAM_BUDGET_BYTES];
        let mut enc = SnapshotEncoder::begin(&mut buf, &hdr, &baseline).expect("begin");
        for b in &baseline {
            // Identical to its baseline record: a found delta writes the id
            // plus six flag bits and nothing else.
            assert_eq!(enc.add_entity(b), Ok(()), "entity {} did not fit", b.id);
        }
        let len = enc.finish().expect("finish");
        // 64 entities × (32 id bits + 6 delta bits) plus the header; an
        // absolute record is 32 + 66 bits, so a single missed lookup adds
        // 60 bits and this bound catches it.
        let head_bits = KIND_BITS + 32 + 8 + 16 + 2 + COUNT_BITS * 2;
        let want = (head_bits as usize + MAX_SNAPSHOT_ENTITIES * 38).div_ceil(8);
        assert_eq!(
            len, want,
            "the encoder wrote {len} B where {want} B is every entity delta-coded \
             — a baseline id the index failed to find fell back to absolute"
        );
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
    /// than discover it.
    ///
    /// It worked exactly as intended twice. The throw verb (v23) spent the
    /// last four-bit code and moved this count to **zero**; the two passes
    /// after it then read the price and *dodged* it — the lock's six ops
    /// and the hearth's three both went into `ACT_ACCESS`'s op field
    /// instead of taking action codes. The demolish verb (v30) is the one
    /// that could not dodge, because it addresses two stores rather than
    /// asking an access question, so it paid the bump: five bits, every
    /// action message a bit longer, every C→S golden regenerated. The
    /// count was **fifteen** at v30, and `ACT_RESEARCH` spent the first
    /// of them at v32 — which is the shape this assert exists to make
    /// visible: a verb landing is supposed to move a number in a comment,
    /// not slip in against a stale one.
    #[test]
    fn the_action_lane_has_the_room_it_claims() {
        assert_eq!(ACT_MAX, ACT_RESEARCH);
        assert_eq!(
            (1 << ACTION_SUB_BITS) - 1 - ACT_MAX,
            14,
            "the spare action codes moved — say so where the count is written"
        );
    }

    /// **Every `REFUSE_*` code this crate declares has a real sentence.**
    ///
    /// Inherited from `client::ui::refusals`, which carried a hand-written
    /// copy of these strings and a gate that counted the constants here.
    /// The copy is gone (one implementation, both sides — the argument
    /// `siwe_message` makes two hundred lines up), so the gate comes with
    /// it: this crate now owns the words and owns the check that a new code
    /// cannot reach a player as `code N`.
    ///
    /// Reads its own source, because the alternative is a list of constants
    /// typed twice — which is the thing being prevented.
    #[test]
    fn every_refusal_code_has_a_sentence() {
        let src = include_str!("lib.rs");
        let declared: Vec<&str> = src
            .lines()
            .filter_map(|l| l.trim_start().strip_prefix("pub const REFUSE_"))
            .filter_map(|l| l.split(':').next())
            .collect();
        assert!(
            declared.len() >= 4,
            "the scan found {} codes, so it is not reading this file",
            declared.len()
        );
        for (code, name) in [
            (REFUSE_VERSION, "REFUSE_VERSION"),
            (REFUSE_FULL, "REFUSE_FULL"),
            (REFUSE_AUTH, "REFUSE_AUTH"),
            (REFUSE_TICKET, "REFUSE_TICKET"),
        ] {
            assert!(
                refuse_text(code).is_some(),
                "{name} has no sentence — a player would meet it as a bare number"
            );
        }
        assert_eq!(
            declared.len(),
            4,
            "a REFUSE_* code was added ({declared:?}) — give it a sentence in \
             `refuse_text` and a row in this list, or a player meets it as a number"
        );
    }

    /// Two codes reading the same is a player who cannot tell which no they
    /// got — and the ticket/auth pair is the one that would actually collide,
    /// because both are "the shard would not let me in".
    #[test]
    fn no_two_refusals_read_the_same() {
        let all = [REFUSE_VERSION, REFUSE_FULL, REFUSE_AUTH, REFUSE_TICKET];
        for (i, a) in all.iter().enumerate() {
            for b in all.iter().skip(i + 1) {
                assert_ne!(
                    refuse_text(*a).expect("gated above"),
                    refuse_text(*b).expect("gated above"),
                    "codes {a} and {b}"
                );
            }
        }
    }

    /// The layout claim in `REFUSE_TICKET`'s own doc: it is a value in a
    /// field that was always eight bits wide, so nothing about the wire
    /// moved and `PROTO_VER` does not bump. If this ever fails, the doc
    /// comment is lying and the golden needs regenerating.
    #[test]
    fn a_new_refusal_code_does_not_move_the_wire() {
        let mut a = [0u8; 8];
        let mut b = [0u8; 8];
        let la = encode_refuse(&Refuse { code: REFUSE_AUTH }, &mut a).unwrap();
        let lb = encode_refuse(
            &Refuse {
                code: REFUSE_TICKET,
            },
            &mut b,
        )
        .unwrap();
        assert_eq!(la, lb, "a refusal is a fixed-width frame whatever the code");
        assert_eq!(decode_refuse(&b[..lb]).unwrap().code, REFUSE_TICKET);
        // And an older build's code still decodes to itself, which is what
        // makes this backward-compatible rather than merely small.
        assert_eq!(decode_refuse(&a[..la]).unwrap().code, REFUSE_AUTH);
    }
}
