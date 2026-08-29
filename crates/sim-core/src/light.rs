//! **A held light burns itself.** Torch fuel v0 — the half of
//! `ALPHA.md` §1's *"light = visibility = target"* that decides whether a
//! flame is a mechanic or a decoration.
//!
//! Until this module the torch was a lamp that could not be switched off
//! and never ran out: a player's whole answer to nightfall was free, so
//! nothing was traded for it (`NOW.md` §0tl items 3 and 4). Here a flame
//! costs the item it stands on, at the reference's own rate — 1/6 of a
//! condition point per second, which `content/items.toml` states as
//! `light_burn = 1000` hundredths a minute and which turns the torch's
//! 50-point ceiling into exactly five minutes of light.
//!
//! ## There is no `lit` flag, and that is the design
//!
//! A flame is not stored anywhere. It is *derived*, every tick, from three
//! facts both ends of the wire already hold:
//!
//! 1. the player's own [`BTN_LIGHT`](crate::input::BTN_LIGHT) latch, which
//!    the client owns and the input datagram carries, exactly like `sel`;
//! 2. the held item's content row declaring a `light_burn` at all;
//! 3. that stack having condition left to spend.
//!
//! So "put it out" is releasing the latch, "it went out" is the third fact
//! going false, and neither needs a verb, an event or a byte of state. The
//! quantize-both-sides law (CLAUDE.md's trap list) applied to a flag: the
//! client draws its hand light off the same three facts — its own latch,
//! its own `HELD_MODELS` row, and the `cond` that `SUB_INV` mirrors — so
//! the two sides cannot disagree about a stored bit, because there is not
//! one. A dropped datagram costs a frame of flame, never an inverted
//! flame that stays inverted.
//!
//! What *is* stored is [`Player::light_acc`](crate::world::Player), the
//! sub-point remainder, for the reason `persist.rs` gives about the food
//! and water accumulators: a restore that zeroed it would hand back a
//! fraction of a torch on every reconnect.
//!
//! ## The debit is in whole points on purpose
//!
//! `cond` is hundredths, and burning hundredths would move it about
//! seventeen times a second — and the server diffs the whole inventory
//! against its last copy every tick (`ShardCore`), so every one of those
//! would put a `SUB_INV` message on the wire for a number nobody reads
//! that precisely. The accumulator therefore counts out **points**: with
//! the torch's rate the held stack changes once every six seconds, which
//! is ten messages a minute per lit player instead of a thousand.
//!
//! Content rule V9 keeps `light_burn` under `u16::MAX`, which is below
//! [`BURN_DEN`], so [`tick_units`](crate::survival::tick_units) can never
//! put more than one point out of the accumulator in a tick: the per-tick
//! work is bounded by the content bound rather than by a clamp somebody
//! has to remember (wall 4).

use crate::gather::GatherContent;
use crate::input::BTN_LIGHT;
use crate::limits::{HOTBAR_SLOTS, TICK_HZ};
use crate::world::Player;

/// Ticks × hundredths that buy one whole condition point.
///
/// `TICK_HZ · 60` turns a per-minute rate into a per-tick numerator and
/// the `100` is what makes the quotient a *point* rather than a hundredth
/// — see the module note on why the debit is coarse. Derived rather than
/// typed, so changing the tick rate re-derives the burn instead of
/// silently rescaling every light in the game.
pub const BURN_DEN: u32 = TICK_HZ * 60 * 100;

/// Is this hand actually alight?
///
/// The three facts, in the order that costs least: the latch, then the
/// content row, then the fuel. A body nobody is driving is never alight —
/// `dead` and `sleeping` both take the input path away (`world::tick`),
/// so their last frame is stale and a corpse holding a burning torch
/// would be that stale frame spending an inventory nobody is watching.
///
/// Total over any `sel`: `world::apply` clamps the wire's three bits and
/// falls a non-wire frame back to slot 0, and this bounds it again rather
/// than trusting that — one id arrives from a datagram and one from a WAL
/// (`GatherContent::cond_max_of`'s reason, one layer up).
pub fn is_lit(p: &Player, gc: &GatherContent) -> bool {
    if p.dead || p.sleeping || p.frame.buttons & BTN_LIGHT == 0 {
        return false;
    }
    let sel = p.frame.sel as usize;
    if sel >= HOTBAR_SLOTS {
        return false;
    }
    let s = p.inv[sel];
    s.count > 0 && s.cond > 0 && gc.light_burn_of(s.item) > 0
}

/// One tick of burning, returning the whole condition **points** spent —
/// 0 or 1 by construction (see [`BURN_DEN`] and content rule V9).
///
/// `saturating_sub` for the reason `gather`'s wear uses it: the last
/// partial point is spent and never owed, so a torch at 20 hundredths
/// finishes at 0 rather than wrapping into a full one.
///
/// The flame going out needs no announcement. `cond` reaching 0 is an
/// inventory change, the server's own per-tick diff puts it on the wire as
/// `SUB_INV`, and the client's light is derived from that same `cond` — so
/// the news that the torch died travels on the message that says why.
pub fn step(p: &mut Player, gc: &GatherContent) -> u32 {
    if !is_lit(p, gc) {
        return 0;
    }
    let sel = p.frame.sel as usize;
    let burn = gc.light_burn_of(p.inv[sel].item) as u32;
    let points = crate::survival::tick_units(&mut p.light_acc, burn, BURN_DEN);
    if points > 0 {
        let s = &mut p.inv[sel];
        // `points * 100` cannot overflow a `u16` conversion path because
        // `points` is 0 or 1; the cast is bounded by V9 and asserted by
        // `one_point_a_tick_is_the_ceiling`.
        s.cond = s
            .cond
            .saturating_sub((points * 100).min(u16::MAX as u32) as u16);
    }
    points
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gather::ItemStack;
    use crate::input::{BTN_PRIMARY, BTN_SPRINT};

    /// A player holding `stack` in slot 0 with `buttons` pressed.
    fn holder(stack: ItemStack, buttons: u8) -> Player {
        let mut p = Player {
            active: true,
            frame: crate::input::InputFrame {
                buttons,
                sel: 0,
                ..Default::default()
            },
            ..Player::default()
        };
        p.inv[0] = stack;
        p
    }

    /// The fixture's light (item 0, 600 hundredths a minute) at full.
    fn torch() -> ItemStack {
        ItemStack {
            item: 0,
            count: 1,
            cond: 400,
        }
    }

    fn gc() -> GatherContent {
        GatherContent::probe_fixture()
    }

    /// The three facts, one at a time — each one alone is enough to say no.
    #[test]
    fn a_flame_needs_the_latch_the_row_and_the_fuel() {
        let g = gc();
        assert!(is_lit(&holder(torch(), BTN_LIGHT), &g), "all three hold");

        assert!(
            !is_lit(&holder(torch(), BTN_PRIMARY | BTN_SPRINT), &g),
            "no latch: swinging and sprinting is not lighting a torch"
        );
        let not_a_light = ItemStack {
            item: 1,
            count: 1,
            cond: 300,
        };
        assert!(
            !is_lit(&holder(not_a_light, BTN_LIGHT), &g),
            "item 1 declares no `light_burn`, so the latch lights nothing"
        );
        let spent = ItemStack { cond: 0, ..torch() };
        assert!(
            !is_lit(&holder(spent, BTN_LIGHT), &g),
            "a torch burned to nothing is a stick"
        );
        let empty = ItemStack::default();
        assert!(!is_lit(&holder(empty, BTN_LIGHT), &g), "an empty hand");
    }

    /// A body nobody is driving holds a stale frame, and a stale frame must
    /// not spend an inventory.
    #[test]
    fn a_corpse_and_a_sleeper_burn_nothing() {
        let g = gc();
        let mut dead = holder(torch(), BTN_LIGHT);
        dead.dead = true;
        assert!(!is_lit(&dead, &g), "a corpse is not holding a torch up");
        assert_eq!(step(&mut dead, &g), 0);
        assert_eq!(dead.inv[0].cond, 400, "and it spent nothing");

        let mut asleep = holder(torch(), BTN_LIGHT);
        asleep.sleeping = true;
        assert!(!is_lit(&asleep, &g), "nor is a sleeper");
        assert_eq!(step(&mut asleep, &g), 0);
        assert_eq!(asleep.inv[0].cond, 400);
    }

    /// The cadence, exactly: `BURN_DEN / light_burn` ticks a point, no
    /// point before it and exactly one on it.
    ///
    /// The fixture burns 600 a minute, so 180 000 / 600 = **300 ticks**.
    /// Derived rather than typed, so the assertion follows a fixture edit
    /// instead of going stale under one.
    #[test]
    fn a_point_costs_exactly_its_own_share_of_a_minute() {
        let g = gc();
        let rate = g.light_burn_of(0) as u32;
        let period = BURN_DEN / rate;
        assert_eq!(period, 300, "the fixture's rate stopped dividing evenly");

        let mut p = holder(torch(), BTN_LIGHT);
        for t in 1..period {
            assert_eq!(step(&mut p, &g), 0, "a point fell at tick {t}, early");
        }
        assert_eq!(step(&mut p, &g), 1, "the point falls on tick {period}");
        assert_eq!(p.inv[0].cond, 300, "and it is a whole point off `cond`");
        assert_eq!(p.light_acc, 0, "the remainder is spent, not carried");
    }

    /// The debit can never exceed one point a tick, which is what content
    /// rule V9 is actually buying (wall 4 — the per-tick work is bounded by
    /// the content bound, not by a clamp).
    #[test]
    fn one_point_a_tick_is_the_ceiling() {
        let mut g = gc();
        g.light_burn[0] = u16::MAX; // the fastest row V9 admits
        let mut p = holder(
            ItemStack {
                cond: u16::MAX,
                ..torch()
            },
            BTN_LIGHT,
        );
        for _ in 0..64 {
            assert!(step(&mut p, &g) <= 1, "V9's bound stopped holding");
        }
        assert!(
            u16::MAX as u32 * 2 < BURN_DEN,
            "V9's ceiling is no longer under half `BURN_DEN`, so the \
             one-point bound above is not the arithmetic saying so"
        );
    }

    /// Putting it out and lighting it again does not refund the remainder —
    /// otherwise a client that flicked the latch every tick would burn
    /// nothing at all, which is the exact-arithmetic exploit `persist.rs`
    /// names about the food accumulator.
    #[test]
    fn flicking_the_latch_does_not_dodge_the_burn() {
        let g = gc();
        let period = BURN_DEN / g.light_burn_of(0) as u32;
        let mut flicker = holder(torch(), BTN_LIGHT);
        let mut steady = holder(torch(), BTN_LIGHT);
        for t in 0..period * 4 {
            // Lit on the even ticks only: half the flame, and it had better
            // be half the cost rather than none of it.
            flicker.frame.buttons = if t % 2 == 0 { BTN_LIGHT } else { 0 };
            step(&mut flicker, &g);
            step(&mut steady, &g);
        }
        assert_eq!(steady.inv[0].cond, 400 - 4 * 100);
        assert_eq!(
            flicker.inv[0].cond,
            400 - 2 * 100,
            "half the lit ticks must cost half the points, not zero"
        );
    }

    /// The shipped torch's five minutes, from the numbers themselves.
    ///
    /// `content/items.toml` is `condition_max = 5000` and
    /// `light_burn = 1000`, taken from the reference's 50 points at 1/6 a
    /// second. This walks a whole torch to zero on those two numbers and
    /// asserts the wall clock, because "five minutes" is the claim
    /// `NOW.md` §0tl item 3 and `DECISIONS.md` are making and neither of
    /// the two constants states it alone.
    #[test]
    fn the_shipped_torch_is_five_minutes_of_light() {
        let mut g = GatherContent::EMPTY;
        g.item_count = 1;
        g.stack_max[0] = 1;
        g.cond_max[0] = 5_000;
        g.light_burn[0] = 1_000;
        let mut p = holder(
            ItemStack {
                item: 0,
                count: 1,
                cond: 5_000,
            },
            BTN_LIGHT,
        );
        let mut ticks = 0u32;
        while is_lit(&p, &g) {
            step(&mut p, &g);
            ticks += 1;
            assert!(ticks < 100_000, "the torch never went out");
        }
        assert_eq!(p.inv[0].cond, 0, "it ends spent, not merely dark");
        assert_eq!(
            ticks,
            5 * 60 * TICK_HZ,
            "the shipped torch is no longer five minutes of light"
        );
    }
}
