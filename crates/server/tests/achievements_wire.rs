//! Achievements through a real `ShardCore` tick: a wallet that crafts earns
//! the row, hears it in chat, and a guest beside it earns nothing.

use protocol::{decode_event, ActionMsg, EventMsg};
use server::achievements::{Achievements, Baked, Def, Rule, Set};
use server::core::{Lane, ShardCore};
use server::stats::ShardStats;
use server::store::PlayerKey;
use sim_core::craft::CraftContent;
use sim_core::gather::{GatherContent, ItemStack};

const SEED: u64 = 20_260_731;
const SPAWN: (f32, f32) = (2048.0, 2048.0);

const SETS: &[Set] = &[Set {
    id: "t",
    name: "Test",
    about: "",
    archived: false,
}];
const DEFS: &[Def] = &[Def {
    id: "CRAFT_P2",
    set: "t",
    name: "Made P2",
    desc: "",
    rule: Rule::Craft(&["p2"]),
    spoiler: false,
    archived: false,
}];

fn id_of(slot: usize) -> u32 {
    (1 << 8) | slot as u32
}

#[test]
fn a_wallet_that_crafts_earns_and_hears_it() {
    let stats = ShardStats::default();
    let mut core = Box::new(ShardCore::new(SEED));
    let craft = CraftContent::probe_fixture();
    assert_eq!(
        craft.recipes[0].output, 2,
        "the fixture's first recipe makes item 2"
    );
    core.world.gather = GatherContent::probe_fixture();
    core.world.craft = craft;
    core.world.dev_spawn = Some(SPAWN);
    let items = ["p0", "p1", "p2", "p3", "p4"];
    let baked = Baked::bake(
        DEFS,
        SETS,
        items.len(),
        |id| items.iter().position(|i| *i == id).map(|i| i as u16),
        &[],
    );
    core.achievements = Achievements::new(DEFS, SETS, baked);

    let wallet = PlayerKey::new(format!("0x{:040x}", 0xa1).as_bytes()).unwrap();
    assert!(core.connect_as(0, id_of(0), Some(wallet), None).is_some());
    assert!(core.connect(1, id_of(1)), "a guest");
    let mut said: Vec<(usize, String)> = Vec::new();
    let tick = |core: &mut ShardCore, said: &mut Vec<(usize, String)>| {
        core.tick_bare(&stats, |lane, slot, bytes| {
            if lane == Lane::Event {
                if let Ok(EventMsg::Chat { text, .. }) = decode_event(bytes) {
                    said.push((slot, String::from_utf8_lossy(text.as_bytes()).into_owned()));
                }
            }
            true
        });
    };
    tick(&mut core, &mut said);
    assert!(core.achievements.tracks(0));
    assert!(!core.achievements.tracks(1), "a guest is nobody");

    for slot in [0usize, 1] {
        let w = core
            .world
            .players
            .iter()
            .position(|p| p.active && p.id == id_of(slot))
            .expect("seated");
        core.world.players[w].inv[0] = ItemStack {
            item: 0,
            count: 20,
            cond: 0,
            skin: 0,
        };
        assert!(core.wants_action(slot));
        core.push_action(
            slot,
            ActionMsg::Craft {
                recipe: 0,
                count: 1,
                skin: 0,
            },
        );
    }
    for _ in 0..30 {
        tick(&mut core, &mut said);
    }
    assert_eq!(core.achievements.row(0).map(|r| r.got), Some(1));
    let told: Vec<_> = said.iter().filter(|(_, l)| l.contains("Made P2")).collect();
    assert_eq!(
        told,
        vec![&(0, "[server] Achievement: Made P2".to_string())],
        "told once, to the crafter only"
    );
    assert!(said
        .iter()
        .any(|(s, l)| *s == 0 && l == "[server] Set complete: Test"));
}

/// The writer thread lands the record, the status thread serves it, and a
/// restart reads it back: an unlock is a career, not a session.
#[test]
fn the_record_is_served_and_survives_a_restart() {
    use std::io::{Read, Write};
    let dir = std::env::temp_dir().join(format!("gates-achievements-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let base = dir.join("world.bin");
    let wallet = format!("0x{:040x}", 0xa1);

    // The shipped list through the real writer: join, flush, close.
    let mut live = Achievements::open(server::achievements::Baked::default(), Some(base.as_path()));
    live.join(0, Some(&wallet));
    live.close();
    let json = std::fs::read_to_string(server::achievements::json_path(&base)).expect("written");
    let v: serde_json::Value = serde_json::from_str(&json).expect("json");
    assert_eq!(v["players"], 1, "a joined wallet is a player on record");
    assert_eq!(
        v["achievements"].as_array().map(Vec::len),
        Some(server::achievements::ACHIEVEMENTS.len())
    );

    // A record with an unlock in it reads back into the row at the next boot.
    let rec = format!("{wallet} a:COLLECT_100_WOOD@100 c:gather:item.wood=140\n");
    std::fs::write(server::achievements::record_path(&base), rec).expect("record");
    let mut again =
        Achievements::open(server::achievements::Baked::default(), Some(base.as_path()));
    again.join(3, Some(&wallet));
    assert_eq!(
        again.row(3).map(|r| r.got & 1),
        Some(1),
        "COLLECT_100_WOOD is row 0"
    );
    again.close();

    let addr = server::status::spawn_status_with(
        "127.0.0.1:0".parse().expect("static addr"),
        std::sync::Arc::new(ShardStats::default()),
        Some(base.clone()),
    )
    .expect("bind loopback");
    let mut s = std::net::TcpStream::connect(addr).expect("connect");
    s.write_all(b"GET /achievements.json HTTP/1.1\r\n\r\n")
        .expect("send");
    let mut text = String::new();
    s.read_to_string(&mut text).expect("read");
    assert!(text.starts_with("HTTP/1.1 200"), "{text}");
    assert!(text.contains("Access-Control-Allow-Origin: *"));
    let body: serde_json::Value =
        serde_json::from_str(text.split("\r\n\r\n").nth(1).expect("body")).expect("json");
    assert_eq!(
        body["wallets"][&wallet]["got"]["COLLECT_100_WOOD"], 100,
        "the stamp is kept"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
