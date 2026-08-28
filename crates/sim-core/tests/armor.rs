//! `test_armor` — the gate for **worn protection**: that a piece of armor
//! actually blunts a hit, that it blunts only the routes it is allowed to,
//! and that a body's plates end up somewhere a killer can pick them up.
//!
//! ## What this suite exists to stop
//!
//! `content/armor.toml` was priced, validated, hashed and balance-anchored
//! from M1 to 2026-08-19 with **nothing in `sim-core` reading a row of it**
//! (`reference/ARMOR.md` §9.1 is the audit). All three pieces are
//! craftable — `content/recipes.toml` — and the road sign jacket is a rung
//! on the research ladder, so a player could spend cloth, scrap and a
//! blueprint on a shirt that protected them from nothing. That is a
//! *charged* dead end, the same class as the revolver a day earlier, and
//! not merely unread data.
//!
//! ## Wearing, and what this paragraph used to say
//!
//! It said **"nothing can put armor on"** — wearing is a container move
//! into a `CONT_WEAR` kind, all four values of the wire's two-bit
//! container field were spent, so equipping cost a `CONT_KIND_BITS`
//! widening and a `PROTO_VER` bump, *a wire slice, not this one*.
//!
//! **That slice landed (wire v51, armor v1)**, and the last section of
//! this file is it: `CONT_WEAR` is kind 4, the field is three bits, and
//! `World::move_item` carries equipping with no new verb. The paragraph
//! is kept in the past tense rather than deleted because the price it
//! quoted was correct and is the reason the feature waited — but read it
//! as history. Reachability is real now.
//!
//! The tests **above** that section still write `Player::worn` directly,
//! and deliberately: they are about what a plate does to a hit, and
//! routing each through a move would make a reduction test fail for a
//! container reason. The tests below are the inverse — they never assert
//! on damage, only on where a stack ended up.
//!
//! ## The numbers
//!
//! All from `CombatContent::probe_fixture`, never invented here: item 0 is
//! a 34-damage melee weapon, item 4 is a 10 % head piece and item 5 a 20 %
//! body piece. The shipped `content/armor.toml` values are gated one crate
//! over, in `crates/content/tests/content.rs`, because that is where the
//! bake and the balance bands live.

use sim_core::combat::{self, ArmorDef, CombatContent, ARMOR_MAX_PCT, WEAR_BODY, WEAR_HEAD};
use sim_core::gather::{GatherContent, ItemStack, SWING_INTERVAL_TICKS};
use sim_core::inventory::{
    CONT_BOX, CONT_MAX, CONT_SELF, CONT_WEAR, REFUSE_M_NO_CONTAINER, REFUSE_M_SLOT, REFUSE_M_WEAR,
};
use sim_core::input::{InputFrame, BTN_PRIMARY};
use sim_core::limits::{INV_SLOTS, MAX_ITEM_DEFS, WEAR_SLOTS};
use sim_core::movement::{Body, POS_XZ_Q};
use sim_core::survival::{self, SurvivalContent};
use sim_core::world::{Command, EventQueue, Player, World, EV_MOVED, EV_MOVE_REFUSED};
use sim_core::yaw_dir;

/// The solved authored sites for `seed` — what `terrain::ground` needs in
/// order to know where the carve is. Memoized per seed for
/// `tests/combat.rs`'s stated reason: `terrain::haven` is a few thousand
/// height taps and these suites call it from inside loops.
fn hv(seed: u64) -> &'static sim_core::terrain::Haven {
    use std::cell::RefCell;
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

const SEED: u64 = 20260802;
/// The fixture's item 0: 34 damage, 2 m reach.
const SPEAR: u16 = 0;
/// The fixture's armor rows.
const HEADWRAP: u16 = 4;
const PLATE: u16 = 5;
const FIXTURE_HP: u16 = 100;

fn one(item: u16) -> ItemStack {
    ItemStack {
        item,
        count: 1,
        cond: 0,
    }
}

/// Two players on one point, armed with fixture item 0 in hotbar slot 0 —
/// `tests/combat.rs`'s `duel_world`, because a swing is the route this
/// slice is really about and inventing a second arrangement for it would
/// be inventing a second set of assumptions about reach and scatter.
fn duel_world() -> World {
    let mut w = World::new(SEED);
    w.gather = GatherContent::probe_fixture();
    w.combat = CombatContent::probe_fixture();
    w.dev_spawn = Some(w.spawn_pos(1));
    w.tick(&[Command::Join { id: 1 }, Command::Join { id: 2 }]);
    for p in w.players.iter_mut().take(2) {
        p.inv[0] = one(SPEAR);
    }
    w
}

fn swing_frame(seq: u16, yaw: u16) -> InputFrame {
    InputFrame {
        seq,
        buttons: BTN_PRIMARY,
        yaw,
        pitch: 128,
        move_x: 0,
        move_z: 0,
        sel: 0,
    }
}

/// Put player 1 two metres in front of player 0 and return the yaw player
/// 0 must face.
fn face_off(w: &mut World) -> u16 {
    let yaw = 0u16;
    let (fx, fz) = yaw_dir(yaw);
    let a = w.players[0].body;
    let (ax, az) = (a.qx as f32 * POS_XZ_Q, a.qz as f32 * POS_XZ_Q);
    w.players[1].body = Body::at(SEED, hv(SEED), ax + fx * 1.5, az + fz * 1.5);
    yaw
}

/// Swings until player 1 falls, and answers how many landed. Bounded so a
/// swing that stops connecting fails as a count rather than as a hang.
fn swings_to_kill(w: &mut World, yaw: u16) -> u32 {
    let mut hits = 0u32;
    for seq in 1..=40u16 {
        w.tick(&[Command::Input {
            id: 1,
            frame: swing_frame(seq, yaw),
        }]);
        hits += 1;
        if w.players[1].hp == 0 {
            return hits;
        }
        for _ in 0..SWING_INTERVAL_TICKS {
            w.tick(&[]);
        }
    }
    panic!("40 swings did not fell a body with {FIXTURE_HP} hp — the swing stopped connecting");
}

// ---------------------------------------------------------------------
// 1 · The headline: armor changes hits-to-kill.
// ---------------------------------------------------------------------

/// **A body wearing a plate takes more hits to kill, through the real
/// swing verb.**
///
/// The arithmetic, so the count below is derived rather than observed:
/// 34 damage against a 20 % body plate is `34 × 80 / 100 = 27`, and
/// `ceil(100 / 27)` is 4 where `ceil(100 / 34)` is 3. Both numbers are
/// asserted, because a test that only checked "more than before" would
/// pass on a plate that made a body immortal.
#[test]
fn a_worn_plate_costs_the_attacker_a_swing() {
    let naked = {
        let mut w = duel_world();
        let yaw = face_off(&mut w);
        swings_to_kill(&mut w, yaw)
    };
    assert_eq!(naked, 3, "the fixture's 34 damage fells 100 hp in three");

    let mut w = duel_world();
    let yaw = face_off(&mut w);
    w.players[1].worn[1] = one(PLATE);
    let armored = swings_to_kill(&mut w, yaw);
    assert_eq!(
        armored, 4,
        "a 20 % body plate turns 34 damage into 27, which is four swings \
         and not three — armor is being read but not applied as the data \
         says, or not read at all"
    );
    assert!(
        armored > naked,
        "the plate cost the attacker nothing: `content/armor.toml` is \
         priced, craftable and researchable, so a piece that changes no \
         fight is a charged dead end"
    );
}

/// Each hit is reduced, not just the last one — the per-hit arithmetic,
/// read off the body rather than inferred from a count.
#[test]
fn every_hit_arrives_reduced_not_only_the_killing_one() {
    let mut w = duel_world();
    let yaw = face_off(&mut w);
    w.players[1].worn[1] = one(PLATE);
    w.tick(&[Command::Input {
        id: 1,
        frame: swing_frame(1, yaw),
    }]);
    assert_eq!(
        w.players[1].hp,
        FIXTURE_HP - 27,
        "the first swing took the unreduced 34 (or something that is \
         neither 34 nor 27)"
    );
}

// ---------------------------------------------------------------------
// 2 · The set, and the slots.
// ---------------------------------------------------------------------

/// **The worn set is one number and both slots contribute.** A head piece
/// pays against a body hit, because the sim has no hit areas at all
/// (`combat.rs`'s header: aim is planar, there is no head to hit) — and
/// the alternative would ship `item.armor_burlap_head` as craftable,
/// priced and protecting nobody on the very day armor started working.
#[test]
fn the_set_sums_both_slots() {
    let cc = CombatContent::probe_fixture();
    let mut p = Player::default();
    assert_eq!(combat::worn_pct(&cc, &p), 0, "a naked body carries no set");

    p.worn[0] = one(HEADWRAP);
    assert_eq!(combat::worn_pct(&cc, &p), 10);
    p.worn[1] = one(PLATE);
    assert_eq!(
        combat::worn_pct(&cc, &p),
        30,
        "10 % + 20 % is 30 % — the set is the sum, and a head piece that \
         paid nothing would be dead content the day armor landed"
    );
    assert_eq!(
        combat::reduce(34, combat::worn_pct(&cc, &p)),
        23,
        "34 × 70 / 100"
    );
}

/// **A piece only pays in the slot its baked row names.** `Player::worn`
/// is indexed by slot, and the check is against the *baked* row rather
/// than against the placement — which is what stops two body plates from
/// stacking before any verb exists to refuse the second one.
#[test]
fn a_piece_in_the_wrong_slot_protects_nobody() {
    let cc = CombatContent::probe_fixture();
    let mut p = Player::default();
    // The body plate, put in the head slot.
    p.worn[0] = one(PLATE);
    assert_eq!(
        combat::worn_pct(&cc, &p),
        0,
        "a body plate worn on the head protected somebody — the slot check \
         reads the placement instead of the baked row, so two plates would \
         stack the moment a wear verb exists"
    );
    // And the same stack in its own slot does pay, so the zero above is
    // the slot rule rather than a table that reads nothing.
    p.worn[0] = ItemStack::default();
    p.worn[1] = one(PLATE);
    assert_eq!(combat::worn_pct(&cc, &p), 20);
}

/// An empty stack and an item off the table are both nothing, and neither
/// panics. `worn` is written by a save, which is the one non-command path
/// into `World` (`CLAUDE.md`'s save trap), so an out-of-range index has to
/// be a no-op rather than an index.
#[test]
fn a_forged_worn_stack_is_a_no_op() {
    let cc = CombatContent::probe_fixture();
    let mut p = Player::default();
    p.worn[1] = ItemStack {
        item: MAX_ITEM_DEFS as u16 + 7,
        count: 1,
        cond: 0,
    };
    assert_eq!(combat::worn_pct(&cc, &p), 0);
    p.worn[1] = ItemStack {
        item: PLATE,
        count: 0,
        cond: 0,
    };
    assert_eq!(
        combat::worn_pct(&cc, &p),
        0,
        "a zero-count stack is an empty slot everywhere else in the crate"
    );
}

/// The set's ceiling. A wear roster wide enough to sum past
/// `ARMOR_MAX_PCT` is refused at the sum, not at the row — the per-row 90
/// `content/validate.rs` enforces cannot see a set.
#[test]
fn the_set_is_clamped_at_the_ceiling() {
    let mut cc = CombatContent::probe_fixture();
    cc.armor[HEADWRAP as usize] = ArmorDef {
        reduction_pct: 80,
        slot: WEAR_HEAD,
    };
    cc.armor[PLATE as usize] = ArmorDef {
        reduction_pct: 80,
        slot: WEAR_BODY,
    };
    let mut p = Player::default();
    p.worn[0] = one(HEADWRAP);
    p.worn[1] = one(PLATE);
    assert_eq!(
        combat::worn_pct(&cc, &p),
        ARMOR_MAX_PCT,
        "160 % of protection was not clamped — a body that takes no damage \
         is not a body"
    );
    assert_eq!(
        combat::reduce(100, 160),
        10,
        "the clamp is inside `reduce` too"
    );
}

// ---------------------------------------------------------------------
// 3 · The three routes armor must NEVER blunt.
// ---------------------------------------------------------------------

/// A body wearing everything the fixture prices.
fn fully_armored() -> Player {
    let mut p = Player::default();
    p.worn[0] = one(HEADWRAP);
    p.worn[1] = one(PLATE);
    p
}

/// **Starving and dehydrating are metabolic, not hits.** A chest plate
/// does not feed you, and farming in armor must not cost a repair bill
/// once condition lands.
///
/// Driven through `survival::step` itself rather than through the funnel,
/// so this is the route and not merely the door: the two bodies differ
/// only in what they are wearing.
#[test]
fn starvation_is_not_blunted_by_armor() {
    let sc = SurvivalContent::probe_fixture();
    let mut ev = EventQueue::default();

    let run = |p: &mut Player, ev: &mut EventQueue| {
        p.hp = 100;
        p.hp_max = 100;
        p.food = 0;
        p.water = 0;
        for _ in 0..90 {
            survival::step(&sc, p, ev);
        }
        p.hp
    };

    let mut naked = Player::default();
    let bare = run(&mut naked, &mut ev);
    let mut armored = fully_armored();
    let plated = run(&mut armored, &mut ev);

    assert!(
        bare < 100,
        "the fixture's clock did not bite, so this proves nothing"
    );
    assert_eq!(
        plated, bare,
        "armor blunted the survival clock — `survival::step` reached \
         `combat::hurt` instead of `hurt_unreduced`, and a helmet is now a \
         meal"
    );
}

/// **The salt water's hp IS the price of the drink.** Reducing it would
/// make a helmet a desalinator.
#[test]
fn the_salt_cost_is_not_blunted_by_armor() {
    let sc = SurvivalContent::probe_fixture();
    // Not driven through `survival::drink`, and the reason is that it
    // would prove less: the verb refuses when there is no sea in reach,
    // so both bodies would pay zero and the comparison would pass on a
    // refusal — a green nothing earned, which `CLAUDE.md` calls the worst
    // bug class. So this is the exact call `drink` makes, with armor on,
    // and the *binding* of the route to that door is
    // `tests/damage_routes.rs`'s job: it already fails loudly if
    // `survival.rs` ever reaches for the reduced one.
    let mut naked = Player {
        hp: 100,
        ..Default::default()
    };
    let mut armored = fully_armored();
    armored.hp = 100;
    let bare = combat::hurt_unreduced(&mut naked, sc.drink_hp_cost).dealt;
    let plated = combat::hurt_unreduced(&mut armored, sc.drink_hp_cost).dealt;
    assert_eq!(bare, sc.drink_hp_cost, "the fixture's drink costs nothing");
    assert_eq!(plated, bare, "armor paid part of a mouthful of sea water");
}

/// **The keypad shock floors at 1 hp and never kills** (`lock.rs`'s
/// header). Reducing it would make an armored raider immune to a mechanic
/// whose entire job is to cost tries.
///
/// The two lines `deploy::lock_op` runs, in order, with armor on. The
/// route's binding to the unreduced door is `damage_routes.rs`'s, which
/// already fails as *"deploy.rs declares an armor-exempt route … and calls
/// `combat::hurt_unreduced` nowhere"*.
#[test]
fn the_keypad_shock_is_not_blunted_by_armor() {
    let mut naked = Player {
        hp: 100,
        ..Default::default()
    };
    let mut armored = fully_armored();
    armored.hp = 100;
    const SHOCK: u16 = 25;
    let bare_amount = sim_core::lock::shock_amount(naked.hp, SHOCK);
    let bare = combat::hurt_unreduced(&mut naked, bare_amount);
    let plated_amount = sim_core::lock::shock_amount(armored.hp, SHOCK);
    let plated = combat::hurt_unreduced(&mut armored, plated_amount);
    assert_eq!(bare.dealt, SHOCK);
    assert_eq!(
        plated.dealt, bare.dealt,
        "armor absorbed part of a keypad shock — the ladder that costs a \
         raider tries now costs an armored one fewer"
    );

    // And the floor still belongs to the door, not to the funnel.
    let mut low = fully_armored();
    low.hp = 3;
    let low_amount = sim_core::lock::shock_amount(low.hp, SHOCK);
    let took = combat::hurt_unreduced(&mut low, low_amount);
    assert_eq!(took.dealt, 2, "the shock stopped at 1 hp");
    assert!(!took.died, "the keypad killed somebody");
}

// ---------------------------------------------------------------------
// 4 · Death: a corpse drops its plates.
// ---------------------------------------------------------------------

/// **What a body was wearing goes into the bag with what it was
/// carrying.** Armor as loot is what makes killing an armored player worth
/// doing; a corpse that keeps its plates is a body nobody fights for.
///
/// The ledger in `tests/persist.rs` forces this to be *decided*
/// (`worn` is on the re-derived list, so `..Player::default()` clears it);
/// this is the half that says where it went.
#[test]
fn a_corpse_drops_what_it_was_wearing() {
    let mut w = duel_world();
    w.backpack = sim_core::backpack::BackpackContent::probe_fixture();
    let yaw = face_off(&mut w);
    w.players[1].worn[0] = one(HEADWRAP);
    w.players[1].worn[1] = one(PLATE);
    // Nothing in the pockets, so anything in the bag came off the body's
    // slots rather than out of its inventory.
    w.players[1].inv = [ItemStack::default(); INV_SLOTS];
    swings_to_kill(&mut w, yaw);

    assert_eq!(w.players[1].hp, 0, "the body did not fall");
    assert_eq!(
        w.players[1].worn,
        [ItemStack::default(); WEAR_SLOTS],
        "the corpse kept its plates — a killer paid two extra swings for \
         nothing"
    );
    let found: Vec<u16> = w
        .backpacks
        .entries()
        .iter()
        .flat_map(|b| b.items.iter())
        .filter(|s| s.count > 0)
        .map(|s| s.item)
        .collect();
    assert!(
        found.contains(&HEADWRAP) && found.contains(&PLATE),
        "the bag holds {found:?} — both worn pieces must be lootable where \
         the body fell"
    );
}

/// The pack being full costs the killer a walk and never an item — the
/// reason `die` sheds through `drain_spill` rather than by merging into
/// the array `drop_for` copies wholesale (a bag holds `INV_SLOTS`; a full
/// pocket plus two plates is `INV_SLOTS + WEAR_SLOTS`).
#[test]
fn a_full_pack_does_not_destroy_the_plates() {
    let mut w = duel_world();
    w.backpack = sim_core::backpack::BackpackContent::probe_fixture();
    let yaw = face_off(&mut w);
    // Every pocket full of a distinct non-armor item, so nothing merges.
    for (i, s) in w.players[1].inv.iter_mut().enumerate() {
        *s = ItemStack {
            item: 10 + i as u16,
            count: 1,
            cond: 0,
        };
    }
    w.players[1].worn[0] = one(HEADWRAP);
    w.players[1].worn[1] = one(PLATE);
    swings_to_kill(&mut w, yaw);

    let found: Vec<u16> = w
        .backpacks
        .entries()
        .iter()
        .flat_map(|b| b.items.iter())
        .filter(|s| s.count > 0)
        .map(|s| s.item)
        .collect();
    assert_eq!(
        found.iter().filter(|&&i| i == HEADWRAP).count(),
        1,
        "the headwrap was destroyed by a full pack (or duplicated), and \
         the bags hold {found:?}"
    );
    assert_eq!(found.iter().filter(|&&i| i == PLATE).count(), 1);
    assert_eq!(
        found.len(),
        INV_SLOTS + WEAR_SLOTS,
        "the death dropped {} stacks and the body had {}",
        found.len(),
        INV_SLOTS + WEAR_SLOTS
    );
}

// ---------------------------------------------------------------------
// 5 · The save.
// ---------------------------------------------------------------------

/// You log off in your armor, you log in in your armor — `hp`'s own
/// sentence. Through the *bytes*, because the record is the format:
/// `PLAYER_SAVE_BYTES` moved 256 → 268 and `store.rs`'s `SAVE_FORMAT` went
/// with it in the same commit.
#[test]
fn a_body_saves_and_reloads_what_it_is_wearing() {
    use sim_core::persist::{PlayerSave, PLAYER_SAVE_BYTES};
    let mut p = Player {
        hp: 60,
        hp_max: 100,
        ..Default::default()
    };
    p.worn[0] = ItemStack {
        item: HEADWRAP,
        count: 1,
        cond: 0x1234,
    };
    p.worn[1] = one(PLATE);
    let save = PlayerSave::of(&p);
    assert_eq!(save.worn, p.worn, "`PlayerSave::of` dropped the plates");

    let mut buf = [0u8; PLAYER_SAVE_BYTES];
    save.write_le(&mut buf);
    let back = PlayerSave::read_le(&buf).expect("a legal record decodes");
    assert_eq!(
        back.worn, p.worn,
        "the record lost or transposed the worn slots on the way through \
         the file"
    );

    // And through the seat: a restored body stands up in its armor.
    let mut w = World::new(SEED);
    w.combat = CombatContent::probe_fixture();
    w.tick(&[Command::JoinAs { id: 7, save: back }]);
    let seated = w
        .players
        .iter()
        .find(|p| p.active && p.id == 7)
        .expect("the body seated");
    assert_eq!(
        seated.worn, p.worn,
        "a returning player woke up naked — closing the game is now a way \
         to lose a plate"
    );
    assert_eq!(
        combat::worn_pct(&w.combat, seated),
        30,
        "the restored set protects nobody"
    );
}

/// A record naming a piece in the wrong slot decodes — `persist` cannot
/// see `CombatContent` and must not learn to — and then protects nobody,
/// which is the read that matters.
#[test]
fn a_forged_record_buys_no_protection() {
    use sim_core::persist::{PlayerSave, PLAYER_SAVE_BYTES};
    let mut p = Player {
        hp: 100,
        hp_max: 100,
        ..Default::default()
    };
    // Two body plates, one of them in the head slot.
    p.worn[0] = one(PLATE);
    p.worn[1] = one(PLATE);
    let mut buf = [0u8; PLAYER_SAVE_BYTES];
    PlayerSave::of(&p).write_le(&mut buf);
    let back = PlayerSave::read_le(&buf).expect("every stack in it is legal");
    let body = Player {
        worn: back.worn,
        ..Default::default()
    };
    assert_eq!(
        combat::worn_pct(&CombatContent::probe_fixture(), &body),
        20,
        "two plates stacked to 40 % through the save path — the slot check \
         is the only thing standing between a hand-edited record and an \
         invulnerable body"
    );
}


// ---------------------------------------------------------------------
// Wearing it: `CONT_WEAR`, the move verb (wire v51).
//
// One verb, so these are container tests rather than combat tests: what
// they assert is which array a stack is in afterwards, and — for every
// refusal — that the world did not move.
// ---------------------------------------------------------------------

/// The move, encoded the way the wire encodes it. `cont` is zero because
/// a body has no handle (`inventory::is_own`).
fn wear_move(id: u32, from_kind: u8, from_slot: u8, to_kind: u8, to_slot: u8) -> Command {
    Command::Move {
        id,
        cont: 0,
        from_kind,
        from_slot,
        to_kind,
        to_slot,
        count: 1,
    }
}

/// The reason a move was refused this tick — and the assertion that it
/// was answered **exactly once**, which `box_container.rs` makes for the
/// same reason: a verb that both refuses and acknowledges has written
/// something, and a client that receives both picks one.
fn refusal(w: &World) -> Option<u32> {
    let answers: Vec<(u8, u32)> = w
        .events
        .entries()
        .iter()
        .filter(|e| e.code == EV_MOVED || e.code == EV_MOVE_REFUSED)
        .map(|e| (e.code, e.b))
        .collect();
    assert_eq!(answers.len(), 1, "a move must answer exactly once: {answers:?}");
    match answers[0] {
        (EV_MOVE_REFUSED, reason) => Some(reason),
        _ => None,
    }
}

/// A world with one player holding a headwrap and a plate in the pack.
fn dressing_room() -> World {
    let mut w = duel_world();
    let s = w.live_slot_of(1).expect("player 1 is in the world");
    w.players[s].inv[1] = one(HEADWRAP);
    w.players[s].inv[2] = one(PLATE);
    w
}

/// **Putting armor on is a move, and it is the whole feature.**
///
/// The headline claim of the slice: a stack leaves `inv` and arrives in
/// `worn`, through `Command::Move` and nothing else. There is no
/// `Command::Equip` for this test to have called.
///
/// It asserts the *pair* — gone from the pack AND present on the body —
/// because either half alone passes under a duplication bug, and a move
/// verb that duplicates is the one failure this container family has
/// historically shipped.
#[test]
fn a_move_into_the_wear_container_puts_armor_on() {
    let mut w = dressing_room();
    let s = w.live_slot_of(1).unwrap();

    w.tick(&[wear_move(1, CONT_SELF, 1, CONT_WEAR, WEAR_HEAD - 1)]);

    assert_eq!(
        w.players[s].worn[(WEAR_HEAD - 1) as usize],
        one(HEADWRAP),
        "the headwrap must be on the head"
    );
    assert_eq!(
        w.players[s].inv[1],
        ItemStack::default(),
        "and must have left the pack — a move that copies is the bug"
    );
    // And it is now doing its job, which is the only reason any of this
    // exists. Asserted through the same reader combat uses rather than
    // by re-deriving the sum.
    assert_eq!(
        combat::worn_pct(&w.combat, &w.players[s]),
        10,
        "a worn headwrap must be worth its row"
    );
}

/// Taking it off is the same verb backwards, and the round trip is exact.
#[test]
fn the_same_verb_takes_it_off_again() {
    let mut w = dressing_room();
    let s = w.live_slot_of(1).unwrap();

    w.tick(&[wear_move(1, CONT_SELF, 2, CONT_WEAR, WEAR_BODY - 1)]);
    assert_eq!(w.players[s].worn[(WEAR_BODY - 1) as usize], one(PLATE));

    w.tick(&[wear_move(1, CONT_WEAR, WEAR_BODY - 1, CONT_SELF, 7)]);
    assert_eq!(w.players[s].inv[7], one(PLATE), "the plate comes back");
    assert_eq!(
        w.players[s].worn[(WEAR_BODY - 1) as usize],
        ItemStack::default(),
        "and the slot is empty, not still holding a copy"
    );
    assert_eq!(
        combat::worn_pct(&w.combat, &w.players[s]),
        0,
        "an undressed body is unprotected"
    );
}

/// **A helmet does not go in the body slot** — `CanWearItem`, and the one
/// rule the wear container has.
///
/// Refused with `REFUSE_M_WEAR` and not `REFUSE_M_SLOT`: the address the
/// client named is real and its picture of both containers was correct,
/// so telling it to resync would be answering the wrong question.
#[test]
fn a_headwrap_is_refused_by_the_body_slot() {
    let mut w = dressing_room();
    let s = w.live_slot_of(1).unwrap();

    w.tick(&[wear_move(1, CONT_SELF, 1, CONT_WEAR, WEAR_BODY - 1)]);

    assert_eq!(refusal(&w), Some(REFUSE_M_WEAR));
    assert_eq!(w.players[s].inv[1], one(HEADWRAP), "the pack is untouched");
    assert_eq!(
        w.players[s].worn,
        [ItemStack::default(); WEAR_SLOTS],
        "and nothing was worn"
    );
}

/// A spear is not armor, and the same reason covers it.
///
/// This is why there is one `REFUSE_M_WEAR` rather than three: to the
/// player "that is not armor" and "that is not *this* armor" are one
/// fact, and `ArmorDef::slot`'s one-based `WEAR_NONE` makes them one
/// expression too — `slot == s + 1` is false for zero at every `s`.
#[test]
fn a_spear_cannot_be_worn_on_the_head() {
    let mut w = dressing_room();
    let s = w.live_slot_of(1).unwrap();

    w.tick(&[wear_move(1, CONT_SELF, 0, CONT_WEAR, WEAR_HEAD - 1)]);

    assert_eq!(refusal(&w), Some(REFUSE_M_WEAR));
    assert_eq!(w.players[s].inv[0], one(SPEAR), "the spear stays put");
    assert_eq!(w.players[s].worn[(WEAR_HEAD - 1) as usize], ItemStack::default());
}

/// **The swap travels backwards, and checking only the forward half
/// admits exactly what the forward half refuses.**
///
/// A move onto an occupied slot is a SWAP: `plan_move` sends `src`
/// forward and `dst` back. So with a plate worn on the body and a
/// headwrap worn on the head, dragging the plate onto the head slot is
/// refused going forward (a plate is not headgear) — and a check that
/// stopped there would still have to answer the *reverse* drag, where the
/// headwrap moves to the body slot and the plate is pushed back to the
/// head. Same two stacks, same two slots, one verb, opposite direction.
///
/// Proven red by mutant: dropping the `from_kind == CONT_WEAR` clause of
/// `move_item`'s check leaves this test failing with the headwrap on the
/// body and the plate on the head — a body wearing both pieces in the
/// wrong slots, which `worn_pct` scores as **zero** protection. The
/// player would have equipped their whole set and been naked.
#[test]
fn a_swap_cannot_smuggle_a_piece_into_the_wrong_slot() {
    let mut w = dressing_room();
    let s = w.live_slot_of(1).unwrap();
    w.tick(&[wear_move(1, CONT_SELF, 1, CONT_WEAR, WEAR_HEAD - 1)]);
    w.tick(&[wear_move(1, CONT_SELF, 2, CONT_WEAR, WEAR_BODY - 1)]);
    assert_eq!(
        combat::worn_pct(&w.combat, &w.players[s]),
        30,
        "the set is on before the swap is attempted"
    );

    // The reverse drag: head -> body. Forward, the headwrap is refused by
    // the body slot; backwards, the plate would be pushed to the head.
    w.tick(&[wear_move(1, CONT_WEAR, WEAR_HEAD - 1, CONT_WEAR, WEAR_BODY - 1)]);

    assert_eq!(refusal(&w), Some(REFUSE_M_WEAR));
    assert_eq!(
        w.players[s].worn[(WEAR_HEAD - 1) as usize],
        one(HEADWRAP),
        "the headwrap stayed on the head"
    );
    assert_eq!(
        w.players[s].worn[(WEAR_BODY - 1) as usize],
        one(PLATE),
        "and the plate stayed on the body"
    );
    assert_eq!(
        combat::worn_pct(&w.combat, &w.players[s]),
        30,
        "a refused swap must not undress the player"
    );
}

/// A forged wear slot is refused as an **address**, not as a fit.
///
/// `Player::worn` is two elements and `slots_in(CONT_WEAR)` says so, so
/// slot 2 is not a thing that can be named — which is `REFUSE_M_SLOT`,
/// the resync reason, and is the branch that would otherwise index a
/// 2-element array out of bounds. The reason matters as much as the
/// refusal: answering this with `REFUSE_M_WEAR` would tell a desynced
/// client its picture was fine.
#[test]
fn a_wear_slot_past_the_body_is_not_an_address() {
    let mut w = dressing_room();
    let s = w.live_slot_of(1).unwrap();

    for forged in [WEAR_SLOTS as u8, 7, (INV_SLOTS - 1) as u8] {
        w.tick(&[wear_move(1, CONT_SELF, 1, CONT_WEAR, forged)]);
        assert_eq!(
            refusal(&w),
            Some(REFUSE_M_SLOT),
            "wear slot {forged} must refuse as an address"
        );
    }
    assert_eq!(w.players[s].inv[1], one(HEADWRAP), "and nothing moved");
}

/// A forged **kind** past `CONT_MAX` is refused, which is the property the
/// widening handed back.
///
/// Before v51 the two-bit field had no spare pattern and this guard could
/// not fire; three bits give it 5, 6 and 7 to refuse, and all three are
/// checked for the reason `protocol/src/lib.rs`'s restored decode case
/// gives — an off-by-one in `kind > CONT_MAX` passes exactly one of them.
#[test]
fn a_kind_past_the_live_set_is_refused_again() {
    let mut w = dressing_room();
    let s = w.live_slot_of(1).unwrap();

    for forged in (CONT_MAX + 1)..8 {
        w.tick(&[wear_move(1, CONT_SELF, 1, forged, 0)]);
        assert_eq!(
            refusal(&w),
            Some(REFUSE_M_SLOT),
            "container kind {forged} must refuse"
        );
        assert_eq!(w.players[s].inv[1], one(HEADWRAP));
    }
}

/// Wearing is not a way to reach a box you are standing nowhere near.
///
/// `is_own` moved the "which side is on the ground" test, and the risk of
/// moving it is that a wear kind starts counting as own on the *ground*
/// side too — at which point `CONT_BOX` -> `CONT_WEAR` would resolve the
/// box through a handle nobody checked. The one handle still names one
/// ground container, and the reach check still runs on it: with no box
/// placed, this is `REFUSE_M_NO_CONTAINER` rather than a silent success.
#[test]
fn a_wear_move_does_not_open_a_container_it_did_not_name() {
    let mut w = dressing_room();
    let s = w.live_slot_of(1).unwrap();

    w.tick(&[wear_move(1, CONT_BOX, 0, CONT_WEAR, WEAR_HEAD - 1)]);

    assert_eq!(refusal(&w), Some(REFUSE_M_NO_CONTAINER));
    assert_eq!(
        w.players[s].worn[(WEAR_HEAD - 1) as usize],
        ItemStack::default(),
        "a box that is not there dresses nobody"
    );
}
