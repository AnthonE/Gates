//! Gate: what another player is holding — and whether it is burning —
//! reaches your client.
//!
//! **The gap this closes was ranked first by two consecutive judges**
//! (`findings/pass-20260829-153230-04-judge.md` and `-05`): "another
//! player is a mute, empty-handed silhouette". The record carried id,
//! position, look and three bits, and nothing about the hand, so a
//! bow, a hatchet and a lit torch all drew the same figure. Torch fuel
//! v0 made that worse rather than better — it priced carrying a light
//! and left the *disclosure* it is priced for unbuilt, so `ALPHA.md`
//! §1's "light = visibility = target" was a tax with no target in it.
//!
//! What this file owns is the **path**, not the arithmetic.
//! `sim-core/light.rs` owns whether a flame burns and
//! `protocol`'s own suite owns whether the field survives a codec; the
//! thing neither can see is whether the server ever *asks* — `is_lit`
//! and `held_of` were both correct functions before `wire_entity` called
//! either, and every gate in this crate was green over a game where no
//! hand crossed the wire. So every assertion below reads a decoded
//! datagram on the OTHER player's `ClientView`, which is the only place
//! this fact is worth anything.
//!
//! **Not gated here and said plainly:** nobody has seen it. A remote
//! hand is drawn by `client/render/bodies.rs` and no capture in this
//! repo contains two players (`NOW.md` §0tl, `§LOOK`).

use protocol::InputDatagram;
use server::core::{Lane, ShardCore};
use server::stats::ShardStats;
use server::view::{Applied, ClientView};
use sim_core::gather::{GatherContent, ItemStack};
use sim_core::input::{InputFrame, BTN_LIGHT};
use sim_core::limits::SNAPSHOT_INTERVAL_TICKS;

const SEED: u64 = 0x0105_EE17;

/// The lit item in `GatherContent::probe_fixture` — the same row
/// `sim-core/tests/torch.rs` burns, borrowed rather than hunting the
/// shipped catalog for a torch, because what is under test is the wire
/// and not which content file declares a flame.
const LIT_ITEM: u16 = 0;

/// An item that is emphatically not the lit one, for the swap below.
const OTHER_ITEM: u16 = 1;

fn id_of(slot: usize) -> u32 {
    (1 << 8) | slot as u32
}

/// Two bodies, side by side, both inside the other's interest radius, on
/// the fixture content so a hand can hold a flame.
///
/// Boxed for `snapshot_budget.rs`'s reason — `ShardCore` by value
/// overflows a test thread's 2 MiB stack.
fn pair(stats: &ShardStats) -> Box<ShardCore> {
    let mut core = Box::new(ShardCore::new(SEED));
    core.world.gather = GatherContent::probe_fixture();
    for slot in 0..2 {
        assert!(core.connect(slot, id_of(slot)), "connect {slot}");
    }
    core.tick_bare(stats, |_, _, _| true);
    for (i, p) in core.world.players.iter_mut().enumerate() {
        if !p.active {
            continue;
        }
        p.body.qx = ((900.0 + i as f32 * 4.0) / 0.03) as i32;
        p.body.qz = (900.0 / 0.03) as i32;
    }
    core
}

/// Put `item` in the first hotbar slot of the body in connection `slot`.
fn give(core: &mut ShardCore, slot: usize, item: u16, cond: u16) {
    let w = core
        .world
        .players
        .iter()
        .position(|p| p.active && p.id == id_of(slot))
        .expect("the join seated a body");
    core.world.players[w].inv[0] = ItemStack {
        item,
        count: 1,
        cond,
    };
}

/// One input frame for `slot`, selecting hotbar 0 and pressing (or not)
/// the light. Sent through `push_input` — the real path, so the button
/// crosses `world::apply`'s mask exactly as a player's would.
fn press(core: &mut ShardCore, slot: usize, seq: u16, light: bool) {
    let mut dg = InputDatagram::new(0, 0, 4);
    let f = InputFrame {
        seq,
        sel: 0,
        buttons: if light { BTN_LIGHT } else { 0 },
        ..InputFrame::default()
    };
    dg.push(f).expect("one frame fits");
    core.push_input(slot, &dg);
}

/// Tick to the next snapshot boundary and hand back what each client got.
fn snapshot_round(core: &mut ShardCore, stats: &ShardStats) -> Vec<(usize, Vec<u8>)> {
    loop {
        let mut sent = Vec::new();
        core.tick_bare(stats, |lane, slot, bytes| {
            if lane == Lane::Snapshot {
                sent.push((slot, bytes.to_vec()));
            }
            true
        });
        if core.world.tick.is_multiple_of(SNAPSHOT_INTERVAL_TICKS) && !sent.is_empty() {
            return sent;
        }
    }
}

/// What connection `watcher` currently believes about `subject`'s body,
/// decoded from the datagram the server actually sent it.
fn seen(
    core: &mut ShardCore,
    stats: &ShardStats,
    view: &mut ClientView,
    watcher: usize,
    subject: usize,
) -> protocol::EntityState {
    let sent = snapshot_round(core, stats);
    let bytes = sent
        .iter()
        .find(|(s, _)| *s == watcher)
        .map(|(_, b)| b.clone())
        .expect("the watcher got a snapshot");
    match view.apply(&bytes).expect("server datagram decodes") {
        Applied::Ok { .. } => {}
        other => panic!("watcher {watcher}: {other:?}"),
    }
    *view
        .get(id_of(subject))
        .expect("the subject is inside the watcher's interest set")
}

/// **The whole path, one transition at a time.** Each step is a fact the
/// watcher could not have worked out: their peer's inventory is not on
/// their wire and their peer's `BTN_LIGHT` is not their input.
#[test]
fn the_watcher_sees_the_other_hand_and_its_flame() {
    let stats = ShardStats::default();
    let mut core = pair(&stats);
    let mut view = ClientView::new();

    // 1 · An empty hand is `None`, not item 0. This is the assertion the
    // whole `Option` spelling exists for: a defaulted record would say
    // `Some(0)` and `Some(0)` is a real item, so a wire that silently
    // fell back would draw everyone holding whatever the catalog's first
    // row happens to be.
    let e = seen(&mut core, &stats, &mut view, 1, 0);
    assert_eq!(e.held, None, "an empty hotbar slot is an empty hand");
    assert!(!e.lit, "an empty hand is not on fire");

    // 2 · Give them a torch, unlit. The item crosses; the flame does not,
    // because the latch is not down — which is the half a client deriving
    // the flame from the item alone would get wrong, and get wrong in the
    // direction that hands away a player's position.
    give(&mut core, 0, LIT_ITEM, 400);
    press(&mut core, 0, 1, false);
    let e = seen(&mut core, &stats, &mut view, 1, 0);
    assert_eq!(e.held, Some(LIT_ITEM), "the held item crosses");
    assert!(!e.lit, "an unlit torch is not lit");

    // 3 · Light it. Same item, flame on — a `hand_changed` delta whose
    // item did not move, which is exactly the case a per-field flag would
    // have encoded differently.
    press(&mut core, 0, 2, true);
    let e = seen(&mut core, &stats, &mut view, 1, 0);
    assert_eq!(e.held, Some(LIT_ITEM), "the item did not change");
    assert!(e.lit, "the latch, the row and the fuel are all there");

    // 4 · Burn it out. The flame dies with the fuel and the stick stays in
    // the hand — `light::is_lit`'s third fact, seen from another player's
    // screen for the first time.
    give(&mut core, 0, LIT_ITEM, 0);
    press(&mut core, 0, 3, true);
    let e = seen(&mut core, &stats, &mut view, 1, 0);
    assert_eq!(e.held, Some(LIT_ITEM), "a spent torch is still a stick");
    assert!(!e.lit, "no fuel, no flame");

    // 5 · Swap to something that cannot burn: the item moves, the flame
    // stays off, and a transposition of the two fields fails here.
    give(&mut core, 0, OTHER_ITEM, 100);
    press(&mut core, 0, 4, true);
    let e = seen(&mut core, &stats, &mut view, 1, 0);
    assert_eq!(e.held, Some(OTHER_ITEM), "the swap crosses");
    assert!(!e.lit, "an item with no `light_burn` row never lights");

    // 6 · And empty the hand again, so the wire is shown returning to the
    // sentinel rather than only arriving at an item.
    give(&mut core, 0, OTHER_ITEM, 0);
    core.world
        .players
        .iter_mut()
        .find(|p| p.active && p.id == id_of(0))
        .expect("still seated")
        .inv[0] = ItemStack {
        item: OTHER_ITEM,
        count: 0,
        cond: 0,
    };
    press(&mut core, 0, 5, false);
    let e = seen(&mut core, &stats, &mut view, 1, 0);
    assert_eq!(e.held, None, "a spent stack is an empty hand");
}

/// **Your own record carries it too**, and that is not redundant: the fill
/// loop writes the own entity through a separate call site
/// (`encode_snapshot`'s unconditional first record), so a version of this
/// that filled only the ranked loop would pass every assertion above.
#[test]
fn the_own_record_carries_the_hand_as_well() {
    let stats = ShardStats::default();
    let mut core = pair(&stats);
    let mut view = ClientView::new();
    give(&mut core, 0, LIT_ITEM, 400);
    press(&mut core, 0, 1, true);
    let e = seen(&mut core, &stats, &mut view, 0, 0);
    assert_eq!(e.held, Some(LIT_ITEM), "own hand on own record");
    assert!(e.lit, "own flame on own record");
}

/// **A pig holds nothing**, and the record it rides is the same one.
/// `wire_mob` fills six of twelve fields with constants now, and `held`
/// is the one that would be an index into a catalog a mob has no
/// inventory to index — so it is asserted rather than assumed.
#[test]
fn an_animal_is_empty_handed() {
    let stats = ShardStats::default();
    let mut core = pair(&stats);
    let mut view = ClientView::new();
    // Whatever the shard has spawned by the first snapshot; if nothing has,
    // the assertion below is vacuous and says so.
    let mut checked = 0;
    for _ in 0..4 {
        let sent = snapshot_round(&mut core, &stats);
        let Some((_, bytes)) = sent.iter().find(|(s, _)| *s == 0) else {
            continue;
        };
        view.apply(bytes).expect("decodes");
        for (id, e) in view.entities.iter() {
            if sim_core::mob::slot_of_id(*id).is_some() {
                assert_eq!(e.held, None, "mob {id} is holding something");
                assert!(!e.lit, "mob {id} is on fire");
                checked += 1;
            }
        }
    }
    // Not an assertion that mobs exist — `MAX_MOBS` and the spawner are
    // another file's subject. This prints so a future reader knows whether
    // the loop above saw anything at all.
    println!("mob records checked: {checked}");
}
