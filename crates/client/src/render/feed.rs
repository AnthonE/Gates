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
//! Every array here is [`FEED_CAP`] long, which is `client_wasm`'s own
//! `TOAST_RING` — the core cannot hand over more per frame than its rings
//! hold, so the cap is exact rather than generous. Overflow policy: **drop the
//! newest and count it** ([`Feed::dropped`]), matching `sound::CUE_QUEUE_CAP`.
//! A non-zero count means the core grew a ring and this file did not follow.

use bevy::prelude::*;
use client_wasm::core::TOAST_RING;

use super::Net;

/// How many of each fact one frame may carry. The core's own ring size, so
/// a full drain always fits.
pub const FEED_CAP: usize = TOAST_RING;

/// Which verb a refusal answers. The three refusal queues are separate on the
/// wire and stay separate here, because the HUD turns each into a different
/// sentence (`ui::refusals`) — the audio side is the one that does not care.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Refused {
    // The default only exists so `Feed` can derive one for its fixed array;
    // no reader ever sees a slot past `n_refusals`.
    #[default]
    Craft,
    Build,
    Deploy,
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
    n_refusals: usize,
    gathered: [(u16, u16); FEED_CAP],
    n_gathered: usize,
    crafted: [(u16, u16); FEED_CAP],
    n_crafted: usize,
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
    /// Facts refused for want of room since the last reset — see the header.
    pub dropped: u32,
}

impl Feed {
    /// `(victim, killer)` pairs, oldest first.
    pub fn deaths(&self) -> &[(u32, u32)] {
        &self.deaths[..self.n_deaths]
    }
    /// `(which verb, reason code)` pairs, oldest first. The code is the
    /// verb's own `REFUSE_*` integer — `ui::refusals` owns the sentences.
    pub fn refusals(&self) -> impl Iterator<Item = (Refused, u8)> + '_ {
        (0..self.n_refusals).map(|i| (self.refusals[i], self.refusal_codes[i]))
    }
    /// `(item index, units)` gathered this frame.
    pub fn gathered(&self) -> &[(u16, u16)] {
        &self.gathered[..self.n_gathered]
    }
    /// `(item index, units)` finished crafting this frame.
    pub fn crafted(&self) -> &[(u16, u16)] {
        &self.crafted[..self.n_crafted]
    }

    fn clear(&mut self) {
        self.damage = 0;
        self.hits = 0;
        self.n_deaths = 0;
        self.n_refusals = 0;
        self.n_gathered = 0;
        self.n_crafted = 0;
    }

    fn push_refusal(&mut self, which: Refused, code: u8) {
        if self.n_refusals >= FEED_CAP {
            self.dropped = self.dropped.saturating_add(1);
            return;
        }
        self.refusals[self.n_refusals] = which;
        self.refusal_codes[self.n_refusals] = code;
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
    let core = &mut net.session.core;

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
    while let Some(code) = core.pop_craft_refusal() {
        feed.push_refusal(Refused::Craft, code);
    }
    while let Some(code) = core.pop_build_refusal() {
        feed.push_refusal(Refused::Build, code);
    }
    while let Some(code) = core.pop_deploy_refusal() {
        feed.push_refusal(Refused::Deploy, code);
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
}
