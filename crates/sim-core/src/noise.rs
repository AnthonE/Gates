//! What the animals hear — the reference's newest sense, ripped.
//!
//! **What the reference has.** Its 2024 AI's sense component hears as well
//! as sees: noises go into a grid, hearing has its own range, and the most
//! recent noise wins. Its tiger hears ore and tree hits and gunshots and
//! comes to look; its scientists dive for cover at a shot.
//!
//! **What this is.** A noise is a point, a kind and a tick. The world
//! records them from facts the tick already produced — `EV_SHOT` (a firearm
//! is loud and a bow is not), `EV_IMPACT` (something struck a surface: a
//! tree being felled, a node being mined, a bullet landing) — and from a
//! charge whose fuse runs out, into a small ring. How far each kind carries
//! is the listener's: a species hears each at its own range
//! (`MobDef::hear`, `content/mobs.toml` `hear_m`). An animal that thinks
//! inside its range of a noise within a think of it heard it
//! (`brain::sense`): prey runs from the spot, a hunter goes to see.
//!
//! **Derived, not state.** A noise is a function of the tick's events and
//! bodies, and the hash already covers what made them, so the ring is
//! neither hashed nor saved — the event queue's and the rewind ring's
//! posture. A restart forgets a gunshot, which an animal that heard it
//! would have done within `brain`'s noise memory anyway.

use crate::limits::{MAX_NOISES, MAX_PLAYERS, MOB_THINK_TICKS};
use crate::world::{EventQueue, Player, EV_IMPACT, EV_SHOT};

/// What made a noise: which of a listener's ranges it is heard at.
#[repr(u8)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Sound {
    /// An empty ring slot: heard by nobody.
    #[default]
    Silence = 0,
    /// A firearm going off.
    Gun = 1,
    /// A bow loosing: quiet, the reason to hunt with one (an arrow reaches
    /// an animal since `ranged::Quarry`).
    Bow = 2,
    /// A strike on a surface: a hatchet in a trunk, a pick on a node, a
    /// round landing in the dirt.
    Strike = 3,
    /// A satchel going off.
    Blast = 4,
}

/// How far one species hears each kind of noise, centimetres — content
/// (`content/mobs.toml` `hear_m`, baked). **Ours**: the reference publishes
/// that its animals hear these, not how far. Zero is deaf to that kind.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Hearing {
    pub gun_cm: i64,
    pub bow_cm: i64,
    pub strike_cm: i64,
    pub blast_cm: i64,
}

impl Hearing {
    /// Hears nothing: the inert species' row.
    pub const DEAF: Self = Self {
        gun_cm: 0,
        bow_cm: 0,
        strike_cm: 0,
        blast_cm: 0,
    };

    /// The range `sound` is heard across, centimetres.
    pub fn range_cm(&self, sound: Sound) -> i64 {
        match sound {
            Sound::Silence => 0,
            Sound::Gun => self.gun_cm,
            Sound::Bow => self.bow_cm,
            Sound::Strike => self.strike_cm,
            Sound::Blast => self.blast_cm,
        }
    }
}

/// One noise.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Noise {
    pub qx: i32,
    pub qz: i32,
    pub sound: Sound,
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
                sound: Sound::Silence,
                at: 0,
            }; MAX_NOISES],
            next: 0,
        }
    }

    pub fn push(&mut self, n: Noise) {
        self.ring[self.next] = n;
        self.next = (self.next + 1) % MAX_NOISES;
    }

    /// The most recent noise audible at `(qx, qz)` on `tick` to a listener
    /// that hears this well: inside its range of the noise's kind, and made
    /// within one think of now — every animal thinks once in any
    /// `MOB_THINK_TICKS` window, so each hears each noise at most once and
    /// never misses one in range. Ties go to the one it hears furthest.
    pub fn heard(&self, tick: u64, qx: i32, qz: i32, hear: &Hearing) -> Option<Noise> {
        let mut best: Option<(Noise, i64)> = None;
        for n in self.ring.iter() {
            let r = hear.range_cm(n.sound);
            if r <= 0 || n.at > tick || tick - n.at > MOB_THINK_TICKS {
                continue;
            }
            let (dx, dz) = ((n.qx - qx) as i64 * 3, (n.qz - qz) as i64 * 3);
            if dx * dx + dz * dz > r * r {
                continue;
            }
            if best.is_none_or(|(b, br)| n.at > b.at || (n.at == b.at && r > br)) {
                best = Some((*n, r));
            }
        }
        best.map(|(n, _)| n)
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
                    let sound = if e.c >> 16 == 0 {
                        Sound::Gun
                    } else {
                        Sound::Bow
                    };
                    self.push(Noise {
                        qx: p.body.qx,
                        qz: p.body.qz,
                        sound,
                        at: tick,
                    });
                }
                // Where something struck the world; the position rides the
                // event (x in `a` under the surface and the weapon kind —
                // `world::impact_parts` — z in `b`).
                EV_IMPACT => self.push(Noise {
                    qx: crate::world::impact_parts(e.a).2,
                    qz: e.b as i32,
                    sound: Sound::Strike,
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

    /// The shipped species' ears: 100, 15, 25 and 200 m.
    const EARS: Hearing = crate::mob::MobContent::FIXTURE_EARS;

    #[test]
    fn a_noise_is_heard_inside_its_range_for_one_think() {
        let mut n = Noises::new();
        n.push(Noise {
            qx: 1_000,
            qz: 1_000,
            sound: Sound::Bow,
            at: 100,
        });
        // 14 m off (467 quanta): inside a bow's 15 m.
        assert!(n.heard(100, 1_000 + 467, 1_000, &EARS).is_some());
        assert!(n
            .heard(100 + MOB_THINK_TICKS, 1_000, 1_000, &EARS)
            .is_some());
        // Past the window, and past the range.
        assert!(n
            .heard(101 + MOB_THINK_TICKS, 1_000, 1_000, &EARS)
            .is_none());
        assert!(n.heard(100, 1_000 + 600, 1_000, &EARS).is_none());
        // Not before it was made.
        assert!(n.heard(99, 1_000, 1_000, &EARS).is_none());
        // An empty ring hears nothing, even on tick 0 at the origin.
        assert!(Noises::new().heard(0, 0, 0, &EARS).is_none());
    }

    /// **The range is the listener's.** One gunshot, two species 60 m off:
    /// the one whose row hears a gun at 100 m hears it, one that hears it at
    /// 50 m does not, and a deaf row hears nothing at its feet.
    #[test]
    fn how_far_a_noise_carries_is_the_listeners() {
        let mut n = Noises::new();
        n.push(Noise {
            qx: 0,
            qz: 0,
            sound: Sound::Gun,
            at: 10,
        });
        let dull = Hearing {
            gun_cm: 5_000,
            ..EARS
        };
        // 60 m is 2 000 quanta.
        assert!(n.heard(10, 2_000, 0, &EARS).is_some());
        assert!(n.heard(10, 2_000, 0, &dull).is_none());
        assert!(n.heard(10, 0, 0, &Hearing::DEAF).is_none());
    }

    /// Two noises on one tick: the one this listener hears furthest wins,
    /// whatever order the ring holds them in.
    #[test]
    fn a_tie_goes_to_the_kind_heard_furthest() {
        for order in [[Sound::Strike, Sound::Blast], [Sound::Blast, Sound::Strike]] {
            let mut n = Noises::new();
            for (i, sound) in order.into_iter().enumerate() {
                n.push(Noise {
                    qx: i as i32,
                    qz: 0,
                    sound,
                    at: 10,
                });
            }
            let got = n.heard(10, 0, 0, &EARS).expect("heard");
            assert_eq!(got.sound, Sound::Blast);
        }
    }

    #[test]
    fn the_newest_noise_wins_and_a_full_ring_forgets_the_oldest() {
        let mut n = Noises::new();
        for i in 0..MAX_NOISES as u64 + 1 {
            n.push(Noise {
                qx: i as i32,
                qz: 0,
                sound: Sound::Blast,
                at: 10 + i,
            });
        }
        let got = n.heard(10 + MAX_NOISES as u64, 0, 0, &EARS).expect("heard");
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
        let kinds: Vec<Sound> = n.ring[..3].iter().map(|x| x.sound).collect();
        assert_eq!(kinds, [Sound::Gun, Sound::Bow, Sound::Strike]);
        assert_eq!((n.ring[0].qx, n.ring[0].qz), (500, 700));
        assert_eq!((n.ring[2].qx, n.ring[2].qz), (900, 1_100));
    }
}
