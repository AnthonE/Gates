//! Real input/action/event bytes: a second player can finish the wound loop.
use client_core::core::ClientCore;
use protocol::{ActionMsg, EventMsg};
use server::{
    core::{Lane, ShardCore},
    stats::ShardStats,
};
use sim_core::{
    assist::ASSIST_TICKS,
    input::{BTN_ASSIST, BTN_PRIMARY},
    movement::{Body, POS_XZ_Q, POS_Y_Q},
};

const SEED: u64 = 42;
fn id(slot: usize) -> u32 {
    256 + slot as u32
}
fn pump(
    core: &mut ShardCore,
    stats: &ShardStats,
    clients: &mut [ClientCore],
    drop_helper: bool,
) -> Vec<(usize, EventMsg)> {
    let mut buf = [0; 1100];
    for (slot, c) in clients.iter_mut().enumerate() {
        c.advance(1000.0 / 30.0);
        let n = c.poll_input(&mut buf);
        if n > 0 {
            core.push_input(slot, &protocol::decode_input(&buf[..n]).unwrap());
        }
    }
    let mut events = Vec::new();
    core.tick_bare(stats, |lane, slot, bytes| {
        match lane {
            Lane::Snapshot => {
                clients[slot].on_datagram(bytes);
            }
            Lane::Event => {
                if slot == 0 && drop_helper {
                    return false;
                }
                events.push((slot, protocol::decode_event(bytes).unwrap()));
                clients[slot].on_stream(bytes).unwrap();
            }
        }
        true
    });
    events
}
#[test]
fn a_hand_hold_crosses_the_wire_and_only_its_participants_see_progress() {
    let mut core = Box::new(ShardCore::new(SEED));
    core.world.combat = sim_core::combat::CombatContent::probe_fixture();
    core.world.gather = sim_core::gather::GatherContent::probe_fixture();
    core.world.dev_spawn = Some(core.world.spawn_pos(id(0)));
    let stats = ShardStats::default();
    let mut clients: Vec<_> = (0..3).map(|s| ClientCore::new(SEED, id(s), 0)).collect();
    for slot in 0..3 {
        assert!(core.connect(slot, id(slot)));
    }
    for _ in 0..20 {
        pump(&mut core, &stats, &mut clients, false);
    }
    let a = core.world.players[0].body;
    core.world.players[1].body = Body::at(
        SEED,
        &core.world.haven,
        a.qx as f32 * POS_XZ_Q,
        a.qz as f32 * POS_XZ_Q + 1.5,
    );
    core.world.players[1].hp = 1;
    core.world.players[0].inv[0] = sim_core::gather::ItemStack {
        item: 0,
        count: 1,
        cond: 400,
        skin: 0,
    };
    // Keep the bystander out of the attack ray, while retaining its session.
    core.world.players[2].body.qx += 200;
    let b = core.world.players[1].body;
    let run = (b.qz - a.qz) as f32 * POS_XZ_Q;
    let rise = (b.qy - a.qy) as f32 * POS_Y_Q + 0.3 - 1.6;
    let pitch = (0..=255u8)
        .max_by(|&a, &b| {
            let (ac, as_) = sim_core::pitch_dir(a);
            let (bc, bs) = sim_core::pitch_dir(b);
            (ac * run + as_ * rise).total_cmp(&(bc * run + bs * rise))
        })
        .unwrap();
    clients[0].set_input(BTN_PRIMARY, 0, pitch, 0, 0, 0);
    for _ in 0..20 {
        pump(&mut core, &stats, &mut clients, false);
        if core.world.players[1].wounded {
            break;
        }
    }
    assert!(
        core.world.players[1].wounded,
        "the setup must wound through combat"
    );
    assert!(clients[1].wounded, "the fall must cross the event lane");
    clients[0].set_input(BTN_ASSIST, 0, pitch, 0, 0, 0);
    let mut buf = [0; 32];
    let n = protocol::encode_action_assist(id(1), &mut buf).unwrap();
    let action = protocol::decode_action(&buf[..n]).unwrap();
    assert_eq!(action, ActionMsg::Assist { target: id(1) });
    core.push_action(0, action);
    let mut progressed = [false; 3];
    let mut completed = false;
    for tick in 0..ASSIST_TICKS * 2 + 30 {
        for (slot, e) in pump(&mut core, &stats, &mut clients, (20..30).contains(&tick)) {
            if let EventMsg::Assist {
                helper,
                target,
                ticks,
            } = e
            {
                if ticks > 0 {
                    assert_eq!((helper, target), (id(0), id(1)));
                }
                assert!(ticks <= ASSIST_TICKS);
                assert_ne!(slot, 2, "a hold is not broadcast to a bystander");
                progressed[slot] |= ticks > 0;
            }
        }
        if tick == 40 {
            assert_eq!(
                clients[0].assist.2, core.world.players[1].assist_ticks,
                "progress after a dropped event is absolute, not accumulated"
            );
            clients[0].set_input(0, 0, pitch, 0, 0, 0);
            for _ in 0..12 {
                pump(&mut core, &stats, &mut clients, true);
            }
            assert_eq!(core.world.players[1].assist_ticks, 0);
            assert!(
                clients[0].assist.2 > 0,
                "the dropped cancellation must leave stale state"
            );
            pump(&mut core, &stats, &mut clients, false);
            assert_eq!(
                clients[0].assist,
                (0, 0, 0),
                "idle state retries without another hold"
            );
            clients[0].set_input(BTN_ASSIST, 0, pitch, 0, 0, 0);
            core.push_action(0, ActionMsg::Assist { target: id(1) });
        }
        if !core.world.players[1].wounded {
            completed = true;
            break;
        }
    }
    assert!(completed, "a steady input stream must finish the hold");
    assert_eq!(progressed, [true, true, false]);
    assert!(!clients[1].wounded);
    assert!(clients[1].pop_recovered().is_some());
    assert_eq!(clients[2].assist, (0, 0, 0));
}
