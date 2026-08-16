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

/// One frame of own-facts. Cleared and refilled by [`drain`]; read-only to
/// everything else.
#[derive(Resource, Default)]
pub struct Feed {
    /// Total damage the local player dealt this frame, and whether any landed.
    /// Summed rather than listed because both readers want the sum: the HUD
    /// prints it and the mixer only asks whether it is non-zero.
    pub damage: u16,
    pub hits: u16,
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
    pub fn auths(&self) -> &[(u16, u16, u8, u8, u8)] {
        &self.auths[..self.n_auths]
    }
    /// Placements that happened this frame, oldest first.
    pub fn placed(&self) -> &[(u16, u16, u8, u8, bool)] {
        &self.placed[..self.n_placed]
    }

    fn clear(&mut self) {
        self.damage = 0;
        self.hits = 0;
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

    while let Some(d) = core.pop_hit() {
        feed.damage = feed.damage.saturating_add(d);
        feed.hits = feed.hits.saturating_add(1);
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
