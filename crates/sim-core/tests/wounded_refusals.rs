//! A rejected hand command must answer without doing any of its work.
use sim_core::world::*;
use sim_core::{
    build, casino, craft, deploy, gather, inventory, monument, ranged, research, shop, survival,
    vend, works,
};

fn world() -> World {
    let mut w = World::new(20260913);
    w.tick(&[Command::Join { id: 1 }]);
    w.players[0].wounded = true;
    w.players[0].wound_until = 100_000;
    w
}

fn commands() -> Vec<(Command, u8, u32, u32)> {
    vec![
        (
            Command::Treat {
                id: 1,
                slot: 0,
                target: 0,
            },
            EV_CONSUME_REFUSED,
            survival::REFUSE_C_DOWN,
            0,
        ),
        (
            Command::Consume { id: 1, slot: 0 },
            EV_CONSUME_REFUSED,
            survival::REFUSE_C_DOWN,
            0,
        ),
        (
            Command::Drink { id: 1 },
            EV_CONSUME_REFUSED,
            survival::REFUSE_C_DOWN,
            0,
        ),
        (
            Command::Craft {
                id: 1,
                recipe: 0,
                count: 0,
                skin: 0,
            },
            EV_CRAFT_REFUSED,
            craft::REFUSE_CR_DOWN,
            0,
        ),
        (
            Command::Reskin {
                id: 1,
                slot: 0,
                skin: 0,
            },
            EV_CRAFT_REFUSED,
            craft::REFUSE_CR_DOWN,
            0,
        ),
        (
            Command::CraftFastTrack {
                id: 1,
                index: 0,
                recipe: 0,
            },
            EV_CRAFT_REFUSED,
            craft::REFUSE_CR_DOWN,
            0,
        ),
        (
            Command::CraftCancel { id: 1, index: 0 },
            EV_CRAFT_REFUSED,
            craft::REFUSE_CR_DOWN,
            0,
        ),
        (
            Command::Research { id: 1, slot: 0 },
            EV_RESEARCH_REFUSED,
            research::REFUSE_R_DOWN,
            0,
        ),
        (
            Command::Unlock { id: 1, recipe: 0 },
            EV_RESEARCH_REFUSED,
            research::REFUSE_R_DOWN,
            0,
        ),
        (
            Command::Place {
                id: 1,
                row: 0,
                cx: 0,
                cz: 0,
                level: 0,
                loc: 0,
                freehand: false,
                plate: 0,
            },
            EV_BUILD_REFUSED,
            build::REFUSE_B_DOWN,
            0,
        ),
        (
            Command::PlaceAt {
                id: 1,
                row: 0,
                bld: 0,
                level: 0,
                sock: Default::default(),
            },
            EV_BUILD_REFUSED,
            build::REFUSE_B_DOWN,
            0,
        ),
        (
            Command::PlaceFree {
                id: 1,
                row: 0,
                x_cm: 0,
                z_cm: 0,
                yaw: 0,
                tri: false,
                plate: 0,
            },
            EV_BUILD_REFUSED,
            build::REFUSE_B_DOWN,
            0,
        ),
        (
            Command::Rotate {
                id: 1,
                cx: 0,
                cz: 0,
                level: 0,
                loc: 0,
            },
            EV_BUILD_REFUSED,
            build::REFUSE_B_DOWN,
            0,
        ),
        (
            Command::Upgrade {
                id: 1,
                cx: 0,
                cz: 0,
                level: 0,
                loc: 0,
                material: 0,
            },
            EV_BUILD_REFUSED,
            build::REFUSE_B_DOWN,
            0,
        ),
        (
            Command::Repair {
                id: 1,
                deploy: false,
                cx: 0,
                cz: 0,
                level: 0,
                loc: 0,
            },
            EV_BUILD_REFUSED,
            build::REFUSE_B_DOWN,
            0,
        ),
        (
            Command::Throw {
                id: 1,
                deploy: false,
                cx: 0,
                cz: 0,
                level: 0,
                loc: 0,
            },
            EV_BUILD_REFUSED,
            build::REFUSE_B_DOWN,
            0,
        ),
        (
            Command::Demolish {
                id: 1,
                deploy: false,
                cx: 0,
                cz: 0,
                level: 0,
                loc: 0,
            },
            EV_BUILD_REFUSED,
            build::REFUSE_B_DOWN,
            0,
        ),
        (
            Command::Demolish {
                id: 1,
                deploy: true,
                cx: 0,
                cz: 0,
                level: 0,
                loc: 0,
            },
            EV_DEPLOY_REFUSED,
            deploy::REFUSE_D_DOWN,
            0,
        ),
        (
            Command::PlaceDeploy {
                id: 1,
                row: 0,
                cx: 0,
                cz: 0,
                level: 0,
                loc: 0,
                pose: Default::default(),
            },
            EV_DEPLOY_REFUSED,
            deploy::REFUSE_D_DOWN,
            0,
        ),
        (
            Command::Feed {
                id: 1,
                cx: 0,
                cz: 0,
                level: 0,
            },
            EV_DEPLOY_REFUSED,
            deploy::REFUSE_D_DOWN,
            0,
        ),
        (
            Command::TakeStock {
                id: 1,
                cx: 0,
                cz: 0,
                level: 0,
                row: 0,
            },
            EV_DEPLOY_REFUSED,
            deploy::REFUSE_D_DOWN,
            0,
        ),
        (
            Command::Access {
                id: 1,
                cx: 0,
                cz: 0,
                level: 0,
                loc: 0,
                op: 0,
                code: 0,
            },
            EV_DEPLOY_REFUSED,
            deploy::REFUSE_D_DOWN,
            0,
        ),
        (
            Command::Wire {
                id: 1,
                wire: Default::default(),
            },
            EV_DEPLOY_REFUSED,
            deploy::REFUSE_D_DOWN,
            0,
        ),
        (
            Command::Unwire { id: 1, key: 0 },
            EV_DEPLOY_REFUSED,
            deploy::REFUSE_D_DOWN,
            0,
        ),
        (
            Command::Plant {
                id: 1,
                slot: 0,
                host: 0,
                spot: 0,
                qx: 0,
                qz: 0,
            },
            EV_DEPLOY_REFUSED,
            deploy::REFUSE_D_DOWN,
            0,
        ),
        (
            Command::Tend {
                id: 1,
                plant: 0,
                act: 0,
            },
            EV_DEPLOY_REFUSED,
            deploy::REFUSE_D_DOWN,
            0,
        ),
        (
            Command::Give {
                id: 1,
                slot: 2,
                count: 0,
                target: 0,
            },
            EV_MOVE_REFUSED,
            inventory::REFUSE_M_DOWN,
            inventory::addr(inventory::CONT_SELF, 2, inventory::CONT_SELF, 2),
        ),
        (
            Command::Drop {
                id: 1,
                slot: 2,
                count: 0,
            },
            EV_MOVE_REFUSED,
            inventory::REFUSE_M_DOWN,
            inventory::addr(inventory::CONT_SELF, 2, inventory::CONT_SELF, 2),
        ),
        (
            Command::Loot { id: 1 },
            EV_MOVE_REFUSED,
            inventory::REFUSE_M_DOWN,
            0,
        ),
        (
            Command::Pickup { id: 1, target: 0 },
            EV_MOVE_REFUSED,
            inventory::REFUSE_M_DOWN,
            0,
        ),
        (
            Command::OpenWorldCont { id: 1, cont: 0 },
            EV_MOVE_REFUSED,
            inventory::REFUSE_M_DOWN,
            0,
        ),
        (
            Command::Move {
                id: 1,
                cont: 0,
                from_kind: inventory::CONT_SELF,
                from_slot: 2,
                to_kind: inventory::CONT_SELF,
                to_slot: 3,
                count: 1,
            },
            EV_MOVE_REFUSED,
            inventory::REFUSE_M_DOWN,
            inventory::addr(inventory::CONT_SELF, 2, inventory::CONT_SELF, 3),
        ),
        (
            Command::Pick { id: 1, cell: 0 },
            EV_GATHER_REFUSED,
            (gather::NO_ITEM as u32) << 16 | gather::REFUSE_G_DOWN,
            0,
        ),
        (
            Command::Reload { id: 1 },
            EV_RELOAD_REFUSED,
            (gather::NO_ITEM as u32) << 16 | ranged::REFUSE_RL_DOWN,
            0,
        ),
        (
            Command::Unload { id: 1 },
            EV_RELOAD_REFUSED,
            (gather::NO_ITEM as u32) << 16 | ranged::REFUSE_RL_DOWN,
            0,
        ),
        (
            Command::Attach { id: 1, slot: 0 },
            EV_RELOAD_REFUSED,
            (gather::NO_ITEM as u32) << 16 | ranged::REFUSE_RL_DOWN,
            0,
        ),
        (
            Command::Detach { id: 1, bit: 0 },
            EV_RELOAD_REFUSED,
            (gather::NO_ITEM as u32) << 16 | ranged::REFUSE_RL_DOWN,
            0,
        ),
        (
            Command::Vend {
                id: 1,
                offer: 3,
                times: 0,
            },
            EV_VEND_REFUSED,
            vend::REFUSE_V_DOWN,
            3,
        ),
        (
            Command::Wheel {
                id: 1,
                colour: 2,
                stake: 7,
            },
            EV_WHEEL_REFUSED,
            casino::REFUSE_CA_DOWN,
            (2 << 16) | 7,
        ),
        (
            Command::Slots {
                id: 1,
                machine: 1,
                mult: 2,
            },
            EV_SLOTS_REFUSED,
            casino::REFUSE_CA_DOWN,
            (1 << 8) | 2,
        ),
        (
            Command::Shop {
                id: 1,
                key: 123,
                act: sim_core::shop::ShopAct::Clear { row: 0 },
            },
            EV_SHOP_REFUSED,
            123,
            (shop::REFUSE_SH_DOWN << 24) | (shop::SHOP_OP_CLEAR as u32) << 16,
        ),
        (
            Command::Extract {
                id: 1,
                target: 2,
                max: 0,
            },
            EV_ARC_REFUSED,
            works::REFUSE_A_DOWN,
            (works::OP_EXTRACT as u32) << 8 | 2,
        ),
        (
            Command::Arc {
                id: 1,
                op: works::OP_FUEL,
                target: 2,
                arg: 0,
            },
            EV_ARC_REFUSED,
            works::REFUSE_A_DOWN,
            (works::OP_FUEL as u32) << 8 | 2,
        ),
        (
            Command::Swipe { id: 1, door: 1 },
            EV_SWIPE_REFUSED,
            monument::REFUSE_S_DOWN,
            1,
        ),
    ]
}

#[test]
fn each_downed_hand_command_answers_once_without_mutating_the_world() {
    let mut w = world();
    let mut idle = world();
    for (cmd, code, b, c) in commands() {
        w.tick(&[cmd]);
        idle.tick(&[]);
        let answers: Vec<_> = w
            .events
            .entries()
            .iter()
            .filter(|e| e.code == code)
            .collect();
        assert_eq!(answers.len(), 1, "{cmd:?}");
        assert_eq!(
            (answers[0].a, answers[0].b, answers[0].c),
            (1, b, c),
            "{cmd:?}"
        );
        assert_eq!(w.state_hash(), idle.state_hash(), "{cmd:?} changed state");
    }
}

#[test]
fn missing_and_dead_players_do_not_receive_downed_refusals() {
    let mut w = world();
    w.players[0].dead = true;
    for (cmd, code, _, _) in commands() {
        w.tick(&[cmd]);
        assert!(
            !w.events.entries().iter().any(|e| e.code == code),
            "{cmd:?}"
        );
    }
    w.players[0].active = false;
    for (cmd, code, _, _) in commands() {
        w.tick(&[cmd]);
        assert!(
            !w.events.entries().iter().any(|e| e.code == code),
            "{cmd:?}"
        );
    }
}

#[test]
fn a_standing_body_still_reaches_the_verbs_own_validation() {
    let mut w = world();
    w.players[0].wounded = false;
    w.tick(&[Command::Craft {
        id: 1,
        recipe: u16::MAX,
        count: 1,
        skin: 0,
    }]);
    let e = w
        .events
        .entries()
        .iter()
        .find(|e| e.code == EV_CRAFT_REFUSED)
        .unwrap();
    assert_eq!(e.b, craft::REFUSE_CR_RECIPE);
}

#[test]
fn downed_reload_keeps_the_held_gun_and_magazine_in_its_answer() {
    let mut w = world();
    w.combat = sim_core::combat::CombatContent::probe_fixture();
    w.players[0].inv[0] = gather::ItemStack {
        item: 6,
        count: 1,
        ..Default::default()
    };
    sim_core::combat::set_held_gun(
        &mut w.players[0],
        sim_core::combat::GunState {
            loaded: 2,
            round: 7,
            ..Default::default()
        },
    );
    w.tick(&[Command::Reload { id: 1 }]);
    let e = w
        .events
        .entries()
        .iter()
        .find(|e| e.code == EV_RELOAD_REFUSED)
        .unwrap();
    assert_eq!(
        (e.a, e.b, e.c),
        (1, (6 << 16) | ranged::REFUSE_RL_DOWN, (2 << 16) | 6)
    );
    assert_eq!(w.players[0].gun().loaded, 2);
}

#[test]
fn downed_reload_does_not_read_a_vessels_data_as_a_magazine() {
    let mut w = world();
    w.combat = sim_core::combat::CombatContent::probe_fixture();
    let water = survival::Water {
        ml: 1000,
        salt: true,
    }
    .data();
    w.players[0].inv[0] = gather::ItemStack {
        item: 8,
        count: 1,
        data: water,
        ..Default::default()
    };
    w.tick(&[Command::Reload { id: 1 }]);
    let e = w
        .events
        .entries()
        .iter()
        .find(|e| e.code == EV_RELOAD_REFUSED)
        .unwrap();
    assert_eq!((e.a, e.b, e.c), (1, (8 << 16) | ranged::REFUSE_RL_DOWN, 0));
    assert_eq!(w.players[0].inv[0].data, water);
}
