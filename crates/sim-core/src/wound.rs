//! Being down instead of dead — wounded v0 (`reference/WOUNDED.md` §9).
//!
//! A lethal blow from an eligible source does not make a corpse. It lays the
//! body down for most of a minute — the reference's own window, 40–50 s
//! since Devblog 121 (2016) — with a little hp, able to crawl and to look
//! and to do nothing else, and at the end of the window the world rolls a
//! die: 20 % to get up, plus up to 25 % for a full stomach and a full canteen
//! (the July 2021 Wounding Update's numbers, taken as `BALANCE.md` §6 says
//! to). A failed roll is the death the blow would have been. Any hit that
//! empties the crawl's hp is also that death, and so is a second lethal
//! blow inside a minute of getting up (Devblog 71, 2015: raid revive loops).
//!
//! **What this module is and is not.** It is the arithmetic — the window,
//! the odds, the crawl — as pure functions over integers, so `World::tick`
//! decides and this file never touches the queue, the corpse or the clock.
//! It is *not* the second, incapacitated state the reference keeps for
//! three triggers this game does not have (being looted alive, a fall, deep
//! water — `WOUNDED.md` §2.1), not the six-second hands-on revive, not the
//! syringe, not the medkit-in-belt rule. `WOUNDED.md` §9.6 stages those.
//!
//! Wall 1: integer arithmetic and `rng::cell_hash` only. Wall 5: every
//! random draw is keyed on `(seed, player id, tick)`, so a replay rolls the
//! same die and lands the same way.

use crate::input::{InputFrame, BTN_JUMP, BTN_PRIMARY, BTN_SPRINT};
use crate::rng::cell_hash;
use crate::world::{DEATH_BY_ARROW, DEATH_BY_BULLET, DEATH_BY_HAND, DEATH_BY_MOB};

/// The shortest a down lasts before the roll, in ticks: **40 s** at
/// `TICK_HZ = 30`. The reference's window (Devblog 121: "40 to 50 seconds
/// instead of 15 to 30", restated for the crawl in 2021). Proposed default,
/// `DECISIONS.md` §open ("wounded v0"); `tests/wounded.rs` holds it to the
/// tick rate so a changed `TICK_HZ` cannot silently shorten the minute.
pub const WOUND_MIN_TICKS: u32 = 1_200;
/// The longest, inclusive: **50 s**. Same source, same row.
pub const WOUND_MAX_TICKS: u32 = 1_500;
/// The base chance of getting up when the window ends, per mille: **20 %**
/// (`woundedrecoverchance 0.2`, the reference's crawling default). Same row.
pub const RECOVER_BASE_PM: u32 = 200;
/// What a full stomach AND a full canteen add on top, per mille: **25 %**
/// (`woundedmaxfoodandwaterbonus 0.25` — "max 20 + 25 = 45% total"). The
/// curve between empty and full is not published; ours is linear in the
/// mean of the two meters (`WOUNDED.md` §2.5, §7). Same row.
pub const RECOVER_BONUS_MAX_PM: u32 = 250;
/// The hp a body is given as it falls — what is left to lose before a
/// second blow finishes it. The reference shipped the crawl at 30–50 and cut
/// it by three quarters two months later (`WOUNDED.md` §2.8) — the midpoint
/// of the cut band, rounded. A `u16` because `Player::hp` is one; proposed
/// default, same row.
pub const WOUNDED_HP: u16 = 10;
/// How long after getting up a second lethal blow kills outright instead of
/// laying the body down again, in ticks: **60 s** (`rewounddelay 60`;
/// Devblog 71's rule against raid revive loops). Same row.
pub const REWOUND_TICKS: u64 = 1_800;
/// The crawl, as a divisor on the frame's movement axes: a third of the
/// walk, ~1.0 m/s against `WALK_SPEED = 3.0`. The reference publishes no
/// speed ("considerably slower than walking") so this is ours; the shape —
/// scaling the wish vector rather than a speed the step reads — is what
/// lets one function serve the sim and the client's predictor identically
/// (`crawl_frame`). Proposed default, same row.
pub const CRAWL_DIV: i8 = 3;

/// Hash channels, `worldcont::CH_REFILL`'s convention (100) continued: one
/// for the window's length, one for the roll, so the two draws off one tick
/// are independent.
pub const CH_WOUND_SPAN: u32 = 101;
pub const CH_WOUND_ROLL: u32 = 102;

/// Does a lethal blow of this kind lay the body down rather than kill it?
///
/// The reference's predicate is on the HIT (`EligibleForWounding(HitInfo)`,
/// `WOUNDED.md` §1): a melee blow always wounds, a projectile wounds unless
/// it took the head ("If you're shot in the head you'll die instantly" —
/// Devblog 53, still true), an animal wounds, and the things that are not a
/// hit — the clock, the sea, a satchel charge — kill. The blast is the one
/// judgement call: the reference does not publish it (§7), and a body
/// inside a blast radius that stood up 45 s later would make explosives the
/// weakest way to kill a defender, which is the wrong way round for the
/// most expensive damage in the game.
#[inline]
pub fn wounds(cause: u8, head: bool) -> bool {
    match cause {
        DEATH_BY_HAND | DEATH_BY_MOB => true,
        DEATH_BY_ARROW | DEATH_BY_BULLET => !head,
        _ => false,
    }
}

/// How long this down lasts, in ticks — `WOUND_MIN_TICKS..=WOUND_MAX_TICKS`,
/// drawn off `(seed, id, tick)` so a replay draws the same span.
#[inline]
pub fn span_ticks(seed: u64, id: u32, tick: u64) -> u32 {
    let range = WOUND_MAX_TICKS - WOUND_MIN_TICKS + 1;
    let h = cell_hash(seed, id as i32, tick as u32 as i32, CH_WOUND_SPAN);
    WOUND_MIN_TICKS + (h % range as u64) as u32
}

/// The chance of getting up, per mille, from the body's meters against
/// their ceilings: the base plus the bonus scaled by the mean fill of food
/// and water. A ceiling of zero (inert survival content) contributes no
/// bonus rather than dividing by it. Never above 1000 by construction:
/// `RECOVER_BASE_PM + RECOVER_BONUS_MAX_PM` is 450.
#[inline]
pub fn recover_chance_pm(food: u16, water: u16, max_food: u16, max_water: u16) -> u32 {
    let fill = |v: u16, max: u16| -> u32 {
        if max == 0 {
            0
        } else {
            (v.min(max) as u32 * 1000) / max as u32
        }
    };
    let mean_pm = (fill(food, max_food) + fill(water, max_water)) / 2;
    RECOVER_BASE_PM + RECOVER_BONUS_MAX_PM * mean_pm / 1000
}

/// The roll: does a body with `chance_pm` get up on this tick? One draw off
/// `(seed, id, tick)` on its own channel, compared against the chance —
/// the same shape as `loot::roll_into`'s tables, and deterministic for the
/// same reason.
#[inline]
pub fn recovers(seed: u64, id: u32, tick: u64, chance_pm: u32) -> bool {
    let h = cell_hash(seed, id as i32, tick as u32 as i32, CH_WOUND_ROLL);
    (h % 1000) < chance_pm as u64
}

/// The frame a downed body actually moves on: no sprint, no jump, no swing,
/// and the movement axes divided by [`CRAWL_DIV`]. Applied by the sim to a
/// wounded body's frame at the step, and by the client's predictor to its
/// own frames while it knows it is down — one function, so the two cannot
/// disagree about how fast a crawl is. The look and the hotbar selection
/// ride through: a downed player can still turn their head.
#[inline]
pub fn crawl_frame(f: &InputFrame) -> InputFrame {
    InputFrame {
        seq: f.seq,
        buttons: f.buttons & !(BTN_SPRINT | BTN_JUMP | BTN_PRIMARY),
        yaw: f.yaw,
        pitch: f.pitch,
        move_x: f.move_x / CRAWL_DIV,
        move_z: f.move_z / CRAWL_DIV,
        sel: f.sel,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::input::BTN_LIGHT;

    #[test]
    fn the_odds_are_the_reference_endpoints() {
        assert_eq!(recover_chance_pm(0, 0, 100, 100), 200, "empty is the base");
        assert_eq!(
            recover_chance_pm(100, 100, 100, 100),
            450,
            "full is base + bonus"
        );
        assert_eq!(
            recover_chance_pm(50, 50, 100, 100),
            325,
            "half is the midpoint"
        );
        assert_eq!(
            recover_chance_pm(100, 0, 100, 100),
            325,
            "one meter is half the bonus"
        );
        assert_eq!(
            recover_chance_pm(7, 7, 0, 0),
            200,
            "inert ceilings add nothing"
        );
        assert_eq!(
            recover_chance_pm(500, 500, 100, 100),
            450,
            "an over-full meter does not pay more than full"
        );
    }

    #[test]
    fn the_span_stays_inside_the_window_and_repeats() {
        for id in 1..200u32 {
            for tick in [0u64, 1, 29, 30, 1_000, 65_536, 1 << 40] {
                let s = span_ticks(77, id, tick);
                assert!((WOUND_MIN_TICKS..=WOUND_MAX_TICKS).contains(&s), "{s}");
                assert_eq!(s, span_ticks(77, id, tick), "the draw is a function");
            }
        }
        assert_ne!(
            span_ticks(77, 1, 10),
            span_ticks(77, 2, 10),
            "two bodies downed on one tick do not share a clock"
        );
    }

    /// The roll is a fair die at the resolution it claims: over ten thousand
    /// (id, tick) pairs the hit rate sits within two points of the chance
    /// at both published endpoints. A hash that clumped would fail this.
    #[test]
    fn the_roll_lands_at_the_rate_it_names() {
        for (chance, lo, hi) in [(200u32, 180usize, 220usize), (450, 430, 470), (0, 0, 0)] {
            let mut ups = 0usize;
            let n = 10_000usize;
            for i in 0..n {
                if recovers(
                    20260913,
                    (i % 100) as u32 + 1,
                    (i / 100) as u64 * 1_301,
                    chance,
                ) {
                    ups += 1;
                }
            }
            let per_mille = ups * 1000 / n;
            assert!(
                (lo..=hi).contains(&per_mille),
                "chance {chance}: {per_mille} per mille over {n}"
            );
        }
        assert!(recovers(1, 1, 1, 1000), "a certainty is certain");
    }

    #[test]
    fn a_crawl_is_a_third_of_the_walk_with_no_arm() {
        let f = InputFrame {
            seq: 9,
            buttons: BTN_SPRINT | BTN_JUMP | BTN_PRIMARY | BTN_LIGHT,
            yaw: 1234,
            pitch: 99,
            move_x: -127,
            move_z: 127,
            sel: 4,
        };
        let c = crawl_frame(&f);
        assert_eq!(
            c.buttons, BTN_LIGHT,
            "sprint, jump and the swing are stripped"
        );
        assert_eq!(c.move_x, -42);
        assert_eq!(c.move_z, 42);
        assert_eq!(
            (c.seq, c.yaw, c.pitch, c.sel),
            (9, 1234, 99, 4),
            "the rest rides through"
        );
    }

    #[test]
    fn what_wounds_and_what_kills() {
        use crate::world::{DEATH_BY_CHARGE, DEATH_BY_CLOCK, DEATH_BY_SALT};
        assert!(
            wounds(DEATH_BY_HAND, true),
            "a melee blow wounds even to the head"
        );
        assert!(wounds(DEATH_BY_MOB, false));
        assert!(wounds(DEATH_BY_ARROW, false));
        assert!(wounds(DEATH_BY_BULLET, false));
        assert!(!wounds(DEATH_BY_ARROW, true), "a headshot kills outright");
        assert!(!wounds(DEATH_BY_BULLET, true));
        assert!(!wounds(DEATH_BY_CHARGE, false));
        assert!(!wounds(DEATH_BY_CLOCK, false));
        assert!(!wounds(DEATH_BY_SALT, false));
    }
}
