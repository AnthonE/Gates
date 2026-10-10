//! A stack handed from one player to another (`Command::Give`, `NOW.md`
//! §5d): aimed at a standing, awake body in hand reach, what fits in their
//! pack moves and the rest stays with the giver. Every other shape of the
//! verb changes nothing, and only a full pack says so.
use sim_core::{
    gather::{GatherContent, ItemStack},
    input::InputFrame,
    inventory::REFUSE_M_GIVE,
    limits::INV_SLOTS,
    movement::{Body, POS_XZ_Q, POS_Y_Q},
    world::{Command, World, EV_GATHER, EV_MOVE_REFUSED, EV_TRUST, TRUST_GIVE},
};

const GIVER: u32 = 1;
const TAKER: u32 = 2;
/// Stacks to 100 in the probe table.
const WOOD: u16 = 2;
/// Stacks to 1 in the probe table, so a pack of it can never take a top-up.
const PAPER: u16 = 11;

fn fixture() -> World {
    let mut w = World::new(42);
    w.gather = GatherContent::probe_fixture();
    w.dev_spawn = Some(w.spawn_pos(1));
    w.tick(&[
        Command::Join { id: GIVER },
        Command::Join { id: TAKER },
        Command::Join { id: 3 },
    ]);
    let a = w.players[0].body;
    w.players[1].body = Body::at(
        42,
        &w.haven,
        a.qx as f32 * POS_XZ_Q,
        a.qz as f32 * POS_XZ_Q + 1.5,
    );
    for p in w.players.iter_mut() {
        p.inv = [ItemStack::default(); INV_SLOTS];
    }
    w.players[0].inv[3] = ItemStack {
        item: WOOD,
        count: 30,
        cond: 0,
        skin: 0,
    };
    w
}

/// The giver's look at the taker's chest, as `tests/assist.rs` aims.
fn aim(w: &World) -> Command {
    let (a, b) = (w.players[0].body, w.players[1].body);
    let run = (b.qz - a.qz) as f32 * POS_XZ_Q;
    let rise = (b.qy - a.qy) as f32 * POS_Y_Q + 0.3 - 1.6;
    let pitch = (0..=255u8)
        .max_by(|&a, &b| {
            let (ac, as_) = sim_core::pitch_dir(a);
            let (bc, bs) = sim_core::pitch_dir(b);
            (ac * run + as_ * rise).total_cmp(&(bc * run + bs * rise))
        })
        .unwrap();
    Command::Input {
        id: GIVER,
        frame: InputFrame {
            seq: w.tick as u16,
            pitch,
            ..Default::default()
        },
        favour: 0,
    }
}

fn give(w: &mut World, slot: u8, count: u16, target: u32) {
    let a = aim(w);
    w.tick(&[a]);
    let a = aim(w);
    w.tick(&[
        a,
        Command::Give {
            id: GIVER,
            slot,
            count,
            target,
        },
    ]);
}

fn count(w: &World, code: u8) -> usize {
    w.events.entries().iter().filter(|e| e.code == code).count()
}

fn wood(w: &World, slot: usize) -> u32 {
    sim_core::craft::inv_count(&w.players[slot].inv, WOOD)
}

#[test]
fn an_aimed_give_in_reach_moves_the_stack_and_tells_the_taker() {
    let mut w = fixture();
    give(&mut w, 3, 30, TAKER);
    assert_eq!(wood(&w, 0), 0, "the giver still holds the wood");
    assert_eq!(w.players[0].inv[3], ItemStack::default());
    assert_eq!(wood(&w, 1), 30, "the wood never arrived");
    let g: Vec<_> = w
        .events
        .entries()
        .iter()
        .filter(|e| e.code == EV_GATHER)
        .collect();
    assert_eq!(g.len(), 1);
    assert_eq!(
        (g[0].a, g[0].b, g[0].c),
        (TAKER, (WOOD as u32) << 16 | 30, 0)
    );
    assert_eq!(w.trust.rows().len(), 1);
    assert_eq!(w.trust.rows()[0].verb, TRUST_GIVE);
}

#[test]
fn a_give_of_part_of_a_stack_leaves_the_rest() {
    let mut w = fixture();
    give(&mut w, 3, 12, TAKER);
    assert_eq!(wood(&w, 0), 18);
    assert_eq!(wood(&w, 1), 12);
}

#[test]
fn a_stack_keeps_its_condition_and_its_skin() {
    let mut w = fixture();
    let paper = ItemStack {
        item: PAPER,
        count: 1,
        cond: 77,
        skin: 5,
    };
    w.players[0].inv[3] = paper;
    give(&mut w, 3, 1, TAKER);
    assert!(
        w.players[1].inv.contains(&paper),
        "the paper arrived changed: {:?}",
        w.players[1].inv
    );
}

/// Out of reach, at a sleeper, a downed body, a corpse, yourself, an empty
/// slot, a zero count, a slot past the pack and from a giver who is down:
/// nothing moves and nothing is said, as a syringe out of reach says
/// nothing.
#[test]
fn every_wrong_give_changes_nothing() {
    type Setup = fn(&mut World) -> (u8, u16, u32);
    let cases: [(&str, Setup); 9] = [
        ("out of reach", |w| {
            w.players[1].body.qz += 200; // six metres further
            (3, 30, TAKER)
        }),
        ("a sleeper", |w| {
            w.players[1].sleeping = true;
            (3, 30, TAKER)
        }),
        ("a downed body", |w| {
            w.players[1].wounded = true;
            w.players[1].wound_until = w.tick + 900;
            (3, 30, TAKER)
        }),
        ("a corpse", |w| {
            w.players[1].dead = true;
            (3, 30, TAKER)
        }),
        ("yourself", |_| (3, 30, GIVER)),
        ("an empty slot", |_| (4, 30, TAKER)),
        ("a zero count", |_| (3, 0, TAKER)),
        ("a slot past the pack", |_| (INV_SLOTS as u8, 30, TAKER)),
        // Down, the hands are gone (`live_slot_of`), whoever they aim at.
        ("a downed giver", |w| {
            w.players[0].wounded = true;
            w.players[0].wound_until = w.tick + 900;
            (3, 30, TAKER)
        }),
    ];
    for (what, setup) in cases {
        let mut w = fixture();
        let (slot, n, target) = setup(&mut w);
        give(&mut w, slot, n, target);
        assert_eq!(wood(&w, 0), 30, "{what}: the giver lost wood");
        assert_eq!(wood(&w, 1), 0, "{what}: the taker gained wood");
        assert_eq!(count(&w, EV_GATHER), 0, "{what}: a gather was announced");
        assert_eq!(count(&w, EV_MOVE_REFUSED), 0, "{what}: a refusal was said");
        assert_eq!(count(&w, EV_TRUST), 0, "{what}: a trust row was kept");
        assert!(w.trust.is_empty(), "{what}: the ledger kept a row");
    }
}

/// A pack with no room for any of it refuses and says why; a pack with room
/// for some of it takes that much and the rest stays with the giver.
#[test]
fn a_full_pack_refuses_and_a_nearly_full_one_takes_what_fits() {
    let paper = ItemStack {
        item: PAPER,
        count: 1,
        cond: 0,
        skin: 0,
    };
    let mut w = fixture();
    w.players[1].inv = [paper; INV_SLOTS];
    give(&mut w, 3, 30, TAKER);
    assert_eq!(wood(&w, 0), 30, "a refused give spent the wood");
    assert_eq!(wood(&w, 1), 0);
    let r: Vec<_> = w
        .events
        .entries()
        .iter()
        .filter(|e| e.code == EV_MOVE_REFUSED)
        .collect();
    assert_eq!(r.len(), 1, "a full pack must say so");
    assert_eq!((r[0].a, r[0].b), (GIVER, REFUSE_M_GIVE));
    assert_eq!(count(&w, EV_GATHER), 0);
    assert!(w.trust.is_empty(), "a refused give kept a trust row");

    let mut w = fixture();
    w.players[1].inv = [paper; INV_SLOTS];
    w.players[1].inv[7] = ItemStack {
        item: WOOD,
        count: 95,
        cond: 0,
        skin: 0,
    };
    give(&mut w, 3, 30, TAKER);
    assert_eq!(wood(&w, 1), 100, "the top-up did not fill the stack");
    assert_eq!(wood(&w, 0), 25, "the giver lost more than fit");
    assert_eq!(count(&w, EV_MOVE_REFUSED), 0);
    assert_eq!(w.trust.rows().len(), 1);
}
