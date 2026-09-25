//! What the animals hear — the reference's newest sense, ripped.
//!
//! **What the reference has.** Its 2024 AI's sense component hears as well
//! as sees: noises go into a grid, hearing has its own range, and the most
//! recent noise wins. Its tiger hears ore and tree hits and gunshots and
//! comes to look; its scientists dive for cover at a shot.
//!
//! **What this is.** A noise is a point, a radius and a tick. The world
//! records them from facts the tick already produced — `EV_SHOT` (a firearm
//! is loud and a bow is not), `EV_IMPACT` (something struck a surface: a
//! tree being felled, a node being mined, a bullet landing) — and from a
//! charge whose fuse runs out, into a small ring. An animal that thinks
//! inside a noise's radius within a think of it heard it (`brain::sense`):
//! prey runs from the spot, a hunter goes to see.
//!
//! **Derived, not state.** A noise is a function of the tick's events and
//! bodies, and the hash already covers what made them, so the ring is
//! neither hashed nor saved — the event queue's and the rewind ring's
//! posture. A restart forgets a gunshot, which an animal that heard it
//! would have done within `brain`'s noise memory anyway.

use crate::limits::{MAX_NOISES, MAX_PLAYERS, MOB_THINK_TICKS};
use crate::world::{EventQueue, Player, EV_IMPACT, EV_SHOT};

/// How far each kind of noise carries, centimetres. **Ours**: the reference
/// publishes that its animals hear these, not how far.
///
/// A gunshot at a hundred metres is the disclosure `reference/AUDIO.md` §9
/// already prices for players; an animal gets the same.
pub const NOISE_GUN_CM: i64 = 10_000;
/// A bow is quiet — the reason to hunt with one (an arrow reaches an
/// animal since `ranged::Quarry`).
pub const NOISE_BOW_CM: i64 = 1_500;
/// A strike on a surface: a hatchet in a trunk, a pick on a node, a round
/// landing in the dirt.
pub const NOISE_STRIKE_CM: i64 = 2_500;
/// A satchel going off.
pub const NOISE_BLAST_CM: i64 = 20_000;

/// One noise.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Noise {
    pub qx: i32,
    pub qz: i32,
    pub radius_cm: i64,
    /// The tick it was made on.
    pub at: u64,
}

/// The recent noises, a ring: a full one forgets the oldest, which is the
/// one every animal has either heard already or been too far away from.
pub struct Noises {
    ring: [Noise; MAX_NOISES],
    next: usize,
}

impl Default for Noises {
    fn default() -> Self {
        Self::new()
    }
}

impl Noises {
    pub const fn new() -> Self {
        Self {
            ring: [Noise {
                qx: 0,
                qz: 0,
                radius_cm: 0,
                at: 0,
            }; MAX_NOISES],
            next: 0,
        }
    }

    pub fn push(&mut self, n: Noise) {
        self.ring[self.next] = n;
        self.next = (self.next + 1) % MAX_NOISES;
    }

    /// The most recent noise audible at `(qx, qz)` on `tick`: inside its
    /// radius, and made within one think of now — every animal thinks once
    /// in any `MOB_THINK_TICKS` window, so each hears each noise at most
    /// once and never misses one in range. Ties go to the louder.
    pub fn heard(&self, tick: u64, qx: i32, qz: i32) -> Option<Noise> {
        let mut best: Option<Noise> = None;
        for n in self.ring.iter() {
            if n.radius_cm == 0 || n.at > tick || tick - n.at > MOB_THINK_TICKS {
                continue;
            }
            let (dx, dz) = ((n.qx - qx) as i64 * 3, (n.qz - qz) as i64 * 3);
            if dx * dx + dz * dz > n.radius_cm * n.radius_cm {
                continue;
            }
            if best.is_none_or(|b| n.at > b.at || (n.at == b.at && n.radius_cm > b.radius_cm)) {
                best = Some(*n);
            }
        }
        best
    }

    /// Record this tick's noises off its events. Called once, at the end of
    /// the tick, so every producer has spoken.
    pub fn record(&mut self, tick: u64, events: &EventQueue, players: &[Player; MAX_PLAYERS]) {
        for e in events.entries() {
            match e.code {
                // A shot, from the shooter: speed zero is a firearm (the
                // event's own convention), anything else is a bow.
                EV_SHOT => {
                    let Some(p) = players.iter().find(|p| p.active && p.id == e.a) else {
                        continue;
                    };
                    let radius_cm = if e.c >> 16 == 0 {
                        NOISE_GUN_CM
                    } else {
                        NOISE_BOW_CM
                    };
                    self.push(Noise {
                        qx: p.body.qx,
                        qz: p.body.qz,
                        radius_cm,
                        at: tick,
                    });
                }
                // Where something struck the world; the position rides the
                // event (x in `a` under the surface and the weapon kind —
                // `world::impact_parts` — z in `b`).
                EV_IMPACT => self.push(Noise {
                    qx: crate::world::impact_parts(e.a).2,
                    qz: e.b as i32,
                    radius_cm: NOISE_STRIKE_CM,
                    at: tick,
                }),
                _ => {}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_noise_is_heard_inside_its_radius_for_one_think() {
        let mut n = Noises::new();
        n.push(Noise {
            qx: 1_000,
            qz: 1_000,
            radius_cm: NOISE_BOW_CM,
            at: 100,
        });
        // 14 m off (467 quanta): inside a bow's 15 m.
        assert!(n.heard(100, 1_000 + 467, 1_000).is_some());
        assert!(n.heard(100 + MOB_THINK_TICKS, 1_000, 1_000).is_some());
        // Past the window, and past the radius.
        assert!(n.heard(101 + MOB_THINK_TICKS, 1_000, 1_000).is_none());
        assert!(n.heard(100, 1_000 + 600, 1_000).is_none());
        // Not before it was made.
        assert!(n.heard(99, 1_000, 1_000).is_none());
    }

    #[test]
    fn the_newest_noise_wins_and_a_full_ring_forgets_the_oldest() {
        let mut n = Noises::new();
        for i in 0..MAX_NOISES as u64 + 1 {
            n.push(Noise {
                qx: i as i32,
                qz: 0,
                radius_cm: NOISE_BLAST_CM,
                at: 10 + i,
            });
        }
        let got = n.heard(10 + MAX_NOISES as u64, 0, 0).expect("heard");
        assert_eq!(got.at, 10 + MAX_NOISES as u64);
        assert!(
            n.ring.iter().all(|x| x.at != 10),
            "the oldest survived a full ring"
        );
    }

    #[test]
    fn a_gunshot_is_loud_and_an_arrow_is_not() {
        let mut players = Box::new([Player::default(); MAX_PLAYERS]);
        players[3].active = true;
        players[3].id = 77;
        players[3].body.qx = 500;
        players[3].body.qz = 700;
        let mut ev = EventQueue::default();
        ev.push(EV_SHOT, 77, 0, 0); // speed 0: a firearm
        ev.push(EV_SHOT, 77, 0, 40 << 16); // a flight: a bow

        // A hatchet blow: the weapon kind sits above x in `a`, and must not
        // move the noise.
        let a = crate::world::impact_a(crate::ranged::SURF_WORLD, crate::ranged::IMPACT_MELEE, 900);
        ev.push(EV_IMPACT, a, 1_100, 0);
        let mut n = Noises::new();
        n.record(5, &ev, &players);
        let radii: Vec<i64> = n.ring[..3].iter().map(|x| x.radius_cm).collect();
        assert_eq!(radii, [NOISE_GUN_CM, NOISE_BOW_CM, NOISE_STRIKE_CM]);
        assert_eq!((n.ring[0].qx, n.ring[0].qz), (500, 700));
        assert_eq!((n.ring[2].qx, n.ring[2].qz), (900, 1_100));
    }
}
