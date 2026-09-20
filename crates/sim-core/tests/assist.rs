use sim_core::{
    assist::ASSIST_TICKS,
    input::{InputFrame, BTN_ASSIST},
    movement::{Body, POS_XZ_Q, POS_Y_Q},
    world::{Command, World, EV_ASSIST, EV_RECOVERED},
};

fn fixture() -> World {
    let mut w = World::new(42);
    w.combat = sim_core::combat::CombatContent::probe_fixture();
    w.dev_spawn = Some(w.spawn_pos(1));
    w.tick(&[
        Command::Join { id: 1 },
        Command::Join { id: 2 },
        Command::Join { id: 3 },
    ]);
    let a = w.players[0].body;
    w.players[1].body = Body::at(
        42,
        &w.haven,
        a.qx as f32 * POS_XZ_Q,
        a.qz as f32 * POS_XZ_Q + 1.5,
    );
    w.players[1].hp = 10;
    w.players[1].wounded = true;
    w.players[1].wound_until = w.tick + 900;
    w
}
fn frame(w: &World, slot: usize) -> InputFrame {
    let (a, b) = (w.players[slot].body, w.players[1].body);
    let run = (b.qz - a.qz) as f32 * POS_XZ_Q;
    let rise = (b.qy - a.qy) as f32 * POS_Y_Q + 0.3 - 1.6;
    let pitch = (0..=255u8)
        .max_by(|&a, &b| {
            let (ac, as_) = sim_core::pitch_dir(a);
            let (bc, bs) = sim_core::pitch_dir(b);
            (ac * run + as_ * rise).total_cmp(&(bc * run + bs * rise))
        })
        .unwrap();
    InputFrame {
        seq: w.tick as u16,
        buttons: BTN_ASSIST,
        yaw: 0,
        pitch,
        ..Default::default()
    }
}
fn hold(w: &mut World) {
    w.tick(&[
        Command::Input {
            id: 1,
            frame: frame(w, 0),
            favour: 0,
        },
        Command::Assist { id: 1, target: 2 },
    ]);
}
#[test]
fn six_seconds_gets_up_with_the_existing_health_and_rewound_window() {
    let mut w = fixture();
    let until = w.players[1].wound_until;
    for elapsed in 1..ASSIST_TICKS {
        hold(&mut w);
        assert!(w.players[1].wounded, "early recovery at {elapsed}");
        assert_eq!(w.players[1].assist_ticks, elapsed);
        assert_eq!(w.players[1].wound_until, until + elapsed as u64);
    }
    hold(&mut w);
    assert!(!w.players[1].wounded);
    assert!(!w.players[1].dead);
    assert_eq!(w.players[1].hp, 10);
    assert_eq!(
        w.players[1].rewound_until,
        w.tick - 1 + sim_core::wound::REWOUND_TICKS
    );
    assert!(w
        .events
        .entries()
        .iter()
        .any(|e| e.code == EV_RECOVERED && e.a == 2));
}
#[test]
fn releasing_resets_progress_and_preserves_the_time_spent_helping() {
    let mut w = fixture();
    let until = w.players[1].wound_until;
    for _ in 0..60 {
        hold(&mut w);
    }
    let mut f = frame(&w, 0);
    f.buttons = 0;
    w.tick(&[Command::Input {
        id: 1,
        frame: f,
        favour: 0,
    }]);
    assert_eq!(w.players[1].assist_ticks, 0);
    assert_eq!(w.players[1].wound_until, until + 60);
    assert!(w
        .events
        .entries()
        .iter()
        .any(|e| e.code == EV_ASSIST && e.c == 0));
    hold(&mut w);
    assert_eq!(w.players[1].assist_ticks, 1);
}
#[test]
fn a_hold_spanning_the_roll_deadline_suspends_the_roll() {
    let mut w = fixture();
    w.players[1].wound_until = w.tick;
    for _ in 0..ASSIST_TICKS {
        hold(&mut w);
        assert!(!w.players[1].dead);
    }
    assert!(!w.players[1].wounded);
}
#[test]
fn movement_and_decayed_input_cannot_finish_a_hold() {
    let mut w = fixture();
    hold(&mut w);
    let mut f = frame(&w, 0);
    f.move_x = 127;
    w.tick(&[Command::Input {
        id: 1,
        frame: f,
        favour: 0,
    }]);
    assert_eq!(w.players[1].assist_ticks, 0);
    hold(&mut w);
    let f = sim_core::input::decay_frame(&frame(&w, 0), 1);
    w.tick(&[Command::Input {
        id: 1,
        frame: f,
        favour: 0,
    }]);
    assert_eq!(w.players[1].assist_ticks, 0);
}
#[test]
fn out_of_reach_sleepers_and_self_assists_do_nothing() {
    let mut w = fixture();
    w.players[1].body.qz += 1000;
    hold(&mut w);
    assert_eq!(w.players[1].assist_ticks, 0);
    let mut w = fixture();
    w.players[1].sleeping = true;
    hold(&mut w);
    assert_eq!(w.players[1].assist_ticks, 0);
    let mut w = fixture();
    w.tick(&[
        Command::Assist { id: 2, target: 2 },
        Command::Input {
            id: 2,
            frame: frame(&w, 1),
            favour: 0,
        },
    ]);
    assert_eq!(w.players[1].assist_ticks, 0);
}
#[test]
fn assist_state_is_hashed_and_a_restore_drops_the_hold() {
    let mut w = fixture();
    let before = w.state_hash();
    w.players[0].assist_target = 2;
    assert_ne!(before, w.state_hash());
    let before = w.state_hash();
    w.players[1].assist_by = 1;
    assert_ne!(before, w.state_hash());
    let before = w.state_hash();
    w.players[1].assist_ticks = 50;
    assert_ne!(before, w.state_hash());
    let save = w.save_of(1).unwrap();
    w.tick(&[Command::JoinAs { id: 4, save }]);
    let p = w.players.iter().find(|p| p.id == 4).unwrap();
    assert_eq!((p.assist_target, p.assist_by, p.assist_ticks), (0, 0, 0));
}

#[test]
fn damage_on_the_finishing_tick_interrupts_before_recovery() {
    finishing_hit(false);
}

#[test]
fn healing_on_the_finishing_tick_cannot_hide_an_interrupting_hit() {
    finishing_hit(true);
}

fn finishing_hit(healing: bool) {
    let mut w = fixture();
    w.combat = sim_core::combat::CombatContent::probe_fixture();
    w.gather = sim_core::gather::GatherContent::probe_fixture();
    for _ in 1..ASSIST_TICKS {
        hold(&mut w);
    }
    assert_eq!(w.players[1].assist_ticks, ASSIST_TICKS - 1);
    let a = w.players[0].body;
    w.players[2].body = Body::at(
        42,
        &w.haven,
        a.qx as f32 * POS_XZ_Q,
        a.qz as f32 * POS_XZ_Q - 1.0,
    );
    w.players[2].inv[0] = sim_core::gather::ItemStack {
        item: 0,
        count: 1,
        cond: 400,
    };
    if healing {
        w.survival = sim_core::survival::SurvivalContent::probe_fixture();
        for p in w.players.iter_mut().take(3) {
            sim_core::survival::grant(&w.survival, p);
        }
        let p = &mut w.players[0];
        p.hp = 20;
        p.heal_rem = 80;
        p.heal_total = 80;
        p.heal_span = 1;
    }
    let hp = w.players[0].hp;
    w.tick(&[
        Command::Input {
            id: 1,
            frame: frame(&w, 0),
            favour: 0,
        },
        Command::Input {
            id: 3,
            frame: InputFrame {
                buttons: sim_core::input::BTN_PRIMARY,
                pitch: frame(&w, 2).pitch,
                ..Default::default()
            },
            favour: 0,
        },
    ]);
    assert!(
        w.events
            .entries()
            .iter()
            .any(|e| e.code == sim_core::world::EV_HURT && e.a == 1 && e.c > 0),
        "the setup must land a real hit"
    );
    if healing {
        assert!(w.players[0].hp >= hp, "the heal must hide the net hp loss");
    } else {
        assert!(w.players[0].hp < hp);
    }
    assert!(w.players[1].wounded);
    assert_eq!(w.players[1].assist_ticks, 0);
}

#[test]
fn helpers_cannot_pool_progress_or_inherit_another_hands_hold() {
    let mut w = fixture();
    for _ in 0..50 {
        w.tick(&[
            Command::Assist { id: 1, target: 2 },
            Command::Assist { id: 3, target: 2 },
            Command::Input {
                id: 1,
                frame: frame(&w, 0),
                favour: 0,
            },
            Command::Input {
                id: 3,
                frame: frame(&w, 2),
                favour: 0,
            },
        ]);
    }
    assert_eq!((w.players[1].assist_by, w.players[1].assist_ticks), (1, 50));
    let mut released = frame(&w, 0);
    released.buttons = 0;
    w.tick(&[
        Command::Input {
            id: 1,
            frame: released,
            favour: 0,
        },
        Command::Input {
            id: 3,
            frame: frame(&w, 2),
            favour: 0,
        },
    ]);
    assert_eq!((w.players[1].assist_by, w.players[1].assist_ticks), (3, 1));
}

#[test]
fn the_parity_probe_really_recovers_after_an_interruption() {
    for seed in [42, 0x0047_4154_4553] {
        let mut w = sim_core::probe::assist_probe_world(seed);
        let outcome = sim_core::probe::run_assist_probe(&mut w);
        assert_eq!(outcome >> 32, 1);
        assert_eq!(
            sim_core::probe::probe_assist(seed),
            outcome,
            "replay agrees"
        );
        assert_eq!(w.players[1].inv[0].count, 1, "hands keep the kit");
        assert!(!w.players[2].dead && !w.players[2].wounded);
        assert_eq!(w.players[2].inv[0], sim_core::gather::ItemStack::default());
    }
}

#[test]
fn a_world_save_cannot_resume_a_disconnected_hands_hold() {
    let mut w = fixture();
    for _ in 0..25 {
        hold(&mut w);
    }
    let until = w.players[1].wound_until;
    let mut bytes = vec![0; sim_core::worldsave::WORLD_SAVE_MAX_BYTES];
    let n = w.save_world(&mut bytes).unwrap();
    let mut back = fixture();
    back.load(&bytes[..n]).unwrap();
    for p in back.players.iter().filter(|p| p.active) {
        assert!(p.sleeping);
        assert_eq!((p.assist_target, p.assist_by, p.assist_ticks), (0, 0, 0));
    }
    assert_eq!(back.players[1].wound_until, until);
    assert!(back.players[1].wounded);
}
