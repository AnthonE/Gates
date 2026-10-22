//! Farming v2: the plant mirror fills from a reset batch, replaces by id,
//! drops what the server says is gone, and raises its flag each time.
use client_core::core::{ClientCore, APPLIED2_PLANTS};
use protocol::*;

fn plant(id: u32, stage: u8) -> WirePlant {
    WirePlant {
        id,
        look: sim_core::farm::LOOK_CORN,
        stage,
        spot: (id % 6) as u8,
        genes: 0,
        band: 10,
        dry: false,
        qx: 1_000 + id as i32,
        qy: 200,
        qz: 3_000,
    }
}

#[test]
fn the_plant_set_follows_sync_upsert_and_gone() {
    let mut c = ClientCore::new(20260731, 7, 0);
    let mut buf = [0; MAX_EVENT_MSG_BYTES];

    let n =
        encode_event_plant_sync(true, &[plant(1, 0), plant(2, 3), plant(3, 6)], &mut buf).unwrap();
    c.on_stream(&buf[..n]).unwrap();
    assert_eq!(c.applied2() & APPLIED2_PLANTS, APPLIED2_PLANTS);
    assert_eq!(c.plants.len(), 3, "a reset batch fills the set");

    // An upsert replaces by id rather than adding a second record.
    let n = encode_event_plant(&plant(2, 4), &mut buf).unwrap();
    c.on_stream(&buf[..n]).unwrap();
    assert_eq!(c.applied2() & APPLIED2_PLANTS, APPLIED2_PLANTS);
    assert_eq!(c.plants.len(), 3);
    assert_eq!(c.plants.find(2).map(|p| p.stage), Some(4));

    // The same record again changes nothing and raises nothing.
    c.on_stream(&buf[..n]).unwrap();
    assert_eq!(c.applied2() & APPLIED2_PLANTS, 0);

    let n = encode_event_plant_gone(1, sim_core::farm::GONE_PICKED, &mut buf).unwrap();
    c.on_stream(&buf[..n]).unwrap();
    assert_eq!(c.applied2() & APPLIED2_PLANTS, APPLIED2_PLANTS);
    assert!(
        c.plants.find(1).is_none() && c.plants.len() == 2,
        "gone removes"
    );

    // An empty reset is the message that nothing stands now.
    let n = encode_event_plant_sync(true, &[], &mut buf).unwrap();
    c.on_stream(&buf[..n]).unwrap();
    assert_eq!(c.applied2() & APPLIED2_PLANTS, APPLIED2_PLANTS);
    assert!(c.plants.is_empty());
}
