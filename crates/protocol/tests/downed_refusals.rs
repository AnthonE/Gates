//! Every new refusal reason survives the wire, including the widened fields.
use protocol::*;
use sim_core::{
    build, casino, craft, deploy, gather, inventory, monument, ranged, research, shop, survival,
    vend, works,
};

#[test]
fn downed_reasons_round_trip_on_their_own_lanes() {
    let mut buf = [0; 64];
    macro_rules! check {
        ($encode:ident ($($arg:expr),*), $expected:expr) => {{
            let n = $encode($($arg,)* &mut buf).unwrap();
            assert_eq!(decode_event(&buf[..n]).unwrap(), $expected);
        }};
    }
    macro_rules! reason {
        ($encode:ident, $variant:ident, $reason:expr) => {
            check!(
                $encode($reason as u8),
                EventMsg::$variant {
                    reason: $reason as u8
                }
            );
        };
    }
    reason!(
        encode_event_craft_refused,
        CraftRefused,
        craft::REFUSE_CR_DOWN
    );
    reason!(
        encode_event_build_refused,
        BuildRefused,
        build::REFUSE_B_DOWN
    );
    reason!(
        encode_event_deploy_refused,
        DeployRefused,
        deploy::REFUSE_D_DOWN
    );
    reason!(
        encode_event_consume_refused,
        ConsumeRefused,
        survival::REFUSE_C_DOWN
    );
    reason!(
        encode_event_research_refused,
        ResearchRefused,
        research::REFUSE_R_DOWN
    );
    check!(
        encode_event_move_refused(inventory::REFUSE_M_DOWN as u8, 0, 2, 0, 3),
        EventMsg::MoveRefused {
            reason: inventory::REFUSE_M_DOWN as u8,
            from_kind: 0,
            from_slot: 2,
            to_kind: 0,
            to_slot: 3
        }
    );
    check!(
        encode_event_gather_refused(gather::NO_ITEM, gather::REFUSE_G_DOWN as u8),
        EventMsg::GatherRefused {
            item: gather::NO_ITEM,
            reason: gather::REFUSE_G_DOWN as u8
        }
    );
    check!(
        encode_event_reload_refused(3, ranged::REFUSE_RL_DOWN as u8, 2, 6),
        EventMsg::ReloadRefused {
            item: 3,
            reason: ranged::REFUSE_RL_DOWN as u8,
            loaded: 2,
            ceiling: 6
        }
    );
    check!(
        encode_event_reload_refused(gather::NO_ITEM, ranged::REFUSE_RL_DOWN as u8, 0, 0),
        EventMsg::ReloadRefused {
            item: gather::NO_ITEM,
            reason: ranged::REFUSE_RL_DOWN as u8,
            loaded: 0,
            ceiling: 0
        }
    );
    check!(
        encode_event_vend_refused(vend::REFUSE_V_DOWN as u8, 3),
        EventMsg::VendRefused {
            code: vend::REFUSE_V_DOWN as u8,
            offer: 3
        }
    );
    check!(
        encode_event_swipe_refused(monument::REFUSE_S_DOWN as u8, 1),
        EventMsg::SwipeRefused {
            code: monument::REFUSE_S_DOWN as u8,
            door: 1
        }
    );
    check!(
        encode_event_arc_refused(works::REFUSE_A_DOWN as u8, works::OP_FUEL, 2),
        EventMsg::ArcRefused {
            code: works::REFUSE_A_DOWN as u8,
            op: works::OP_FUEL,
            target: 2
        }
    );
    check!(
        encode_event_shop_result(123, shop::SHOP_OP_CLEAR, 1, 0, shop::REFUSE_SH_DOWN as u8),
        EventMsg::ShopResult {
            key: 123,
            op: shop::SHOP_OP_CLEAR,
            row: 1,
            times: 0,
            code: shop::REFUSE_SH_DOWN as u8
        }
    );
    check!(
        encode_event_wheel_bet(2, 7, casino::REFUSE_CA_DOWN as u8),
        EventMsg::WheelBet {
            colour: 2,
            stake: 7,
            code: casino::REFUSE_CA_DOWN as u8
        }
    );
    check!(
        encode_event_slots(1, 2, casino::REFUSE_CA_DOWN as u8, 0, 0),
        EventMsg::Slots {
            machine: 1,
            mult: 2,
            code: casino::REFUSE_CA_DOWN as u8,
            reels: 0,
            pay: 0
        }
    );
}
