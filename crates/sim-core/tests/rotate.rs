//! Hammer turns traverse the World command path, survive saves, and replay.
//! The unit tests in build.rs own permission, grace and geometry boundaries.

use sim_core::build::{self, PieceRec, LOC_EDGE_XLO, STAIR_LOCS};
use sim_core::world::{Command, World, EV_PIECE_PLACED};

fn fixture() -> Box<World> {
    let w = Box::new(sim_core::probe::rotation_probe_world());
    assert_eq!(
        w.pieces.len(),
        4,
        "the fixture must build its complete base"
    );
    w
}

fn stairs(w: &World) -> PieceRec {
    *w.pieces
        .entries()
        .iter()
        .find(|r| build::is_stair_loc(r.loc))
        .unwrap()
}

fn rotate(rec: PieceRec, id: u32, loc: u8) -> Command {
    Command::Rotate {
        id,
        cx: rec.cx,
        cz: rec.cz,
        level: rec.level,
        loc,
    }
}

#[test]
fn rotation_replays_and_survives_restart_with_its_original_clocks() {
    let mut a = fixture();
    let mut b = fixture();
    let original = stairs(&a);
    let placed = a.pieces.placed().to_vec();
    let wall = *a
        .pieces
        .find(original.cx, original.cz, 0, LOC_EDGE_XLO)
        .unwrap();
    for (turn, loc) in STAIR_LOCS.into_iter().take(3).enumerate() {
        let commands = [rotate(original, 1, loc), rotate(wall, 1, LOC_EDGE_XLO)];
        a.tick(&commands);
        b.tick(&commands);
        assert_eq!(a.state_hash(), b.state_hash());
        assert_eq!(
            a.pieces.cols().get(original.cx, original.cz),
            b.pieces.cols().get(original.cx, original.cz)
        );
        assert_eq!(
            stairs(&a),
            PieceRec {
                loc: STAIR_LOCS[turn + 1],
                ..original
            }
        );
        assert_eq!(a.pieces.placed(), placed);
        assert_eq!(
            a.events
                .entries()
                .iter()
                .filter(|e| e.code == EV_PIECE_PLACED)
                .count(),
            2
        );
    }
    assert_eq!(
        a.pieces
            .find(wall.cx, wall.cz, wall.level, wall.loc)
            .unwrap()
            .facing,
        wall.facing ^ 1
    );
    // World saves normalize connected players to sleepers. Save an actual
    // sleeper so state_hash can assert a lossless whole-world round trip.
    a.tick(&[Command::Leave { id: 1 }]);
    let mut blob = vec![0; sim_core::worldsave::WORLD_SAVE_MAX_BYTES];
    let n = a.save_world(&mut blob).unwrap();
    b.load(&blob[..n]).unwrap();
    assert_eq!(a.state_hash(), b.state_hash());
    assert_eq!(b.pieces.placed(), placed);
    assert_eq!(stairs(&b), stairs(&a));
    assert_eq!(
        a.pieces.cols().get(original.cx, original.cz),
        b.pieces.cols().get(original.cx, original.cz)
    );
    let base = build::column_floor_y(b.seed, &b.haven, original.cx, original.cz, original.plate);
    let x = original.cx as f32 * build::BUILD_CELL_M + 0.5;
    let z = original.cz as f32 * build::BUILD_CELL_M + 1.0;
    assert_eq!(
        sim_core::collide::piece_ground(
            b.seed,
            &b.haven,
            b.pieces.cols(),
            x,
            z,
            base + build::LEVEL_H_M
        ),
        base + 2.5,
        "the restarted world collides with the final stair direction"
    );
}

#[test]
fn rotation_requires_a_live_unwounded_actor_and_the_current_address() {
    let mut w = fixture();
    let rec = stairs(&w);
    for (id, dead, wounded) in [(99, false, false), (1, true, false), (1, false, true)] {
        w.players[0].dead = dead;
        w.players[0].wounded = wounded;
        w.players[0].wound_until = w.tick + 100;
        w.tick(&[rotate(rec, id, rec.loc)]);
        assert_eq!(stairs(&w), rec);
        assert!(!w.events.entries().iter().any(|e| e.code == EV_PIECE_PLACED));
    }
    w.players[0].wounded = false;
    w.tick(&[rotate(rec, 1, rec.loc)]);
    assert_eq!(stairs(&w).loc, STAIR_LOCS[1]);
    // A stale target cannot turn whatever was moved into the next socket.
    w.tick(&[rotate(rec, 1, rec.loc)]);
    assert_eq!(stairs(&w).loc, STAIR_LOCS[1]);
    assert!(w
        .events
        .entries()
        .iter()
        .any(|e| e.code == sim_core::world::EV_BUILD_REFUSED && e.b == build::REFUSE_B_SPOT));
}

#[test]
fn several_stair_turns_in_one_tick_keep_one_piece_and_one_collision_mask() {
    let mut w = fixture();
    let rec = stairs(&w);
    let placed = w.pieces.placed().to_vec();
    w.tick(&STAIR_LOCS.map(|loc| rotate(rec, 1, loc)));
    assert_eq!(stairs(&w), rec);
    assert_eq!(w.pieces.len(), 4);
    assert_eq!(w.pieces.placed(), placed);
    assert_eq!(
        w.pieces.cols().get(rec.cx, rec.cz).stair_masks(),
        [1, 0, 0, 0]
    );
    assert_eq!(
        w.events
            .entries()
            .iter()
            .filter(|e| e.code == EV_PIECE_PLACED)
            .count(),
        4
    );
}
