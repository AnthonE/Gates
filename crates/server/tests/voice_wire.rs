//! Proximity voice's routing gate (v111, `reference/VOICE.md` §9.1): the
//! server decides who hears a talker, and a listener one quantum past the
//! radius gets nothing at all — not a quieter frame. A dead talker is
//! heard by nobody, and voice never touches the world.

use server::core::{Lane, ShardCore};
use server::slot::MAX_CONNS;
use server::stats::ShardStats;
use sim_core::gather::GatherContent;
use sim_core::limits::VOICE_RADIUS_CM;

const SEED: u64 = 20_261_015;
const SPAWN: (f32, f32) = (2048.0, 2048.0);
const Q_CM: i32 = 3;

fn id_of(slot: usize) -> u32 {
    (1 << 8) | slot as u32
}

fn world_slot(core: &ShardCore, id: u32) -> usize {
    core.world
        .players
        .iter()
        .position(|p| p.active && p.id == id)
        .expect("player in world")
}

fn tick(core: &mut ShardCore, stats: &ShardStats) {
    core.tick_bare(stats, |_: Lane, _, _| true);
}

#[test]
fn the_voice_radius_is_exact_and_server_side() {
    let stats = ShardStats::default();
    let mut core = Box::new(ShardCore::new(SEED));
    core.world.gather = GatherContent::probe_fixture();
    core.world.dev_spawn = Some(SPAWN);
    for slot in 0..3 {
        assert!(core.connect(slot, id_of(slot)));
    }
    tick(&mut core, &stats);
    tick(&mut core, &stats);

    let w0 = world_slot(&core, id_of(0));
    let w1 = world_slot(&core, id_of(1));
    let w2 = world_slot(&core, id_of(2));
    let base = core.world.players[w0].body.qx;
    core.world.players[w2].body.qx = base + 20_000 / Q_CM; // 200 m off
    core.world.players[w1].body.qx = base + (VOICE_RADIUS_CM as i32) / Q_CM;

    let mut hears = [false; MAX_CONNS];
    assert_eq!(core.voice_reach(0, &mut hears), Some(id_of(0)));
    assert!(!hears[0], "a talker is not relayed to themself");
    assert!(hears[1], "a listener exactly at the radius hears");
    assert!(!hears[2], "a listener far off hears nothing");

    core.world.players[w1].body.qx += 1;
    let hash = core.world.state_hash();
    core.voice_reach(0, &mut hears);
    assert!(!hears[1], "one quantum past the radius is silence");
    assert_eq!(core.world.state_hash(), hash, "voice is not sim state");

    core.world.players[w0].dead = true;
    assert_eq!(
        core.voice_reach(0, &mut hears),
        None,
        "the dead do not talk"
    );
    assert!(hears.iter().all(|h| !h));
}
