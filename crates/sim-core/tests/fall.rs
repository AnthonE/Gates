//! Fall damage: a short drop is free, a long one hurts, a very long one
//! puts the body down with the fall's own cause.

use sim_core::combat::CombatContent;
use sim_core::movement::POS_Y_Q;
use sim_core::world::{Command, World};

fn dropped(metres: f32) -> World {
    let mut w = World::new(11);
    w.combat = CombatContent::probe_fixture();
    w.dev_spawn = Some(w.spawn_pos(1));
    w.tick(&[Command::Join { id: 1 }]);
    w.tick(&[]);
    let b = &mut w.players[0].body;
    assert!(b.grounded, "the spawn stands on the ground");
    b.qy += (metres / POS_Y_Q) as i32;
    b.grounded = false;
    b.qvy = 0;
    for _ in 0..(10 * 30) {
        w.tick(&[]);
        if w.players[0].body.grounded {
            break;
        }
    }
    w
}

#[test]
fn a_short_drop_is_free() {
    let w = dropped(3.0);
    assert!(w.players[0].body.grounded);
    assert_eq!(w.players[0].hp, 100);
}

#[test]
fn a_long_drop_hurts_and_a_longer_one_downs() {
    let w = dropped(8.0);
    let hp = w.players[0].hp;
    assert!(hp < 100 && hp > 0, "an 8 m drop left {hp} hp");

    let w = dropped(30.0);
    let p = &w.players[0];
    assert!(p.wounded || p.dead, "a 30 m drop left the body standing");
}
