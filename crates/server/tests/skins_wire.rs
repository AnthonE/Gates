//! Skins on the wire (skins v0): `ShardCore` ↔ `ClientCore` through real
//! encoded bytes. The catalog drips to a joiner; the platform's answer
//! reaches the sim as a server-minted command and the owner hears their
//! set; a skinned craft mints the skinned item, whose skin reaches the
//! owner's inventory mirror and — once it is in hand — every other client's
//! snapshot; and a skin nobody owns refuses the craft before it spends.

use client_core::core::ClientCore;
use protocol::{ActionMsg, ItemCatalog, SkinCatalog, SkinRow, COIN_ELO, COIN_NONE, COIN_ORBS};
use server::core::{Lane, ShardCore};
use server::stats::ShardStats;
use sim_core::craft::{CraftContent, REFUSE_SKIN};
use sim_core::gather::{GatherContent, ItemStack};
use sim_core::skin::{SkinContent, SkinDef, SkinSet};

const SEED: u64 = 20_260_731;
const SPAWN: (f32, f32) = (1024.0, 1024.0);
/// Recipe 1 of the craft probe fixture: 2 × item 1 + 1 × item 2 → item 3,
/// no station.
const RECIPE: u16 = 1;
const OUTPUT: u16 = 3;
/// The one skin that fits the output, and one that fits something else.
const SKIN: u16 = 41;
const OTHER: u16 = 42;

fn id_of(slot: usize) -> u32 {
    (1 << 8) | slot as u32
}

fn probe_catalog() -> ItemCatalog {
    let mut cat = ItemCatalog::EMPTY;
    cat.count = 8;
    for i in 0..8usize {
        cat.set(i, &[b'P', b'0' + i as u8], protocol::ItemRow::EMPTY)
            .unwrap();
    }
    cat
}

fn skins() -> (SkinContent, SkinCatalog) {
    let mut sc = SkinContent::EMPTY;
    let mut wire = SkinCatalog::EMPTY;
    for (i, (catalog, covers, coin, price)) in
        [(SKIN, OUTPUT, COIN_ELO, 250), (OTHER, 4, COIN_NONE, 0)]
            .into_iter()
            .enumerate()
    {
        sc.defs[i] = SkinDef { catalog, covers };
        wire.set(
            i,
            b"Test Skin",
            SkinRow {
                catalog,
                covers,
                tint: [200, 100, 50],
                coin,
                price,
            },
        )
        .unwrap();
    }
    sc.count = 2;
    wire.count = 2;
    (sc, wire)
}

fn pump(core: &mut ShardCore, stats: &ShardStats, clients: &mut [(usize, ClientCore)]) {
    let mut buf = [0u8; 1100];
    for (slot, c) in clients.iter_mut() {
        c.advance(1000.0 / 30.0);
        let n = c.poll_input(&mut buf);
        if n > 0 {
            let dg = protocol::decode_input(&buf[..n]).expect("client encodes valid input");
            core.push_input(*slot, &dg);
        }
    }
    let mut snaps: Vec<(usize, Vec<u8>)> = Vec::new();
    let mut events: Vec<(usize, Vec<u8>)> = Vec::new();
    core.tick_bare(stats, |lane, slot, bytes| {
        match lane {
            Lane::Snapshot => snaps.push((slot, bytes.to_vec())),
            Lane::Event => events.push((slot, bytes.to_vec())),
        }
        true
    });
    for (slot, bytes) in events {
        if let Some(c) = clients.iter_mut().find(|(s, _)| *s == slot).map(|(_, c)| c) {
            c.on_stream(&bytes).expect("server events decode");
        }
    }
    for (slot, bytes) in snaps {
        if let Some(c) = clients.iter_mut().find(|(s, _)| *s == slot).map(|(_, c)| c) {
            c.on_datagram(&bytes);
        }
    }
}

fn world_slot(core: &ShardCore, id: u32) -> usize {
    core.world
        .players
        .iter()
        .position(|p| p.active && p.id == id)
        .expect("player in world")
}

fn act(core: &mut ShardCore, slot: usize, a: ActionMsg) {
    assert!(core.wants_action(slot), "hand should be open");
    core.push_action(slot, a);
}

fn stack(item: u16, count: u16) -> ItemStack {
    ItemStack {
        item,
        count,
        cond: 0,
        skin: 0,
    }
}

#[test]
fn a_skin_is_owned_crafted_carried_and_seen() {
    let stats = ShardStats::default();
    let mut core = Box::new(ShardCore::new(SEED));
    core.world.gather = GatherContent::probe_fixture();
    core.world.craft = CraftContent::probe_fixture();
    let (sc, wire) = skins();
    core.world.skins = sc;
    *core.skin_catalog = wire;
    core.world.dev_spawn = Some(SPAWN);
    core.catalog = probe_catalog();
    assert!(core.connect(0, id_of(0)));
    assert!(core.connect(1, id_of(1)));
    let mut clients = vec![
        (0usize, ClientCore::new(SEED, id_of(0), 0)),
        (1usize, ClientCore::new(SEED, id_of(1), 0)),
    ];
    for _ in 0..6 {
        pump(&mut core, &stats, &mut clients);
    }

    // The catalog dripped, in row order, and nobody owns anything yet.
    for (_, c) in &clients {
        assert_eq!(c.skins.count, 2, "the skin catalog dripped");
        assert_eq!(c.skins.rows[0].catalog, SKIN);
        assert_eq!(c.skins.rows[0].price, 250);
        assert!(c.skins_owned.is_empty());
    }

    let w0 = world_slot(&core, id_of(0));
    core.world.players[w0].inv[0] = stack(1, 4);
    core.world.players[w0].inv[1] = stack(2, 2);

    // A skin the crafter does not own: refused, nothing spent.
    act(
        &mut core,
        0,
        ActionMsg::Craft {
            recipe: RECIPE,
            count: 1,
            skin: SKIN,
        },
    );
    pump(&mut core, &stats, &mut clients);
    assert_eq!(
        clients[0].1.pop_craft_refusal(),
        Some(REFUSE_SKIN as u8),
        "the crafter hears why"
    );
    assert_eq!(core.world.players[w0].inv[0].count, 4, "nothing was spent");

    // The platform answers (what the accept loop does after a read): the
    // set reaches the sim as a command, and the owner hears it.
    let mut owned = SkinSet::EMPTY;
    owned.insert(0);
    core.skins_owned(0, id_of(0), owned);
    // A stale answer about a tenant who left is dropped.
    core.skins_owned(1, 0xDEAD, SkinSet::all(2));
    pump(&mut core, &stats, &mut clients);
    pump(&mut core, &stats, &mut clients);
    assert_eq!(core.world.players[w0].skins, owned, "the sim holds the set");
    assert_eq!(clients[0].1.skins_owned, owned, "and the owner heard it");
    let w1 = world_slot(&core, id_of(1));
    assert!(
        core.world.players[w1].skins.is_empty(),
        "the stale answer landed nowhere"
    );
    assert_eq!(
        clients[0].1.skins_for(OUTPUT).collect::<Vec<_>>(),
        vec![(0, clients[0].1.skins.rows[0], true)],
        "the picker's rows: the one skin that fits, owned"
    );

    // Owned and fitting: the craft mints the skinned item.
    act(
        &mut core,
        0,
        ActionMsg::Craft {
            recipe: RECIPE,
            count: 1,
            skin: SKIN,
        },
    );
    for _ in 0..8 {
        pump(&mut core, &stats, &mut clients);
    }
    let made = core.world.players[w0]
        .inv
        .iter()
        .position(|s| s.item == OUTPUT && s.count > 0)
        .expect("the craft paid out");
    assert_eq!(
        core.world.players[w0].inv[made].skin, SKIN,
        "minted wearing it"
    );
    assert_eq!(
        clients[0].1.inv[made].skin, SKIN,
        "the inventory mirror carries the skin"
    );

    // Into the hand: move it to hotbar slot 0 if it is not there, and
    // select it. Every other client sees the skin on the held item.
    core.world.players[w0].inv.swap(0, made);
    core.world.players[w0].frame.sel = 0;
    for _ in 0..4 {
        pump(&mut core, &stats, &mut clients);
    }
    let seen = *clients[1]
        .1
        .view
        .get(id_of(0))
        .expect("the bystander draws the crafter");
    assert_eq!(seen.held, Some(OUTPUT));
    assert_eq!(seen.held_skin, SKIN, "the bystander sees the skin in hand");
}

/// The platform's store reprices mid-session (`skins::prices_of` →
/// `ShardCore::skin_prices`): every client is sent the catalog again and ends
/// with the store's price, the same rows overwritten rather than appended. A
/// read that moves nothing re-sends nothing.
#[test]
fn a_store_price_reaches_every_client_and_replaces_the_row() {
    let stats = ShardStats::default();
    let mut core = Box::new(ShardCore::new(SEED));
    core.world.gather = GatherContent::probe_fixture();
    core.world.craft = CraftContent::probe_fixture();
    let (sc, wire) = skins();
    core.world.skins = sc;
    *core.skin_catalog = wire;
    core.world.dev_spawn = Some(SPAWN);
    core.catalog = probe_catalog();
    assert!(core.connect(0, id_of(0)));
    let mut clients = vec![(0usize, ClientCore::new(SEED, id_of(0), 0))];
    for _ in 0..6 {
        pump(&mut core, &stats, &mut clients);
    }
    assert_eq!(
        clients[0].1.skins.rows[0].price, 250,
        "content's price, before any read"
    );

    let mut prices = [None; sim_core::limits::MAX_SKINS];
    prices[0] = Some((COIN_ORBS, 7));
    prices[1] = Some((COIN_ELO, 3));
    core.skin_prices(&prices);
    for _ in 0..6 {
        pump(&mut core, &stats, &mut clients);
    }
    let c = &clients[0].1;
    assert_eq!(c.skins.count, 2, "re-sent, not appended");
    assert_eq!(
        (c.skins.rows[0].coin, c.skins.rows[0].price),
        (COIN_ORBS, 7)
    );
    assert_eq!((c.skins.rows[1].coin, c.skins.rows[1].price), (COIN_ELO, 3));
    assert_eq!(c.skins.rows[0].catalog, SKIN, "the row is the same skin");

    core.skin_prices(&prices);
    assert_eq!(
        core.clients[0].skins_cursor, 2,
        "an unchanged read re-sends nothing"
    );

    // Off sale at the store: the row reads unpriced, whatever content said.
    core.skin_prices(&[None; sim_core::limits::MAX_SKINS]);
    for _ in 0..6 {
        pump(&mut core, &stats, &mut clients);
    }
    let c = &clients[0].1;
    assert_eq!(
        (c.skins.rows[0].coin, c.skins.rows[0].price),
        (COIN_NONE, 0)
    );
}
