//! This frame's own-facts, drained from the core **once** and read by
//! everything that wants them.
//!
//! ## Why this exists, and the bug it is the fix for
//!
//! `ClientCore::pop_hit`, `pop_death`, `pop_toast`, `pop_craft_toast` and the
//! three refusal queues are **destructive**: each hands a fact over exactly
//! once and then it is gone. That is correct for a ring the client owns — but
//! it means *two* readers is not "two readers", it is a race the earlier
//! system always wins.
//!
//! It happened. The audio slice added `audio::feed` popping all seven queues,
//! and the same-day HUD slice added `hud::feedback` popping six of them; the
//! two branches touched no common line, so **git merged them cleanly and the
//! result was silently broken** — `feedback` runs inside the `Stream` set,
//! `audio::feed` runs after it, so the HUD drained every ring and the game
//! made no sound for a hit, a gather, a craft or a refusal. Nothing failed. No
//! test could see it, because each half is correct alone.
//!
//! The fix is the one shape that cannot regress: **one drain, at a fixed point
//! in the frame, into a resource that readers borrow immutably.** A second
//! reader is now a `Res<Feed>` parameter, which cannot consume anything, so
//! the failure mode is not available. Adding a third costs nothing and risks
//! nothing.
//!
//! ## The bound
//!
//! Every array here is [`FEED_CAP`] long, which is `client_core`'s own
//! `TOAST_RING`. For the single-source arrays that cap is exact — the core
//! cannot hand over more per frame than the one ring holds. The shared
//! refusal queue is the exception and this line used to deny it: SIX verb
//! rings drain into it (research, craft, gather, build, deploy, consume),
//! so a frame can offer more refusals than one array holds with nothing
//! drifted anywhere. Overflow policy: **drop the newest and count it**
//! ([`Feed::dropped`]), matching `sound::CUE_QUEUE_CAP` — a seventh
//! refusal in one frame is noise, not news. A non-zero count on a
//! single-source array still means the core grew a ring and this file did
//! not follow.

use bevy::prelude::*;
use client_core::core::TOAST_RING;

use super::Net;

/// How many of each fact one frame may carry. The core's own ring size, so
/// a full drain always fits.
pub const FEED_CAP: usize = TOAST_RING;

/// Which verb a refusal answers. The refusal queues are separate on the
/// wire and stay separate here, because the HUD turns each into a different
/// sentence (`ui::refusals`) — the audio side is the one that does not care.
///
/// **Adding a variant is green on `cargo test --workspace` and red at the
/// Bevy gate**, because the one exhaustive `match` over this type lives in
/// `render/hud.rs`, on the same side of `--features render` as the enum
/// (`CLAUDE.md`'s feature-line trap). `tests/ui.rs` §H moves that failure
/// into the code tier by reading both files as text.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Refused {
    // The default only exists so `Feed` can derive one for its fixed array;
    // no reader ever sees a slot past `n_refusals`.
    #[default]
    Craft,
    Build,
    Deploy,
    Research,
    /// An eat (`J`) or a drink (`H`) that did nothing —
    /// `sim_core::survival`'s `REFUSE_C_*`.
    ///
    /// **One variant for two verbs**, because the sim answers both on one
    /// `EV_CONSUME_REFUSED` and one of its three codes belongs to the drink
    /// alone. `ui::refusals::CONSUME` words all three so neither verb reads
    /// as the other.
    Consume,
    /// A gather swing the node refused (wire v42) —
    /// `sim_core::gather`'s `REFUSE_G_*`. The one variant whose entry
    /// carries an **item** beside the code (the held tool, `NO_ITEM` =
    /// bare hands), because its sentence names it: *your torch cannot
    /// harvest this* (`ui::refusals::GATHER`).
    Gather,
}

/// One frame's blows from **one direction**, as [`Feed::hurt_from`] hands
/// them out.
///
/// `from` is an absolute world bearing sector
/// (`sim_core::combat::bearing_sector`) — clockwise from north, on the
/// world's axes rather than the camera's, which is what lets the HUD
/// subtract its own yaw every frame and keep the mark on the attacker while
/// the player turns.
///
/// `damage` is the total that arrived from that bearing this frame and
/// `hits` how many blows it took. Both are kept: `damage` is what the arc's
/// weight reads, and the mixer reads the frame totals beside them —
/// [`Feed::hurt_damage`] weighs the cue and [`Feed::hurts`] is what tells a
/// blow from a starve tick, since a metabolic route announces nothing and
/// has no entry here at all (`crate::sound::hurt`).
///
/// **The per-sector split is still the arc's alone.** `Cue::Hurt` is
/// non-positional and its cooldown is per-cue, so three light blows from
/// three directions are one voice at the weight of their sum — `NOW.md`
/// §0hrt item 1 carries what that costs and why it is a second slice.
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub struct Hurt {
    pub from: u8,
    pub damage: u16,
    pub hits: u16,
}

/// One frame of own-facts. Cleared and refilled by [`drain`]; read-only to
/// everything else.
#[derive(Resource, Default)]
pub struct Feed {
    /// Total damage the local player dealt this frame, and whether any landed.
    /// Summed rather than listed because both readers want the sum: the HUD
    /// prints it and the mixer only asks whether it is non-zero.
    pub damage: u16,
    pub hits: u16,
    /// The bodies this frame's own hits landed on, oldest first — an
    /// **own-fact**, because `EV_HIT` is unicast to the attacker.
    ///
    /// Beside the sum rather than replacing it: the two readers of `damage`
    /// and `hits` want the total (the HUD prints it, the mixer asks only
    /// whether it is non-zero) and the flinch wants the identities, and
    /// neither can be recovered from the other. A hit on a wall
    /// (`EV_STRUCT_HIT`) shares the core's ring and carries
    /// `client_core::core::NO_VICTIM`, which is filtered out here rather
    /// than handed on — nothing downstream should have to know the
    /// sentinel exists.
    hit_victims: [u32; FEED_CAP],
    n_hit_victims: usize,
    /// Blows landed on **you** this frame, merged by the direction they
    /// arrived from. `hurt_damage` is the total across all of them and
    /// `hurts` how many arrived; [`Feed::hurt_from`] is the list.
    ///
    /// **Merged by sector, not queued, and the cap is therefore structural
    /// (wall 4).** A bearing is one of `HURT_SECTORS` values and the wire
    /// cannot carry another (`HURT_SECTOR_BITS = 4`), so an array with one
    /// slot per sector cannot overflow however many blows land in a frame:
    /// a fifth shot from the north is the north entry getting heavier, not
    /// a sixth entry. There is no drop policy here because there is nothing
    /// to drop — which is the shape a bounded queue wants whenever the key
    /// space is small enough to be the bound.
    ///
    /// This replaced a single `Option<u8>` that kept the **latest** blow
    /// (hurt direction v0, 2026-08-29). Two attackers on opposite sides
    /// collapsed to whichever the sim resolved second: right about one real
    /// threat, silent about the other, and worst exactly when a fight has
    /// more than one person in it. `NOW.md` §0hrt item 2 owed the list.
    ///
    /// The `Option` it replaced was there because sector 0 is **north** and
    /// not "nothing"; a list keeps that guarantee for free, since an empty
    /// slice is unambiguous where a sentinel sector never was.
    hurt: [Hurt; sim_core::combat::HURT_SECTORS as usize],
    n_hurt: usize,
    pub hurt_damage: u16,
    pub hurts: u16,
    deaths: [(u32, u32); FEED_CAP],
    n_deaths: usize,
    refusals: [Refused; FEED_CAP],
    refusal_codes: [u8; FEED_CAP],
    /// The item a refusal names, `sim_core::gather::NO_ITEM` when the
    /// sentence needs none — only `Refused::Gather` carries one today.
    refusal_items: [u16; FEED_CAP],
    n_refusals: usize,
    gathered: [(u16, u16); FEED_CAP],
    n_gathered: usize,
    crafted: [(u16, u16); FEED_CAP],
    n_crafted: usize,
    /// Items that did not fit and went to the ground this frame — a gather
    /// or a craft whose payout the pack could not hold. Own-fact, like the
    /// two rings above it and unlike `knocks`/`shots`.
    ///
    /// The item index only: the wire says what reached the hands and never
    /// what was paid, so there is no amount to carry (`client-core`'s
    /// `spills` says why in full).
    spills: [u16; FEED_CAP],
    n_spills: usize,
    /// `(recipe, coin burned)` per blueprint learned this frame.
    learned: [(u16, u16); FEED_CAP],
    n_learned: usize,
    /// Eats that landed this frame: (item index, the slot it was spent
    /// from). Own-fact; the refused half rides `refusals` as
    /// `Refused::Consume`. A ring since 2026-08-15 — it was a latched field
    /// pair (`last_eat` / `last_eat_refused`), and two consume answers in
    /// one drain window collapsed, which one frame reaches from the
    /// keyboard (`KeyJ` + `KeyH` are answered by one `World::tick`).
    consumed: [(u16, u16); FEED_CAP],
    n_consumed: usize,
    /// Knocks heard this frame: the door's address and who knocked (lock
    /// v1). Broadcast, so this is the one entry here that can be somebody
    /// else's action — the mixer wants the address, the HUD wants to say
    /// somebody is at the door.
    knocks: [(u16, u16, u8, u8, u32); FEED_CAP],
    n_knocks: usize,
    /// Grants earned this frame: address + `sim_core::lock::GRANT_*`.
    auths: [(u16, u16, u8, u8, u8); FEED_CAP],
    n_auths: usize,
    /// Arrows loosed this frame (wire v33): shooter id, yaw, pitch, and the
    /// round's speed and drop in mm/tick. Broadcast, like `knocks`.
    ///
    /// Cosmetic only. The tracer spawned from this decides nothing — the
    /// arrow that can kill you is the server's, and its hit arrives on its
    /// own events whether or not anything drew a streak.
    shots: [(u32, u16, u8, u16, u16); FEED_CAP],
    n_shots: usize,
    /// Arrow impacts heard this frame: the stop point in the wire's quanta
    /// (3 cm x/z, 1 cm y, y signed) and what it stopped on
    /// (`sim_core::ranged::SURF_*`).
    ///
    /// Broadcast, like `shots` and `knocks` above it and unlike the own-fact
    /// rings — every arrow on the island that stops on something lands here,
    /// not only this player's. Cosmetic only: what reads this leaves a mark,
    /// and a mark decides nothing.
    impacts: [(i32, i32, i32, u8); FEED_CAP],
    n_impacts: usize,
    /// Bodies whose arm started to move this frame, by wire entity id
    /// (broadcast, wire v47). Cosmetic and unvalidated: an id naming no
    /// live body matches nothing when `bodies::stream` walks its set.
    swings: [u32; FEED_CAP],
    n_swings: usize,
    /// Placements that happened this frame: address + which store (`true` =
    /// deployable). Broadcast-only by construction — the core's ring is fed
    /// by `PiecePlaced`/`DeployPlaced` and never by a sync walk, so a join
    /// or resync restating the whole world hands over nothing here. The
    /// mixer wants the address for the positional place cue.
    placed: [(u16, u16, u8, u8, bool); FEED_CAP],
    n_placed: usize,
    /// Every `APPLIED*` bit raised since the last drain.
    ///
    /// **Latched facts need this and rings do not.** `struct_hit`,
    /// `charge_placed` and `stock` are single fields on `ClientCore` holding
    /// the LAST one of their kind, so "is it fresh this frame" is not
    /// answerable from the field — only from the bit `on_stream` raised when
    /// it wrote. A reader without this either redraws the last hit forever or
    /// never notices the first. The word was discarded at the socket
    /// (`Session::pump`'s `let _`) until this landed, which is the whole
    /// reason those three had no readers.
    pub applied: u32,
    /// The same for word 1 (`APPLIED2_*`) — see [`Feed::applied`].
    pub applied2: u32,
    /// Facts refused for want of room since the last reset — see the header.
    pub dropped: u32,
    /// The client's smoothed estimate of the server tick
    /// (`client-core/clock.rs` `server_est`), copied here each drain so
    /// render systems can read the world's clock as a `Res<Feed>` instead
    /// of each taking the non-send `Net`. The day/night rig derives the
    /// time of day from it (`sim_core::world::day_frac`); zero until the
    /// first snapshot, which reads as the boot phase — mid-morning — and
    /// is exactly what a loading world should look like.
    pub server_tick_est: f64,
}

impl Feed {
    /// `(victim, killer)` pairs, oldest first.
    pub fn deaths(&self) -> &[(u32, u32)] {
        &self.deaths[..self.n_deaths]
    }
    /// `(which verb, reason code, named item)` triples, oldest first. The
    /// code is the verb's own `REFUSE_*` integer — `ui::refusals` owns the
    /// sentences — and the item is `NO_ITEM` for every verb whose sentence
    /// names none (all but `Gather` today).
    pub fn refusals(&self) -> impl Iterator<Item = (Refused, u8, u16)> + '_ {
        (0..self.n_refusals).map(|i| {
            (
                self.refusals[i],
                self.refusal_codes[i],
                self.refusal_items[i],
            )
        })
    }
    /// `(item index, units)` gathered this frame.
    pub fn gathered(&self) -> &[(u16, u16)] {
        &self.gathered[..self.n_gathered]
    }
    /// `(item index, units)` finished crafting this frame.
    pub fn crafted(&self) -> &[(u16, u16)] {
        &self.crafted[..self.n_crafted]
    }

    /// Item indices the pack could not hold this frame, oldest first.
    pub fn spills(&self) -> &[u16] {
        &self.spills[..self.n_spills]
    }

    /// `(recipe index, coin burned)` learned this frame (research v0).
    pub fn learned(&self) -> &[(u16, u16)] {
        &self.learned[..self.n_learned]
    }
    /// `(item index, slot)` eaten or used this frame, oldest first.
    pub fn consumed(&self) -> &[(u16, u16)] {
        &self.consumed[..self.n_consumed]
    }
    /// Knocks heard this frame, oldest first.
    pub fn knocks(&self) -> &[(u16, u16, u8, u8, u32)] {
        &self.knocks[..self.n_knocks]
    }
    /// Grants earned this frame, oldest first.
    /// `(shooter, yaw, pitch, speed mm/tick, drop mm/tick²)` this frame.
    pub fn shots(&self) -> &[(u32, u16, u8, u16, u16)] {
        &self.shots[..self.n_shots]
    }
    /// Arrow impacts heard this frame, oldest first.
    pub fn impacts(&self) -> &[(i32, i32, i32, u8)] {
        &self.impacts[..self.n_impacts]
    }

    /// Bodies that swung this frame, oldest first.
    pub fn swings(&self) -> &[u32] {
        &self.swings[..self.n_swings]
    }

    /// Bodies this player's blows landed on this frame, oldest first.
    /// Never contains `client_core::core::NO_VICTIM` — see the field.
    pub fn hit_victims(&self) -> &[u32] {
        &self.hit_victims[..self.n_hit_victims]
    }
    pub fn auths(&self) -> &[(u16, u16, u8, u8, u8)] {
        &self.auths[..self.n_auths]
    }
    /// Placements that happened this frame, oldest first.
    /// The directions blows arrived from this frame, merged, in the order
    /// each direction was first seen.
    ///
    /// Empty on a quiet frame, and that is the whole of the "is anything
    /// there" question — no sentinel, because sector 0 is north.
    pub fn hurt_from(&self) -> &[Hurt] {
        &self.hurt[..self.n_hurt]
    }

    /// Fold one blow into the frame's list, merging with the entry for the
    /// same bearing if there already is one.
    ///
    /// A linear scan over at most `HURT_SECTORS` entries. It is the right
    /// shape at this size and it is also the only shape wall 1's sibling
    /// rules leave attractive: a map keyed by sector would be a
    /// `HashMap`, and the array-indexed-by-sector alternative loses the
    /// arrival order, which is what decides who gets an arc when more
    /// directions turn up than there are arcs to draw.
    ///
    /// A sector the wire cannot produce is dropped rather than clamped —
    /// `HURT_SECTOR_BITS = 4` means the server range-refuses one before it
    /// is ever encoded, so reaching this arm is a decode defect and folding
    /// it into sector 0 would draw a confident north.
    fn note_hurt(&mut self, sector: u8, damage: u16) {
        if sector >= sim_core::combat::HURT_SECTORS {
            return;
        }
        for h in self.hurt[..self.n_hurt].iter_mut() {
            if h.from == sector {
                h.damage = h.damage.saturating_add(damage);
                h.hits = h.hits.saturating_add(1);
                return;
            }
        }
        // Unreachable by construction: one slot per sector, and the loop
        // above catches every repeat. Kept as a refusal rather than an
        // index, because "cannot overflow" is an argument and a `[n]` is a
        // panic when the argument stops being true.
        if self.n_hurt >= self.hurt.len() {
            return;
        }
        self.hurt[self.n_hurt] = Hurt {
            from: sector,
            damage,
            hits: 1,
        };
        self.n_hurt += 1;
    }

    pub fn placed(&self) -> &[(u16, u16, u8, u8, bool)] {
        &self.placed[..self.n_placed]
    }

    fn clear(&mut self) {
        self.damage = 0;
        self.hits = 0;
        self.n_hit_victims = 0;
        self.n_hurt = 0;
        self.hurt_damage = 0;
        self.hurts = 0;
        self.n_deaths = 0;
        self.n_refusals = 0;
        self.n_gathered = 0;
        self.n_crafted = 0;
        self.n_spills = 0;
        self.n_learned = 0;
        self.n_consumed = 0;
        self.n_knocks = 0;
        self.n_auths = 0;
        self.n_shots = 0;
        self.n_impacts = 0;
        self.n_swings = 0;
        self.n_placed = 0;
    }

    fn push_refusal(&mut self, which: Refused, code: u8, item: u16) {
        if self.n_refusals >= FEED_CAP {
            self.dropped = self.dropped.saturating_add(1);
            return;
        }
        self.refusals[self.n_refusals] = which;
        self.refusal_codes[self.n_refusals] = code;
        self.refusal_items[self.n_refusals] = item;
        self.n_refusals += 1;
    }
}

/// Drain the core's own-fact rings into [`Feed`].
///
/// **The only caller of `pop_*` in the client.** Anything that wants one of
/// these facts takes `Res<Feed>`; see the header for why that is a rule and
/// not a style.
///
/// Each ring is drained to EMPTY rather than one entry per frame — they are
/// small, and a backlog drip-fed at frame rate would still be showing the
/// first refusal after the tenth.
pub fn drain(mut net: NonSendMut<Net>, mut feed: ResMut<Feed>) {
    feed.clear();
    // Taken and cleared in one move: the word describes the messages drained
    // since the last frame, so leaving it set would report them again.
    feed.applied = core::mem::take(&mut net.session.applied);
    feed.applied2 = core::mem::take(&mut net.session.applied2);
    let core = &mut net.session.core;
    feed.server_tick_est = core.clock.server_est;

    while let Some((victim, d)) = core.pop_hit() {
        feed.damage = feed.damage.saturating_add(d);
        feed.hits = feed.hits.saturating_add(1);
        // A wall took it, not a person: the sentinel stops here.
        if victim == client_core::core::NO_VICTIM {
            continue;
        }
        if feed.n_hit_victims >= FEED_CAP {
            feed.dropped = feed.dropped.saturating_add(1);
        } else {
            let n = feed.n_hit_victims;
            feed.hit_victims[n] = victim;
            feed.n_hit_victims += 1;
        }
    }
    // The other half of the same blow. No sentinel to filter: a sector is
    // always a real direction, because a wall does not get hurt.
    while let Some((sector, d)) = core.pop_hurt() {
        feed.hurt_damage = feed.hurt_damage.saturating_add(d);
        feed.hurts = feed.hurts.saturating_add(1);
        feed.note_hurt(sector, d);
    }
    while let Some(victim) = core.pop_death() {
        // `last_death_killer` is set by the pop, so one pop yields a whole
        // feed line — see `ClientCore::pop_death`.
        let killer = core.last_death_killer;
        if feed.n_deaths >= FEED_CAP {
            feed.dropped = feed.dropped.saturating_add(1);
        } else {
            let n = feed.n_deaths;
            feed.deaths[n] = (victim, killer);
            feed.n_deaths += 1;
        }
    }
    while let Some(t) = core.pop_research_toast() {
        if feed.n_learned < FEED_CAP {
            let n = feed.n_learned;
            feed.learned[n] = t;
            feed.n_learned += 1;
        }
    }
    while let Some(code) = core.pop_research_refusal() {
        feed.push_refusal(Refused::Research, code, sim_core::gather::NO_ITEM);
    }
    while let Some(code) = core.pop_craft_refusal() {
        feed.push_refusal(Refused::Craft, code, sim_core::gather::NO_ITEM);
    }
    while let Some((item, code)) = core.pop_gather_refusal() {
        feed.push_refusal(Refused::Gather, code, item);
    }
    while let Some(code) = core.pop_build_refusal() {
        feed.push_refusal(Refused::Build, code, sim_core::gather::NO_ITEM);
    }
    while let Some(code) = core.pop_deploy_refusal() {
        feed.push_refusal(Refused::Deploy, code, sim_core::gather::NO_ITEM);
    }
    // The consume verbs, rings since 2026-08-15. They were a latched field
    // pair (`last_eat` / `last_eat_refused`) plus `APPLIED_CONSUME`, and two
    // answers in one drain window collapsed — `Consumed` zeroed the reason,
    // `ConsumeRefused` overwrote it — which one frame reaches from the
    // keyboard, because `KeyJ` and `KeyH` are two independent presses that
    // one `World::tick` answers together. `client-core`'s
    // `two_consume_answers_in_one_drain_window_both_surface` holds it.
    //
    // The refusal joins the shared queue rather than a private surface, and
    // that is the point: a refusal in this queue is a refusal to every
    // reader, so `render::audio` plays the refusal cue for a dry shoreline
    // without knowing the verb exists. Zero never arrives — it is not a
    // refusal on this wire and the encoder refuses it; the landed half is
    // its own ring below.
    while let Some(code) = core.pop_consume_refusal() {
        feed.push_refusal(Refused::Consume, code, sim_core::gather::NO_ITEM);
    }
    while let Some(t) = core.pop_consume_toast() {
        if feed.n_consumed >= FEED_CAP {
            feed.dropped = feed.dropped.saturating_add(1);
        } else {
            let n = feed.n_consumed;
            feed.consumed[n] = t;
            feed.n_consumed += 1;
        }
    }
    while let Some(k) = core.pop_knock() {
        if feed.n_knocks >= FEED_CAP {
            feed.dropped = feed.dropped.saturating_add(1);
        } else {
            let n = feed.n_knocks;
            feed.knocks[n] = k;
            feed.n_knocks += 1;
        }
    }
    while let Some(a) = core.pop_auth() {
        if feed.n_auths >= FEED_CAP {
            feed.dropped = feed.dropped.saturating_add(1);
        } else {
            let n = feed.n_auths;
            feed.auths[n] = a;
            feed.n_auths += 1;
        }
    }
    while let Some(sh) = core.pop_shot() {
        if feed.n_shots >= FEED_CAP {
            feed.dropped = feed.dropped.saturating_add(1);
        } else {
            let n = feed.n_shots;
            feed.shots[n] = sh;
            feed.n_shots += 1;
        }
    }
    while let Some(im) = core.pop_impact() {
        if feed.n_impacts >= FEED_CAP {
            feed.dropped = feed.dropped.saturating_add(1);
        } else {
            let n = feed.n_impacts;
            feed.impacts[n] = im;
            feed.n_impacts += 1;
        }
    }
    while let Some(sw) = core.pop_swing() {
        if feed.n_swings >= FEED_CAP {
            feed.dropped = feed.dropped.saturating_add(1);
        } else {
            let n = feed.n_swings;
            feed.swings[n] = sw;
            feed.n_swings += 1;
        }
    }
    while let Some(p) = core.pop_placed() {
        if feed.n_placed >= FEED_CAP {
            feed.dropped = feed.dropped.saturating_add(1);
        } else {
            let n = feed.n_placed;
            feed.placed[n] = p;
            feed.n_placed += 1;
        }
    }
    while let Some(t) = core.pop_toast() {
        if feed.n_gathered >= FEED_CAP {
            feed.dropped = feed.dropped.saturating_add(1);
        } else {
            let n = feed.n_gathered;
            feed.gathered[n] = t;
            feed.n_gathered += 1;
        }
    }
    while let Some(t) = core.pop_craft_toast() {
        if feed.n_crafted >= FEED_CAP {
            feed.dropped = feed.dropped.saturating_add(1);
        } else {
            let n = feed.n_crafted;
            feed.crafted[n] = t;
            feed.n_crafted += 1;
        }
    }
    while let Some(item) = core.pop_spill() {
        if feed.n_spills >= FEED_CAP {
            feed.dropped = feed.dropped.saturating_add(1);
        } else {
            let n = feed.n_spills;
            feed.spills[n] = item;
            feed.n_spills += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Blows from one bearing are **one** entry, and blows from different
    /// bearings are different entries.
    ///
    /// The half of hurt direction v1 that lives below the HUD. `Toast::hurt`
    /// merges by sector too, one layer up, and the two are not redundant:
    /// this one decides how many times the latch is *called* in a frame, and
    /// a burst that arrived as four separate calls would evict three other
    /// directions off a three-arc ring before the latch's own merge could
    /// help.
    #[test]
    fn one_gun_is_one_direction_and_two_guns_are_two() {
        let mut f = Feed::default();
        assert!(
            f.hurt_from().is_empty(),
            "a quiet frame is an empty list — there is no sentinel sector, \
             because sector 0 is north"
        );
        f.note_hurt(4, 10);
        f.note_hurt(4, 7);
        f.note_hurt(12, 3);
        f.note_hurt(4, 1);
        assert_eq!(
            f.hurt_from(),
            &[
                Hurt {
                    from: 4,
                    damage: 18,
                    hits: 3
                },
                Hurt {
                    from: 12,
                    damage: 3,
                    hits: 1
                },
            ],
            "four blows from two bearings are two entries, in the order the \
             bearings were first seen, with the damage summed per bearing"
        );
    }

    /// Every sector at once still fits, and a frame boundary empties it.
    ///
    /// The cap is structural rather than a policy: one slot per bearing and
    /// the wire cannot carry a bearing outside the set (`HURT_SECTOR_BITS`),
    /// so there is no overflow arm to get wrong (wall 4). This is that
    /// argument, run.
    #[test]
    fn the_whole_compass_fits_and_the_frame_clears_it() {
        let mut f = Feed::default();
        for s in 0..sim_core::combat::HURT_SECTORS {
            f.note_hurt(s, 1);
        }
        assert_eq!(
            f.hurt_from().len(),
            sim_core::combat::HURT_SECTORS as usize,
            "every bearing the wire can carry must have a slot — an entry \
             dropped here is a threat the ring can never learn about"
        );
        // A sector the wire cannot produce is refused, not folded into 0.
        f.note_hurt(sim_core::combat::HURT_SECTORS, 99);
        assert_eq!(
            f.hurt_from().len(),
            sim_core::combat::HURT_SECTORS as usize,
            "an out-of-range sector must not open a slot"
        );
        assert_eq!(
            f.hurt_from()[0],
            Hurt {
                from: 0,
                damage: 1,
                hits: 1
            },
            "and must not be folded into north, which would draw a confident \
             arc at a direction nothing came from"
        );
        f.clear();
        assert!(
            f.hurt_from().is_empty(),
            "the list is one frame's blows and the frame ended"
        );
    }
}
