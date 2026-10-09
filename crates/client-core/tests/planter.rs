//! Crops v1: a planter's beds reach the mirror on placement and on change.
use client_core::core::ClientCore;
use protocol::*;
use sim_core::deploy::DeployRec;

#[test]
fn a_planter_event_moves_its_beds_on_the_mirror() {
    let mut c = ClientCore::new(20260731, 7, 0);
    let mut buf = [0; MAX_EVENT_MSG_BYTES];
    let (cx, cz, level, loc) = (300, 301, 0, 0);
    let rec = DeployRec {
        cx,
        cz,
        level,
        loc,
        row: 2,
        grow: 0b01,
        ..DeployRec::default()
    };
    let n = encode_event_deploy_placed(&rec, &mut buf).unwrap();
    c.on_stream(&buf[..n]).unwrap();
    let grow = |c: &ClientCore| {
        c.deploys
            .entries()
            .iter()
            .find(|r| r.cx == cx)
            .map(|r| r.grow)
    };
    assert_eq!(grow(&c), Some(0b01), "the placed record carries its beds");
    let n = encode_event_planter(cx, cz, level, loc, 0b11_00_10_01, &mut buf).unwrap();
    c.on_stream(&buf[..n]).unwrap();
    assert_eq!(grow(&c), Some(0b11_00_10_01), "the event moved the beds");
}
