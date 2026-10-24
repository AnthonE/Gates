//! Achievements through a real `ShardCore` tick: a wallet that crafts earns
//! the row and gets its popup, `/ach` says where it stands, and a guest
//! beside it earns nothing.

use protocol::{decode_event, ActionMsg, Channel, ChatMsg, ChatText, EventMsg};
use server::achievements::{Achievements, Baked, Def, Lookup, Rule, Set};
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
    icon: "ui_star",
    archived: false,
}];
const DEFS: &[Def] = &[Def {
    id: "CRAFT_P2",
    set: "t",
    name: "Made P2",
    desc: "Craft a P2",
    icon: "hammer",
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
    let item = |id: &str| items.iter().position(|i| *i == id).map(|i| i as u16);
    let baked = Baked::bake(
        DEFS,
        SETS,
        &Lookup {
            n_items: items.len(),
            item: &item,
            deploy_items: &[],
            works: &[],
            mechs: &[],
            cards: &[],
            speakers: 0,
            glyphs: 0,
        },
    );
    core.achievements = Achievements::new(DEFS, SETS, baked);

    let wallet = PlayerKey::new(format!("0x{:040x}", 0xa1).as_bytes()).unwrap();
    assert!(core.connect_as(0, id_of(0), Some(wallet), None).is_some());
    assert!(core.connect(1, id_of(1)), "a guest");
    let mut said: Vec<(usize, String)> = Vec::new();
    let tick = |core: &mut ShardCore, said: &mut Vec<(usize, String)>| {
        core.tick_bare(&stats, |lane, slot, bytes| {
            if lane == Lane::Event {
                let line = match decode_event(bytes) {
                    Ok(EventMsg::Chat { text, .. }) => {
                        String::from_utf8_lossy(text.as_bytes()).into_owned()
                    }
                    Ok(EventMsg::Achievement(a)) => format!(
                        "popup {} {} {}/{} {}",
                        if a.set { "set" } else { "row" },
                        String::from_utf8_lossy(a.name.as_bytes()),
                        a.done,
                        a.total,
                        a.icon
                            .map(|i| String::from_utf8_lossy(i.as_bytes()).into_owned())
                            .unwrap_or_default()
                    ),
                    _ => return true,
                };
                said.push((slot, line));
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
            ..ItemStack::EMPTY
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
    let popups: Vec<_> = said
        .iter()
        .filter(|(_, l)| l.starts_with("popup"))
        .collect();
    assert_eq!(
        popups,
        vec![
            &(0, "popup row Made P2 1/1 hammer".to_string()),
            &(0, "popup set Test 1/1 ui_star".to_string())
        ],
        "popped once each, to the crafter only"
    );

    // `/ach`, asked in chat, answers the asker alone.
    said.clear();
    let ask = || ChatMsg {
        channel: Channel::Global,
        text: ChatText::sanitize(b"/ach").unwrap(),
    };
    core.push_chat(0, ask());
    core.push_chat(1, ask());
    tick(&mut core, &mut said);
    assert!(
        said.contains(&(0, "[server] Test: 1/1 - done".to_string())),
        "{said:?}"
    );
    assert!(
        said.contains(&(1, "[server] Achievements need a wallet sign-in".to_string())),
        "{said:?}"
    );
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
    live.close(|_| None);
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
    again.close(|_| None);

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
