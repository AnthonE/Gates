//! **Can a raid actually finish?** A satchel, on the shipped content, put
//! against a shipped twig foundation, left alone for the shipped fuse —
//! and at the end of it the wall is gone and the raider was told so.
//!
//! This gate exists because the judge ranked *"you cannot get into
//! anyone's base"* as the largest playable gap in the game
//! (`gates-loop/findings/pass-20260813-230343-16-judge.md` §B.1), off a
//! measurement that could not explain itself: 27 charges armed over 905
//! shard ticks against a 300-tick fuse and `EV_STRUCT_HIT` never once
//! reached a client (`findings/note-20260814-charge-never-detonates.md`).
//! Emit, routing, ring pressure and decode were each ruled out there. What
//! was never tried is the cheap thing — run the same verb in the sim, on
//! the content the shard actually loads, and see.
//!
//! **Why the existing blast tests could not have caught this.**
//! `sim-core/tests/blast.rs` is a fixture: it writes its own throwable row
//! and, in its own words, bumps the piece rows to `hp = 1000` because "the
//! probe rows die outright to one satchel". `raid_storm.rs` asserts
//! `EV_STRUCT_HIT` is reachable, also on a fixture. So every blast in this
//! tree until now has been a charge against a wall that SURVIVES it, at a
//! 60-tick fuse. The shipped case is the opposite on both axes — a twig
//! foundation is hp 10 against `structure` 125, a 12.5× overkill, on a
//! 300-tick fuse — and it is the case a player is in.
//!
//! Nothing here is granted that a shard would not have: the content is
//! `content/`, baked; the fuse is the shipped `fuse_s`; the piece is the
//! shipped row at the shipped hp. The one fixture is the raider's pocket,
//! which `bot_smoke.rs` already documents as load-bearing and the judge
//! ranked separately (§B.3, the progression ladder) — that is a different
//! gap and this gate does not pretend to close it.
//!
//! No clock, no sockets: ticks and observable state.

use sim_core::build::{foundation_terrain_ok, BUILD_CELL_M, LOC_PLANE};
use sim_core::gather::ItemStack;
use sim_core::input::InputFrame;
use sim_core::movement::Body;

/// The solved authored sites for `seed` — what `terrain::ground` needs in order
/// to know where the carve is.
///
/// Memoized per seed, and that is not premature: `terrain::haven` is a few
/// thousand `height` taps (a shoreline march, a bisect and a rosette per
/// candidate bearing), these suites call it from inside assertion loops, and
/// the first draft of this helper resolved it per call and took the workspace
/// test run past five minutes. It is a pure function of the seed, so caching
/// cannot change a result.
fn hv(seed: u64) -> &'static sim_core::terrain::Haven {
    use std::cell::RefCell;
    // A thread-local rather than a `Mutex`: `std::sync::Mutex` is on
    // `sim-core/clippy.toml`'s disallowed list (wall 3), and that list is
    // crate-scoped, so it binds this suite too. Per-thread is the right shape
    // anyway — the cache exists to stop a per-assertion recompute, not to be
    // shared.
    thread_local! {
        static CACHE: RefCell<Vec<(u64, &'static sim_core::terrain::Haven)>> =
            const { RefCell::new(Vec::new()) };
    }
    let hit = CACHE.with(|c| c.borrow().iter().find(|(s, _)| *s == seed).map(|&(_, h)| h));
    if let Some(h) = hit {
        return h;
    }
    let h: &'static sim_core::terrain::Haven = Box::leak(Box::new(sim_core::terrain::haven(seed)));
    CACHE.with(|c| c.borrow_mut().push((seed, h)));
    h
}

use sim_core::world::{
    Command, SimEvent, World, EV_CHARGE_PLACED, EV_PIECE_REMOVED, EV_STRUCT_HIT,
};

const SEED: u64 = 0xB1A57;
const RAIDER: u32 = 3;

/// The shipped fuse is 10 s at `TICK_HZ` 30 = 300 ticks. Run past it with
/// room to spare, and assert on the event rather than on the margin — a
/// window is not the claim, the detonation is.
const FUSE_WINDOW_TICKS: u32 = 400;

fn cell_center(cx: u16, cz: u16) -> (f32, f32) {
    (
        (cx as f32 + 0.5) * BUILD_CELL_M,
        (cz as f32 + 0.5) * BUILD_CELL_M,
    )
}

/// `blast.rs`'s search, for `blast.rs`'s reason: the terrain rule decides
/// where a foundation may stand, so the test asks it rather than guessing
/// a cell and reddening on a worldgen change that is nobody's bug.
fn buildable_cell(seed: u64) -> (u16, u16) {
    for r in 0..64i32 {
        for dz in -r..=r {
            for dx in -r..=r {
                if dx.abs() != r && dz.abs() != r {
                    continue;
                }
                let cx = (512 + dx).clamp(0, 1023) as u16;
                let cz = (512 + dz).clamp(0, 1023) as u16;
                let (x, z) = cell_center(cx, cz);
                if foundation_terrain_ok(seed, hv(seed), x, z) {
                    return (cx, cz);
                }
            }
        }
    }
    panic!("no buildable cell");
}

/// The rows a raid needs, resolved by id off the shipped tables. There is
/// no name lookup in `sim-core` (wall 1 forbids the strings), so the test
/// resolves here and passes integers down, exactly as `bots.rs` does.
struct Raid {
    w: World,
    cx: u16,
    cz: u16,
    /// The build row the tests place. The satchel and the wood are spent
    /// inside `shipped_world` and never named again, so they are not
    /// carried here.
    twig: u16,
}

/// Everything the shard bakes that a raid touches. Deliberately the whole
/// set rather than the two tables this file reads: a content set that
/// disarmed one of the others would change what the verb does, and the
/// point of the gate is that it runs on what ships.
fn shipped_world() -> Raid {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../content");
    let c = content::Content::load_dir(&dir).expect("shipped content loads");
    let mut w = World::new(SEED);
    w.gather = c.bake_gather().expect("gather bakes");
    w.build = c.bake_building().expect("building bakes");
    w.deploy = c.bake_deployables().expect("deployables bake");
    w.combat = c.bake_combat().expect("weapons bake");
    w.spawn_kit = c.bake_spawn_kit().expect("the kit bakes");

    let twig = c
        .piece_index("build.foundation_twig")
        .expect("shipped twig foundation");
    let satchel = c
        .item_index("item.satchel_charge")
        .expect("shipped satchel");
    let wood = c.item_index("item.wood").expect("shipped wood");

    // The two numbers this gate is actually about, asserted rather than
    // assumed — a rebalance that moved either would otherwise leave the
    // test passing while measuring a different claim.
    assert_eq!(
        w.build.pieces[twig as usize].hp, 10,
        "the shipped twig foundation is hp 10"
    );
    let throw = w
        .combat
        .held_throw(satchel)
        .expect("the shipped satchel is a live throwable");
    assert_eq!(throw.structure, 125, "the shipped satchel is structure 125");
    assert_eq!(
        throw.fuse_ticks, 300,
        "the shipped fuse is 10 s at TICK_HZ 30"
    );
    assert!(
        throw.structure > w.build.pieces[twig as usize].hp,
        "this gate's whole point is the OVERKILL case blast.rs avoids: \
         structure {} against hp {}",
        throw.structure,
        w.build.pieces[twig as usize].hp
    );

    let (cx, cz) = buildable_cell(SEED);
    let (x, z) = cell_center(cx, cz);
    w.dev_spawn = Some((x, z));
    w.tick(&[Command::Join { id: RAIDER }]);
    w.players[0].body = Body::at(SEED, hv(SEED), x, z);
    // The raider's pocket. A fixture, and named as one: §B.3 of the judge
    // report is that a player cannot craft their way to a satchel, which
    // is a gap this gate does not close and must not paper over.
    w.players[0].inv[0] = ItemStack {
        item: wood,
        count: 200,
        cond: 0,
    };
    w.players[0].inv[1] = ItemStack {
        item: satchel,
        count: 1,
        cond: 0,
    };
    Raid { w, cx, cz, twig }
}

/// Tick once and hand back every event the tick produced — `World::tick`
/// clears the ring at the top, so a test that only looks at the end sees
/// nothing. This is the same shape `event_roles.rs` uses.
fn tick_events(w: &mut World, cmds: &[Command]) -> Vec<SimEvent> {
    w.tick(cmds);
    w.events.entries().to_vec()
}

fn piece_hp(w: &World, cx: u16, cz: u16, level: u8, loc: u8) -> Option<u16> {
    w.pieces
        .entries()
        .iter()
        .find(|p| p.cx == cx && p.cz == cz && p.level == level && p.loc == loc)
        .map(|p| p.hp)
}

/// **The raid completes.** A twig foundation goes up on shipped content, a
/// shipped satchel is planted on it, the shipped fuse runs out, and the
/// foundation is gone — with `EV_STRUCT_HIT` announcing the hit that took
/// it, because a wall that vanishes with no event is a wall that vanishes
/// on the server and stands forever on every client.
#[test]
fn a_shipped_satchel_breaks_a_shipped_twig_foundation() {
    let Raid {
        mut w,
        cx,
        cz,
        twig,
    } = shipped_world();

    w.tick(&[Command::Place {
        id: RAIDER,
        row: twig,
        cx,
        cz,
        level: 0,
        loc: LOC_PLANE,
        freehand: false,
    }]);
    assert_eq!(
        w.pieces.len(),
        1,
        "the twig foundation must go up — everything after this measures it"
    );
    assert_eq!(
        piece_hp(&w, cx, cz, 0, LOC_PLANE),
        Some(10),
        "and it stands at the shipped twig hp"
    );

    // Select the satchel (hotbar slot 1 rides the input frame), then plant.
    w.tick(&[Command::Input {
        id: RAIDER,
        frame: InputFrame {
            seq: 900,
            sel: 1,
            ..InputFrame::default()
        },
        favour: 0,
    }]);
    let planted = tick_events(
        &mut w,
        &[Command::Throw {
            id: RAIDER,
            deploy: false,
            cx,
            cz,
            level: 0,
            loc: LOC_PLANE,
        }],
    );
    assert_eq!(
        w.charges.len(),
        1,
        "the plant must take on shipped content — a refusal here is the bug \
         one step earlier than the one this gate is looking for"
    );
    assert!(
        planted.iter().any(|e| e.code == EV_CHARGE_PLACED),
        "and it must announce itself"
    );

    // Stand clear: the shipped satchel is 475 body damage and a corpse
    // would change which sweeps run. The raid, not the suicide, is the
    // claim.
    let (x, z) = cell_center(cx, cz);
    w.players[0].body = Body::at(SEED, hv(SEED), x + 40.0, z);

    let mut hit: Option<SimEvent> = None;
    let mut removed = false;
    let mut fired_at: Option<u32> = None;
    for t in 0..FUSE_WINDOW_TICKS {
        let evs = tick_events(&mut w, &[]);
        for e in &evs {
            if e.code == EV_STRUCT_HIT && hit.is_none() {
                hit = Some(*e);
                fired_at = Some(t);
            }
            if e.code == EV_PIECE_REMOVED {
                removed = true;
            }
        }
        if w.charges.is_empty() && hit.is_some() {
            break;
        }
    }

    let hit = hit.unwrap_or_else(|| {
        panic!(
            "the charge never detonated on shipped content: {} ticks after a \
             plant that took, charges left = {}, foundation hp = {:?}, \
             pieces = {}. This is the defect \
             findings/note-20260814-charge-never-detonates.md measured over \
             the wire, reproduced in the sim.",
            FUSE_WINDOW_TICKS,
            w.charges.len(),
            piece_hp(&w, cx, cz, 0, LOC_PLANE),
            w.pieces.len()
        )
    });

    // The fuse is a commitment, and its length is the defender's warning.
    // Detonating early would be a different game, so the tick it fired on
    // is asserted and not merely observed.
    let fired_at = fired_at.expect("a hit has a tick");
    assert!(
        (299..=301).contains(&fired_at),
        "the shipped 300-tick fuse must burn its whole length, fired at {fired_at}"
    );
    assert!(w.charges.is_empty(), "a detonated charge leaves the store");

    // The hit is on OUR address, and it took the foundation with it — the
    // overkill case, where `dealt` is the piece's whole hp and not the
    // charge's whole `structure`.
    assert_eq!(
        hit.a,
        sim_core::gather::cell_key(cx, cz),
        "EV_STRUCT_HIT names the cell that was raided"
    );
    assert_eq!((hit.b >> 16) as u8, 0, "level 0");
    assert_eq!(
        ((hit.b >> 8) & 0xFF) as u8,
        LOC_PLANE,
        "the plane that was planted on"
    );
    let (dealt, left) = ((hit.c >> 16) as u16, (hit.c & 0xFFFF) as u16);
    assert_eq!(left, 0, "125 into a 10 hp foundation leaves nothing");
    assert_eq!(
        dealt, 10,
        "and the damage REPORTED is what the piece had, not the charge's 125 \
         — a client drawing a health bar off this must not be handed a \
         number larger than the bar"
    );

    assert!(
        removed,
        "the broken foundation must announce its removal too"
    );
    assert_eq!(
        piece_hp(&w, cx, cz, 0, LOC_PLANE),
        None,
        "the foundation is GONE — this is the sentence 'you can get into \
         someone's base' reduces to"
    );
    assert_eq!(w.pieces.len(), 0, "and the store agrees");
}

/// **The charge survives the wait.** The defect measured over the wire was
/// consistent with the charge being dropped from the store during its
/// fuse, so that is asserted directly rather than inferred from the blast:
/// the store holds exactly one live charge on every tick until it fires.
///
/// Separate from the test above because it fails differently — if this
/// reddens, nothing is wrong with `detonate` at all.
#[test]
fn a_shipped_fuse_burns_its_whole_length_without_being_swept() {
    let Raid {
        mut w,
        cx,
        cz,
        twig,
    } = shipped_world();
    w.tick(&[Command::Place {
        id: RAIDER,
        row: twig,
        cx,
        cz,
        level: 0,
        loc: LOC_PLANE,
        freehand: false,
    }]);
    w.tick(&[Command::Input {
        id: RAIDER,
        frame: InputFrame {
            seq: 900,
            sel: 1,
            ..InputFrame::default()
        },
        favour: 0,
    }]);
    w.tick(&[Command::Throw {
        id: RAIDER,
        deploy: false,
        cx,
        cz,
        level: 0,
        loc: LOC_PLANE,
    }]);
    assert_eq!(w.charges.len(), 1, "the plant took");
    let (x, z) = cell_center(cx, cz);
    w.players[0].body = Body::at(SEED, hv(SEED), x + 40.0, z);

    // 299 ticks: one short of the fuse, so the charge is still burning at
    // the end of the loop and every tick in between is checked.
    for t in 0..299 {
        w.tick(&[]);
        assert_eq!(
            w.charges.len(),
            1,
            "the charge left the store at tick {t}, before its fuse — \
             nothing may sweep a live fuse"
        );
        assert!(
            piece_hp(&w, cx, cz, 0, LOC_PLANE).is_some(),
            "the target left the store at tick {t}, before the fuse burned: \
             a charge whose wall decayed out from under it detonates into \
             an empty cell and reports nothing"
        );
    }
}
