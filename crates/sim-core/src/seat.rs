//! Sitting down: THE GATE's casino seats (`town::SEATS`), Rust's chairs.
//!
//! `Command::Sit` puts a live body on a free seat in reach: its feet go to
//! the seat's spot and [`Player::seat`] holds which. Nothing else holds it
//! there. A sitter sends no movement, so the body stays put on its own; the
//! first step or jump moves it off the spot and the tick stands it up
//! ([`stand_if_moved`]). That is why the client's predictor needs no seat:
//! the frames it steps are the frames the sim steps, seated or not. A death,
//! a fall asleep or a wound stands the body up too.

use crate::movement::{quant_xz, Body, POS_XZ_Q};
use crate::town::{self, Town};
use crate::world::Player;

/// How far from a seat's spot a player may sit in it, metres (planar).
pub const SEAT_REACH_M: f32 = 2.0;
/// `Command::Sit`'s seat for "stand up".
pub const SEAT_NONE: u8 = 255;

/// Whether seat `k` (0-based) is taken by a body other than `slot`'s.
fn taken(players: &[Player], slot: usize, k: usize) -> bool {
    players.iter().enumerate().any(|(i, p)| {
        i != slot && p.active && p.seat as usize == k + 1 && !p.dead && !p.sleeping && !p.wounded
    })
}

/// Sit `players[slot]` in seat `seat`, or stand it up for [`SEAT_NONE`]. A
/// seat out of reach, taken, or under a body in the air is refused without
/// a word: the client offers only a free seat in reach.
pub fn sit(town: &Town, players: &mut [Player], slot: usize, seat: u8) {
    if seat == SEAT_NONE {
        players[slot].seat = 0;
        return;
    }
    let k = seat as usize;
    let Some((sx, sz)) = town::seat_world(town, k) else {
        return;
    };
    if !town.live || !players[slot].body.grounded || taken(players, slot, k) {
        return;
    }
    let p = &mut players[slot];
    let (x, z) = (p.body.qx as f32 * POS_XZ_Q, p.body.qz as f32 * POS_XZ_Q);
    if (x - sx) * (x - sx) + (z - sz) * (z - sz) > SEAT_REACH_M * SEAT_REACH_M {
        return;
    }
    p.body.qx = quant_xz(sx);
    p.body.qz = quant_xz(sz);
    p.seat = seat + 1;
}

/// Stand a sitter up if this tick's step moved it off its spot or off the
/// ground: a step, a jump, anything the frames did.
pub fn stand_if_moved(p: &mut Player, before: &Body) {
    if p.seat != 0 && (p.body.qx != before.qx || p.body.qz != before.qz || !p.body.grounded) {
        p.seat = 0;
    }
}
