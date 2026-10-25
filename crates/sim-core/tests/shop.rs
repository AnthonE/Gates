//! The compound shop (`shop.rs`): a player's vending machine. Its owner sets
//! offers and stocks it; a stranger buys, all or nothing; every refusal names
//! its reason and moves nothing; strangers cannot move its slots or lift it;
//! the offers go with the machine and survive a save.

use sim_core::build::{foundation_terrain_ok, BuildContent, BUILD_CELL_M, LOC_PLANE};
use sim_core::combat::CombatContent;
use sim_core::deploy::{box_key, DeployContent, DeployDef, ARCH_VENDING, PLACE_ANY};
use sim_core::gather::{GatherContent, ItemStack};
use sim_core::inventory::{CONT_BOX, CONT_SELF};
use sim_core::limits::{BOX_SLOTS, INV_SLOTS};
use sim_core::movement::Body;
use sim_core::shop::{
    ShopAct, ShopOffer, REFUSE_SH_ALLOWED, REFUSE_SH_FULL, REFUSE_SH_FUNDS, REFUSE_SH_OFFER,
    REFUSE_SH_REACH, REFUSE_SH_STOCK, REFUSE_SH_TAKINGS, SHOP_OP_BUY,
};
use sim_core::world::{Command, World, EV_DEPLOY_REFUSED, EV_SHOP, EV_SHOP_REFUSED};

const SEED: u64 = 0x0FEE_0FEE;
const OWNER: u32 = 3;
const STRANGER: u32 = 4;
const MACHINE: u16 = 40;
const JUNK: u16 = 41;
const CORN: u16 = 42;
const FILLER: u16 = 43;

fn cell_center(cx: u16, cz: u16) -> (f32, f32) {
    (
        (cx as f32 + 0.5) * BUILD_CELL_M,
        (cz as f32 + 0.5) * BUILD_CELL_M,
    )
}

fn buildable_cell(w: &World) -> (u16, u16) {
    let (hx, hz) = sim_core::find::build_hint(SEED, 512, 512);
    for r in 0..64i32 {
        for dz in -r..=r {
            for dx in -r..=r {
                if dx.abs() != r && dz.abs() != r {
                    continue;
                }
                let (cx, cz) = ((hx + dx) as u16, (hz + dz) as u16);
                if (-1..=1).all(|k| {
                    let (x, z) = cell_center((cx as i32 + k) as u16, cz);
                    foundation_terrain_ok(SEED, &w.haven, x, z)
                }) {
                    return (cx, cz);
                }
            }
        }
    }
    panic!("no flat run of cells near the middle");
}

fn stack(item: u16, count: u16) -> ItemStack {
    ItemStack {
        item,
        count,
        cond: 0,
        skin: 0,
        ..ItemStack::EMPTY
    }
}

fn slot_of(w: &World, id: u32) -> usize {
    w.players
        .iter()
        .position(|p| p.active && p.id == id)
        .unwrap()
}

/// The owner's machine placed with the owner beside it, the stranger a cell
/// the other side; returns the world and the machine's key.
fn shop_world() -> (Box<World>, u32) {
    let mut w = Box::new(World::new(SEED));
    w.gather = GatherContent::probe_fixture();
    w.build = BuildContent::probe_fixture();
    w.deploy = DeployContent::probe_fixture();
    w.combat = CombatContent::probe_fixture();
    w.gather.stack_max[MACHINE as usize] = 1;
    w.gather.stack_max[JUNK as usize] = 1000;
    w.gather.stack_max[CORN as usize] = 100;
    w.gather.stack_max[FILLER as usize] = 1;
    w.gather.item_count = w.gather.item_count.max(FILLER + 1);
    let row = w.deploy.def_count;
    w.deploy.defs[row as usize] = DeployDef {
        arch: ARCH_VENDING,
        placement: PLACE_ANY,
        hp: 500,
        matter: sim_core::deploy::MATTER_METAL,
        item: MACHINE,
        n_costs: 0,
        costs: [(0, 0); 4],
    };
    w.deploy.def_count += 1;
    w.deploy.shop_coin = JUNK;

    let (cx, cz) = buildable_cell(&w);
    w.dev_spawn = Some(cell_center(cx, cz));
    w.tick(&[Command::Join { id: OWNER }, Command::Join { id: STRANGER }]);
    let (o, s) = (slot_of(&w, OWNER), slot_of(&w, STRANGER));
    let (ox, oz) = cell_center(cx - 1, cz);
    let (sx, sz) = cell_center(cx + 1, cz);
    w.players[o].body = Body::at(SEED, &w.haven, ox, oz);
    w.players[s].body = Body::at(SEED, &w.haven, sx, sz);
    w.players[o].inv = [ItemStack::default(); INV_SLOTS];
    w.players[s].inv = [ItemStack::default(); INV_SLOTS];
    w.players[o].inv[3] = stack(MACHINE, 1);
    w.players[o].frame.sel = 3;
    w.tick(&[Command::PlaceDeploy {
        id: OWNER,
        row,
        cx,
        cz,
        level: 0,
        loc: LOC_PLANE,
        pose: sim_core::footprint::Pose::CENTRE,
    }]);
    let key = box_key(cx, cz, 0, LOC_PLANE);
    assert!(
        w.deploys.vending_index(key).is_some(),
        "a vending machine is a container: placing one stands a box up"
    );
    (w, key)
}

fn machine(w: &World, key: u32) -> [ItemStack; BOX_SLOTS] {
    w.deploys.boxes()[w.deploys.vending_index(key).unwrap()].items
}

fn set_slot(w: &mut World, key: u32, s: usize, st: ItemStack) {
    let i = w.deploys.vending_index(key).unwrap();
    w.deploys.set_box_slot(i, s, st);
}

fn count(slots: &[ItemStack], item: u16) -> u32 {
    sim_core::shop::count_in(slots, item)
}

fn press(w: &mut World, id: u32, key: u32, act: ShopAct) {
    w.tick(&[Command::Shop { id, key, act }]);
}

/// The one shop event this tick: `Ok(c)` for a press that went through,
/// `Err(reason)` for a refusal.
fn answer(w: &World) -> Result<u32, u32> {
    let mut out = None;
    for e in w.events.entries() {
        if e.code == EV_SHOP {
            assert!(out.is_none(), "two shop answers in one tick");
            out = Some(Ok(e.c));
        } else if e.code == EV_SHOP_REFUSED {
            assert!(out.is_none(), "two shop answers in one tick");
            out = Some(Err(e.c >> 24));
        }
    }
    out.expect("the press was not answered")
}

fn refused(w: &mut World, id: u32, key: u32, act: ShopAct, why: u32) {
    let s = slot_of(w, id);
    let (inv, mach) = (w.players[s].inv, machine(w, key));
    let offers = w.deploys.shops().offers(key);
    press(w, id, key, act);
    assert_eq!(answer(w), Err(why), "{act:?}");
    assert_eq!(w.players[s].inv, inv, "a refused {act:?} moved the pack");
    assert_eq!(machine(w, key), mach, "a refused {act:?} moved the machine");
    assert_eq!(w.deploys.shops().offers(key), offers);
}

const OFFER: ShopAct = ShopAct::Set {
    row: 1,
    item: CORN,
    lot: 3,
    price: 10,
};

/// The terms `OFFER` shows a buyer.
const TERMS: ShopOffer = ShopOffer {
    item: CORN,
    lot: 3,
    price: 10,
};

#[test]
fn an_owner_sets_an_offer_and_a_stranger_buys_it() {
    let (mut w, key) = shop_world();
    press(&mut w, OWNER, key, OFFER);
    assert_eq!(answer(&w), Ok(1 << 8));
    set_slot(&mut w, key, 0, stack(CORN, 10));
    let s = slot_of(&w, STRANGER);
    w.players[s].inv[0] = stack(JUNK, 25);

    press(
        &mut w,
        STRANGER,
        key,
        ShopAct::Buy {
            row: 1,
            times: 2,
            terms: TERMS,
        },
    );
    assert_eq!(answer(&w), Ok((SHOP_OP_BUY as u32) << 16 | 1 << 8 | 2));
    let inv = &w.players[s].inv;
    assert_eq!(count(inv, CORN), 6, "two lots of three");
    assert_eq!(count(inv, JUNK), 5, "paid ten twice");
    let m = machine(&w, key);
    assert_eq!(count(&m, CORN), 4);
    assert_eq!(count(&m, JUNK), 20, "the machine keeps the takings");
}

#[test]
fn every_refusal_names_its_reason_and_moves_nothing() {
    let (mut w, key) = shop_world();
    let s = slot_of(&w, STRANGER);
    // A stranger may not set an offer.
    refused(&mut w, STRANGER, key, OFFER, REFUSE_SH_ALLOWED);
    press(&mut w, OWNER, key, OFFER);
    assert!(answer(&w).is_ok());
    // No such offer.
    refused(
        &mut w,
        STRANGER,
        key,
        ShopAct::Buy {
            row: 0,
            times: 1,
            terms: TERMS,
        },
        REFUSE_SH_OFFER,
    );
    // Out of stock: two corn for a lot of three.
    set_slot(&mut w, key, 0, stack(CORN, 2));
    w.players[s].inv[0] = stack(JUNK, 100);
    let buy = ShopAct::Buy {
        row: 1,
        times: 1,
        terms: TERMS,
    };
    refused(&mut w, STRANGER, key, buy, REFUSE_SH_STOCK);
    // Funds: nine junk for a price of ten.
    set_slot(&mut w, key, 0, stack(CORN, 10));
    w.players[s].inv[0] = stack(JUNK, 9);
    refused(&mut w, STRANGER, key, buy, REFUSE_SH_FUNDS);
    // The buyer's pack is full, and the junk left after paying keeps its slot.
    w.players[s].inv[0] = stack(JUNK, 100);
    for k in 1..INV_SLOTS {
        w.players[s].inv[k] = stack(FILLER, 1);
    }
    refused(&mut w, STRANGER, key, buy, REFUSE_SH_FULL);
    // The machine is full: twelve slots of corn, none freed by one lot.
    w.players[s].inv = [ItemStack::default(); INV_SLOTS];
    w.players[s].inv[0] = stack(JUNK, 100);
    for k in 0..BOX_SLOTS {
        set_slot(&mut w, key, k, stack(CORN, 10));
    }
    refused(&mut w, STRANGER, key, buy, REFUSE_SH_TAKINGS);
    // Out of reach.
    let far = cell_center(600, 600);
    w.players[s].body = Body::at(SEED, &w.haven, far.0, far.1);
    refused(&mut w, STRANGER, key, buy, REFUSE_SH_REACH);
}

#[test]
fn only_its_owner_moves_its_slots_or_lifts_it_and_its_offers_go_with_it() {
    let (mut w, key) = shop_world();
    set_slot(&mut w, key, 0, stack(CORN, 5));
    let take = |id| Command::Move {
        id,
        cont: key,
        from_kind: CONT_BOX,
        from_slot: 0,
        to_kind: CONT_SELF,
        to_slot: 5,
        count: 5,
    };
    w.tick(&[take(STRANGER)]);
    assert!(
        w.events
            .entries()
            .iter()
            .any(|e| e.code == EV_DEPLOY_REFUSED
                && e.a == STRANGER
                && e.b == sim_core::deploy::REFUSE_D_OWNER),
        "a stranger took stock out of a vending machine"
    );
    assert_eq!(count(&machine(&w, key), CORN), 5);
    w.tick(&[take(OWNER)]);
    assert_eq!(count(&machine(&w, key), CORN), 0, "its owner stocks it");

    press(&mut w, OWNER, key, OFFER);
    assert!(answer(&w).is_ok());
    let (cx, cz, level, loc) = sim_core::deploy::box_addr(key);
    let lift = |id| Command::Demolish {
        id,
        deploy: true,
        cx,
        cz,
        level,
        loc,
    };
    w.tick(&[lift(STRANGER)]);
    assert!(
        w.deploys.vending_index(key).is_some(),
        "a stranger lifted it"
    );
    w.tick(&[lift(OWNER)]);
    assert!(w.deploys.vending_index(key).is_none(), "its owner lifts it");
    assert!(
        w.deploys.shops().is_empty(),
        "the offers outlived the machine"
    );
}

/// Save `w` (everybody asleep, so a load is the same world) and load it back
/// into a fresh one under the same tables.
fn reload(w: &mut World, cut: impl Fn(&mut Vec<u8>)) -> Box<World> {
    w.tick(&[
        Command::Leave { id: OWNER },
        Command::Leave { id: STRANGER },
    ]);
    let mut blob = vec![0u8; sim_core::worldsave::WORLD_SAVE_MAX_BYTES];
    let n = w.save_world(&mut blob).unwrap();
    blob.truncate(n);
    cut(&mut blob);
    let mut back = Box::new(World::new(SEED));
    back.gather = w.gather;
    back.build = w.build;
    back.deploy = w.deploy;
    back.combat = w.combat;
    back.load(&blob).expect("the blob loads");
    back
}

#[test]
fn the_offers_survive_a_save_and_an_older_file_is_refused() {
    let (mut w, key) = shop_world();
    press(&mut w, OWNER, key, OFFER);
    assert!(answer(&w).is_ok());
    let back = reload(&mut w, |_| {});
    assert!(!back.deploys.shops().is_empty());
    assert_eq!(
        back.deploys.shops().offers(key),
        w.deploys.shops().offers(key)
    );
    assert_eq!(back.state_hash(), w.state_hash());

    // A 25 file was a 26 file minus the shop section and still read, until
    // format 27 moved every body's record (the blueprint mask's high word):
    // an older file reads as nothing now, refused by its format.
    let (mut w, _) = shop_world();
    w.tick(&[
        Command::Leave { id: OWNER },
        Command::Leave { id: STRANGER },
    ]);
    let mut blob = vec![0u8; sim_core::worldsave::WORLD_SAVE_MAX_BYTES];
    let n = w.save_world(&mut blob).unwrap();
    blob.truncate(n);
    blob[0..2].copy_from_slice(&26u16.to_le_bytes());
    let mut back = Box::new(World::new(SEED));
    assert!(back.load(&blob).is_err(), "a format 26 file is refused");
}

/// **A buy is charged only on the terms it was shown.** The owner reprices
/// the row between the buyer's look and their press: the buy is refused as
/// a gone offer and nothing moves, and the same press on the new terms goes
/// through.
#[test]
fn a_buy_on_terms_the_owner_rewrote_is_refused_and_moves_nothing() {
    let (mut w, key) = shop_world();
    press(&mut w, OWNER, key, OFFER);
    assert!(answer(&w).is_ok());
    set_slot(&mut w, key, 0, stack(CORN, 10));
    let s = slot_of(&w, STRANGER);
    w.players[s].inv[0] = stack(JUNK, 25);
    press(
        &mut w,
        OWNER,
        key,
        ShopAct::Set {
            row: 1,
            item: CORN,
            lot: 3,
            price: 20,
        },
    );
    assert!(answer(&w).is_ok());
    let stale = ShopAct::Buy {
        row: 1,
        times: 1,
        terms: TERMS,
    };
    refused(&mut w, STRANGER, key, stale, REFUSE_SH_OFFER);
    assert_eq!(count(&w.players[s].inv, JUNK), 25, "nothing was charged");
    assert_eq!(count(&machine(&w, key), CORN), 10);
    let shown = ShopAct::Buy {
        row: 1,
        times: 1,
        terms: ShopOffer { price: 20, ..TERMS },
    };
    press(&mut w, STRANGER, key, shown);
    assert!(answer(&w).is_ok());
    assert_eq!(count(&w.players[s].inv, JUNK), 5);
}
