//! Bullets that fly (`sim_core::bullet`): Rust's travel time and drop, the
//! shooter's lag-comp favour kept for the whole flight, pellets of one shell
//! landing as one blow, falloff over the distance flown, the shooter's share
//! of the store, the exact cadence, and `EV_SHOT`'s bullet reading.

#![allow(clippy::disallowed_macros)]

use sim_core::bullet::{self, Bullets};
use sim_core::collide::ColIndex;
use sim_core::combat::{AmmoDef, CombatContent, Gun, RangedDef, RoundBlast};
use sim_core::gather::{ItemStack, NO_ITEM};
use sim_core::input::{InputFrame, BTN_PRIMARY};
use sim_core::limits::{MAX_ARROWS, MAX_BULLETS_PER_OWNER, MAX_PLAYERS};
use sim_core::mech::{DrumShot, Drums};
use sim_core::mob::{MobContent, Mobs};
use sim_core::movement::{Body, POS_XZ_Q, POS_Y_Q};
use sim_core::occupy::Scratch;
use sim_core::ranged::{self, Arrows, Kill, ARROW_EYE_MM};
use sim_core::rewind::Rewind;
use sim_core::terrain;
use sim_core::world::{
    shot_bullet_c, EventQueue, Player, EV_HIT, EV_SHOT, SHOT_BULLET, SHOT_DROP_BITS,
    SHOT_REACH_BITS, SHOT_REACH_UNIT_MM,
};

const SEED: u64 = 20261023;
const GUN: u16 = 5;
const ROUND: u16 = 6;
const LEVEL: u8 = 128;
/// Rust's 5.56: 375 m/s is 12 500 mm a tick at 30 Hz.
const SPEED: u16 = 12_500;
/// Full gravity, 9.9 m/s², the bake's nearest to Rust's 9.81.
const DROP: u16 = 11;

fn hv() -> &'static terrain::Haven {
    use std::sync::OnceLock;
    static H: OnceLock<terrain::Haven> = OnceLock::new();
    H.get_or_init(|| terrain::haven(SEED))
}

fn fixture(gun: Gun, damage: u16, rate_sub: u32) -> CombatContent {
    let mut c = CombatContent::EMPTY;
    c.player_hp = 100;
    c.ranged[GUN as usize] = RangedDef {
        damage,
        ammo: [ROUND, NO_ITEM, NO_ITEM, NO_ITEM],
        rate_ticks: 4,
        rate_sub,
        hitscan: true,
        range_mm: 170_000,
        structure: 0,
        head_pct: 200,
        limb_pct: 50,
        magazine: 30,
        reload_ticks: 100,
        mag_slot: 0,
        draw_ticks: 0,
        gun,
    };
    c.ammo[ROUND as usize] = AmmoDef {
        speed_mmpt: SPEED,
        drop_mmpt2: DROP,
        damage_pct: 100,
        fire_ticks: [0, 0],
        blast: RoundBlast::NONE,
        ..Default::default()
    };
    c
}

fn body(id: u32, x: f32, y: f32, z: f32) -> Player {
    let mut p = Player {
        id,
        active: true,
        hp: 100,
        hp_max: 100,
        ..Player::default()
    };
    p.body = Body {
        qx: (x / POS_XZ_Q) as i32,
        qy: (y / POS_Y_Q) as i32,
        qz: (z / POS_XZ_Q) as i32,
        grounded: true,
        ..Body::default()
    };
    p
}

/// Feet that put a body's chest on the shooter's eye line, `drop_m` lower.
fn chest_on_line(drop_m: f32) -> f32 {
    400.0 + ARROW_EYE_MM as f32 / 1000.0 - 1.2 - drop_m
}

/// A shooter at the origin facing +Z with a loaded gun, holding the trigger,
/// and a target `d` metres down the line `drop_m` under it.
fn range(d: f32, drop_m: f32) -> Box<[Player; MAX_PLAYERS]> {
    let mut ps = Box::new([Player::default(); MAX_PLAYERS]);
    let mut s = body(1, 0.0, 400.0, 0.0);
    s.inv[0] = ItemStack {
        item: GUN,
        count: 1,
        cond: 0,
        skin: 0,
        ..ItemStack::EMPTY
    };
    s.set_gun(|g| g.loaded = 30);
    s.set_gun(|g| g.round = ROUND);
    s.frame = InputFrame {
        seq: 1,
        buttons: BTN_PRIMARY,
        yaw: 0,
        // Byte 128 is a hair over level (`aim_dir`: half a step up); the
        // fine pitch takes it back to dead level so drop is all that moves
        // the round off the line.
        pitch: LEVEL,
        pitch_fine: -128,
        sel: 0,
        ..InputFrame::default()
    };
    ps[0] = s;
    if d > 0.0 {
        ps[1] = body(2, 0.0, chest_on_line(drop_m), d);
    }
    ps
}

/// One tick: the trigger pass at `back` ticks of favour, then one step of
/// every round in the air. Returns the tick's events.
fn tick(
    t: u64,
    cc: &CombatContent,
    ps: &mut [Player; MAX_PLAYERS],
    bullets: &mut Bullets,
    rewind: &Rewind,
    back: u8,
) -> EventQueue {
    let mut events = EventQueue::default();
    let mut kills = [Kill::default(); MAX_ARROWS];
    let mut chips = [ranged::Chip::default(); MAX_ARROWS];
    let mut sc = Scratch::barren();
    let mobs = Mobs::new(SEED, hv());
    let mut hits = [ranged::MobShot::default(); MAX_ARROWS];
    let mut struck = [DrumShot::default(); MAX_ARROWS];
    let mut quarry = ranged::Quarry {
        mobs: &mobs,
        mc: &MobContent::EMPTY,
        hits: &mut hits,
        n: 0,
        drums: &Drums::NONE,
        struck: &mut struck,
        nd: 0,
    };
    let mut favour = [0u8; MAX_PLAYERS];
    favour[0] = back;
    ranged::hitscan_hunting(
        SEED,
        SEED,
        hv(),
        &ColIndex::new(),
        &mut sc.occupants(),
        t,
        rewind,
        &favour,
        cc,
        ps,
        &mut events,
        &mut kills,
        &mut chips,
        bullets,
        &mut quarry,
    );
    let mut arrows = Arrows::new();
    bullet::step(
        SEED,
        t,
        hv(),
        &ColIndex::new(),
        &mut sc.occupants(),
        rewind,
        cc,
        bullets,
        arrows.lit_mut(),
        ps,
        &mut events,
        &mut kills,
        &mut chips,
        None,
    );
    events
}

fn hits(ev: &EventQueue) -> Vec<u32> {
    ev.entries()
        .iter()
        .filter(|e| e.code == EV_HIT)
        .map(|e| e.c & 0xFFFF)
        .collect()
}

/// Fire once on tick 0 and let go; fly until the store is empty or `ticks`
/// pass. Returns the tick each `EV_HIT` landed on.
fn one_shot(cc: &CombatContent, ps: &mut [Player; MAX_PLAYERS], ticks: u64) -> Vec<(u64, u32)> {
    let mut bullets = Bullets::new();
    let rw = Rewind::new();
    let mut landed = Vec::new();
    for t in 0..ticks {
        let ev = tick(t, cc, ps, &mut bullets, &rw, 0);
        ps[0].frame.buttons = 0;
        landed.extend(hits(&ev).into_iter().map(|h| (t, h)));
        if t > 0 && bullets.is_empty() {
            break;
        }
    }
    landed
}

/// **Travel time.** 100 m at 12.5 m a tick is eight ticks of flight: the
/// round crosses the body on the tick its segment reaches 100 m, not on the
/// trigger's.
#[test]
fn a_bullet_takes_its_flight_to_arrive() {
    let cc = fixture(Gun::NONE, 30, 0);
    let mut ps = range(100.0, 0.0);
    let landed = one_shot(&cc, &mut ps, 40);
    assert_eq!(landed.len(), 1, "one hit: {landed:?}");
    assert_eq!(landed[0].0, 7, "100 m lands on the eighth tick of flight");
    // Close enough to be inside the first tick's segment, it is as quick as
    // a hitscan was.
    let mut ps = range(8.0, 0.0);
    let landed = one_shot(&cc, &mut ps, 40);
    assert_eq!(landed.first().map(|h| h.0), Some(0), "{landed:?}");
}

/// **Drop.** A level round falls `drop × n(n+1)/2` millimetres in `n` ticks —
/// the arrow's integration — so it is under the line at range, and a body
/// standing where the drop puts the round is hit while one on the line is not.
#[test]
fn drop_pulls_a_level_round_under_the_line() {
    let cc = fixture(Gun::NONE, 30, 0);
    let mut ps = range(0.0, 0.0);
    let mut bullets = Bullets::new();
    let rw = Rewind::new();
    let eye = ps[0].body.qy * (POS_Y_Q * 1000.0) as i32 + ARROW_EYE_MM;
    let n = 10u64;
    for t in 0..n {
        tick(t, &cc, &mut ps, &mut bullets, &rw, 0);
        ps[0].frame.buttons = 0;
    }
    let b = bullets.entries().next().expect("still flying at 125 m");
    let fell = eye - b.qy;
    let want = i32::from(DROP) * (n * (n + 1) / 2) as i32;
    assert!((fell - want).abs() <= 2, "fell {fell} mm, want {want}");

    // 160 m out the round has fallen a metre: a body standing on the line
    // (feet 10 cm under it) is over the round, one a metre lower is in it —
    // and at 20 m, where the round has barely fallen, the first is hit.
    let mut on_line = range(160.0, -1.1);
    assert!(one_shot(&cc, &mut on_line, 40).is_empty(), "flew under");
    let mut low = range(160.0, 1.0);
    assert_eq!(one_shot(&cc, &mut low, 40).len(), 1, "met the drop");
    let mut near = range(20.0, -1.1);
    assert_eq!(one_shot(&cc, &mut near, 40).len(), 1, "no drop to speak of");
}

/// **Pellets of one shell are one blow**: eight pellets that reach one body
/// on one tick are one `EV_HIT` for the total.
#[test]
fn pellets_of_one_shell_land_as_one_blow() {
    let gun = Gun {
        pellets: 8,
        ..Gun::NONE
    };
    let cc = fixture(gun, 10, 0);
    let mut ps = range(10.0, 0.0);
    let landed = one_shot(&cc, &mut ps, 10);
    assert_eq!(landed.len(), 1, "{landed:?}");
    // Limb-or-chest crossing for each pellet, summed: 8 × 10 at the chest.
    assert_eq!(landed[0].1, 80, "{landed:?}");
}

/// **Falloff is the distance flown**: Rust's 5.56, whole to 20 m and 80 %
/// from 100 m, read where the round crosses the body.
#[test]
fn damage_falls_off_over_the_distance_flown() {
    let gun = Gun {
        falloff_mm: [20_000, 100_000],
        falloff_pct: 80,
        ..Gun::NONE
    };
    let cc = fixture(gun, 50, 0);
    let mut near = range(10.0, 0.0);
    assert_eq!(one_shot(&cc, &mut near, 40)[0].1, 50);
    let mut far = range(120.0, 0.5);
    assert_eq!(one_shot(&cc, &mut far, 40)[0].1, 40);
}

/// **Lag compensation for the whole flight.** The target stood in the line
/// on every recorded tick and has stepped out of it now. Fired at four ticks
/// of favour, the round meets it where the shooter's screen had it on every
/// tick of flight; at none, it flies past the body that is really there.
#[test]
fn a_bullet_is_judged_where_the_shooter_saw_the_target() {
    let cc = fixture(Gun::NONE, 30, 0);
    let shot_at = 20u64;
    let run = |back: u8| {
        let mut ps = range(30.0, 0.0);
        let mut rw = Rewind::new();
        for t in 0..shot_at {
            rw.write_row(t, &ps);
        }
        // Live, the target is two metres to the side.
        ps[1].body.qx = (2.0 / POS_XZ_Q) as i32;
        let mut bullets = Bullets::new();
        let mut landed = 0;
        for t in shot_at..shot_at + 6 {
            let ev = tick(t, &cc, &mut ps, &mut bullets, &rw, back);
            ps[0].frame.buttons = 0;
            landed += hits(&ev).len();
        }
        landed
    };
    assert_eq!(run(0), 0, "favour 0 judges the live body, off the line");
    assert_eq!(run(4), 1, "favour 4 judges the body the shooter saw");
}

/// **The shooter's share.** With `MAX_BULLETS_PER_OWNER` rounds already in
/// the air, the trigger is refused before the magazine is touched.
#[test]
fn a_full_share_refuses_the_shot_before_the_round_is_spent() {
    let cc = fixture(Gun::NONE, 30, 0);
    let mut ps = range(0.0, 0.0);
    let mut bullets = Bullets::new();
    for _ in 0..MAX_BULLETS_PER_OWNER {
        bullets.launch(bullet::Bullet {
            owner: 1,
            life: 50,
            reach: 1_000_000,
            ..bullet::Bullet::default()
        });
    }
    assert!(!bullets.room_for(1, 1));
    let mut events = EventQueue::default();
    let mut kills = [Kill::default(); MAX_ARROWS];
    let mut chips = [ranged::Chip::default(); MAX_ARROWS];
    let mut sc = Scratch::barren();
    let mobs = Mobs::new(SEED, hv());
    let mut mh = [ranged::MobShot::default(); MAX_ARROWS];
    let mut struck = [DrumShot::default(); MAX_ARROWS];
    let mut quarry = ranged::Quarry {
        mobs: &mobs,
        mc: &MobContent::EMPTY,
        hits: &mut mh,
        n: 0,
        drums: &Drums::NONE,
        struck: &mut struck,
        nd: 0,
    };
    ranged::hitscan_hunting(
        SEED,
        SEED,
        hv(),
        &ColIndex::new(),
        &mut sc.occupants(),
        0,
        &Rewind::new(),
        &[0; MAX_PLAYERS],
        &cc,
        &mut ps,
        &mut events,
        &mut kills,
        &mut chips,
        &mut bullets,
        &mut quarry,
    );
    assert_eq!(ps[0].gun().loaded, 30, "the round stayed in the magazine");
    assert!(events.entries().iter().all(|e| e.code != EV_SHOT));
}

/// **The exact cadence.** 343/min is 5.25 ticks: held, the trigger fires on
/// 0, 6, 11, 16, 21 — four gaps in 21 ticks, the gun's own rate — where whole
/// ticks would fire every fifth and make it 360.
#[test]
fn a_held_trigger_carries_the_sub_tick_remainder() {
    let cc = fixture(Gun::NONE, 30, 1344);
    let mut ps = range(0.0, 0.0);
    let mut bullets = Bullets::new();
    let rw = Rewind::new();
    let mut fired = Vec::new();
    for t in 0..22 {
        let ev = tick(t, &cc, &mut ps, &mut bullets, &rw, 0);
        if ev.entries().iter().any(|e| e.code == EV_SHOT) {
            fired.push(t);
        }
    }
    assert_eq!(fired, vec![0, 6, 11, 16, 21]);
    // A shot taken late starts the count afresh rather than banking time.
    let def = cc.ranged[GUN as usize];
    let (next, due) = ranged::cadence(40, 10 * 256, &def);
    assert_eq!((next, due), (46, 40 * 256 + 1344));
}

/// **The report.** A bullet's `EV_SHOT` carries its speed in the high half,
/// as an arrow's does, and the bullet bit, its reach and its drop below.
#[test]
fn a_bullet_reports_its_speed_drop_and_reach() {
    let cc = fixture(Gun::NONE, 30, 0);
    let mut ps = range(0.0, 0.0);
    let ev = tick(0, &cc, &mut ps, &mut Bullets::new(), &Rewind::new(), 0);
    let shot = ev
        .entries()
        .iter()
        .find(|e| e.code == EV_SHOT)
        .expect("the shot is reported");
    assert_eq!(shot.c >> 16, u32::from(SPEED));
    assert_ne!(shot.c & SHOT_BULLET, 0, "the bullet bit");
    assert_eq!(shot.c & ((1 << SHOT_DROP_BITS) - 1), u32::from(DROP));
    let reach = (shot.c >> SHOT_DROP_BITS) & ((1 << SHOT_REACH_BITS) - 1);
    assert_eq!(reach * SHOT_REACH_UNIT_MM, 170_000);
    assert_eq!(shot.c, shot_bullet_c(SPEED, DROP, 170_000, false));
}

/// **A round may name its own count**: a slug is one projectile out of a
/// gun whose buckshot is eight, and one blow at the slug's damage.
#[test]
fn a_slug_is_one_projectile_out_of_a_shotgun() {
    let gun = Gun {
        pellets: 8,
        pellet: 600,
        ..Gun::NONE
    };
    let mut cc = fixture(gun, 10, 0);
    cc.ammo[ROUND as usize].pellets = 1;
    cc.ammo[ROUND as usize].damage_pct = 500;
    let mut ps = range(0.0, 0.0);
    let mut bullets = Bullets::new();
    tick(0, &cc, &mut ps, &mut bullets, &Rewind::new(), 0);
    assert_eq!(bullets.len(), 1, "one ball in the air");
    assert_eq!(bullets.entries().next().unwrap().damage, 50);
    let mut ps = range(10.0, 0.0);
    assert_eq!(one_shot(&cc, &mut ps, 10), vec![(0, 50)]);
}
