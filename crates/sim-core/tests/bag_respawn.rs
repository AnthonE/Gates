//! Respawn on your own bag: the mechanic that converts "I built a base"
//! into "I have a base".
//!
//! `deploy.rs`'s own unit tests own the *scan* — nearest, owner-only, spent
//! for its cooldown, blind to every archetype that is not a bag. This file
//! owns the **consequence**, which lives in `World::die`/`World::wake` and
//! which the deploy module cannot reach: that a death lays the body down
//! and the *player's answer* consults the scan, that the spawn ring is
//! still there when it answers `None`, that asking for a beach never spends
//! a bag, and that a body waking on a bag wakes with everything else a
//! respawn owes it.
//!
//! Nothing here invents a number. The clock that does the killing is
//! `SurvivalContent::probe_fixture`, whose spans are seconds precisely so a
//! whole death fits inside a test; the cooldown is `BAG_COOLDOWN_TICKS`,
//! which is `DECISIONS.md` §open's spoken five minutes.

use sim_core::build::{foundation_terrain_ok, BUILD_CELL_M, LOC_PLANE};
use sim_core::combat::CombatContent;
use sim_core::deploy::{DeployContent, BAG_COOLDOWN_TICKS};
use sim_core::gather::ItemStack;
use sim_core::limits::TICK_HZ;
use sim_core::survival::SurvivalContent;
use sim_core::world::{Command, World, EV_RESPAWN};

const SEED: u64 = 20260803;

/// Row 3 of `DeployContent::probe_fixture` is the ground-class bag, and it
/// costs one unit of fixture item 5.
const BAG_ROW: u16 = 3;
const BAG_ITEM: u16 = 5;

/// A cell whose center will hold a ground-class deployable, found by
/// scanning the heightfield rather than by typing a coordinate that held at
/// one seed. The rule asked is `build::foundation_terrain_ok` — the same
/// one `deploy::ground_ok` applies — so this fixture cannot drift away from
/// what the sim actually refuses.
///
/// The scan starts one cell off the given point and spirals outward in
/// rings, so "a second bag" lands near the first rather than across the
/// island: the nearest-bag rule is only interesting when the alternatives
/// are close enough for a player to have built both.
fn buildable_cell_near(seed: u64, cx0: u16, cz0: u16, skip: usize) -> (u16, u16) {
    let mut found = 0usize;
    for r in 0..64i32 {
        for dz in -r..=r {
            for dx in -r..=r {
                // Ring, not disc: the interior was covered by smaller r.
                if dx.abs() != r && dz.abs() != r {
                    continue;
                }
                let cx = (cx0 as i32 + dx).clamp(0, 1023) as u16;
                let cz = (cz0 as i32 + dz).clamp(0, 1023) as u16;
                let (x, z) = cell_center(cx, cz);
                if foundation_terrain_ok(seed, x, z) {
                    if found == skip {
                        return (cx, cz);
                    }
                    found += 1;
                }
            }
        }
    }
    panic!("no buildable cell within 64 cells — the generator changed under this test");
}

fn cell_center(cx: u16, cz: u16) -> (f32, f32) {
    (
        (cx as f32 + 0.5) * BUILD_CELL_M,
        (cz as f32 + 0.5) * BUILD_CELL_M,
    )
}

/// One player, hp from the combat fixture (an inert table grants zero hp
/// and a body at zero can never *reach* zero), the clock armed so hunger
/// can do the killing, and the deploy table that knows what a bag is.
fn lone_world() -> World {
    let mut w = World::new(SEED);
    w.combat = CombatContent::probe_fixture();
    w.survival = SurvivalContent::probe_fixture();
    w.deploy = DeployContent::probe_fixture();
    w.tick(&[Command::Join { id: 1 }]);
    w
}

fn stand(w: &mut World, cx: u16, cz: u16) {
    let (x, z) = cell_center(cx, cz);
    w.players[0].body = sim_core::movement::Body::at(w.seed, x, z);
}

/// Stand on the cell and place a bag on it through the real verb — no
/// store surgery, so every refusal the sim would raise is one this fixture
/// would trip over rather than route around.
fn place_bag(w: &mut World, cx: u16, cz: u16) {
    stand(w, cx, cz);
    w.players[0].inv[10] = ItemStack {
        item: BAG_ITEM,
        count: 1,
    };
    let before = w.deploys.len();
    w.tick(&[Command::PlaceDeploy {
        id: 1,
        row: BAG_ROW,
        cx,
        cz,
        level: 0,
        loc: LOC_PLANE,
    }]);
    assert_eq!(
        w.deploys.len(),
        before + 1,
        "the bag did not place at ({cx}, {cz}) — the fixture, not the mechanic"
    );
}

/// Empty both meters and run until the clock takes the body. The body is
/// on the death screen when this returns and has gone nowhere: since v16 a
/// death is not a respawn, it is a body lying where it fell waiting for an
/// answer.
fn die(w: &mut World) {
    let before = w.players[0].deaths;
    w.players[0].food = 0;
    w.players[0].water = 0;
    for _ in 0..120 * TICK_HZ {
        w.tick(&[]);
        if w.players[0].deaths > before {
            assert!(
                w.players[0].dead,
                "the clock killed the body and it woke by itself — the death \
                 screen has no one waiting on it"
            );
            return;
        }
    }
    panic!("the clock never killed the body — the survival fixture changed under this test");
}

/// Answer the screen and return the position the body woke on, read on the
/// tick the answer landed so a later step of movement cannot smear it.
fn wake(w: &mut World, on_bag: bool) -> (i32, i32) {
    assert!(
        w.players[0].dead,
        "nothing to answer — the body is standing"
    );
    w.tick(&[Command::Respawn { id: 1, on_bag }]);
    assert!(!w.players[0].dead, "the answer did not wake the body");
    (w.players[0].body.qx, w.players[0].body.qz)
}

/// The common case: die, then ask for a bag. Every test below that used to
/// call `die` and read a position calls this, because "the bag answered"
/// is now two facts — the world offered and the player asked.
fn die_and_wake(w: &mut World, on_bag: bool) -> (i32, i32) {
    die(w);
    wake(w, on_bag)
}

/// Where `Body::at` would put a body standing on this cell — the answer the
/// respawn must produce, computed the way the respawn computes it.
fn body_on(w: &World, cx: u16, cz: u16) -> (i32, i32) {
    let (x, z) = cell_center(cx, cz);
    let b = sim_core::movement::Body::at(w.seed, x, z);
    (b.qx, b.qz)
}

/// The last `EV_RESPAWN` in the tick's ring: (player, woke-on-a-bag).
fn respawn_event(w: &World) -> Option<(u32, bool)> {
    w.events
        .entries()
        .iter()
        .rev()
        .find(|e| e.code == EV_RESPAWN)
        .map(|e| (e.a, e.b == 1))
}

/// The whole item, in one assertion: a player who placed a bag wakes on it
/// instead of on a blind ring point on the far side of the island.
#[test]
fn a_death_wakes_you_on_your_own_bag() {
    let mut w = lone_world();
    let (cx, cz) = buildable_cell_near(SEED, 341, 341, 0);
    place_bag(&mut w, cx, cz);

    // Walk away first: waking *where you happened to die* would pass a
    // weaker version of this test, so the body dies somewhere else.
    let (fx, fz) = buildable_cell_near(SEED, 341, 341, 40);
    stand(&mut w, fx, fz);
    assert_ne!((fx, fz), (cx, cz), "the fixture must move the body");

    let woke = die_and_wake(&mut w, true);
    assert_eq!(
        woke,
        body_on(&w, cx, cz),
        "the body did not wake on its own bag"
    );
    assert_eq!(
        respawn_event(&w),
        Some((1, true)),
        "the respawn did not announce that a bag answered it"
    );
    // And it is still a real respawn in every other way the module owes.
    assert_eq!(w.players[0].deaths, 1, "the death still counts");
    assert_eq!(w.players[0].hp, w.combat.player_hp, "full hp");
    assert!(
        w.players[0].food > 0 && w.players[0].water > 0,
        "a body that wakes on a bag is fed exactly like one that wakes on the ring"
    );
}

/// With no bag placed, nothing about the spawn ring moved. This is the
/// assertion that fails if the bag path is wired in as an unconditional
/// replacement rather than as an answer with a fallback.
#[test]
fn no_bag_is_still_the_spawn_ring() {
    let mut w = lone_world();
    let woke = die_and_wake(&mut w, true);
    let (x, z) = w.spawn_pos_n(1, 1);
    let b = sim_core::movement::Body::at(SEED, x, z);
    assert_eq!(
        woke,
        (b.qx, b.qz),
        "a bagless death stopped walking the spawn ring"
    );
    assert_eq!(
        respawn_event(&w),
        Some((1, false)),
        "the respawn claimed a bag it did not have"
    );
}

/// The cooldown, from the world's side: killed twice inside five minutes
/// with one bag, the second death goes back to the ring — and the ring it
/// goes to is the generation the *death count* names, not a generation the
/// bag path skipped.
#[test]
fn a_second_death_inside_the_cooldown_falls_back_to_the_ring() {
    let mut w = lone_world();
    let (cx, cz) = buildable_cell_near(SEED, 341, 341, 0);
    place_bag(&mut w, cx, cz);

    assert_eq!(
        die_and_wake(&mut w, true),
        body_on(&w, cx, cz),
        "first death: the bag"
    );
    let after_first = w.tick;

    let woke = die_and_wake(&mut w, true);
    assert!(
        w.tick - after_first < BAG_COOLDOWN_TICKS,
        "the second death landed after the cooldown had already lapsed — \
         this test proves nothing at that speed"
    );
    let (x, z) = w.spawn_pos_n(1, 2);
    let b = sim_core::movement::Body::at(SEED, x, z);
    assert_eq!(
        woke,
        (b.qx, b.qz),
        "a bag on cooldown answered a second death inside five minutes"
    );
    assert_eq!(respawn_event(&w), Some((1, false)));
}

/// …and a second bag is a second answer. This is what `BAG_CAP` is a cap
/// *on*: how many deaths in a row a defender can keep answering.
#[test]
fn a_second_bag_answers_the_second_death() {
    let mut w = lone_world();
    let (cx, cz) = buildable_cell_near(SEED, 341, 341, 0);
    let (bx, bz) = buildable_cell_near(SEED, 341, 341, 1);
    assert_ne!((cx, cz), (bx, bz));
    place_bag(&mut w, cx, cz);
    place_bag(&mut w, bx, bz);

    // Die on the first bag's cell, so the first bag is the nearer one and
    // the second death has to reach past a spent bag to find the other.
    stand(&mut w, cx, cz);
    assert_eq!(
        die_and_wake(&mut w, true),
        body_on(&w, cx, cz),
        "first death: the near bag"
    );
    assert_eq!(
        die_and_wake(&mut w, true),
        body_on(&w, bx, bz),
        "the second death did not fall through to the owner's other bag"
    );
    assert_eq!(respawn_event(&w), Some((1, true)));
}

/// The cooldown ends. A bag is a permanent answer that rations itself, not
/// a consumable — the same bag takes the next death once its five minutes
/// are up.
#[test]
fn the_bag_answers_again_once_its_cooldown_lapses() {
    let mut w = lone_world();
    let (cx, cz) = buildable_cell_near(SEED, 341, 341, 0);
    place_bag(&mut w, cx, cz);
    assert_eq!(
        die_and_wake(&mut w, true),
        body_on(&w, cx, cz),
        "first death: the bag"
    );

    // Leap the clock past the cooldown, exactly as the deploy probe leaps
    // it past an upkeep period: every timer here is tick-driven, so the
    // leap is the same arithmetic five real minutes would have done.
    w.tick += BAG_COOLDOWN_TICKS;
    assert_eq!(
        die_and_wake(&mut w, true),
        body_on(&w, cx, cz),
        "the bag never woke up — a cooldown that does not lapse is a consumable"
    );
    assert_eq!(respawn_event(&w), Some((1, true)));
}

/// A bag is state, so two worlds that disagree about which bags are spent
/// must not agree about their hashes (wall 5). Without the cooldown in the
/// digest, a WAL replayed from mid-cooldown would put the next death on a
/// bag the first run had already spent.
#[test]
fn the_cooldown_is_hashed_state() {
    let mut w = lone_world();
    let (cx, cz) = buildable_cell_near(SEED, 341, 341, 0);
    place_bag(&mut w, cx, cz);

    // Spend the bag through the store's own verb and nothing else — no
    // death, no movement, no tick — so the cooldown is the only byte of
    // this world that changed and the hash has nothing else to move on.
    let (x, z) = cell_center(cx, cz);
    let before = w.state_hash();
    assert_eq!(
        w.deploys.claim_bag(&w.deploy, 1, x, z, w.tick),
        Some((x, z))
    );
    assert!(w.deploys.bag_ready()[0] > 0, "the bag was not stamped");
    assert_ne!(
        w.state_hash(),
        before,
        "the bag cooldown is not inside state_hash — a replay resuming \
         mid-cooldown would wake a body on a bag the first run had spent, \
         and the two runs would still call themselves the same world"
    );
}

// ---------------------------------------------------------------------------
// The death screen and the choice (wire v16, ALPHA.md §1's respawn flow)
// ---------------------------------------------------------------------------

/// The state the whole slice exists for: a death is a body lying where it
/// fell, not a body somewhere else. Before v16 the sim picked the anchor
/// and the player was already standing on it by the time the client heard
/// anything, so there was nothing a death screen could have been about.
#[test]
fn a_death_leaves_the_body_where_it_fell_and_waiting() {
    let mut w = lone_world();
    let (cx, cz) = buildable_cell_near(SEED, 341, 341, 0);
    place_bag(&mut w, cx, cz);
    let (fx, fz) = buildable_cell_near(SEED, 341, 341, 40);
    stand(&mut w, fx, fz);
    let fell = (w.players[0].body.qx, w.players[0].body.qz);

    die(&mut w);
    assert!(w.players[0].dead, "the screen is not up");
    assert_eq!(w.players[0].hp, 0, "a corpse is at zero");
    assert_eq!(
        (w.players[0].body.qx, w.players[0].body.qz),
        fell,
        "the body moved without anyone answering the screen"
    );
    assert!(
        respawn_event(&w).is_none(),
        "the death announced a respawn — nothing has answered yet"
    );

    // …and it stays there. A minute of ticks with a ready bag two cells
    // away and nothing wakes it: there is no timer here, by design.
    for _ in 0..60 * TICK_HZ {
        w.tick(&[]);
        assert!(w.players[0].dead, "something released the death screen");
    }
    assert_eq!(
        (w.players[0].body.qx, w.players[0].body.qz),
        fell,
        "the corpse drifted"
    );
}

/// The choice, in one assertion: the same death, the same ready bag, and
/// the beach button puts you on the ring instead. This is the half v2 could
/// not express — it always took the bag.
#[test]
fn the_beach_button_refuses_a_ready_bag() {
    let mut w = lone_world();
    let (cx, cz) = buildable_cell_near(SEED, 341, 341, 0);
    place_bag(&mut w, cx, cz);
    stand(&mut w, cx, cz);

    die(&mut w);
    let woke = wake(&mut w, false);
    let (x, z) = w.spawn_pos_n(1, 1);
    let b = sim_core::movement::Body::at(SEED, x, z);
    assert_eq!(
        woke,
        (b.qx, b.qz),
        "the beach button woke the body on its bag anyway"
    );
    assert_eq!(
        respawn_event(&w),
        Some((1, false)),
        "the respawn claimed a bag the player refused"
    );
}

/// …and refusing it does not *spend* it. A choice that cost the same either
/// way would not be a choice: walk away from one fight and the bag is still
/// there for the next death.
#[test]
fn a_refused_bag_is_not_a_spent_bag() {
    let mut w = lone_world();
    let (cx, cz) = buildable_cell_near(SEED, 341, 341, 0);
    place_bag(&mut w, cx, cz);
    stand(&mut w, cx, cz);

    die(&mut w);
    let hash_before = w.state_hash();
    wake(&mut w, false);
    assert_eq!(
        w.deploys.bag_ready()[0],
        0,
        "the beach answer stamped the bag's cooldown"
    );
    assert_ne!(
        hash_before,
        w.state_hash(),
        "the wake moved nothing at all — this fixture is not proving anything"
    );

    // The proof it is still live: the very next death takes it.
    assert_eq!(
        die_and_wake(&mut w, true),
        body_on(&w, cx, cz),
        "the refused bag would not answer the next death — it was spent after all"
    );
}

/// Asking for a bag you do not have is a beach, not a refusal. A player
/// stuck behind a screen their button cannot dismiss has left the game,
/// which is why this is the one place the sim silently gives you the other
/// answer instead of announcing that it will not.
#[test]
fn asking_for_a_bag_you_have_not_got_is_a_beach() {
    let mut w = lone_world();
    die(&mut w);
    let woke = wake(&mut w, true);
    let (x, z) = w.spawn_pos_n(1, 1);
    let b = sim_core::movement::Body::at(SEED, x, z);
    assert_eq!(woke, (b.qx, b.qz), "an unanswerable ask stranded the body");
    assert_eq!(
        respawn_event(&w),
        Some((1, false)),
        "the event must say a beach answered, so the client can say so too"
    );
}

/// A corpse does not play. Every verb resolves through `live_slot_of`, and
/// the one that proves it end-to-end is the one a dead body could otherwise
/// use to un-kill itself: eating.
#[test]
fn a_corpse_cannot_act() {
    let mut w = lone_world();
    w.players[0].inv[0] = ItemStack {
        item: BAG_ITEM,
        count: 4,
    };
    die(&mut w);
    // The backpack took the inventory with it; put something back by hand
    // so the refusal below is the death screen's and not an empty slot's.
    w.players[0].inv[0] = ItemStack {
        item: BAG_ITEM,
        count: 4,
    };
    let hash_before = w.state_hash();

    w.tick(&[
        Command::Consume { id: 1, slot: 0 },
        Command::Drink { id: 1 },
        Command::Loot { id: 1 },
        Command::PlaceDeploy {
            id: 1,
            row: BAG_ROW,
            cx: 341,
            cz: 341,
            level: 0,
            loc: LOC_PLANE,
        },
    ]);
    assert!(w.players[0].dead, "a verb woke the corpse");
    assert_eq!(w.players[0].hp, 0, "a corpse healed itself");
    assert_eq!(
        w.players[0].inv[0].count, 4,
        "the corpse ate — a dead body spent an item"
    );
    // The tick counter is the only thing that moved, and it is not state
    // this compares: re-hash at the same tick by construction.
    w.tick -= 1;
    assert_eq!(
        hash_before,
        w.state_hash(),
        "four verbs from a dead body changed the world"
    );
}

/// A respawn from a body that is *standing* does nothing. The wire cannot
/// forge a bag id (the action carries one bit), so this is the whole of the
/// verb's forgeable surface: pressing it twice, or at all, while alive.
#[test]
fn a_respawn_from_a_live_body_does_nothing() {
    let mut w = lone_world();
    let (cx, cz) = buildable_cell_near(SEED, 341, 341, 0);
    place_bag(&mut w, cx, cz);
    stand(&mut w, cx, cz);
    let stood = (w.players[0].body.qx, w.players[0].body.qz);

    w.tick(&[
        Command::Respawn {
            id: 1,
            on_bag: true,
        },
        Command::Respawn {
            id: 1,
            on_bag: false,
        },
    ]);
    assert_eq!(
        (w.players[0].body.qx, w.players[0].body.qz),
        stood,
        "a live body was moved by a respawn press"
    );
    assert_eq!(w.players[0].deaths, 0, "a press invented a death");
    assert_eq!(
        w.deploys.bag_ready()[0],
        0,
        "a live body's press spent its own bag"
    );
    assert!(respawn_event(&w).is_none(), "a live press announced a wake");
}

/// The clock and the sea are different sentences, and the record says
/// which. `EV_DRANK` exists for exactly this reason one shelf over: a
/// number with no cause on it is not information.
#[test]
fn the_world_names_which_way_it_killed_you() {
    let mut w = lone_world();
    die(&mut w);
    assert_eq!(w.players[0].death_cause, sim_core::world::DEATH_BY_CLOCK);
    assert_eq!(
        w.players[0].death_by, 1,
        "the world's deaths are self-dealt"
    );
    assert_eq!(
        w.players[0].death_item,
        sim_core::gather::NO_ITEM,
        "the clock holds no weapon"
    );
    assert_eq!(w.players[0].death_range_cm, 0);
}

/// The screen is sim state, so two worlds that disagree about whether a
/// body is standing must not agree about their hashes (wall 5). A replay
/// that reproduced the position and lost the screen would resume with a
/// player who can act and a client that cannot.
#[test]
fn the_death_screen_is_hashed_state() {
    let mut w = lone_world();
    die(&mut w);
    let dead = w.state_hash();
    let tick = w.tick;
    wake(&mut w, false);
    w.tick = tick;
    assert_ne!(
        dead,
        w.state_hash(),
        "the death screen is not inside state_hash"
    );
}

// ---------------------------------------------------------------------------
// The spawn kit across a death (DECISIONS.md 2026-08-15; `NOW.md` §0die
// mechanism 3)
//
// The kit was granted on the fresh-spawn arm of `World::seat` and nowhere
// else, because `inventory::grant_kit`'s own doc said re-granting "would be
// an item printer". That was true of a kit worth 900 wood, 500 stone and 100
// metal frags. It is false of a rock and a torch, and what the old rule
// bought instead was the compound §0die names: your inventory drops into a
// bag where you fell, the bag despawns on its timer, and no kit ever comes
// back — so one death ended a session for good.
//
// These gates are content-blind on purpose. They install a two-entry kit of
// fixture item ids rather than the shipped rock and torch, because what
// `wake` owes is a MECHANISM — "a fresh body gets the floor it needs" — and
// the shipped kit's identity is `crates/server/tests/spawn_kit.rs`'s
// question. A kit-shaped assertion here would redden on every content edit
// and say nothing about the code that changed.

/// A two-entry kit of hand-item stand-ins, in `SpawnKit`'s own shape.
///
/// Ids 8 and 9 are outside every fixture table's live range, so nothing else
/// in this file can pay them into a pocket and make a re-grant look like a
/// gather.
fn probe_kit() -> sim_core::inventory::SpawnKit {
    let mut kit = sim_core::inventory::SpawnKit::EMPTY;
    assert!(kit.set(0, ItemStack { item: 8, count: 1 }), "kit slot 0");
    assert!(kit.set(1, ItemStack { item: 9, count: 1 }), "kit slot 1");
    kit
}

/// **A respawn re-grants the spawn kit.** The §0die fix, and the assertion
/// this whole file exists to carry now: a body that wakes is a NEW body, and
/// a new body is armed exactly as the first one was.
///
/// Proven red by deleting `inventory::grant_kit` from `World::wake`: the
/// death empties the inventory (the bag takes it, or an inert backpack table
/// destroys it, and either way `wake`'s `..Player::default()` zeroes the
/// slots), so without the grant every slot below reads `ItemStack::default()`
/// and the first assertion fails on slot 0.
#[test]
fn a_respawn_re_grants_the_spawn_kit() {
    let mut w = World::new(SEED);
    w.combat = CombatContent::probe_fixture();
    w.survival = SurvivalContent::probe_fixture();
    w.deploy = DeployContent::probe_fixture();
    w.spawn_kit = probe_kit();
    w.tick(&[Command::Join { id: 1 }]);

    // The fresh arm still works — otherwise the test below could pass on a
    // kit that was never granted at all.
    assert_eq!(
        (w.players[0].inv[0], w.players[0].inv[1]),
        (
            ItemStack { item: 8, count: 1 },
            ItemStack { item: 9, count: 1 }
        ),
        "the fresh spawn did not get the kit — nothing below proves anything"
    );

    // Something the player EARNED, in a slot the kit does not write. It must
    // not survive the death: a respawn that kept your pockets would make the
    // assertion below true for the wrong reason.
    w.players[0].inv[6] = ItemStack { item: 3, count: 40 };

    die(&mut w);
    wake(&mut w, false);

    assert_eq!(
        (w.players[0].inv[0], w.players[0].inv[1]),
        (
            ItemStack { item: 8, count: 1 },
            ItemStack { item: 9, count: 1 }
        ),
        "a respawned body woke naked — the kit is still fresh-arm only"
    );
    assert_eq!(
        w.players[0].inv[6],
        ItemStack::default(),
        "the death did not take what the player was carrying, so this test \
         proves nothing about the re-grant"
    );
}

/// **Every** death re-grants, and each one grants exactly the kit. The test
/// above proves the first death; this proves the rule is not "the first one".
///
/// Proven red by granting only on the first death (`if deaths <= 1` around
/// the `wake` call): this fails on iteration 2 reading 0 carried items while
/// the test above stays green — which is the whole reason it is a separate
/// gate. It is red on the missing grant too, on iteration 1.
///
/// ⚠ **What it does NOT catch, measured rather than assumed.** The first
/// draft of this doc claimed it was red under a *merging* `grant_kit`
/// (`inv_add` per stack instead of a slot write) — "two deaths cannot leave
/// four rocks". That was run, and it is false: **no test in this file can
/// tell a merge from a write**, because both live call sites grant into an
/// inventory that was zeroed one line earlier — `seat`'s fresh arm builds
/// the `Player` from scratch and `wake` respreads `..Player::default()`.
/// `grant_kit`'s write-not-merge shape is therefore currently unobservable
/// from here, and what actually keeps the kit from printing is that the
/// grant only ever fires where the pockets were just emptied. THAT is the
/// gate below (`a_live_body_pressing_respawn_is_not_paid`), and it is the
/// one to keep red if a future pass lets a body carry anything through a
/// death. Left written down because a plausible-sounding red proof that was
/// never run is exactly the kind of gate `CLAUDE.md` says is not one.
#[test]
fn every_death_re_grants_the_kit_and_never_more_than_it() {
    let mut w = World::new(SEED);
    w.combat = CombatContent::probe_fixture();
    w.survival = SurvivalContent::probe_fixture();
    w.deploy = DeployContent::probe_fixture();
    w.spawn_kit = probe_kit();
    w.tick(&[Command::Join { id: 1 }]);

    for n in 1..=3u16 {
        die(&mut w);
        wake(&mut w, false);
        assert_eq!(w.players[0].deaths, n, "the death count is the loop's own");
        let carried: u32 = w.players[0]
            .inv
            .iter()
            .filter(|s| s.item == 8 || s.item == 9)
            .map(|s| s.count as u32)
            .sum();
        assert_eq!(
            carried, 2,
            "after {n} deaths the body carries {carried} kit items, not 2 — \
             the re-grant is minting"
        );
    }
}

/// A **living** body pressing respawn gets nothing. The wake path is the
/// only door the kit comes through, and `World` refuses the command outright
/// for a body that is standing (`a_respawn_press_from_a_live_body_is_a_no_op`
/// owns the position half of that; this owns the pocket half).
///
/// Proven red by moving the `grant_kit` call out of `wake` and into the
/// `Command::Respawn` arm ahead of the `dead` check — the shape somebody
/// reaching for "grant it where the command arrives" would write.
#[test]
fn a_live_body_pressing_respawn_is_not_paid() {
    let mut w = World::new(SEED);
    w.combat = CombatContent::probe_fixture();
    w.survival = SurvivalContent::probe_fixture();
    w.deploy = DeployContent::probe_fixture();
    w.spawn_kit = probe_kit();
    w.tick(&[Command::Join { id: 1 }]);
    // Empty the hands the fresh arm filled, so a grant that fires here is
    // visible as a refill rather than hidden under what is already there.
    w.players[0].inv = Default::default();

    w.tick(&[Command::Respawn {
        id: 1,
        on_bag: false,
    }]);
    assert!(!w.players[0].dead, "the fixture body was not standing");
    assert_eq!(
        w.players[0].inv[0],
        ItemStack::default(),
        "a live press paid out a spawn kit — the kit is a free action"
    );
}
